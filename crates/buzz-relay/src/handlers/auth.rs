//! NIP-42 AUTH handler — verify challenge response, transition auth state.
//!
//! Relay membership enforcement uses the shared
//! [`crate::api::relay_members::enforce_relay_membership`] helper, which supports
//! NIP-OA owner-delegation fallback on closed relays. The owner recorded for
//! agent→owner backfill (observer frame auth, agent rate class) is then resolved
//! with [`crate::api::relay_members::resolve_nip_oa_owner`], which keeps the
//! delegated owner when membership came through one and otherwise verifies the
//! presented tag — including for a direct member, whose attestation was
//! previously dropped. On a closed relay the claimed owner must itself be a
//! relay member, and the tag's time bounds are enforced against this AUTH
//! event's `created_at`.
//!
//! For WebSocket auth, the NIP-OA `auth` tag is extracted from the signed AUTH
//! event itself (the tag is integrity-protected by the event signature).

use std::sync::Arc;

use axum::extract::ws::Message as WsMessage;
use tracing::{debug, info, warn};

use crate::connection::{AuthState, ConnectionState};
use crate::protocol::RelayMessage;
use crate::state::AppState;

/// Extract a NIP-OA `auth` tag from a verified AUTH event and serialize it as
/// the JSON-array string that [`buzz_sdk::nip_oa::verify_auth_tag`] expects.
///
/// Returns `None` if no `auth` tag is present (direct-member auth path) or if
/// more than one `auth` tag exists (per NIP-OA spec: >1 auth tag ⇒ no valid tag).
pub fn extract_auth_tag_json(event: &nostr::Event) -> Option<String> {
    let mut iter = event
        .tags
        .iter()
        .filter(|t| t.as_slice().first().map(|s| s.as_str()) == Some("auth"));
    let first = iter.next()?;
    if iter.next().is_some() {
        return None; // NIP-OA spec: treat >1 auth tag as no valid auth tag
    }
    serde_json::to_string(first.as_slice()).ok()
}

/// Handle a NIP-42 AUTH message: verify the challenge response and transition
/// the connection to authenticated state.
///
/// Pure crypto verification — no API tokens, no JWT, no DB token lookups.
#[tracing::instrument(skip_all, fields(event_id, conn_id))]
pub async fn handle_auth(event: nostr::Event, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let event_id_hex = event.id.to_hex();
    let (challenge, conn_id) = {
        let auth = conn.auth_state.read().await;
        match &*auth {
            AuthState::Pending { challenge } => (challenge.clone(), conn.conn_id),
            AuthState::Authenticated(_) => {
                debug!(conn_id = %conn.conn_id, "AUTH received but already authenticated");
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: already authenticated",
                ));
                return;
            }
            AuthState::Failed => {
                debug!(conn_id = %conn.conn_id, "AUTH received after failed auth");
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: authentication already failed",
                ));
                return;
            }
        }
    };

    // Record the declared span fields now that we have the values.
    tracing::Span::current()
        .record("event_id", event_id_hex.as_str())
        .record("conn_id", conn_id.to_string().as_str());

    // Extract the NIP-OA auth tag before verification consumes the event.
    // The tag is integrity-protected by the event's Schnorr signature — if
    // tampered, NIP-42 verification will fail before we ever inspect it.
    let auth_tag_json = extract_auth_tag_json(&event);
    // Captured before `verify_auth_event` consumes the event. The NIP-OA tag
    // rides inside this AUTH event and is integrity-protected by its signature,
    // so this is the timestamp the attestation's time bounds are judged against.
    let auth_event_created_at = event.created_at.as_secs();

    let relay_url =
        crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &conn.tenant);
    let auth_svc = Arc::clone(&state.auth);

    metrics::counter!("buzz_auth_attempts_total", "method" => "nip42").increment(1);

    // Pure NIP-42 verification — crypto only, no DB lookups.
    match auth_svc
        .verify_auth_event(event, &challenge, &relay_url)
        .await
    {
        Ok(mut auth_ctx) => {
            let pubkey = auth_ctx.pubkey;

            // Community ban gate (NIP-42 seam). Runs immediately after auth
            // verification succeeds and before the allowlist and relay-membership
            // gates, per COMMUNITY_MODERATION_PLAN.md §0 decision 4 and the
            // MOD-7/M20 invariant (a ban must block connection auth even for open
            // channels — enforcement is structural, not filtered later). A banned
            // principal gets the standard protocol denial and the connection is
            // dropped with zero further processing.
            //
            // NIP-OA cascade: a ban on the authenticated pubkey blocks it directly;
            // a ban on its cryptographically-proven owner cascades to the agent
            // (owner ban ⇒ agents banned; agent ban is agent-only). The owner is
            // extracted from the self-proving auth tag with no DB round-trip.
            {
                // Fail closed on a DB error, but distinguish it from a real ban:
                // a transient blip must deny (never let a banned principal
                // through) without telling an innocent user they are banned and
                // pinning `Failed` for the connection's life on a false premise.
                // `Banned` claims the ban; `DbError` denies with `error: internal`
                // (mirrors the ingest write-path gate).
                enum BanOutcome {
                    Clear,
                    Banned,
                    DbError,
                }

                let mut outcome = match state
                    .db
                    .moderation_restriction_state(conn.tenant.community(), pubkey.as_bytes())
                    .await
                {
                    Ok(state) if state.banned => BanOutcome::Banned,
                    Ok(_) => BanOutcome::Clear,
                    Err(e) => {
                        warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e,
                              "ban-state DB lookup failed, denying (fail-closed)");
                        BanOutcome::DbError
                    }
                };

                // Cascade: check the proven NIP-OA owner only if the agent itself
                // is clear (a DB error already denies; a direct ban already blocks
                // — both skip the needless second DB read).
                if matches!(outcome, BanOutcome::Clear) {
                    if let Some(owner) = crate::api::relay_members::extract_nip_oa_owner(
                        pubkey.as_bytes(),
                        auth_tag_json.as_deref(),
                    ) {
                        outcome = match state
                            .db
                            .moderation_restriction_state(conn.tenant.community(), owner.as_bytes())
                            .await
                        {
                            Ok(state) if state.banned => BanOutcome::Banned,
                            Ok(_) => BanOutcome::Clear,
                            Err(e) => {
                                warn!(conn_id = %conn_id, owner = %owner.to_hex(), error = %e,
                                      "owner ban-state DB lookup failed, denying (fail-closed)");
                                BanOutcome::DbError
                            }
                        };
                    }
                }

                let denial: Option<(&str, &str)> = match outcome {
                    BanOutcome::Clear => None,
                    BanOutcome::Banned => {
                        Some(("banned", "blocked: you are banned from this community"))
                    }
                    BanOutcome::DbError => Some((
                        "ban_check_error",
                        "error: internal error checking restriction state",
                    )),
                };

                if let Some((metric_reason, deny_reason)) = denial {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), reason = deny_reason, "principal denied at ban seam");
                    metrics::counter!("buzz_auth_failures_total", "reason" => metric_reason)
                        .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    // Decision 4: banned ⇒ OK false + immediate WebSocket close.
                    // Route the reason frame on the control channel (not `send`,
                    // which uses the data channel and would race the cancel), so
                    // the send loop drains it ahead of the Close it emits on
                    // cancel. Then cancel to close the socket immediately.
                    let _ = conn.ctrl_tx.try_send(WsMessage::Text(
                        RelayMessage::ok(&event_id_hex, false, deny_reason).into(),
                    ));
                    conn.cancel.cancel();
                    return;
                }
            }

            // Pubkey allowlist gate — only for pubkey-only auth.
            if state.config.pubkey_allowlist_enabled
                && auth_ctx.auth_method == buzz_auth::AuthMethod::Nip42
            {
                let allowed = match state
                    .db
                    .is_pubkey_allowed(conn.tenant.community(), pubkey.as_bytes())
                    .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e,
                              "allowlist DB lookup failed, denying (fail-closed)");
                        false
                    }
                };
                if !allowed {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "pubkey not in allowlist");
                    metrics::counter!("buzz_auth_failures_total", "reason" => "allowlist_denied")
                        .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    conn.send(RelayMessage::ok(
                        &event_id_hex,
                        false,
                        "auth-required: verification failed",
                    ));
                    return;
                }
            }

            // Relay membership gate — uses the shared helper with NIP-OA fallback.
            let nip_oa_owner = match crate::api::relay_members::enforce_relay_membership(
                &state,
                conn.tenant.community(),
                pubkey.as_bytes(),
                auth_tag_json.as_deref(),
            )
            .await
            {
                Ok(owner) => owner,
                Err(e) => {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = ?e, "not a relay member");
                    metrics::counter!("buzz_auth_failures_total", "reason" => "not_relay_member")
                        .increment(1);
                    *conn.auth_state.write().await = AuthState::Failed;
                    conn.send(RelayMessage::ok(
                        &event_id_hex,
                        false,
                        "restricted: not a relay member",
                    ));
                    return;
                }
            };

            // NIP-OA backfill: resolve the owner for the agent→owner DB mapping
            // (needed for observer frame auth and for the agent rate class).
            // `enforce_relay_membership` reports an owner only when membership was
            // granted *through* it, so a direct member's tag is resolved here —
            // on closed relays too, subject to the owner itself being a relay
            // member. The tag's time bounds are judged against this AUTH event's
            // `created_at`: the tag is carried inside it and integrity-protected
            // by its signature, so it is the artifact the attestation authorized.
            let nip_oa_owner = crate::api::relay_members::resolve_nip_oa_owner(
                &state,
                conn.tenant.community(),
                nip_oa_owner,
                pubkey.as_bytes(),
                auth_tag_json.as_deref(),
                auth_event_created_at,
            )
            .await;

            // Stash NIP-OA owner on the auth context only after the shared
            // backfill confirms the first-write-wins relationship.
            if let Some(owner) = nip_oa_owner {
                if crate::api::relay_members::materialize_nip_oa_owner(
                    &state,
                    &conn.tenant,
                    &pubkey,
                    &owner,
                )
                .await
                {
                    auth_ctx.agent_owner_pubkey = Some(owner);
                } else {
                    warn!(
                        conn_id = %conn_id,
                        agent = %pubkey.to_hex(),
                        nip_oa_owner = %owner.to_hex(),
                        "NIP-OA owner could not be materialized"
                    );
                }
            }

            info!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "NIP-42 auth successful");
            *conn.auth_state.write().await = AuthState::Authenticated(auth_ctx);
            state
                .conn_manager
                .set_authenticated_pubkey(conn_id, pubkey.to_bytes().to_vec());
            conn.send(RelayMessage::ok(&event_id_hex, true, ""));
        }
        Err(e) => {
            warn!(conn_id = %conn_id, error = %e, "NIP-42 auth failed");
            metrics::counter!("buzz_auth_failures_total", "reason" => "nip42_invalid").increment(1);
            *conn.auth_state.write().await = AuthState::Failed;
            conn.send(RelayMessage::ok(
                &event_id_hex,
                false,
                "auth-required: verification failed",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_auth_tag_json;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    /// Build a signed NIP-98 (kind 27235) event carrying the given tags. The
    /// `auth` tag lives inside the signed event exactly as the git and
    /// WebSocket auth paths receive it.
    fn signed_event_with_tags(tags: Vec<Tag>) -> nostr::Event {
        EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign auth event")
    }

    /// A single `auth` tag is extracted verbatim as its JSON-array string —
    /// this is the exact value fed to `verify_auth_tag` on the git path.
    #[test]
    fn single_auth_tag_extracted_verbatim() {
        let owner = Keys::generate().public_key().to_hex();
        let sig = "00".repeat(64);
        let event = signed_event_with_tags(vec![
            Tag::parse(["u", "https://relay/git/x/y"]).unwrap(),
            Tag::parse(["auth", owner.as_str(), "", sig.as_str()]).unwrap(),
        ]);

        let extracted = extract_auth_tag_json(&event).expect("auth tag present");
        let expected = serde_json::to_string(&["auth", owner.as_str(), "", sig.as_str()]).unwrap();
        assert_eq!(extracted, expected);
    }

    /// No `auth` tag → `None` (the direct-member path, tag absent).
    #[test]
    fn no_auth_tag_returns_none() {
        let event =
            signed_event_with_tags(vec![Tag::parse(["u", "https://relay/git/x/y"]).unwrap()]);
        assert_eq!(extract_auth_tag_json(&event), None);
    }

    /// More than one `auth` tag → `None`. Per NIP-OA, an ambiguous set of
    /// attestations is treated as no valid attestation (fail-closed), so a
    /// second forged tag cannot smuggle an alternate delegation past the gate.
    #[test]
    fn duplicate_auth_tags_return_none() {
        let a = Keys::generate().public_key().to_hex();
        let b = Keys::generate().public_key().to_hex();
        let sig = "00".repeat(64);
        let event = signed_event_with_tags(vec![
            Tag::parse(["auth", a.as_str(), "", sig.as_str()]).unwrap(),
            Tag::parse(["auth", b.as_str(), "", sig.as_str()]).unwrap(),
        ]);
        assert_eq!(extract_auth_tag_json(&event), None);
    }
    // ── NIP-OA owner materialization over the real NIP-42 path ──────────────
    //
    // These drive `handle_auth` itself, so reverting its production call site
    // makes the first one fail. `#[ignore]`d — they need Postgres and Redis,
    // and CI selects them by name (`test(/nip_oa_owner_/)`).

    use std::collections::HashMap;
    use std::sync::atomic::AtomicU8;
    use std::sync::Arc;
    use tokio::sync::{mpsc, Mutex, RwLock};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use crate::connection::{AuthState, ConnectionState};
    use crate::state::AppState;
    use buzz_core::tenant::TenantContext;

    async fn ws_test_state() -> Option<(Arc<AppState>, sqlx::PgPool)> {
        let mut config = crate::config::Config::from_env().ok()?;
        config.require_relay_membership = true;
        let pool = sqlx::PgPool::connect(&config.database_url).await.ok()?;
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .ok()?;
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .ok()?,
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).ok()?;
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            Keys::generate(),
            media_storage,
        );
        Some((Arc::new(state), pool))
    }

    async fn ws_seed_community(pool: &sqlx::PgPool) -> TenantContext {
        let id = Uuid::new_v4();
        let host = format!("nip-oa-ws-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&host)
            .execute(pool)
            .await
            .expect("insert test community");
        TenantContext::resolved(buzz_core::CommunityId::from_uuid(id), host)
    }

    /// A pending connection ready to receive AUTH, plus its challenge.
    fn pending_conn(tenant: &TenantContext) -> (Arc<ConnectionState>, String) {
        let challenge = buzz_auth::generate_challenge();
        let (send_tx, _send_rx) = mpsc::channel(16);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(16);
        let conn = Arc::new(ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: tenant.clone(),
            remote_addr: "127.0.0.1:1234".parse().expect("socket addr"),
            auth_state: RwLock::new(AuthState::Pending {
                challenge: challenge.clone(),
            }),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            cancel: CancellationToken::new(),
            backpressure_count: Arc::new(AtomicU8::new(0)),
            grace_limit: 3,
        });
        (conn, challenge)
    }

    /// Sign a NIP-42 AUTH event for `state`'s relay URL, carrying `tag_json`
    /// as an `auth` tag, stamped at `created_at`.
    fn auth_event(
        state: &AppState,
        tenant: &TenantContext,
        agent: &Keys,
        challenge: &str,
        tag_json: Option<&str>,
        created_at: u64,
    ) -> nostr::Event {
        let relay_url =
            crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, tenant);
        let url = nostr::RelayUrl::parse(&relay_url).expect("relay url");
        let mut builder = EventBuilder::auth(challenge, url);
        if let Some(tag) = tag_json {
            let parts: Vec<String> = serde_json::from_str(tag).expect("auth tag json");
            builder = builder.tags([Tag::parse(parts).expect("auth tag")]);
        }
        builder
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(agent)
            .expect("sign auth event")
    }

    /// NIP-42 rejects a stale AUTH event, so these tests stamp at the real
    /// clock and express the tag's bounds relative to it — which is also how a
    /// live deployment presents an expiring credential.
    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after epoch")
            .as_secs()
    }

    async fn ws_stored_owner(
        state: &AppState,
        tenant: &TenantContext,
        agent: &nostr::PublicKey,
    ) -> Option<Vec<u8>> {
        state
            .db
            .get_agent_channel_policy(tenant.community(), agent.as_bytes())
            .await
            .expect("read agent policy")
            .and_then(|(_, owner)| owner)
    }

    /// The regression on the WebSocket path: a direct member authenticating
    /// with a valid attestation gets its owner recorded *and* carried onto the
    /// live auth context, which is what observer-frame authorization and the
    /// agent rate class both read.
    #[tokio::test]
    #[ignore = "requires Postgres and Redis"]
    async fn nip_oa_owner_ws_records_owner_and_sets_auth_context() {
        let Some((state, pool)) = ws_test_state().await else {
            return;
        };
        let tenant = ws_seed_community(&pool).await;
        let agent = Keys::generate();
        let owner = Keys::generate();

        for pubkey in [agent.public_key(), owner.public_key()] {
            state
                .db
                .add_relay_member(tenant.community(), &pubkey.to_hex(), "member", None)
                .await
                .expect("add relay member");
        }

        let tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "")
            .expect("compute auth tag");
        let (conn, challenge) = pending_conn(&tenant);
        let event = auth_event(&state, &tenant, &agent, &challenge, Some(&tag), now_secs());

        super::handle_auth(event, Arc::clone(&conn), Arc::clone(&state)).await;

        assert_eq!(
            ws_stored_owner(&state, &tenant, &agent.public_key()).await,
            Some(owner.public_key().to_bytes().to_vec()),
            "NIP-42 auth by a direct member must record its verified owner",
        );

        let auth_state = conn.auth_state.read().await;
        match &*auth_state {
            AuthState::Authenticated(ctx) => assert_eq!(
                ctx.agent_owner_pubkey,
                Some(owner.public_key()),
                "the live auth context must carry the owner",
            ),
            other => panic!("expected authenticated connection, got {other:?}"),
        }
    }

    /// An expired attestation authenticates the agent but confers nothing:
    /// no ownership record, and no owner on the session, so the connection
    /// cannot be classified into the agent rate tier.
    #[tokio::test]
    #[ignore = "requires Postgres and Redis"]
    async fn nip_oa_owner_ws_refuses_an_expired_attestation() {
        let Some((state, pool)) = ws_test_state().await else {
            return;
        };
        let tenant = ws_seed_community(&pool).await;
        let agent = Keys::generate();
        let owner = Keys::generate();

        for pubkey in [agent.public_key(), owner.public_key()] {
            state
                .db
                .add_relay_member(tenant.community(), &pubkey.to_hex(), "member", None)
                .await
                .expect("add relay member");
        }

        let now = now_secs();
        // Bound sits at the AUTH event's own timestamp; bounds are strict, so
        // the credential is one second past expiry.
        let expired = buzz_sdk::nip_oa::compute_auth_tag(
            &owner,
            &agent.public_key(),
            &format!("created_at<{now}"),
        )
        .expect("compute auth tag");
        let (conn, challenge) = pending_conn(&tenant);
        let event = auth_event(&state, &tenant, &agent, &challenge, Some(&expired), now);

        super::handle_auth(event, Arc::clone(&conn), Arc::clone(&state)).await;

        assert_eq!(
            ws_stored_owner(&state, &tenant, &agent.public_key()).await,
            None,
            "an expired attestation must not be materialized",
        );

        let auth_state = conn.auth_state.read().await;
        match &*auth_state {
            AuthState::Authenticated(ctx) => assert_eq!(
                ctx.agent_owner_pubkey, None,
                "an expired attestation must not classify the session as an agent",
            ),
            other => panic!("expected authenticated connection, got {other:?}"),
        }
    }

    /// A non-member owner is not trusted on a closed relay, so nothing is
    /// recorded and the session stays unclassified.
    #[tokio::test]
    #[ignore = "requires Postgres and Redis"]
    async fn nip_oa_owner_ws_refuses_an_owner_that_is_not_a_relay_member() {
        let Some((state, pool)) = ws_test_state().await else {
            return;
        };
        let tenant = ws_seed_community(&pool).await;
        let agent = Keys::generate();
        let stranger = Keys::generate();

        state
            .db
            .add_relay_member(
                tenant.community(),
                &agent.public_key().to_hex(),
                "member",
                None,
            )
            .await
            .expect("add relay member");

        let tag = buzz_sdk::nip_oa::compute_auth_tag(&stranger, &agent.public_key(), "")
            .expect("compute auth tag");
        let (conn, challenge) = pending_conn(&tenant);
        let event = auth_event(&state, &tenant, &agent, &challenge, Some(&tag), now_secs());

        super::handle_auth(event, Arc::clone(&conn), Arc::clone(&state)).await;

        assert_eq!(
            ws_stored_owner(&state, &tenant, &agent.public_key()).await,
            None,
            "a non-member owner must not be recorded on a closed relay",
        );
    }
}
