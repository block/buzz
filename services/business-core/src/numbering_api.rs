use crate::{
    api::AppState, b2::common::DomainError, numbering::SaveNumberingRule, security::RequestContext,
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, put},
    Extension, Json, Router,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug)]
struct NumberingApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    trace_id: Uuid,
}

impl IntoResponse for NumberingApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"code":self.code,"message":self.message,"traceId":self.trace_id})),
        )
            .into_response()
    }
}

impl NumberingApiError {
    fn domain(error: DomainError, trace_id: Uuid) -> Self {
        match error {
            DomainError::NotFoundOrForbidden => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found_or_forbidden",
                message: "resource was not found or is not accessible".into(),
                trace_id,
            },
            DomainError::VersionConflict => Self {
                status: StatusCode::CONFLICT,
                code: "VERSION_CONFLICT",
                message: "object version changed; refresh and retry".into(),
                trace_id,
            },
            DomainError::IdempotencyConflict => Self {
                status: StatusCode::CONFLICT,
                code: "IDEMPOTENCY_CONFLICT",
                message: "idempotency key was reused with a different request".into(),
                trace_id,
            },
            DomainError::Invalid(message) => Self {
                status: StatusCode::BAD_REQUEST,
                code: "invalid_request",
                message,
                trace_id,
            },
            other => {
                tracing::error!(%trace_id,error=%other,"numbering rule command failed");
                Self {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    code: "service_unavailable",
                    message: "numbering rule command could not be completed".into(),
                    trace_id,
                }
            }
        }
    }

    fn invalid(message: &str, trace_id: Uuid) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: message.into(),
            trace_id,
        }
    }
}

pub fn service_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/numbering-rules", get(list))
        .route("/v1/numbering-ledger", get(ledger))
}

pub fn browser_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/numbering-rules", get(list))
        .route("/api/v1/numbering-ledger", get(ledger))
        .route("/api/v1/numbering-rules/{record_type}", put(save))
}

async fn list(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
) -> Result<Json<impl serde::Serialize>, NumberingApiError> {
    state
        .numbering
        .list(context.actor_user_id)
        .await
        .map(Json)
        .map_err(|error| NumberingApiError::domain(error, context.trace_id))
}

async fn ledger(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
) -> Result<Json<impl serde::Serialize>, NumberingApiError> {
    state
        .numbering
        .ledger(context.actor_user_id)
        .await
        .map(Json)
        .map_err(|error| NumberingApiError::domain(error, context.trace_id))
}

async fn save(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(record_type): Path<String>,
    headers: HeaderMap,
    Json(input): Json<SaveNumberingRule>,
) -> Result<Json<impl serde::Serialize>, NumberingApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            NumberingApiError::invalid("Idempotency-Key is required", context.trace_id)
        })?;
    state
        .numbering
        .save(
            context.actor_user_id,
            context.trace_id,
            &record_type,
            key,
            &input,
        )
        .await
        .map(Json)
        .map_err(|error| NumberingApiError::domain(error, context.trace_id))
}
