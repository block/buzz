//! Sprout admission helpers for mesh-LLM iroh-relay connections.
//!
//! This module intentionally keeps the admission logic independent from the
//! concrete `iroh-relay` server type for the first landing. Max's runtime lane
//! owns adding the iroh dependency and wiring this decision into
//! `AccessConfig::Restricted`. The security boundary lives here: a connecting
//! endpoint must present a NIP-98 bearer signed by a direct relay member.
//!
//! V1 deliberately admits direct relay members only. NIP-OA owner delegation for
//! agent-owned mesh nodes is a follow-up so the mesh compute trust boundary stays
//! legible.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use nostr::PublicKey;

use crate::api::relay_members::MembershipDecision;
use crate::state::AppState;

/// Path component appended to the configured iroh-relay public URL.
pub const IROH_RELAY_PATH: &str = "/relay";

const NIP98_METHOD: &str = "GET";
const MAX_BEARER_LEN: usize = 64 * 1024;

/// Result of a mesh iroh admission check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrohAdmissionDecision {
    /// Allow the connection for this direct relay member.
    Allow {
        /// NIP-98-proven member pubkey.
        pubkey: PublicKey,
    },
    /// Deny the connection. Reason is for server logs/tests, not client display.
    Deny(String),
}

/// Decide whether an iroh-relay connection should be admitted.
pub async fn decide_admission(
    state: &AppState,
    canonical_url: &str,
    auth_token: Option<&str>,
) -> IrohAdmissionDecision {
    let pubkey = match verify_bearer(canonical_url, auth_token) {
        Ok(pubkey) => pubkey,
        Err(reason) => return IrohAdmissionDecision::Deny(reason),
    };

    let membership =
        crate::api::relay_members::check_relay_membership(state, &pubkey.to_bytes(), None).await;
    admission_from_membership(membership, pubkey)
}

/// Map a relay-membership outcome to an iroh admission decision.
///
/// This is the mesh admission *invariant*, isolated from any I/O so it can be
/// asserted directly: relay membership is the only thing that admits. A direct
/// member (or an open relay) is allowed; everything else — non-members,
/// NIP-OA owner-delegated agents (denied in v1), and membership-check errors —
/// is denied. Possession of dial metadata or a valid NIP-98 bearer is by itself
/// never sufficient; only membership flips this to `Allow`.
pub fn admission_from_membership(
    membership: Result<MembershipDecision, String>,
    pubkey: PublicKey,
) -> IrohAdmissionDecision {
    match membership {
        Ok(MembershipDecision::OpenRelay) | Ok(MembershipDecision::Member) => {
            IrohAdmissionDecision::Allow { pubkey }
        }
        Ok(MembershipDecision::ViaOwner(_)) => IrohAdmissionDecision::Deny(
            // v1 deliberately denies NIP-OA owner-delegated agents even when their
            // owner is a relay member. HTTP endpoints accept that delegation; iroh
            // admission does not, keeping the mesh-compute trust boundary tighter
            // and legible. Lifting this is a follow-up gated on NIP-OA scope review.
            "owner-delegated mesh admission is not enabled in v1".to_string(),
        ),
        Ok(MembershipDecision::Denied) => {
            IrohAdmissionDecision::Deny(format!("not a relay member: {pubkey}"))
        }
        Err(error) => IrohAdmissionDecision::Deny(format!("membership check failed: {error}")),
    }
}

/// Verify a base64-encoded NIP-98 bearer for the iroh-relay WebSocket URL.
pub fn verify_bearer(canonical_url: &str, auth_token: Option<&str>) -> Result<PublicKey, String> {
    let token = match auth_token {
        Some(token) if !token.trim().is_empty() => token.trim(),
        _ => return Err("missing bearer token".to_string()),
    };
    if token.len() > MAX_BEARER_LEN {
        return Err(format!("bearer token exceeds {MAX_BEARER_LEN} bytes"));
    }

    let json = decode_bearer(token).ok_or_else(|| "bearer token is not base64 JSON".to_string())?;
    sprout_auth::verify_nip98_event(&json, canonical_url, NIP98_METHOD, None)
        .map_err(|error| format!("NIP-98 verification failed: {error}"))
}

fn decode_bearer(token: &str) -> Option<String> {
    for engine in [STANDARD, URL_SAFE, STANDARD_NO_PAD, URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(token) {
            if let Ok(json) = String::from_utf8(bytes) {
                return Some(json);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn bearer_for(keys: &Keys, canonical_url: &str, method: &str) -> String {
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", canonical_url]).unwrap(),
                Tag::parse(["method", method]).unwrap(),
            ])
            .custom_created_at(Timestamp::now())
            .sign_with_keys(keys)
            .unwrap();
        STANDARD.encode(serde_json::to_string(&event).unwrap())
    }

    #[test]
    fn verify_bearer_accepts_valid_nip98() {
        let keys = Keys::generate();
        let url = "https://relay.example/iroh/relay";
        let token = bearer_for(&keys, url, NIP98_METHOD);

        assert_eq!(verify_bearer(url, Some(&token)).unwrap(), keys.public_key());
    }

    #[test]
    fn verify_bearer_rejects_missing_token() {
        assert!(verify_bearer("https://relay.example/iroh/relay", None).is_err());
    }

    #[test]
    fn verify_bearer_rejects_wrong_method() {
        let keys = Keys::generate();
        let url = "https://relay.example/iroh/relay";
        let token = bearer_for(&keys, url, "POST");

        let error = verify_bearer(url, Some(&token)).unwrap_err();
        assert!(error.contains("method"), "{error}");
    }

    #[test]
    fn verify_bearer_rejects_wrong_url() {
        let keys = Keys::generate();
        let token = bearer_for(&keys, "https://other.example/iroh/relay", NIP98_METHOD);

        let error = verify_bearer("https://relay.example/iroh/relay", Some(&token)).unwrap_err();
        assert!(error.contains("URL"), "{error}");
    }

    #[test]
    fn verify_bearer_rejects_expired_timestamp() {
        // NIP-98's ±60s window must reject stale bearers — guards against
        // observed-token replay outside the live admission moment.
        let keys = Keys::generate();
        let url = "https://relay.example/iroh/relay";
        let stale = Timestamp::now() - 120u64;
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::parse(["u", url]).unwrap(),
                Tag::parse(["method", NIP98_METHOD]).unwrap(),
            ])
            .custom_created_at(stale)
            .sign_with_keys(&keys)
            .unwrap();
        let token = STANDARD.encode(serde_json::to_string(&event).unwrap());

        let error = verify_bearer(url, Some(&token)).unwrap_err();
        assert!(error.contains("window"), "{error}");
    }
    // ── Admission invariant: relay membership is the ONLY factor ──────────────
    //
    // These assert the policy mapping directly (no db/AppState needed). The
    // bearer-proof layer is covered by the verify_bearer tests above; here we
    // pin that a *proven* identity is admitted iff it is a relay member.

    fn any_pubkey() -> PublicKey {
        Keys::generate().public_key()
    }

    #[test]
    fn admission_allows_direct_relay_member() {
        let pk = any_pubkey();
        let decision = admission_from_membership(Ok(MembershipDecision::Member), pk);
        assert_eq!(decision, IrohAdmissionDecision::Allow { pubkey: pk });
    }

    #[test]
    fn admission_allows_when_relay_is_open() {
        // require_relay_membership disabled → OpenRelay → admitted.
        let pk = any_pubkey();
        let decision = admission_from_membership(Ok(MembershipDecision::OpenRelay), pk);
        assert_eq!(decision, IrohAdmissionDecision::Allow { pubkey: pk });
    }

    #[test]
    fn admission_denies_non_member() {
        // A valid Nostr identity that is not a relay member gets nothing.
        let pk = any_pubkey();
        let decision = admission_from_membership(Ok(MembershipDecision::Denied), pk);
        assert!(matches!(decision, IrohAdmissionDecision::Deny(_)));
    }

    #[test]
    fn admission_denies_owner_delegation_in_v1() {
        // NIP-OA owner-delegated agents are explicitly NOT admitted to the mesh
        // in v1, even though HTTP endpoints accept the same delegation.
        let pk = any_pubkey();
        let owner = any_pubkey();
        let decision = admission_from_membership(Ok(MembershipDecision::ViaOwner(owner)), pk);
        match decision {
            IrohAdmissionDecision::Deny(reason) => {
                assert!(reason.contains("owner-delegated"), "{reason}");
            }
            other => panic!("owner delegation must be denied, got {other:?}"),
        }
    }

    #[test]
    fn admission_denies_on_membership_check_error() {
        // Fail closed: an errored membership lookup denies, never admits.
        let pk = any_pubkey();
        let decision = admission_from_membership(Err("db down".to_string()), pk);
        assert!(matches!(decision, IrohAdmissionDecision::Deny(_)));
    }
}
