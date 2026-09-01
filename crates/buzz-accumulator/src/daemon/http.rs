//! Localhost HTTP API: status for the future standalone UI, plus the fold
//! machinery (selections, folds, runs, artifacts).
//!
//! The daemon is headless; this is its only external surface. It binds
//! loopback only — the mirror contains everything the key can read.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::runner::FoldRunner;
use crate::{schema, FoldSpec, Selection};

use super::folds::{self, FoldError, RunGuard};
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
}

/// Builds the router. Separated from serving for tests.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/status", get(get_status))
        .route("/channels", get(get_channels))
        .route("/select/preview", post(select_preview))
        .route("/folds", get(list_folds))
        .route(
            "/folds/{name}",
            get(get_fold).put(put_fold).delete(delete_fold),
        )
        .route("/folds/{name}/preflight", post(preflight_fold))
        .route("/folds/{name}/run", post(run_fold))
        .route("/folds/{name}/artifacts", get(list_artifacts))
        .route("/folds/{name}/artifacts/{version}", get(get_artifact))
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

/// Window bounds shared by preview/preflight/run bodies. Omitted bounds mean
/// "everything up to now".
#[derive(Debug, Default, serde::Deserialize)]
struct WindowBody {
    since: Option<i64>,
    until_exclusive: Option<i64>,
}

impl WindowBody {
    fn resolve(&self) -> (i64, i64) {
        let (default_since, default_until) = folds::default_window();
        (
            self.since.unwrap_or(default_since),
            self.until_exclusive.unwrap_or(default_until),
        )
    }
}

#[derive(Debug, serde::Deserialize)]
struct SelectPreviewBody {
    selection: Selection,
    #[serde(flatten)]
    window: WindowBody,
}

/// What would this selection materialize? Count + size, zero model calls.
async fn select_preview(
    State(s): State<AppState>,
    Json(body): Json<SelectPreviewBody>,
) -> Result<Json<Value>, ApiError> {
    let mut selection = body.selection;
    selection.canonicalize()?;
    let (since, until) = body.window.resolve();
    let signals = s.store.query_signals(&selection, since, until).await?;
    let total_chars: usize = signals.iter().map(|sig| sig.content.len()).sum();
    Ok(Json(json!({
        "selection": selection,
        "window": { "since": since, "until_exclusive": until },
        "count": signals.len(),
        "total_chars": total_chars,
        "oldest_ts": signals.first().map(|sig| sig.created_at),
        "newest_ts": signals.last().map(|sig| sig.created_at),
    })))
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
    /// Defaults to the built-in channel digest prompt.
    instructions: Option<String>,
    /// Defaults to `channel-digest@v1` (the only schema).
    schema: Option<String>,
}

async fn put_fold(
    State(s): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<PutFoldBody>,
) -> Result<Json<Value>, ApiError> {
    let mut spec = FoldSpec {
        name,
        selection: body.selection,
        schema: body
            .schema
            .unwrap_or_else(|| schema::CHANNEL_DIGEST_V1.name.to_string()),
        model: body.model,
        instructions: body
            .instructions
            .filter(|i| !i.trim().is_empty())
            .unwrap_or_else(|| schema::CHANNEL_DIGEST_PROMPT.to_string()),
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

async fn preflight_fold(
    State(s): State<AppState>,
    Path(name): Path<String>,
    body: Option<Json<WindowBody>>,
) -> Result<Json<Value>, ApiError> {
    let (since, until) = body
        .map(|Json(w)| w.resolve())
        .unwrap_or_else(|| WindowBody::default().resolve());
    let out = folds::preflight(&s.store, &name, since, until).await?;
    Ok(Json(
        serde_json::to_value(out).unwrap_or_else(|_| json!({})),
    ))
}

async fn run_fold(
    State(s): State<AppState>,
    Path(name): Path<String>,
    body: Option<Json<WindowBody>>,
) -> Result<Json<Value>, ApiError> {
    let (since, until) = body
        .map(|Json(w)| w.resolve())
        .unwrap_or_else(|| WindowBody::default().resolve());
    let out = folds::run_fold(&s.store, s.runner.clone(), &s.runs, &name, since, until).await?;
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
        "schema": a.schema,
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

    async fn test_state() -> AppState {
        AppState {
            store: Store::open(":memory:").await.expect("open"),
            registry: StatusRegistry::new("wss://test", "ab", ":memory:", 0),
            runner: Arc::new(NoRunner),
            runs: RunGuard::default(),
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
        assert_eq!(saved["saved"]["schema"], "channel-digest@v1");
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
}
