use super::{
    api::{key, B2ApiError},
    AcknowledgePurchaseReturn, DispatchPurchaseReturn, InspectSalesReturn,
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
pub(super) struct ReturnAnalyticsQuery {
    period: String,
    #[serde(default = "default_currency")]
    currency: String,
}

fn default_currency() -> String {
    "CNY".into()
}

pub(super) async fn sales_return_inspection(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.return_disposition
        .sales_inspection(c.actor_user_id, id)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}

pub(super) async fn inspect_sales_return(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<InspectSalesReturn>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.return_disposition
        .inspect_sales_return(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}

pub(super) async fn dispatch_purchase_return(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<DispatchPurchaseReturn>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.return_disposition
        .dispatch_purchase_return(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}

pub(super) async fn acknowledge_purchase_return(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<AcknowledgePurchaseReturn>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.return_disposition
        .acknowledge_purchase_return(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}

pub(super) async fn return_analytics(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ReturnAnalyticsQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.return_disposition
        .analytics(c.actor_user_id, &q.period, &q.currency)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
