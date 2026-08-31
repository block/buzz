//! HTTP API — media, git, NIP-05, and the Nostr HTTP bridge.

pub mod admin;
pub mod bridge;
pub mod events;
pub mod gifs;
pub mod git;
pub mod invites;
pub mod media;
pub mod mesh_demo;
pub mod nip05;
pub mod operator;
pub mod workflows;

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
    use buzz_core::{tenant::CommunityId, TenantContext};
    use tracing::{debug, info};

    use crate::state::AppState;

    /// Transport-neutral outcome of a relay-membership check.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum MembershipDecision {
        /// Relay membership enforcement is disabled.
        ///
        /// Carries a verified NIP-OA owner when the caller presented one. The
        /// owner relationship is independent of admission: it is proven by the
        /// attestation's own signature, not by how the caller got in.
        OpenRelay(Option<nostr::PublicKey>),
        /// Caller is directly present in `relay_members`. Carries a verified
        /// NIP-OA owner when the caller presented one, for the same reason.
        Member(Option<nostr::PublicKey>),
        /// Caller is admitted through a NIP-OA owner that is a relay member.
        ViaOwner(nostr::PublicKey),
        /// Caller is not admitted.
        Denied,
    }

    /// Check relay membership without committing to an HTTP response shape.
    ///
    /// `community` is the server-resolved tenant of the request; membership is
    /// scoped to it so admitting a pubkey to community A never admits it to B.
    pub async fn check_relay_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
    ) -> Result<MembershipDecision, String> {
        // Verify the attestation up front, independent of admission.
        //
        // Membership answers "may this caller connect"; the attestation answers
        // "who owns this agent". Deriving the second from the first is what
        // regressed: a direct member was admitted without the delegation being
        // consulted, so a perfectly valid owner relationship was discarded and
        // `users.agent_owner_pubkey` stayed NULL.
        let verified_owner = extract_nip_oa_owner(pubkey_bytes, auth_tag_header);

        if !state.config.require_relay_membership {
            return Ok(MembershipDecision::OpenRelay(verified_owner));
        }

        let pubkey_hex = hex::encode(pubkey_bytes);
        let is_member = state
            .db
            .is_relay_member(community, &pubkey_hex)
            .await
            .map_err(|e| format!("relay membership check failed: {e}"))?;
        if is_member {
            return Ok(MembershipDecision::Member(verified_owner));
        }

        if state.config.allow_nip_oa_auth {
            if let Some(tag_json) = auth_tag_header {
                let agent_pubkey = nostr::PublicKey::from_slice(pubkey_bytes)
                    .map_err(|e| format!("invalid agent pubkey for NIP-OA check: {e}"))?;

                match buzz_sdk::nip_oa::verify_auth_tag(tag_json, &agent_pubkey) {
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
    /// Returns `Ok(Some(owner_pubkey))` whenever the caller is admitted AND
    /// presented a verifying NIP-OA attestation — whether it was admitted as a
    /// direct relay member, through owner delegation, or on an open relay.
    ///
    /// The owner relationship is deliberately independent of the admission path:
    /// it is proven by the attestation's own signature. Reporting it only for
    /// delegated admissions left `users.agent_owner_pubkey` NULL for every
    /// directly-enrolled agent, which is what `is_agent`, owner-managed agent
    /// lists and `channel_add_policy = "owner_only"` all derive from.
    ///
    /// Returns `Ok(None)` when the caller presented no attestation, or one that
    /// does not verify against its pubkey.
    pub async fn enforce_relay_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
    ) -> Result<Option<nostr::PublicKey>, (StatusCode, Json<serde_json::Value>)> {
        match check_relay_membership(state, community, pubkey_bytes, auth_tag_header).await {
            Ok(
                decision @ (MembershipDecision::OpenRelay(_)
                | MembershipDecision::Member(_)
                | MembershipDecision::ViaOwner(_)),
            ) => Ok(owner_from_decision(&decision)),
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

    /// The owner relationship an admitted decision proves, if any.
    ///
    /// Split out from `enforce_relay_membership` so the behaviour that regressed
    /// is testable without a database. Every admitted variant reports its
    /// verified owner; only `Denied` has none, because a rejected caller proves
    /// nothing. The bug was returning `None` for `Member`, which discarded a
    /// valid attestation purely because admission had not needed it.
    pub(crate) fn owner_from_decision(decision: &MembershipDecision) -> Option<nostr::PublicKey> {
        match decision {
            MembershipDecision::OpenRelay(owner) | MembershipDecision::Member(owner) => *owner,
            MembershipDecision::ViaOwner(owner) => Some(*owner),
            MembershipDecision::Denied => None,
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
        match buzz_sdk::nip_oa::verify_auth_tag(tag_json, &agent_pubkey) {
            Ok(owner) => Some(owner),
            Err(e) => {
                info!("extract_nip_oa_owner: invalid auth tag: {e}");
                None
            }
        }
    }

    /// Persist a cryptographically verified NIP-OA agent→owner relationship.
    ///
    /// Both principals are ensured first because `agent_owner_pubkey` has a
    /// community-scoped foreign key. The mapping is first-write-wins; an
    /// existing mapping is accepted only when it names the same owner.
    pub async fn materialize_nip_oa_owner(
        state: &AppState,
        tenant: &TenantContext,
        agent: &nostr::PublicKey,
        owner: &nostr::PublicKey,
    ) -> bool {
        for (role, pubkey) in [("agent", agent), ("owner", owner)] {
            match state
                .db
                .ensure_user(tenant.community(), pubkey.as_bytes())
                .await
            {
                Ok(true) => {
                    metrics::counter!(
                        "buzz_users_created_total",
                        "community" => tenant.host().to_owned()
                    )
                    .increment(1);
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(%role, error = %e, "ensure_user failed during NIP-OA backfill");
                    return false;
                }
            }
        }

        let materialized = match state
            .db
            .set_agent_owner(tenant.community(), agent.as_bytes(), owner.as_bytes())
            .await
        {
            Ok(true) => true,
            Ok(false) => state
                .db
                .is_agent_owner(tenant.community(), agent.as_bytes(), owner.as_bytes())
                .await
                .unwrap_or(false),
            Err(e) => {
                tracing::warn!(error = %e, "failed to backfill agent_owner_pubkey");
                false
            }
        };

        if materialized {
            state
                .author_type_cache
                .insert((tenant.community(), agent.to_bytes().to_vec()), true);
            state.observer_owner_cache.insert(
                (
                    tenant.community(),
                    agent.to_bytes().to_vec(),
                    owner.to_bytes().to_vec(),
                ),
                true,
            );
        }
        materialized
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

        /// A direct relay member that presented a valid attestation must report
        /// its owner.
        ///
        /// This is the regression. `Member` previously carried no owner and the
        /// mapping returned `None` for it, so on a closed relay a
        /// directly-enrolled agent never had `users.agent_owner_pubkey`
        /// materialized — leaving it rate-limited as a human, absent from
        /// owner-managed agent lists, and unaddable to channels under its own
        /// `channel_add_policy = "owner_only"`.
        #[test]
        fn direct_member_reports_its_verified_owner() {
            let owner = Keys::generate().public_key();

            assert_eq!(
                owner_from_decision(&MembershipDecision::Member(Some(owner))),
                Some(owner),
                "admission path must not decide whether an owner is reported"
            );
        }

        /// The same owner is reported no matter which admitted path produced it.
        #[test]
        fn every_admitted_path_reports_the_same_owner() {
            let owner = Keys::generate().public_key();

            for decision in [
                MembershipDecision::OpenRelay(Some(owner)),
                MembershipDecision::Member(Some(owner)),
                MembershipDecision::ViaOwner(owner),
            ] {
                assert_eq!(
                    owner_from_decision(&decision),
                    Some(owner),
                    "{decision:?} should report the verified owner"
                );
            }
        }

        /// An admitted caller that presented no attestation has no owner, and a
        /// denied caller proves nothing regardless.
        #[test]
        fn no_attestation_or_denied_reports_no_owner() {
            for decision in [
                MembershipDecision::OpenRelay(None),
                MembershipDecision::Member(None),
                MembershipDecision::Denied,
            ] {
                assert_eq!(owner_from_decision(&decision), None, "{decision:?}");
            }
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
