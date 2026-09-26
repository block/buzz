use crate::{
    api::{bridge, relay_members},
    state::AppState,
};
use axum::{
    http::{header, HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use buzz_core::TenantContext;
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) struct Error {
    status: StatusCode,
    code: &'static str,
}
impl Error {
    pub(super) fn new(status: StatusCode, code: &'static str) -> Self {
        Self { status, code }
    }
    pub(super) fn unavailable() -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "unavailable")
    }
    pub(super) fn invalid() -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request")
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(json!({"error":{"code":self.code,
            "request_id":uuid::Uuid::new_v4().to_string()}})),
        )
            .into_response();
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("private, no-store"),
        );
        if self.status == StatusCode::TOO_MANY_REQUESTS
            || self.status == StatusCode::SERVICE_UNAVAILABLE
        {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, header::HeaderValue::from_static("60"));
        }
        response
    }
}

pub(super) fn bridge_error((status, _): (StatusCode, Json<Value>)) -> Error {
    let code = match status {
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::TOO_MANY_REQUESTS => "rate_limited",
        _ => "unavailable",
    };
    Error::new(status, code)
}

pub(super) struct Principal {
    pub(super) tenant: TenantContext,
    pub(super) actor: nostr::PublicKey,
    signed_at: Option<u64>,
}

pub(super) async fn authorize(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    uri: &Uri,
    method: &str,
    body: Option<&[u8]>,
) -> Result<Principal, Error> {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, host)
        .await
        .map_err(|_| Error::new(StatusCode::NOT_FOUND, "not_found"))?;
    if !state.config.buzz_v1_enabled {
        return Err(Error::new(StatusCode::NOT_FOUND, "not_found"));
    }
    let path = uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or(uri.path());
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    // Private state always requires cryptographic identity, even on a dev relay.
    let verified =
        bridge::verify_bridge_auth_with_options(headers, method, &url, body, true, body.is_some())
            .map_err(bridge_error)?;
    bridge::enforce_http_admission(state, &tenant, &verified.pubkey)
        .await
        .map_err(bridge_error)?;
    bridge::check_nip98_replay(state, &tenant, verified.event_id_bytes)
        .await
        .map_err(bridge_error)?;
    let principal = Principal {
        tenant,
        actor: verified.pubkey,
        signed_at: verified.signed_created_at,
    };
    recheck(state, headers, &principal).await?;
    Ok(principal)
}

pub(super) async fn recheck(
    state: &AppState,
    headers: &HeaderMap,
    principal: &Principal,
) -> Result<(), Error> {
    relay_members::enforce_relay_membership(
        state,
        principal.tenant.community(),
        &principal.actor.to_bytes(),
        relay_members::extract_auth_tag_header(headers),
        principal.signed_at,
    )
    .await
    .map_err(bridge_error)?;
    let restrictions = state
        .db
        .moderation_restriction_state(principal.tenant.community(), &principal.actor.to_bytes())
        .await
        .map_err(|_| Error::unavailable())?;
    if restrictions.banned {
        return Err(Error::new(StatusCode::FORBIDDEN, "forbidden"));
    }
    // Timeouts block posting conversation content, not personal reading.
    Ok(())
}

pub(super) fn response(value: impl serde::Serialize) -> Result<Response, Error> {
    let bytes = serde_json::to_vec(&value).map_err(|_| Error::unavailable())?;
    if bytes.len() > 1024 * 1024 {
        return Err(Error::unavailable());
    }
    Ok((
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "private, no-store"),
        ],
        bytes,
    )
        .into_response())
}
