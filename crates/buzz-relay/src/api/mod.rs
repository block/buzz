//! HTTP API — media, git, NIP-05, and the Nostr HTTP bridge.

pub mod admin;
pub mod bridge;
pub mod buzz_v1;
pub mod events;
pub mod gifs;
pub mod git;
pub mod invites;
pub mod media;
pub mod mesh_demo;
pub mod nip05;
pub mod nip_fi;
pub mod operator;
pub mod profiles;
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

/// Stable client-visible body for a read cancelled by its server-side
/// statement deadline. Clients match this string to skip retrying: a retry
/// would re-run the same expensive query.
pub(crate) const QUERY_TIMED_OUT: &str = "query timed out";

/// Map a DB read failure: a statement-deadline cancel becomes a distinct
/// 503 `query timed out`; anything else stays a generic 500.
pub(crate) fn db_read_error(
    context: &str,
    e: &buzz_db::DbError,
) -> (StatusCode, Json<serde_json::Value>) {
    if e.is_statement_cancelled() {
        tracing::warn!("{context}: {e}");
        return api_error(StatusCode::SERVICE_UNAVAILABLE, QUERY_TIMED_OUT);
    }
    internal_error(&format!("{context}: {e}"))
}

#[allow(dead_code)]
pub(crate) fn not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    api_error(StatusCode::NOT_FOUND, msg)
}

/// Parse a raw query string into `T` after NIP-FI/NIP-98 admission has
/// already succeeded.
///
/// - Absent or empty query → `Ok(T::default())` (no params is valid).
/// - Non-empty but malformed → `Err(400 bad request)`.
///
/// **Do not use `.ok().unwrap_or_default()` here.**  That pattern silently
/// discards malformed input and changes query semantics — for example,
/// `?status=open&limit=abc` would drop the valid `status=` field together
/// with the bad `limit=`, broadening the query to all statuses.  Post-
/// admission a parse failure is the caller's error, not an auth failure.
/// [FI-TRACE-HTTP-INGRESS]
pub(crate) fn parse_query_or_400<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&str>,
) -> Result<T, (StatusCode, Json<serde_json::Value>)> {
    match raw {
        None | Some("") => Ok(T::default()),
        Some(q) => serde_urlencoded::from_str(q)
            .map_err(|e| api_error(StatusCode::BAD_REQUEST, &format!("invalid query: {e}"))),
    }
}

/// Relay membership enforcement — single gate for all authenticated entry points.
///
/// Moved here from the deleted `relay_members` module. Called by `media.rs`, `bridge.rs`,
/// `git/transport.rs`, and `audio/handler.rs`.
pub mod relay_members {
    use axum::{
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Json, Response},
    };
    use buzz_auth::DenialClass;
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

    /// An HTTP relay-membership refusal.
    #[derive(Debug)]
    pub enum MembershipRefusal {
        /// The relay's own JSON refusal.
        Api((StatusCode, Json<serde_json::Value>)),
        /// NIP-FI's fixed denial. A request carrying a verified agent token
        /// gets only this, so the response never shows which step refused it
        /// or whether the agent already has a stored owner.
        NipFi(DenialClass),
    }

    impl MembershipRefusal {
        /// The response status.
        pub fn status(&self) -> StatusCode {
            match self {
                Self::Api((status, _)) => *status,
                Self::NipFi(class) => StatusCode::from_u16(class.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            }
        }
    }

    impl From<(StatusCode, Json<serde_json::Value>)> for MembershipRefusal {
        fn from(refusal: (StatusCode, Json<serde_json::Value>)) -> Self {
            Self::Api(refusal)
        }
    }

    impl IntoResponse for MembershipRefusal {
        fn into_response(self) -> Response {
            match self {
                Self::Api(refusal) => refusal.into_response(),
                Self::NipFi(class) => crate::nip_fi_core::http_denial(class),
            }
        }
    }

    /// Return the sole NIP-OA credential header, if one was supplied.
    ///
    /// Repeated security-sensitive headers are ambiguous across HTTP stacks,
    /// so they are treated as no credential instead of silently selecting one.
    pub fn extract_auth_tag_header(headers: &HeaderMap) -> Option<&str> {
        let mut values = headers.get_all("x-auth-tag").iter();
        let (Some(value), None) = (values.next(), values.next()) else {
            return None;
        };
        value.to_str().ok()
    }

    /// Check relay membership without committing to an HTTP response shape.
    ///
    /// `community` is the server-resolved tenant of the request; membership is
    /// scoped to it so admitting a pubkey to community A never admits it to B.
    /// A NIP-OA credential is usable only when `signed_auth_created_at` came
    /// from the already-verified authentication event carrying that request.
    /// `token_owner` is the owner named by the request's verified NIP-FI agent
    /// assertion; when present it is the only owner considered.
    pub async fn check_relay_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
    ) -> Result<MembershipDecision, String> {
        check_membership(
            state,
            community,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
            false,
        )
        .await
    }

    /// [`check_relay_membership`] reading principal and owner membership from
    /// the writer. The final admission fence uses it: a removal whose
    /// disconnect already ran must not be undone by a stale replica row.
    pub async fn check_relay_membership_authoritative(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
    ) -> Result<MembershipDecision, String> {
        check_membership(
            state,
            community,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
            true,
        )
        .await
    }

    async fn read_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_hex: &str,
        writer: bool,
    ) -> buzz_db::Result<bool> {
        if writer {
            state.db.is_relay_member_writer(community, pubkey_hex).await
        } else {
            state.db.is_relay_member(community, pubkey_hex).await
        }
    }

    async fn check_membership(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
        writer: bool,
    ) -> Result<MembershipDecision, String> {
        if !state.config.require_relay_membership {
            return Ok(MembershipDecision::OpenRelay);
        }

        let pubkey_hex = hex::encode(pubkey_bytes);
        let is_member = read_membership(state, community, &pubkey_hex, writer)
            .await
            .map_err(|e| format!("relay membership check failed: {e}"))?;
        if is_member {
            return Ok(MembershipDecision::Member);
        }

        // A verified agent assertion's issuer already passed this community's
        // NIP-FI binding check, so its owner's membership admits the agent
        // without NIP-OA.
        if let Some(owner) = token_owner {
            let owner_is_member = read_membership(state, community, &owner.to_hex(), writer)
                .await
                .map_err(|e| format!("relay membership check (token owner) failed: {e}"))?;
            return Ok(if owner_is_member {
                MembershipDecision::ViaOwner(owner)
            } else {
                MembershipDecision::Denied
            });
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
                        let owner_is_member = read_membership(state, community, &owner_hex, writer)
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
        tenant: &TenantContext,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
    ) -> Result<Option<nostr::PublicKey>, MembershipRefusal> {
        enforce_membership(
            state,
            tenant,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
            false,
        )
        .await
    }

    /// [`enforce_relay_membership`] for HTTP typing pulses: the ban check
    /// reads through the 30-second ephemeral-path restriction cache, the same
    /// staleness bound the pulse's own write-restriction check already has.
    /// Every other HTTP request reads ban state fresh.
    pub async fn enforce_relay_membership_ephemeral(
        state: &AppState,
        tenant: &TenantContext,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
    ) -> Result<Option<nostr::PublicKey>, MembershipRefusal> {
        enforce_membership(
            state,
            tenant,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
            true,
        )
        .await
    }

    /// With a verified agent token, every refusal is NIP-FI's fixed denial.
    async fn enforce_membership(
        state: &AppState,
        tenant: &TenantContext,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
        cached_ban: bool,
    ) -> Result<Option<nostr::PublicKey>, MembershipRefusal> {
        admit_member(
            state,
            tenant,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
            cached_ban,
        )
        .await
        .map_err(|refusal| match token_owner {
            None => MembershipRefusal::Api(refusal),
            Some(_) if refusal.0.is_server_error() => {
                MembershipRefusal::NipFi(DenialClass::AuthorizationUnavailable)
            }
            Some(_) => MembershipRefusal::NipFi(DenialClass::AuthorizationDenied),
        })
    }

    async fn admit_member(
        state: &AppState,
        tenant: &TenantContext,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
        cached_ban: bool,
    ) -> Result<Option<nostr::PublicKey>, (StatusCode, Json<serde_json::Value>)> {
        let community = tenant.community();
        let owner = match check_relay_membership(
            state,
            community,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
        )
        .await
        {
            Ok(MembershipDecision::OpenRelay) | Ok(MembershipDecision::Member) => None,
            Ok(MembershipDecision::ViaOwner(owner)) => Some(owner),
            Ok(MembershipDecision::Denied) => {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "relay_membership_required",
                        "message": "You must be a relay member to access this relay"
                    })),
                ))
            }
            Err(e) => {
                tracing::error!("relay membership check errored: {e}");
                return Err(super::internal_error(&e));
            }
        };
        deny_banned(
            state,
            community,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
            cached_ban,
        )
        .await?;
        if let Some(token_owner) = token_owner {
            let Ok(agent) = nostr::PublicKey::from_slice(pubkey_bytes) else {
                return Err(super::internal_error("invalid pubkey for owner link"));
            };
            match link_agent_owner(state, tenant, &agent, &token_owner).await {
                OwnerLink::Linked => {}
                OwnerLink::Conflict => {
                    return Err(super::api_error(StatusCode::FORBIDDEN, OWNER_MISMATCH))
                }
                OwnerLink::Unavailable => {
                    return Err(super::api_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        crate::handlers::auth::OWNER_LINK_ERROR,
                    ))
                }
            }
        }
        Ok(owner)
    }

    /// Refuse a community-banned principal (own ban or its agent owner's) on
    /// HTTP. Bans only: a timeout blocks writes, and HTTP writes reach the
    /// ingest gate, while reads stay allowed. Fails closed with 503.
    async fn deny_banned(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
        cached: bool,
    ) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
        let Ok(pubkey) = nostr::PublicKey::from_slice(pubkey_bytes) else {
            return Err(super::internal_error("invalid pubkey for ban check"));
        };
        match crate::handlers::auth::community_ban_outcome(
            state,
            community,
            pubkey,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
            cached,
        )
        .await
        {
            crate::handlers::auth::BanOutcome::Clear => Ok(()),
            crate::handlers::auth::BanOutcome::Banned => Err(super::api_error(
                StatusCode::FORBIDDEN,
                "blocked: you are banned from this community",
            )),
            crate::handlers::auth::BanOutcome::DbError => Err(super::api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "error: internal error checking restriction state",
            )),
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

    /// Persist a cryptographically verified NIP-OA agent→owner relationship.
    ///
    /// Both principals are ensured first because `agent_owner_pubkey` has a
    /// community-scoped foreign key. The mapping is first-write-wins; an
    /// existing mapping is accepted only when it names the same owner.
    ///
    /// A mapping already confirmed in `observer_owner_cache` returns early:
    /// `agent_owner_pubkey` is set once and never cleared, so the writes
    /// below would all be no-ops. HTTP agents call this on every request.
    pub async fn materialize_nip_oa_owner(
        state: &AppState,
        tenant: &TenantContext,
        agent: &nostr::PublicKey,
        owner: &nostr::PublicKey,
    ) -> bool {
        link_agent_owner(state, tenant, agent, owner).await == OwnerLink::Linked
    }

    /// Refusal when a verified agent assertion names an owner other than the
    /// one already stored for the agent (NIP-FI agent keys).
    pub(crate) const OWNER_MISMATCH: &str = "restricted: agent owner mismatch";

    /// Result of recording an agent's owner.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum OwnerLink {
        /// The stored owner is now `owner`.
        Linked,
        /// The agent already has a different stored owner.
        Conflict,
        /// The link could not be read or written.
        Unavailable,
    }

    /// The agent's stored owner compared with `owner`, or `None` when the
    /// agent has no stored owner yet. Shadow and enforce both apply the
    /// owner-conflict rule through this read; [`link_agent_owner`] writes only
    /// when it returns `None`.
    pub(crate) async fn stored_owner_link(
        state: &AppState,
        community: CommunityId,
        agent: &nostr::PublicKey,
        owner: &nostr::PublicKey,
    ) -> Option<OwnerLink> {
        match state
            .db
            .get_agent_channel_policy(community, agent.as_bytes())
            .await
        {
            Ok(Some((_, Some(stored)))) if stored.as_slice() == owner.as_bytes() => {
                Some(OwnerLink::Linked)
            }
            Ok(Some((_, Some(_)))) => Some(OwnerLink::Conflict),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(error = %e, "failed to read agent_owner_pubkey");
                Some(OwnerLink::Unavailable)
            }
        }
    }

    /// [`materialize_nip_oa_owner`], distinguishing a different stored owner
    /// from a failed read or write.
    pub async fn link_agent_owner(
        state: &AppState,
        tenant: &TenantContext,
        agent: &nostr::PublicKey,
        owner: &nostr::PublicKey,
    ) -> OwnerLink {
        let owner_key = (
            tenant.community(),
            agent.to_bytes().to_vec(),
            owner.to_bytes().to_vec(),
        );
        if state.observer_owner_cache.get(&owner_key) == Some(true) {
            return OwnerLink::Linked;
        }
        let link = match stored_owner_link(state, tenant.community(), agent, owner).await {
            Some(link) => link,
            None => write_agent_owner(state, tenant, agent, owner).await,
        };
        if link == OwnerLink::Linked {
            state
                .author_type_cache
                .insert((tenant.community(), agent.to_bytes().to_vec()), true);
            state.observer_owner_cache.insert(owner_key, true);
        }
        link
    }

    /// Record `owner` for an agent that had none when read. The write is
    /// first-write-wins, so a concurrent writer that got there first is
    /// judged by the same read.
    async fn write_agent_owner(
        state: &AppState,
        tenant: &TenantContext,
        agent: &nostr::PublicKey,
        owner: &nostr::PublicKey,
    ) -> OwnerLink {
        for (role, pubkey) in [("agent", agent), ("owner", owner)] {
            match state
                .db
                .ensure_user_for_authorization(tenant.community(), pubkey.as_bytes())
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
                    return OwnerLink::Unavailable;
                }
            }
        }
        match state
            .db
            .set_agent_owner_for_authorization(
                tenant.community(),
                agent.as_bytes(),
                owner.as_bytes(),
            )
            .await
        {
            Ok(true) => {
                // The owner was just recorded. Sockets this agent opened
                // without it would only be found by an owner-to-agent lookup
                // at revoke time; make them reconnect with the owner attached.
                state.disconnect_unowned_agent_clusterwide(tenant, &agent.to_bytes());
                OwnerLink::Linked
            }
            // Another writer stored an owner first.
            Ok(false) => stored_owner_link(state, tenant.community(), agent, owner)
                .await
                .unwrap_or(OwnerLink::Unavailable),
            Err(e) => {
                tracing::warn!(error = %e, "failed to backfill agent_owner_pubkey");
                OwnerLink::Unavailable
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use axum::http::{HeaderMap, HeaderValue};
        use buzz_sdk::nip_oa::compute_auth_tag;
        use nostr::Keys;

        #[test]
        fn auth_tag_header_must_be_unique() {
            let mut headers = HeaderMap::new();
            assert_eq!(extract_auth_tag_header(&headers), None);

            headers.insert("x-auth-tag", HeaderValue::from_static("credential-one"));
            assert_eq!(extract_auth_tag_header(&headers), Some("credential-one"));

            headers.append("x-auth-tag", HeaderValue::from_static("credential-two"));
            assert_eq!(extract_auth_tag_header(&headers), None);
        }

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

#[cfg(test)]
mod db_read_error_tests {
    use super::*;

    #[test]
    fn non_cancel_db_error_stays_generic_500() {
        let (status, body) = db_read_error("ctx", &buzz_db::DbError::NotFound("x".into()));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body.0["error"], "internal server error");
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn statement_timeout_maps_to_distinct_503() {
        let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB");
        let mut tx = pool.begin().await.expect("begin");
        sqlx::query("SET LOCAL statement_timeout = '10ms'")
            .execute(&mut *tx)
            .await
            .expect("set timeout");
        let err: buzz_db::DbError = sqlx::query("SELECT pg_sleep(1)")
            .execute(&mut *tx)
            .await
            .expect_err("statement must be cancelled")
            .into();
        assert!(err.is_statement_cancelled());

        let (status, body) = db_read_error("ctx", &err);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["error"], QUERY_TIMED_OUT);
    }
}

// ── parse_query_or_400 regression tests ──────────────────────────────────────

#[cfg(test)]
mod parse_query_tests {
    use super::parse_query_or_400;
    use serde::Deserialize;

    /// Mirror of `ModerationReadQuery` — independent so this module compiles
    /// without pulling in handler dependencies.
    #[derive(Debug, Deserialize, Default, PartialEq)]
    struct QueryFixture {
        status: Option<String>,
        limit: Option<i64>,
    }

    /// Absent query → Default.  No params is always valid.
    #[test]
    fn absent_query_gives_default() {
        let result: Result<QueryFixture, _> = parse_query_or_400(None);
        assert_eq!(result.unwrap(), QueryFixture::default());
    }

    /// Empty string → Default.  Same as absent.
    #[test]
    fn empty_query_gives_default() {
        let result: Result<QueryFixture, _> = parse_query_or_400(Some(""));
        assert_eq!(result.unwrap(), QueryFixture::default());
    }

    /// Well-formed query → parsed correctly.
    #[test]
    fn valid_query_parses_correctly() {
        let result: Result<QueryFixture, _> = parse_query_or_400(Some("status=open&limit=50"));
        let q = result.unwrap();
        assert_eq!(q.status.as_deref(), Some("open"));
        assert_eq!(q.limit, Some(50));
    }

    /// Malformed limit → 400 error, NOT default.
    ///
    /// Regression for the `.ok().unwrap_or_default()` bug: the old code would
    /// silently discard ALL fields on any parse error, so `status=open&limit=abc`
    /// would return `QueryFixture::default()` (status=None) instead of 400.
    /// That made malformed input yield a BROADER query than intended.
    #[test]
    fn malformed_limit_is_400_not_default() {
        let result: Result<QueryFixture, _> = parse_query_or_400(Some("status=open&limit=abc"));
        let err = result.unwrap_err();
        assert_eq!(
            err.0,
            axum::http::StatusCode::BAD_REQUEST,
            "malformed ?limit= must return 400, not silently default \
             (old bug: .ok().unwrap_or_default() would drop status= too)"
        );
    }

    /// Malformed standalone limit → 400 error, NOT default.
    #[test]
    fn malformed_standalone_limit_is_400_not_default() {
        let result: Result<QueryFixture, _> = parse_query_or_400(Some("limit=abc"));
        let err = result.unwrap_err();
        assert_eq!(
            err.0,
            axum::http::StatusCode::BAD_REQUEST,
            "malformed ?limit=abc must return 400, not silently default to the cap"
        );
    }
}

mod artifact;
