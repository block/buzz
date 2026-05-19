//! Embedded iroh-relay server, gated by Sprout relay membership.
//!
//! This is the **Step 3** half of the mesh-LLM plan (v6.1). The desktop
//! sidecar connects its iroh endpoint to `iroh_relay_url` advertised in the
//! Sprout NIP-11 document; this module hosts that relay endpoint inside the
//! Sprout process and gates every connection with the same NIP-98 + relay-
//! membership check we already use for HTTP entry points.
//!
//! The result: mesh-LLM QUIC traffic never leaves the relay's trust boundary
//! and **n0's public relays are never in the path**. No subscriptions, no
//! signups, no out-of-band config — relay members get pooled compute "for
//! free" once they install Sprout.
//!
//! # Access flow
//!
//! 1. The iroh client opens a WebSocket to `https://<relay>/iroh/relay`
//!    carrying `Authorization: Bearer <base64(NIP-98 event JSON)>` and its
//!    proven `EndpointId` (proved by iroh-relay's handshake before we run).
//! 2. iroh-relay calls our [`AccessConfig::Restricted`] callback with the
//!    [`ClientRequest`].
//! 3. We verify the NIP-98 event against the canonical relay URL using
//!    [`sprout_auth::nip98_canonical_url`] + [`sprout_auth::verify_nip98_event`].
//!    Any failure → `Access::Deny`. This proves the connecting pubkey.
//! 4. We run [`crate::api::relay_members::check_relay_membership`] against
//!    that pubkey. Anything other than `OpenRelay`/`Member`/`ViaOwner` →
//!    `Access::Deny`.
//! 5. We log the bound (NIP-98 pubkey, EndpointId, decision) for audit.
//!
//! No state is cached: every connection re-runs steps 3 + 4. The cost is one
//! Schnorr verify and 1-2 DB reads per connection, which is negligible
//! versus the QUIC + model traffic that follows.
//!
//! # Patched-fork hooks (forward-looking)
//!
//! Upstream iroh-relay rc.0 does not expose a per-client maximum-lifetime
//! hook. The mesh-LLM plan (v6.1, upstream PR C) will add one so we can
//! force re-auth every N minutes. The `patched-iroh-relay` Cargo feature
//! and the `TODO(patched-iroh-relay)` marker in [`spawn`] are reserved
//! insertion points for that wiring — **the hook is not implemented yet**.
//! Unpatched rc.0 stays compile-clean either way.

use std::net::SocketAddr;
use std::sync::Arc;

use iroh_relay::server::{
    Access, AccessConfig, ClientRequest, RelayConfig, Server, ServerConfig, SpawnError,
};
use tracing::{debug, info, warn};

use crate::api::relay_members::{check_relay_membership, MembershipDecision};
use crate::state::AppState;

/// Path component appended to `iroh_relay_public_url` for the access check.
///
/// Iroh's WebSocket upgrade always hits `/relay`; if the relay is reverse-
/// proxied under a path prefix (e.g. `https://host/iroh`), the full canonical
/// URL is `https://host/iroh/relay`. The NIP-98 signer and verifier both
/// compute this via [`sprout_auth::nip98_canonical_url`].
pub const IROH_RELAY_PATH: &str = "/relay";

/// HTTP method bound into NIP-98 events for iroh-relay connection auth.
const NIP98_METHOD: &str = "GET";

/// Maximum size of a bearer token (raw, pre-base64-decode) we'll even try to
/// process. A well-formed NIP-98 event JSON is well under a kilobyte; the
/// base64 expansion of that is ~1.4 KiB. We allow up to 64 KiB so generous
/// signers don't trip over `payload` tag hashes etc., but reject anything
/// larger before allocating decode buffers — admission requests must not be
/// able to coerce the relay into multi-megabyte allocations.
const MAX_BEARER_LEN: usize = 64 * 1024;

/// Handle returned by [`spawn`].
///
/// Dropping the handle aborts the iroh-relay supervisor task (via
/// `AbortOnDropHandle` inside `Server`). For graceful drain, prefer
/// [`IrohRelayHandle::shutdown`] which waits for in-flight connections to
/// finish before returning.
pub struct IrohRelayHandle {
    /// The bound HTTP address (resolved if the caller passed port 0).
    pub http_addr: Option<SocketAddr>,
    /// The bound HTTPS address, if TLS was configured.
    pub https_addr: Option<SocketAddr>,
    server: Server,
}

impl IrohRelayHandle {
    /// Request graceful shutdown of the embedded iroh-relay.
    ///
    /// Returns once all relay tasks have stopped. Used by the Sprout main
    /// shutdown loop so SIGTERM stops admitting new mesh-LLM connections
    /// alongside the HTTP listener, instead of yanking the socket out from
    /// under in-flight QUIC sessions.
    pub async fn shutdown(self) -> Result<(), iroh_relay::server::SupervisorError> {
        self.server.shutdown().await
    }
}

/// Spawn an embedded iroh-relay bound to `bind_addr`, gated by Sprout's
/// NIP-98 + relay-membership check.
///
/// Returns `Ok(None)` if `state.config.iroh_relay_public_url` is not set:
/// without a stable public URL the NIP-98 `u`-tag can't be canonicalised, so
/// hosting an iroh-relay endpoint would just produce an undebuggable storm
/// of `URL mismatch` denials. We surface that as "not enabled" instead.
pub async fn spawn(
    state: Arc<AppState>,
    bind_addr: SocketAddr,
) -> Result<Option<IrohRelayHandle>, SpawnError> {
    let Some(public_url) = state.config.iroh_relay_public_url.clone() else {
        info!("SPROUT_IROH_RELAY_PUBLIC_URL not set — embedded iroh-relay disabled");
        return Ok(None);
    };

    let canonical_url = match sprout_auth::nip98_canonical_url(&public_url, IROH_RELAY_PATH) {
        Some(u) => u,
        None => {
            warn!(
                public_url = %public_url,
                "SPROUT_IROH_RELAY_PUBLIC_URL is not a parseable URL — iroh-relay disabled",
            );
            return Ok(None);
        }
    };

    info!(
        bind_addr = %bind_addr,
        canonical_url = %canonical_url,
        "spawning embedded iroh-relay",
    );

    let access = build_access_config(state.clone(), canonical_url);

    let mut relay = RelayConfig::new(bind_addr);
    relay.access = access;

    // TODO(patched-iroh-relay): once upstream PR C lands, set the per-client
    // maximum-lifetime hook here (gated on `#[cfg(feature = "patched-iroh-relay")]`)
    // so we force re-auth every N minutes. Until then the connection lifetime
    // is whatever iroh-relay's defaults are.

    let mut cfg = ServerConfig::default();
    cfg.relay = Some(relay);

    let server = Server::spawn(cfg).await?;
    Ok(Some(IrohRelayHandle {
        http_addr: server.http_addr(),
        https_addr: server.https_addr(),
        server,
    }))
}

/// Build the [`AccessConfig::Restricted`] callback that gates every
/// iroh-relay connection on (NIP-98 ∧ relay-membership).
fn build_access_config(state: Arc<AppState>, canonical_url: String) -> AccessConfig {
    AccessConfig::Restricted(Box::new(move |request: &ClientRequest| {
        let state = state.clone();
        let canonical_url = canonical_url.clone();
        let endpoint_id = request.endpoint_id();
        let auth_token = request.auth_token();
        Box::pin(async move {
            match decide(&state, &canonical_url, auth_token.as_deref()).await {
                Decision::Allow { pubkey, owner } => {
                    debug!(
                        endpoint = %endpoint_id,
                        pubkey = %pubkey,
                        via_owner = ?owner,
                        "iroh-relay admission allowed",
                    );
                    Access::Allow
                }
                Decision::Deny(reason) => {
                    debug!(
                        endpoint = %endpoint_id,
                        reason = %reason,
                        "iroh-relay admission denied",
                    );
                    Access::Deny
                }
            }
        })
    }))
}

/// Internal decision type for the access callback — kept separate so it's
/// straightforward to unit-test [`decide`] without spinning up a full server.
#[derive(Debug)]
enum Decision {
    /// Connection should be admitted.
    Allow {
        /// The NIP-98-proven pubkey of the connecting client.
        pubkey: nostr::PublicKey,
        /// `Some(owner)` if admission was via NIP-OA delegation.
        owner: Option<nostr::PublicKey>,
    },
    /// Connection should be rejected, with a debug-only reason string.
    Deny(String),
}

/// Pure-logic admission decision, decoupled from iroh-relay's types so it
/// can be unit-tested with a real [`AppState`] and a synthetic bearer token.
async fn decide(state: &AppState, canonical_url: &str, auth_token: Option<&str>) -> Decision {
    // Step 1+2+3 — extract and verify the NIP-98 bearer to recover the
    // Nostr pubkey. This sub-function is unit-testable in isolation.
    let pubkey = match verify_bearer(canonical_url, auth_token) {
        Ok(pk) => pk,
        Err(reason) => return Decision::Deny(reason),
    };

    // Step 4 — now and only now, run the membership check. We pass the NIP-98
    // pubkey bytes, not the iroh `EndpointId` — the latter is just an
    // anonymous network identifier; membership is on Nostr identity.
    match check_relay_membership(state, &pubkey.to_bytes(), None).await {
        Ok(MembershipDecision::OpenRelay) | Ok(MembershipDecision::Member) => Decision::Allow {
            pubkey,
            owner: None,
        },
        Ok(MembershipDecision::ViaOwner(owner)) => Decision::Allow {
            pubkey,
            owner: Some(owner),
        },
        Ok(MembershipDecision::Denied) => Decision::Deny(format!("not a relay member: {}", pubkey)),
        Err(e) => {
            // Infrastructure failure. Fail closed.
            warn!("iroh-relay membership check infra error: {e}");
            Decision::Deny(format!("membership check infra error: {e}"))
        }
    }
}

/// Decode + verify the bearer token, returning the proven Nostr pubkey.
///
/// Fail-closed on:
/// - missing/empty token,
/// - non-base64,
/// - non-UTF-8 JSON,
/// - any NIP-98 verification failure (wrong kind, bad signature, stale
///   timestamp, URL mismatch, method mismatch, payload mismatch).
///
/// The returned `String` is a debug-only deny reason; do not forward to
/// clients (some failures distinguish "what" from "why" in ways we don't
/// want to leak).
fn verify_bearer(
    canonical_url: &str,
    auth_token: Option<&str>,
) -> Result<nostr::PublicKey, String> {
    let token = match auth_token {
        Some(t) if !t.is_empty() => t,
        _ => return Err("missing or empty bearer token".to_string()),
    };

    // Pre-decode length cap (Mari's review note). NIP-98 events are tiny; an
    // attacker shouldn't be able to coerce the relay into allocating a
    // multi-megabyte decode buffer before any signature check runs.
    if token.len() > MAX_BEARER_LEN {
        return Err(format!(
            "bearer token exceeds {MAX_BEARER_LEN}-byte limit ({} bytes)",
            token.len()
        ));
    }

    let json =
        decode_bearer(token).ok_or_else(|| "bearer token is not valid base64".to_string())?;

    sprout_auth::verify_nip98_event(&json, canonical_url, NIP98_METHOD, None)
        .map_err(|e| format!("NIP-98 verification failed: {e}"))
}

/// Decode a NIP-98 bearer token. NIP-98 specifies base64 over the JSON event;
/// some signers use URL-safe encoding and/or omit padding, so we accept both.
fn decode_bearer(token: &str) -> Option<String> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
    use base64::Engine;

    let trimmed = token.trim();
    for engine in [&STANDARD, &URL_SAFE, &STANDARD_NO_PAD, &URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(trimmed) {
            if let Ok(s) = String::from_utf8(bytes) {
                return Some(s);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use sprout_auth::nip98_canonical_url;

    /// Build a signed NIP-98 event JSON for the given canonical URL.
    fn signed_event_json(keys: &Keys, canonical_url: &str, method: &str) -> String {
        let event = EventBuilder::new(
            Kind::HttpAuth,
            "",
            vec![
                Tag::parse(&["u", canonical_url]).unwrap(),
                Tag::parse(&["method", method]).unwrap(),
            ],
        )
        .custom_created_at(Timestamp::now())
        .sign_with_keys(keys)
        .unwrap();
        serde_json::to_string(&event).unwrap()
    }

    fn bearer(json: &str) -> String {
        STANDARD.encode(json)
    }

    fn canonical() -> String {
        nip98_canonical_url("https://relay.example.com/iroh", IROH_RELAY_PATH).unwrap()
    }

    // ── Bearer verification ──────────────────────────────────────────────

    #[test]
    fn verify_bearer_accepts_valid_nip98() {
        let keys = Keys::generate();
        let url = canonical();
        let json = signed_event_json(&keys, &url, NIP98_METHOD);
        let token = bearer(&json);

        let result = verify_bearer(&url, Some(&token));
        assert!(result.is_ok(), "expected accept, got {result:?}");
        assert_eq!(result.unwrap(), keys.public_key());
    }

    #[test]
    fn verify_bearer_rejects_missing_token() {
        let url = canonical();
        let result = verify_bearer(&url, None);
        assert!(matches!(result, Err(ref e) if e.contains("missing")));
    }

    #[test]
    fn verify_bearer_rejects_empty_token() {
        let url = canonical();
        let result = verify_bearer(&url, Some(""));
        assert!(matches!(result, Err(ref e) if e.contains("missing")));
    }

    #[test]
    fn verify_bearer_rejects_non_base64() {
        let url = canonical();
        let result = verify_bearer(&url, Some("not!!!base64!!!"));
        assert!(matches!(result, Err(ref e) if e.contains("base64")));
    }

    #[test]
    fn verify_bearer_rejects_oversized_token() {
        // 64 KiB + 1 byte. Must be rejected by the length cap *before* any
        // decode allocation happens, so an attacker can't coerce a giant
        // base64 buffer.
        let url = canonical();
        let huge = "A".repeat(MAX_BEARER_LEN + 1);
        let result = verify_bearer(&url, Some(&huge));
        assert!(
            matches!(result, Err(ref e) if e.contains("exceeds")),
            "expected length-cap denial, got {result:?}",
        );
    }

    #[test]
    fn verify_bearer_rejects_internal_whitespace() {
        // base64 0.22's `general_purpose` engines reject internal whitespace
        // (no MIME mode). A valid token with a space spliced into the middle
        // must therefore fail decode, not be silently accepted as if the
        // whitespace were ignored.
        let keys = Keys::generate();
        let url = canonical();
        let json = signed_event_json(&keys, &url, NIP98_METHOD);
        let mut token = bearer(&json);
        // Splice a space into the middle of an otherwise valid token.
        let mid = token.len() / 2;
        token.insert(mid, ' ');

        let result = verify_bearer(&url, Some(&token));
        assert!(
            matches!(result, Err(ref e) if e.contains("base64")),
            "expected base64 denial on internal whitespace, got {result:?}",
        );
    }

    #[test]
    fn verify_bearer_rejects_wrong_method() {
        // NIP-98 event signed for POST but the iroh-relay handshake is GET.
        // The bearer must not be accepted with the wrong method.
        let keys = Keys::generate();
        let url = canonical();
        let json = signed_event_json(&keys, &url, "POST");
        let token = bearer(&json);

        let result = verify_bearer(&url, Some(&token));
        assert!(
            matches!(result, Err(ref e) if e.contains("method")),
            "expected method-mismatch denial, got {result:?}",
        );
    }

    #[test]
    fn verify_bearer_rejects_wrong_url() {
        // Event signed for a DIFFERENT relay URL must not authorize access
        // to *this* relay. This is the property that breaks if the canonical
        // helper drifts between signer and verifier.
        let keys = Keys::generate();
        let other_url =
            nip98_canonical_url("https://other-relay.example.com/iroh", IROH_RELAY_PATH).unwrap();
        let json = signed_event_json(&keys, &other_url, NIP98_METHOD);
        let token = bearer(&json);

        let result = verify_bearer(&canonical(), Some(&token));
        assert!(
            matches!(result, Err(ref e) if e.contains("URL")),
            "expected URL-mismatch denial, got {result:?}",
        );
    }

    #[test]
    fn verify_bearer_rejects_wrong_kind() {
        let keys = Keys::generate();
        let url = canonical();
        // Build a kind:1 (text note) event instead of kind:27235 — should fail.
        let event = EventBuilder::new(
            Kind::TextNote,
            "",
            vec![
                Tag::parse(&["u", &url]).unwrap(),
                Tag::parse(&["method", NIP98_METHOD]).unwrap(),
            ],
        )
        .sign_with_keys(&keys)
        .unwrap();
        let token = bearer(&serde_json::to_string(&event).unwrap());

        let result = verify_bearer(&url, Some(&token));
        assert!(
            matches!(result, Err(ref e) if e.contains("kind")),
            "expected kind-mismatch denial, got {result:?}",
        );
    }

    #[test]
    fn verify_bearer_rejects_stale_timestamp() {
        let keys = Keys::generate();
        let url = canonical();
        let event = EventBuilder::new(
            Kind::HttpAuth,
            "",
            vec![
                Tag::parse(&["u", &url]).unwrap(),
                Tag::parse(&["method", NIP98_METHOD]).unwrap(),
            ],
        )
        // Two hours in the past — well outside the ±60s NIP-98 tolerance.
        .custom_created_at(Timestamp::from(Timestamp::now().as_u64() - 7200))
        .sign_with_keys(&keys)
        .unwrap();
        let token = bearer(&serde_json::to_string(&event).unwrap());

        let result = verify_bearer(&url, Some(&token));
        assert!(
            matches!(result, Err(ref e) if e.contains("timestamp")),
            "expected timestamp denial, got {result:?}",
        );
    }

    // ── Bearer decoding ──────────────────────────────────────────────────

    #[test]
    fn decode_bearer_accepts_standard() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let payload = r#"{"hello":"world"}"#;
        let token = STANDARD.encode(payload);
        assert_eq!(decode_bearer(&token).as_deref(), Some(payload));
    }

    #[test]
    fn decode_bearer_accepts_url_safe_no_pad() {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        let payload = r#"{"hello":"world"}"#;
        let token = URL_SAFE_NO_PAD.encode(payload);
        assert_eq!(decode_bearer(&token).as_deref(), Some(payload));
    }

    #[test]
    fn decode_bearer_rejects_garbage() {
        assert!(decode_bearer("not base64 at all !!!").is_none());
    }
}
