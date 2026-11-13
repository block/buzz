//! HTTP API — media, git, NIP-05, and the Nostr HTTP bridge.

pub mod admin;
pub mod bridge;
pub mod buzz_v1;
pub mod events;
pub mod gifs;
pub mod git;
pub mod invites;
pub mod media;
/// Channel-aware media authorization.
pub mod media_access;
pub mod mesh_demo;
pub mod nip05;
pub mod nip_fi;
pub mod operator;
pub mod profiles;
pub mod transcriptions;
pub mod workflows;

// Re-export imeta helpers used by ingest pipeline.
pub use crate::handlers::imeta::{validate_imeta_tags, verify_imeta_blobs};

use axum::{http::StatusCode, response::Json};

/// Standard error envelope.
pub(crate) fn api_error(status: StatusCode, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (status, Json(serde_json::json!({ "error": msg })))
}

/// Build the shared client for relay-proxied third-party providers (GIF
/// search and transcription). Redirects are disabled: a provider credential
/// can ride in the request (KLIPY's API key is in the path), so following a
/// provider 3xx could replay it to an attacker-chosen host. With no redirect
/// policy, a 3xx comes back as a non-success status that callers map to a
/// generic `502`, and the `Location` target is never read or forwarded. The
/// 10-second default is a backstop; callers set their own per-request timeout.
pub fn build_provider_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static provider HTTP client configuration")
}

/// Why [`read_capped`] stopped before the end of a provider response.
pub(crate) enum CappedReadError {
    /// The declared or streamed body exceeded the cap.
    TooLarge,
    /// The body stream failed partway.
    Read(reqwest::Error),
}

/// Reads a provider response body, refusing more than `max` bytes whether the
/// length is declared up front or only discovered while streaming. Callers
/// map the error to their own generic message.
pub(crate) async fn read_capped(
    response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, CappedReadError> {
    use futures_util::StreamExt;

    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err(CappedReadError::TooLarge);
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(CappedReadError::Read)?;
        if body.len().saturating_add(chunk.len()) > max {
            return Err(CappedReadError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Per-minute member quota for a relay-proxied provider route: 429 with the
/// reset time once spent, 503 when shared admission is unavailable. `feature`
/// names the route in messages; quota rejections increment
/// `rejection_metric{reason="quota"}`.
pub(crate) async fn enforce_provider_quota(
    state: &crate::state::AppState,
    tenant: &buzz_core::TenantContext,
    pubkey: &nostr::PublicKey,
    limit_type: buzz_auth::LimitType,
    limit: u64,
    feature: &str,
    rejection_metric: &'static str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    match crate::admission::check_principal(
        state.admission_rate_limiter.as_ref(),
        tenant,
        pubkey,
        limit_type,
        60,
        limit,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(crate::admission::AdmissionError::Exceeded { reset_in_secs }) => {
            metrics::counter!(rejection_metric, "reason" => "quota").increment(1);
            Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                &format!("rate-limited: {feature} quota exceeded; retry in {reset_in_secs}s"),
            ))
        }
        Err(crate::admission::AdmissionError::Unavailable) => Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            &format!("rate-limited: {feature} admission unavailable"),
        )),
    }
}

pub(crate) fn internal_error(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("Internal error: {msg}");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
}

/// Stable client-visible body for a read cancelled by its server-side
/// statement deadline. Clients match this string to skip retrying: a retry
/// would re-run the same expensive query.
pub(crate) const QUERY_TIMED_OUT: &str = "query timed out";

/// Stable client-visible body for a read the relay could not get a database
/// connection for (pool timeout, closed pool, socket failure), including a
/// history read shed because the reader is busy. Transient: retry later.
pub(crate) const DATABASE_UNAVAILABLE: &str = "database unavailable";

/// Map a DB read failure: a statement-deadline cancel becomes a distinct
/// 503 `query timed out`, an unreachable or saturated database a 503
/// `database unavailable`; anything else stays a generic 500.
pub(crate) fn db_read_error(
    context: &str,
    e: &buzz_db::DbError,
) -> (StatusCode, Json<serde_json::Value>) {
    if e.is_connection_unavailable() {
        tracing::warn!(error_kind = %e.kind(), "{context}");
        return api_error(StatusCode::SERVICE_UNAVAILABLE, DATABASE_UNAVAILABLE);
    }
    read_error(context, e.is_statement_cancelled(), e.kind())
}

/// [`db_read_error`] for a search: a search cancelled by the search pool's
/// statement timeout gets the same 503 `query timed out`.
pub(crate) fn search_read_error(
    context: &str,
    e: &buzz_search::SearchError,
) -> (StatusCode, Json<serde_json::Value>) {
    read_error(context, e.is_statement_cancelled(), e.kind())
}

/// Only the error kind is logged: DB error text can repeat a bound value.
fn read_error(
    context: &str,
    statement_cancelled: bool,
    kind: String,
) -> (StatusCode, Json<serde_json::Value>) {
    if statement_cancelled {
        tracing::warn!(error_kind = %kind, "{context}");
        return api_error(StatusCode::SERVICE_UNAVAILABLE, QUERY_TIMED_OUT);
    }
    internal_error(&format!("{context}: {kind}"))
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

    use crate::access::AuthReason;
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

    /// An HTTP relay-membership refusal, with its access-log reason.
    #[derive(Debug)]
    pub(crate) enum MembershipRefusal {
        /// The relay's own JSON refusal.
        Api(AuthReason, (StatusCode, Json<serde_json::Value>)),
        /// NIP-FI's fixed denial. A request carrying a verified agent token
        /// gets only this, so the response never shows which step refused it
        /// or whether the agent already has a stored owner.
        NipFi(AuthReason, DenialClass),
    }

    /// A refusal decided inside [`admit_member`], before the agent-token
    /// rule picks its response shape, with shadow's `owner` reason when the
    /// agent token's owner decided it.
    type Refused = (
        AuthReason,
        (StatusCode, Json<serde_json::Value>),
        Option<&'static str>,
    );

    impl MembershipRefusal {
        /// The access-log result for this refusal.
        pub(crate) fn auth_result(&self) -> crate::access::AuthResult {
            let (Self::Api(reason, _) | Self::NipFi(reason, _)) = self;
            crate::access::AuthResult::Denied(*reason)
        }

        /// The response status.
        pub fn status(&self) -> StatusCode {
            match self {
                Self::Api(_, (status, _)) => *status,
                Self::NipFi(_, class) => StatusCode::from_u16(class.http_status())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            }
        }
    }

    impl IntoResponse for MembershipRefusal {
        fn into_response(self) -> Response {
            match self {
                Self::Api(_, refusal) => refusal.into_response(),
                Self::NipFi(_, class) => crate::nip_fi_core::http_denial(class),
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
    pub(crate) async fn enforce_relay_membership(
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
    ) -> Result<Option<nostr::PublicKey>, MembershipRefusal> {
        let result = admit_member(
            state,
            tenant,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
        )
        .await;
        // An agent token's owner-decided refusal is its request's NIP-FI
        // verdict, as shadow's verdict includes it.
        if let (Some(_), Err((_, refusal, Some(owner_reason)))) = (token_owner, &result) {
            crate::nip_fi_http::owner_refused(refusal_class(refusal.0), owner_reason);
        }
        result.map_err(|(reason, refusal, _)| match token_owner {
            None => MembershipRefusal::Api(reason, refusal),
            Some(_) => MembershipRefusal::NipFi(reason, refusal_class(refusal.0)),
        })
    }

    fn refusal_class(status: StatusCode) -> DenialClass {
        if status.is_server_error() {
            DenialClass::AuthorizationUnavailable
        } else {
            DenialClass::AuthorizationDenied
        }
    }

    async fn admit_member(
        state: &AppState,
        tenant: &TenantContext,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
    ) -> Result<Option<nostr::PublicKey>, Refused> {
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
                    AuthReason::NotRelayMember,
                    (
                        StatusCode::FORBIDDEN,
                        Json(serde_json::json!({
                            "error": "relay_membership_required",
                            "message": "You must be a relay member to access this relay"
                        })),
                    ),
                    None,
                ))
            }
            Err(e) => {
                tracing::error!("relay membership check errored: {e}");
                return Err((
                    AuthReason::MembershipUnavailable,
                    super::internal_error(&e),
                    None,
                ));
            }
        };
        deny_banned(
            state,
            community,
            pubkey_bytes,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
        )
        .await?;
        if let Some(token_owner) = token_owner {
            let Ok(agent) = nostr::PublicKey::from_slice(pubkey_bytes) else {
                return Err((
                    AuthReason::MembershipUnavailable,
                    super::internal_error("invalid pubkey for owner link"),
                    None,
                ));
            };
            match link_agent_owner(state, tenant, &agent, &token_owner).await {
                OwnerLink::Linked => {}
                OwnerLink::Conflict => {
                    return Err((
                        AuthReason::OwnerMismatch,
                        super::api_error(StatusCode::FORBIDDEN, OWNER_MISMATCH),
                        Some("not_stored_owner"),
                    ))
                }
                OwnerLink::Unavailable => {
                    return Err((
                        AuthReason::NipOaOwnerUnlinked,
                        super::api_error(
                            StatusCode::SERVICE_UNAVAILABLE,
                            crate::handlers::auth::OWNER_LINK_ERROR,
                        ),
                        Some("owner_link_unavailable"),
                    ))
                }
            }
        }
        Ok(owner)
    }

    /// Refuse a community-banned principal (own ban or its agent owner's) on
    /// HTTP. Bans only: a timeout blocks writes, and HTTP writes reach the
    /// ingest gate, while reads stay allowed. Each request's grant ends with
    /// the request, so this reads through the restriction cache (see
    /// [`AppState::restriction_state_cached`]). Fails closed with 503.
    async fn deny_banned(
        state: &AppState,
        community: CommunityId,
        pubkey_bytes: &[u8],
        auth_tag_header: Option<&str>,
        signed_auth_created_at: Option<u64>,
        token_owner: Option<nostr::PublicKey>,
    ) -> Result<(), Refused> {
        let Ok(pubkey) = nostr::PublicKey::from_slice(pubkey_bytes) else {
            return Err((
                AuthReason::MembershipUnavailable,
                super::internal_error("invalid pubkey for ban check"),
                None,
            ));
        };
        let verdict = crate::handlers::auth::community_ban_verdict(
            state,
            community,
            pubkey,
            auth_tag_header,
            signed_auth_created_at,
            token_owner,
            true,
        )
        .await;
        let (reason, refusal) = match verdict.outcome {
            crate::handlers::auth::BanOutcome::Clear => return Ok(()),
            crate::handlers::auth::BanOutcome::Banned => (
                AuthReason::Banned,
                super::api_error(
                    StatusCode::FORBIDDEN,
                    "blocked: you are banned from this community",
                ),
            ),
            crate::handlers::auth::BanOutcome::DbError => (
                AuthReason::BanCheckUnavailable,
                super::api_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "error: internal error checking restriction state",
                ),
            ),
        };
        Err((reason, refusal, verdict.owner_reason()))
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
    /// Returns `true` only when the link is recorded; an owner conflict and a
    /// storage failure both return `false`. Callers that must tell denial
    /// from unavailability use `link_agent_owner` directly.
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
            state.agent_owner_cache.insert(
                (tenant.community(), agent.to_bytes().to_vec()),
                owner.to_bytes().to_vec(),
            );
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
mod provider_http_client_tests {
    use std::sync::Arc;

    use axum::{
        http::{header, StatusCode},
        routing::get,
        Router,
    };

    use super::{api_error, build_provider_http_client};

    /// A provider 3xx must never cause a second connection, and the error
    /// the GIF handlers surface must leak neither the API key
    /// (carried in the request path) nor the redirect target.
    ///
    /// Mutation check: swapping `build_provider_http_client`'s redirect policy back
    /// to the default makes the client follow the 302, the redirect listener
    /// records a request, and this test fails on the `redirect_hits` assertion.
    #[tokio::test]
    async fn provider_client_refuses_redirects_without_leaking_secrets() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        const SECRET_KEY: &str = "super-secret-klipy-key";

        // Second listener: the redirect target. It must never be reached.
        let redirect_hits = Arc::new(AtomicUsize::new(0));
        let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redirect target");
        let redirect_addr = redirect_listener.local_addr().expect("redirect address");
        let redirect_hits_server = redirect_hits.clone();
        let redirect_server = tokio::spawn(async move {
            axum::serve(
                redirect_listener,
                Router::new().route(
                    "/leaked",
                    get(move || {
                        redirect_hits_server.fetch_add(1, Ordering::SeqCst);
                        async { "reached the redirect target" }
                    }),
                ),
            )
            .await
            .expect("serve redirect target");
        });

        // Fake upstream: answers the key-bearing path with a 302 whose Location
        // points at the second listener, exactly the disclosure vector.
        let redirect_location = format!("http://{redirect_addr}/leaked");
        let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake upstream");
        let upstream_addr = upstream_listener.local_addr().expect("upstream address");
        let location_header = redirect_location.clone();
        let upstream_server = tokio::spawn(async move {
            axum::serve(
                upstream_listener,
                Router::new().route(
                    &format!("/{SECRET_KEY}/gifs/search"),
                    get(move || {
                        let location = location_header.clone();
                        async move {
                            (
                                StatusCode::FOUND,
                                [(header::LOCATION, location)],
                                "provider body naming the secret-key",
                            )
                        }
                    }),
                ),
            )
            .await
            .expect("serve fake upstream");
        });

        let response = build_provider_http_client()
            .get(format!("http://{upstream_addr}/{SECRET_KEY}/gifs/search"))
            .send()
            .await
            .expect("request completes without following the redirect");

        // The redirect was not followed: the client surfaces the 3xx itself.
        assert!(response.status().is_redirection());
        assert!(!response.status().is_success());
        assert_eq!(redirect_hits.load(Ordering::SeqCst), 0);

        // The GIF handlers' reject path (both gate on `!is_success`) returns a
        // static generic error carrying no key and no redirect target.
        let (status, body) = api_error(
            StatusCode::BAD_GATEWAY,
            "GIF provider rejected the search request",
        );
        let serialized = serde_json::to_string(&body.0).expect("serialize generic error");
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(!serialized.contains(SECRET_KEY));
        assert!(!serialized.contains(&redirect_location));

        upstream_server.abort();
        redirect_server.abort();
        let _ = upstream_server.await;
        let _ = redirect_server.await;
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

    #[test]
    fn unavailable_database_maps_to_retryable_503() {
        for error in [
            sqlx::Error::PoolTimedOut,
            sqlx::Error::PoolClosed,
            sqlx::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
        ] {
            let (status, body) = db_read_error("ctx", &buzz_db::DbError::Sqlx(error));
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(body.0["error"], DATABASE_UNAVAILABLE);
        }
    }

    #[test]
    fn non_cancel_search_error_stays_generic_500() {
        let (status, body) = search_read_error(
            "ctx",
            &buzz_search::SearchError::Db(sqlx::Error::PoolTimedOut),
        );
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
pub(crate) mod artifact_sync;

#[cfg(test)]
mod media_access_postgres_tests;
