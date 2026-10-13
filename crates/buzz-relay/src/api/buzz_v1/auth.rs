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

pub(super) enum Error {
    Application {
        status: StatusCode,
        code: &'static str,
    },
    Admission(Box<Response>),
}
impl Error {
    pub(super) fn new(status: StatusCode, code: &'static str) -> Self {
        Self::Application { status, code }
    }
    pub(super) fn unavailable() -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "unavailable")
    }
    pub(super) fn terminal_denial(&self) -> bool {
        let status = match self {
            Self::Application { status, .. } => *status,
            Self::Admission(response) => response.status(),
        };
        matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
    }
    pub(super) fn invalid() -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request")
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let mut response = match self {
            Self::Admission(response) => *response,
            Self::Application { status, code } => {
                let mut response = (
                    status,
                    Json(json!({"error":{"code":code,
                    "request_id":uuid::Uuid::new_v4().to_string()}})),
                )
                    .into_response();
                if status == StatusCode::TOO_MANY_REQUESTS
                    || status == StatusCode::SERVICE_UNAVAILABLE
                {
                    response
                        .headers_mut()
                        .insert(header::RETRY_AFTER, header::HeaderValue::from_static("60"));
                }
                response
            }
        };
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("private, no-store"),
        );
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
    method: &'static str,
    body: Option<&[u8]>,
) -> Result<Principal, Error> {
    let tenant = crate::nip_fi_shadow::bind_tenant(state, headers)
        .await
        .ok_or_else(|| Error::new(StatusCode::NOT_FOUND, "not_found"))?;
    let path = uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or(uri.path());
    let url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, path);
    // Private state always requires cryptographic identity, even on a dev relay.
    // NIP-FI admission owns NIP-98 extraction, assertion pairing and deny map.
    let admission = crate::nip_fi_http::admit_nip_fi_http_on_state(
        state,
        headers,
        bridge::make_nip98_closure_for_admission(
            headers.clone(),
            method,
            url,
            body.map(<[u8]>::to_vec),
            true,
            body.is_some(),
        ),
    )
    .map_err(|response| {
        if state.config.nip_fi.mode.restricts() {
            // Preserve the shared NIP-FI wire contract, not just its status.
            Error::Admission(Box::new(response))
        } else {
            bridge_error((response.status(), Json(Value::Null)))
        }
    })?;
    let actor = *admission.proven_pubkey();
    let (event_id, signed_at) = admission.into_extra();
    bridge::enforce_http_admission(state, &tenant, &actor)
        .await
        .map_err(bridge_error)?;
    bridge::check_nip98_replay(state, &tenant, event_id)
        .await
        .map_err(bridge_error)?;
    let principal = Principal {
        tenant,
        actor,
        signed_at,
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

/// Largest serialized API response; a larger one is a 503.
pub(super) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(super) fn response(value: impl serde::Serialize) -> Result<Response, Error> {
    let bytes = serde_json::to_vec(&value).map_err(|_| Error::unavailable())?;
    if bytes.len() > MAX_RESPONSE_BYTES {
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
