use super::{
    api::{key, B2ApiError, ListQuery},
    CreateInventoryCount, SubmitInventoryCount,
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
pub(super) struct AgingQuery {
    #[serde(default = "default_threshold")]
    threshold_days: i32,
    #[serde(default = "default_limit")]
    limit: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct TurnoverQuery {
    period: String,
    #[serde(default = "default_currency")]
    currency: String,
}

fn default_threshold() -> i32 {
    90
}
fn default_limit() -> i64 {
    200
}
fn default_currency() -> String {
    "CNY".into()
}

pub(super) async fn inventory_count_options(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory_count
        .options(c.actor_user_id)
        .await
        .map(|items| Json(json!({"items":items})))
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
pub(super) async fn list_inventory_counts(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory_count
        .list(c.actor_user_id, q.limit)
        .await
        .map(|items| Json(json!({"items":items})))
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
pub(super) async fn get_inventory_count(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory_count
        .detail(c.actor_user_id, id)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
pub(super) async fn create_inventory_count(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateInventoryCount>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory_count
        .create(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
pub(super) async fn submit_inventory_count(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<SubmitInventoryCount>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory_count
        .submit(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
pub(super) async fn post_inventory_count(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory_count
        .post(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
pub(super) async fn cancel_inventory_count(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory_count
        .cancel(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
pub(super) async fn inventory_aging(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<AgingQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory_count
        .aging(c.actor_user_id, q.threshold_days, q.limit)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
pub(super) async fn inventory_turnover(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<TurnoverQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory_count
        .turnover(c.actor_user_id, &q.period, &q.currency)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
