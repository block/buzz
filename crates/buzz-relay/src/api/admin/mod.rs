//! Private deployment moderation API.
//!
//! Read routes are available in all auth modes (token, disabled, nip98).
//! Mutation and staffing routes require `BUZZ_ADMIN_AUTH=nip98`.

mod auth;
mod error;

use std::sync::Arc;

use auth::{authorize, require_mutation_principal, require_operator, AdminRole, AdminSource};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, Uri},
    middleware::{self, Next},
    response::Response,
    routing::{delete, get, patch, put},
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

/// Build the deployment-admin routes.
///
/// Read routes are available in all auth modes.
/// Mutation routes (/reports/{id}/resolve, /feedback/{id}) and staffing routes
/// (/operators) require BUZZ_ADMIN_AUTH=nip98.
pub fn router(state: Arc<crate::state::AppState>) -> Router {
    Router::new()
        .route("/probe", get(probe))
        .route("/reports", get(reports))
        .route("/reports/{id}", get(report_detail))
        .route("/reports/{id}/resolve", axum::routing::post(resolve_report))
        .route("/feedback", get(feedback))
        .route("/feedback/{id}", get(feedback_detail))
        .route("/feedback/{id}", patch(update_feedback_status))
        .route(
            "/feedback/{id}/attachments/{sha256}",
            get(feedback_attachment),
        )
        .route("/operators", get(list_operators))
        .route("/operators/{pubkey}", put(upsert_operator))
        .route("/operators/{pubkey}", delete(delete_operator))
        .layer(middleware::from_fn(security_headers))
        // Mutation routes carry a JSON body (max ~4 KB); read-only routes have no body.
        .layer(RequestBodyLimitLayer::new(4096))
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

/// Probe response — allows the desktop to discover the auth mode, role, and
/// available capabilities before rendering the console UI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeResponse {
    /// `"ok"`
    status: &'static str,
    /// Auth mode: `"nip98"` | `"token"` | `"disabled"`.
    auth_mode: &'static str,
    /// Role of the authenticated principal (`"operator"` | `"moderator"`),
    /// or `null` in token/disabled modes (no named principal).
    role: Option<&'static str>,
    /// How the role was established (`"config"` | `"owner_fallback"` | `"db"`),
    /// or `null` when role is null.
    source: Option<&'static str>,
    /// Whether mutation (report-action) endpoints are available.
    can_act: bool,
    /// Whether staffing endpoints (/operators) are available.
    can_staff: bool,
}

async fn probe(
    State(state): State<Arc<crate::state::AppState>>,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Json<ProbeResponse>, ApiError> {
    let principal = authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "GET",
        None,
    )
    .await?;

    let (auth_mode, role, source, can_act, can_staff) = match &state.config.admin {
        Some(config) => match &config.auth {
            crate::config::AdminAuth::Token(_) => ("token", None, None, false, false),
            crate::config::AdminAuth::Disabled => ("disabled", None, None, false, false),
            crate::config::AdminAuth::Nip98 => {
                // principal is Some in nip98 mode (authorize returns Ok(Some(_)))
                let p = principal
                    .as_ref()
                    .expect("nip98 mode always resolves principal");
                let role_str = match p.role {
                    AdminRole::Operator => "operator",
                    AdminRole::Moderator => "moderator",
                };
                let source_str = match p.source {
                    AdminSource::Config => "config",
                    AdminSource::OwnerFallback => "owner_fallback",
                    AdminSource::Db => "db",
                };
                let can_act = true; // both Operator and Moderator can act
                let can_staff = p.role == AdminRole::Operator;
                (
                    "nip98",
                    Some(role_str),
                    Some(source_str),
                    can_act,
                    can_staff,
                )
            }
        },
        None => return Err(ApiError::not_found()),
    };

    Ok(Json(ProbeResponse {
        status: "ok",
        auth_mode,
        role,
        source,
        can_act,
        can_staff,
    }))
}

async fn reports(
    State(state): State<Arc<crate::state::AppState>>,
    uri: Uri,
    headers: HeaderMap,
    Query(query): Query<ReportQuery>,
) -> Result<Json<Vec<buzz_db::admin_moderation::AdminReport>>, ApiError> {
    authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "GET",
        None,
    )
    .await?;
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
    uri: Uri,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<buzz_db::admin_moderation::AdminReportDetail>, ApiError> {
    authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "GET",
        None,
    )
    .await?;
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
    uri: Uri,
    headers: HeaderMap,
) -> Result<Json<Vec<FeedbackSummary>>, ApiError> {
    authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "GET",
        None,
    )
    .await?;
    let items = state
        .db
        .admin_list_feedback(100)
        .await?
        .into_iter()
        .map(|item| {
            let body_summary = summarize_body(&item.body, &item.tags);
            FeedbackSummary {
                id: item.id,
                community_id: item.community_id,
                community_host: item.community_host,
                submitter_pubkey: item.submitter_pubkey,
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
    uri: Uri,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<buzz_db::admin_moderation::AdminFeedback>, ApiError> {
    authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "GET",
        None,
    )
    .await?;
    state
        .db
        .admin_get_feedback(id)
        .await?
        .map(Json)
        .ok_or_else(ApiError::not_found)
}

async fn feedback_attachment(
    State(state): State<Arc<crate::state::AppState>>,
    uri: Uri,
    headers: HeaderMap,
    Path((id, sha256)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "GET",
        None,
    )
    .await?;
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

// ── Phase 2: Report resolution ────────────────────────────────────────────────

/// Request body for POST /reports/{id}/resolve.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolveReportBody {
    /// Action to take: delete | kick | ban | timeout | dismiss | escalate.
    action: String,
    /// Client-generated idempotency key. Required for enforcement actions.
    request_id: Option<Uuid>,
    /// Seconds until timeout expiry. Required for `timeout`, rejected otherwise.
    expiration_secs: Option<u64>,
    /// Optional operator reason.
    reason: Option<String>,
}

/// POST /reports/{id}/resolve
///
/// Requires nip98 auth. Both Operator and Moderator may act.
///
/// - dismiss/escalate: decision-only (no enforcement), runs in-transaction.
/// - delete/kick/ban/timeout: server-side enforcement state machine.
async fn resolve_report(
    State(state): State<Arc<crate::state::AppState>>,
    uri: Uri,
    headers: HeaderMap,
    Path(report_id): Path<Uuid>,
    body_bytes: Bytes,
) -> Result<axum::http::Response<axum::body::Body>, ApiError> {
    use crate::handlers::report_resolution::{
        enforcement_audit_action, http_validate_and_derive_status, resolve_report_decision_only,
        resolve_report_with_enforcement, ResolutionError,
    };

    let principal_opt = authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "POST",
        Some(&body_bytes),
    )
    .await?;

    let principal = require_mutation_principal(principal_opt)?;

    let body: ResolveReportBody = serde_json::from_slice(&body_bytes)
        .map_err(|_e| ApiError::bad_request("invalid_body", "invalid JSON body"))?;

    // Validate action name.
    let valid_actions = ["delete", "kick", "ban", "timeout", "dismiss", "escalate"];
    if !valid_actions.contains(&body.action.as_str()) {
        return Err(ApiError::bad_request("invalid_action", "unknown action"));
    }

    // Load report globally to derive target provenance.
    let report_detail = state
        .db
        .admin_get_report(report_id)
        .await?
        .ok_or_else(ApiError::not_found)?;

    // Compute timeout_until if needed.
    let timeout_until: Option<DateTime<Utc>> = body
        .expiration_secs
        .map(|secs| Utc::now() + chrono::Duration::seconds(secs as i64));

    // Validate action/target matrix and derive HTTP terminal status.
    let _derived_status = http_validate_and_derive_status(
        &body.action,
        &report_detail.report.target_kind,
        report_detail.report.channel_id,
        timeout_until,
    )
    .map_err(|msg| ApiError::bad_request("invalid_action_for_target", &msg))?;

    let actor_pubkey: Vec<u8> = principal.pubkey.to_vec();
    let actor_role_str = match principal.role {
        AdminRole::Operator => "operator",
        AdminRole::Moderator => "moderator",
    };
    let actor_authority = match principal.role {
        AdminRole::Operator => "relay_operator",
        AdminRole::Moderator => "relay_moderator",
    };

    // Bind tenant from server-owned report provenance (never from client input).
    let tenant = crate::tenant::bind_community(&state.db, &report_detail.report.community_host)
        .await
        .map_err(|_| ApiError::internal())?;

    match body.action.as_str() {
        "dismiss" | "escalate" => {
            // Decision-only: CAS open→terminal + audit row in one transaction.
            let audit_action = enforcement_audit_action(&body.action);
            let terminal_status = if body.action == "escalate" {
                "escalated"
            } else {
                "dismissed"
            };

            // Derive target fields from the report row.
            let (target_pubkey_bytes, target_event_id_bytes) = decode_report_target_hex(
                &report_detail.report.target_kind,
                &report_detail.report.target,
            )
            .map_err(|_| ApiError::internal())?;

            let reporter_bytes = hex::decode(&report_detail.report.reporter_pubkey)
                .map_err(|_| ApiError::internal())?;

            resolve_report_decision_only(
                &state,
                &tenant,
                report_id,
                terminal_status,
                audit_action,
                &actor_pubkey,
                actor_authority,
                target_pubkey_bytes.as_deref(),
                target_event_id_bytes.as_deref(),
                report_detail.report.channel_id,
                body.reason.as_deref(),
                &reporter_bytes,
            )
            .await
            .map_err(|e| match e {
                ResolutionError::NotFound => ApiError::not_found(),
                ResolutionError::NotOpen(status) => {
                    ApiError::conflict(&format!("report is not open (current status: {status})"))
                }
                ResolutionError::InvalidAction(msg) => {
                    ApiError::bad_request("invalid_action", &msg)
                }
                _ => ApiError::internal(),
            })?;

            Ok(axum::http::Response::builder()
                .status(200)
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({"status": terminal_status}).to_string(),
                ))
                .unwrap())
        }
        _ => {
            // Enforcement actions require a request_id.
            let request_id = body.request_id.ok_or_else(|| {
                ApiError::bad_request(
                    "missing_request_id",
                    "requestId is required for enforcement actions",
                )
            })?;

            let result = resolve_report_with_enforcement(
                &state,
                &tenant,
                &report_detail,
                &body.action,
                body.reason.as_deref(),
                timeout_until,
                request_id,
                &actor_pubkey,
                actor_role_str,
                actor_authority,
            )
            .await
            .map_err(|e| match e {
                ResolutionError::NotFound => ApiError::not_found(),
                ResolutionError::NotOpen(status) => ApiError::conflict(&format!(
                    "report is not open (current status: {status})"
                )),
                ResolutionError::InvalidAction(msg) => ApiError::bad_request("invalid_action", &msg),
                ResolutionError::EnforcementFailed { action_id, error } => {
                    ApiError::unprocessable(&format!(
                        "enforcement failed (action_id={action_id}): {error}"
                    ))
                }
                ResolutionError::Internal(msg) => {
                    tracing::error!(report_id = %report_id, error = %msg, "resolve_report internal error");
                    ApiError::internal()
                }
            })?;

            Ok(axum::http::Response::builder()
                .status(200)
                .header(header::CONTENT_TYPE, "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({
                        "status": "resolved",
                        "actionId": result.action_id.to_string(),
                    })
                    .to_string(),
                ))
                .unwrap())
        }
    }
}

/// PATCH /feedback/{id}
///
/// Update product_feedback status. Requires nip98 auth.
async fn update_feedback_status(
    State(state): State<Arc<crate::state::AppState>>,
    uri: Uri,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body_bytes: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let principal_opt = authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "PATCH",
        Some(&body_bytes),
    )
    .await?;

    let _principal = require_mutation_principal(principal_opt)?;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct FeedbackStatusBody {
        status: String,
    }

    let body: FeedbackStatusBody = serde_json::from_slice(&body_bytes)
        .map_err(|_| ApiError::bad_request("invalid_body", "invalid JSON body"))?;

    let allowed = ["new", "reviewed", "archived"];
    if !allowed.contains(&body.status.as_str()) {
        return Err(ApiError::bad_request(
            "invalid_status",
            "status must be new|reviewed|archived",
        ));
    }

    let updated = state.db.update_feedback_status(id, &body.status).await?;
    if !updated {
        return Err(ApiError::not_found());
    }

    Ok(Json(serde_json::json!({"status": body.status})))
}

// ── Phase 2: Staffing endpoints ───────────────────────────────────────────────

/// Effective principal entry returned by GET /operators.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OperatorEntry {
    /// Hex-encoded pubkey.
    pubkey: String,
    /// Effective role: `"operator"` | `"moderator"`.
    effective_role: String,
    /// Sources contributing to this principal's grant.
    sources: Vec<String>,
}

/// GET /operators
///
/// List all effective principals (union of config and DB). Source-aware.
/// Requires nip98 auth + Operator role.
async fn list_operators(
    State(state): State<Arc<crate::state::AppState>>,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Json<Vec<OperatorEntry>>, ApiError> {
    let principal_opt = authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "GET",
        None,
    )
    .await?;

    let principal = require_mutation_principal(principal_opt)?;
    require_operator(&principal)?;

    let config = state
        .config
        .admin
        .as_ref()
        .ok_or_else(ApiError::not_found)?;
    let _ = config; // admin config present — we already passed auth

    // Build effective principal set.
    let mut entries: Vec<OperatorEntry> = vec![];

    // 1. Config-backed operators (RELAY_OPERATOR_PUBKEYS).
    for hex_key in &state.config.relay_operator_pubkeys {
        entries.push(OperatorEntry {
            pubkey: hex_key.clone(),
            effective_role: "operator".to_string(),
            sources: vec!["config".to_string()],
        });
    }

    // 2. Owner fallback B: implicit operator when RELAY_OPERATOR_PUBKEYS is empty.
    if state.config.relay_operator_pubkeys.is_empty() {
        if let Some(owner_hex) = &state.config.relay_owner_pubkey {
            entries.push(OperatorEntry {
                pubkey: owner_hex.clone(),
                effective_role: "operator".to_string(),
                sources: vec!["owner_fallback".to_string()],
            });
        }
    }

    // 3. DB rows. Config outranks DB: if a DB entry is already covered by config,
    //    add "db" to its sources rather than creating a duplicate.
    let db_rows = state.db.list_relay_operators().await?;
    let config_pubkeys: std::collections::HashSet<&str> = state
        .config
        .relay_operator_pubkeys
        .iter()
        .map(String::as_str)
        .collect();

    for row in db_rows {
        let hex = hex::encode(&row.pubkey);
        if config_pubkeys.contains(hex.as_str()) {
            // Config already grants Operator; annotate sources but don't demote.
            if let Some(e) = entries.iter_mut().find(|e| e.pubkey == hex) {
                e.sources.push("db".to_string());
            }
        } else {
            entries.push(OperatorEntry {
                pubkey: hex,
                effective_role: row.role.clone(),
                sources: vec!["db".to_string()],
            });
        }
    }

    Ok(Json(entries))
}

/// Request body for PUT /operators/{pubkey}.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpsertOperatorBody {
    role: String,
}

/// PUT /operators/{pubkey}
///
/// Idempotent upsert of a DB operator/moderator row.
/// Returns 409 if the target pubkey is config-backed (immutable through the API).
/// Requires nip98 auth + Operator role.
async fn upsert_operator(
    State(state): State<Arc<crate::state::AppState>>,
    uri: Uri,
    headers: HeaderMap,
    Path(pubkey_hex): Path<String>,
    body_bytes: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let principal_opt = authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "PUT",
        Some(&body_bytes),
    )
    .await?;

    let principal = require_mutation_principal(principal_opt)?;
    require_operator(&principal)?;

    // Decode target pubkey.
    let target_bytes = decode_hex_pubkey(&pubkey_hex)?;

    // Reject if config-backed (immutable through the API).
    if is_config_backed_pubkey(&state.config, &pubkey_hex) {
        return Err(ApiError::conflict(
            "pubkey is backed by config (RELAY_OPERATOR_PUBKEYS or owner fallback) — immutable through the API",
        ));
    }

    let body: UpsertOperatorBody = serde_json::from_slice(&body_bytes)
        .map_err(|_| ApiError::bad_request("invalid_body", "invalid JSON body"))?;

    if !["operator", "moderator"].contains(&body.role.as_str()) {
        return Err(ApiError::bad_request(
            "invalid_role",
            "role must be operator|moderator",
        ));
    }

    state
        .db
        .upsert_relay_operator(&target_bytes, &body.role, &principal.pubkey)
        .await?;

    Ok(Json(
        serde_json::json!({"pubkey": pubkey_hex, "role": body.role}),
    ))
}

/// DELETE /operators/{pubkey}
///
/// Remove a DB operator/moderator row.
/// Returns 409 if the target pubkey is config-backed.
/// Requires nip98 auth + Operator role.
async fn delete_operator(
    State(state): State<Arc<crate::state::AppState>>,
    uri: Uri,
    headers: HeaderMap,
    Path(pubkey_hex): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let principal_opt = authorize(
        &state,
        &headers,
        uri.path_and_query()
            .map_or_else(|| uri.path(), |pq| pq.as_str()),
        "DELETE",
        None,
    )
    .await?;

    let principal = require_mutation_principal(principal_opt)?;
    require_operator(&principal)?;

    // Reject if config-backed.
    if is_config_backed_pubkey(&state.config, &pubkey_hex) {
        return Err(ApiError::conflict(
            "pubkey is backed by config (RELAY_OPERATOR_PUBKEYS or owner fallback) — immutable through the API",
        ));
    }

    let target_bytes = decode_hex_pubkey(&pubkey_hex)?;
    let removed = state.db.remove_relay_operator(&target_bytes).await?;
    if !removed {
        return Err(ApiError::not_found());
    }

    Ok(Json(serde_json::json!({"deleted": pubkey_hex})))
}

// ── Staffing helpers ──────────────────────────────────────────────────────────

/// Returns true if the hex pubkey is covered by a config-backed grant
/// (RELAY_OPERATOR_PUBKEYS or owner-fallback B).
fn is_config_backed_pubkey(config: &crate::config::Config, pubkey_hex: &str) -> bool {
    if config
        .relay_operator_pubkeys
        .iter()
        .any(|k| k == pubkey_hex)
    {
        return true;
    }
    // Owner fallback B: only when RELAY_OPERATOR_PUBKEYS is empty.
    if config.relay_operator_pubkeys.is_empty() {
        if let Some(owner) = &config.relay_owner_pubkey {
            if owner == pubkey_hex {
                return true;
            }
        }
    }
    false
}

/// Decode a 64-character hex string into 32 bytes, returning 404 on failure.
fn decode_hex_pubkey(hex_str: &str) -> Result<Vec<u8>, ApiError> {
    if hex_str.len() != 64 {
        return Err(ApiError::not_found());
    }
    hex::decode(hex_str).map_err(|_| ApiError::not_found())
}

/// Decode a hex-encoded report target into (pubkey_bytes, event_id_bytes).
type TargetPairMod = (Option<Vec<u8>>, Option<Vec<u8>>);

fn decode_report_target_hex(target_kind: &str, target_hex: &str) -> Result<TargetPairMod, String> {
    match target_kind {
        "event" => {
            let bytes = hex::decode(target_hex).map_err(|e| e.to_string())?;
            Ok((None, Some(bytes)))
        }
        "pubkey" => {
            let bytes = hex::decode(target_hex).map_err(|e| e.to_string())?;
            Ok((Some(bytes), None))
        }
        "blob" => Ok((None, None)),
        other => Err(format!("unknown target_kind: {other}")),
    }
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
    use auth::ADMIN_API_PREFIX;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use sqlx::Row as _;
    use tower::ServiceExt;
    use uuid::Uuid;

    const TOKEN: &str = "5f0e1d2c3b4a59687786958493a2b1c0decadebeefcafe0123456789abcdef01";

    async fn test_state() -> Arc<crate::state::AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        let mut token = [0u8; 32];
        hex::decode_to_slice(TOKEN, &mut token).expect("test token is hex");
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Token(crate::config::AdminToken::from_bytes(token)),
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

    async fn disabled_mode_state() -> Arc<crate::state::AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Disabled,
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

    /// Every route the admin API mounts. Each must enforce the credential.
    fn mounted_routes() -> Vec<String> {
        let id = Uuid::nil();
        vec![
            "/reports".to_string(),
            format!("/reports/{id}"),
            "/feedback".to_string(),
            format!("/feedback/{id}"),
            format!("/feedback/{id}/attachments/{HASH}"),
        ]
    }

    fn authorized(uri: &str) -> axum::http::request::Builder {
        Request::builder()
            .uri(uri)
            .header(header::HOST, "admin.example")
            .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
    }

    fn status_request(builder: axum::http::request::Builder) -> Request<Body> {
        builder.body(Body::empty()).expect("request")
    }

    async fn status_for(
        state: Arc<crate::state::AppState>,
        request: Request<Body>,
    ) -> axum::response::Response {
        router(state).oneshot(request).await.expect("response")
    }

    #[tokio::test]
    async fn every_route_rejects_a_missing_credential_before_database_access() {
        let state = test_state().await;
        for uri in mounted_routes() {
            let response = status_for(
                state.clone(),
                Request::builder()
                    .uri(&uri)
                    .header(header::HOST, "admin.example")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
        }
    }

    #[tokio::test]
    async fn every_route_rejects_a_wrong_credential_before_database_access() {
        let state = test_state().await;
        let wrong = "f".repeat(64);
        for uri in mounted_routes() {
            let response = status_for(
                state.clone(),
                Request::builder()
                    .uri(&uri)
                    .header(header::HOST, "admin.example")
                    .header(header::AUTHORIZATION, format!("Bearer {wrong}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
        }
    }

    #[tokio::test]
    async fn malformed_credentials_all_collapse_to_the_same_challenge() {
        let state = test_state().await;
        for value in [
            format!("Basic {TOKEN}"),
            TOKEN.to_string(),
            "Bearer ".to_string(),
            "Bearer".to_string(),
            format!("Bearer {}", &TOKEN[..63]),
            format!("Bearer {TOKEN}00"),
            format!("Bearer {}", "z".repeat(64)),
        ] {
            let response = status_for(
                state.clone(),
                Request::builder()
                    .uri("/reports")
                    .header(header::HOST, "admin.example")
                    .header(header::AUTHORIZATION, &value)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{value}");
            assert_eq!(
                response
                    .headers()
                    .get(header::WWW_AUTHENTICATE)
                    .and_then(|value| value.to_str().ok()),
                Some("Bearer"),
                "{value}"
            );
        }
    }

    #[tokio::test]
    async fn duplicate_authorization_headers_are_rejected() {
        let response = status_for(
            test_state().await,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn lowercase_bearer_scheme_is_accepted() {
        // Use /probe (no DB dependency) to verify that a lowercase "bearer" scheme
        // is accepted and auth succeeds. A 200 from probe confirms auth passed.
        let response = status_for(
            test_state().await,
            Request::builder()
                .uri("/probe")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, format!("bearer {TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_valid_credential_on_the_wrong_host_is_forbidden_not_unauthorized() {
        let response = status_for(
            test_state().await,
            Request::builder()
                .uri(format!("/reports/{}", Uuid::nil()))
                .header(header::HOST, "community.example")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_valid_credential_with_a_mismatched_origin_is_forbidden() {
        let response = status_for(
            test_state().await,
            status_request(
                authorized(&format!("/reports/{}", Uuid::nil()))
                    .header(header::ORIGIN, "https://attacker.example"),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn a_valid_credential_on_the_admin_host_without_an_origin_is_served() {
        // Use /probe (no DB dependency) to confirm auth succeeds without an Origin header.
        let response = status_for(test_state().await, status_request(authorized("/probe"))).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_unauthenticated_request_on_the_wrong_host_reveals_no_host_oracle() {
        let state = test_state().await;
        let wrong_host = status_for(
            state.clone(),
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "community.example")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        let right_host = status_for(
            state,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "admin.example")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(wrong_host.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(right_host.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[ignore = "requires Postgres — DB lookup returns 500 without a database"]
    async fn report_detail_rejects_unknown_report() {
        let response = status_for(
            test_state().await,
            status_request(authorized(&format!("/reports/{}", Uuid::nil()))),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[ignore = "requires Postgres — DB lookup returns 500 without a database"]
    async fn feedback_attachment_rejects_unknown_feedback() {
        let response = status_for(
            test_state().await,
            status_request(authorized(&format!(
                "/feedback/{}/attachments/{HASH}",
                Uuid::nil()
            ))),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn feedback_attachment_rejects_write_methods() {
        let state = test_state().await;
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            let response = status_for(
                state.clone(),
                status_request(
                    authorized(&format!("/feedback/{}/attachments/{HASH}", Uuid::nil()))
                        .method(method),
                ),
            )
            .await;
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
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

    #[tokio::test]
    async fn disabled_mode_allows_unauthenticated_requests_on_the_admin_host() {
        let state = disabled_mode_state().await;
        for uri in mounted_routes() {
            let response = status_for(
                state.clone(),
                Request::builder()
                    .uri(&uri)
                    .header(header::HOST, "admin.example")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await;
            // The routes return 200 (or 404 for unknown resources) — never 401.
            // 404 is fine here: there is no real DB, so the row lookups fail.
            assert_ne!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{uri} must not return 401 in disabled mode"
            );
        }
    }

    #[tokio::test]
    async fn disabled_mode_still_requires_the_correct_host() {
        let state = disabled_mode_state().await;
        let response = status_for(
            state,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "community.example")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "wrong host must still be rejected in disabled mode"
        );
    }

    #[tokio::test]
    async fn disabled_mode_still_requires_a_matching_origin() {
        let state = disabled_mode_state().await;
        let response = status_for(
            state,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "admin.example")
                .header(header::ORIGIN, "https://attacker.example")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "mismatched origin must still be rejected in disabled mode"
        );
    }

    // ── NIP-98 mode helpers and tests ─────────────────────────────────────

    /// Replay guard that always returns `true` — every event is "fresh".
    /// Used in NIP-98 tests that don't specifically test replay protection.
    struct AlwaysFreshReplayGuard;

    impl buzz_auth::Nip98ReplayGuard for AlwaysFreshReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            _event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            Box::pin(async { Ok(true) })
        }
    }

    /// Replay guard that rejects any event ID it has seen before.
    /// Used to test that the replay guard is actually invoked and enforced.
    struct TrackingReplayGuard {
        seen: std::sync::Mutex<std::collections::HashSet<[u8; 32]>>,
    }

    impl TrackingReplayGuard {
        fn new() -> Self {
            Self {
                seen: std::sync::Mutex::new(std::collections::HashSet::new()),
            }
        }
    }

    impl buzz_auth::Nip98ReplayGuard for TrackingReplayGuard {
        fn try_mark_in_scope<'a>(
            &'a self,
            _scope: &'a str,
            event_id: &'a nostr::EventId,
            _ttl_secs: u64,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, buzz_auth::AuthError>> + Send + 'a>,
        > {
            let bytes = event_id.to_bytes();
            let is_fresh = self.seen.lock().unwrap().insert(bytes);
            Box::pin(async move { Ok(is_fresh) })
        }
    }

    /// Build a test AppState in nip98 mode with the given operator pubkeys
    /// (populated in relay_operator_pubkeys config) and an AlwaysFreshReplayGuard.
    async fn nip98_state(pubkeys: Vec<String>) -> Arc<crate::state::AppState> {
        nip98_state_with_replay(pubkeys, Arc::new(AlwaysFreshReplayGuard)).await
    }

    async fn nip98_state_with_replay(
        pubkeys: Vec<String>,
        replay: Arc<dyn buzz_auth::Nip98ReplayGuard>,
    ) -> Arc<crate::state::AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        // Populate relay_operator_pubkeys so resolve_admin_principal can grant
        // Operator/Config to the test pubkeys without a DB lookup.
        config.relay_operator_pubkeys = pubkeys;
        // Ensure relay_operator_api_origin is set (required when pubkeys is non-empty).
        if !config.relay_operator_pubkeys.is_empty() {
            config.relay_operator_api_origin = Some("https://admin.example".to_string());
        }
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Nip98,
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
        let (mut state, _audit_shutdown) = crate::state::AppState::new(
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
        state.nip98_replay = replay;
        Arc::new(state)
    }

    /// Build a NIP-98 Authorization header value for a GET to the given path
    /// on `admin.example` (the test host). The path should be the handler-level
    /// path (e.g. `/reports`); this helper prefixes it with `ADMIN_API_PREFIX`
    /// to match the canonical URL the auth layer constructs in production.
    fn make_nostr_auth(keys: &nostr::Keys, path: &str) -> String {
        use nostr::{EventBuilder, Kind, Tag};
        let url = format!("https://admin.example{ADMIN_API_PREFIX}{path}");
        let tags = vec![
            Tag::parse(["u", &url]).unwrap(),
            Tag::parse(["method", "GET"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign");
        let json = serde_json::to_string(&event).expect("serialize");
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine as _;
        format!("Nostr {}", BASE64.encode(json.as_bytes()))
    }

    #[tokio::test]
    async fn nip98_mode_rejects_missing_credential_with_nostr_challenge() {
        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        let response = status_for(
            state,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "admin.example")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok()),
            Some("Nostr"),
            "nip98 mode must advertise Nostr challenge"
        );
    }

    #[tokio::test]
    async fn nip98_mode_valid_event_from_operator_pubkey_is_served() {
        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        // Use /probe (no DB dependency) — config-backed operator resolves without DB.
        let auth = make_nostr_auth(&keys, "/probe");
        let response = status_for(
            state,
            Request::builder()
                .uri("/probe")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        // 200 from probe confirms the event was authenticated and operator was resolved.
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    #[ignore = "requires Postgres — DB lookup returns None for unknown key → 403"]
    async fn nip98_mode_valid_event_unknown_pubkey_is_403() {
        let operator = nostr::Keys::generate();
        let unknown = nostr::Keys::generate();
        let state = nip98_state(vec![operator.public_key().to_hex()]).await;
        let auth = make_nostr_auth(&unknown, "/reports");
        let response = status_for(
            state,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        // NIP-98 signature is valid but the pubkey has no operator/moderator
        // grant — that is an authorization failure (403), not an auth failure.
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn nip98_mode_duplicate_authorization_headers_are_401() {
        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        let auth = make_nostr_auth(&keys, "/reports");
        let response = status_for(
            state,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth.clone())
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn nip98_mode_wrong_url_in_event_is_401() {
        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        // Sign for /feedback but send to /reports — u-tag mismatch.
        let auth = make_nostr_auth(&keys, "/feedback");
        let response = status_for(
            state,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn nip98_mode_replay_is_rejected() {
        let keys = nostr::Keys::generate();
        let tracking = Arc::new(TrackingReplayGuard::new());
        let state =
            nip98_state_with_replay(vec![keys.public_key().to_hex()], tracking.clone()).await;
        // Use /probe (no DB dependency) to verify first request succeeds
        // and second (same event ID) is rejected by the replay guard.
        let auth = make_nostr_auth(&keys, "/probe");
        // First request succeeds.
        let first = status_for(
            state.clone(),
            Request::builder()
                .uri("/probe")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth.clone())
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        // Second request with the same event ID must be rejected.
        let second = status_for(
            state,
            Request::builder()
                .uri("/probe")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(second.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn nip98_mode_valid_credential_on_wrong_host_is_forbidden_not_unauthorized() {
        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        let auth = make_nostr_auth(&keys, "/reports");
        let response = status_for(
            state,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "community.example")
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    // ── Regression pins — token-mode and disabled-mode unchanged ──────────

    #[tokio::test]
    async fn token_mode_regression_pin_valid_credential_is_served() {
        // Use /probe (no DB dependency) to confirm token mode serves valid credentials.
        let response = status_for(test_state().await, status_request(authorized("/probe"))).await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn token_mode_regression_pin_missing_credential_is_401() {
        let state = test_state().await;
        let response = status_for(
            state,
            Request::builder()
                .uri("/reports")
                .header(header::HOST, "admin.example")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(header::WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok()),
            Some("Bearer"),
            "token mode must advertise Bearer challenge"
        );
    }

    #[tokio::test]
    async fn disabled_mode_regression_pin_unauthenticated_request_is_served() {
        let state = disabled_mode_state().await;
        // Use /probe (no DB dependency) to confirm disabled mode allows unauthenticated requests.
        let response = status_for(
            state,
            Request::builder()
                .uri("/probe")
                .header(header::HOST, "admin.example")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    // ── Query-bearing NIP-98 requests ────────────────────────────────────

    #[tokio::test]
    async fn nip98_mode_query_bearing_request_signed_with_full_url_is_served() {
        // Verify that the signed u-tag must include the query string; the relay
        // verifies against the full path-and-query, not just the path component.
        // We use /probe with a dummy query string (no DB dependency) to test the
        // URL-binding without hitting Postgres.
        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        let auth = make_nostr_auth(&keys, "/probe?mode=check");
        let response = status_for(
            state,
            Request::builder()
                .uri("/probe?mode=check")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        // 200 — full URL matched; not 401.
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn nip98_mode_path_only_event_for_query_bearing_request_is_401() {
        // A credential signed for just /probe must not authenticate a
        // request sent to /probe?mode=check: the u-tag would not match the
        // full canonical URL.
        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        let auth = make_nostr_auth(&keys, "/probe");
        let response = status_for(
            state,
            Request::builder()
                .uri("/probe?mode=check")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // ── Phase 1 acceptance tests ─────────────────────────────────────────
    //
    // Method-substitution and payload-tag checks are exercised via
    // authorize() directly (see auth::tests) — the admin API calls
    // authorize() per-handler after routing, so a POST to a GET-only route
    // returns 405 from the router before any auth code runs. The HTTP-level
    // integration tests for mutation endpoints live in Phase 2 once those
    // routes exist.

    // ── token/disabled mode probe tests ─────────────────────────────────

    #[tokio::test]
    async fn probe_in_token_mode_returns_no_role_and_no_capabilities() {
        let state = test_state().await; // token mode
        let response = status_for(state.clone(), status_request(authorized("/probe"))).await;
        assert_eq!(response.status(), StatusCode::OK);
        // Body should report no role and can_act=false.
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let probe: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(probe["authMode"], "token");
        assert!(probe["role"].is_null(), "token mode has no role");
        assert_eq!(probe["canAct"], false);
        assert_eq!(probe["canStaff"], false);
    }

    #[tokio::test]
    async fn probe_in_disabled_mode_returns_no_role_and_no_capabilities() {
        let state = disabled_mode_state().await;
        let response = status_for(
            state,
            Request::builder()
                .uri("/probe")
                .header(header::HOST, "admin.example")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let probe: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(probe["authMode"], "disabled");
        assert!(probe["role"].is_null(), "disabled mode has no role");
        assert_eq!(probe["canAct"], false);
        assert_eq!(probe["canStaff"], false);
    }

    /// Fallback B: when RELAY_OPERATOR_PUBKEYS is empty, RELAY_OWNER_PUBKEY is
    /// the implicit Operator and the probe returns role=operator, source=owner_fallback.
    #[tokio::test]
    async fn probe_in_nip98_mode_with_owner_fallback_b_returns_operator_role() {
        let owner_keys = nostr::Keys::generate();
        let mut config = crate::config::Config::from_env().expect("default config");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        // Empty operator list — activates fallback B.
        config.relay_operator_pubkeys = vec![];
        config.relay_owner_pubkey = Some(owner_keys.public_key().to_hex());
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Nip98,
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
        let (mut state, _) = crate::state::AppState::new(
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
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        let state = Arc::new(state);

        let auth_header = make_nostr_auth(&owner_keys, "/probe");
        let response = status_for(
            state,
            Request::builder()
                .uri("/probe")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth_header)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let probe: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(probe["authMode"], "nip98");
        assert_eq!(probe["role"], "operator");
        assert_eq!(probe["source"], "owner_fallback");
        assert_eq!(probe["canAct"], true);
        assert_eq!(probe["canStaff"], true);
    }

    /// Fallback B does NOT activate when RELAY_OPERATOR_PUBKEYS is non-empty:
    /// the owner key is then treated as an unknown pubkey → DB lookup → 403.
    #[tokio::test]
    #[ignore = "requires Postgres — owner key not in config, falls to DB lookup → 403"]
    async fn probe_owner_fallback_b_disabled_when_operator_pubkeys_nonempty() {
        let owner_keys = nostr::Keys::generate();
        let other_operator = nostr::Keys::generate();
        // Non-empty RELAY_OPERATOR_PUBKEYS — owner fallback should NOT apply.
        let state = nip98_state(vec![other_operator.public_key().to_hex()]).await;

        // Inject RELAY_OWNER_PUBKEY into the state config manually.
        // We need a fresh state with both set.
        let mut config = crate::config::Config::from_env().expect("default config");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_operator_pubkeys = vec![other_operator.public_key().to_hex()];
        config.relay_operator_api_origin = Some("https://admin.example".to_string());
        config.relay_owner_pubkey = Some(owner_keys.public_key().to_hex());
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Nip98,
            web_dir: None,
        });
        drop(state); // not used
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
        let (mut state, _) = crate::state::AppState::new(
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
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        let state = Arc::new(state);

        // Owner key signs a valid NIP-98 credential, but fallback B is OFF.
        let auth_header = make_nostr_auth(&owner_keys, "/probe");
        let response = status_for(
            state,
            Request::builder()
                .uri("/probe")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth_header)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        // Should be 403: valid NIP-98 credential, but no grant.
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    // ── Phase 2: mutation routes require nip98 ────────────────────────────

    /// Build a NIP-98 POST body-bearing Authorization header with `payload` sha256.
    fn make_nostr_auth_post(keys: &nostr::Keys, path: &str, body: &[u8]) -> String {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine as _;
        use nostr::{EventBuilder, Kind, Tag};
        use sha2::{Digest, Sha256};

        let url = format!("https://admin.example{ADMIN_API_PREFIX}{path}");
        let payload_hash = hex::encode(Sha256::digest(body));
        let tags = vec![
            Tag::parse(["u", &url]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &payload_hash]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign");
        let json = serde_json::to_string(&event).expect("serialize");
        format!("Nostr {}", BASE64.encode(json.as_bytes()))
    }

    fn make_nostr_auth_patch(keys: &nostr::Keys, path: &str, body: &[u8]) -> String {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine as _;
        use nostr::{EventBuilder, Kind, Tag};
        use sha2::{Digest, Sha256};

        let url = format!("https://admin.example{ADMIN_API_PREFIX}{path}");
        let payload_hash = hex::encode(Sha256::digest(body));
        let tags = vec![
            Tag::parse(["u", &url]).unwrap(),
            Tag::parse(["method", "PATCH"]).unwrap(),
            Tag::parse(["payload", &payload_hash]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign");
        let json = serde_json::to_string(&event).expect("serialize");
        format!("Nostr {}", BASE64.encode(json.as_bytes()))
    }

    fn make_nostr_auth_put(keys: &nostr::Keys, path: &str, body: &[u8]) -> String {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine as _;
        use nostr::{EventBuilder, Kind, Tag};
        use sha2::{Digest, Sha256};

        let url = format!("https://admin.example{ADMIN_API_PREFIX}{path}");
        let payload_hash = hex::encode(Sha256::digest(body));
        let tags = vec![
            Tag::parse(["u", &url]).unwrap(),
            Tag::parse(["method", "PUT"]).unwrap(),
            Tag::parse(["payload", &payload_hash]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign");
        let json = serde_json::to_string(&event).expect("serialize");
        format!("Nostr {}", BASE64.encode(json.as_bytes()))
    }

    fn make_nostr_auth_delete(keys: &nostr::Keys, path: &str) -> String {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine as _;
        use nostr::{EventBuilder, Kind, Tag};

        let url = format!("https://admin.example{ADMIN_API_PREFIX}{path}");
        let tags = vec![
            Tag::parse(["u", &url]).unwrap(),
            Tag::parse(["method", "DELETE"]).unwrap(),
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign");
        let json = serde_json::to_string(&event).expect("serialize");
        format!("Nostr {}", BASE64.encode(json.as_bytes()))
    }

    /// POST /reports/{id}/resolve in token mode → 403 (mutations require nip98).
    #[tokio::test]
    async fn mutation_routes_in_token_mode_return_403() {
        let state = test_state().await; // token mode
        let id = Uuid::nil();
        let body = r#"{"action":"dismiss"}"#.as_bytes().to_vec();
        let response = status_for(
            state,
            Request::builder()
                .method("POST")
                .uri(format!("/reports/{id}/resolve"))
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await;
        // Token mode: valid token passes auth, but require_mutation_principal
        // returns 403 because there is no AdminPrincipal in token mode.
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "mutations must require nip98"
        );
    }

    /// PATCH /feedback/{id} in token mode → 403.
    #[tokio::test]
    async fn feedback_status_patch_in_token_mode_returns_403() {
        let state = test_state().await;
        let id = Uuid::nil();
        let body = r#"{"status":"reviewed"}"#.as_bytes().to_vec();
        let response = status_for(
            state,
            Request::builder()
                .method("PATCH")
                .uri(format!("/feedback/{id}"))
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "patch must require nip98"
        );
    }

    /// GET /operators in token mode → 403.
    #[tokio::test]
    async fn list_operators_in_token_mode_returns_403() {
        let state = test_state().await;
        let response = status_for(
            state,
            Request::builder()
                .method("GET")
                .uri("/operators")
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "operators must require nip98"
        );
    }

    /// Moderator cannot access staffing endpoints.
    #[tokio::test]
    #[ignore = "requires Postgres — moderator DB lookup"]
    async fn moderator_cannot_access_staffing_endpoints() {
        // This test needs DB to resolve moderator role.
        // Covered by negative-matrix integration test suite.
    }

    /// Config-backed pubkey upsert → 409 Conflict.
    #[tokio::test]
    async fn upsert_config_backed_pubkey_returns_409() {
        let operator_keys = nostr::Keys::generate();
        let target_keys = nostr::Keys::generate();
        let target_hex = target_keys.public_key().to_hex();
        // Put target in config — makes it config-backed and immutable.
        let mut config = crate::config::Config::from_env().expect("default config");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_operator_pubkeys =
            vec![operator_keys.public_key().to_hex(), target_hex.clone()];
        config.relay_operator_api_origin = Some("https://admin.example".to_string());
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Nip98,
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
        let (mut state, _) = crate::state::AppState::new(
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
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        let state = Arc::new(state);

        let path = format!("/operators/{target_hex}");
        let body = r#"{"role":"moderator"}"#.as_bytes();
        let auth_header = make_nostr_auth_put(&operator_keys, &path, body);

        let response = status_for(
            state,
            Request::builder()
                .method("PUT")
                .uri(path)
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth_header)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_vec()))
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "config-backed pubkey must return 409"
        );
    }

    /// Config-backed pubkey delete → 409 Conflict.
    #[tokio::test]
    async fn delete_config_backed_pubkey_returns_409() {
        let operator_keys = nostr::Keys::generate();
        let target_keys = nostr::Keys::generate();
        let target_hex = target_keys.public_key().to_hex();
        let mut config = crate::config::Config::from_env().expect("default config");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_operator_pubkeys =
            vec![operator_keys.public_key().to_hex(), target_hex.clone()];
        config.relay_operator_api_origin = Some("https://admin.example".to_string());
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Nip98,
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
        let (mut state, _) = crate::state::AppState::new(
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
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        let state = Arc::new(state);

        let path = format!("/operators/{target_hex}");
        let auth_header = make_nostr_auth_delete(&operator_keys, &path);

        let response = status_for(
            state,
            Request::builder()
                .method("DELETE")
                .uri(path)
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth_header)
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "config-backed pubkey delete must return 409"
        );
    }

    /// Owner-fallback B config-backed pubkey upsert → 409.
    #[tokio::test]
    async fn upsert_owner_fallback_b_pubkey_returns_409() {
        // Owner fallback B: RELAY_OPERATOR_PUBKEYS empty, owner key is implicit operator.
        let owner_keys = nostr::Keys::generate();
        let owner_hex = owner_keys.public_key().to_hex();
        let mut config = crate::config::Config::from_env().expect("default config");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        config.relay_operator_pubkeys = vec![]; // activates fallback B
        config.relay_owner_pubkey = Some(owner_hex.clone());
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Nip98,
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
        let (mut state, _) = crate::state::AppState::new(
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
        state.nip98_replay = Arc::new(AlwaysFreshReplayGuard);
        let state = Arc::new(state);

        // Try to upsert the owner key (config-backed via fallback B) — should return 409.
        let path = format!("/operators/{owner_hex}");
        let body = r#"{"role":"moderator"}"#.as_bytes();
        let auth_header = make_nostr_auth_put(&owner_keys, &path, body);

        let response = status_for(
            state,
            Request::builder()
                .method("PUT")
                .uri(path)
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth_header)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_vec()))
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "owner fallback B key must return 409 on upsert"
        );
    }

    /// Method-substitution: a POST credential cannot authenticate a PATCH.
    #[tokio::test]
    async fn nip98_mutation_method_substitution_returns_401() {
        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        let id = Uuid::nil();
        let body = r#"{"status":"reviewed"}"#.as_bytes();

        // Sign a PATCH credential but send as POST — method mismatch → 401.
        let auth_header = make_nostr_auth_post(&keys, &format!("/feedback/{id}"), body);

        let response = status_for(
            state,
            Request::builder()
                .method("PATCH")
                .uri(format!("/feedback/{id}"))
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth_header)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_vec()))
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "method substitution must be rejected"
        );
    }

    /// Body substitution: credential signed for one body, different body sent → 401.
    #[tokio::test]
    async fn nip98_mutation_body_substitution_returns_401() {
        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        let id = Uuid::nil();
        let original_body = r#"{"status":"reviewed"}"#.as_bytes();
        let tampered_body = r#"{"status":"archived"}"#.as_bytes();

        // Credential is signed for `original_body` but we send `tampered_body`.
        let auth_header = make_nostr_auth_patch(&keys, &format!("/feedback/{id}"), original_body);

        let response = status_for(
            state,
            Request::builder()
                .method("PATCH")
                .uri(format!("/feedback/{id}"))
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth_header)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(tampered_body.to_vec()))
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "body substitution must be rejected"
        );
    }

    /// Missing payload tag on a body-bearing POST → 401.
    #[tokio::test]
    async fn nip98_mutation_missing_payload_tag_returns_401() {
        use base64::engine::general_purpose::STANDARD as BASE64;
        use base64::Engine as _;
        use nostr::{EventBuilder, Kind, Tag};

        let keys = nostr::Keys::generate();
        let state = nip98_state(vec![keys.public_key().to_hex()]).await;
        let id = Uuid::nil();
        let body = r#"{"action":"dismiss"}"#.as_bytes();

        // Sign NIP-98 for the URL and method but omit the `payload` tag.
        let url = format!("https://admin.example{ADMIN_API_PREFIX}/reports/{id}/resolve");
        let tags = vec![
            Tag::parse(["u", &url]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            // Intentionally no payload tag.
        ];
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign");
        let json = serde_json::to_string(&event).expect("serialize");
        let auth_header = format!("Nostr {}", BASE64.encode(json.as_bytes()));

        let response = status_for(
            state,
            Request::builder()
                .method("POST")
                .uri(format!("/reports/{id}/resolve"))
                .header(header::HOST, "admin.example")
                .header(header::AUTHORIZATION, auth_header)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_vec()))
                .expect("request"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "missing payload tag must be rejected"
        );
    }

    // ── Phase 2: DB-backed acceptance tests ───────────────────────────────
    //
    // These tests require Postgres and are tagged #[ignore]. They exercise the
    // full enforcement state machine including racing moderators, retry
    // idempotency, and community 9044 vs processing report.
    //
    // They delegate to the DB-layer tests in buzz_db::relay_admin_actions::tests
    // which directly exercise the state machine functions, proving the contracts
    // Paul's dispatch requires without needing the full HTTP stack.

    #[tokio::test]
    #[ignore = "requires Postgres — racing moderators, exactly one claim"]
    async fn racing_moderators_one_succeeds_one_gets_409() {
        // Covered by buzz_db relay_admin_actions::tests::racing_moderators_exactly_one_claim_one_conflict
        // Run: cargo test -p buzz-db relay_admin_actions::tests::racing -- --ignored
        //
        // Two concurrent POST /reports/{id}/resolve with different request_ids
        // against the same open report. Exactly one must succeed (200) and one
        // must return 409 (report not open). No orphan audit row.
        //
        // At the DB level: claim_report with two concurrent UUIDs on the same report_id.
        // FOR UPDATE row lock ensures serial execution; first commit wins, second
        // returns NotOpen. moderation_actions must have exactly 1 row.
        let pool = sqlx::PgPool::connect(
            &std::env::var("BUZZ_TEST_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string()),
        )
        .await
        .expect("connect to test DB");

        let community_id = {
            let id = uuid::Uuid::new_v4();
            let host = format!("admin-racing-test-{}.example", id.simple());
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(id)
                .bind(host)
                .execute(&pool)
                .await
                .expect("insert community");
            id
        };
        let report_id = {
            let row = sqlx::query(
                r#"
                INSERT INTO moderation_reports (community_id, report_event_id, reporter_pubkey, target_kind, target_pubkey, report_type)
                VALUES ($1, $2, $3, 'pubkey', $4, 'harassment')
                RETURNING id
                "#,
            )
            .bind(community_id)
            .bind({
                // report_event_id requires 32 bytes (Nostr event ID length).
                // Duplicate the UUID bytes to fill the 32-byte requirement.
                let uid = uuid::Uuid::new_v4();
                uid.as_bytes().iter().chain(uid.as_bytes().iter()).copied().collect::<Vec<u8>>()
            })
            .bind(vec![0u8; 32])
            .bind(vec![1u8; 32])
            .fetch_one(&pool)
            .await
            .expect("insert report");
            row.try_get::<uuid::Uuid, _>("id").expect("id")
        };

        let actor = vec![2u8; 32];
        let target = vec![1u8; 32];
        let cid = buzz_core::CommunityId::from_uuid(community_id);
        let req_a = uuid::Uuid::new_v4();
        let req_b = uuid::Uuid::new_v4();

        let claim = |request_id: uuid::Uuid,
                     pool: sqlx::PgPool,
                     actor: Vec<u8>,
                     target: Vec<u8>| async move {
            buzz_db::relay_admin_actions::claim_report(
                &pool,
                cid,
                report_id,
                request_id,
                &actor,
                "operator",
                "ban",
                None,
                None,
                "resolve:ban",
                "relay_operator",
                Some(&target),
                None,
                None,
            )
            .await
            .expect("claim_report")
        };

        let (ra, rb) = tokio::join!(
            claim(req_a, pool.clone(), actor.clone(), target.clone()),
            claim(req_b, pool.clone(), actor.clone(), target.clone()),
        );

        let outcomes = [&ra, &rb];
        let claimed_count = outcomes
            .iter()
            .filter(|r| matches!(r, buzz_db::relay_admin_actions::ClaimResult::Claimed(_)))
            .count();
        let conflict_count = outcomes
            .iter()
            .filter(|r| matches!(r, buzz_db::relay_admin_actions::ClaimResult::NotOpen(_)))
            .count();
        assert_eq!(claimed_count, 1, "exactly one claim must succeed");
        assert_eq!(conflict_count, 1, "exactly one must be rejected");

        let audit_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions WHERE community_id = $1")
                .bind(community_id)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(audit_count, 1, "no orphan audit row");
    }

    #[tokio::test]
    #[ignore = "requires Postgres — same request_id retry returns existing action record"]
    async fn same_request_id_retry_returns_existing_action() {
        // Two POST /reports/{id}/resolve calls with the same requestId UUID.
        // Both should return 200 with the same actionId.
        let pool = sqlx::PgPool::connect(
            &std::env::var("BUZZ_TEST_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string()),
        )
        .await
        .expect("connect to test DB");

        let community_id = {
            let id = uuid::Uuid::new_v4();
            let host = format!("admin-idempotent-test-{}.example", id.simple());
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(id)
                .bind(host)
                .execute(&pool)
                .await
                .expect("insert community");
            id
        };
        let report_id = {
            let row = sqlx::query(
                r#"
                INSERT INTO moderation_reports (community_id, report_event_id, reporter_pubkey, target_kind, target_pubkey, report_type)
                VALUES ($1, $2, $3, 'pubkey', $4, 'harassment')
                RETURNING id
                "#,
            )
            .bind(community_id)
            .bind({
                // report_event_id requires 32 bytes (Nostr event ID length).
                // Duplicate the UUID bytes to fill the 32-byte requirement.
                let uid = uuid::Uuid::new_v4();
                uid.as_bytes().iter().chain(uid.as_bytes().iter()).copied().collect::<Vec<u8>>()
            })
            .bind(vec![0u8; 32])
            .bind(vec![1u8; 32])
            .fetch_one(&pool)
            .await
            .expect("insert report");
            row.try_get::<uuid::Uuid, _>("id").expect("id")
        };

        let actor = vec![2u8; 32];
        let target = vec![1u8; 32];
        let cid = buzz_core::CommunityId::from_uuid(community_id);
        let request_id = uuid::Uuid::new_v4();

        let first = buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            request_id,
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("first claim");
        let first_id = match first {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        let second = buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            request_id,
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("second claim");
        let second_id = match second {
            buzz_db::relay_admin_actions::ClaimResult::AlreadyClaimed(a) => a.id,
            other => panic!("expected AlreadyClaimed, got {other:?}"),
        };

        assert_eq!(
            first_id, second_id,
            "same request_id must return same action id"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres — community 9044 against processing report fails cleanly"]
    async fn community_9044_against_processing_report_fails_cleanly() {
        // A community 9044 event against a processing report must fail the CAS
        // on status='open' and return an error. The enforcement must not be duplicated.
        //
        // resolve_report_decision_atomic CASes on status='open'; if the report is
        // already 'processing', the transaction rolls back with no audit row.
        let pool = sqlx::PgPool::connect(
            &std::env::var("BUZZ_TEST_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string()),
        )
        .await
        .expect("connect to test DB");

        let community_id = {
            let id = uuid::Uuid::new_v4();
            let host = format!("admin-9044-test-{}.example", id.simple());
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(id)
                .bind(host)
                .execute(&pool)
                .await
                .expect("insert community");
            id
        };
        let report_id = {
            let row = sqlx::query(
                r#"
                INSERT INTO moderation_reports (community_id, report_event_id, reporter_pubkey, target_kind, target_pubkey, report_type)
                VALUES ($1, $2, $3, 'pubkey', $4, 'harassment')
                RETURNING id
                "#,
            )
            .bind(community_id)
            .bind({
                // report_event_id requires 32 bytes (Nostr event ID length).
                // Duplicate the UUID bytes to fill the 32-byte requirement.
                let uid = uuid::Uuid::new_v4();
                uid.as_bytes().iter().chain(uid.as_bytes().iter()).copied().collect::<Vec<u8>>()
            })
            .bind(vec![0u8; 32])
            .bind(vec![1u8; 32])
            .fetch_one(&pool)
            .await
            .expect("insert report");
            row.try_get::<uuid::Uuid, _>("id").expect("id")
        };

        let actor = vec![2u8; 32];
        let target = vec![1u8; 32];
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        // HTTP enforcement claim moves report to 'processing'.
        let _ = buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("enforcement claim");

        // Community 9044 (decision-only) against the now-processing report must fail.
        let result = buzz_db::relay_admin_actions::resolve_report_decision_atomic(
            &pool,
            cid,
            report_id,
            "dismissed",
            "dismiss_report",
            &actor,
            "community",
            Some(&target),
            None,
            None,
            None,
        )
        .await
        .expect("decision-only attempt");

        assert!(!result, "9044 against processing report must fail the CAS");

        // Only one audit row — from the enforcement claim.
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions WHERE community_id = $1")
                .bind(community_id)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count, 1, "no duplicate audit rows from failed 9044");
    }

    #[tokio::test]
    #[ignore = "requires Postgres — cancel rejected after mutation success"]
    async fn cancel_after_mutation_success_is_rejected() {
        // After an enforcement action reaches mutation_committed step_marker,
        // attempting to cancel the action record must fail (cancel is only
        // legal pre-mutation).
        let pool = sqlx::PgPool::connect(
            &std::env::var("BUZZ_TEST_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string()),
        )
        .await
        .expect("connect to test DB");

        let community_id = {
            let id = uuid::Uuid::new_v4();
            let host = format!("admin-cancel-test-{}.example", id.simple());
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(id)
                .bind(host)
                .execute(&pool)
                .await
                .expect("insert community");
            id
        };
        let report_id = {
            let row = sqlx::query(
                r#"
                INSERT INTO moderation_reports (community_id, report_event_id, reporter_pubkey, target_kind, target_pubkey, report_type)
                VALUES ($1, $2, $3, 'pubkey', $4, 'harassment')
                RETURNING id
                "#,
            )
            .bind(community_id)
            .bind({
                // report_event_id requires 32 bytes (Nostr event ID length).
                // Duplicate the UUID bytes to fill the 32-byte requirement.
                let uid = uuid::Uuid::new_v4();
                uid.as_bytes().iter().chain(uid.as_bytes().iter()).copied().collect::<Vec<u8>>()
            })
            .bind(vec![0u8; 32])
            .bind(vec![1u8; 32])
            .fetch_one(&pool)
            .await
            .expect("insert report");
            row.try_get::<uuid::Uuid, _>("id").expect("id")
        };

        let actor = vec![2u8; 32];
        let target = vec![1u8; 32];
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        let _ = buzz_db::relay_admin_actions::commit_mutation_step(&pool, action_id)
            .await
            .expect("commit_mutation_step");

        let cancelled =
            buzz_db::relay_admin_actions::cancel_action(&pool, action_id, cid, report_id)
                .await
                .expect("cancel_action");
        assert!(
            !cancelled,
            "cancel after mutation_committed must be rejected"
        );
    }

    #[tokio::test]
    #[ignore = "requires Postgres — worker crash re-drive convergence"]
    async fn worker_crash_redrive_converges_to_exactly_one_enforcement() {
        // Simulate a crash after mutation_committed but before finalization.
        // Re-drive from persisted step state must produce exactly one
        // enforcement, one report transition, one audit chain, one reporter notice.
        let pool = sqlx::PgPool::connect(
            &std::env::var("BUZZ_TEST_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string()),
        )
        .await
        .expect("connect to test DB");

        let community_id = {
            let id = uuid::Uuid::new_v4();
            let host = format!("admin-redrive-test-{}.example", id.simple());
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(id)
                .bind(host)
                .execute(&pool)
                .await
                .expect("insert community");
            id
        };
        let report_id = {
            let row = sqlx::query(
                r#"
                INSERT INTO moderation_reports (community_id, report_event_id, reporter_pubkey, target_kind, target_pubkey, report_type)
                VALUES ($1, $2, $3, 'pubkey', $4, 'harassment')
                RETURNING id
                "#,
            )
            .bind(community_id)
            .bind({
                // report_event_id requires 32 bytes (Nostr event ID length).
                // Duplicate the UUID bytes to fill the 32-byte requirement.
                let uid = uuid::Uuid::new_v4();
                uid.as_bytes().iter().chain(uid.as_bytes().iter()).copied().collect::<Vec<u8>>()
            })
            .bind(vec![0u8; 32])
            .bind(vec![1u8; 32])
            .fetch_one(&pool)
            .await
            .expect("insert report");
            row.try_get::<uuid::Uuid, _>("id").expect("id")
        };

        let actor = vec![2u8; 32];
        let target = vec![1u8; 32];
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        let _ = buzz_db::relay_admin_actions::commit_mutation_step(&pool, action_id)
            .await
            .expect("commit_mutation_step");

        // Simulate crash-before-finalization: re-load action.
        let reloaded = buzz_db::relay_admin_actions::get_action(&pool, action_id)
            .await
            .expect("get_action")
            .expect("action exists");
        assert_eq!(reloaded.step_marker.as_deref(), Some("mutation_committed"));
        assert_eq!(reloaded.state, "enforcing");

        // Re-drive: finalize from persisted state (step_marker present → skip mutation).
        let finalized = buzz_db::relay_admin_actions::finalize_success(
            &pool,
            action_id,
            cid,
            report_id,
            "resolved",
            &actor,
            "ban",
            Some(&actor),
            None,
            None,
            None,
        )
        .await
        .expect("finalize_success");
        assert!(finalized, "re-drive must finalize to succeeded");

        // Second finalize call must be idempotent (CAS fails but action is succeeded).
        let second_finalize = buzz_db::relay_admin_actions::finalize_success(
            &pool,
            action_id,
            cid,
            report_id,
            "resolved",
            &actor,
            "ban",
            Some(&actor),
            None,
            None,
            None,
        )
        .await
        .expect("second finalize_success");
        assert!(
            !second_finalize,
            "second finalize must return false (already succeeded)"
        );

        // Report is resolved.
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM moderation_reports WHERE id = $1")
                .bind(report_id)
                .fetch_optional(&pool)
                .await
                .expect("fetch report");
        assert_eq!(status.as_deref(), Some("resolved"));

        // Outbox rows are written in the finalize_success transaction (success-gated delivery).
        let outbox_rows = buzz_db::relay_admin_actions::list_pending_outbox(&pool, action_id)
            .await
            .expect("list outbox");
        // After finalization the outbox rows are still pending (worker hasn't run).
        // They must exist so the worker can deliver them.
        assert!(
            !outbox_rows.is_empty() || {
                // Also check delivered rows (if worker ran).
                let delivered: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM relay_admin_outbox WHERE action_id = $1",
                )
                .bind(action_id)
                .fetch_one(&pool)
                .await
                .expect("count outbox");
                delivered > 0
            },
            "outbox must have rows for reporter_notice delivery"
        );

        // Exactly one audit row.
        let audit_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions WHERE community_id = $1")
                .bind(community_id)
                .fetch_one(&pool)
                .await
                .expect("count audit");
        assert_eq!(audit_count, 1, "exactly one audit row after re-drive");
    }

    // ── E2E state-machine tests through the production driver/workers ─────────
    //
    // These tests drive through the actual production code paths:
    // `resolve_report_with_enforcement` (claim + drive_enforcement + finalize),
    // `drive_enforcement_pub` (action recovery worker re-drive path), and the
    // outbox retry mechanics. They require a live Postgres instance.

    /// Build an AppState wired to the given pool. Used by the e2e driver tests so
    /// they share the same DB connection the test fixtures wrote to.
    async fn state_from_pool(pool: sqlx::PgPool) -> Arc<crate::state::AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        let mut token = [0u8; 32];
        hex::decode_to_slice(TOKEN, &mut token).expect("test token is hex");
        config.admin = Some(crate::config::AdminConfig {
            host: "admin.example".to_string(),
            auth: crate::config::AdminAuth::Token(crate::config::AdminToken::from_bytes(token)),
            web_dir: None,
        });
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

    async fn e2e_pool() -> sqlx::PgPool {
        let url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
        sqlx::PgPool::connect(&url)
            .await
            .expect("connect to test DB")
    }

    async fn e2e_community(pool: &sqlx::PgPool, label: &str) -> (uuid::Uuid, String) {
        let id = uuid::Uuid::new_v4();
        let host = format!("e2e-{label}-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&host)
            .execute(pool)
            .await
            .expect("insert community");
        (id, host)
    }

    async fn e2e_report_pubkey(
        pool: &sqlx::PgPool,
        community_id: uuid::Uuid,
        target: &[u8],
    ) -> uuid::Uuid {
        let reporter = vec![0u8; 32];
        let uid = uuid::Uuid::new_v4();
        let event_id: Vec<u8> = uid
            .as_bytes()
            .iter()
            .chain(uid.as_bytes().iter())
            .copied()
            .collect();
        sqlx::query_scalar(
            r#"
            INSERT INTO moderation_reports (
                community_id, report_event_id, reporter_pubkey, target_kind,
                target_pubkey, report_type
            ) VALUES ($1, $2, $3, 'pubkey', $4, 'harassment')
            RETURNING id
            "#,
        )
        .bind(community_id)
        .bind(event_id)
        .bind(&reporter)
        .bind(target)
        .fetch_one(pool)
        .await
        .expect("insert report")
    }

    fn e2e_tenant(community_id: uuid::Uuid, host: &str) -> buzz_core::tenant::TenantContext {
        buzz_core::tenant::TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(community_id),
            host.to_string(),
        )
    }

    fn e2e_admin_report(
        report_id: uuid::Uuid,
        community_id: uuid::Uuid,
        target: &[u8],
    ) -> buzz_db::admin_moderation::AdminReportDetail {
        // Minimal AdminReportDetail sufficient to drive enforcement (ban action).
        // target_kind = "pubkey", target = hex of target bytes.
        buzz_db::admin_moderation::AdminReportDetail {
            report: buzz_db::admin_moderation::AdminReport {
                id: report_id,
                community_id,
                community_host: "e2e.example".to_string(),
                report_event_id: "0".repeat(64),
                reporter_pubkey: "0".repeat(64),
                target_kind: "pubkey".to_string(),
                target: hex::encode(target),
                channel_id: None,
                report_type: "harassment".to_string(),
                note: None,
                status: "open".to_string(),
                resolved_by: None,
                resolved_at: None,
                action_id: None,
                created_at: chrono::Utc::now(),
            },
            message: None,
        }
    }

    // ── 1. delete-then-crash-before-tombstone re-drive ────────────────────────

    #[tokio::test]
    #[ignore = "requires Postgres — delete crash-before-tombstone re-drive"]
    async fn delete_then_crash_before_tombstone_redrive() {
        // Simulate: DELETE action with atomic mutation+marker committed, crash
        // before finalization. Re-drive via `recover_one` (the actual action
        // recovery worker entry point) must finalize and create the tombstone +
        // reporter_notice outbox rows.
        let pool = e2e_pool().await;
        let (community_id, _host) = e2e_community(&pool, "crash-before-tombstone").await;
        let target_event_id: Vec<u8> = {
            let u = uuid::Uuid::new_v4();
            u.as_bytes()
                .iter()
                .chain(u.as_bytes().iter())
                .copied()
                .collect()
        };
        let actor = vec![5u8; 32];
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        // Create a channel and insert the target event into it.
        let channel_id = uuid::Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by)
               VALUES ($1, $2, 'crash-tombstone-ch', 'stream', 'open', $3)"#,
        )
        .bind(channel_id)
        .bind(community_id)
        .bind(&actor)
        .execute(&pool)
        .await
        .expect("create channel");

        // Insert a minimal event row (sig = 64 zero bytes, all required fields).
        let sig = vec![0u8; 64];
        sqlx::query(
            r#"INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id)
               VALUES ($1, $2, $3, now(), 1, '[]', 'test', $4, now(), $5)"#,
        )
        .bind(community_id)
        .bind(target_event_id.as_slice())
        .bind(&actor)
        .bind(sig.as_slice())
        .bind(channel_id)
        .execute(&pool).await.expect("insert event");

        // Create a target_kind='event' report that includes channel_id (so the
        // finalization creates a tombstone outbox row).
        let reporter = vec![0u8; 32];
        let report_event_raw: Vec<u8> = {
            let u = uuid::Uuid::new_v4();
            u.as_bytes()
                .iter()
                .chain(u.as_bytes().iter())
                .copied()
                .collect()
        };
        let report_id: uuid::Uuid = sqlx::query_scalar(
            r#"INSERT INTO moderation_reports
               (community_id, report_event_id, reporter_pubkey, target_kind, target_event_id,
                channel_id, report_type)
               VALUES ($1, $2, $3, 'event', $4, $5, 'harassment') RETURNING id"#,
        )
        .bind(community_id)
        .bind(report_event_raw.as_slice())
        .bind(&reporter)
        .bind(target_event_id.as_slice())
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("insert event report");

        // Step 1: claim DELETE action, advance to enforcing, acquire lease,
        // atomically execute delete mutation + step_marker.
        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "delete",
            Some("e2e test"),
            None,
            "resolve:delete",
            "relay_operator",
            None,
            Some(target_event_id.as_slice()),
            Some(channel_id),
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(60);
        let lease_token =
            match buzz_db::relay_admin_actions::acquire_action_lease(&pool, action_id, lease_until)
                .await
                .expect("acquire lease")
            {
                buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
                other => panic!("expected Acquired, got {other:?}"),
            };

        // Execute the delete mutation + step_marker atomically (simulates normal
        // execution; crash happens before finalization below).
        let committed = buzz_db::relay_admin_actions::execute_delete_with_marker(
            &pool,
            action_id,
            lease_token,
            cid,
            target_event_id.as_slice(),
            None, // no parent
            None,
        )
        .await
        .expect("execute_delete_with_marker");
        assert!(committed, "delete mutation+marker must commit");

        // Crash point: step_marker is set but action not yet finalized.
        let rec = buzz_db::relay_admin_actions::get_action(&pool, action_id)
            .await
            .expect("get_action")
            .expect("action exists");
        assert_eq!(rec.step_marker.as_deref(), Some("mutation_committed"));
        assert_eq!(
            rec.state, "enforcing",
            "must still be enforcing (not yet finalized)"
        );

        // No outbox rows yet (success-gated delivery).
        let outbox_before: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM relay_admin_outbox WHERE action_id = $1")
                .bind(action_id)
                .fetch_one(&pool)
                .await
                .expect("count outbox before");
        assert_eq!(outbox_before, 0, "no outbox rows before finalization");

        // Step 2: re-drive via recover_one — the actual action recovery worker
        // entry point. Expire the lease so the worker can re-claim it.
        let expired = chrono::Utc::now() - chrono::Duration::seconds(300);
        sqlx::query("UPDATE relay_admin_actions SET action_lease_expires_at = $2 WHERE id = $1")
            .bind(action_id)
            .bind(expired)
            .execute(&pool)
            .await
            .expect("expire lease");

        let batch = buzz_db::relay_admin_actions::claim_stranded_action_batch(
            &pool,
            "e2e-crash-worker",
            chrono::Utc::now() + chrono::Duration::seconds(120),
            1000,
        )
        .await
        .expect("claim_stranded_action_batch");

        let claim = batch
            .into_iter()
            .find(|c| c.record.id == action_id)
            .expect("stranded action must appear in batch");

        let state = state_from_pool(pool.clone()).await;
        // Call through recover_one — the real production worker entry point.
        crate::handlers::admin_action_worker::recover_one(&state, claim).await;

        // Verify: action succeeded, report resolved, tombstone + reporter_notice created.
        let final_rec = buzz_db::relay_admin_actions::get_action(&pool, action_id)
            .await
            .expect("get_action after recover_one")
            .expect("action still exists");
        assert_eq!(
            final_rec.state, "succeeded",
            "action must be succeeded after recover_one"
        );

        let report_status: Option<String> =
            sqlx::query_scalar("SELECT status FROM moderation_reports WHERE id = $1")
                .bind(report_id)
                .fetch_optional(&pool)
                .await
                .expect("fetch report status");
        assert_eq!(
            report_status.as_deref(),
            Some("resolved"),
            "report must be resolved"
        );

        // Both tombstone (for 'delete' + channel_id) and reporter_notice must exist.
        let outbox_rows: Vec<String> = sqlx::query_scalar(
            "SELECT task_type FROM relay_admin_outbox WHERE action_id = $1 ORDER BY task_type",
        )
        .bind(action_id)
        .fetch_all(&pool)
        .await
        .expect("fetch outbox rows");

        assert!(
            outbox_rows.iter().any(|t| t == "tombstone"),
            "tombstone outbox row must exist after delete finalization; got: {outbox_rows:?}"
        );
        assert!(
            outbox_rows.iter().any(|t| t == "reporter_notice"),
            "reporter_notice outbox row must exist; got: {outbox_rows:?}"
        );

        // Idempotent re-drive: a second recover_one must not double-finalize.
        // Expire the lease again so the stranded batch can pick it up (but action is now
        // 'succeeded' so it won't be returned by claim_stranded_action_batch).
        let batch2 = buzz_db::relay_admin_actions::claim_stranded_action_batch(
            &pool,
            "e2e-crash-worker-2",
            chrono::Utc::now() + chrono::Duration::seconds(120),
            1000,
        )
        .await
        .expect("second claim_stranded_action_batch");
        assert!(
            !batch2.iter().any(|c| c.record.id == action_id),
            "succeeded action must not appear in stranded batch (idempotent)"
        );

        let outbox_after_idempotent: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM relay_admin_outbox WHERE action_id = $1")
                .bind(action_id)
                .fetch_one(&pool)
                .await
                .expect("count outbox stable");
        assert_eq!(
            outbox_after_idempotent,
            outbox_rows.len() as i64,
            "idempotent re-drive must not create duplicate outbox rows"
        );

        // Step 3: deliver the tombstone outbox row via deliver_one — the real
        // outbox worker delivery entry point. Requirement: DELETE with tombstone delivery.
        let tombstone_outbox: (uuid::Uuid, serde_json::Value) = sqlx::query_as(
            "SELECT id, payload FROM relay_admin_outbox WHERE action_id = $1 AND task_type = 'tombstone'",
        )
        .bind(action_id)
        .fetch_one(&pool)
        .await
        .expect("fetch tombstone outbox row");
        let tombstone_outbox_id = tombstone_outbox.0;

        // Claim the tombstone row so deliver_one has a token.
        let outbox_lease_until = chrono::Utc::now() + chrono::Duration::seconds(30);
        let mut outbox_batch = state
            .db
            .claim_pending_admin_outbox_batch("tombstone-delivery-worker", outbox_lease_until, 100)
            .await
            .expect("claim tombstone outbox batch");
        let outbox_row_idx = outbox_batch
            .iter()
            .position(|r| r.id == tombstone_outbox_id)
            .expect("tombstone outbox row must be in batch");
        let outbox_row = outbox_batch.remove(outbox_row_idx);

        crate::handlers::admin_outbox_worker::deliver_one(&state, &outbox_row).await;

        // Assert: tombstone system message event is durably persisted.
        // emit_system_message writes a kind:40099 event into the target channel.
        let tombstone_event_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE community_id = $1 AND channel_id = $2 AND kind = 40099",
        )
        .bind(community_id)
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("count tombstone events");
        assert_eq!(
            tombstone_event_count, 1,
            "tombstone system message event must be durably persisted after deliver_one"
        );

        // Assert: tombstone outbox row is now delivered.
        let tombstone_state: String =
            sqlx::query_scalar("SELECT state FROM relay_admin_outbox WHERE id = $1")
                .bind(tombstone_outbox_id)
                .fetch_one(&pool)
                .await
                .expect("tombstone outbox state");
        assert_eq!(
            tombstone_state, "delivered",
            "tombstone outbox row must be marked delivered after deliver_one"
        );

        // Assert: target event has deleted_at set (the delete mutation committed
        // it when execute_delete_with_marker ran).
        // `deleted_at` is a nullable column — fetch_optional on a nullable column
        // yields Option<Option<T>>: outer None = row not found, inner None = NULL.
        let deleted_at: Option<Option<chrono::DateTime<chrono::Utc>>> =
            sqlx::query_scalar("SELECT deleted_at FROM events WHERE community_id = $1 AND id = $2")
                .bind(community_id)
                .bind(target_event_id.as_slice())
                .fetch_optional(&pool)
                .await
                .expect("fetch deleted_at");
        assert!(
            deleted_at.flatten().is_some(),
            "target event must have deleted_at set after DELETE action"
        );
    }

    // ── 2. kick retry after this action removed the member ────────────────────

    #[tokio::test]
    #[ignore = "requires Postgres — kick retry provenance: Removed vs AlreadyGone"]
    async fn kick_retry_after_this_action_removed_member() {
        // Action 1 kicks a member (Removed + marker committed). A re-drive of
        // action 1 must see AlreadyMarked (skip mutation) and succeed via finalize.
        // A second kick action (new report) must see AlreadyGone (enforcement failure).
        let pool = e2e_pool().await;
        let actor = vec![6u8; 32];
        let target = vec![7u8; 32];

        // Create community and channel.
        let (community_id, host) = e2e_community(&pool, "kick-provenance").await;
        let cid = buzz_core::CommunityId::from_uuid(community_id);
        let channel_id = uuid::Uuid::new_v4();
        sqlx::query(
            r#"INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by)
               VALUES ($1, $2, 'test-kick-e2e', 'stream', 'open', $3)"#,
        )
        .bind(channel_id)
        .bind(community_id)
        .bind(&actor)
        .execute(&pool)
        .await
        .expect("create channel");
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role) VALUES ($1, $2, $3, 'member')",
        )
        .bind(community_id).bind(channel_id).bind(&target)
        .execute(&pool).await.expect("add member");

        // Create report1 with channel_id.
        let report_event1: Vec<u8> = {
            let u = uuid::Uuid::new_v4();
            u.as_bytes()
                .iter()
                .chain(u.as_bytes().iter())
                .copied()
                .collect()
        };
        let report_id1: uuid::Uuid = sqlx::query_scalar(
            r#"INSERT INTO moderation_reports
               (community_id, report_event_id, reporter_pubkey, target_kind, target_pubkey, channel_id, report_type)
               VALUES ($1, $2, $3, 'pubkey', $4, $5, 'harassment') RETURNING id"#,
        )
        .bind(community_id).bind(&report_event1).bind(vec![0u8; 32])
        .bind(&target).bind(channel_id)
        .fetch_one(&pool).await.expect("insert report1");

        // Claim action1 for kick.
        let action_id1 = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id1,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "kick",
            None,
            None,
            "resolve:kick",
            "relay_operator",
            Some(&target),
            None,
            Some(channel_id),
        )
        .await
        .expect("claim1")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id1)
            .await
            .expect("begin_enforcing1");

        // Acquire lease for action_id1 (required by execute_kick_with_marker).
        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(60);
        let lease_token1 = match buzz_db::relay_admin_actions::acquire_action_lease(
            &pool,
            action_id1,
            lease_until,
        )
        .await
        .expect("acquire lease1")
        {
            buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired for action1, got {other:?}"),
        };

        // Kick: member is present → Removed + step_marker committed.
        let r1 = buzz_db::relay_admin_actions::execute_kick_with_marker(
            &pool,
            action_id1,
            lease_token1,
            cid,
            channel_id,
            &target,
            &actor,
        )
        .await
        .expect("kick1");
        assert!(
            matches!(
                r1,
                buzz_db::relay_admin_actions::KickWithMarkerResult::Removed
            ),
            "first kick must be Removed"
        );

        // Re-drive action1 via drive_enforcement_pub: sees marker set, skips kick,
        // goes to finalize → succeeded.
        let rec1 = buzz_db::relay_admin_actions::get_action(&pool, action_id1)
            .await
            .expect("get_action1")
            .expect("exists");
        let state = state_from_pool(pool.clone()).await;
        let tenant = e2e_tenant(community_id, &host);
        let result1 = crate::handlers::report_resolution::drive_enforcement_pub(
            &state,
            &tenant,
            cid,
            report_id1,
            "kick",
            None,
            None,
            &actor,
            Some(&target),
            None,
            Some(channel_id),
            &rec1,
            None,
        )
        .await;
        assert!(
            result1.is_ok(),
            "re-drive of action1 must succeed: {result1:?}"
        );

        let final_rec1 = buzz_db::relay_admin_actions::get_action(&pool, action_id1)
            .await
            .expect("get_action1 final")
            .expect("exists");
        assert_eq!(final_rec1.state, "succeeded", "action1 must succeed");

        // Create report2 and action2 for the same target (now absent).
        let report_event2: Vec<u8> = {
            let u = uuid::Uuid::new_v4();
            u.as_bytes()
                .iter()
                .chain(u.as_bytes().iter())
                .copied()
                .collect()
        };
        let report_id2: uuid::Uuid = sqlx::query_scalar(
            r#"INSERT INTO moderation_reports
               (community_id, report_event_id, reporter_pubkey, target_kind, target_pubkey, channel_id, report_type)
               VALUES ($1, $2, $3, 'pubkey', $4, $5, 'harassment') RETURNING id"#,
        )
        .bind(community_id).bind(&report_event2).bind(vec![0u8; 32])
        .bind(&target).bind(channel_id)
        .fetch_one(&pool).await.expect("insert report2");

        let action_id2 = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id2,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "kick",
            None,
            None,
            "resolve:kick",
            "relay_operator",
            Some(&target),
            None,
            Some(channel_id),
        )
        .await
        .expect("claim2")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id2)
            .await
            .expect("begin_enforcing2");

        // Acquire lease for action_id2.
        let lease_token2 = match buzz_db::relay_admin_actions::acquire_action_lease(
            &pool,
            action_id2,
            lease_until,
        )
        .await
        .expect("acquire lease2")
        {
            buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired for action2, got {other:?}"),
        };

        // Second kick: target already gone → AlreadyGone; step_marker NOT committed.
        let r2 = buzz_db::relay_admin_actions::execute_kick_with_marker(
            &pool,
            action_id2,
            lease_token2,
            cid,
            channel_id,
            &target,
            &actor,
        )
        .await
        .expect("kick2");
        assert!(
            matches!(
                r2,
                buzz_db::relay_admin_actions::KickWithMarkerResult::AlreadyGone
            ),
            "second kick must return AlreadyGone (pre-existing absence)"
        );

        // step_marker must NOT be set on action2 — the marker-fence prevented commit.
        let rec2 = buzz_db::relay_admin_actions::get_action(&pool, action_id2)
            .await
            .expect("get_action2")
            .expect("exists");
        assert!(
            rec2.step_marker.is_none(),
            "AlreadyGone must not commit step_marker; got: {:?}",
            rec2.step_marker
        );

        // Expire action2's lease so the production driver can re-acquire it.
        // (In production this happens when the original worker's lease times out.)
        sqlx::query(
            "UPDATE relay_admin_actions SET action_lease_expires_at = $2, action_lease_token = NULL WHERE id = $1",
        )
        .bind(action_id2)
        .bind(chrono::Utc::now() - chrono::Duration::seconds(300))
        .execute(&pool)
        .await
        .expect("expire action2 lease");

        // Drive enforcement via production driver: AlreadyGone → enforcement failure.
        let result2 = crate::handlers::report_resolution::drive_enforcement_pub(
            &state,
            &tenant,
            cid,
            report_id2,
            "kick",
            None,
            None,
            &actor,
            Some(&target),
            None,
            Some(channel_id),
            &rec2,
            None,
        )
        .await;
        assert!(
            matches!(
                result2,
                Err(crate::handlers::report_resolution::ResolutionError::EnforcementFailed { .. })
            ),
            "AlreadyGone kick via driver must return EnforcementFailed: {result2:?}"
        );
    }

    // ── 3. delivery failure: report resolved but delivery retryable ───────────

    #[tokio::test]
    #[ignore = "requires Postgres — delivery failure leaves report resolved with retryable delivery"]
    async fn delivery_failure_leaves_report_resolved_with_retryable_delivery_state() {
        // Fully finalize a ban, then simulate delivery failures via the outbox
        // worker path (`deliver_one`). The outbox row must use retryable backoff
        // state; terminal `failed` only after exhausting the attempt limit.
        // The report must remain `resolved` throughout.
        let pool = e2e_pool().await;
        let (community_id, _host) = e2e_community(&pool, "delivery-failure").await;
        let target = vec![8u8; 32];
        let actor = vec![9u8; 32];
        let report_id = e2e_report_pubkey(&pool, community_id, &target).await;
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        // Full enforcement cycle: claim → enforcing → ban+marker → finalize.
        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(60);
        let lease_token =
            match buzz_db::relay_admin_actions::acquire_action_lease(&pool, action_id, lease_until)
                .await
                .expect("acquire lease")
            {
                buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
                other => panic!("expected Acquired, got {other:?}"),
            };

        let _ = buzz_db::relay_admin_actions::execute_ban_with_marker(
            &pool,
            action_id,
            lease_token,
            cid,
            &target,
            &actor,
            None,
        )
        .await
        .expect("execute_ban_with_marker");

        let finalized = buzz_db::relay_admin_actions::finalize_success(
            &pool,
            action_id,
            cid,
            report_id,
            "resolved",
            &actor,
            "ban",
            Some(&target),
            None,
            None,
            None,
        )
        .await
        .expect("finalize_success");
        assert!(finalized, "finalize must succeed");

        // Report is resolved.
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM moderation_reports WHERE id = $1")
                .bind(report_id)
                .fetch_optional(&pool)
                .await
                .expect("status");
        assert_eq!(status.as_deref(), Some("resolved"));

        // Insert a tombstone row with a bogus community UUID so delivery predictably
        // fails (community not found → resolve_tenant fails). This lets us exercise
        // claim-token-fenced retry logic through the real outbox worker path.
        let bogus_community = uuid::Uuid::new_v4(); // not in communities table
        let bogus_channel = uuid::Uuid::new_v4();
        let tombstone_id: uuid::Uuid = sqlx::query_scalar(
            r#"INSERT INTO relay_admin_outbox (action_id, task_type, payload, dedup_key)
               VALUES ($1, 'tombstone', $2, $3) RETURNING id"#,
        )
        .bind(action_id)
        .bind(serde_json::json!({
            "community_id": bogus_community.to_string(),
            "channel_id": bogus_channel.to_string(),
            "target_event_id": hex::encode(vec![0u8; 32]),
            "action_id": action_id.to_string(),
        }))
        .bind(format!("tombstone-failure-test:{action_id}"))
        .fetch_one(&pool)
        .await
        .expect("insert tombstone outbox row");

        let state = state_from_pool(pool.clone()).await;

        // Run deliver_one through OUTBOX_MAX_ATTEMPTS iterations.
        // Each iteration: claim the pending row, call deliver_one (which fails →
        // calls fail_outbox_row with the claim token internally), verify state.
        for attempt in 1..=buzz_db::relay_admin_actions::OUTBOX_MAX_ATTEMPTS {
            // Reset retry_after and lease so the row is immediately re-claimable.
            sqlx::query(
                "UPDATE relay_admin_outbox \
                 SET retry_after = NULL, held_by = NULL, lease_expires_at = NULL, \
                     outbox_claim_token = NULL WHERE id = $1",
            )
            .bind(tombstone_id)
            .execute(&pool)
            .await
            .expect("reset retry_after");

            let lease_until = chrono::Utc::now() + chrono::Duration::seconds(30);
            let mut batch = state
                .db
                .claim_pending_admin_outbox_batch("e2e-delivery-fail-worker", lease_until, 100)
                .await
                .expect("claim_pending_admin_outbox_batch");

            let row_idx = batch
                .iter()
                .position(|r| r.id == tombstone_id)
                .unwrap_or_else(|| panic!("tombstone row must be in batch on attempt {attempt}"));
            let row = batch.remove(row_idx);

            // deliver_one calls the delivery primitive, which fails (bogus community),
            // then calls fail_outbox_row(row.id, row.claim_token, error) — exercising
            // the full claim-token-fenced failure path.
            crate::handlers::admin_outbox_worker::deliver_one(&state, &row).await;

            let (row_state, row_attempt): (String, i32) =
                sqlx::query_as("SELECT state, attempt_count FROM relay_admin_outbox WHERE id = $1")
                    .bind(tombstone_id)
                    .fetch_one(&pool)
                    .await
                    .expect("fetch row");

            assert_eq!(row_attempt, attempt, "attempt_count must be {attempt}");
            if attempt < buzz_db::relay_admin_actions::OUTBOX_MAX_ATTEMPTS {
                assert_eq!(
                    row_state, "pending",
                    "after {attempt} failures, row must remain pending (retryable)"
                );
            } else {
                assert_eq!(
                    row_state,
                    "failed",
                    "after {} failures, row must be terminal failed",
                    buzz_db::relay_admin_actions::OUTBOX_MAX_ATTEMPTS
                );
            }
        }

        // Report stays resolved even though delivery is exhausted.
        let final_status: Option<String> =
            sqlx::query_scalar("SELECT status FROM moderation_reports WHERE id = $1")
                .bind(report_id)
                .fetch_optional(&pool)
                .await
                .expect("final status");
        assert_eq!(
            final_status.as_deref(),
            Some("resolved"),
            "report must remain resolved even when delivery is exhausted"
        );
    }

    // ── 4. lease-expiry action takeover by the worker ─────────────────────────

    #[tokio::test]
    #[ignore = "requires Postgres — lease-expiry action takeover"]
    async fn lease_expiry_action_takeover_by_worker() {
        // Two-phase test for the C1-liveness fix:
        //
        // Phase 1: `drive_enforcement_pub` is called with an expired lease token
        //   (simulating a worker whose lease expired mid-mutation). The new
        //   `LeaseLost` path must terminate — not loop — and return an Err.
        //   The action stays in `enforcing` with no step_marker so the recovery
        //   worker can pick it up.
        //
        // Phase 2: `recover_one` (the actual production worker entry point) is
        //   called with a freshly-claimed live token. It must converge the action
        //   to `succeeded`.
        let pool = e2e_pool().await;
        let (community_id, _host) = e2e_community(&pool, "lease-expiry").await;
        let target = vec![10u8; 32];
        let actor = vec![11u8; 32];
        let report_id = e2e_report_pubkey(&pool, community_id, &target).await;
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        // Claim: creates pending action.
        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        // Advance to enforcing so drive_enforcement_pub sees the right state.
        buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        // Assign an expired lease token: simulates a worker that acquired a lease
        // but it has since expired (e.g. pod stalled for > 60 s).
        let expired_token = uuid::Uuid::new_v4();
        let expired_at = chrono::Utc::now() - chrono::Duration::seconds(300);
        sqlx::query(
            "UPDATE relay_admin_actions SET action_lease_token = $2, action_lease_expires_at = $3 WHERE id = $1",
        )
        .bind(action_id)
        .bind(expired_token)
        .bind(expired_at)
        .execute(&pool)
        .await
        .expect("install expired lease");

        // Phase 1: call drive_enforcement_pub with the expired token.
        // With the C1-liveness fix, this must return an error (LeaseLost) rather
        // than spinning in a tight loop with the expired token.
        let state = state_from_pool(pool.clone()).await;
        let rec = buzz_db::relay_admin_actions::get_action(&pool, action_id)
            .await
            .expect("get")
            .expect("exists");
        let host = state
            .db
            .lookup_community_host(cid)
            .await
            .expect("lookup")
            .expect("host");
        let tenant = buzz_core::tenant::TenantContext::resolved(cid, host);
        let result = crate::handlers::report_resolution::drive_enforcement_pub(
            &state,
            &tenant,
            cid,
            report_id,
            &rec.action.clone(),
            rec.reason.as_deref(),
            rec.timeout_until,
            &rec.actor_pubkey.clone(),
            Some(target.as_slice()),
            None,
            None,
            &rec,
            Some(expired_token), // expired caller-supplied token
        )
        .await;
        assert!(
            result.is_err(),
            "drive_enforcement_pub with an expired token must return Err (LeaseLost), not loop"
        );
        let err_msg = format!("{:?}", result.unwrap_err());
        assert!(
            err_msg.contains("lease lost")
                || err_msg.contains("lease_lost")
                || err_msg.contains("LeaseLost"),
            "error must name the lease-lost cause; got: {err_msg}"
        );

        // Action must still be in `enforcing` with step_marker NULL — nothing was committed.
        let after_phase1 = buzz_db::relay_admin_actions::get_action(&pool, action_id)
            .await
            .expect("get after phase1")
            .expect("exists");
        assert_eq!(
            after_phase1.state, "enforcing",
            "action must still be enforcing after LeaseLost"
        );
        assert!(
            after_phase1.step_marker.is_none(),
            "step_marker must be NULL after LeaseLost"
        );

        // Phase 2: recovery worker re-claims and converges the action.
        // Expire the DB-side lease so claim_stranded_action_batch can pick it up.
        sqlx::query("UPDATE relay_admin_actions SET action_lease_expires_at = $2 WHERE id = $1")
            .bind(action_id)
            .bind(chrono::Utc::now() - chrono::Duration::seconds(1))
            .execute(&pool)
            .await
            .expect("expire lease for batch");

        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(120);
        let batch = buzz_db::relay_admin_actions::claim_stranded_action_batch(
            &pool,
            "e2e-worker",
            lease_until,
            1000,
        )
        .await
        .expect("claim_stranded_action_batch");
        let claim = batch
            .into_iter()
            .find(|c| c.record.id == action_id)
            .expect("stranded action must appear in batch after lease expiry");

        crate::handlers::admin_action_worker::recover_one(&state, claim).await;

        // Action must be succeeded and report resolved.
        let final_rec = buzz_db::relay_admin_actions::get_action(&pool, action_id)
            .await
            .expect("final get_action")
            .expect("exists");
        assert_eq!(final_rec.state, "succeeded");

        let report_status: Option<String> =
            sqlx::query_scalar("SELECT status FROM moderation_reports WHERE id = $1")
                .bind(report_id)
                .fetch_optional(&pool)
                .await
                .expect("report status");
        assert_eq!(report_status.as_deref(), Some("resolved"));

        // Second claim attempt must find nothing (action is now succeeded).
        let batch2 = buzz_db::relay_admin_actions::claim_stranded_action_batch(
            &pool,
            "e2e-worker-2",
            lease_until,
            10,
        )
        .await
        .expect("second claim_stranded");
        assert!(
            !batch2.iter().any(|c| c.record.id == action_id),
            "succeeded action must not appear in stranded batch"
        );
    }

    // ── 5. success-gated artifacts: nothing published before enforcement ───────

    #[tokio::test]
    #[ignore = "requires Postgres — success-gated delivery: no artifacts before enforcement"]
    async fn success_gated_artifacts_nothing_published_before_enforcement_succeeds() {
        // Verify the key invariant: no outbox rows exist until finalize_success
        // commits. Steps: claim → (check no outbox) → advance+marker → (check no
        // outbox) → finalize → (check outbox rows exist).
        let pool = e2e_pool().await;
        let (community_id, _host) = e2e_community(&pool, "success-gated").await;
        let target = vec![12u8; 32];
        let actor = vec![13u8; 32];
        let report_id = e2e_report_pubkey(&pool, community_id, &target).await;
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        // After claim: no outbox rows.
        let after_claim: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM relay_admin_outbox WHERE action_id = $1")
                .bind(action_id)
                .fetch_one(&pool)
                .await
                .expect("count after claim");
        assert_eq!(after_claim, 0, "no outbox rows after claim (success-gated)");

        // After begin_enforcing + execute_ban_with_marker: still no outbox rows.
        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(60);
        let lease_token =
            match buzz_db::relay_admin_actions::acquire_action_lease(&pool, action_id, lease_until)
                .await
                .expect("acquire lease")
            {
                buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
                other => panic!("expected Acquired, got {other:?}"),
            };

        let _ = buzz_db::relay_admin_actions::execute_ban_with_marker(
            &pool,
            action_id,
            lease_token,
            cid,
            &target,
            &actor,
            None,
        )
        .await
        .expect("execute_ban_with_marker");

        let after_mutation: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM relay_admin_outbox WHERE action_id = $1")
                .bind(action_id)
                .fetch_one(&pool)
                .await
                .expect("count after mutation");
        assert_eq!(
            after_mutation, 0,
            "no outbox rows after mutation (before finalize)"
        );

        // After finalize_success: outbox rows must exist.
        let finalized = buzz_db::relay_admin_actions::finalize_success(
            &pool,
            action_id,
            cid,
            report_id,
            "resolved",
            &actor,
            "ban",
            Some(&target),
            None,
            None,
            None,
        )
        .await
        .expect("finalize_success");
        assert!(finalized);

        let after_finalize: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM relay_admin_outbox WHERE action_id = $1")
                .bind(action_id)
                .fetch_one(&pool)
                .await
                .expect("count after finalize");
        assert!(
            after_finalize > 0,
            "outbox rows must exist only after finalization (success-gated)"
        );

        // Full e2e via resolve_report_with_enforcement: same invariant through the
        // production driver entry point.
        let (community_id2, host2) = e2e_community(&pool, "success-gated-e2e").await;
        let target2 = vec![14u8; 32];
        let report_id2 = e2e_report_pubkey(&pool, community_id2, &target2).await;
        let report2 = e2e_admin_report(report_id2, community_id2, &target2);
        let state = state_from_pool(pool.clone()).await;
        let tenant2 = e2e_tenant(community_id2, &host2);

        let result = crate::handlers::report_resolution::resolve_report_with_enforcement(
            &state,
            &tenant2,
            &report2,
            "ban",
            None,
            None,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "relay_operator",
        )
        .await;
        assert!(
            result.is_ok(),
            "full enforcement via production driver must succeed: {result:?}"
        );

        let action_id2 = result.unwrap().action_id;
        let outbox_e2e: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM relay_admin_outbox WHERE action_id = $1")
                .bind(action_id2)
                .fetch_one(&pool)
                .await
                .expect("e2e outbox count");
        assert!(
            outbox_e2e > 0,
            "production driver must create outbox rows on success"
        );
    }

    // ── 6. 9044 vs processing through the actual 9044 adapter ─────────────────

    #[tokio::test]
    #[ignore = "requires Postgres — 9044 adapter against processing report fails cleanly"]
    async fn community_9044_through_actual_adapter_against_processing_report() {
        // Drive through `handle_moderation_command` — the production dispatch boundary
        // that performs ban checks, freshness checks, kind routing, and actor derivation —
        // against a report already in 'processing'. The CAS must fail cleanly —
        // no orphan audit row.
        use nostr::{EventBuilder, Kind, Tag};

        let pool = e2e_pool().await;
        let (community_id, host) = e2e_community(&pool, "9044-adapter").await;
        let target = vec![15u8; 32];
        let cid = buzz_core::CommunityId::from_uuid(community_id);
        let state = state_from_pool(pool.clone()).await;
        let tenant = e2e_tenant(community_id, &host);

        // Generate actor keys and register as community owner (authorize_moderation_action
        // checks relay_members before dispatching).
        let actor_keys = nostr::Keys::generate();
        let actor_pubkey = actor_keys.public_key().to_bytes().to_vec();
        let actor_hex = hex::encode(&actor_pubkey);
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'owner')",
        )
        .bind(community_id)
        .bind(&actor_hex)
        .execute(&pool)
        .await
        .expect("insert owner");

        // Create a report with a known report_event_id (needed for the `report` tag).
        let uid = uuid::Uuid::new_v4();
        let report_event_id_bytes: Vec<u8> = uid
            .as_bytes()
            .iter()
            .chain(uid.as_bytes().iter())
            .copied()
            .collect();
        let report_event_id_hex = hex::encode(&report_event_id_bytes);

        let report_id: uuid::Uuid = sqlx::query_scalar(
            r#"INSERT INTO moderation_reports
               (community_id, report_event_id, reporter_pubkey, target_kind,
                target_pubkey, report_type)
               VALUES ($1, $2, $3, 'pubkey', $4, 'harassment') RETURNING id"#,
        )
        .bind(community_id)
        .bind(report_event_id_bytes.as_slice())
        .bind(vec![0u8; 32])
        .bind(&target)
        .fetch_one(&pool)
        .await
        .expect("insert report");

        // HTTP enforcement: move report to 'processing'.
        let _ = buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor_pubkey,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("enforcement claim");

        // Community 9044 path — drive through handle_moderation_command, which
        // performs ban checks, freshness validation, kind dispatch, actor derivation,
        // and ultimately resolves via resolve_report_decision_only →
        // resolve_report_decision_atomic. Construct a kind-9044 event signed with
        // current time so the freshness check passes (±120 s window).
        let event = EventBuilder::new(Kind::Custom(9044), "")
            .tags([
                Tag::parse(["report", &report_event_id_hex]).unwrap(),
                Tag::parse(["status", "dismissed"]).unwrap(),
                Tag::parse(["action", "dismiss"]).unwrap(),
            ])
            .sign_with_keys(&actor_keys)
            .expect("sign 9044 event");

        let result = crate::handlers::moderation_commands::handle_moderation_command(
            &tenant, &state, &event,
        )
        .await;

        // The CAS must fail because the report is in 'processing', not 'open'.
        assert!(
            result.is_err(),
            "9044 adapter against processing report must return error: {result:?}"
        );

        // Exactly one audit row (from the enforcement claim); the 9044 attempt
        // must not have inserted an orphan.
        let audit_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions WHERE community_id = $1")
                .bind(community_id)
                .fetch_one(&pool)
                .await
                .expect("audit count");
        assert_eq!(
            audit_count, 1,
            "no orphan audit row from failed 9044 adapter call"
        );
    }

    // ── Race C1: stale action lease token rejected at mutation boundary ───────

    #[tokio::test]
    #[ignore = "requires Postgres — stale action lease token cannot commit mutation"]
    async fn stale_action_lease_token_rejected_at_mutation() {
        // Two concurrent workers claim the same action batch.  Simulate: worker A
        // holds token A, its lease expires, worker B re-claims (token B).  Worker A
        // must NOT be able to commit the domain mutation — `execute_ban_with_marker`
        // returns `false` when the token no longer matches the live row.
        let pool = e2e_pool().await;
        let (community_id, _host) = e2e_community(&pool, "race-c1").await;
        let target = vec![20u8; 32];
        let actor = vec![21u8; 32];
        let report_id = e2e_report_pubkey(&pool, community_id, &target).await;
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        // Claim and advance to enforcing.
        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        // Worker A acquires lease.
        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(60);
        let stale_token =
            match buzz_db::relay_admin_actions::acquire_action_lease(&pool, action_id, lease_until)
                .await
                .expect("acquire lease A")
            {
                buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
                other => panic!("expected Acquired, got {other:?}"),
            };

        // Simulate lease expiry by back-dating the expiry in the DB.
        let expired = chrono::Utc::now() - chrono::Duration::seconds(300);
        sqlx::query("UPDATE relay_admin_actions SET action_lease_expires_at = $2 WHERE id = $1")
            .bind(action_id)
            .bind(expired)
            .execute(&pool)
            .await
            .expect("expire lease");

        // Worker B re-claims (new token, fresh expiry).
        let valid_token = match buzz_db::relay_admin_actions::acquire_action_lease(
            &pool,
            action_id,
            chrono::Utc::now() + chrono::Duration::seconds(60),
        )
        .await
        .expect("acquire lease B")
        {
            buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired for B, got {other:?}"),
        };

        // Worker A attempts mutation with stale token — must be rejected.
        let stale_result = buzz_db::relay_admin_actions::execute_ban_with_marker(
            &pool,
            action_id,
            stale_token,
            cid,
            &target,
            &actor,
            None,
        )
        .await
        .expect("execute_ban stale");
        assert!(
            !stale_result,
            "stale token must not commit mutation (execute_ban_with_marker returned true)"
        );

        // Domain row must be untouched (no ban entry written by stale worker).
        let ban_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM community_bans WHERE community_id = $1 AND pubkey = $2",
        )
        .bind(community_id)
        .bind(&target)
        .fetch_one(&pool)
        .await
        .expect("ban count");
        assert_eq!(
            ban_count, 0,
            "stale worker must not have written community_bans row"
        );

        // step_marker must still be NULL (mutation was rolled back).
        let rec = buzz_db::relay_admin_actions::get_action(&pool, action_id)
            .await
            .expect("get_action")
            .expect("action exists");
        assert!(
            rec.step_marker.is_none(),
            "step_marker must be NULL after stale token rejection; got {:?}",
            rec.step_marker
        );

        // Worker B commits successfully with its valid token.
        let valid_result = buzz_db::relay_admin_actions::execute_ban_with_marker(
            &pool,
            action_id,
            valid_token,
            cid,
            &target,
            &actor,
            None,
        )
        .await
        .expect("execute_ban valid");
        assert!(valid_result, "valid token must commit mutation");

        let rec2 = buzz_db::relay_admin_actions::get_action(&pool, action_id)
            .await
            .expect("get_action after valid")
            .expect("action exists");
        assert_eq!(
            rec2.step_marker.as_deref(),
            Some("mutation_committed"),
            "step_marker must be set after valid commit"
        );
    }

    // ── Race C2: stale outbox claim token cannot overwrite newer worker's result

    #[tokio::test]
    #[ignore = "requires Postgres — stale outbox claim token rejected on completion"]
    async fn stale_outbox_claim_token_rejected_on_completion() {
        // Worker A claims an outbox row (token A), its lease expires, worker B
        // re-claims (token B) and marks it delivered.  Worker A then tries to
        // record a failure with its stale token — must be rejected (zero rows
        // updated), so the delivered row is not rewritten to pending/failed.
        let pool = e2e_pool().await;
        let (community_id, _host) = e2e_community(&pool, "race-c2").await;
        let target = vec![22u8; 32];
        let actor = vec![23u8; 32];
        let report_id = e2e_report_pubkey(&pool, community_id, &target).await;
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        // Full finalization to produce an outbox row.
        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");

        let lease_token = match buzz_db::relay_admin_actions::acquire_action_lease(
            &pool,
            action_id,
            chrono::Utc::now() + chrono::Duration::seconds(60),
        )
        .await
        .expect("acquire lease")
        {
            buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired, got {other:?}"),
        };

        let _ = buzz_db::relay_admin_actions::execute_ban_with_marker(
            &pool,
            action_id,
            lease_token,
            cid,
            &target,
            &actor,
            None,
        )
        .await
        .expect("execute_ban");

        let finalized = buzz_db::relay_admin_actions::finalize_success(
            &pool,
            action_id,
            cid,
            report_id,
            "resolved",
            &actor,
            "ban",
            Some(&target),
            None,
            None,
            None,
        )
        .await
        .expect("finalize");
        assert!(finalized, "finalize must succeed");

        // Fetch the outbox row.
        let outbox_rows = buzz_db::relay_admin_actions::list_pending_outbox(&pool, action_id)
            .await
            .expect("list_pending_outbox");
        assert!(
            !outbox_rows.is_empty(),
            "must have outbox rows after finalization"
        );
        let outbox_id = outbox_rows[0].id;

        // Worker A claims the row (token A).
        let state = state_from_pool(pool.clone()).await;
        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(30);
        let batch_a = state
            .db
            .claim_pending_admin_outbox_batch("race-c2-worker-a", lease_until, 100)
            .await
            .expect("claim batch A");
        let row_a = batch_a
            .iter()
            .find(|r| r.id == outbox_id)
            .expect("outbox row must be in batch A");
        let stale_claim_token = row_a.claim_token;

        // Expire worker A's lease.
        sqlx::query("UPDATE relay_admin_outbox SET lease_expires_at = $2 WHERE id = $1")
            .bind(outbox_id)
            .bind(chrono::Utc::now() - chrono::Duration::seconds(60))
            .execute(&pool)
            .await
            .expect("expire outbox lease");

        // Worker B re-claims (token B) and marks delivered.
        let batch_b = state
            .db
            .claim_pending_admin_outbox_batch(
                "race-c2-worker-b",
                chrono::Utc::now() + chrono::Duration::seconds(30),
                100,
            )
            .await
            .expect("claim batch B");
        let row_b = batch_b
            .iter()
            .find(|r| r.id == outbox_id)
            .expect("outbox row must be in batch B");
        let valid_claim_token = row_b.claim_token;
        assert_ne!(
            stale_claim_token, valid_claim_token,
            "claim tokens must differ"
        );

        let delivered = buzz_db::relay_admin_actions::mark_outbox_delivered(
            &pool,
            outbox_id,
            valid_claim_token,
        )
        .await
        .expect("mark_delivered B");
        assert!(delivered, "worker B must mark delivered");

        // Verify delivered.
        let state_after_b: String =
            sqlx::query_scalar("SELECT state FROM relay_admin_outbox WHERE id = $1")
                .bind(outbox_id)
                .fetch_one(&pool)
                .await
                .expect("state after B");
        assert_eq!(
            state_after_b, "delivered",
            "row must be delivered after worker B"
        );

        // Worker A tries to record a failure with stale token — must fail (0 rows updated).
        let stale_fail = buzz_db::relay_admin_actions::fail_outbox_row(
            &pool,
            outbox_id,
            stale_claim_token,
            "stale error",
        )
        .await
        .expect("fail_outbox_row stale");
        assert!(
            !stale_fail,
            "stale claim token must not update already-delivered row"
        );

        // Row must still be delivered, not rewritten.
        let state_after_stale: String =
            sqlx::query_scalar("SELECT state FROM relay_admin_outbox WHERE id = $1")
                .bind(outbox_id)
                .fetch_one(&pool)
                .await
                .expect("state after stale fail");
        assert_eq!(
            state_after_stale, "delivered",
            "stale worker fail must not rewrite delivered row to failed/pending"
        );

        // mark_outbox_delivered with stale token on a non-pending row also returns false.
        let stale_delivered = buzz_db::relay_admin_actions::mark_outbox_delivered(
            &pool,
            outbox_id,
            stale_claim_token,
        )
        .await
        .expect("mark_delivered stale");
        assert!(
            !stale_delivered,
            "stale mark_delivered on already-delivered row must return false"
        );
    }

    // ── Race C3: failed durable system-message insert is not marked delivered ─

    #[tokio::test]
    #[ignore = "requires Postgres — failed emit_system_message insert is not marked delivered"]
    async fn failed_system_message_insert_not_marked_delivered() {
        // `emit_system_message` propagates durable event insert failures (previously
        // it swallowed them). This test verifies that `deliver_one` correctly calls
        // `fail_outbox_row` (not `mark_outbox_delivered`) when the insert itself
        // fails — so nothing is durably persisted, and the row is NOT marked delivered.
        //
        // The failure is induced AFTER tenant resolution, inside `emit_system_message`'s
        // `insert_event` call, by:
        //  1. Building a dedicated test pool whose `after_connect` sets the
        //     `buzz.created_at_floor` GUC session-locally (not database-globally).
        //     Every connection from that pool inherits the floor; no other pool or
        //     test is affected, and there is no cleanup race on panic.
        //  2. Backdating the outbox row's `created_at` beyond that floor.
        //  `emit_system_message` derives the Nostr event's `created_at` from
        //  `row.created_at` (the idempotency timestamp). With the floor active, the
        //  deferrable trigger fires on INSERT and raises a check_violation, which
        //  `insert_event` propagates as `Err`. The `?` in `emit_system_message` then
        //  propagates it up through `deliver_tombstone → deliver_one → fail_outbox_row`.
        let pool = e2e_pool().await;
        let (community_id, _host) = e2e_community(&pool, "race-c3").await;
        let target = vec![24u8; 32];
        let actor = vec![25u8; 32];
        let report_id = e2e_report_pubkey(&pool, community_id, &target).await;
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        // Build a real community + channel so resolve_tenant succeeds and
        // deliver_tombstone has a channel_id to pass to emit_system_message.
        let channel_id: uuid::Uuid = sqlx::query_scalar(
            r#"INSERT INTO channels (community_id, name, channel_type, created_by)
               VALUES ($1, 'c3-test', 'stream', $2) RETURNING id"#,
        )
        .bind(community_id)
        .bind(actor.as_slice())
        .fetch_one(&pool)
        .await
        .expect("create test channel");

        // Finalize an action so we have a real action_id to attach the outbox row to.
        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };

        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        let lease_token = match buzz_db::relay_admin_actions::acquire_action_lease(
            &pool,
            action_id,
            chrono::Utc::now() + chrono::Duration::seconds(60),
        )
        .await
        .expect("acquire lease")
        {
            buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
            other => panic!("expected Acquired, got {other:?}"),
        };
        let _ = buzz_db::relay_admin_actions::execute_ban_with_marker(
            &pool,
            action_id,
            lease_token,
            cid,
            &target,
            &actor,
            None,
        )
        .await
        .expect("execute_ban");
        let _ = buzz_db::relay_admin_actions::finalize_success(
            &pool,
            action_id,
            cid,
            report_id,
            "resolved",
            &actor,
            "ban",
            Some(&target),
            None,
            None,
            None,
        )
        .await
        .expect("finalize");

        // Insert a tombstone outbox row with a real channel_id so resolve_tenant
        // and all payload parsing succeed; deliver_tombstone reaches emit_system_message.
        let fail_outbox_id: uuid::Uuid = sqlx::query_scalar(
            r#"INSERT INTO relay_admin_outbox (action_id, task_type, payload, dedup_key)
               VALUES ($1, 'tombstone', $2, $3) RETURNING id"#,
        )
        .bind(action_id)
        .bind(serde_json::json!({
            "community_id": community_id.to_string(),
            "channel_id": channel_id.to_string(),
            "target_event_id": hex::encode(vec![0u8; 32]),
            "action_id": action_id.to_string(),
        }))
        .bind(format!("c3-test:{action_id}"))
        .fetch_one(&pool)
        .await
        .expect("insert fail-outbox row");

        // Backdate the outbox row's created_at so emit_system_message uses an old
        // idempotency_ts. The events_created_at_floor trigger will reject the INSERT
        // once we arm the GUC below.
        sqlx::query(
            "UPDATE relay_admin_outbox SET created_at = now() - interval '10 seconds' WHERE id = $1",
        )
        .bind(fail_outbox_id)
        .execute(&pool)
        .await
        .expect("backdate outbox created_at");

        // Build a dedicated pool whose after_connect sets buzz.created_at_floor = 5
        // session-locally on each connection (set_config 3rd arg false = session scope).
        // A floor of 5 s means any event with created_at > 5 s ago is rejected.
        // Our outbox row's created_at is ~10 s ago → trigger fires on insert_event.
        // This pool is fully isolated: no other pool or test is affected, and there
        // is no cleanup dependence (dropping the pool closes all its connections).
        let db_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
        let floor_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("SELECT set_config('buzz.created_at_floor', '5', false)")
                        .execute(conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(&db_url)
            .await
            .expect("connect floor pool");

        // Build an AppState around the floor pool so deliver_one's insert_event call
        // runs on a connection where the deferrable trigger is active.
        let fresh_state = state_from_pool(floor_pool.clone()).await;

        // Claim the row via the floor pool so deliver_one has a real claim token.
        let lease_until = chrono::Utc::now() + chrono::Duration::seconds(30);
        let mut batch = fresh_state
            .db
            .claim_pending_admin_outbox_batch("race-c3-worker", lease_until, 100)
            .await
            .expect("claim outbox batch");
        let row_idx = batch
            .iter()
            .position(|r| r.id == fail_outbox_id)
            .expect("fail_outbox_id must be in batch");
        let row = batch.remove(row_idx);

        // deliver_one fails inside emit_system_message at insert_event (deferrable
        // floor-guard trigger → check_violation) and must call fail_outbox_row —
        // NOT mark_outbox_delivered.
        crate::handlers::admin_outbox_worker::deliver_one(&fresh_state, &row).await;

        // Drop the floor pool — all its connections close, GUC vanishes with them.
        // No ALTER DATABASE, no global state, no reset required.
        drop(floor_pool);

        // No tombstone event was persisted — the failure was inside insert_event.
        let post_event_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE community_id = $1 AND channel_id = $2 AND kind = 40099",
        )
        .bind(community_id)
        .bind(channel_id)
        .fetch_one(&pool)
        .await
        .expect("post-delivery event count");
        assert_eq!(
            post_event_count, 0,
            "no tombstone event must be persisted when insert_event failed"
        );

        // Row must be `pending` (retryable), not `delivered` (nothing was persisted).
        let (row_state, attempt): (String, i32) =
            sqlx::query_as("SELECT state, attempt_count FROM relay_admin_outbox WHERE id = $1")
                .bind(fail_outbox_id)
                .fetch_one(&pool)
                .await
                .expect("fetch row state");

        assert_ne!(
            row_state, "delivered",
            "row must not be marked delivered when durable insert failed"
        );
        assert_eq!(
            row_state, "pending",
            "failed delivery must leave row pending (retryable), not delivered"
        );
        assert_eq!(attempt, 1, "attempt_count must be 1 after one failure");
    }

    // ── 10. reporter notice overlap: concurrent deliveries persist exactly one ─

    #[tokio::test]
    #[ignore = "requires Postgres — reporter notice idempotency under concurrent delivery"]
    async fn reporter_notice_duplicate_delivery_persists_exactly_one() {
        // Two workers race to deliver the same reporter_notice outbox row.
        // Worker A holds a stale (expired) token; worker B holds the current
        // (reclaimed) token. Both derive the same Nostr event from the row's
        // immutable `created_at`, so both insert_event calls produce the same
        // event ID → ON CONFLICT DO NOTHING ensures exactly one durable notice.
        // Worker A's mark_outbox_delivered fails the claim-token fence (C2);
        // worker B's succeeds. The row ends delivered and owned only by B's token.
        let pool = e2e_pool().await;
        let (community_id, _host) = e2e_community(&pool, "notice-overlap").await;
        let target = vec![31u8; 32];
        let actor = vec![32u8; 32];
        let cid = buzz_core::CommunityId::from_uuid(community_id);

        // Insert a report using the standard helper (handles correct column names
        // and types for `moderation_reports`).
        let report_id = e2e_report_pubkey(&pool, community_id, &target).await;

        // Finalize an action so we have an action_id.
        let action_id = match buzz_db::relay_admin_actions::claim_report(
            &pool,
            cid,
            report_id,
            uuid::Uuid::new_v4(),
            &actor,
            "operator",
            "ban",
            None,
            None,
            "resolve:ban",
            "relay_operator",
            Some(&target),
            None,
            None,
        )
        .await
        .expect("claim")
        {
            buzz_db::relay_admin_actions::ClaimResult::Claimed(a) => a.id,
            other => panic!("expected Claimed, got {other:?}"),
        };
        let _ = buzz_db::relay_admin_actions::begin_enforcing(&pool, action_id)
            .await
            .expect("begin_enforcing");
        let lt = match buzz_db::relay_admin_actions::acquire_action_lease(
            &pool,
            action_id,
            chrono::Utc::now() + chrono::Duration::seconds(60),
        )
        .await
        .expect("lease")
        {
            buzz_db::relay_admin_actions::LeaseResult::Acquired(t) => t,
            other => panic!("{other:?}"),
        };
        let _ = buzz_db::relay_admin_actions::execute_ban_with_marker(
            &pool, action_id, lt, cid, &target, &actor, None,
        )
        .await
        .expect("execute_ban");
        let _ = buzz_db::relay_admin_actions::finalize_success(
            &pool,
            action_id,
            cid,
            report_id,
            "resolved",
            &actor,
            "ban",
            Some(&target),
            None,
            None,
            None,
        )
        .await
        .expect("finalize");

        // Find the reporter_notice outbox row created by finalize_success.
        let notice_outbox_id: uuid::Uuid = sqlx::query_scalar(
            "SELECT id FROM relay_admin_outbox WHERE action_id = $1 AND task_type = 'reporter_notice'",
        )
        .bind(action_id)
        .fetch_one(&pool)
        .await
        .expect("reporter_notice outbox row");

        // Pre-warm: deliver once through the full production path so the DM channel
        // is created (open_dm is check-then-insert; concurrent creation races on the
        // unique participant_hash index). After this delivery the DM channel exists,
        // so both concurrent workers will hit the idempotent fast path. Delete the
        // resulting events and reset the outbox row so the actual overlap test starts
        // from a clean state.
        let state = state_from_pool(pool.clone()).await;
        {
            let lease_until = chrono::Utc::now() + chrono::Duration::seconds(30);
            let warm_batch = state
                .db
                .claim_pending_admin_outbox_batch("notice-warmup", lease_until, 100)
                .await
                .expect("warmup claim batch");
            let warm_row = warm_batch
                .into_iter()
                .find(|r| r.id == notice_outbox_id)
                .expect("notice row in warmup batch");
            crate::handlers::admin_outbox_worker::deliver_one(&state, &warm_row).await;
        }
        // Delete the events produced by the warm-up (kind:9 notice + discovery/profile
        // events) so the concurrent test proves fresh insertion, not dedup against
        // warm-up artefacts.
        sqlx::query("DELETE FROM events WHERE community_id = $1")
            .bind(community_id)
            .execute(&pool)
            .await
            .expect("delete warmup events");
        // Reset outbox row to pending so it can be re-claimed.
        sqlx::query(
            "UPDATE relay_admin_outbox SET state = 'pending', outbox_claim_token = NULL, \
             held_by = NULL, lease_expires_at = NULL, attempt_count = 0 WHERE id = $1",
        )
        .bind(notice_outbox_id)
        .execute(&pool)
        .await
        .expect("reset outbox row for overlap test");

        // Worker A claims the outbox row and captures the stable created_at.
        let lease_until_a = chrono::Utc::now() + chrono::Duration::seconds(30);
        let record_a: buzz_db::relay_admin_actions::OutboxRecord = {
            let state_a = state_from_pool(pool.clone()).await;
            let batch = state_a
                .db
                .claim_pending_admin_outbox_batch("notice-worker-a", lease_until_a, 100)
                .await
                .expect("claim batch a");
            batch
                .into_iter()
                .find(|r| r.id == notice_outbox_id)
                .expect("notice row in batch a")
        };
        // Capture the immutable idempotency timestamp — both workers will derive
        // the same Nostr event ID from this.
        let idempotency_ts = record_a.created_at;

        // Simulate worker A's lease expiring and worker B reclaiming the row:
        // assign a fresh token_b. This does NOT change created_at (the immutable
        // idempotency anchor), so both workers still produce the same Nostr event.
        let token_b = uuid::Uuid::new_v4();
        sqlx::query(
            "UPDATE relay_admin_outbox \
             SET outbox_claim_token = $2, held_by = 'notice-worker-b', \
                 lease_expires_at = now() + interval '30 seconds' \
             WHERE id = $1",
        )
        .bind(notice_outbox_id)
        .bind(token_b)
        .execute(&pool)
        .await
        .expect("reassign token to worker b");

        // Build record_b directly from the same immutable row fields but with the
        // current (B) token. record_a keeps the stale (A) token — it is now a
        // "ghost" delivery from the expired worker.
        let record_b = buzz_db::relay_admin_actions::OutboxRecord {
            id: record_a.id,
            action_id: record_a.action_id,
            task_type: record_a.task_type.clone(),
            payload: record_a.payload.clone(),
            state: record_a.state.clone(),
            dedup_key: record_a.dedup_key.clone(),
            error_message: None,
            attempt_count: record_a.attempt_count,
            claim_token: token_b,
            created_at: idempotency_ts, // same as record_a — same Nostr event ID
        };

        // Run both deliveries concurrently. Both call insert_event with the same
        // event ID → ON CONFLICT DO NOTHING. Worker A's mark_outbox_delivered is
        // rejected by the C2 token fence (token_a ≠ token_b in DB). Worker B's
        // mark_outbox_delivered succeeds.
        let (_, _) = tokio::join!(
            crate::handlers::admin_outbox_worker::deliver_one(&state, &record_a),
            crate::handlers::admin_outbox_worker::deliver_one(&state, &record_b),
        );

        // Assert: exactly one notice event (kind:9) with the specific report_id
        // source tag is persisted. The moderation_source tag carries report_id
        // (from ModerationNotice::ReportResolved). Filter by kind and tag to
        // isolate the notice from profile/discovery events emitted by the same worker.
        let report_id_str = report_id.to_string();
        let relay_pubkey_bytes = state.relay_keypair.public_key().to_bytes();
        let total_notices: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*) FROM events
               WHERE community_id = $1
                 AND kind = 9
                 AND pubkey = $2
                 AND tags @> jsonb_build_array(jsonb_build_array('moderation_source', $3::text))"#,
        )
        .bind(community_id)
        .bind(relay_pubkey_bytes.as_slice())
        .bind(&report_id_str)
        .fetch_one(&pool)
        .await
        .expect("count notice events");
        assert_eq!(
            total_notices, 1,
            "exactly one notice event must be persisted after two concurrent deliveries (ON CONFLICT DO NOTHING dedup)"
        );

        // Assert: row is delivered and owned only by token_b (worker B).
        let (row_state, row_token): (String, uuid::Uuid) = sqlx::query_as(
            "SELECT state, outbox_claim_token FROM relay_admin_outbox WHERE id = $1",
        )
        .bind(notice_outbox_id)
        .fetch_one(&pool)
        .await
        .expect("fetch row state");
        assert_eq!(
            row_state, "delivered",
            "row must be delivered after worker B completes"
        );
        assert_eq!(
            row_token, token_b,
            "row claim token must belong to worker B (stale A token must not rewrite)"
        );
    }
}
