use super::model::{
    CreateAdjustmentBatch, GenerateReportSnapshot, PostAdjustment, ReplaceAdjustmentDraft,
    VersionCommand,
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
struct B4ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    trace_id: Uuid,
}

impl IntoResponse for B4ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"code":self.code,"message":self.message,"traceId":self.trace_id})),
        )
            .into_response()
    }
}

impl B4ApiError {
    fn simple(status: StatusCode, code: &'static str, message: &str, trace_id: Uuid) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            trace_id,
        }
    }
    fn domain(error: DomainError, trace_id: Uuid) -> Self {
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
            DomainError::StalePreview => Self::simple(
                StatusCode::CONFLICT,
                "STALE_PREVIEW",
                "allocation inputs changed; create a new preview",
                trace_id,
            ),
            DomainError::Invalid(message) => Self::simple(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                &message,
                trace_id,
            ),
            DomainError::Database(error) => {
                tracing::error!(%trace_id,error=%error,"B4 database command failed");
                Self::simple(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "service_unavailable",
                    "profitability command could not be completed",
                    trace_id,
                )
            }
            DomainError::Serialization(error) => {
                tracing::error!(%trace_id,error=%error,"B4 serialization failed");
                Self::simple(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "profitability command could not be completed",
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
struct ProfitQuery {
    #[serde(default)]
    order_id: Option<Uuid>,
    #[serde(default)]
    order_number: Option<String>,
    #[serde(default)]
    management_period: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DimensionQuery {
    management_period: String,
    currency: String,
    dimension_one: String,
    #[serde(default)]
    dimension_two: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReportQuery {
    management_period: String,
    currency: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProfitChangeQuery {
    base_from: chrono::NaiveDate,
    base_to: chrono::NaiveDate,
    comparison_from: chrono::NaiveDate,
    comparison_to: chrono::NaiveDate,
    currency: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LimitQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    100
}

pub fn service_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/order-profits", get(order_profits))
        .route("/v1/profitability", get(profitability))
        .route("/v1/management-profit-report", get(management_report))
        .route("/v1/profit-change", get(profit_change))
        .route("/v1/management-report-snapshots", get(list_snapshots))
        .route("/v1/management-report-snapshots/{id}", get(get_snapshot))
        .route("/v1/profit-evidence/{id}", get(profit_evidence))
        .route("/v1/reconciliation/profit-facts", get(reconcile))
}

pub fn browser_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/order-profits", get(order_profits))
        .route("/api/v1/profitability", get(profitability))
        .route("/api/v1/management-profit-report", get(management_report))
        .route("/api/v1/profit-evidence/{id}", get(profit_evidence))
        .route(
            "/api/v1/profit-adjustments",
            get(list_adjustments).post(create_adjustment),
        )
        .route("/api/v1/profit-adjustments/{id}", put(replace_adjustment))
        .route(
            "/api/v1/profit-adjustments/{id}/preview",
            post(preview_adjustment),
        )
        .route(
            "/api/v1/profit-adjustments/{id}/post",
            post(post_adjustment),
        )
        .route(
            "/api/v1/profit-adjustments/{id}/reverse",
            post(reverse_adjustment),
        )
        .route("/api/v1/reconciliation/profit-facts", get(reconcile))
        .route(
            "/api/v1/management-report-snapshots",
            get(list_snapshots).post(generate_snapshot),
        )
        .route(
            "/api/v1/management-report-snapshots/{id}",
            get(get_snapshot),
        )
}

fn key(headers: &HeaderMap, trace_id: Uuid) -> Result<&str, B4ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            B4ApiError::simple(
                StatusCode::BAD_REQUEST,
                "idempotency_key_required",
                "Idempotency-Key is required",
                trace_id,
            )
        })
}
fn enabled(state: &AppState, index: usize, trace_id: Uuid) -> Result<(), B4ApiError> {
    if state.b4_enabled[index] {
        Ok(())
    } else {
        Err(B4ApiError::simple(
            StatusCode::SERVICE_UNAVAILABLE,
            "feature_disabled",
            "Business Core B4 module is disabled",
            trace_id,
        ))
    }
}

async fn order_profits(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ProfitQuery>,
) -> Result<Json<serde_json::Value>, B4ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.profit_reporting
        .order_profits(
            c.actor_user_id,
            q.order_id,
            q.order_number.as_deref(),
            q.management_period.as_deref(),
            q.limit,
        )
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn profitability(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<DimensionQuery>,
) -> Result<Json<serde_json::Value>, B4ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.profit_reporting
        .profitability(
            c.actor_user_id,
            &q.management_period,
            &q.currency,
            &q.dimension_one,
            q.dimension_two.as_deref(),
            q.limit,
        )
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn management_report(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ReportQuery>,
) -> Result<Json<serde_json::Value>, B4ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.profit_reporting
        .management_report(c.actor_user_id, &q.management_period, &q.currency)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn profit_change(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<ProfitChangeQuery>,
) -> Result<Json<serde_json::Value>, B4ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.profit_reporting
        .profit_change(
            c.actor_user_id,
            q.base_from,
            q.base_to,
            q.comparison_from,
            q.comparison_to,
            &q.currency,
        )
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn profit_evidence(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<serde_json::Value>, B4ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.profit_reporting
        .evidence(c.actor_user_id, id, q.limit)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn list_adjustments(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<serde_json::Value>, B4ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.adjustments
        .list(c.actor_user_id, q.limit)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn create_adjustment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<CreateAdjustmentBatch>,
) -> Result<Json<impl serde::Serialize>, B4ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.adjustments
        .create(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn replace_adjustment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<ReplaceAdjustmentDraft>,
) -> Result<Json<impl serde::Serialize>, B4ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.adjustments
        .replace_draft(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn preview_adjustment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B4ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.adjustments
        .preview(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn post_adjustment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<PostAdjustment>,
) -> Result<Json<impl serde::Serialize>, B4ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.adjustments
        .post(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn reverse_adjustment(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    h: HeaderMap,
    Json(i): Json<VersionCommand>,
) -> Result<Json<impl serde::Serialize>, B4ApiError> {
    enabled(&s, 2, c.trace_id)?;
    s.adjustments
        .reverse(c.actor_user_id, c.trace_id, id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn reconcile(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
) -> Result<Json<serde_json::Value>, B4ApiError> {
    enabled(&s, 0, c.trace_id)?;
    s.profit_projection
        .reconcile(c.actor_user_id)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn generate_snapshot(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    h: HeaderMap,
    Json(i): Json<GenerateReportSnapshot>,
) -> Result<Json<impl serde::Serialize>, B4ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.profit_reporting
        .generate_snapshot(c.actor_user_id, c.trace_id, key(&h, c.trace_id)?, &i)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn list_snapshots(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Query(q): Query<LimitQuery>,
) -> Result<Json<serde_json::Value>, B4ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.profit_reporting
        .snapshots(c.actor_user_id, None, q.limit)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
async fn get_snapshot(
    State(s): State<Arc<AppState>>,
    Extension(c): Extension<RequestContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, B4ApiError> {
    enabled(&s, 1, c.trace_id)?;
    s.profit_reporting
        .snapshots(c.actor_user_id, Some(id), 1)
        .await
        .map(Json)
        .map_err(|e| B4ApiError::domain(e, c.trace_id))
}
