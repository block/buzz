use super::{
    api::{key, B3ApiError},
    RecordDeliveryCommitment,
};
use crate::{api::AppState, security::RequestContext};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    Extension, Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DeliveryQuery {
    #[serde(default)]
    supplier_id: Option<Uuid>,
    #[serde(default = "default_limit")]
    limit: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PerformanceQuery {
    #[serde(default = "default_days")]
    days: i64,
}

fn default_limit() -> i64 {
    200
}

fn default_days() -> i64 {
    90
}

pub(super) async fn deliveries(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Query(query): Query<DeliveryQuery>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .delivery
        .deliveries(context.actor_user_id, query.supplier_id, query.limit)
        .await
        .map(Json)
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}

pub(super) async fn supplier_performance(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Query(query): Query<PerformanceQuery>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .delivery
        .supplier_performance(context.actor_user_id, query.days)
        .await
        .map(Json)
        .map_err(|error| B3ApiError::domain(error, context.trace_id))
}

pub(super) async fn record_commitment(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<RecordDeliveryCommitment>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    state
        .delivery
        .record_commitment(
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
