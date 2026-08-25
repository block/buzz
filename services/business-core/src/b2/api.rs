use super::{
    inventory_count_api::{
        cancel_inventory_count, create_inventory_count, get_inventory_count, inventory_aging,
        inventory_count_options, inventory_turnover, list_inventory_counts, post_inventory_count,
        submit_inventory_count,
    },
    model::{
        ApplyReceipt, CreateCustomerReceipt, CreateInventoryOpening, CreateSalesOrder,
        CreateShipment, ReplaceSalesOrderDraft, ReverseAllocation, VersionCommand,
    },
    return_disposition_api::{
        acknowledge_purchase_return, dispatch_purchase_return, inspect_sales_return,
        return_analytics, sales_return_inspection,
    },
    CreateReturn, DomainError,
};
use crate::{api::AppState, security::RequestContext};
use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use uuid::Uuid;

#[derive(Debug)]
pub(super) struct B2ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    trace_id: Uuid,
    details: Option<serde_json::Value>,
}
impl IntoResponse for B2ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({"code":self.code,"message":self.message,"traceId":self.trace_id});
        if let Some(details) = self.details {
            body["details"] = details;
        }
        (self.status, Json(body)).into_response()
    }
}
impl B2ApiError {
    fn auth(status: StatusCode, code: &'static str) -> Self {
        Self {
            status,
            code,
            message: code.replace('_', " "),
            trace_id: Uuid::new_v4(),
            details: None,
        }
    }
    fn simple(
        status: StatusCode,
        code: &'static str,
        message: &'static str,
        trace_id: Uuid,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            trace_id,
            details: None,
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
            DomainError::InsufficientStock(details) => Self {
                status: StatusCode::CONFLICT,
                code: "INSUFFICIENT_STOCK",
                message: "full inventory reservation is not available".into(),
                trace_id,
                details: Some(details),
            },
            DomainError::MissingInventoryCost => Self::simple(
                StatusCode::CONFLICT,
                "MISSING_INVENTORY_COST",
                "inventory cost is required before shipment confirmation",
                trace_id,
            ),
            DomainError::OrderOnHold => Self::simple(
                StatusCode::CONFLICT,
                "ORDER_ON_MANUAL_REVIEW_HOLD",
                "shipment confirmation is blocked while the order is on hold",
                trace_id,
            ),
            DomainError::ReceivableAlreadySettled => Self::simple(
                StatusCode::CONFLICT,
                "RECEIVABLE_ALREADY_SETTLED",
                "reverse allocations before reversing the shipment",
                trace_id,
            ),
            DomainError::OverAllocation => Self::simple(
                StatusCode::CONFLICT,
                "OVER_ALLOCATION",
                "allocation exceeds an open balance",
                trace_id,
            ),
            DomainError::OverReceipt
            | DomainError::SubsequentInventoryMovementsExist
            | DomainError::PayableAlreadySettled
            | DomainError::StalePreview => Self::simple(
                StatusCode::CONFLICT,
                "business_rule_conflict",
                "business rule conflict",
                trace_id,
            ),
            DomainError::Invalid(message) => Self {
                status: StatusCode::BAD_REQUEST,
                code: "invalid_request",
                message,
                trace_id,
                details: None,
            },
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
                tracing::error!(%trace_id,error=%error,"B2 database command failed");
                Self::simple(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service_unavailable",
                    "business command could not be completed",
                    trace_id,
                )
            }
            DomainError::Serialization(error) => {
                tracing::error!(%trace_id,error=%error,"B2 serialization failed");
                Self::simple(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "business command could not be completed",
                    trace_id,
                )
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ListQuery {
    #[serde(default = "default_limit")]
    pub(super) limit: i64,
    #[serde(default)]
    sku_id: Option<Uuid>,
    #[serde(default)]
    customer_id: Option<Uuid>,
    #[serde(default)]
    shipment_id: Option<Uuid>,
}
fn default_limit() -> i64 {
    100
}

pub fn service_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/sales-orders", get(list_orders))
        .route("/v1/inventory-balances", get(list_inventory))
        .route("/v1/inventory-openings", get(list_openings))
        .route("/v1/inventory-movements", get(list_movements))
        .route("/v1/inventory-counts", get(list_inventory_counts))
        .route("/v1/inventory-aging", get(inventory_aging))
        .route("/v1/inventory-turnover", get(inventory_turnover))
        .route("/v1/shipments", get(list_shipments))
        .route("/v1/trade-receivables", get(list_receivables))
        .route("/v1/customer-receipts", get(list_receipts))
        .route("/v1/customer-receipts/{id}", get(get_receipt))
        .route("/v1/sales-returns", get(list_sales_returns))
        .route("/v1/purchase-returns", get(list_purchase_returns))
        .route("/v1/reconciliation/inventory", get(reconcile_inventory))
        .route("/v1/reconciliation/receivables", get(reconcile_receivables))
}
pub fn browser_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/v1/master-data/{resource_type}",
            get(crate::api::list_master_data),
        )
        .route(
            "/api/v1/master-data/{resource_type}/{id}",
            get(crate::api::get_master_data),
        )
        .route("/api/v1/sales-orders", get(list_orders).post(create_order))
        .route("/api/v1/sales-orders/{id}", put(replace_order))
        .route("/api/v1/agent-query-runs", get(list_agent_query_runs))
        .route("/api/v1/agent-query-runs/{id}", get(get_agent_query_run))
        .route(
            "/api/v1/sales-orders/{id}/confirmation-preview",
            get(confirmation_preview),
        )
        .route("/api/v1/sales-orders/{id}/confirm", post(confirm_order))
        .route(
            "/api/v1/sales-orders/{id}/manual-review-hold",
            post(place_hold),
        )
        .route(
            "/api/v1/sales-orders/{id}/release-manual-review-hold",
            post(release_hold),
        )
        .route(
            "/api/v1/sales-orders/{id}/cancel-remaining",
            post(cancel_remaining),
        )
        .route(
            "/api/v1/inventory-openings",
            get(list_openings).post(create_opening),
        )
        .route("/api/v1/inventory-openings/{id}/post", post(post_opening))
        .route(
            "/api/v1/inventory-openings/{id}/reverse",
            post(reverse_opening),
        )
        .route("/api/v1/inventory-balances", get(list_inventory))
        .route("/api/v1/inventory-movements", get(list_movements))
        .route(
            "/api/v1/inventory-counts",
            get(list_inventory_counts).post(create_inventory_count),
        )
        .route(
            "/api/v1/inventory-counts/options",
            get(inventory_count_options),
        )
        .route("/api/v1/inventory-counts/{id}", get(get_inventory_count))
        .route(
            "/api/v1/inventory-counts/{id}/submit",
            post(submit_inventory_count),
        )
        .route(
            "/api/v1/inventory-counts/{id}/post",
            post(post_inventory_count),
        )
        .route(
            "/api/v1/inventory-counts/{id}/cancel",
            post(cancel_inventory_count),
        )
        .route("/api/v1/inventory-aging", get(inventory_aging))
        .route("/api/v1/inventory-turnover", get(inventory_turnover))
        .route(
            "/api/v1/shipments",
            get(list_shipments).post(create_shipment),
        )
        .route(
            "/api/v1/shipments/draft-options",
            get(shipment_draft_options),
        )
        .route(
            "/api/v1/shipments/{id}/confirmation-preview",
            get(shipment_confirmation_preview),
        )
        .route("/api/v1/shipments/{id}/confirm", post(confirm_shipment))
        .route("/api/v1/shipments/{id}/reverse", post(reverse_shipment))
        .route(
            "/api/v1/sales-returns",
            get(list_sales_returns).post(create_sales_return),
        )
        .route("/api/v1/sales-returns/options", get(sales_return_options))
        .route(
            "/api/v1/sales-returns/{id}/confirm",
            post(confirm_sales_return),
        )
        .route(
            "/api/v1/sales-returns/{id}/cancel",
            post(cancel_sales_return),
        )
        .route(
            "/api/v1/sales-returns/{id}/inspection",
            get(sales_return_inspection).post(inspect_sales_return),
        )
        .route(
            "/api/v1/purchase-returns",
            get(list_purchase_returns).post(create_purchase_return),
        )
        .route(
            "/api/v1/purchase-returns/options",
            get(purchase_return_options),
        )
        .route(
            "/api/v1/purchase-returns/{id}/confirm",
            post(confirm_purchase_return),
        )
        .route(
            "/api/v1/purchase-returns/{id}/cancel",
            post(cancel_purchase_return),
        )
        .route(
            "/api/v1/purchase-returns/{id}/dispatch",
            post(dispatch_purchase_return),
        )
        .route(
            "/api/v1/purchase-returns/{id}/supplier-acknowledge",
            post(acknowledge_purchase_return),
        )
        .route("/api/v1/return-analytics", get(return_analytics))
        .route("/api/v1/trade-receivables", get(list_receivables))
        .route(
            "/api/v1/customer-receipts",
            get(list_receipts).post(create_receipt),
        )
        .route("/api/v1/customer-receipts/{id}", get(get_receipt))
        .route(
            "/api/v1/customer-receipts/{id}/confirm",
            post(confirm_receipt),
        )
        .route(
            "/api/v1/customer-receipts/{id}/allocations",
            post(apply_receipt),
        )
        .route(
            "/api/v1/receivable-allocations/{id}/reverse",
            post(reverse_allocation),
        )
        .route(
            "/api/v1/customer-receipts/{id}/reverse",
            post(reverse_receipt),
        )
        .route("/api/v1/reconciliation/inventory", get(reconcile_inventory))
        .route(
            "/api/v1/reconciliation/receivables",
            get(reconcile_receivables),
        )
        .merge(crate::b3::api::browser_routes())
        .merge(crate::b4::api::browser_routes())
        .merge(crate::s1::api::browser_routes())
        .merge(crate::master_data_api::browser_routes())
        .merge(crate::product_master_api::browser_routes())
        .merge(crate::numbering_api::browser_routes())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            business_session_auth,
        ))
        .with_state(state)
}

#[derive(Debug, sqlx::FromRow)]
struct AgentQueryAuditRow {
    occurred_at: DateTime<Utc>,
    event_type: String,
    result: String,
    reason_code: Option<String>,
    tool_name: Option<String>,
    result_count: Option<i32>,
    resource_ref_count: Option<i32>,
    duration_ms: Option<i64>,
    source_buzz_event_id: Option<String>,
    response_buzz_event_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentQueryStage {
    event_type: String,
    result: String,
    reason_code: Option<String>,
    occurred_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentQueryRun {
    trace_id: Uuid,
    status: &'static str,
    tool_name: Option<String>,
    result_count: i32,
    resource_ref_count: i32,
    duration_ms: Option<i64>,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
    source_buzz_event_id: Option<String>,
    response_buzz_event_id: Option<String>,
    stages: Vec<AgentQueryStage>,
}

#[derive(Debug, sqlx::FromRow)]
struct AgentQuerySummaryRow {
    trace_id: Uuid,
    failed: bool,
    response_emitted: bool,
    tool_succeeded: bool,
    tool_name: Option<String>,
    result_count: Option<i32>,
    resource_ref_count: Option<i32>,
    duration_ms: Option<i64>,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentQueryRunSummary {
    trace_id: Uuid,
    status: &'static str,
    tool_name: Option<String>,
    result_count: i32,
    resource_ref_count: i32,
    duration_ms: Option<i64>,
    started_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentQueryRunList {
    items: Vec<AgentQueryRunSummary>,
    data_as_of: DateTime<Utc>,
}

fn query_status(failed: bool, response_emitted: bool, tool_succeeded: bool) -> &'static str {
    if failed {
        "failed"
    } else if response_emitted {
        "complete"
    } else if tool_succeeded {
        "query_complete"
    } else {
        "running"
    }
}

async fn list_agent_query_runs(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
) -> Result<Json<AgentQueryRunList>, B2ApiError> {
    let rows = sqlx::query_as::<_, AgentQuerySummaryRow>(
        "SELECT trace_id,
                bool_or(result='failure') AS failed,
                bool_or(event_type='AGENT_BUSINESS_RESPONSE_EMITTED') AS response_emitted,
                bool_or(event_type IN ('BUSINESS_MCP_TOOL_SUCCEEDED','BUSINESS_READ_PARTIAL_RESULT')) AS tool_succeeded,
                max(tool_name) FILTER (WHERE tool_name <> 'buzz_response') AS tool_name,
                max(result_count) FILTER (WHERE event_type IN ('BUSINESS_MCP_TOOL_SUCCEEDED','BUSINESS_READ_PARTIAL_RESULT')) AS result_count,
                max(resource_ref_count) FILTER (WHERE event_type IN ('BUSINESS_MCP_TOOL_SUCCEEDED','BUSINESS_READ_PARTIAL_RESULT')) AS resource_ref_count,
                max(duration_ms) FILTER (WHERE event_type IN ('BUSINESS_MCP_TOOL_SUCCEEDED','BUSINESS_READ_PARTIAL_RESULT')) AS duration_ms,
                min(occurred_at) AS started_at,
                max(occurred_at) AS completed_at
         FROM security_audit_events
         WHERE enterprise_user_id=$1
           AND (delegation_id IS NOT NULL OR event_type LIKE 'AGENT_%' OR event_type LIKE 'BUSINESS_MCP_%')
         GROUP BY trace_id
         ORDER BY max(occurred_at) DESC
         LIMIT 20",
    )
    .bind(context.actor_user_id)
    .fetch_all(state.store.pool())
    .await
    .map_err(|_| B2ApiError::auth(StatusCode::SERVICE_UNAVAILABLE, "service_unavailable"))?;
    Ok(Json(AgentQueryRunList {
        items: rows
            .into_iter()
            .map(|row| AgentQueryRunSummary {
                trace_id: row.trace_id,
                status: query_status(row.failed, row.response_emitted, row.tool_succeeded),
                tool_name: row.tool_name,
                result_count: row.result_count.unwrap_or(0),
                resource_ref_count: row.resource_ref_count.unwrap_or(0),
                duration_ms: row.duration_ms,
                started_at: row.started_at,
                completed_at: row.completed_at,
            })
            .collect(),
        data_as_of: Utc::now(),
    }))
}

async fn get_agent_query_run(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(trace_id): Path<Uuid>,
) -> Result<Json<AgentQueryRun>, B2ApiError> {
    let rows = sqlx::query_as::<_, AgentQueryAuditRow>(
        "SELECT occurred_at,event_type,result,reason_code,tool_name,result_count,resource_ref_count,duration_ms,source_buzz_event_id,response_buzz_event_id
         FROM security_audit_events
         WHERE trace_id=$1 AND enterprise_user_id=$2
           AND (delegation_id IS NOT NULL OR event_type LIKE 'AGENT_%' OR event_type LIKE 'BUSINESS_MCP_%')
         ORDER BY occurred_at,id",
    )
    .bind(trace_id)
    .bind(context.actor_user_id)
    .fetch_all(state.store.pool())
    .await
    .map_err(|_| B2ApiError::auth(StatusCode::SERVICE_UNAVAILABLE, "service_unavailable"))?;
    let Some(first) = rows.first() else {
        return Err(B2ApiError::simple(
            StatusCode::NOT_FOUND,
            "not_found_or_forbidden",
            "query record was not found or is not accessible",
            context.trace_id,
        ));
    };
    let started_at = first.occurred_at;
    let completed_at = rows
        .last()
        .map(|row| row.occurred_at)
        .unwrap_or(first.occurred_at);
    let failed = rows.iter().any(|row| row.result == "failure");
    let response_emitted = rows
        .iter()
        .any(|row| row.event_type == "AGENT_BUSINESS_RESPONSE_EMITTED");
    let tool_succeeded = rows.iter().any(|row| {
        matches!(
            row.event_type.as_str(),
            "BUSINESS_MCP_TOOL_SUCCEEDED" | "BUSINESS_READ_PARTIAL_RESULT"
        )
    });
    let status = query_status(failed, response_emitted, tool_succeeded);
    let tool_row = rows.iter().rev().find(|row| {
        row.tool_name
            .as_deref()
            .is_some_and(|tool| tool != "buzz_response")
    });
    let source_buzz_event_id = rows.iter().find_map(|row| row.source_buzz_event_id.clone());
    let response_buzz_event_id = rows
        .iter()
        .rev()
        .find_map(|row| row.response_buzz_event_id.clone());
    let tool_name = tool_row.and_then(|row| row.tool_name.clone());
    let result_count = tool_row.and_then(|row| row.result_count).unwrap_or(0);
    let resource_ref_count = tool_row.and_then(|row| row.resource_ref_count).unwrap_or(0);
    let duration_ms = tool_row.and_then(|row| row.duration_ms);
    let stages = rows
        .into_iter()
        .map(|row| AgentQueryStage {
            event_type: row.event_type,
            result: row.result,
            reason_code: row.reason_code,
            occurred_at: row.occurred_at,
        })
        .collect();
    Ok(Json(AgentQueryRun {
        trace_id,
        status,
        tool_name,
        result_count,
        resource_ref_count,
        duration_ms,
        started_at,
        completed_at,
        source_buzz_event_id,
        response_buzz_event_id,
        stages,
    }))
}

async fn business_session_auth(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, B2ApiError> {
    let token = cookie(request.headers(), &state.business_session_cookie_name)
        .ok_or_else(|| B2ApiError::auth(StatusCode::UNAUTHORIZED, "business_session_required"))?;
    let row=sqlx::query("SELECT s.enterprise_user_id,s.csrf_token_hash,s.trace_id FROM business_sessions s JOIN enterprise_users u ON u.id=s.enterprise_user_id JOIN buzz_identity_bindings b ON b.id=s.identity_binding_id JOIN workbench_sessions w ON w.id=s.workbench_session_id WHERE s.session_token_hash=$1 AND s.status='active' AND s.expires_at>now() AND u.status='active' AND b.status='active' AND w.status='active' AND w.expires_at>now()")
  .bind(business_auth_gateway::security::hash(&token)).fetch_optional(state.store.pool()).await.map_err(|_|B2ApiError::auth(StatusCode::SERVICE_UNAVAILABLE,"service_unavailable"))?.ok_or_else(||B2ApiError::auth(StatusCode::UNAUTHORIZED,"business_session_invalid"))?;
    let actor: Uuid = row.get("enterprise_user_id");
    let trace_id = request
        .headers()
        .get("x-trace-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .unwrap_or_else(|| row.get("trace_id"));
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .filter(|origin| {
                state
                    .business_web_origins
                    .iter()
                    .any(|allowed| allowed == origin)
            })
            .ok_or_else(|| B2ApiError::auth(StatusCode::FORBIDDEN, "origin_rejected"))?;
        let csrf = request
            .headers()
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| B2ApiError::auth(StatusCode::FORBIDDEN, "csrf_required"))?;
        let expected: Vec<u8> = row.get("csrf_token_hash");
        if !bool::from(
            expected
                .as_slice()
                .ct_eq(business_auth_gateway::security::hash(csrf).as_slice()),
        ) {
            return Err(B2ApiError::auth(StatusCode::FORBIDDEN, "csrf_rejected"));
        }
        let minute = chrono::Utc::now().timestamp() / 60;
        let mut rate = state.command_rate.lock().map_err(|_| {
            B2ApiError::auth(StatusCode::SERVICE_UNAVAILABLE, "service_unavailable")
        })?;
        rate.retain(|(_, bucket), _| *bucket >= minute - 1);
        let used = rate.entry((actor, minute)).or_default();
        if *used >= state.command_rate_limit_per_minute {
            return Err(B2ApiError::auth(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
            ));
        }
        *used += 1;
    }
    request.extensions_mut().insert(RequestContext {
        actor_user_id: actor,
        trace_id,
    });
    Ok(next.run(request).await)
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|value| value.trim().split_once('='))
        .find(|(cookie_name, _)| *cookie_name == name)
        .map(|(_, value)| value.to_string())
}
pub(super) fn key(headers: &HeaderMap, trace_id: Uuid) -> Result<&str, B2ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            B2ApiError::simple(
                StatusCode::BAD_REQUEST,
                "idempotency_key_required",
                "Idempotency-Key is required",
                trace_id,
            )
        })
}

async fn create_order(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateSalesOrder>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.sales
        .create_order(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn replace_order(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<ReplaceSalesOrderDraft>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.sales
        .replace_order_draft(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn confirm_order(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .confirm_order(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn confirmation_preview(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .confirmation_preview(c.actor_user_id, id)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn place_hold(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .set_hold(
            c.actor_user_id,
            c.trace_id,
            id,
            key(&h, c.trace_id)?,
            &i,
            true,
        )
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn release_hold(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .set_hold(
            c.actor_user_id,
            c.trace_id,
            id,
            key(&h, c.trace_id)?,
            &i,
            false,
        )
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn cancel_remaining(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .cancel_remaining(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn list_orders(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .list_orders(c.actor_user_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b2"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn create_opening(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateInventoryOpening>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory
        .create_opening(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn post_opening(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory
        .post_opening(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn reverse_opening(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory
        .reverse_opening(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn list_inventory(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory
        .balances(c.actor_user_id, q.sku_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b2"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn list_openings(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory
        .openings(c.actor_user_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b2"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn list_movements(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory
        .movements(c.actor_user_id, q.sku_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b2"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn list_shipments(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .shipments(c.actor_user_id, q.shipment_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b2"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn create_shipment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateShipment>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .create_shipment(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn shipment_draft_options(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .shipment_draft_options(c.actor_user_id, q.limit)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn shipment_confirmation_preview(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.sales
        .shipment_confirmation_preview(c.actor_user_id, id)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn confirm_shipment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory
        .confirm_shipment(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn reverse_shipment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory
        .reverse_shipment(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn list_receivables(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.settlement
        .receivables(c.actor_user_id, q.customer_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b2"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn create_receipt(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateCustomerReceipt>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.settlement
        .create_receipt(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn list_receipts(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.settlement
        .receipts(c.actor_user_id, q.customer_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b2"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn get_receipt(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.settlement
        .receipt(c.actor_user_id, id)
        .await
        .map(|item| {
            Json(json!({"item":item,"dataAsOf":chrono::Utc::now(),"source":"business-core-b2"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn confirm_receipt(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.settlement
        .confirm_receipt(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn apply_receipt(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<ApplyReceipt>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.settlement
        .apply_receipt(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn reverse_allocation(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<ReverseAllocation>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.settlement
        .reverse_allocation(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn reverse_receipt(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.settlement
        .reverse_receipt(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn list_sales_returns(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, B2ApiError> {
    s.returns
        .sales_returns(c.actor_user_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b2"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn list_purchase_returns(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, B2ApiError> {
    s.returns
        .purchase_returns(c.actor_user_id, q.limit)
        .await
        .map(|items| {
            Json(json!({"items":items,"dataAsOf":chrono::Utc::now(),"source":"business-core-b3"}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn sales_return_options(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.returns
        .sales_options(c.actor_user_id)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn purchase_return_options(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.returns
        .purchase_options(c.actor_user_id)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn create_sales_return(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateReturn>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.returns
        .create_sales_return(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn create_purchase_return(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateReturn>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.returns
        .create_purchase_return(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn confirm_sales_return(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.returns
        .confirm_sales_return(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn confirm_purchase_return(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.returns
        .confirm_purchase_return(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn cancel_sales_return(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.returns
        .cancel_sales_return(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn cancel_purchase_return(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.returns
        .cancel_purchase_return(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn reconcile_inventory(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.inventory
        .reconcile(c.actor_user_id)
        .await
        .map(|differences| {
            Json(json!({"consistent":differences.is_empty(),"differences":differences}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
async fn reconcile_receivables(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
) -> Result<Json<impl serde::Serialize>, B2ApiError> {
    s.settlement
        .reconcile(c.actor_user_id)
        .await
        .map(|differences| {
            Json(json!({"consistent":differences.is_empty(),"differences":differences}))
        })
        .map_err(|e| B2ApiError::domain(e, c.trace_id))
}
fn enabled(state: &AppState, index: usize, trace_id: Uuid) -> Result<(), B2ApiError> {
    if state.b2_enabled[index] {
        Ok(())
    } else {
        Err(B2ApiError::simple(
            StatusCode::SERVICE_UNAVAILABLE,
            "feature_disabled",
            "Business Core B2 module is disabled",
            trace_id,
        ))
    }
}

#[cfg(test)]
mod agent_query_tests {
    use super::query_status;

    #[test]
    fn query_status_prefers_failure_then_emitted_response() {
        assert_eq!(query_status(true, true, true), "failed");
        assert_eq!(query_status(false, true, true), "complete");
        assert_eq!(query_status(false, false, true), "query_complete");
        assert_eq!(query_status(false, false, false), "running");
    }
}
