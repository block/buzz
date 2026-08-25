use crate::{
    agent::{
        AgentToolAuditRequest, ConsumeAgentDelegationRequest, IssueAgentDelegationRequest,
        VerifyAgentDelegationRequest,
    },
    auth::{bearer, JwtVerifier},
    config::Config,
    model::{ChallengeRequest, IssueEmbedRequest, RequestFacts, VerifyBindingRequest},
    security,
    store::{Rejection, Store},
};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post},
    Form, Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub verifier: JwtVerifier,
    pub config: Config,
}

#[derive(Debug)]
pub struct ApiError(Rejection);
impl From<Rejection> for ApiError {
    fn from(value: Rejection) -> Self {
        Self(value)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self.0 {
            Rejection::Unauthorized(c) => (StatusCode::UNAUTHORIZED, c),
            Rejection::Forbidden(c) => (StatusCode::FORBIDDEN, c),
            Rejection::Invalid(c) => (StatusCode::BAD_REQUEST, c),
            Rejection::Conflict(c) => (StatusCode::CONFLICT, c),
            Rejection::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Rejection::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Rejection::Database => (StatusCode::SERVICE_UNAVAILABLE, "service_unavailable"),
        };
        let mut response = (status, Json(serde_json::json!({"error":code}))).into_response();
        no_store(response.headers_mut());
        response
    }
}

pub fn router(state: AppState) -> Router {
    let origins = state
        .config
        .allowed_workbench_origins
        .iter()
        .chain(std::iter::once(&state.config.business_origin))
        .filter_map(|value| HeaderValue::from_str(value).ok())
        .collect::<Vec<_>>();
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::DELETE,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            axum::http::HeaderName::from_static("x-trace-id"),
            axum::http::HeaderName::from_static("x-csrf-token"),
        ]);
    Router::new()
        .route("/health/live", get(live))
        .route("/health/ready", get(ready))
        .route("/api/me", get(me))
        .route("/api/identity-bindings/challenges", post(challenge))
        .route("/api/identity-bindings/verify", post(verify_binding))
        .route("/api/me/identity-bindings", get(bindings))
        .route("/api/me/identity-bindings/{id}", delete(revoke_binding))
        .route("/api/embed-sessions", post(issue_embed))
        .route("/api/embed-sessions/{id}/revoke", post(revoke_embed))
        .route("/embed/bootstrap", get(bootstrap))
        .route("/api/business/session", get(business_session))
        .route("/api/session", get(business_session))
        .route("/api/logout/business", post(business_logout))
        .route("/api/logout", post(business_logout))
        .route("/api/logout/workbench", post(workbench_logout))
        .route("/api/logout/global", post(global_logout))
        .route("/api/oidc/backchannel-logout", post(backchannel_logout))
        .route("/internal/agent-delegations", post(issue_agent_delegation))
        .route(
            "/internal/agent-delegations/consume",
            post(consume_agent_delegation),
        )
        .route(
            "/internal/agent-delegations/verify",
            post(verify_agent_delegation),
        )
        .route(
            "/internal/agent-delegations/{id}/revoke",
            post(revoke_agent_delegation),
        )
        .route("/internal/agent-audit", post(audit_agent_tool))
        .with_state(Arc::new(state))
        .layer(cors)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
}

fn no_store(headers: &mut HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}
fn facts(headers: &HeaderMap) -> RequestFacts {
    let ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| v.parse::<std::net::IpAddr>().is_ok())
        .map(str::to_string);
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());
    RequestFacts {
        ip,
        user_agent_hash: security::hash_optional(ua),
        trace_id: security::trace_id(headers.get("x-trace-id").and_then(|v| v.to_str().ok())),
    }
}
fn origin_allowed(headers: &HeaderMap, state: &AppState, business: bool) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            if business {
                v == state.config.business_origin
            } else {
                state.config.allowed_workbench_origins.contains(v)
            }
        })
        .unwrap_or(false)
}
fn service_authorized(headers: &HeaderMap, state: &AppState) -> bool {
    use subtle::ConstantTimeEq as _;
    if !state.config.business_agent_read_enabled {
        return false;
    }
    let Some(expected) = state.config.business_read_service_credential.as_deref() else {
        return false;
    };
    let Some(presented) = headers
        .get("x-business-service-credential")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    security::hash(expected)
        .as_slice()
        .ct_eq(security::hash(presented).as_slice())
        .into()
}
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|v| v.trim().split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v.to_string())
}
fn set_cookie(value: &str, http_only: bool, max_age: Option<u64>) -> String {
    format!(
        "{value}; Path=/; Secure; SameSite=None; Partitioned{}{}",
        if http_only { "; HttpOnly" } else { "" },
        max_age
            .map(|v| format!("; Max-Age={v}"))
            .unwrap_or_default()
    )
}

async fn principal(
    state: &AppState,
    headers: &HeaderMap,
    facts: &RequestFacts,
) -> Result<crate::model::Principal, ApiError> {
    let token =
        bearer(headers).map_err(|_| ApiError(Rejection::Unauthorized("bearer_token_required")))?;
    let claims = match state.verifier.workbench(token).await {
        Ok(claims) => claims,
        Err(error) => {
            let reason = match error.as_str() {
                "jwt_signature_invalid" => "jwt_signature_invalid",
                "jwt_expired" => "jwt_expired",
                "jwt_issuer_mismatch" => "jwt_issuer_mismatch",
                "jwt_audience_mismatch" => "jwt_audience_mismatch",
                "jwt_not_yet_valid" => "jwt_not_yet_valid",
                "jwt_required_claim_missing" => "jwt_required_claim_missing",
                "jwt_algorithm_rejected" => "jwt_algorithm_rejected",
                "jwt_claims_invalid" => "jwt_claims_invalid",
                "jwt client mismatch" => "jwt_client_mismatch",
                _ => "jwt_verification_failed",
            };
            let mut audit =
                crate::model::Audit::event("OIDC_LOGIN_FAILED", "failure", facts.clone());
            audit.reason = Some(reason);
            state.store.audit(audit).await?;
            return Err(ApiError(Rejection::Unauthorized(reason)));
        }
    };
    state
        .store
        .principal(&claims, facts)
        .await
        .map_err(Into::into)
}
async fn live() -> StatusCode {
    StatusCode::NO_CONTENT
}
async fn ready(State(s): State<Arc<AppState>>) -> Result<StatusCode, ApiError> {
    s.store.ready().await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn me(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<crate::model::MeResponse>, ApiError> {
    let f = facts(&headers);
    let p = principal(&s, &headers, &f).await?;
    Ok(Json(s.store.me(&p).await?))
}
async fn bindings(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::model::Binding>>, ApiError> {
    let f = facts(&headers);
    let p = principal(&s, &headers, &f).await?;
    Ok(Json(s.store.bindings(&p).await?))
}
async fn challenge(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ChallengeRequest>,
) -> Result<Json<crate::model::ChallengeResponse>, ApiError> {
    if !origin_allowed(&headers, &s, false) {
        return Err(ApiError(Rejection::Forbidden("origin_rejected")));
    }
    let f = facts(&headers);
    let p = principal(&s, &headers, &f).await?;
    Ok(Json(s.store.challenge(&p, body, f).await?))
}
async fn verify_binding(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<VerifyBindingRequest>,
) -> Result<Json<crate::model::Binding>, ApiError> {
    if !origin_allowed(&headers, &s, false) {
        return Err(ApiError(Rejection::Forbidden("origin_rejected")));
    }
    let f = facts(&headers);
    let p = principal(&s, &headers, &f).await?;
    Ok(Json(
        s.store
            .verify_binding(&p, body.challenge_id, body.signed_event, f)
            .await?,
    ))
}
async fn revoke_binding(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !origin_allowed(&headers, &s, false) {
        return Err(ApiError(Rejection::Forbidden("origin_rejected")));
    }
    let f = facts(&headers);
    let p = principal(&s, &headers, &f).await?;
    s.store.revoke_binding(&p, id, f).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn issue_embed(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<IssueEmbedRequest>,
) -> Result<Json<crate::model::IssueEmbedResponse>, ApiError> {
    if !origin_allowed(&headers, &s, false) {
        return Err(ApiError(Rejection::Forbidden("origin_rejected")));
    }
    let f = facts(&headers);
    let p = principal(&s, &headers, &f).await?;
    Ok(Json(s.store.issue_embed(&p, body, f).await?))
}
async fn revoke_embed(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !origin_allowed(&headers, &s, false) {
        return Err(ApiError(Rejection::Forbidden("origin_rejected")));
    }
    let f = facts(&headers);
    let p = principal(&s, &headers, &f).await?;
    s.store.revoke_embed(&p, id, f).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct BootstrapQuery {
    code: String,
}
async fn bootstrap(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<BootstrapQuery>,
) -> Result<Response, ApiError> {
    let f = facts(&headers);
    let result = s.store.bootstrap(&q.code, f).await?;
    let mut response = Redirect::to(&result.target_path).into_response();
    no_store(response.headers_mut());
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie(
            &format!("{}={}", s.config.cookie_name, result.session_token),
            true,
            Some(s.config.business_ttl.as_secs()),
        ))
        .map_err(|_| ApiError(Rejection::Database))?,
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie(
            &format!("__Host-bizfin_csrf={}", result.csrf_token),
            false,
            Some(s.config.business_ttl.as_secs()),
        ))
        .map_err(|_| ApiError(Rejection::Database))?,
    );
    response.headers_mut().insert(
        "x-trace-id",
        HeaderValue::from_str(&result.trace_id.to_string())
            .map_err(|_| ApiError(Rejection::Database))?,
    );
    Ok(response)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionResponse {
    authenticated: bool,
    subject: String,
    display_name: String,
    csrf_required: bool,
    csrf_token: String,
    trace_id: Uuid,
}
async fn business_session(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<SessionResponse>, ApiError> {
    let token = cookie(&headers, &s.config.cookie_name).ok_or(ApiError(
        Rejection::Unauthorized("business_session_required"),
    ))?;
    let state = s.store.business_state(&token).await?;
    let csrf_token = s.store.refresh_business_csrf(&state).await?;
    Ok(Json(SessionResponse {
        authenticated: true,
        subject: state.subject,
        display_name: state.display_name,
        csrf_required: true,
        csrf_token,
        trace_id: state.trace_id,
    }))
}
async fn business_logout(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !origin_allowed(&headers, &s, true) {
        return Err(ApiError(Rejection::Forbidden("origin_rejected")));
    }
    let token = cookie(&headers, &s.config.cookie_name).ok_or(ApiError(
        Rejection::Unauthorized("business_session_required"),
    ))?;
    let state = s.store.business_state(&token).await?;
    let csrf = headers
        .get("x-csrf-token")
        .and_then(|v| v.to_str().ok())
        .ok_or(ApiError(Rejection::Forbidden("csrf_required")))?;
    if security::hash(csrf) != state.csrf_token_hash {
        return Err(ApiError(Rejection::Forbidden("csrf_rejected")));
    }
    s.store.business_logout(&state, facts(&headers)).await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie(
            &format!("{}=", s.config.cookie_name),
            true,
            Some(0),
        ))
        .unwrap(),
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&set_cookie("__Host-bizfin_csrf=", false, Some(0))).unwrap(),
    );
    no_store(response.headers_mut());
    Ok(response)
}
async fn workbench_logout(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    if !origin_allowed(&headers, &s, false) {
        return Err(ApiError(Rejection::Forbidden("origin_rejected")));
    }
    let f = facts(&headers);
    let p = principal(&s, &headers, &f).await?;
    s.store.workbench_logout(&p, false, f).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn global_logout(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !origin_allowed(&headers, &s, false) {
        return Err(ApiError(Rejection::Forbidden("origin_rejected")));
    }
    let f = facts(&headers);
    let p = principal(&s, &headers, &f).await?;
    s.store.workbench_logout(&p, true, f).await?;
    let url = format!(
        "{}/end-session/?post_logout_redirect_uri={}",
        s.config.authentik_issuer,
        url::form_urlencoded::byte_serialize(s.config.global_logout_redirect_uri.as_bytes())
            .collect::<String>()
    );
    Ok(Json(serde_json::json!({"logoutUrl":url})))
}

#[derive(Deserialize)]
struct LogoutForm {
    logout_token: String,
}
async fn backchannel_logout(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(body): Form<LogoutForm>,
) -> Result<StatusCode, ApiError> {
    let f = facts(&headers);
    let claims = s
        .verifier
        .logout(&body.logout_token)
        .await
        .map_err(|_| ApiError(Rejection::Unauthorized("logout_token_rejected")))?;
    s.store
        .backchannel_logout(claims.sid.as_deref(), &claims.iss, &claims.sub, f)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn issue_agent_delegation(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<IssueAgentDelegationRequest>,
) -> Result<Json<crate::agent::IssueAgentDelegationResponse>, ApiError> {
    if !service_authorized(&headers, &s) {
        return Err(ApiError(Rejection::Forbidden(
            "business_agent_disabled_or_service_rejected",
        )));
    }
    Ok(Json(
        s.store
            .issue_agent_delegation(body, facts(&headers))
            .await?,
    ))
}

async fn consume_agent_delegation(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ConsumeAgentDelegationRequest>,
) -> Result<Json<crate::agent::AgentDelegationContext>, ApiError> {
    if !service_authorized(&headers, &s) {
        return Err(ApiError(Rejection::Forbidden(
            "business_agent_disabled_or_service_rejected",
        )));
    }
    let token = bearer(&headers)
        .map_err(|_| ApiError(Rejection::Unauthorized("delegation_token_required")))?;
    Ok(Json(
        s.store
            .consume_agent_delegation(token, body, facts(&headers))
            .await?,
    ))
}

async fn revoke_agent_delegation(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if !service_authorized(&headers, &s) {
        return Err(ApiError(Rejection::Forbidden(
            "business_agent_disabled_or_service_rejected",
        )));
    }
    s.store.revoke_agent_delegation(id, facts(&headers)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn verify_agent_delegation(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<VerifyAgentDelegationRequest>,
) -> Result<Json<business_iam::EffectiveGrant>, ApiError> {
    if !service_authorized(&headers, &s) {
        return Err(ApiError(Rejection::Forbidden(
            "business_agent_disabled_or_service_rejected",
        )));
    }
    Ok(Json(
        s.store
            .verify_agent_delegation(body, facts(&headers))
            .await?,
    ))
}

async fn audit_agent_tool(
    State(s): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<AgentToolAuditRequest>,
) -> Result<StatusCode, ApiError> {
    if !service_authorized(&headers, &s) {
        return Err(ApiError(Rejection::Forbidden(
            "business_agent_disabled_or_service_rejected",
        )));
    }
    s.store.audit_agent_tool(body, facts(&headers)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cookie_is_host_only_secure_and_http_only() {
        let v = set_cookie("__Host-test=value", true, Some(30));
        assert!(v.contains("HttpOnly"));
        assert!(v.contains("Secure"));
        assert!(v.contains("SameSite=None"));
        assert!(v.contains("Partitioned"));
        assert!(!v.contains("Domain="));
    }
}
