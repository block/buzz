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
        OpenRelay,
        /// Caller is directly present in `relay_members`.
        Member,
        /// Caller is directly present in `relay_members` *and* presented a valid
        /// NIP-OA auth tag naming an owner that is also a relay member.
        ///
        /// Distinct from [`Self::ViaOwner`]: membership does not depend on the
        /// owner here, it is already established. The relationship is carried
        /// out of the check so it can still be recorded.
        MemberWithOwner(nostr::PublicKey),
        /// Caller is admitted through a NIP-OA owner that is a relay member.
        ViaOwner(nostr::PublicKey),
        /// Caller is not admitted.
        Denied,
    }

    impl MembershipDecision {
        /// The NIP-OA owner proved during the check, if any.
        ///
        /// Deliberately independent of *why* the caller was admitted: both a
        /// delegated agent and a direct member can prove an owner, and the
        /// relationship is worth recording either way.
        pub fn nip_oa_owner(&self) -> Option<nostr::PublicKey> {
            match self {
                Self::ViaOwner(owner) | Self::MemberWithOwner(owner) => Some(*owner),
                Self::OpenRelay | Self::Member | Self::Denied => None,
            }
        }
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
        if !state.config.require_relay_membership {
            return Ok(MembershipDecision::OpenRelay);
        }

        let pubkey_hex = hex::encode(pubkey_bytes);
        let is_member = state
            .db
            .is_relay_member(community, &pubkey_hex)
            .await
            .map_err(|e| format!("relay membership check failed: {e}"))?;
        // A direct member still has its NIP-OA owner resolved. Membership is
        // already settled, but the agent→owner relationship is not, and this is
        // the only point where the auth tag is examined. Returning plain
        // `Member` here discards a proof the agent did present — so an agent
        // registered as a relay member in its own right never gets an owner
        // recorded, and every owner-gated feature silently fails for it.
        //
        // The visible symptom is kind 24200 observer frames: the relay gates
        // them on `users.agent_owner_pubkey` and rejects all of them, so Buzz
        // Desktop's ACP activity tab stays empty for a working agent while the
        // harness reports `relay observer enabled`. Desktop-managed agents are
        // unaffected because they are minted with an owner rather than added to
        // `relay_members`.
        if is_member {
            let owner =
                resolve_nip_oa_owner(state, community, pubkey_bytes, auth_tag_header, &pubkey_hex)
                    .await?;
            return Ok(match owner {
                Some(owner) => MembershipDecision::MemberWithOwner(owner),
                None => MembershipDecision::Member,
            });
        }

        if let Some(owner_pubkey) =
            resolve_nip_oa_owner(state, community, pubkey_bytes, auth_tag_header, &pubkey_hex)
                .await?
        {
            debug!(
                agent = %pubkey_hex,
                owner = %owner_pubkey.to_hex(),
                "NIP-OA membership granted via owner"
            );
            return Ok(MembershipDecision::ViaOwner(owner_pubkey));
        }

        Ok(MembershipDecision::Denied)
    }

    /// Resolve the NIP-OA owner named by an auth tag.
    ///
    /// Returns `Ok(None)` — never an error — when NIP-OA auth is disabled, no
    /// tag was presented, the tag does not verify, or the named owner is not
    /// itself a relay member. Only a malformed caller pubkey or a failed DB
    /// lookup is an error.
    ///
    /// The owner-is-member requirement is inherited from the delegation path
    /// rather than newly imposed: on a closed relay, recording an owner that
    /// cannot itself connect would hand agent-management authority to a
    /// non-member.
    async fn resolve_nip_oa_owner(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        pubkey_hex: &str,
    ) -> Result<Option<nostr::PublicKey>, String> {
        if !state.config.allow_nip_oa_auth {
            return Ok(None);
        }
        // Costs nothing for ordinary clients: only agents send an auth tag, so
        // the extra owner lookup below never runs for a human's connection.
        let Some(tag_json) = auth_tag_header else {
            return Ok(None);
        };

        let agent_pubkey = nostr::PublicKey::from_slice(pubkey_bytes)
            .map_err(|e| format!("invalid agent pubkey for NIP-OA check: {e}"))?;

        let owner_pubkey = match buzz_sdk::nip_oa::verify_auth_tag(tag_json, &agent_pubkey) {
            Ok(owner_pubkey) => owner_pubkey,
            Err(e) => {
                info!(agent = %pubkey_hex, "NIP-OA auth tag invalid: {e}");
                return Ok(None);
            }
        };

        let owner_is_member = state
            .db
            .is_relay_member(community, &owner_pubkey.to_hex())
            .await
            .map_err(|e| format!("relay membership check (owner) failed: {e}"))?;

        Ok(owner_is_member.then_some(owner_pubkey))
    }

    /// Enforce relay membership for a pubkey, with NIP-OA agent delegation fallback.
    ///
    /// Returns `Ok(Some(owner_pubkey))` whenever a NIP-OA owner was proved,
    /// whether or not membership depended on it: for an agent admitted *by*
    /// delegation, and for a direct member that also carries a valid auth tag.
    /// Callers use it to record the agent→owner relationship, which is needed
    /// in both cases.
    ///
    /// On open relays (`require_relay_membership = false`), returns `Ok(None)`
    /// immediately — no membership check is performed. Callers that need NIP-OA
    /// owner extraction on open relays should call [`extract_nip_oa_owner`] directly.
    ///
    /// Returns `Ok(None)` when no NIP-OA tag is present or applicable.
    pub async fn enforce_relay_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
    ) -> Result<Option<nostr::PublicKey>, (StatusCode, Json<serde_json::Value>)> {
        match check_relay_membership(state, community, pubkey_bytes, auth_tag_header).await {
            Ok(
                decision @ (MembershipDecision::OpenRelay
                | MembershipDecision::Member
                | MembershipDecision::MemberWithOwner(_)
                | MembershipDecision::ViaOwner(_)),
            ) => Ok(decision.nip_oa_owner()),
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

        /// Invalid auth tag → returns None.
        #[test]
        fn invalid_auth_tag_returns_none() {
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let result = extract_nip_oa_owner(&agent_pubkey.to_bytes(), Some("not valid json"));

            assert_eq!(result, None);
        }

        /// A direct member that proved an owner must surface it. This is the
        /// regression: returning a plain `Member` here drops the relationship,
        /// and an agent registered as a relay member in its own right then
        /// never gets `users.agent_owner_pubkey` written — which silently
        /// disables observer frames and every other owner-gated feature.
        #[test]
        fn member_with_owner_surfaces_the_owner() {
            let owner = Keys::generate().public_key();

            assert_eq!(
                MembershipDecision::MemberWithOwner(owner).nip_oa_owner(),
                Some(owner)
            );
        }

        /// Delegation keeps surfacing the owner exactly as before.
        #[test]
        fn via_owner_surfaces_the_owner() {
            let owner = Keys::generate().public_key();

            assert_eq!(
                MembershipDecision::ViaOwner(owner).nip_oa_owner(),
                Some(owner)
            );
        }

        /// Admission without a proved owner surfaces nothing — a member that
        /// sent no auth tag must not acquire one.
        #[test]
        fn admission_without_proof_surfaces_no_owner() {
            assert_eq!(MembershipDecision::Member.nip_oa_owner(), None);
            assert_eq!(MembershipDecision::OpenRelay.nip_oa_owner(), None);
            assert_eq!(MembershipDecision::Denied.nip_oa_owner(), None);
        }
    }
}
