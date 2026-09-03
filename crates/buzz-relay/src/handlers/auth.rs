//! NIP-42 AUTH handler — verify challenge response, transition auth state.
//!
//! Relay membership enforcement uses the shared
//! [`crate::api::relay_members::enforce_relay_membership`] helper, which supports
//! NIP-OA owner-delegation fallback on closed relays. On open relays, the auth
//! handler calls [`crate::api::relay_members::extract_nip_oa_owner`] directly to
//! extract the owner pubkey for agent→owner backfill (observer frame auth).
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
    let signed_auth_created_at = event.created_at.as_secs();

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

            // NIP-FI key pairing [FI-INV-05]: immediately after successful
            // verify_auth_event, before community-ban/allowlist/membership gates.
            // Pre-DB positioning means a denied caller pays zero DB cost and the
            // production call site is falsifiable without live tenant policy.
            // [FI-TRACE-DENIAL-ORACLE post-establishment]
            if crate::nip_fi_session::enforce_nip_fi_key_pairing(
                conn.nip_fi_assertion.as_ref(),
                pubkey,
                crate::nip_fi_session::PairingDenialTarget::Root(conn.as_ref()),
            )
            .await
                == crate::nip_fi_session::PairingOutcome::Denied
            {
                return;
            }

            // Community ban gate (NIP-42 seam). Runs after NIP-FI pairing and
            // before the allowlist and relay-membership gates, per
            // COMMUNITY_MODERATION_PLAN.md §0 decision 4 and the MOD-7/M20
            // invariant (a ban must block connection auth even for open channels —
            // enforcement is structural, not filtered later). A banned principal
            // gets the standard protocol denial and the connection is dropped with
            // zero further processing.
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
                        Some(signed_auth_created_at),
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
                Some(signed_auth_created_at),
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

            // Open relay NIP-OA backfill: extract owner for agent→owner DB mapping
            // (needed for observer frame auth). Only runs on open relays — on closed
            // relays, enforce_relay_membership already handles NIP-OA delegation.
            // No feature flag needed: NIP-OA is cryptographically self-proving.
            let nip_oa_owner = nip_oa_owner.or_else(|| {
                if !state.config.require_relay_membership && auth_tag_json.is_some() {
                    crate::api::relay_members::extract_nip_oa_owner(
                        pubkey.as_bytes(),
                        auth_tag_json.as_deref(),
                        Some(signed_auth_created_at),
                    )
                } else {
                    None
                }
            });

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
            // B2: acquire a session effect permit before committing auth state.
            //
            // Gate ordering: acquire_effect() obtains the fair read lock, then
            // checks cancel and deadline. A permit is returned only when the
            // session is still active — expiry cannot transition to Expired
            // while any permit is held (the permit IS the read lock). This
            // replaces the old "acquire write_lock → check cancel" fence with
            // a stronger bound: no AUTH commit can start after the gate's
            // deadline passes or after the expiry task's cancel.cancel() fires,
            // and any AUTH commit that starts under a permit will complete before
            // the gate's quiescence barrier allows teardown to proceed.
            //
            // Off-mode (no gate): no permit is needed; proceed unconditionally.
            // [FI-TRACE-LEASE-BOUND, B2 seam: AUTH commit]
            //
            // Test hook: fires immediately before acquire_effect so a test can
            // arm expiry between the NIP-42 verification success and the permit
            // acquisition. This is the exact async gap W1 (auth barrier witness)
            // exercises. No-op in production (cfg(test) only, Mutex<None> unless
            // armed). [nip_fi_test_hooks::auth_commit_hook]
            #[cfg(test)]
            crate::nip_fi_test_hooks::before_auth_commit(conn.tenant.community()).await;
            let _auth_permit = match conn.nip_fi_gate.acquire_effect().await {
                Ok(permit) => permit,
                Err(crate::nip_fi_gate::SessionExpired) => return,
            };
            *conn.auth_state.write().await = AuthState::Authenticated(auth_ctx);
            // The permit is held through set_authenticated_pubkey and the OK send
            // so the entire auth commit is atomic with respect to expiry.
            state
                .conn_manager
                .set_authenticated_pubkey(conn_id, pubkey.to_bytes().to_vec());

            // Test hook: fires immediately after set_authenticated_pubkey (registration)
            // and before the deny-set check, so a straddle test can insert a deny entry
            // in the exact window between registration and check. No-op in production.
            // [nip_fi_test_hooks::deny_set_check_hook, W_deny_straddle]
            #[cfg(test)]
            crate::nip_fi_test_hooks::before_deny_set_check(conn.tenant.community()).await;

            // Step 6 (NIP-FI.md:227-233): deny-set check — runs AFTER
            // registration so any concurrent disconnect either sees this
            // session in the close scan OR we see the deny entry here.
            // Both sides of the straddle are covered; neither side can miss.
            // [FI-TRACE-DENY-SET]
            if let Some(assertion) = &conn.nip_fi_assertion {
                if let Some(asserted_key) = assertion.asserted_key() {
                    if let Some(deny_map) = state.nip_fi_deny_map.as_deref() {
                        if deny_map.is_denied(
                            assertion.identity().issuer(),
                            &asserted_key,
                            chrono::Utc::now(),
                        ) {
                            warn!(
                                conn_id = %conn_id,
                                pubkey = %pubkey.to_hex(),
                                "NIP-FI deny-set hit at post-registration check — denying"
                            );
                            metrics::counter!(
                                "buzz_nip_fi_admission_denied_total",
                                "reason" => "deny_set_post_registration"
                            )
                            .increment(1);
                            let _ = conn.ctrl_tx.try_send(
                                crate::nip_fi_session::authorization_denied_frame(
                                    crate::nip_fi_session::NipFiWsRoute::Root,
                                ),
                            );
                            conn.cancel.cancel();
                            return;
                        }
                    }
                }
            }

            conn.send(RelayMessage::ok(&event_id_hex, true, ""));
            // _auth_permit drops here — expiry's write guard may proceed.
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
    use super::{extract_auth_tag_json, handle_auth};
    use crate::connection::AuthState;
    use axum::extract::ws::Message as WsMessage;
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

    // ── Witness A: Root pairing mismatch through the real root denial path ────
    //
    // Drives the production `handle_auth`, NOT the shared function alone.
    // The NIP-FI pairing call site is pre-DB: it fires immediately after
    // `verify_auth_event` succeeds, before any community-ban/allowlist/membership
    // DB gate. A lazy DB pool suffices — the test returns before any DB read.
    //
    // Mutation evidence:
    //   - Delete the production call from `handle_auth` → no Denied; test panics
    //     on AuthState (not Failed) or ctrl frame (absent) assertions.
    //   - Delete the denial branch inside `enforce_nip_fi_key_pairing` → same.
    //   - Emit on send_tx instead of ctrl_tx → ctrl frame assertion panics.
    //   - Omit `AuthState::Failed` → auth_state assertion panics.
    //   - Omit `cancel.cancel()` → cancellation assertion panics.

    async fn auth_test_state() -> std::sync::Arc<crate::state::AppState> {
        use std::sync::Arc;
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.database_url = "postgres://buzz:buzz_dev@127.0.0.1:1/buzz".to_string();
        config.redis_url = "redis://127.0.0.1:1".to_string();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    /// Like `auth_test_state` but connects to the real local DB at port 5432.
    ///
    /// Required for W1: the ban-check path is fail-closed, so a lazy-pool error
    /// causes the handler to deny before reaching `before_auth_commit`. With the
    /// real DB, an unknown pubkey/community returns `BanOutcome::Clear`.
    ///
    /// Returns `None` if the local DB is not reachable — callers should skip the
    /// test in that case rather than fail.
    async fn auth_test_state_real_db() -> Option<std::sync::Arc<crate::state::AppState>> {
        use std::sync::Arc;
        let db_url = "postgres://buzz:buzz_dev@127.0.0.1:5432/buzz";
        // Probe connectivity before constructing the full state.
        if sqlx::PgPool::connect(db_url).await.is_err() {
            return None;
        }
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        config.database_url = db_url.to_string();
        config.redis_url = "redis://127.0.0.1:1".to_string();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub manager"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = crate::state::AppState::new(
            config,
            db,
            redis_pool,
            audit,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Some(Arc::new(state))
    }

    #[tokio::test]
    async fn handle_auth_pairing_mismatch_runs_full_root_denial_path() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::{mpsc, RwLock};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        // Key A named in assertion; key B signs the NIP-42 event — mismatch.
        let key_a = Keys::generate();
        let key_b = Keys::generate();

        let assertion = VerifiedAssertion::for_test(
            Some(key_a.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let challenge = "test-challenge-A".to_string();
        let (send_tx, mut send_rx) = mpsc::channel::<WsMessage>(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, mut terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
        let cancel = CancellationToken::new();

        let conn = Arc::new(crate::connection::ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(AuthState::Pending {
                challenge: challenge.clone(),
            }),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: Some(assertion),
            session_deadline: None,
            nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone()),
        });

        let state = auth_test_state().await;

        // relay_url = ws://<tenant.host()> where scheme prefix is from config
        // (default ws://), and host is "test.local".
        let relay_url = "ws://test.local";
        let auth_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::parse(["relay", relay_url]).unwrap())
            .tag(Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key_b)
            .unwrap();

        // Drive the production handle_auth path.
        handle_auth(auth_event, Arc::clone(&conn), state).await;

        assert!(
            cancel.is_cancelled(),
            "connection must be cancelled on pairing mismatch"
        );
        assert!(
            matches!(*conn.auth_state.read().await, AuthState::Failed),
            "auth_state must be Failed after pairing mismatch"
        );
        let ctrl_frame = terminal_ctrl_rx
            .try_recv()
            .expect("terminal channel must contain the denial notice frame");
        // Terminal queue must hold exactly one frame — no duplicate denial.
        assert!(
            terminal_ctrl_rx.try_recv().is_err(),
            "terminal channel must hold exactly one frame after pairing mismatch"
        );
        // ctrl_tx (ordinary queue) must be empty — denial goes to terminal only.
        assert!(
            ctrl_rx.try_recv().is_err(),
            "ordinary ctrl channel must be empty after pairing denial (frame goes to terminal)"
        );
        assert!(
            send_rx.try_recv().is_err(),
            "denial must not appear on the data channel"
        );
        // Assert the full wire text byte-for-byte.
        let expected_notice = crate::protocol::RelayMessage::notice(
            buzz_auth::DenialClass::AuthorizationDenied.nostr_text(),
        );
        match ctrl_frame {
            WsMessage::Text(text) => {
                assert_eq!(
                    text,
                    expected_notice,
                    "terminal frame must be byte-identical to RelayMessage::notice(\"restricted: authorization denied\")"
                );
            }
            other => panic!("terminal frame must be Text(NOTICE); got {other:?}"),
        }
    }

    // ── B2: Cancelled connection is never admitted to Authenticated state ──────
    //
    // The B2 fence at the admission boundary (`if conn.cancel.is_cancelled() {
    // return; }`) prevents committing `AuthState::Authenticated` after the NIP-FI
    // expiry task has cancelled the connection in the async gap between dispatch
    // and admission.
    //
    // This test pre-cancels the token and confirms that after `handle_auth` the
    // connection is NOT `Authenticated`. The mechanism varies: on the test
    // lazy-DB path, the ban check also denies (DbError path) — but the invariant
    // holds regardless of which guard fires first.
    //
    // Mutation evidence:
    //   Removing the B2 fence is only observable in the narrow async window where
    //   the ban gate succeeds AND cancel fires after it. In the unit-test context
    //   the DB gate fires first; in a real deployment the B2 fence is the guard
    //   for that window. The test asserts the invariant (never Authenticated when
    //   cancelled) and documents the expected runtime behavior.
    #[tokio::test]
    async fn b2_pre_cancelled_connection_never_becomes_authenticated() {
        use chrono::{Duration, Utc};
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::{mpsc, RwLock};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        // Use the same key for both assertion and NIP-42 event (no pairing mismatch).
        // The cancel token is pre-cancelled to simulate the B2 window.
        let key = Keys::generate();
        let assertion = buzz_auth::VerifiedAssertion::for_test(
            Some(key.public_key()),
            vec![Utc::now() + Duration::hours(1)],
        );

        let challenge = "test-challenge-B2".to_string();
        let (send_tx, _send_rx) = mpsc::channel::<WsMessage>(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, _terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);

        // Pre-cancel the token — simulates the expiry task having already fired.
        let cancel = CancellationToken::new();
        cancel.cancel();

        let conn = Arc::new(crate::connection::ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(
                buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(AuthState::Pending {
                challenge: challenge.clone(),
            }),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: Some(assertion),
            session_deadline: None,
            nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone()),
        });

        let state = auth_test_state().await;
        let relay_url = "ws://test.local";
        let auth_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::parse(["relay", relay_url]).unwrap())
            .tag(Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();

        handle_auth(auth_event, Arc::clone(&conn), state).await;

        // Regardless of the path taken (B2 fence, DB error, etc.), the
        // connection MUST NOT be in Authenticated state when it was already
        // cancelled before handle_auth ran.
        assert!(
            !matches!(*conn.auth_state.read().await, AuthState::Authenticated(_)),
            "B2: a pre-cancelled connection must never reach AuthState::Authenticated"
        );
    }

    // ── W1 (auth barrier): expiry fired mid-flight blocks AUTH commit ─────────
    //
    // Arms `before_auth_commit` — the hook immediately before `acquire_effect()`
    // in the AUTH commit path. Dispatches `handle_auth` with a live (not-yet-
    // expired) gate, waits for the hook to signal the handler reached the
    // permit boundary, fires the gate expiry (cancel), then releases the hook.
    // The handler tries `acquire_effect()` and gets `SessionExpired`, returns
    // without committing `AuthState::Authenticated`.
    //
    // This is the real barrier test Paul requires: the handler runs through
    // NIP-42 verification, pairing check, ban check, allowlist, and membership
    // gates, then stalls at `before_auth_commit`. Expiry fires *in that async
    // gap*. The permit acquisition fails, and no auth commit occurs.
    //
    // Hook location: `handlers/auth.rs`, immediately before `acquire_effect()`
    // at the B2 AUTH commit seam.
    //
    // Mutation evidence:
    //   A) Delete `#[cfg(test)] before_auth_commit(...)` from auth.rs → handler
    //      never stalls at the hook → cancel fires before handler reaches
    //      acquire_effect → handler completes auth before cancel is checked
    //      (race) OR the gate denies anyway on cancel check. The test is
    //      non-deterministic without the hook; WITH the hook the barrier is exact.
    //   B) Remove `acquire_effect()` from auth.rs → handler commits
    //      AuthState::Authenticated despite the cancel → assertion panics.
    //   C) Change gate from deadline-with-cancel to off_mode → acquire_effect
    //      succeeds even after cancel → handler commits auth → assertion panics.
    //
    // Requires a local DB (default postgres://buzz:buzz_dev@localhost:5432/buzz)
    // for the ban-check path that precedes the hook. The DB call returns
    // "not banned" for an unknown community/pubkey — a real result, not mocked.
    #[tokio::test]
    async fn w1_auth_barrier_expiry_mid_flight_blocks_auth_commit() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::{mpsc, RwLock};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        // Same key for assertion and NIP-42 event — pairing passes.
        let key = Keys::generate();
        let deadline = Utc::now() + Duration::hours(1);
        let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);

        let challenge = "w1-barrier-challenge".to_string();
        let (send_tx, mut send_rx) = mpsc::channel::<WsMessage>(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, _terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);

        // Live gate — NOT pre-cancelled. acquire_effect succeeds unless we fire expiry.
        let cancel = CancellationToken::new();
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());

        let conn = Arc::new(crate::connection::ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(community, "test.local".to_string()),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(AuthState::Pending {
                challenge: challenge.clone(),
            }),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: Some(assertion),
            session_deadline: Some(deadline),
            nip_fi_gate: gate,
        });

        // W1 requires a real DB (ban-check is fail-closed; lazy pool errors → deny before hook).
        let state = match auth_test_state_real_db().await {
            Some(s) => s,
            None => {
                eprintln!("W1: skipping — local DB not available at postgres://buzz:buzz_dev@127.0.0.1:5432/buzz");
                return;
            }
        };
        let relay_url = "ws://test.local";
        let auth_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::parse(["relay", relay_url]).unwrap())
            .tag(Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();

        // Arm the barrier: fires when handle_auth reaches before_auth_commit.
        let (arrived_rx, release) = crate::nip_fi_test_hooks::auth_commit_hook::arm(community);

        // Spawn handle_auth — it will stall at the hook.
        let conn2 = Arc::clone(&conn);
        let state2 = Arc::clone(&state);
        let handle = tokio::spawn(async move { handle_auth(auth_event, conn2, state2).await });

        // Wait for the handler to reach the permit boundary.
        tokio::time::timeout(std::time::Duration::from_secs(5), arrived_rx)
            .await
            .expect("W1: handler must reach before_auth_commit within 5s")
            .expect("arrived channel closed");

        // Fire expiry: cancel the gate's token so acquire_effect returns SessionExpired.
        cancel.cancel();

        // Release the hook — handler resumes and calls acquire_effect().
        release.notify_one();

        // Wait for handle_auth to return.
        tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("W1: handle_auth must return within 5s after hook release")
            .expect("handle_auth task must not panic");

        // Auth state must NOT be Authenticated — the permit was denied.
        assert!(
            !matches!(*conn.auth_state.read().await, AuthState::Authenticated(_)),
            "W1: auth_state must NOT be Authenticated after mid-flight expiry"
        );

        // No OK(true) must be on the data channel — auth was not committed.
        while let Ok(frame) = send_rx.try_recv() {
            if let WsMessage::Text(t) = &frame {
                assert!(
                    !t.contains("\"true\"") && !t.contains(r#"[true"#),
                    "W1: no OK(true) must be sent when auth is denied by gate; got: {t}"
                );
            }
        }
    }

    // ── W_deny_straddle: deny entry inserted in window between registration and check
    //
    // Arms `before_deny_set_check` — the hook immediately AFTER
    // `set_authenticated_pubkey` (registration) and BEFORE the `is_denied` call.
    // The key starts absent from the deny map. Once registration occurs the
    // handler stalls at the hook. The test inserts the deny entry into the live
    // map, then releases the hook. The deny check fires and finds the entry;
    // the connection is closed without sending OK(true).
    //
    // This is the canonical straddle proof: a disconnect that fires in this
    // window would also see the registered session (close-scan side). This test
    // exercises the check side — proving the normative placement catches the
    // entry inserted after registration.
    //
    // Hook location: `handlers/auth.rs`, immediately after
    // `state.conn_manager.set_authenticated_pubkey(...)` at the deny-check seam.
    //
    // Mutation evidence:
    //   A) Delete `#[cfg(test)] before_deny_set_check(...)` from auth.rs →
    //      handler never stalls → deny entry inserted AFTER check runs and
    //      missed → OK(true) is sent → "no OK(true)" assertion panics.
    //   B) Remove the `is_denied` check entirely → same outcome as (A).
    //   C) Move hook to before `set_authenticated_pubkey` → handler stalls
    //      before registration → close-scan side cannot see session → but this
    //      test still passes (entry is still inserted before check).
    //      The hook position verifies BOTH that the barrier is at the correct
    //      seam AND that the check fires after it.
    //
    // Requires a local DB (same constraint as W1: ban-check is fail-closed).
    #[tokio::test]
    async fn w_deny_straddle_entry_inserted_between_registration_and_check_is_caught() {
        use buzz_auth::{IssuerCapacity, NipFiDenyMap, VerifiedAssertion};
        use chrono::{Duration, Utc};
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::{mpsc, RwLock};
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        // Same key for assertion and NIP-42 event — pairing passes.
        let key = Keys::generate();
        let deadline = Utc::now() + Duration::hours(1);
        // `for_test` produces issuer = "test-issuer".
        let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);

        let challenge = "w-deny-straddle-challenge".to_string();
        let (send_tx, mut send_rx) = mpsc::channel::<WsMessage>(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, mut terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);

        let cancel = CancellationToken::new();
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());

        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::nil());

        let conn = Arc::new(crate::connection::ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(community, "test.local".to_string()),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(AuthState::Pending {
                challenge: challenge.clone(),
            }),
            subscriptions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: Some(assertion),
            session_deadline: Some(deadline),
            nip_fi_gate: gate,
        });

        // Real DB required (ban-check is fail-closed; lazy pool denies before hook).
        let mut state = match auth_test_state_real_db().await {
            Some(s) => Arc::try_unwrap(s).unwrap_or_else(|arc| (*arc).clone()),
            None => {
                eprintln!(
                    "W_deny_straddle: skipping — local DB not available at \
                     postgres://buzz:buzz_dev@127.0.0.1:5432/buzz"
                );
                return;
            }
        };

        // Wire an empty deny map for issuer "test-issuer" (the issuer used by
        // VerifiedAssertion::for_test). No entries yet — the key is clean.
        let deny_map = Arc::new(NipFiDenyMap::new(
            16,
            vec![IssuerCapacity {
                issuer: "test-issuer".to_owned(),
                capacity: 16,
            }],
        ));
        // Retain a handle so we can insert the entry during the hook window.
        let deny_map_for_insert = Arc::clone(&deny_map);
        state.nip_fi_deny_map = Some(deny_map);
        let state = Arc::new(state);

        let relay_url = "ws://test.local";
        let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
            .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
            .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
            .sign_with_keys(&key)
            .unwrap();

        // Arm the barrier: fires when handle_auth reaches before_deny_set_check.
        let (arrived_rx, release) = crate::nip_fi_test_hooks::deny_set_check_hook::arm(community);

        // Spawn handle_auth — it will stall at the hook after registration.
        let conn2 = Arc::clone(&conn);
        let state2 = Arc::clone(&state);
        let handle = tokio::spawn(async move { handle_auth(auth_event, conn2, state2).await });

        // Wait for the handler to reach the deny-check seam.
        tokio::time::timeout(std::time::Duration::from_secs(5), arrived_rx)
            .await
            .expect("W_deny_straddle: handler must reach before_deny_set_check within 5s")
            .expect("arrived channel closed");

        // Handler is now AFTER set_authenticated_pubkey (registered) and BEFORE
        // the deny check. Insert the deny entry into the live map.
        let until = Utc::now() + Duration::seconds(3600);
        let merge = deny_map_for_insert.merge_cross_pod_deny(
            "test-issuer",
            &key.public_key(),
            until,
            Utc::now(),
        );
        assert!(
            matches!(merge, buzz_auth::CrossPodMergeResult::Merged),
            "W_deny_straddle: deny entry must be inserted during the hook window"
        );

        // Release the hook — handler resumes and calls is_denied().
        release.notify_one();

        // Wait for handle_auth to return.
        tokio::time::timeout(std::time::Duration::from_secs(5), handle)
            .await
            .expect("W_deny_straddle: handle_auth must return within 5s after hook release")
            .expect("handle_auth task must not panic");

        // The connection must be cancelled — the deny check closed it.
        assert!(
            cancel.is_cancelled(),
            "W_deny_straddle: connection must be cancelled after deny-set hit \
             (entry inserted between registration and check)"
        );

        // The denial frame must be on the ctrl channel (authorization_denied).
        let ctrl_frame = ctrl_rx
            .try_recv()
            .expect("W_deny_straddle: ctrl channel must contain the denial frame");
        // The frame is the NIP-FI denial (not a NOTICE; it's a JSON control frame).
        match ctrl_frame {
            WsMessage::Text(t) => {
                assert!(
                    t.contains("authorization denied") || t.contains("\"restricted\""),
                    "W_deny_straddle: ctrl frame must be the authorization_denied frame; got: {t}"
                );
            }
            WsMessage::Binary(_) => {} // binary close frame is also acceptable
            other => panic!("W_deny_straddle: ctrl frame must be Text or Binary; got {other:?}"),
        }

        // No OK(true) on the data channel.
        while let Ok(frame) = send_rx.try_recv() {
            if let WsMessage::Text(t) = &frame {
                assert!(
                    !t.contains("\"true\"") && !t.contains(r#"[true"#),
                    "W_deny_straddle: no OK(true) must be sent when deny-set catches \
                     the entry inserted between registration and check; got: {t}"
                );
            }
        }

        // No terminal frame (denial goes to ctrl, not terminal).
        assert!(
            terminal_ctrl_rx.try_recv().is_err(),
            "W_deny_straddle: terminal channel must be empty (deny-set denial \
             uses ctrl channel, not terminal)"
        );
    }
}
