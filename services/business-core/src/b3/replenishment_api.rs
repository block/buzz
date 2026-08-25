use super::{
    api::{key, B3ApiError},
    ConvertPurchaseRequisition, CreatePurchaseRequisition, UpsertReplenishmentPolicy,
};
use crate::{api::AppState, b2::model::VersionCommand, security::RequestContext};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    200
}

pub(super) async fn options(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .replenishment
        .options(context.actor_user_id)
        .await
        .map(Json)
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}

pub(super) async fn suggestions(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Query(query): Query<LimitQuery>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .replenishment
        .suggestions(context.actor_user_id, query.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b3"}))
        })
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}

pub(super) async fn requisitions(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Query(query): Query<LimitQuery>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .replenishment
        .requisitions(context.actor_user_id, query.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b3"}))
        })
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}

pub(super) async fn upsert_policy(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    headers: HeaderMap,
    Json(input): Json<UpsertReplenishmentPolicy>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .replenishment
        .upsert_policy(
            context.actor_user_id,
            context.trace_id,
            key(&headers, context.trace_id)?,
            &input,
        )
        .await
        .map(Json)
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}

pub(super) async fn create_requisition(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    headers: HeaderMap,
    Json(input): Json<CreatePurchaseRequisition>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .replenishment
        .create_requisition(
            context.actor_user_id,
            context.trace_id,
            key(&headers, context.trace_id)?,
            &input,
        )
        .await
        .map(Json)
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}

pub(super) async fn confirm_requisition(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .replenishment
        .transition_requisition(
            context.actor_user_id,
            context.trace_id,
            id,
            key(&headers, context.trace_id)?,
            &input,
            true,
        )
        .await
        .map(Json)
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}

pub(super) async fn cancel_requisition(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .replenishment
        .transition_requisition(
            context.actor_user_id,
            context.trace_id,
            id,
            key(&headers, context.trace_id)?,
            &input,
            false,
        )
        .await
        .map(Json)
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}

pub(super) async fn convert_requisition(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<ConvertPurchaseRequisition>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .replenishment
        .convert_requisition(
            context.actor_user_id,
            context.trace_id,
            id,
            key(&headers, context.trace_id)?,
            &input,
        )
        .await
        .map(Json)
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}
