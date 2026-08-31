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
        /// Caller is admitted through a NIP-OA owner that is a relay member.
        ViaOwner(nostr::PublicKey),
        /// Caller is not admitted.
        Denied,
    }

    /// Check relay membership without committing to an HTTP response shape.
    ///
    /// `community` is the server-resolved tenant of the request; membership is
    /// scoped to it so admitting a pubkey to community A never admits it to B.
    ///
    /// `auth_event_created_at` is the `created_at` of the signed authentication
    /// event that carried the NIP-OA tag — the NIP-42 AUTH event on WebSocket,
    /// the NIP-98 request event over HTTP. NIP-AA Step 4 requires the tag's
    /// `created_at<t` / `created_at>t` clauses to be evaluated against exactly
    /// that field, so delegated admission needs it and **`None` denies the
    /// delegated fallback**: without a trusted timestamp there is nothing to
    /// judge the attested window against, and admitting anyway would let an
    /// expired capability back onto a closed relay. Direct membership is
    /// unaffected — it never consults the tag.
    pub async fn check_relay_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        auth_event_created_at: Option<u64>,
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

                // Fail closed: a caller that cannot supply the carrier event's
                // signed `created_at` cannot have the tag's window evaluated, so
                // it does not get delegated admission.
                let Some(created_at) = auth_event_created_at else {
                    info!(
                        agent = %pubkey_hex,
                        "NIP-OA delegation denied: no signed timestamp to evaluate the tag window against"
                    );
                    return Ok(MembershipDecision::Denied);
                };

                match buzz_sdk::nip_oa::verify_auth_tag_at(tag_json, &agent_pubkey, created_at) {
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
    /// immediately — no membership check is performed.
    ///
    /// Returns `Ok(None)` when the caller is a direct member (closed relay) or when
    /// no NIP-OA tag is present/applicable (open relay without auth tag). `Ok(None)`
    /// therefore means "admitted on its own", **not** "has no owner": callers that
    /// record ownership must pass the result through [`resolve_nip_oa_owner`], which
    /// recovers the owner from the presented tag in exactly those cases.
    ///
    /// `auth_event_created_at` carries the signed timestamp the NIP-OA window is
    /// judged against; see [`check_relay_membership`]. `None` denies the
    /// delegated fallback rather than skipping the check.
    pub async fn enforce_relay_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        auth_event_created_at: Option<u64>,
    ) -> Result<Option<nostr::PublicKey>, (StatusCode, Json<serde_json::Value>)> {
        match check_relay_membership(
            state,
            community,
            pubkey_bytes,
            auth_tag_header,
            auth_event_created_at,
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

    /// Identify the NIP-OA owner named by an auth tag, **without** enforcing the
    /// tag's time bounds.
    ///
    /// This answers "who does this tag name as the owner?", which is the right
    /// question for *restriction* paths: the ban cascades in `handlers::auth`
    /// and `api::git::transport` deny an agent whose owner is banned, and an
    /// expired attestation must not become an escape hatch from that. Widening
    /// who gets denied is safe; widening who gets trusted is not.
    ///
    /// Anything that *grants* — recording ownership, admitting a session,
    /// choosing a rate class — must use [`extract_nip_oa_owner_at`] instead, so
    /// an expired credential cannot confer authority. Returns `None` if the tag
    /// is absent or fails signature verification.
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

    /// Verify a NIP-OA auth tag *and* its time bounds, for paths that grant
    /// authority from the result.
    ///
    /// `auth_event_created_at` is the `created_at` of the signed authentication
    /// event that carried the tag — the NIP-42 AUTH event on WebSocket, the
    /// NIP-98 request event over HTTP. The bound is part of what the owner
    /// authorized, so it is judged against the same signed artifact the
    /// attestation travelled with rather than against wall clock.
    ///
    /// Returns `None` if the tag is absent, fails signature verification, or is
    /// outside its attested window.
    pub fn extract_nip_oa_owner_at(
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        auth_event_created_at: u64,
    ) -> Option<nostr::PublicKey> {
        let tag_json = auth_tag_header?;
        let agent_pubkey = nostr::PublicKey::from_slice(pubkey_bytes).ok()?;
        match buzz_sdk::nip_oa::verify_auth_tag_at(tag_json, &agent_pubkey, auth_event_created_at) {
            Ok(owner) => Some(owner),
            Err(e) => {
                info!("extract_nip_oa_owner_at: auth tag not usable: {e}");
                None
            }
        }
    }

    /// Resolve the NIP-OA owner to materialize for a caller that has already
    /// passed the membership gate.
    ///
    /// `gate_owner` is what [`enforce_relay_membership`] returned: `Some` only
    /// when membership was granted *through* the owner, in which case that owner
    /// has already been proven to be a relay member and is kept as-is. Every
    /// other admitted caller — a direct relay member on a closed relay, or
    /// anyone on an open relay — arrives here with `None`, and the `auth` tag
    /// they presented is verified here instead.
    ///
    /// Gating this on `require_relay_membership` inverted the deployment
    /// posture — the stricter relay was the only one that never recorded
    /// ownership, so `owner_only` policies, observer-frame authorization and the
    /// agent rate class all silently degraded for agents enrolled as members
    /// (#4223, #4937).
    ///
    /// # Why a self-presented tag is not sufficient on its own
    ///
    /// The attestation proves an owner key signed for this agent. It does *not*
    /// prove that key is anyone this relay trusts, and the resolved owner is not
    /// inert metadata: `materialize_nip_oa_owner` creates a user row for it, and
    /// `connection.rs` derives the agent rate class from the session carrying
    /// it. Believing an arbitrary key would let any direct member mint a
    /// throwaway keypair, attest itself, and take the agent message quota —
    /// while `set_agent_owner` is first-write-wins, so that bogus mapping would
    /// then be permanent and the real owner refused. On a closed relay the
    /// claimed owner must therefore be a relay member, exactly as the
    /// `ViaOwner` branch of [`check_relay_membership`] already requires.
    ///
    /// Open relays are unchanged: with no membership boundary to honour there is
    /// nothing to check the owner against, and extraction stays unconditional.
    ///
    /// `allow_nip_oa_auth` is deliberately not consulted — its own doc comment
    /// scopes it to whether NIP-OA may *grant membership*, which this never
    /// does. The boundary enforced here is owner membership, not that flag.
    ///
    /// `auth_event_created_at` is the `created_at` of the signed authentication
    /// event that carried the tag; the attestation's own time bounds are
    /// enforced against it, so an expired or not-yet-valid credential resolves
    /// to `None`.
    pub async fn resolve_nip_oa_owner(
        state: &AppState,
        community: CommunityId,
        gate_owner: Option<nostr::PublicKey>,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        auth_event_created_at: u64,
    ) -> Option<nostr::PublicKey> {
        // Re-verified here even when the gate already resolved an owner. The two
        // checks judge the same window against the same signed timestamp, so
        // this is belt-and-braces rather than a second opinion — it keeps
        // materialization correct on its own terms if the gate is ever changed
        // or called differently.
        let owner = extract_nip_oa_owner_at(pubkey_bytes, auth_tag_header, auth_event_created_at)?;

        if let Some(gate_owner) = gate_owner {
            // Membership came through this owner, so it is already a proven
            // relay member and needs no second membership read. The keys are
            // derived from the same tag and must agree; if they somehow do not,
            // fail closed rather than pick one.
            if gate_owner != owner {
                tracing::error!(
                    "resolve_nip_oa_owner: gate owner and re-verified owner disagree; \
                     refusing to record ownership"
                );
                return None;
            }
            return Some(gate_owner);
        }

        if !state.config.require_relay_membership {
            return Some(owner);
        }

        let owner_hex = owner.to_hex();
        match state.db.is_relay_member(community, &owner_hex).await {
            Ok(true) => Some(owner),
            Ok(false) => {
                info!(
                    owner = %owner_hex,
                    "resolve_nip_oa_owner: claimed owner is not a relay member; \
                     refusing to record ownership on a closed relay"
                );
                None
            }
            Err(e) => {
                // Fail closed: without a membership answer the owner cannot be
                // trusted, and materializing it is not reversible.
                tracing::error!(
                    owner = %owner_hex,
                    "resolve_nip_oa_owner: owner membership check failed: {e}"
                );
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

        /// A caller the gate admitted on its own — a direct relay member on a
        /// closed relay — still has its verified owner recovered from the tag,
        /// instead of the attestation being dropped. This is the identification
        /// half of the fix; whether that owner is *trusted* is decided by
        /// `resolve_nip_oa_owner`, which needs a relay and is covered by the
        /// Postgres-backed tests over the real HTTP and NIP-42 paths.
        #[test]
        fn extract_at_recovers_owner_for_a_direct_member() {
            let owner_keys = Keys::generate();
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let tag_json = compute_auth_tag(&owner_keys, &agent_pubkey, "")
                .expect("compute_auth_tag must succeed");

            let result = extract_nip_oa_owner_at(&agent_pubkey.to_bytes(), Some(&tag_json), 1_000);

            assert_eq!(result, Some(owner_keys.public_key()));
        }

        /// A direct member that presents no tag stays ownerless — membership
        /// alone never invents an owner.
        #[test]
        fn extract_at_without_a_tag_returns_none() {
            let agent_keys = Keys::generate();

            let result = extract_nip_oa_owner_at(&agent_keys.public_key().to_bytes(), None, 1_000);

            assert_eq!(result, None);
        }

        /// A tag that attests a *different* agent is not evidence about this
        /// one: `verify_auth_tag_at` binds the attestation to the signing
        /// pubkey, so an intercepted tag can't be replayed onto another agent.
        #[test]
        fn extract_at_rejects_a_tag_minted_for_another_agent() {
            let owner_keys = Keys::generate();
            let attested_agent_keys = Keys::generate();
            let impostor_keys = Keys::generate();

            let tag_json = compute_auth_tag(&owner_keys, &attested_agent_keys.public_key(), "")
                .expect("compute_auth_tag must succeed");

            let result = extract_nip_oa_owner_at(
                &impostor_keys.public_key().to_bytes(),
                Some(&tag_json),
                1_000,
            );

            assert_eq!(result, None);
        }

        /// An expired or not-yet-valid attestation resolves to no owner, so it
        /// cannot be materialized, seed an auth context, or lift the rate class.
        /// The signature-only entry point still accepts both — that difference
        /// is the whole reason the two functions exist.
        #[test]
        fn extract_at_refuses_credentials_outside_their_window() {
            let owner_keys = Keys::generate();
            let agent_keys = Keys::generate();
            let agent_pubkey = agent_keys.public_key();

            let expired = compute_auth_tag(&owner_keys, &agent_pubkey, "created_at<1000")
                .expect("compute_auth_tag must succeed");
            let not_yet = compute_auth_tag(&owner_keys, &agent_pubkey, "created_at>9000")
                .expect("compute_auth_tag must succeed");

            let agent_bytes = agent_pubkey.to_bytes();
            let owner = Some(owner_keys.public_key());

            assert_eq!(
                extract_nip_oa_owner_at(&agent_bytes, Some(&expired), 999),
                owner
            );
            assert_eq!(
                extract_nip_oa_owner_at(&agent_bytes, Some(&expired), 1000),
                None
            );
            assert_eq!(
                extract_nip_oa_owner_at(&agent_bytes, Some(&not_yet), 9001),
                owner
            );
            assert_eq!(
                extract_nip_oa_owner_at(&agent_bytes, Some(&not_yet), 9000),
                None
            );

            // The identification-only path is deliberately unaffected: ban
            // cascades must still recognise the owner of an expired tag.
            assert_eq!(extract_nip_oa_owner(&agent_bytes, Some(&expired)), owner);
        }
    }
}
