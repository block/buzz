use super::{CreateSubscription, GenerateOperatingSnapshot, IncidentCommand, SubscriptionCommand};
use crate::{api::AppState, b2::DomainError, security::RequestContext};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DashboardQuery {
    management_period: String,
    currency: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrendQuery {
    cadence: String,
    currency: String,
    #[serde(default = "default_trend_limit")]
    limit: i64,
}

fn default_trend_limit() -> i64 {
    14
}

#[derive(Debug)]
struct S1ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    trace_id: Uuid,
}

impl IntoResponse for S1ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"code":self.code,"message":self.message,"traceId":self.trace_id})),
        )
            .into_response()
    }
}

impl S1ApiError {
    fn domain(error: DomainError, trace_id: Uuid) -> Self {
        match error {
            DomainError::NotFoundOrForbidden => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found_or_forbidden",
                message: "resource was not found or is not accessible".into(),
                trace_id,
            },
            DomainError::Invalid(message) => Self {
                status: StatusCode::BAD_REQUEST,
                code: "invalid_request",
                message,
                trace_id,
            },
            DomainError::VersionConflict => Self {
                status: StatusCode::CONFLICT,
                code: "VERSION_CONFLICT",
                message: "resource version changed; refresh and retry".into(),
                trace_id,
            },
            DomainError::IdempotencyConflict => Self {
                status: StatusCode::CONFLICT,
                code: "IDEMPOTENCY_CONFLICT",
                message: "idempotency key was reused with a different request".into(),
                trace_id,
            },
            other => {
                tracing::error!(%trace_id,error=%other,"S1 operating read failed");
                Self {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    code: "service_unavailable",
                    message: "operating read could not be completed".into(),
                    trace_id,
                }
            }
        }
    }
}

pub fn service_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/v1/operations/data-quality", get(data_quality))
        .route("/v1/operations/dashboard", get(dashboard))
        .route("/v1/operations/incidents", get(list_incidents))
        .route("/v1/operations/incidents/scan", post(scan_incidents))
        .route(
            "/v1/operations/incidents/{id}/commands",
            post(command_incident),
        )
        .route("/v1/operations/trends", get(operating_trends))
        .route("/v1/operations/snapshots", post(generate_snapshot))
        .route(
            "/v1/operations/subscriptions",
            get(list_subscriptions).post(create_subscription),
        )
        .route(
            "/v1/operations/subscriptions/{id}/commands",
            post(command_subscription),
        )
}

pub fn browser_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/v1/operations/data-quality", get(data_quality))
        .route("/api/v1/operations/dashboard", get(dashboard))
        .route("/api/v1/operations/incidents", get(list_incidents))
        .route("/api/v1/operations/incidents/scan", post(scan_incidents))
        .route(
            "/api/v1/operations/incidents/{id}/commands",
            post(command_incident),
        )
        .route("/api/v1/operations/trends", get(operating_trends))
        .route("/api/v1/operations/snapshots", post(generate_snapshot))
        .route(
            "/api/v1/operations/subscriptions",
            get(list_subscriptions).post(create_subscription),
        )
        .route(
            "/api/v1/operations/subscriptions/{id}/commands",
            post(command_subscription),
        )
}

async fn operating_trends(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Query(query): Query<TrendQuery>,
) -> Result<Json<Value>, S1ApiError> {
    state
        .operations
        .operating_trends(
            context.actor_user_id,
            &query.cadence,
            &query.currency,
            query.limit,
        )
        .await
        .map(Json)
        .map_err(|error| S1ApiError::domain(error, context.trace_id))
}

async fn generate_snapshot(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    headers: HeaderMap,
    Json(input): Json<GenerateOperatingSnapshot>,
) -> Result<Json<Value>, S1ApiError> {
    let key = idempotency_key(&headers, context.trace_id)?;
    state
        .operations
        .generate_operating_snapshot(context.actor_user_id, context.trace_id, key, &input)
        .await
        .map(Json)
        .map_err(|error| S1ApiError::domain(error, context.trace_id))
}

async fn list_subscriptions(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
) -> Result<Json<Value>, S1ApiError> {
    state
        .operations
        .list_operating_subscriptions(context.actor_user_id)
        .await
        .map(Json)
        .map_err(|error| S1ApiError::domain(error, context.trace_id))
}

async fn create_subscription(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    headers: HeaderMap,
    Json(input): Json<CreateSubscription>,
) -> Result<Json<Value>, S1ApiError> {
    let key = idempotency_key(&headers, context.trace_id)?;
    state
        .operations
        .create_operating_subscription(context.actor_user_id, context.trace_id, key, &input)
        .await
        .map(Json)
        .map_err(|error| S1ApiError::domain(error, context.trace_id))
}

async fn command_subscription(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<SubscriptionCommand>,
) -> Result<Json<Value>, S1ApiError> {
    let key = idempotency_key(&headers, context.trace_id)?;
    state
        .operations
        .command_operating_subscription(context.actor_user_id, context.trace_id, key, id, &input)
        .await
        .map(Json)
        .map_err(|error| S1ApiError::domain(error, context.trace_id))
}

async fn list_incidents(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
) -> Result<Json<Value>, S1ApiError> {
    state
        .operations
        .list_incidents(context.actor_user_id)
        .await
        .map(Json)
        .map_err(|error| S1ApiError::domain(error, context.trace_id))
}

async fn scan_incidents(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    headers: HeaderMap,
) -> Result<Json<Value>, S1ApiError> {
    let key = idempotency_key(&headers, context.trace_id)?;
    state
        .operations
        .scan_incidents(context.actor_user_id, context.trace_id, key)
        .await
        .map(Json)
        .map_err(|error| S1ApiError::domain(error, context.trace_id))
}

async fn command_incident(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<IncidentCommand>,
) -> Result<Json<Value>, S1ApiError> {
    let key = idempotency_key(&headers, context.trace_id)?;
    state
        .operations
        .command_incident(context.actor_user_id, context.trace_id, key, id, input)
        .await
        .map(Json)
        .map_err(|error| S1ApiError::domain(error, context.trace_id))
}

fn idempotency_key(headers: &HeaderMap, trace_id: Uuid) -> Result<&str, S1ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| S1ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "idempotency_key_required",
            message: "Idempotency-Key header is required".into(),
            trace_id,
        })
}

async fn data_quality(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
) -> Result<Json<serde_json::Value>, S1ApiError> {
    let started = std::time::Instant::now();
    let result = state.operations.data_quality(context.actor_user_id).await;
    finish_read(result, &context, "data_quality", started, 2_000.0)
}

async fn dashboard(
    State(state): State<Arc<AppState>>,
    Extension(context): Extension<RequestContext>,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<serde_json::Value>, S1ApiError> {
    let started = std::time::Instant::now();
    let result = state
        .operations
        .dashboard(
            context.actor_user_id,
            &query.management_period,
            &query.currency,
        )
        .await;
    finish_read(result, &context, "dashboard", started, 500.0)
}

fn finish_read(
    result: Result<Value, DomainError>,
    context: &RequestContext,
    route: &'static str,
    started: std::time::Instant,
    target_ms: f64,
) -> Result<Json<Value>, S1ApiError> {
    let duration_ms = (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000.0;
    let success = result.is_ok();
    let slow = duration_ms > target_ms;
    if slow {
        tracing::warn!(trace_id=%context.trace_id,route,duration_ms,target_ms,success,"S1 operating read exceeded target");
    } else {
        tracing::info!(trace_id=%context.trace_id,route,duration_ms,target_ms,success,"S1 operating read completed");
    }
    result
        .map(|mut value| {
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "run".into(),
                    json!({
                        "traceId": context.trace_id,
                        "status": if slow { "slow" } else { "completed" },
                        "durationMs": duration_ms,
                        "targetMs": target_ms,
                        "completedAt": chrono::Utc::now()
                    }),
                );
            }
            Json(value)
        })
        .map_err(|error| S1ApiError::domain(error, context.trace_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_reads_include_traceable_run_metadata() {
        let context = RequestContext {
            actor_user_id: Uuid::new_v4(),
            trace_id: Uuid::new_v4(),
        };
        let Json(value) = finish_read(
            Ok(json!({"status":"complete"})),
            &context,
            "dashboard",
            std::time::Instant::now(),
            500.0,
        )
        .unwrap();
        assert_eq!(value["run"]["traceId"], context.trace_id.to_string());
        assert_eq!(value["run"]["status"], "completed");
        assert_eq!(value["run"]["targetMs"], 500.0);
    }
}
