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

use buzz_core::nostr_identity::canonical_npub_or_invalid;
use buzz_db::admin_moderation::{
    AdminFeedback, AdminReport, AdminReportDetail, AdminReportedMessage,
};

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
        .route(
            "/feedback/{id}/attachments/{sha256}",
            get(feedback_attachment),
        )
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
) -> Result<Json<Vec<AdminReportResponse>>, ApiError> {
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
    Ok(Json(
        items.into_iter().map(AdminReportResponse::from).collect(),
    ))
}

async fn report_detail(
    State(state): State<Arc<crate::state::AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<AdminReportDetailResponse>, ApiError> {
    authorize(&state, &headers)?;
    state
        .db
        .admin_get_report(id)
        .await?
        .map(AdminReportDetailResponse::from)
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminReportResponse {
    id: Uuid,
    community_id: Uuid,
    community_host: String,
    report_event_id: String,
    /// Legacy protocol-hex field retained for existing dashboard clients.
    reporter_pubkey: String,
    /// Canonical public identity for new clients.
    reporter_npub: String,
    target_kind: String,
    /// Legacy target value. Pubkey targets remain protocol hex; event and blob
    /// targets retain their existing encodings.
    target: String,
    /// Canonical identity when `target_kind` is `pubkey`.
    #[serde(skip_serializing_if = "Option::is_none")]
    target_npub: Option<String>,
    channel_id: Option<Uuid>,
    report_type: String,
    note: Option<String>,
    status: String,
    /// Legacy protocol-hex resolver retained for existing clients.
    resolved_by: Option<String>,
    /// Canonical resolver identity for new clients.
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_by_npub: Option<String>,
    resolved_at: Option<DateTime<Utc>>,
    action_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

impl From<AdminReport> for AdminReportResponse {
    fn from(report: AdminReport) -> Self {
        let target_npub = if report.target_kind == "pubkey" {
            Some(canonical_npub_or_invalid(&report.target))
        } else {
            None
        };
        let reporter_npub = canonical_npub_or_invalid(&report.reporter_pubkey);
        let resolved_by_npub = report.resolved_by.as_deref().map(canonical_npub_or_invalid);
        Self {
            id: report.id,
            community_id: report.community_id,
            community_host: report.community_host,
            report_event_id: report.report_event_id,
            reporter_pubkey: report.reporter_pubkey,
            reporter_npub,
            target_kind: report.target_kind,
            target: report.target,
            target_npub,
            channel_id: report.channel_id,
            report_type: report.report_type,
            note: report.note,
            status: report.status,
            resolved_by: report.resolved_by,
            resolved_by_npub,
            resolved_at: report.resolved_at,
            action_id: report.action_id,
            created_at: report.created_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminReportedMessageResponse {
    /// Legacy protocol-hex field retained for existing dashboard clients.
    author_pubkey: String,
    /// Canonical author identity for new clients.
    author_npub: String,
    content: String,
    created_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
}

impl From<AdminReportedMessage> for AdminReportedMessageResponse {
    fn from(message: AdminReportedMessage) -> Self {
        let author_npub = canonical_npub_or_invalid(&message.author_pubkey);
        Self {
            author_pubkey: message.author_pubkey,
            author_npub,
            content: message.content,
            created_at: message.created_at,
            deleted_at: message.deleted_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminReportDetailResponse {
    #[serde(flatten)]
    report: AdminReportResponse,
    message: Option<AdminReportedMessageResponse>,
}

impl From<AdminReportDetail> for AdminReportDetailResponse {
    fn from(detail: AdminReportDetail) -> Self {
        Self {
            report: detail.report.into(),
            message: detail.message.map(Into::into),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FeedbackSummary {
    id: Uuid,
    community_id: Uuid,
    community_host: String,
    /// Legacy protocol-hex field retained for existing dashboard clients.
    submitter_pubkey: String,
    /// Canonical submitter identity for new clients.
    submitter_npub: String,
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
        .map(|item| {
            let body_summary = summarize_body(&item.body, &item.tags);
            let submitter_npub = canonical_npub_or_invalid(&item.submitter_pubkey);
            FeedbackSummary {
                id: item.id,
                community_id: item.community_id,
                community_host: item.community_host,
                submitter_pubkey: item.submitter_pubkey,
                submitter_npub,
                category: item.category,
                body_summary,
                received_at: item.received_at,
            }
        })
        .collect();
    Ok(Json(items))
}

async fn feedback_detail(
    State(state): State<Arc<crate::state::AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<AdminFeedbackResponse>, ApiError> {
    authorize(&state, &headers)?;
    state
        .db
        .admin_get_feedback(id)
        .await?
        .map(AdminFeedbackResponse::from)
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminFeedbackResponse {
    id: Uuid,
    community_id: Uuid,
    community_host: String,
    event_id: String,
    /// Legacy protocol-hex field retained for existing dashboard clients.
    submitter_pubkey: String,
    /// Canonical submitter identity for new clients.
    submitter_npub: String,
    category: Option<String>,
    body: String,
    tags: serde_json::Value,
    event_created_at: DateTime<Utc>,
    received_at: DateTime<Utc>,
}

impl From<AdminFeedback> for AdminFeedbackResponse {
    fn from(feedback: AdminFeedback) -> Self {
        let submitter_npub = canonical_npub_or_invalid(&feedback.submitter_pubkey);
        Self {
            id: feedback.id,
            community_id: feedback.community_id,
            community_host: feedback.community_host,
            event_id: feedback.event_id,
            submitter_pubkey: feedback.submitter_pubkey,
            submitter_npub,
            category: feedback.category,
            body: feedback.body,
            tags: feedback.tags,
            event_created_at: feedback.event_created_at,
            received_at: feedback.received_at,
        }
    }
}

async fn feedback_attachment(
    State(state): State<Arc<crate::state::AppState>>,
    headers: HeaderMap,
    Path((id, sha256)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    authorize(&state, &headers)?;
    if !is_sha256(&sha256) {
        return Err(ApiError::not_found());
    }

    let feedback = state
        .db
        .admin_get_feedback(id)
        .await?
        .ok_or_else(ApiError::not_found)?;
    if !feedback_references_hash(&feedback.tags, &feedback.community_host, &sha256) {
        return Err(ApiError::not_found());
    }

    // Resolve the tenant from server-owned feedback provenance, then assert the
    // resolved row still agrees with the feedback FK. Client input never names
    // a community, host, object key, extension, or upstream URL.
    let tenant = crate::tenant::bind_community(&state.db, &feedback.community_host)
        .await
        .map_err(|_| ApiError::not_found())?;
    if tenant.community().as_uuid() != &feedback.community_id {
        tracing::warn!(
            feedback_id = %feedback.id,
            feedback_community_id = %feedback.community_id,
            resolved_community_id = %tenant.community(),
            "admin feedback attachment tenant provenance mismatch"
        );
        return Err(ApiError::not_found());
    }

    let response = crate::api::media::serve_blob_for_tenant(&state, &tenant, &sha256, &headers)
        .await
        .map_err(|error| match error {
            buzz_media::MediaError::NotFound => ApiError::not_found(),
            _ => ApiError::internal(),
        })?;
    tracing::info!(
        feedback_id = %feedback.id,
        community_id = %feedback.community_id,
        attachment_sha256 = %sha256,
        "admin feedback attachment read"
    );
    Ok(response)
}

fn feedback_references_hash(tags: &serde_json::Value, community_host: &str, sha256: &str) -> bool {
    tags.as_array()
        .into_iter()
        .flatten()
        .filter_map(|tag| tag.as_array())
        .filter(|tag| tag.first().and_then(|value| value.as_str()) == Some("imeta"))
        .any(|tag| {
            let fields = tag
                .iter()
                .skip(1)
                .filter_map(|value| value.as_str()?.split_once(' '))
                .collect::<std::collections::HashMap<_, _>>();
            fields.get("x") == Some(&sha256)
                && fields
                    .get("url")
                    .is_some_and(|url| attachment_url_matches(url, community_host, sha256))
        })
}

fn attachment_url_matches(url: &str, community_host: &str, sha256: &str) -> bool {
    let parsed = if url.starts_with('/') {
        url::Url::parse(&format!("https://{community_host}{url}"))
    } else {
        url::Url::parse(url)
    };
    let Ok(url) = parsed else {
        return false;
    };
    let authority = url.port().map_or_else(
        || url.host_str().unwrap_or_default().to_string(),
        |port| format!("{}:{port}", url.host_str().unwrap_or_default()),
    );
    let Some(media_name) = url.path().strip_prefix("/media/") else {
        return false;
    };
    let Some((url_hash, extension)) = media_name.split_once('.') else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && buzz_core::tenant::normalize_host(&authority)
            == buzz_core::tenant::normalize_host(community_host)
        && url_hash == sha256
        && crate::api::media::is_safe_ext(extension)
        && url.query().is_none()
        && url.fragment().is_none()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|character| matches!(character, '0'..='9' | 'a'..='f'))
}

fn summarize_body(body: &str, tags: &serde_json::Value) -> String {
    const MAX_CHARS: usize = 240;
    let attachment_urls = tags
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tag| tag.as_array())
        .filter(|tag| tag.first().and_then(|value| value.as_str()) == Some("imeta"))
        .flat_map(|tag| tag.iter().skip(1))
        .filter_map(|value| value.as_str()?.strip_prefix("url "))
        .collect::<std::collections::HashSet<_>>();
    let body = body
        .lines()
        .filter(|line| {
            let line = line.trim();
            let url = line
                .strip_suffix(')')
                .and_then(|line| line.rsplit_once("]("))
                .and_then(|(label, url)| {
                    (label.starts_with('[') || label.starts_with("![")).then_some(url)
                });
            url.is_none_or(|url| !attachment_urls.contains(url))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut chars = body.trim().chars();
    let mut summary = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        summary.push('…');
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn test_state() -> Arc<crate::state::AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            web_dir: None,
        });
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    const HASH: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    #[tokio::test]
    async fn report_detail_requires_admin_host_before_database_access() {
        let response = router(test_state().await)
            .oneshot(
                Request::builder()
                    .uri(format!("/reports/{}", Uuid::nil()))
                    .header(header::HOST, "community.example")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn report_detail_rejects_unknown_report() {
        let response = router(test_state().await)
            .oneshot(
                Request::builder()
                    .uri(format!("/reports/{}", Uuid::nil()))
                    .header(header::HOST, "admin.example")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn feedback_attachment_requires_admin_host_before_database_access() {
        let response = router(test_state().await)
            .oneshot(
                Request::builder()
                    .uri(format!("/feedback/{}/attachments/{HASH}", Uuid::nil()))
                    .header(header::HOST, "community.example")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn feedback_attachment_rejects_unknown_feedback() {
        let response = router(test_state().await)
            .oneshot(
                Request::builder()
                    .uri(format!("/feedback/{}/attachments/{HASH}", Uuid::nil()))
                    .header(header::HOST, "admin.example")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn feedback_attachment_rejects_write_methods() {
        let state = test_state().await;
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let response = router(state.clone())
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(format!("/feedback/{}/attachments/{HASH}", Uuid::nil()))
                        .header(header::HOST, "admin.example")
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(
                response.status(),
                axum::http::StatusCode::METHOD_NOT_ALLOWED,
                "{method}"
            );
        }
    }

    #[test]
    fn report_filters_reject_unknown_values() {
        assert!(validate(Some("open"), &["open"], "invalid_status").is_ok());
        assert!(validate(Some("unknown"), &["open"], "invalid_status").is_err());
    }

    #[test]
    fn feedback_summary_is_unicode_safe_and_marks_truncation() {
        let body = "🐝".repeat(241);
        let summary = summarize_body(&body, &serde_json::Value::Null);
        assert_eq!(summary.chars().count(), 241);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn feedback_summary_omits_imeta_attachment_lines() {
        let url = "http://localhost:3000/media/abc.png";
        let tags = serde_json::json!([["imeta", format!("url {url}"), "m image/png"]]);
        assert_eq!(
            summarize_body(&format!("Useful context.\n![image]({url})"), &tags),
            "Useful context."
        );
    }

    fn attachment_tags(host: &str, x: &str, url_hash: &str) -> serde_json::Value {
        serde_json::json!([[
            "imeta",
            format!("url https://{host}/media/{url_hash}.png"),
            "m image/png",
            format!("x {x}"),
            "size 100"
        ]])
    }

    #[test]
    fn feedback_attachment_requires_matching_imeta_hash_and_source_host() {
        let tags = attachment_tags("community.example", HASH, HASH);
        assert!(feedback_references_hash(&tags, "community.example", HASH));

        let unreferenced = "f".repeat(64);
        assert!(!feedback_references_hash(
            &tags,
            "community.example",
            &unreferenced
        ));
        assert!(!feedback_references_hash(
            &tags,
            "other-community.example",
            HASH
        ));
    }

    #[test]
    fn feedback_attachment_rejects_cross_field_and_path_substitution() {
        let other_hash = "f".repeat(64);
        assert!(!feedback_references_hash(
            &attachment_tags("community.example", HASH, &other_hash),
            "community.example",
            HASH
        ));

        for url in [
            format!("https://community.example/media/{HASH}.png?token=leak"),
            format!("https://community.example/media/{HASH}.thumb.jpg"),
            format!("https://community.example/media/{HASH}.png/extra"),
            format!("https://evil.example/media/{HASH}.png"),
        ] {
            assert!(!attachment_url_matches(&url, "community.example", HASH));
        }
    }

    #[test]
    fn feedback_attachment_accepts_valid_relative_source_url() {
        assert!(attachment_url_matches(
            &format!("/media/{HASH}.png"),
            "community.example",
            HASH
        ));
    }

    #[test]
    fn feedback_attachment_hash_is_exact_lowercase_sha256() {
        assert!(is_sha256(HASH));
        assert!(!is_sha256(&HASH.to_uppercase()));
        assert!(!is_sha256(&HASH[..63]));
        assert!(!is_sha256(&format!("{HASH}.png")));
    }

    #[test]
    fn admin_report_projection_adds_npub_without_changing_legacy_fields() {
        let keys = nostr::Keys::generate();
        let pubkey_hex = keys.public_key().to_hex();
        let expected_npub =
            buzz_core::nostr_identity::public_key_to_npub(&keys.public_key()).expect("npub");
        let now = Utc::now();
        let report = AdminReport {
            id: Uuid::new_v4(),
            community_id: Uuid::new_v4(),
            community_host: "community.example".to_string(),
            report_event_id: HASH.to_string(),
            reporter_pubkey: pubkey_hex.clone(),
            target_kind: "pubkey".to_string(),
            target: pubkey_hex.clone(),
            channel_id: Some(Uuid::new_v4()),
            report_type: "spam".to_string(),
            note: Some("report note".to_string()),
            status: "resolved".to_string(),
            resolved_by: Some(pubkey_hex.clone()),
            resolved_at: Some(now),
            action_id: Some(Uuid::new_v4()),
            created_at: now,
        };

        let value = serde_json::to_value(AdminReportResponse::from(report.clone()))
            .expect("serialize report response");
        assert_eq!(value["reporterPubkey"], pubkey_hex);
        assert_eq!(value["reporterNpub"], expected_npub);
        assert_eq!(value["target"], pubkey_hex);
        assert_eq!(value["targetNpub"], expected_npub);
        assert_eq!(value["resolvedBy"], pubkey_hex);
        assert_eq!(value["resolvedByNpub"], expected_npub);
        assert_eq!(value["reportEventId"], HASH);

        let event_target = AdminReportResponse::from(AdminReport {
            target_kind: "event".to_string(),
            target: HASH.to_string(),
            ..report
        });
        assert_eq!(event_target.target, HASH);
    }

    #[test]
    fn admin_detail_and_feedback_add_npub_without_changing_event_data() {
        let keys = nostr::Keys::generate();
        let pubkey_hex = keys.public_key().to_hex();
        let expected_npub =
            buzz_core::nostr_identity::public_key_to_npub(&keys.public_key()).expect("npub");
        let now = Utc::now();
        let report = AdminReport {
            id: Uuid::new_v4(),
            community_id: Uuid::new_v4(),
            community_host: "community.example".to_string(),
            report_event_id: HASH.to_string(),
            reporter_pubkey: pubkey_hex.clone(),
            target_kind: "event".to_string(),
            target: HASH.to_string(),
            channel_id: None,
            report_type: "spam".to_string(),
            note: None,
            status: "open".to_string(),
            resolved_by: None,
            resolved_at: None,
            action_id: None,
            created_at: now,
        };
        let detail = AdminReportDetailResponse::from(AdminReportDetail {
            report,
            message: Some(AdminReportedMessage {
                author_pubkey: pubkey_hex.clone(),
                content: "message".to_string(),
                created_at: now,
                deleted_at: None,
            }),
        });
        let detail_value = serde_json::to_value(detail).expect("serialize report detail response");
        assert_eq!(detail_value["message"]["authorPubkey"], pubkey_hex);
        assert_eq!(detail_value["message"]["authorNpub"], expected_npub);
        assert_eq!(detail_value["target"], HASH);

        let tags = serde_json::json!([["e", HASH]]);
        let feedback = AdminFeedbackResponse::from(AdminFeedback {
            id: Uuid::new_v4(),
            community_id: Uuid::new_v4(),
            community_host: "community.example".to_string(),
            event_id: HASH.to_string(),
            submitter_pubkey: pubkey_hex.clone(),
            category: Some("bug".to_string()),
            body: "feedback".to_string(),
            tags: tags.clone(),
            event_created_at: now,
            received_at: now,
        });
        let feedback_value = serde_json::to_value(feedback).expect("serialize feedback response");
        assert_eq!(feedback_value["submitterPubkey"], pubkey_hex);
        assert_eq!(feedback_value["submitterNpub"], expected_npub);
        assert_eq!(feedback_value["eventId"], HASH);
        assert_eq!(feedback_value["tags"], tags);
    }

    #[test]
    fn admin_identity_projection_fails_closed_for_an_invalid_curve_point() {
        let now = Utc::now();
        let response = AdminReportResponse::from(AdminReport {
            id: Uuid::new_v4(),
            community_id: Uuid::new_v4(),
            community_host: "community.example".to_string(),
            report_event_id: HASH.to_string(),
            reporter_pubkey: "ff".repeat(32),
            target_kind: "pubkey".to_string(),
            target: "ff".repeat(32),
            channel_id: None,
            report_type: "spam".to_string(),
            note: None,
            status: "open".to_string(),
            resolved_by: None,
            resolved_at: None,
            action_id: None,
            created_at: now,
        });

        assert_eq!(
            response.reporter_npub,
            buzz_core::nostr_identity::INVALID_PUBLIC_KEY_DISPLAY
        );
        assert_eq!(
            response.target_npub.as_deref(),
            Some(buzz_core::nostr_identity::INVALID_PUBLIC_KEY_DISPLAY)
        );
        assert_eq!(response.reporter_pubkey, "ff".repeat(32));
        assert_eq!(response.target, "ff".repeat(32));
    }
}
