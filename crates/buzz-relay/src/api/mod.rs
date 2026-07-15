//! HTTP API — media, git, NIP-05, and the Nostr HTTP bridge.

pub mod admin;
pub mod bridge;
pub mod events;
pub mod git;
pub mod invites;
pub mod media;
pub mod mesh_demo;
pub mod nip05;
pub mod operator;

// Re-export imeta helpers used by ingest pipeline.
pub use crate::handlers::imeta::{validate_imeta_tags, verify_imeta_blobs};

use axum::{http::StatusCode, response::Json};

/// Standard error envelope.
pub(crate) fn api_error(status: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({ "error": msg })))
}

pub(crate) fn internal_error(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("Internal error: {msg}");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
}

#[allow(dead_code)]
pub(crate) fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    api_error(StatusCode::NOT_FOUND, msg)
}

/// Relay membership enforcement — single gate for all authenticated entry points.
///
/// Moved here from the deleted `relay_members` module. Called by `media.rs`, `bridge.rs`,
/// `git/transport.rs`, and `audio/handler.rs`.
pub mod relay_members {
    use axum::{http::StatusCode, response::Json};
    use buzz_core::tenant::CommunityId;
    use tracing::{debug, info};

    use crate::state::AppState;

    /// Transport-neutral outcome of a relay-membership check.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum MembershipDecision {
        /// Relay membership enforcement is disabled.
        OpenRelay,
        /// Caller is directly present in `relay_members`.
        Member,
        /// Caller is admitted through a NIP-OA owner that is a relay member.
        ViaOwner(nostr::PublicKey),
        /// Caller is not admitted.
        Denied,
    }

    /// Check relay membership without committing to an HTTP response shape.
    ///
    /// `community` is the server-resolved tenant of the request; membership is
    /// scoped to it so admitting a pubkey to community A never admits it to B.
    /// A NIP-OA credential is usable only when `signed_auth_created_at` came
    /// from the already-verified authentication event carrying that request.
    pub async fn check_relay_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
    ) -> Result<MembershipDecision, String> {
        if !state.config.require_relay_membership {
            return Ok(MembershipDecision::OpenRelay);
        }

        let pubkey_hex = hex::encode(pubkey_bytes);
        let is_member = state
            .db
            .is_relay_member(community, &pubkey_hex)
            .await
            .map_err(|e| format!("relay membership check failed: {e}"))?;
        if is_member {
            return Ok(MembershipDecision::Member);
        }

        if state.config.allow_nip_oa_auth {
            if let Some(tag_json) = auth_tag_header {
                let agent_pubkey = nostr::PublicKey::from_slice(pubkey_bytes)
                    .map_err(|e| format!("invalid agent pubkey for NIP-OA check: {e}"))?;
                let Some(auth_created_at) = signed_auth_created_at else {
                    info!(agent = %pubkey_hex, "NIP-OA auth tag has no verified signed auth timestamp");
                    return Ok(MembershipDecision::Denied);
                };

                match buzz_sdk::nip_oa::verify_auth_tag_for_auth_event(
                    tag_json,
                    &agent_pubkey,
                    auth_created_at,
                ) {
                    Ok(owner_pubkey) => {
                        let owner_hex = owner_pubkey.to_hex();
                        let owner_is_member = state
                            .db
                            .is_relay_member(community, &owner_hex)
                            .await
                            .map_err(|e| format!("relay membership check (owner) failed: {e}"))?;
                        if owner_is_member {
                            debug!(
                                agent = %pubkey_hex,
                                owner = %owner_hex,
                                "NIP-OA membership granted via owner"
                            );
                            return Ok(MembershipDecision::ViaOwner(owner_pubkey));
                        }
                    }
                    Err(e) => {
                        info!(agent = %pubkey_hex, "NIP-OA auth tag invalid: {e}");
                    }
                }
            }
        }

        Ok(MembershipDecision::Denied)
    }

    /// Enforce relay membership for a pubkey, with NIP-OA agent delegation fallback.
    ///
    /// Returns `Ok(Some(owner_pubkey))` when the agent is not a direct member but
    /// its NIP-OA owner *is* — access is granted via delegation.
    ///
    /// On open relays (`require_relay_membership = false`), returns `Ok(None)`
    /// immediately — no membership check is performed. Callers that need NIP-OA
    /// owner extraction on open relays should call [`extract_nip_oa_owner`] directly.
    ///
    /// Returns `Ok(None)` when the caller is a direct member (closed relay) or when
    /// no NIP-OA tag is present/applicable (open relay without auth tag).
    pub async fn enforce_relay_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
    ) -> Result<Option<nostr::PublicKey>, (StatusCode, Json<serde_json::Value>)> {
        match check_relay_membership(
            state,
            community,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
        )
        .await
        {
            Ok(MembershipDecision::OpenRelay) | Ok(MembershipDecision::Member) => Ok(None),
            Ok(MembershipDecision::ViaOwner(owner)) => Ok(Some(owner)),
            Ok(MembershipDecision::Denied) => Err((
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "relay_membership_required",
                    "message": "You must be a relay member to access this relay"
                })),
            )),
            Err(e) => {
                tracing::error!("relay membership check errored: {e}");
                Err(super::internal_error(&e))
            }
        }
    }

    /// Extract NIP-OA owner from an auth tag without membership enforcement.
    ///
    /// Used on open relays (`require_relay_membership = false`) to opportunistically
    /// extract the owner pubkey for agent→owner backfill. The NIP-OA signature is
    /// cryptographically self-proving, so no feature flag is needed. Temporal
    /// conditions are evaluated against `signed_auth_created_at`. Returns
    /// `None` if the tag, timestamp, or conditions are absent or invalid.
    pub fn extract_nip_oa_owner(
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
    ) -> Option<nostr::PublicKey> {
        let tag_json = auth_tag_header?;
        let auth_created_at = signed_auth_created_at?;
        let agent_pubkey = nostr::PublicKey::from_slice(pubkey_bytes).ok()?;
        match buzz_sdk::nip_oa::verify_auth_tag_for_auth_event(
            tag_json,
            &agent_pubkey,
            auth_created_at,
        ) {
            Ok(owner) => Some(owner),
            Err(e) => {
                info!("extract_nip_oa_owner: invalid auth tag: {e}");
                None
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use buzz_sdk::nip_oa::compute_auth_tag;
        use nostr::Keys;

        /// Valid NIP-OA auth tag → returns Some(owner_pubkey).
        #[test]
        fn valid_nip_oa_returns_owner() {
            let owner_keys = Keys::generate();
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let tag_json = compute_auth_tag(&owner_keys, &agent_pubkey, "")
                .expect("compute_auth_tag must succeed");

            let result = extract_nip_oa_owner(
                &agent_pubkey.to_bytes(),
                Some(&tag_json),
                Some(nostr::Timestamp::now().as_secs()),
            );

            assert_eq!(result, Some(owner_keys.public_key()));
        }

        #[test]
        fn nip_oa_time_conditions_use_signed_auth_event_time() {
            let owner_keys = Keys::generate();
            let agent_pubkey = Keys::generate().public_key();

            let expired = compute_auth_tag(&owner_keys, &agent_pubkey, "created_at<200")
                .expect("sign expired credential");
            assert_eq!(
                extract_nip_oa_owner(&agent_pubkey.to_bytes(), Some(&expired), Some(200)),
                None
            );

            let future = compute_auth_tag(&owner_keys, &agent_pubkey, "created_at>200")
                .expect("sign future credential");
            assert_eq!(
                extract_nip_oa_owner(&agent_pubkey.to_bytes(), Some(&future), Some(200)),
                None
            );

            let in_window = compute_auth_tag(
                &owner_keys,
                &agent_pubkey,
                "kind=9&created_at>199&created_at<201",
            )
            .expect("sign in-window credential");
            assert_eq!(
                extract_nip_oa_owner(&agent_pubkey.to_bytes(), Some(&in_window), Some(200)),
                Some(owner_keys.public_key())
            );
            assert_eq!(
                extract_nip_oa_owner(&agent_pubkey.to_bytes(), Some(&in_window), None),
                None,
                "a credential without a verified signed auth timestamp must fail closed"
            );
        }

        /// No auth tag → returns None.
        #[test]
        fn no_auth_tag_returns_none() {
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let result = extract_nip_oa_owner(
                &agent_pubkey.to_bytes(),
                None,
                Some(nostr::Timestamp::now().as_secs()),
            );

            assert_eq!(result, None);
        }

        /// Invalid auth tag → returns None.
        #[test]
        fn invalid_auth_tag_returns_none() {
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let result = extract_nip_oa_owner(
                &agent_pubkey.to_bytes(),
                Some("not valid json"),
                Some(nostr::Timestamp::now().as_secs()),
            );

            assert_eq!(result, None);
        }
    }
}
