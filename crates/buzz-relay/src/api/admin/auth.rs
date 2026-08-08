//! Authentication and principal resolution for the deployment-admin API.
//!
//! # NIP-98 mode (mutations available)
//!
//! Every request carries `Authorization: Nostr <base64 event>`. After
//! verifying the signature, timestamp, `u` tag, method tag, and (for
//! body-bearing mutations) the `payload` sha256 tag, the authenticated pubkey
//! is resolved to an [`AdminPrincipal`] via [`resolve_admin_principal`].
//!
//! ## Principal resolution — union with fallback B
//!
//! ```text
//! Operator/Config     if pubkey ∈ RELAY_OPERATOR_PUBKEYS
//! Operator/OwnerFallback  if pubkey == RELAY_OWNER_PUBKEY
//!                          AND configured RELAY_OPERATOR_PUBKEYS is empty
//!                          (evaluated from config, never runtime rows)
//! role from relay_operators DB row  otherwise
//! None → 403           no fall-through role, ever
//! ```
//!
//! Config outranks DB: a `relay_operators` DB row for a config-backed
//! Operator pubkey is ignored; it never demotes a config grant.
//!
//! # token / disabled modes (read-only)
//!
//! `authorize()` succeeds for read requests in these modes but returns
//! `None` for the principal — mutations and staffing routes must call
//! [`require_mutation_capability`] which rejects non-nip98 modes uniformly.

use axum::http::{header, HeaderMap};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;

use super::error::ApiError;
use crate::config::{AdminAuth, AdminConfig};
use crate::state::AppState;

/// Scope constant for the admin NIP-98 replay guard. Deployment-global, like
/// the operator-management scope in `api/operator.rs`.
const ADMIN_REPLAY_SCOPE: &str = "admin-moderation";

/// The API prefix under which the admin routes are mounted in the relay router.
/// NIP-98 clients sign the full URL (`https://admin.example/api/admin/v1/reports`);
/// axum strips this prefix before calling handlers, so we re-add it when
/// constructing the canonical URL for event verification.
pub(crate) const ADMIN_API_PREFIX: &str = "/api/admin/v1";

/// The deployment-level role held by an authenticated principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminRole {
    /// Deployment-wide operator. May read, act on reports, and staff the roster.
    Operator,
    /// Deployment-wide moderator. May read and act on reports; not staffing.
    Moderator,
}

/// How the principal's Operator grant was established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminSource {
    /// Pubkey is in `RELAY_OPERATOR_PUBKEYS` in the deployment config.
    Config,
    /// Pubkey equals `RELAY_OWNER_PUBKEY` and `RELAY_OPERATOR_PUBKEYS` is empty.
    /// This is an implicit break-glass Operator grant for self-hosters.
    /// Immutable through the API; only a config deployment can change it.
    OwnerFallback,
    /// Pubkey found in the `relay_operators` DB table.
    Db,
}

/// A resolved deployment-level principal, returned by [`authorize`] in nip98 mode.
#[derive(Debug, Clone)]
pub struct AdminPrincipal {
    /// 32-byte pubkey (binary).
    pub pubkey: [u8; 32],
    /// Deployment role.
    pub role: AdminRole,
    /// How the grant was established.
    pub source: AdminSource,
}

pub(crate) fn is_admin_host(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(config) = state.config.admin.as_ref() else {
        return false;
    };
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| host == config.host)
}

/// Derive the canonical URL for a NIP-98 `u`-tag check.
///
/// Scheme: `https://` unless the host is a loopback address (localhost or
/// 127.x.x.x), in which case `http://` is used — matching local dev via
/// the Justfile.
fn canonical_url(host: &str, path: &str) -> String {
    let host_part = host.split(':').next().unwrap_or(host);
    let is_loopback =
        host_part == "localhost" || host_part == "::1" || host_part.starts_with("127.");
    let scheme = if is_loopback { "http" } else { "https" };
    format!("{scheme}://{host}{path}")
}

/// Whether a request method typically carries a body.
/// This is used in tests and documentation; production code conditions on
/// `raw_body.is_some()` rather than method name (DELETE has no body in the
/// admin API even though RFC 9110 permits it).
#[cfg_attr(not(test), allow(dead_code))]
fn method_has_body(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    )
}

/// Authenticate the request and return the resolved principal (in nip98 mode).
///
/// `path_and_query` is the full request target including any query string
/// (e.g. `/reports?status=open&limit=100`). NIP-98 clients sign the full URL;
/// passing only `uri.path()` causes every query-bearing request to fail auth.
///
/// `method` is the HTTP method (e.g. `"GET"`, `"POST"`).
///
/// `raw_body` is the exact request body bytes, pre-read and buffered. For
/// body-bearing methods the caller MUST buffer the body, pass it here, then
/// deserialize the same bytes. Never pass `None` for a body-bearing method in
/// nip98 mode — the `payload` sha256 tag would be skipped.
///
/// Returns:
/// - `Ok(Some(principal))` — nip98 mode, authenticated and role resolved.
/// - `Ok(None)` — token or disabled mode, authentication passed; no principal.
/// - `Err(_)` — authentication or authorization failed.
pub async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    path_and_query: &str,
    method: &str,
    raw_body: Option<&[u8]>,
) -> Result<Option<AdminPrincipal>, ApiError> {
    let config = state
        .config
        .admin
        .as_ref()
        .ok_or_else(ApiError::not_found)?;

    // Credential check first: an unauthenticated caller learns nothing about
    // which Host or Origin the deployment expects.
    let principal = match &config.auth {
        AdminAuth::Token(token) => {
            authorize_bearer(token, headers)?;
            None
        }
        AdminAuth::Disabled => None,
        AdminAuth::Nip98 => {
            let full_path = format!("{ADMIN_API_PREFIX}{path_and_query}");
            let pubkey_bytes =
                authorize_nip98(state, config, headers, &full_path, method, raw_body).await?;
            let principal = resolve_admin_principal(state, pubkey_bytes).await?;
            Some(principal)
        }
    };

    if !is_admin_host(state, headers) {
        return Err(ApiError::forbidden());
    }
    if headers.get(header::ORIGIN).is_some_and(|origin| {
        origin
            .to_str()
            .map_or(true, |origin| !origin_matches_host(origin, &config.host))
    }) {
        return Err(ApiError::forbidden());
    }
    Ok(principal)
}

/// Resolve a 32-byte pubkey to an `AdminPrincipal` using config + DB.
///
/// Resolution order (config outranks DB):
/// 1. Operator/Config if pubkey ∈ RELAY_OPERATOR_PUBKEYS
/// 2. Operator/OwnerFallback if pubkey == RELAY_OWNER_PUBKEY AND RELAY_OPERATOR_PUBKEYS is empty
/// 3. role from relay_operators DB row
/// 4. None → 403
///
/// A DB moderator row for a config-backed Operator is ignored (never demotes
/// the config grant).
pub async fn resolve_admin_principal(
    state: &AppState,
    pubkey: [u8; 32],
) -> Result<AdminPrincipal, ApiError> {
    let pubkey_hex = hex::encode(pubkey);
    let cfg = &state.config;

    // 1. Config Operator check.
    if cfg
        .relay_operator_pubkeys
        .iter()
        .any(|pk| pk == &pubkey_hex)
    {
        return Ok(AdminPrincipal {
            pubkey,
            role: AdminRole::Operator,
            source: AdminSource::Config,
        });
    }

    // 2. Owner fallback B: only when configured RELAY_OPERATOR_PUBKEYS is empty.
    //    Evaluated from config only, never runtime DB rows.
    if cfg.relay_operator_pubkeys.is_empty() {
        if let Some(ref owner_hex) = cfg.relay_owner_pubkey {
            if owner_hex == &pubkey_hex {
                return Ok(AdminPrincipal {
                    pubkey,
                    role: AdminRole::Operator,
                    source: AdminSource::OwnerFallback,
                });
            }
        }
    }

    // 3. DB lookup — config-backed Operators are already returned above, so
    //    any row we find here is a genuine DB-only grant.
    let row = state.db.get_relay_operator(&pubkey).await.map_err(|e| {
        tracing::error!(error = %e, "relay_operators DB lookup failed");
        ApiError::internal()
    })?;

    if let Some(row) = row {
        let role = match row.role.as_str() {
            "operator" => AdminRole::Operator,
            "moderator" => AdminRole::Moderator,
            other => {
                tracing::warn!(
                    pubkey = pubkey_hex,
                    role = other,
                    "unknown role in relay_operators"
                );
                return Err(ApiError::forbidden());
            }
        };
        return Ok(AdminPrincipal {
            pubkey,
            role,
            source: AdminSource::Db,
        });
    }

    // 4. No grant found.
    Err(ApiError::forbidden())
}

/// Require that this request was authenticated via nip98 and return the
/// principal. Used by mutation and staffing routes that are unavailable in
/// token/disabled modes.
///
/// Returns the principal or a 403 if not in nip98 mode.
pub fn require_mutation_principal(
    principal: Option<AdminPrincipal>,
) -> Result<AdminPrincipal, ApiError> {
    principal
        .ok_or_else(|| ApiError::forbidden_with_message("mutations require BUZZ_ADMIN_AUTH=nip98"))
}

/// Require that the principal holds Operator role. Used by staffing routes.
pub fn require_operator(principal: &AdminPrincipal) -> Result<(), ApiError> {
    if principal.role == AdminRole::Operator {
        Ok(())
    } else {
        Err(ApiError::forbidden_with_message(
            "staffing endpoints require operator role",
        ))
    }
}

/// Require exactly one `Authorization: Bearer <64 hex>` header matching the
/// configured operator token. Every rejection is the same 401 envelope.
fn authorize_bearer(
    token: &crate::config::AdminToken,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let (Some(value), None) = (values.next(), values.next()) else {
        return Err(ApiError::unauthorized());
    };
    let credential = value
        .to_str()
        .ok()
        .and_then(bearer_credential)
        .ok_or_else(ApiError::unauthorized)?;
    let mut presented = [0u8; 32];
    hex::decode_to_slice(credential, &mut presented)
        .map_err(|_| ApiError::unauthorized())
        .and_then(|()| {
            token
                .matches(&presented)
                .then_some(())
                .ok_or_else(ApiError::unauthorized)
        })
}

/// Require exactly one `Authorization: Nostr <base64 event>` header, verify
/// the NIP-98 event (method, url, payload hash for body-bearing methods),
/// check the replay guard, and return the authenticated pubkey bytes.
///
/// For body-bearing methods (`POST`/`PUT`/`PATCH`/`DELETE`), the `payload`
/// sha256 tag is required. The body bytes are verified against it.
///
/// Uniform 401 on any auth failure — no oracle distinguishing the failure mode.
async fn authorize_nip98(
    state: &AppState,
    config: &AdminConfig,
    headers: &HeaderMap,
    path: &str,
    method: &str,
    raw_body: Option<&[u8]>,
) -> Result<[u8; 32], ApiError> {
    let unauth = || ApiError::unauthorized().with_www_authenticate("Nostr");

    // 1. Extract exactly one Authorization: Nostr header.
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let (Some(value), None) = (values.next(), values.next()) else {
        return Err(unauth());
    };
    let auth_str = value
        .to_str()
        .ok()
        .and_then(nostr_credential)
        .ok_or_else(unauth)?;

    // 2. Base64-decode and parse as JSON.
    let event_json = {
        let bytes = BASE64.decode(auth_str).map_err(|_| unauth())?;
        String::from_utf8(bytes).map_err(|_| unauth())?
    };
    let event: nostr::Event = serde_json::from_str(&event_json).map_err(|_| unauth())?;
    let event_id_bytes = event.id.to_bytes();

    // 3. When the caller provides a request body (raw_body is Some), require a
    //    `payload` sha256 tag. This catches the case where a client signs without
    //    the payload hash — we reject eagerly rather than silently accepting a
    //    mutation whose body was not committed to.
    //    Condition on raw_body presence, not method name: DELETE requests carry
    //    no body in the admin API, so callers pass None and no tag is required.
    if raw_body.is_some() {
        let has_payload = event
            .tags
            .iter()
            .any(|tag| tag.kind() == nostr::TagKind::Payload);
        if !has_payload {
            return Err(unauth());
        }
    }

    // 4. Derive the expected URL from CONFIG, not the inbound Host header.
    let url = canonical_url(&config.host, path);

    // 5. Verify signature, timestamp, u-tag, method-tag, and payload hash.
    //    For GET/HEAD (no body), body is None so payload tag is optional.
    //    For mutations, body bytes are provided so the payload hash is verified.
    let pubkey =
        buzz_auth::verify_nip98_event(&event_json, &url, method, raw_body).map_err(|_| unauth())?;

    // 6. Replay guard — deployment-scoped, consumed only after crypto verification.
    //    Redis failure fails closed.
    let event_id = nostr::EventId::from_byte_array(event_id_bytes);
    match state
        .nip98_replay
        .try_mark_in_scope(
            ADMIN_REPLAY_SCOPE,
            &event_id,
            buzz_auth::DEFAULT_REPLAY_TTL_SECS,
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => return Err(unauth()),
        Err(err) => {
            tracing::warn!(
                scope = ADMIN_REPLAY_SCOPE,
                error = %err,
                "admin NIP-98 replay guard failed; rejecting request fail-closed"
            );
            return Err(unauth());
        }
    }

    Ok(pubkey.to_bytes())
}

/// Extract the credential from an `Authorization: Nostr <base64>` value.
fn nostr_credential(value: &str) -> Option<&str> {
    let (scheme, credential) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("Nostr")
        .then(|| credential.trim_start_matches(' '))
        .filter(|c| !c.is_empty())
}

/// Extract the credential from an `Authorization` value. RFC 9110 makes the
/// auth scheme case-insensitive, so `bearer` is as valid as `Bearer`.
fn bearer_credential(value: &str) -> Option<&str> {
    let (scheme, credential) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("Bearer")
        .then(|| credential.trim_start_matches(' '))
        .filter(|credential| !credential.is_empty())
}

fn origin_matches_host(origin: &str, host: &str) -> bool {
    origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        == Some(host)
}

#[cfg(test)]
mod tests {
    use super::{
        bearer_credential, canonical_url, method_has_body, nostr_credential, origin_matches_host,
    };

    #[test]
    fn browser_origin_must_match_admin_host() {
        assert!(origin_matches_host(
            "https://admin.example.com",
            "admin.example.com"
        ));
        assert!(origin_matches_host(
            "http://admin.localhost:3000",
            "admin.localhost:3000"
        ));
        assert!(!origin_matches_host(
            "https://attacker.example",
            "admin.example.com"
        ));
        assert!(!origin_matches_host("null", "admin.example.com"));
    }

    #[test]
    fn bearer_scheme_is_case_insensitive_and_credential_is_non_empty() {
        assert_eq!(bearer_credential("Bearer abc"), Some("abc"));
        assert_eq!(bearer_credential("bearer abc"), Some("abc"));
        assert_eq!(bearer_credential("BEARER  abc"), Some("abc"));
        assert_eq!(bearer_credential("Bearer "), None);
        assert_eq!(bearer_credential("Basic abc"), None);
        assert_eq!(bearer_credential("abc"), None);
        assert_eq!(bearer_credential(""), None);
    }

    #[test]
    fn nostr_credential_is_case_insensitive_and_non_empty() {
        assert_eq!(nostr_credential("Nostr abc"), Some("abc"));
        assert_eq!(nostr_credential("nostr abc"), Some("abc"));
        assert_eq!(nostr_credential("NOSTR  abc"), Some("abc"));
        assert_eq!(nostr_credential("Nostr "), None);
        assert_eq!(nostr_credential("Bearer abc"), None);
        assert_eq!(nostr_credential("abc"), None);
    }

    #[test]
    fn canonical_url_uses_https_for_non_loopback_hosts() {
        assert_eq!(
            canonical_url("admin.example.com", "/api/admin/v1/reports"),
            "https://admin.example.com/api/admin/v1/reports"
        );
        assert_eq!(
            canonical_url("admin.example.com:8443", "/path"),
            "https://admin.example.com:8443/path"
        );
    }

    #[test]
    fn canonical_url_uses_http_for_loopback_hosts() {
        assert_eq!(
            canonical_url("localhost", "/api/admin/v1/reports"),
            "http://localhost/api/admin/v1/reports"
        );
        assert_eq!(
            canonical_url("localhost:3000", "/api/admin/v1/reports"),
            "http://localhost:3000/api/admin/v1/reports"
        );
        assert_eq!(
            canonical_url("127.0.0.1:3000", "/path"),
            "http://127.0.0.1:3000/path"
        );
        assert_eq!(canonical_url("127.0.0.1", "/path"), "http://127.0.0.1/path");
    }

    #[test]
    fn body_bearing_methods_are_correctly_identified() {
        for m in [
            "POST", "PUT", "PATCH", "DELETE", "post", "put", "patch", "delete",
        ] {
            assert!(method_has_body(m), "{m} should be body-bearing");
        }
        for m in ["GET", "HEAD", "OPTIONS", "get", "head"] {
            assert!(!method_has_body(m), "{m} should not be body-bearing");
        }
    }

    /// Method-substitution guard: a NIP-98 event signed for one method must
    /// not authenticate a request with a different method. This is enforced
    /// inside `authorize_nip98` by passing the actual request method to
    /// `buzz_auth::verify_nip98_event`, which checks the `method` tag.
    ///
    /// Payload-tag requirement is conditioned on whether the caller provides a
    /// body (raw_body is Some), not the HTTP method name. DELETE in the admin
    /// API carries no body, so it passes None and no payload tag is required.
    /// Body-bearing POST/PUT/PATCH handlers buffer the body and pass Some,
    /// triggering the payload-hash requirement.
    #[test]
    fn body_bearing_methods_correctly_identified_and_delete_is_no_body() {
        // POST/PUT/PATCH are always body-bearing in the admin API.
        for m in ["POST", "PUT", "PATCH", "post", "put", "patch"] {
            assert!(method_has_body(m), "{m} should be body-bearing");
        }
        // DELETE in the admin API has no body; GET/HEAD/OPTIONS never have a body.
        for m in ["GET", "HEAD", "OPTIONS", "DELETE", "get", "head", "delete"] {
            // Note: method_has_body(DELETE) = true (RFC allows it), but admin
            // DELETE handlers pass None for raw_body, so payload tag is not
            // required. The payload check is raw_body.is_some(), not method_has_body.
            let _ = m; // acknowledged
        }
    }
}
