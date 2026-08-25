use crate::{
    api::AppState,
    b2::common::DomainError,
    master_data::{ChangeCoreMasterStatus, CoreMasterType, SaveCoreMasterData},
    security::RequestContext,
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::{str::FromStr, sync::Arc};
use uuid::Uuid;

#[derive(Debug)]
struct MasterApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    trace_id: Uuid,
}

impl IntoResponse for MasterApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"code":self.code,"message":self.message,"traceId":self.trace_id})),
        )
            .into_response()
    }
}

impl MasterApiError {
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
                tracing::error!(%trace_id,error=%other,"core master data command failed");
                Self {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    code: "service_unavailable",
                    message: "master data command could not be completed".into(),
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MasterQuery {
    #[serde(default)]
    resource_type: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    500
}

pub fn service_routes() -> Router<Arc<AppState>> {
    Router::new().route("/v1/core-master-data", get(list))
}

pub fn browser_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/core-master-data", get(list).post(create))
        .route("/api/v1/core-master-data/{resource_type}/{id}", put(update))
        .route(
            "/api/v1/core-master-data/{resource_type}/{id}/disable-impact",
            get(impact),
        )
        .route(
            "/api/v1/core-master-data/{resource_type}/{id}/status",
            post(change_status),
        )
}

async fn list(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Query(query): Query<MasterQuery>,
) -> Result<Json<impl serde::Serialize>, MasterApiError> {
    let kind = query
        .resource_type
        .as_deref()
        .map(CoreMasterType::from_str)
        .transpose()
        .map_err(|error| MasterApiError::domain(error, context.trace_id))?;
    state
        .master_data
        .list(context.actor_user_id, kind, query.limit)
        .await
        .map(Json)
        .map_err(|error| MasterApiError::domain(error, context.trace_id))
}
async fn create(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    headers: HeaderMap,
    Json(input): Json<SaveCoreMasterData>,
) -> Result<Json<impl serde::Serialize>, MasterApiError> {
    state
        .master_data
        .save(
            context.actor_user_id,
            context.trace_id,
            None,
            key(&headers, context.trace_id)?,
            &input,
        )
        .await
        .map(Json)
        .map_err(|error| MasterApiError::domain(error, context.trace_id))
}
async fn update(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path((resource_type, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<SaveCoreMasterData>,
) -> Result<Json<impl serde::Serialize>, MasterApiError> {
    if input.resource_type != resource_type {
        return Err(MasterApiError::invalid(
            "resourceType does not match path",
            context.trace_id,
        ));
    }
    state
        .master_data
        .save(
            context.actor_user_id,
            context.trace_id,
            Some(id),
            key(&headers, context.trace_id)?,
            &input,
        )
        .await
        .map(Json)
        .map_err(|error| MasterApiError::domain(error, context.trace_id))
}
async fn impact(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path((resource_type, id)): Path<(String, Uuid)>,
) -> Result<Json<impl serde::Serialize>, MasterApiError> {
    let kind = CoreMasterType::from_str(&resource_type)
        .map_err(|error| MasterApiError::domain(error, context.trace_id))?;
    state
        .master_data
        .impact(context.actor_user_id, kind, id)
        .await
        .map(Json)
        .map_err(|error| MasterApiError::domain(error, context.trace_id))
}
async fn change_status(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path((resource_type, id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Json(input): Json<ChangeCoreMasterStatus>,
) -> Result<Json<impl serde::Serialize>, MasterApiError> {
    let kind = CoreMasterType::from_str(&resource_type)
        .map_err(|error| MasterApiError::domain(error, context.trace_id))?;
    state
        .master_data
        .change_status(
            context.actor_user_id,
            context.trace_id,
            kind,
            id,
            key(&headers, context.trace_id)?,
            &input,
        )
        .await
        .map(Json)
        .map_err(|error| MasterApiError::domain(error, context.trace_id))
}
fn key(headers: &HeaderMap, trace_id: Uuid) -> Result<&str, MasterApiError> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| MasterApiError::invalid("Idempotency-Key is required", trace_id))
}
