//! Localhost HTTP API: status for the future standalone UI, plus the fold
//! machinery (selections, folds, runs, artifacts).
//!
//! The daemon is headless; this is its only external surface. It binds
//! loopback only — the mirror contains everything the key can read.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::runner::FoldRunner;
use crate::{FoldSpec, Selection};

use super::folds::{self, FoldError, RunGuard, WindowClamp};
use super::publish::Publisher;
use super::status::StatusRegistry;
use super::store::Store;

/// Shared handler state.
#[derive(Clone)]
pub struct AppState {
    /// Local mirror + fold storage.
    pub store: Store,
    /// Live sync status.
    pub registry: StatusRegistry,
    /// Model runner used by `POST /folds/{name}/run`.
    pub runner: Arc<dyn FoldRunner + Send + Sync>,
    /// Per-fold run serialization (concurrent runs 409 instead of racing).
    pub runs: RunGuard,
    /// Publishes artifacts back into channels as messages.
    pub publisher: Arc<dyn Publisher>,
}

/// Builds the router. Separated from serving for tests.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route("/channels", get(get_channels))
        .route("/events/{id}", get(get_event))
        .route("/select/preview", post(select_preview))
        .route("/select/events", post(select_events))
        .route("/folds", get(list_folds))
        .route(
            "/folds/{name}",
            get(get_fold).put(put_fold).delete(delete_fold),
        )
        .route("/folds/{name}/preflight", post(preflight_fold))
        .route("/folds/{name}/run", post(run_fold))
        .route("/folds/{name}/artifacts", get(list_artifacts))
        .route("/folds/{name}/artifacts/{version}", get(get_artifact))
        .route(
            "/folds/{name}/artifacts/{version}/publish",
            post(publish_artifact),
        )
        .with_state(state)
}

/// Serves the API on `addr` (loopback) until the process exits.
pub async fn serve(state: AppState, addr: std::net::SocketAddr) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "http api listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

/// JSON error body with a proper status code.
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("storage: {e}"))
    }
}

impl From<FoldError> for ApiError {
    fn from(e: FoldError) -> Self {
        match &e {
            FoldError::NotFound(_) => ApiError(StatusCode::NOT_FOUND, e.to_string()),
            FoldError::Busy(_) => ApiError(StatusCode::CONFLICT, e.to_string()),
            FoldError::Engine(crate::Error::InvalidSpec(_)) => {
                ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
            }
            _ => ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
}

impl From<crate::Error> for ApiError {
    fn from(e: crate::Error) -> Self {
        match &e {
            crate::Error::InvalidSpec(_) => {
                ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string())
            }
            _ => ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        }
    }
}

async fn get_status(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    let (total, per_channel) = s.store.event_counts().await?;
    let folds = s.store.folds().await?.len() as i64;
    let artifacts = s.store.artifact_count().await?;
    let snap = s.registry.snapshot(total, &per_channel, folds, artifacts);
    Ok(Json(
        serde_json::to_value(snap).unwrap_or_else(|_| json!({})),
    ))
}

async fn get_channels(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    let channels = s.store.channels().await?;
    Ok(Json(json!({ "channels": channels })))
}

/// Optional window clamp shared by preview/preflight/run bodies. The
/// selection's own window is authoritative; these bounds can only narrow it
/// (this is how a run stays pinned to its priced preflight window).
#[derive(Debug, Default, serde::Deserialize)]
struct WindowBody {
    since: Option<i64>,
    until_exclusive: Option<i64>,
}

impl WindowBody {
    fn clamp(&self) -> WindowClamp {
        (self.since, self.until_exclusive)
    }
}

#[derive(Debug, serde::Deserialize)]
struct SelectPreviewBody {
    selection: Selection,
    #[serde(flatten)]
    window: WindowBody,
}

/// What would this selection materialize? Count + size + daily rhythm,
/// zero model calls.
async fn select_preview(
    State(s): State<AppState>,
    Json(body): Json<SelectPreviewBody>,
) -> Result<Json<Value>, ApiError> {
    let mut selection = body.selection;
    selection.canonicalize()?;
    let now = chrono::Utc::now().timestamp();
    let (since, until) =
        selection.resolve_window(body.window.since, body.window.until_exclusive, now);
    let signals = s.store.query_signals(&selection, since, until).await?;
    let total_chars: usize = signals.iter().map(|sig| sig.content.len()).sum();
    // Daily histogram (UTC day starts). Sparse: days with no matches are absent.
    let mut buckets: BTreeMap<i64, i64> = BTreeMap::new();
    for sig in &signals {
        *buckets
            .entry(sig.created_at.div_euclid(86_400) * 86_400)
            .or_default() += 1;
    }
    Ok(Json(json!({
        "selection": selection,
        "window": { "since": since, "until_exclusive": until },
        "count": signals.len(),
        "total_chars": total_chars,
        "oldest_ts": signals.first().map(|sig| sig.created_at),
        "newest_ts": signals.last().map(|sig| sig.created_at),
        "buckets": buckets
            .iter()
            .map(|(day, n)| json!({ "day": day, "count": n }))
            .collect::<Vec<_>>(),
    })))
}

/// Keyset cursor for [`select_events`]: the `(created_at, id)` of the last
/// row of the previous page.
#[derive(Debug, serde::Deserialize)]
struct PageCursor {
    created_at: i64,
    id: String,
}

#[derive(Debug, serde::Deserialize)]
struct SelectEventsBody {
    selection: Selection,
    #[serde(flatten)]
    window: WindowBody,
    /// Page size; clamped to `1..=500`, default 100.
    limit: Option<i64>,
    /// Resume after this cursor (from the previous page's `next`).
    after: Option<PageCursor>,
}

/// The actual matching events, paged, oldest first — the selection browser.
async fn select_events(
    State(s): State<AppState>,
    Json(body): Json<SelectEventsBody>,
) -> Result<Json<Value>, ApiError> {
    let mut selection = body.selection;
    selection.canonicalize()?;
    let now = chrono::Utc::now().timestamp();
    let (since, until) =
        selection.resolve_window(body.window.since, body.window.until_exclusive, now);
    let limit = body.limit.unwrap_or(100).clamp(1, 500);
    let after = body.after.as_ref().map(|c| (c.created_at, c.id.as_str()));
    // Fetch one extra row purely to learn whether a next page exists.
    let mut page = s
        .store
        .page_signals(&selection, since, until, after, limit + 1)
        .await?;
    let has_more = page.len() as i64 > limit;
    page.truncate(limit as usize);
    let authors: BTreeSet<String> = page.iter().map(|sig| sig.pubkey.clone()).collect();
    let names = s.store.names(&authors).await?;
    let next = if has_more {
        page.last()
            .map(|sig| json!({ "created_at": sig.created_at, "id": sig.id }))
    } else {
        None
    };
    Ok(Json(json!({
        "window": { "since": since, "until_exclusive": until },
        "count": page.len(),
        "next": next,
        "events": page
            .iter()
            .map(|sig| event_json(sig, names.get(&sig.pubkey)))
            .collect::<Vec<_>>(),
    })))
}

/// Resolves one mirrored event by id — the citation-chip endpoint.
async fn get_event(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let sig = s
        .store
        .event_by_id(&id)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("event not found: {id}")))?;
    let names = s.store.names(&BTreeSet::from([sig.pubkey.clone()])).await?;
    Ok(Json(event_json(&sig, names.get(&sig.pubkey))))
}

fn event_json(sig: &crate::Signal, author_name: Option<&String>) -> Value {
    json!({
        "id": sig.id,
        "channel": sig.channel,
        "pubkey": sig.pubkey,
        "author_name": author_name,
        "kind": sig.kind,
        "created_at": sig.created_at,
        "content": sig.content,
    })
}

async fn list_folds(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "folds": s.store.folds().await? })))
}

async fn get_fold(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let spec = s
        .store
        .get_fold(&name)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("fold not found: {name}")))?;
    let chain = s.store.artifacts(&name).await?;
    Ok(Json(json!({
        "spec": spec,
        "versions": chain.len(),
        "latest": chain.last().map(artifact_summary),
    })))
}

#[derive(Debug, serde::Deserialize)]
struct PutFoldBody {
    selection: Selection,
    model: String,
    /// Defaults to the built-in running-digest task.
    instructions: Option<String>,
    /// Free-form client-owned JSON, stored verbatim and never read by the
    /// engine (see [`FoldSpec::meta`]).
    meta: Option<serde_json::Value>,
}

async fn put_fold(
    State(s): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<PutFoldBody>,
) -> Result<Json<Value>, ApiError> {
    let mut spec = FoldSpec {
        name,
        selection: body.selection,
        model: body.model,
        instructions: body
            .instructions
            .filter(|i| !i.trim().is_empty())
            .unwrap_or_else(|| crate::spec::DEFAULT_INSTRUCTIONS.to_string()),
        meta: body.meta,
    };
    spec.validate()?;
    s.store
        .put_fold(&spec, chrono::Utc::now().timestamp())
        .await?;
    Ok(Json(json!({ "saved": spec })))
}

async fn delete_fold(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if !s.store.delete_fold(&name).await? {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("fold not found: {name}"),
        ));
    }
    // Artifacts are append-only history and deliberately survive spec deletion.
    Ok(Json(json!({ "deleted": name })))
}

#[derive(Debug, Default, serde::Deserialize)]
struct PreflightBody {
    #[serde(flatten)]
    window: WindowBody,
    /// When true, the Ready plan carries the exact string the model would
    /// receive — the "show me what's behind the curtain" seam.
    #[serde(default)]
    include_input: bool,
}

async fn preflight_fold(
    State(s): State<AppState>,
    Path(name): Path<String>,
    body: Option<Json<PreflightBody>>,
) -> Result<Json<Value>, ApiError> {
    let Json(body) = body.unwrap_or_default();
    let out = folds::preflight(&s.store, &name, body.window.clamp(), body.include_input).await?;
    Ok(Json(
        serde_json::to_value(out).unwrap_or_else(|_| json!({})),
    ))
}

async fn run_fold(
    State(s): State<AppState>,
    Path(name): Path<String>,
    body: Option<Json<WindowBody>>,
) -> Result<Json<Value>, ApiError> {
    let clamp = body.map(|Json(w)| w.clamp()).unwrap_or((None, None));
    let out = folds::run_fold(&s.store, s.runner.clone(), &s.runs, &name, clamp).await?;
    Ok(Json(
        serde_json::to_value(out).unwrap_or_else(|_| json!({})),
    ))
}

fn artifact_summary(a: &crate::ArtifactPayload) -> Value {
    json!({
        "version": a.version,
        "created_at": a.created_at,
        "shown": a.shown_ids.len(),
        "coverage_since": a.coverage_since,
        "coverage_until": a.coverage_until,
        "channels": a.channels,
        "model": a.model,
        "truncated": a.truncated,
    })
}

async fn list_artifacts(
    State(s): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let chain = s.store.artifacts(&name).await?;
    Ok(Json(json!({
        "fold": name,
        "artifacts": chain.iter().map(artifact_summary).collect::<Vec<_>>(),
    })))
}

async fn get_artifact(
    State(s): State<AppState>,
    Path((name, version)): Path<(String, u32)>,
) -> Result<Json<Value>, ApiError> {
    let artifact = s.store.artifact(&name, version).await?.ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            format!("artifact not found: {name} v{version}"),
        )
    })?;
    Ok(Json(
        serde_json::to_value(artifact).unwrap_or_else(|_| json!({})),
    ))
}

#[derive(Debug, serde::Deserialize)]
struct PublishBody {
    /// Channel UUID to post into.
    channel: String,
    /// The taint guard requires the artifact's chain to have read only the
    /// target channel; set this to cross that line deliberately.
    #[serde(default)]
    allow_cross_channel: bool,
}

/// Publishes an artifact version into a channel as an ordinary message —
/// the composition verb. The mirror's live tail picks the message back up,
/// so a later fold can select it: folds all the way down.
async fn publish_artifact(
    State(s): State<AppState>,
    Path((name, version)): Path<(String, u32)>,
    Json(body): Json<PublishBody>,
) -> Result<Json<Value>, ApiError> {
    let artifact = s.store.artifact(&name, version).await?.ok_or_else(|| {
        ApiError(
            StatusCode::NOT_FOUND,
            format!("artifact not found: {name} v{version}"),
        )
    })?;
    let channel = body.channel.trim().to_lowercase();
    // Taint guard: the chain-union `channels` is what this artifact has ever
    // read. Publishing into a channel it didn't come from can leak another
    // channel's content, so that path is opt-in only.
    if !body.allow_cross_channel && artifact.channels != vec![channel.clone()] {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!(
                "artifact provenance covers channels {:?}, not just {channel} — \
                 pass allow_cross_channel to publish it there deliberately",
                artifact.channels
            ),
        ));
    }
    let content = format!(
        "{}\n\n— fold {} v{} · folded {} event(s) · {}",
        artifact.output.trim_end(),
        artifact.fold,
        artifact.version,
        artifact.shown_ids.len(),
        artifact.model,
    );
    let event_id = s
        .publisher
        .publish(&channel, &content)
        .await
        .map_err(|e| ApiError(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(json!({
        "published": event_id,
        "channel": channel,
        "fold": name,
        "version": version,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    const CHANNEL: &str = "6ba7b810-9dad-11d1-80b4-00c04fd430c8";

    struct NoRunner;
    impl FoldRunner for NoRunner {
        fn run(&self, _: &str, _: &str) -> Result<String, crate::Error> {
            Err(crate::Error::Runner("no model in tests".into()))
        }
    }

    /// Records publishes instead of touching a relay.
    #[derive(Default)]
    struct FakePublisher {
        calls: std::sync::Mutex<Vec<(String, String)>>,
    }
    impl Publisher for FakePublisher {
        fn publish<'a>(
            &'a self,
            channel: &'a str,
            content: &'a str,
        ) -> futures_util::future::BoxFuture<'a, Result<String, String>> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .expect("lock")
                    .push((channel.to_string(), content.to_string()));
                Ok("e".repeat(64))
            })
        }
    }

    async fn test_state() -> AppState {
        test_state_with(Arc::new(FakePublisher::default())).await
    }

    async fn test_state_with(publisher: Arc<FakePublisher>) -> AppState {
        AppState {
            store: Store::open(":memory:").await.expect("open"),
            registry: StatusRegistry::new("wss://test", "ab", ":memory:", 0),
            runner: Arc::new(NoRunner),
            runs: RunGuard::default(),
            publisher,
        }
    }

    async fn body_json(resp: Response) -> Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    #[tokio::test]
    async fn status_reports_shape() {
        let app = router(test_state().await);
        let resp = app
            .oneshot(Request::get("/status").body(Body::empty()).expect("req"))
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["relay"], "wss://test");
        assert_eq!(v["total_events"], 0);
        assert_eq!(v["connection"], "connecting");
    }

    #[tokio::test]
    async fn fold_crud_over_http() {
        let state = test_state().await;
        let app = router(state);
        let put = Request::put("/folds/weekly")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "selection": { "channels": [CHANNEL] }, "model": "haiku" }).to_string(),
            ))
            .expect("req");
        let resp = app.clone().oneshot(put).await.expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);
        let saved = body_json(resp).await;
        assert!(!saved["saved"]["instructions"]
            .as_str()
            .unwrap_or("")
            .is_empty());

        let get = Request::get("/folds/weekly")
            .body(Body::empty())
            .expect("req");
        let resp = app.clone().oneshot(get).await.expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);

        let del = Request::delete("/folds/weekly")
            .body(Body::empty())
            .expect("req");
        assert_eq!(
            app.clone().oneshot(del).await.expect("resp").status(),
            StatusCode::OK
        );
        let get = Request::get("/folds/weekly")
            .body(Body::empty())
            .expect("req");
        assert_eq!(
            app.oneshot(get).await.expect("resp").status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn invalid_fold_spec_is_422() {
        let app = router(test_state().await);
        // Leading dash violates the fold-name contract (edge dashes refused).
        let put = Request::put("/folds/-badname")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "selection": { "channels": [CHANNEL] }, "model": "haiku" }).to_string(),
            ))
            .expect("req");
        let resp = app.oneshot(put).await.expect("resp");
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn preflight_on_empty_mirror_stalls_without_spend() {
        let state = test_state().await;
        let app = router(state.clone());
        let put = Request::put("/folds/weekly")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "selection": { "channels": [CHANNEL] }, "model": "haiku" }).to_string(),
            ))
            .expect("req");
        app.clone().oneshot(put).await.expect("resp");
        let pf = Request::post("/folds/weekly/preflight")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .expect("req");
        let resp = app.oneshot(pf).await.expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["plan"], "stalled");
    }

    fn seeded(id_char: char, ts: i64) -> super::super::store::StoredEvent {
        super::super::store::StoredEvent {
            id: id_char.to_string().repeat(64),
            channel: Some(CHANNEL.into()),
            pubkey: "a".repeat(64),
            kind: 9,
            created_at: ts,
            content: format!("msg-{id_char}"),
            raw: "{}".into(),
        }
    }

    #[tokio::test]
    async fn event_lookup_resolves_and_404s() {
        let state = test_state().await;
        state
            .store
            .upsert_events(&[seeded('e', 100)])
            .await
            .expect("seed");
        let app = router(state);
        let id = "e".repeat(64);
        let resp = app
            .clone()
            .oneshot(
                Request::get(format!("/events/{id}"))
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["id"], id.as_str());
        assert_eq!(v["content"], "msg-e");
        let missing = format!("/events/{}", "f".repeat(64));
        let resp = app
            .oneshot(Request::get(missing).body(Body::empty()).expect("req"))
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn select_events_pages_with_keyset_cursor() {
        let state = test_state().await;
        state
            .store
            .upsert_events(&[seeded('1', 100), seeded('2', 200), seeded('3', 300)])
            .await
            .expect("seed");
        let app = router(state);
        let page = |body: Value| {
            Request::post("/select/events")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("req")
        };
        let body = json!({ "selection": { "channels": [CHANNEL] }, "limit": 2 });
        let resp = app.clone().oneshot(page(body)).await.expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["count"], 2);
        assert_eq!(v["events"][0]["created_at"], 100);
        let next = v["next"].clone();
        assert_eq!(next["created_at"], 200);

        let body = json!({ "selection": { "channels": [CHANNEL] }, "limit": 2, "after": next });
        let v = body_json(app.oneshot(page(body)).await.expect("resp")).await;
        assert_eq!(v["count"], 1);
        assert_eq!(v["events"][0]["created_at"], 300);
        assert!(v["next"].is_null(), "last page must not offer a cursor");
    }

    #[tokio::test]
    async fn preview_reports_daily_buckets() {
        let state = test_state().await;
        // Two events on day 0, one on day 2 (UTC-day starts).
        state
            .store
            .upsert_events(&[
                seeded('1', 100),
                seeded('2', 200),
                seeded('3', 2 * 86_400 + 5),
            ])
            .await
            .expect("seed");
        let app = router(state);
        let req = Request::post("/select/preview")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({ "selection": { "channels": [CHANNEL] } }).to_string(),
            ))
            .expect("req");
        let v = body_json(app.oneshot(req).await.expect("resp")).await;
        assert_eq!(v["count"], 3);
        assert_eq!(
            v["buckets"],
            json!([
                { "day": 0, "count": 2 },
                { "day": 2 * 86_400, "count": 1 },
            ])
        );
    }

    #[tokio::test]
    async fn fold_meta_roundtrips_verbatim() {
        let state = test_state().await;
        let app = router(state);
        let meta = json!({ "strategy": { "kind": "partition", "period": "isoweek" } });
        let put = Request::put("/folds/digest--2026-w35")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "selection": { "channels": [CHANNEL] },
                    "model": "haiku",
                    "meta": meta,
                })
                .to_string(),
            ))
            .expect("req");
        let v = body_json(app.clone().oneshot(put).await.expect("resp")).await;
        assert_eq!(v["saved"]["meta"], meta);
        let get = Request::get("/folds/digest--2026-w35")
            .body(Body::empty())
            .expect("req");
        let v = body_json(app.oneshot(get).await.expect("resp")).await;
        assert_eq!(v["spec"]["meta"], meta);
    }

    #[tokio::test]
    async fn missing_artifact_is_404() {
        let app = router(test_state().await);
        let resp = app
            .oneshot(
                Request::get("/folds/weekly/artifacts/3")
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    fn artifact(channels: &[&str]) -> crate::ArtifactPayload {
        crate::ArtifactPayload {
            fold: "weekly".into(),
            version: 1,
            output: "the digest".into(),
            shown_ids: vec!["s".repeat(64), "t".repeat(64)],
            coverage_since: Some(100),
            coverage_until: Some(201),
            selection: Selection {
                channels: channels.iter().map(|s| s.to_string()).collect(),
                ..Selection::default()
            },
            channels: channels.iter().map(|s| s.to_string()).collect(),
            model: "haiku".into(),
            prompt_sha256: "0".repeat(64),
            truncated: false,
            created_at: 1,
        }
    }

    fn publish_req(body: Value) -> Request<Body> {
        Request::post("/folds/weekly/artifacts/1/publish")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("req")
    }

    #[tokio::test]
    async fn publish_posts_artifact_with_provenance_footer() {
        let fake = Arc::new(FakePublisher::default());
        let state = test_state_with(fake.clone()).await;
        state
            .store
            .insert_artifact(&artifact(&[CHANNEL]))
            .await
            .expect("seed artifact");
        let app = router(state);
        let resp = app
            .oneshot(publish_req(json!({ "channel": CHANNEL })))
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["published"], "e".repeat(64));
        assert_eq!(v["channel"], CHANNEL);
        let calls = fake.calls.lock().expect("lock");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, CHANNEL);
        assert!(calls[0].1.starts_with("the digest\n\n— fold weekly v1"));
        assert!(calls[0].1.contains("2 event(s)"));
        assert!(calls[0].1.contains("haiku"));
    }

    #[tokio::test]
    async fn publish_refuses_cross_channel_unless_deliberate() {
        let state = test_state().await;
        // Chain has read a second channel; sharing into CHANNEL alone leaks it.
        let other = "0e415c05-98cc-4c39-9a10-e07ba0479560";
        let mut a = artifact(&[CHANNEL]);
        a.channels = vec![other.to_string(), CHANNEL.to_string()];
        state.store.insert_artifact(&a).await.expect("seed");
        let app = router(state);
        let resp = app
            .clone()
            .oneshot(publish_req(json!({ "channel": CHANNEL })))
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let v = body_json(resp).await;
        assert!(v["error"]
            .as_str()
            .expect("error string")
            .contains("allow_cross_channel"));
        // The same publish goes through when crossed deliberately.
        let resp = app
            .oneshot(publish_req(
                json!({ "channel": CHANNEL, "allow_cross_channel": true }),
            ))
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn publish_of_missing_artifact_is_404() {
        let app = router(test_state().await);
        let resp = app
            .oneshot(publish_req(json!({ "channel": CHANNEL })))
            .await
            .expect("resp");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
