//! Channel subscription management, called by relays over NIP-98.
//!
//! ```text
//! PUT  /v1/integrations/{id}/subscriptions/{sid}         create or replace
//! GET  /v1/integrations/{id}/subscriptions/{sid}         read (tombstones too)
//! POST /v1/integrations/{id}/subscriptions/{sid}/delete  tombstone
//! GET  /v1/integrations/{id}/subscriptions               live subscriptions
//! ```
//!
//! Writes are idempotent by revision: each carries the revision it produces,
//! a retry of the applied write returns 200 with the stored subscription, and
//! an older revision gets 409 with the current one. Filter and message
//! template programs are checked here so a typo fails the request, not every delivery.
//! Deletion is a POST because NIP-98 signs no DELETE method.
use super::{authenticate, invalid, parse_body, store_error};
use crate::filter::{compile_filter, compile_message_template, CompileError};
use crate::http::{error, AppState};
use crate::store::{SubscriptionChange, SubscriptionView, SubscriptionWrite, WriteOutcome};
use axum::{
    body::Bytes,
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post, put},
    Router,
};
use serde::Deserialize;
use uuid::Uuid;

/// Subscription routes, merged into the private management router.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/integrations/{id}/subscriptions",
            get(list_subscriptions),
        )
        .route(
            "/v1/integrations/{id}/subscriptions/{subscription_id}",
            put(put_subscription).get(get_subscription),
        )
        .route(
            "/v1/integrations/{id}/subscriptions/{subscription_id}/delete",
            post(delete_subscription),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PutSubscriptionBody {
    revision: i64,
    channel_id: Uuid,
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    message_template: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteSubscriptionBody {
    revision: i64,
}

fn invalid_program(field: &'static str, e: CompileError) -> Response {
    let body =
        serde_json::json!({ "error": "invalid_program", "field": field, "message": e.to_string() });
    (StatusCode::BAD_REQUEST, Json(body)).into_response()
}

/// Check both programs off the async runtime: the parser runs on a scoped
/// thread with a large stack. `Some` is the response refusing the request.
async fn refuse_programs(
    filter: Option<String>,
    message_template: Option<String>,
) -> Option<Response> {
    let checked = tokio::task::spawn_blocking(move || {
        if let Some(source) = &filter {
            compile_filter(source).map_err(|e| ("filter", e))?;
        }
        if let Some(source) = &message_template {
            compile_message_template(source).map_err(|e| ("message_template", e))?;
        }
        Ok(())
    })
    .await;
    match checked {
        Ok(Ok(())) => None,
        Ok(Err((_, CompileError::Unavailable(_)))) | Err(_) => Some(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
        )),
        Ok(Err((field, e))) => Some(invalid_program(field, e)),
    }
}

fn write_response(outcome: WriteOutcome) -> Response {
    match outcome {
        WriteOutcome::Applied(s) | WriteOutcome::Unchanged(s) => {
            Json(SubscriptionView::from(&s)).into_response()
        }
        WriteOutcome::StaleRevision { current } => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "error": "stale_revision", "current": current })),
        )
            .into_response(),
        WriteOutcome::NotFound => error(StatusCode::NOT_FOUND, "not_found"),
        WriteOutcome::WrongIntegration => error(StatusCode::CONFLICT, "subscription_conflict"),
        WriteOutcome::LimitReached => error(StatusCode::CONFLICT, "subscription_limit"),
    }
}

async fn write(state: &AppState, write: SubscriptionWrite) -> Response {
    if write.revision <= 0 {
        return invalid();
    }
    match state.store.write_subscription(write, (state.now)()).await {
        Ok(outcome) => write_response(outcome),
        Err(e) => store_error(e),
    }
}

async fn put_subscription(
    State(state): State<AppState>,
    Path((id, subscription_id)): Path<(Uuid, Uuid)>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, Some(&body)) {
        return reject.into_response();
    }
    let request: PutSubscriptionBody = match parse_body(&body) {
        Ok(request) => request,
        Err(reject) => return reject.into_response(),
    };
    if let Some(refusal) =
        refuse_programs(request.filter.clone(), request.message_template.clone()).await
    {
        return refusal;
    }
    write(
        &state,
        SubscriptionWrite {
            id: subscription_id,
            integration_id: id,
            revision: request.revision,
            change: SubscriptionChange::Put {
                channel_id: request.channel_id,
                filter: request.filter,
                message_template: request.message_template,
            },
        },
    )
    .await
}

async fn delete_subscription(
    State(state): State<AppState>,
    Path((id, subscription_id)): Path<(Uuid, Uuid)>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, Some(&body)) {
        return reject.into_response();
    }
    let request: DeleteSubscriptionBody = match parse_body(&body) {
        Ok(request) => request,
        Err(reject) => return reject.into_response(),
    };
    write(
        &state,
        SubscriptionWrite {
            id: subscription_id,
            integration_id: id,
            revision: request.revision,
            change: SubscriptionChange::Delete,
        },
    )
    .await
}

async fn get_subscription(
    State(state): State<AppState>,
    Path((id, subscription_id)): Path<(Uuid, Uuid)>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, None) {
        return reject.into_response();
    }
    match state.store.subscription(id, subscription_id).await {
        Ok(Some(s)) => Json(SubscriptionView::from(&s)).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => store_error(e),
    }
}

async fn list_subscriptions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    method: Method,
    uri: OriginalUri,
    headers: HeaderMap,
) -> Response {
    if let Err(reject) = authenticate(&state, &headers, &method, &uri, None) {
        return reject.into_response();
    }
    match state.store.integration(id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "not_found"),
        Err(e) => return store_error(e),
    }
    match state.store.live_subscriptions(id).await {
        Ok(live) => {
            let views: Vec<_> = live.iter().map(SubscriptionView::from).collect();
            Json(serde_json::json!({ "subscriptions": views })).into_response()
        }
        Err(e) => store_error(e),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{call, fixture, put_body};
    use crate::store::MAX_SUBSCRIPTIONS_PER_INTEGRATION;
    use axum::http::StatusCode;
    use nostr::Keys;
    use serde_json::json;
    use uuid::Uuid;

    #[tokio::test]
    async fn relay_manages_subscriptions_by_revision() {
        let fx = fixture();
        let id = Uuid::new_v4();
        let integration = format!("/v1/integrations/{id}");
        let (status, _) = call(
            &fx,
            &fx.relay,
            "PUT",
            &integration,
            &integration,
            Some(put_body(Uuid::new_v4(), 1)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let sub = Uuid::new_v4();
        let channel = Uuid::new_v4();
        let path = format!("{integration}/subscriptions/{sub}");
        let body = json!({
            "revision": 1,
            "channel_id": channel,
            "filter": "payload.action == \"opened\"",
            "message_template": "\"PR: \" + payload.title",
        });
        let (status, created) = call(&fx, &fx.relay, "PUT", &path, &path, Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{created}");
        assert_eq!(created["channel_id"], json!(channel));
        assert_eq!(created["revision"], 1);
        assert_eq!(created["deleted"], false);

        let (status, retried) = call(&fx, &fx.relay, "PUT", &path, &path, Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "a blind retry is idempotent");
        assert_eq!(retried, created);

        let moved = json!({ "revision": 1, "channel_id": Uuid::new_v4() });
        let (status, stale) = call(&fx, &fx.relay, "PUT", &path, &path, Some(moved)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(stale, json!({ "error": "stale_revision", "current": 1 }));

        for (field, bad) in [
            (
                "filter",
                json!({ "revision": 2, "channel_id": channel, "filter": "payload.a ==" }),
            ),
            (
                "filter",
                json!({ "revision": 2, "channel_id": channel, "filter": "1" }),
            ),
            (
                "message_template",
                json!({ "revision": 2, "channel_id": channel, "message_template": "true" }),
            ),
        ] {
            let (status, refused) = call(&fx, &fx.relay, "PUT", &path, &path, Some(bad)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
            assert_eq!(refused["error"], "invalid_program");
            assert_eq!(refused["field"], field);
        }
        let unknown = json!({ "revision": 2, "channel_id": channel, "channel": "x" });
        let (status, _) = call(&fx, &fx.relay, "PUT", &path, &path, Some(unknown)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let list = format!("{integration}/subscriptions");
        let (status, listed) = call(&fx, &fx.relay, "GET", &list, &list, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["subscriptions"], json!([created]));

        let delete = format!("{path}/delete");
        let (status, deleted) = call(
            &fx,
            &fx.relay,
            "POST",
            &delete,
            &delete,
            Some(json!({ "revision": 2 })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(deleted["deleted"], true);
        let (status, stale) = call(&fx, &fx.relay, "PUT", &path, &path, Some(body)).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "a delayed put cannot resurrect it"
        );
        assert_eq!(stale["current"], 2);
        let (_, read) = call(&fx, &fx.relay, "GET", &path, &path, None).await;
        assert_eq!(read["deleted"], true);
        let (_, listed) = call(&fx, &fx.relay, "GET", &list, &list, None).await;
        assert_eq!(listed["subscriptions"], json!([]));

        let other = format!("/v1/integrations/{}/subscriptions/{sub}", Uuid::new_v4());
        let (status, _) = call(
            &fx,
            &fx.relay,
            "PUT",
            &other,
            &other,
            Some(json!({ "revision": 3, "channel_id": channel })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "the integration must exist");

        let stranger = Keys::generate();
        let (status, _) = call(&fx, &stranger, "GET", &list, &list, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn an_integration_holds_a_bounded_number_of_subscriptions() {
        let fx = fixture();
        let id = Uuid::new_v4();
        let integration = format!("/v1/integrations/{id}");
        call(
            &fx,
            &fx.relay,
            "PUT",
            &integration,
            &integration,
            Some(put_body(Uuid::new_v4(), 1)),
        )
        .await;
        let channel = Uuid::new_v4();
        for n in 0..=MAX_SUBSCRIPTIONS_PER_INTEGRATION {
            let path = format!("{integration}/subscriptions/{}", Uuid::new_v4());
            let body = json!({ "revision": 1, "channel_id": channel });
            let (status, response) = call(&fx, &fx.relay, "PUT", &path, &path, Some(body)).await;
            if n < MAX_SUBSCRIPTIONS_PER_INTEGRATION {
                assert_eq!(status, StatusCode::OK, "{response}");
            } else {
                assert_eq!(status, StatusCode::CONFLICT);
                assert_eq!(response["error"], "subscription_limit");
            }
        }
    }
}
