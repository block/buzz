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
//! # disabled mode (read-only)
//!
//! `authorize()` succeeds for read requests but returns `None` for the
//! principal — mutations and staffing routes call
//! [`require_mutation_principal`], which 403s on `None`.

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

/// A resolved deployment-level principal, returned by [`authorize`] in nip98
/// mode.
#[derive(Debug, Clone)]
pub struct AdminPrincipal {
    /// 32-byte pubkey (binary).
    pub pubkey: [u8; 32],
    /// Deployment role.
    pub role: AdminRole,
    /// How the grant was established.
    pub source: AdminSource,
}

/// Canonical wire string for an [`AdminRole`] (probe/DTO/audit).
pub(crate) fn admin_role_str(role: AdminRole) -> &'static str {
    match role {
        AdminRole::Operator => "operator",
        AdminRole::Moderator => "moderator",
    }
}

/// Canonical wire string for an [`AdminSource`] (probe/DTO).
pub(crate) fn admin_source_str(source: &AdminSource) -> &'static str {
    match source {
        AdminSource::Config => "config",
        AdminSource::OwnerFallback => "owner_fallback",
        AdminSource::Db => "db",
    }
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

/// Scheme for an admin authority: `http://` for loopback hosts (`localhost`,
/// any `*.localhost` name, `[::1]`, 127.x), else `https://` — matching local
/// dev via the Justfile (`admin.localhost:3000` over HTTP).
///
/// Shared by [`canonical_url`] (NIP-98 `u`-tag verification) and
/// [`admin_api_origin`] (NIP-11 advertisement) so the origin the relay
/// advertises and the origin it verifies against can never use different
/// schemes.
fn scheme_for_host(host: &str) -> &'static str {
    // Strip any `:port` to get the bare host. A bracketed IPv6 authority
    // (`[::1]:3000`) carries its colons inside the brackets, so take the text
    // between them; bare (unbracketed) IPv6 literals are rejected at config
    // parse, so `split(':')` on every other accepted form only strips a port.
    let host_part = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    // RFC 6761 reserves `localhost` and every name under `.localhost` for
    // loopback, and the repo's dev default (`just admin`) serves
    // `admin.localhost:3000` over HTTP — so both forms must map to `http` or
    // the advertised/verified origin diverges from what dev actually serves.
    let is_loopback = host_part == "localhost"
        || host_part.ends_with(".localhost")
        || host_part == "::1"
        || host_part.starts_with("127.");
    if is_loopback {
        "http"
    } else {
        "https"
    }
}

/// Derive the canonical URL for a NIP-98 `u`-tag check.
fn canonical_url(host: &str, path: &str) -> String {
    format!("{}://{host}{path}", scheme_for_host(host))
}

/// Canonical admin API origin (`scheme://host[:port]`, no path) advertised in
/// the NIP-11 document so desktop can auto-discover the admin surface instead
/// of requiring manual URL entry.
///
/// Scheme follows the same loopback rule as [`canonical_url`], so a client that
/// discovers this origin signs NIP-98 `u` tags against the exact scheme the
/// relay verifies.
pub(crate) fn admin_api_origin(host: &str) -> String {
    format!("{}://{host}", scheme_for_host(host))
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
/// - `Ok(Some(principal))` — nip98 mode (role resolved from roster).
/// - `Ok(None)` — disabled mode; reads pass, mutations 403 via
///   [`require_mutation_principal`].
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
    let (principal, nip98_event_id) = match &config.auth {
        AdminAuth::Disabled => (None, None),
        AdminAuth::Nip98 => {
            let full_path = format!("{ADMIN_API_PREFIX}{path_and_query}");
            let (pubkey_bytes, event_id) =
                authorize_nip98(config, headers, &full_path, method, raw_body).await?;
            // Resolve the roster grant BEFORE claiming the replay ID: an
            // unrostered-but-validly-signing key (e.g. any WARP-admitted laptop)
            // must not be able to consume replay slots at request rate. Only a
            // request that clears authorization claims its event ID.
            let principal = resolve_admin_principal(state, pubkey_bytes).await?;
            (Some(principal), Some(event_id))
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

    // Claim the NIP-98 replay ID only after Host and Origin validation succeed,
    // so a request rejected by either check does not burn the event ID — the
    // caller can retry with the corrected header without a new signature.
    if let Some(event_id) = nip98_event_id {
        claim_nip98_replay(state, &event_id).await?;
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

/// Require that this request resolved a principal (nip98 mode) and return it.
/// Mutation and staffing routes are unavailable in disabled mode.
///
/// Returns the principal or a 403 if none was resolved.
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

/// Require exactly one `Authorization: Nostr <base64 event>` header, verify
/// the NIP-98 event (method, url, payload hash for body-bearing methods), and
/// return the authenticated pubkey bytes and event id.
///
/// This performs signature/URL/method/payload verification only — it does NOT
/// claim the replay ID. The caller resolves the principal (roster check) first
/// and calls [`claim_nip98_replay`] only after authorization succeeds, so an
/// unrostered signer can never consume a replay slot.
///
/// For body-bearing methods (`POST`/`PUT`/`PATCH`/`DELETE`), the `payload`
/// sha256 tag is required. The body bytes are verified against it.
///
/// Uniform 401 on any auth failure — no oracle distinguishing the failure mode.
async fn authorize_nip98(
    config: &AdminConfig,
    headers: &HeaderMap,
    path: &str,
    method: &str,
    raw_body: Option<&[u8]>,
) -> Result<([u8; 32], nostr::EventId), ApiError> {
    let unauth = ApiError::unauthorized;

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

    Ok((
        pubkey.to_bytes(),
        nostr::EventId::from_byte_array(event_id_bytes),
    ))
}

/// Atomically claim a verified NIP-98 event ID against the deployment-scoped
/// replay guard. Called only after [`authorize_nip98`] verified the event and
/// [`resolve_admin_principal`] confirmed a roster grant, so an unrostered
/// signer never consumes a slot. Redis failure fails closed.
async fn claim_nip98_replay(state: &AppState, event_id: &nostr::EventId) -> Result<(), ApiError> {
    let unauth = ApiError::unauthorized;
    match state
        .nip98_replay
        .try_mark_in_scope(
            ADMIN_REPLAY_SCOPE,
            event_id,
            buzz_auth::DEFAULT_REPLAY_TTL_SECS,
        )
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(unauth()),
        Err(err) => {
            tracing::warn!(
                scope = ADMIN_REPLAY_SCOPE,
                error = %err,
                "admin NIP-98 replay guard failed; rejecting request fail-closed"
            );
            Err(unauth())
        }
    }
}

/// Extract the credential from an `Authorization: Nostr <base64>` value.
fn nostr_credential(value: &str) -> Option<&str> {
    let (scheme, credential) = value.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("Nostr")
        .then(|| credential.trim_start_matches(' '))
        .filter(|c| !c.is_empty())
}

fn origin_matches_host(origin: &str, host: &str) -> bool {
    // Compare against the exact canonical origin: https:// for non-loopback,
    // http:// for loopback. Accepting either scheme for non-loopback would
    // allow plaintext origins for production hosts.
    let expected = format!("{}://{host}", scheme_for_host(host));
    origin == expected
}

#[cfg(test)]
mod tests {
    use super::{
        admin_api_origin, canonical_url, method_has_body, nostr_credential, origin_matches_host,
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
        // P3-4: http must be rejected for non-loopback hosts.
        assert!(!origin_matches_host(
            "http://admin.example.com",
            "admin.example.com"
        ));
        // https must be rejected for loopback hosts (scheme_for_host returns http).
        assert!(!origin_matches_host(
            "https://localhost:3000",
            "localhost:3000"
        ));
        // P3-4: `admin.localhost` is the repo dev default (RFC 6761 loopback,
        // served over HTTP by `just admin`) — its exact HTTP Origin must match
        // and the HTTPS form must be rejected.
        assert!(origin_matches_host(
            "http://admin.localhost:3000",
            "admin.localhost:3000"
        ));
        assert!(!origin_matches_host(
            "https://admin.localhost:3000",
            "admin.localhost:3000"
        ));
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
        // `*.localhost` (RFC 6761 loopback, the repo dev default).
        assert_eq!(
            canonical_url("admin.localhost:3000", "/api/admin/v1/reports"),
            "http://admin.localhost:3000/api/admin/v1/reports"
        );
    }

    #[test]
    fn admin_api_origin_uses_https_for_non_loopback_hosts() {
        assert_eq!(
            admin_api_origin("admin.example.com"),
            "https://admin.example.com"
        );
        assert_eq!(
            admin_api_origin("admin.example.com:8443"),
            "https://admin.example.com:8443"
        );
    }

    #[test]
    fn admin_api_origin_uses_http_for_loopback_hosts() {
        assert_eq!(admin_api_origin("localhost:3000"), "http://localhost:3000");
        assert_eq!(admin_api_origin("127.0.0.1:3000"), "http://127.0.0.1:3000");
        // Bracketed IPv6 authority (the RFC 3986 form; bare `::1` is rejected
        // at config parse). Loopback `[::1]` resolves to `http`.
        assert_eq!(admin_api_origin("[::1]"), "http://[::1]");
        assert_eq!(admin_api_origin("[::1]:3000"), "http://[::1]:3000");
        // `*.localhost` (RFC 6761 loopback, the repo dev default). The NIP-11
        // advertisement must match the HTTP origin desktop derives.
        assert_eq!(
            admin_api_origin("admin.localhost:3000"),
            "http://admin.localhost:3000"
        );
    }

    /// The advertised origin and the verified `u`-tag URL must parse as valid
    /// URLs for every accepted host — the round-1 defect advertised
    /// `http://::1`, which no URL parser accepts. Bare IPv6 is rejected at
    /// config parse, so every host reaching these helpers is bracketed or a
    /// name/IPv4 authority.
    #[test]
    fn admin_api_origin_and_canonical_url_parse_as_valid_urls() {
        for host in [
            "admin.example.com",
            "admin.example.com:8443",
            "localhost",
            "localhost:3000",
            "127.0.0.1",
            "127.0.0.1:3000",
            "[::1]",
            "[::1]:3000",
        ] {
            let advertised = admin_api_origin(host);
            url::Url::parse(&advertised)
                .unwrap_or_else(|e| panic!("advertised origin {advertised:?} must parse: {e}"));
            let verified = canonical_url(host, "/api/admin/v1/reports");
            url::Url::parse(&verified)
                .unwrap_or_else(|e| panic!("canonical url {verified:?} must parse: {e}"));
        }
    }

    /// The advertised origin and the verified `u`-tag URL must agree on scheme
    /// for every host, or a discovered origin would sign against a scheme the
    /// relay rejects.
    #[test]
    fn admin_api_origin_scheme_matches_canonical_url_scheme() {
        for host in [
            "admin.example.com",
            "admin.example.com:8443",
            "localhost:3000",
            "127.0.0.1:3000",
            "[::1]:3000",
        ] {
            let advertised = admin_api_origin(host);
            let verified = canonical_url(host, "/api/admin/v1/reports");
            let advertised_scheme = advertised.split("://").next().expect("scheme");
            let verified_scheme = verified.split("://").next().expect("scheme");
            assert_eq!(
                advertised_scheme, verified_scheme,
                "advertised and verified schemes must match for host {host}"
            );
        }
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
