//! HTTP API — media, git, NIP-05, and the Nostr HTTP bridge.

pub mod bridge;
pub mod events;
pub mod git;
pub mod media;
pub mod nip05;

// Re-export imeta helpers used by ingest pipeline.
pub use crate::handlers::imeta::{validate_imeta_tags, verify_imeta_blobs};

// ── Shared helpers (used by media.rs, bridge.rs) ──────────────────────────────

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
    use tracing::{debug, info};

    use crate::state::AppState;

    /// Transport-neutral outcome of a relay-membership check.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum MembershipDecision {
        /// Relay membership enforcement is disabled.
        OpenRelay,
        /// Caller is directly present in `relay_members`.
        Member,
        /// Caller is directly present in `relay_members` with the read-only viewer role.
        Viewer {
            /// Explicit channel allowlist for the viewer.
            channel_ids: Vec<uuid::Uuid>,
        },
        /// Caller is admitted through a NIP-OA owner that is a relay member.
        ViaOwner(nostr::PublicKey),
        /// Caller is admitted through a NIP-OA owner with the read-only viewer role.
        ViaViewerOwner {
            /// The verified NIP-OA owner pubkey.
            owner: nostr::PublicKey,
            /// Explicit channel allowlist inherited from the owner.
            channel_ids: Vec<uuid::Uuid>,
        },
        /// Caller is not admitted.
        Denied,
    }

    /// Check relay membership without committing to an HTTP response shape.
    pub async fn check_relay_membership(
        state: &AppState,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
    ) -> Result<MembershipDecision, String> {
        if !state.config.require_relay_membership {
            return Ok(MembershipDecision::OpenRelay);
        }

        let pubkey_hex = hex::encode(pubkey_bytes);
        let member = state
            .db
            .get_relay_member(&pubkey_hex)
            .await
            .map_err(|e| format!("relay membership check failed: {e}"))?;
        if let Some(member) = member {
            if member.role == "viewer" {
                let channel_ids = state
                    .db
                    .get_relay_member_channel_allowlist(&pubkey_hex)
                    .await
                    .map_err(|e| format!("relay viewer allowlist lookup failed: {e}"))?;
                return Ok(MembershipDecision::Viewer { channel_ids });
            }
            return Ok(MembershipDecision::Member);
        }

        if state.config.allow_nip_oa_auth {
            if let Some(tag_json) = auth_tag_header {
                let agent_pubkey = nostr::PublicKey::from_slice(pubkey_bytes)
                    .map_err(|e| format!("invalid agent pubkey for NIP-OA check: {e}"))?;

                match sprout_sdk::nip_oa::verify_auth_tag(tag_json, &agent_pubkey) {
                    Ok(owner_pubkey) => {
                        let owner_hex = owner_pubkey.to_hex();
                        let owner_member =
                            state.db.get_relay_member(&owner_hex).await.map_err(|e| {
                                format!("relay membership check (owner) failed: {e}")
                            })?;
                        if let Some(owner_member) = owner_member {
                            debug!(
                                agent = %pubkey_hex,
                                owner = %owner_hex,
                                "NIP-OA membership granted via owner"
                            );
                            if owner_member.role == "viewer" {
                                let channel_ids = state
                                    .db
                                    .get_relay_member_channel_allowlist(&owner_hex)
                                    .await
                                    .map_err(|e| {
                                        format!("relay viewer allowlist lookup (owner) failed: {e}")
                                    })?;
                                return Ok(MembershipDecision::ViaViewerOwner {
                                    owner: owner_pubkey,
                                    channel_ids,
                                });
                            }
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

    /// Build the restricted scopes/channel allowlist implied by a membership decision.
    ///
    /// Returns `(agent_owner_pubkey, channel_ids)`. The caller applies read-only
    /// scopes when `channel_ids` is `Some`; `None` means an unrestricted relay
    /// member/open-relay context.
    pub fn relay_access_profile_from_decision(
        decision: MembershipDecision,
    ) -> (Option<nostr::PublicKey>, Option<Vec<uuid::Uuid>>) {
        match decision {
            MembershipDecision::OpenRelay | MembershipDecision::Member => (None, None),
            MembershipDecision::Viewer { channel_ids } => (None, Some(channel_ids)),
            MembershipDecision::ViaOwner(owner) => (Some(owner), None),
            MembershipDecision::ViaViewerOwner { owner, channel_ids } => {
                (Some(owner), Some(channel_ids))
            }
            MembershipDecision::Denied => (None, None),
        }
    }

    /// Extract NIP-OA owner from an auth tag without membership enforcement.
    ///
    /// Used on open relays (`require_relay_membership = false`) to opportunistically
    /// extract the owner pubkey for agent→owner backfill. The NIP-OA signature is
    /// cryptographically self-proving, so no feature flag is needed — if the tag
    /// verifies, the owner relationship is authentic. Returns `None` if the tag
    /// is absent or invalid.
    pub fn extract_nip_oa_owner(
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
    ) -> Option<nostr::PublicKey> {
        let tag_json = auth_tag_header?;
        let agent_pubkey = nostr::PublicKey::from_slice(pubkey_bytes).ok()?;
        match sprout_sdk::nip_oa::verify_auth_tag(tag_json, &agent_pubkey) {
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
        use nostr::Keys;
        use sprout_sdk::nip_oa::compute_auth_tag;

        /// Valid NIP-OA auth tag → returns Some(owner_pubkey).
        #[test]
        fn valid_nip_oa_returns_owner() {
            let owner_keys = Keys::generate();
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let tag_json = compute_auth_tag(&owner_keys, &agent_pubkey, "")
                .expect("compute_auth_tag must succeed");

            let result = extract_nip_oa_owner(&agent_pubkey.to_bytes(), Some(&tag_json));

            assert_eq!(result, Some(owner_keys.public_key()));
        }

        /// No auth tag → returns None.
        #[test]
        fn no_auth_tag_returns_none() {
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let result = extract_nip_oa_owner(&agent_pubkey.to_bytes(), None);

            assert_eq!(result, None);
        }

        /// Invalid auth tag → returns None.
        #[test]
        fn invalid_auth_tag_returns_none() {
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let result = extract_nip_oa_owner(&agent_pubkey.to_bytes(), Some("not valid json"));

            assert_eq!(result, None);
        }
    }
}
