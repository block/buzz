//! Private, read-only deployment moderation API.

mod auth;
mod error;

use std::sync::Arc;

use auth::authorize;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue},
    middleware::{self, Next},
    response::Response,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use error::ApiError;
use serde::{Deserialize, Serialize};
use tower_http::limit::RequestBodyLimitLayer;
use uuid::Uuid;

pub(crate) fn is_admin_host(state: &crate::state::AppState, headers: &HeaderMap) -> bool {
    auth::is_admin_host(state, headers)
}

/// Build the read-only deployment-admin routes.
pub fn router(state: Arc<crate::state::AppState>) -> Router {
    Router::new()
        .route("/reports", get(reports))
        .route("/reports/{id}", get(report_detail))
        .route("/feedback", get(feedback))
        .route("/feedback/{id}", get(feedback_detail))
        .layer(middleware::from_fn(security_headers))
        .layer(RequestBodyLimitLayer::new(1024))
        .with_state(state)
}

async fn security_headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    response
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReportQuery {
    community_id: Option<Uuid>,
    status: Option<String>,
    report_type: Option<String>,
    target_kind: Option<String>,
    before: Option<DateTime<Utc>>,
    after: Option<DateTime<Utc>>,
    limit: Option<i64>,
}

fn limit(value: Option<i64>) -> Result<i64, ApiError> {
    match value.unwrap_or(50) {
        value @ 1..=200 => Ok(value),
        _ => Err(ApiError::bad_request(
            "invalid_limit",
            "limit must be between 1 and 200",
        )),
    }
}

fn validate(value: Option<&str>, allowed: &[&str], code: &'static str) -> Result<(), ApiError> {
    if value.is_some_and(|value| !allowed.contains(&value)) {
        Err(ApiError::bad_request(code, "filter is invalid"))
    } else {
        Ok(())
    }
}

async fn reports(
    State(state): State<Arc<crate::state::AppState>>,
    headers: HeaderMap,
    Query(query): Query<ReportQuery>,
) -> Result<Json<Vec<buzz_db::admin_moderation::AdminReport>>, ApiError> {
    authorize(&state, &headers)?;
    validate(
        query.status.as_deref(),
        &["open", "resolved", "dismissed", "escalated"],
        "invalid_status",
    )?;
    validate(
        query.target_kind.as_deref(),
        &["event", "pubkey", "blob"],
        "invalid_target_kind",
    )?;
    let items = state
        .db
        .admin_list_reports(
            query.community_id,
            query.status.as_deref(),
            query.report_type.as_deref(),
            query.target_kind.as_deref(),
            query.after,
            query.before,
            None,
            limit(query.limit)?,
        )
        .await?;
    Ok(Json(items))
}

async fn report_detail(
    State(state): State<Arc<crate::state::AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<buzz_db::admin_moderation::AdminReport>, ApiError> {
    authorize(&state, &headers)?;
    state
        .db
        .admin_get_report(id)
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackSummary {
    id: Uuid,
    community_id: Uuid,
    community_host: String,
    submitter_pubkey: String,
    category: Option<String>,
    body_summary: String,
    received_at: DateTime<Utc>,
}

async fn feedback(
    State(state): State<Arc<crate::state::AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<FeedbackSummary>>, ApiError> {
    authorize(&state, &headers)?;
    let items = state
        .db
        .admin_list_feedback(100)
        .await?
        .into_iter()
        .map(|item| FeedbackSummary {
            id: item.id,
            community_id: item.community_id,
            community_host: item.community_host,
            submitter_pubkey: item.submitter_pubkey,
            category: item.category,
            body_summary: summarize_body(&item.body),
            received_at: item.received_at,
        })
        .collect();
    Ok(Json(items))
}

async fn feedback_detail(
    State(state): State<Arc<crate::state::AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<buzz_db::admin_moderation::AdminFeedback>, ApiError> {
    authorize(&state, &headers)?;
    state
        .db
        .admin_get_feedback(id)
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

fn summarize_body(body: &str) -> String {
    const MAX_CHARS: usize = 240;
    let mut chars = body.chars();
    let mut summary = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        summary.push('…');
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_filters_reject_unknown_values() {
        assert!(validate(Some("open"), &["open"], "invalid_status").is_ok());
        assert!(validate(Some("unknown"), &["open"], "invalid_status").is_err());
    }

    #[test]
    fn feedback_summary_is_unicode_safe_and_marks_truncation() {
        let body = "🐝".repeat(241);
        let summary = summarize_body(&body);
        assert_eq!(summary.chars().count(), 241);
        assert!(summary.ends_with('…'));
    }
}
