use super::model::{
    ApplySupplierPayment, CreateGoodsReceipt, CreatePurchaseOrder, CreateSupplierPayment,
    ReplacePurchaseOrderDraft, ReversePayableAllocation, VersionCommand,
};
use crate::{api::AppState, b2::DomainError, security::RequestContext};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug)]
pub(super) struct B3ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    trace_id: Uuid,
}

impl IntoResponse for B3ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"code":self.code,"message":self.message,"traceId":self.trace_id})),
        )
            .into_response()
    }
}

impl B3ApiError {
    fn simple(status: StatusCode, code: &'static str, message: &str, trace_id: Uuid) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            trace_id,
        }
    }
    pub(super) fn domain(error: DomainError, trace_id: Uuid) -> Self {
        match error {
            DomainError::NotFoundOrForbidden => Self::simple(
                StatusCode::NOT_FOUND,
                "not_found_or_forbidden",
                "resource was not found or is not accessible",
                trace_id,
            ),
            DomainError::VersionConflict => Self::simple(
                StatusCode::CONFLICT,
                "VERSION_CONFLICT",
                "object version changed; refresh and retry",
                trace_id,
            ),
            DomainError::IdempotencyConflict => Self::simple(
                StatusCode::CONFLICT,
                "IDEMPOTENCY_CONFLICT",
                "idempotency key was reused with a different request",
                trace_id,
            ),
            DomainError::OverReceipt => Self::simple(
                StatusCode::CONFLICT,
                "OVER_RECEIPT",
                "receipt exceeds purchase order remainder",
                trace_id,
            ),
            DomainError::OverAllocation => Self::simple(
                StatusCode::CONFLICT,
                "OVER_ALLOCATION",
                "allocation exceeds an open balance",
                trace_id,
            ),
            DomainError::SubsequentInventoryMovementsExist => Self::simple(
                StatusCode::CONFLICT,
                "SUBSEQUENT_INVENTORY_MOVEMENTS_EXIST",
                "receipt has later inventory movements",
                trace_id,
            ),
            DomainError::PayableAlreadySettled => Self::simple(
                StatusCode::CONFLICT,
                "PAYABLE_ALREADY_SETTLED",
                "reverse payable allocations first",
                trace_id,
            ),
            DomainError::Invalid(message) => Self::simple(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                &message,
                trace_id,
            ),
            DomainError::Database(error) => {
                if error.as_database_error().is_some_and(|database_error| {
                    database_error.message() == "inventory count scope is frozen"
                }) {
                    return Self::simple(
                        StatusCode::CONFLICT,
                        "INVENTORY_COUNT_SCOPE_FROZEN",
                        "inventory changes are blocked while this SKU is being counted",
                        trace_id,
                    );
                }
                tracing::error!(%trace_id,error=%error,"B3 database command failed");
                Self::simple(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service_unavailable",
                    "business command could not be completed",
                    trace_id,
                )
            }
            DomainError::Serialization(error) => {
                tracing::error!(%trace_id,error=%error,"B3 serialization failed");
                Self::simple(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "business command could not be completed",
                    trace_id,
                )
            }
            other => Self::simple(
                StatusCode::CONFLICT,
                "business_rule_conflict",
                &other.to_string(),
                trace_id,
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ListQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    supplier_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PurchaseOrderEntryQuery {
    #[serde(default)]
    order_id: Option<Uuid>,
}

fn default_limit() -> i64 {
    100
}

pub fn service_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/agent-drafts/purchase-orders", post(create_order))
        .route("/v1/agent-drafts/goods-receipts", post(create_receipt))
        .route("/v1/agent-drafts/supplier-payments", post(create_payment))
        .route("/v1/purchase-orders", get(list_orders))
        .route("/v1/goods-receipts", get(list_receipts))
        .route("/v1/trade-payables", get(list_payables))
        .route("/v1/supplier-payments", get(list_payments))
        .route("/v1/reconciliation/payables", get(reconcile_payables))
        .route(
            "/v1/replenishment-suggestions",
            get(super::replenishment_api::suggestions),
        )
        .route(
            "/v1/purchase-requisitions",
            get(super::replenishment_api::requisitions),
        )
        .route(
            "/v1/purchase-deliveries",
            get(super::delivery_api::deliveries),
        )
        .route(
            "/v1/supplier-delivery-performance",
            get(super::delivery_api::supplier_performance),
        )
}

pub fn browser_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/v1/purchase-orders",
            get(list_orders).post(create_order),
        )
        .route(
            "/api/v1/purchase-orders/entry-options",
            get(purchase_order_entry_options),
        )
        .route("/api/v1/purchase-orders/{id}", put(replace_order))
        .route(
            "/api/v1/purchase-orders/{id}/confirmation-preview",
            get(purchase_order_confirmation_preview),
        )
        .route("/api/v1/purchase-orders/{id}/confirm", post(confirm_order))
        .route(
            "/api/v1/purchase-orders/{id}/cancel-remaining",
            post(cancel_remaining),
        )
        .route(
            "/api/v1/goods-receipts",
            get(list_receipts).post(create_receipt),
        )
        .route(
            "/api/v1/goods-receipts/draft-options",
            get(goods_receipt_draft_options),
        )
        .route(
            "/api/v1/goods-receipts/{id}/confirmation-preview",
            get(goods_receipt_confirmation_preview),
        )
        .route("/api/v1/goods-receipts/{id}/confirm", post(confirm_receipt))
        .route("/api/v1/goods-receipts/{id}/reverse", post(reverse_receipt))
        .route("/api/v1/trade-payables", get(list_payables))
        .route(
            "/api/v1/supplier-payments",
            get(list_payments).post(create_payment),
        )
        .route(
            "/api/v1/supplier-payments/{id}/confirm",
            post(confirm_payment),
        )
        .route(
            "/api/v1/supplier-payments/{id}/allocations",
            post(apply_payment),
        )
        .route(
            "/api/v1/payable-allocations/{id}/reverse",
            post(reverse_allocation),
        )
        .route(
            "/api/v1/supplier-payments/{id}/reverse",
            post(reverse_payment),
        )
        .route("/api/v1/reconciliation/payables", get(reconcile_payables))
        .route(
            "/api/v1/replenishment-options",
            get(super::replenishment_api::options),
        )
        .route(
            "/api/v1/replenishment-suggestions",
            get(super::replenishment_api::suggestions),
        )
        .route(
            "/api/v1/replenishment-policies",
            post(super::replenishment_api::upsert_policy),
        )
        .route(
            "/api/v1/purchase-requisitions",
            get(super::replenishment_api::requisitions)
                .post(super::replenishment_api::create_requisition),
        )
        .route(
            "/api/v1/purchase-requisitions/{id}/confirm",
            post(super::replenishment_api::confirm_requisition),
        )
        .route(
            "/api/v1/purchase-requisitions/{id}/cancel",
            post(super::replenishment_api::cancel_requisition),
        )
        .route(
            "/api/v1/purchase-requisitions/{id}/convert",
            post(super::replenishment_api::convert_requisition),
        )
        .route(
            "/api/v1/purchase-deliveries",
            get(super::delivery_api::deliveries),
        )
        .route(
            "/api/v1/purchase-orders/{id}/delivery-commitments",
            post(super::delivery_api::record_commitment),
        )
        .route(
            "/api/v1/supplier-delivery-performance",
            get(super::delivery_api::supplier_performance),
        )
}

pub(super) fn key(headers: &HeaderMap, trace_id: Uuid) -> Result<&str, B3ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            B3ApiError::simple(
                StatusCode::BAD_REQUEST,
                "idempotency_key_required",
                "Idempotency-Key is required",
                trace_id,
            )
        })
}

fn enabled(state: &AppState, index: usize, trace_id: Uuid) -> Result<(), B3ApiError> {
    if state.b3_enabled[index] {
        Ok(())
    } else {
        Err(B3ApiError::simple(
            StatusCode::SERVICE_UNAVAILABLE,
            "feature_disabled",
            "Business Core B3 module is disabled",
            trace_id,
        ))
    }
}

async fn list_orders(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, B3ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.purchasing
        .orders(c.actor_user_id, q.supplier_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b3"}))
        })
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn create_order(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreatePurchaseOrder>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.purchasing
        .create_order(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn purchase_order_entry_options(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<PurchaseOrderEntryQuery>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.purchasing
        .entry_options(c.actor_user_id, q.order_id)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn replace_order(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<ReplacePurchaseOrderDraft>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.purchasing
        .replace_draft(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn confirm_order(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.purchasing
        .confirm_order(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn purchase_order_confirmation_preview(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.purchasing
        .confirmation_preview(c.actor_user_id, id)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn cancel_remaining(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.purchasing
        .cancel_remaining(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn list_receipts(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, B3ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.receiving.receipts(c.actor_user_id,q.supplier_id,q.limit).await.map(|items|Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b3","costStatus":"provisional"}))).map_err(|e|B3ApiError::domain(e,c.trace_id))
}
async fn create_receipt(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateGoodsReceipt>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.receiving
        .create_receipt(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn goods_receipt_draft_options(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.receiving
        .draft_options(c.actor_user_id, q.limit)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn goods_receipt_confirmation_preview(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.receiving
        .confirmation_preview(c.actor_user_id, id)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn confirm_receipt(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.receiving
        .confirm_receipt(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn reverse_receipt(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.receiving
        .reverse_receipt(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn list_payables(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, B3ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.payables.payables(c.actor_user_id,q.supplier_id,q.limit).await.map(|items|Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b3","authority":"operational_payable","unsupported":["supplier_invoice","three_way_match","general_ledger"]}))).map_err(|e|B3ApiError::domain(e,c.trace_id))
}
async fn list_payments(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, B3ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.payables
        .payments(c.actor_user_id, q.supplier_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b3"}))
        })
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn create_payment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateSupplierPayment>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.payables
        .create_payment(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn confirm_payment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.payables
        .confirm_payment(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn apply_payment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<ApplySupplierPayment>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.payables
        .apply_payment(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn reverse_allocation(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<ReversePayableAllocation>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.payables
        .reverse_allocation(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn reverse_payment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B3ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.payables
        .reverse_payment(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
async fn reconcile_payables(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
) -> Result<Json<serde_json::Value>, B3ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.payables
        .reconcile(c.actor_user_id)
        .await
        .map(Json)
        .map_err(|e| B3ApiError::domain(e, c.trace_id))
}
