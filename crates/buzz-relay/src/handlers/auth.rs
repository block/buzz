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
use crate::metrics::{AuthOutcome, AuthPostTerminalState};
use crate::protocol::RelayMessage;
use crate::state::AppState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BanOutcome {
    Clear,
    Banned,
    DbError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PolicyCheck<T> {
    Allowed(T),
    Denied,
    DependencyError,
}

fn classify_allowlist<E>(result: Result<bool, E>) -> PolicyCheck<()> {
    match result {
        Ok(true) => PolicyCheck::Allowed(()),
        Ok(false) => PolicyCheck::Denied,
        Err(_) => PolicyCheck::DependencyError,
    }
}

fn classify_relay_membership(
    result: Result<crate::api::relay_members::MembershipDecision, String>,
) -> PolicyCheck<Option<nostr::PublicKey>> {
    use crate::api::relay_members::MembershipDecision;

    match result {
        Ok(MembershipDecision::OpenRelay | MembershipDecision::Member) => {
            PolicyCheck::Allowed(None)
        }
        Ok(MembershipDecision::ViaOwner(owner)) => PolicyCheck::Allowed(Some(owner)),
        Ok(MembershipDecision::Denied) => PolicyCheck::Denied,
        Err(_) => PolicyCheck::DependencyError,
    }
}

fn ban_denial(outcome: BanOutcome) -> Option<(&'static str, &'static str, AuthOutcome)> {
    match outcome {
        BanOutcome::Clear => None,
        BanOutcome::Banned => Some((
            "banned",
            "blocked: you are banned from this community",
            AuthOutcome::Banned,
        )),
        BanOutcome::DbError => Some((
            "ban_check_error",
            "error: internal error checking restriction state",
            AuthOutcome::BanCheckError,
        )),
    }
}

/// NIP-FI post-upgrade AUTH denial: queue the canonical Root NOTICE for
/// `class` on the terminal channel, then close. Callers invoke this only when
/// `conn.nip_fi_assertion` is present, so every FI denial is uniform in frame
/// type, body, and close behaviour. [FI-TRACE-DENIAL-ORACLE]
fn deny_nip_fi_auth(conn: &ConnectionState, class: buzz_auth::DenialClass) {
    let _ = conn
        .terminal_ctrl_tx
        .try_send(crate::nip_fi_session::root_denial_frame(class));
    conn.cancel.cancel();
}

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
        match conn.auth_state_snapshot() {
            AuthState::Pending { challenge, .. } => (challenge, conn.conn_id),
            AuthState::Authenticated(_) => {
                debug!(conn_id = %conn.conn_id, "AUTH received but already authenticated");
                crate::metrics::record_post_terminal_auth_frame(
                    AuthPostTerminalState::Authenticated,
                );
                conn.send(RelayMessage::ok(
                    &event_id_hex,
                    false,
                    "auth-required: already authenticated",
                ));
                return;
            }
            AuthState::Failed => {
                debug!(conn_id = %conn.conn_id, "AUTH received after failed auth");
                crate::metrics::record_post_terminal_auth_frame(AuthPostTerminalState::Failed);
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

                if let Some((metric_reason, deny_reason, auth_outcome)) = ban_denial(outcome) {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), reason = deny_reason, "principal denied at ban seam");
                    metrics::counter!("buzz_auth_failures_total", "reason" => metric_reason)
                        .increment(1);
                    if !conn.reject_auth(auth_outcome) {
                        return;
                    }
                    // Decision 4: banned ⇒ OK false + immediate WebSocket close.
                    // Route the reason frame on the control channel (not `send`,
                    // which uses the data channel and would race the cancel), so
                    // the send loop drains it ahead of the Close it emits on
                    // cancel. Then cancel to close the socket immediately.
                    //
                    // With an FI assertion, NIP-FI requires the canonical
                    // NOTICE instead: a ban is `authorization denied`, a failed
                    // lookup is `authorization unavailable`.
                    if conn.nip_fi_assertion.is_some() {
                        let class = match outcome {
                            BanOutcome::DbError => buzz_auth::DenialClass::AuthorizationUnavailable,
                            _ => buzz_auth::DenialClass::AuthorizationDenied,
                        };
                        deny_nip_fi_auth(&conn, class);
                        return;
                    }
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
                let allowlist = state
                    .db
                    .is_pubkey_allowed(conn.tenant.community(), pubkey.as_bytes())
                    .await;
                if let Err(e) = &allowlist {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e,
                              "allowlist DB lookup failed, denying (fail-closed)");
                }
                match classify_allowlist(allowlist) {
                    PolicyCheck::Allowed(()) => {}
                    PolicyCheck::Denied => {
                        warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "pubkey not in allowlist");
                        metrics::counter!("buzz_auth_failures_total", "reason" => "allowlist_denied")
                            .increment(1);
                        if !conn.reject_auth(AuthOutcome::AllowlistDenied) {
                            return;
                        }
                        // Fix 4a: when an FI assertion is present, use the uniform
                        // canonical NIP-FI denial frame (NOTICE, not OK) so the
                        // frame type and body are byte-identical to expiry and
                        // pairing-mismatch denials — allowlist status is not
                        // distinguishable. [FI-TRACE-DENIAL-ORACLE]
                        if conn.nip_fi_assertion.is_some() {
                            deny_nip_fi_auth(&conn, buzz_auth::DenialClass::AuthorizationDenied);
                        } else {
                            conn.send(RelayMessage::ok(
                                &event_id_hex,
                                false,
                                "auth-required: verification failed",
                            ));
                        }
                        return;
                    }
                    PolicyCheck::DependencyError => {
                        metrics::counter!("buzz_auth_failures_total", "reason" => "allowlist_check_error")
                            .increment(1);
                        if !conn.reject_auth(AuthOutcome::AllowlistCheckError) {
                            return;
                        }
                        if conn.nip_fi_assertion.is_some() {
                            deny_nip_fi_auth(
                                &conn,
                                buzz_auth::DenialClass::AuthorizationUnavailable,
                            );
                            return;
                        }
                        conn.send(RelayMessage::ok(
                            &event_id_hex,
                            false,
                            "error: internal error checking allowlist",
                        ));
                        return;
                    }
                }
            }

            // Relay membership gate — uses the shared helper with NIP-OA fallback.
            let membership = crate::api::relay_members::check_relay_membership(
                &state,
                conn.tenant.community(),
                pubkey.as_bytes(),
                auth_tag_json.as_deref(),
                Some(signed_auth_created_at),
            )
            .await;
            if let Err(e) = &membership {
                warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), error = %e,
                    "relay membership DB lookup failed, denying (fail-closed)");
            }
            let nip_oa_owner = match classify_relay_membership(membership) {
                PolicyCheck::Allowed(owner) => owner,
                PolicyCheck::Denied => {
                    warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "not a relay member");
                    metrics::counter!("buzz_auth_failures_total", "reason" => "not_relay_member")
                        .increment(1);
                    if !conn.reject_auth(AuthOutcome::NotRelayMember) {
                        return;
                    }
                    // With an FI assertion, membership status must not be
                    // distinguishable from a ban or allowlist denial.
                    if conn.nip_fi_assertion.is_some() {
                        deny_nip_fi_auth(&conn, buzz_auth::DenialClass::AuthorizationDenied);
                        return;
                    }
                    conn.send(RelayMessage::ok(
                        &event_id_hex,
                        false,
                        "restricted: not a relay member",
                    ));
                    return;
                }
                PolicyCheck::DependencyError => {
                    metrics::counter!("buzz_auth_failures_total", "reason" => "relay_membership_check_error")
                        .increment(1);
                    if !conn.reject_auth(AuthOutcome::RelayMembershipCheckError) {
                        return;
                    }
                    if conn.nip_fi_assertion.is_some() {
                        deny_nip_fi_auth(&conn, buzz_auth::DenialClass::AuthorizationUnavailable);
                        return;
                    }
                    conn.send(RelayMessage::ok(
                        &event_id_hex,
                        false,
                        "error: internal error checking relay membership",
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
            if !conn.authenticate(auth_ctx) {
                return;
            }
            // The permit is held through set_authenticated_pubkey and the OK send
            // so the entire auth commit is atomic with respect to expiry.
            state
                .conn_manager
                .set_authenticated_pubkey(conn_id, pubkey.to_bytes().to_vec());
            conn.send(RelayMessage::ok(&event_id_hex, true, ""));
            // _auth_permit drops here — expiry's write guard may proceed.
        }
        Err(e) => {
            warn!(conn_id = %conn_id, error = %e, "NIP-42 auth failed");
            metrics::counter!("buzz_auth_failures_total", "reason" => "nip42_invalid").increment(1);
            if !conn.reject_auth(AuthOutcome::Invalid) {
                return;
            }
            // With an FI assertion: a bad proof is client evidence
            // (`evidence rejected`); only a relay-internal verifier failure
            // is `authorization unavailable`.
            if conn.nip_fi_assertion.is_some() {
                let class = match e {
                    buzz_auth::AuthError::Internal(_) => {
                        buzz_auth::DenialClass::AuthorizationUnavailable
                    }
                    _ => buzz_auth::DenialClass::EvidenceRejected,
                };
                deny_nip_fi_auth(&conn, class);
                return;
            }
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
    use super::{
        ban_denial, classify_allowlist, classify_relay_membership, extract_auth_tag_json,
        handle_auth, BanOutcome, PolicyCheck,
    };
    use crate::api::relay_members::MembershipDecision;
    use crate::connection::{tests::test_conn_with_auth, AuthState};
    use crate::metrics::{AuthOutcome, AuthPostTerminalState};
    use axum::extract::ws::Message as WsMessage;
    use metrics_util::debugging::DebugValue;
    use nostr::{EventBuilder, Keys, Kind, RelayUrl, Tag};
    use std::time::Instant;

    type MetricSnapshot = Vec<(
        metrics_util::CompositeKey,
        Option<metrics::Unit>,
        Option<metrics::SharedString>,
        DebugValue,
    )>;

    fn metric_counter(snapshot: &MetricSnapshot, name: &str, outcome: Option<&str>) -> u64 {
        snapshot
            .iter()
            .find_map(|(key, _, _, value)| {
                if key.key().name() != name {
                    return None;
                }
                let labels = key.key().labels().collect::<Vec<_>>();
                if outcome.is_some_and(|expected| {
                    !labels
                        .iter()
                        .any(|label| label.key() == "outcome" && label.value() == expected)
                }) {
                    return None;
                }
                let DebugValue::Counter(value) = value else {
                    panic!("{name} must be a counter");
                };
                Some(*value)
            })
            .unwrap_or_default()
    }

    fn pending(challenge: &str) -> AuthState {
        AuthState::Pending {
            challenge: challenge.to_owned(),
            started_at: Instant::now(),
        }
    }

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
        // Fix 5: use Config::for_test() which holds NIP_FI_ENV_LOCK internally. [FI-TRACE-ENV-RACE]
        let mut config = crate::config::Config::for_test();
        config.require_relay_membership = false;
        config.database_url = "postgres://buzz:buzz_dev@127.0.0.1:1/buzz".to_string();
        config.redis_url = "redis://127.0.0.1:1".to_string();
        // 100ms acquire timeout: a request that falls through to the stub
        // pool still waits, but for 100ms instead of sqlx's 30s default.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(100))
            .connect_lazy(&config.database_url)
            .expect("lazy pg pool");
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

    #[tokio::test]
    async fn handle_auth_pairing_mismatch_runs_full_root_denial_path() {
        use buzz_auth::VerifiedAssertion;
        use chrono::{Duration, Utc};
        use std::collections::HashMap;
        use std::sync::Arc;
        use tokio::sync::mpsc;
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
            auth_state: std::sync::Mutex::new(AuthState::Pending {
                challenge: challenge.clone(),
                started_at: Instant::now(),
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
            matches!(conn.auth_state_snapshot(), AuthState::Failed),
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
        use tokio::sync::mpsc;
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
            auth_state: std::sync::Mutex::new(AuthState::Pending {
                challenge: challenge.clone(),
                started_at: Instant::now(),
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
            !matches!(conn.auth_state_snapshot(), AuthState::Authenticated(_)),
            "B2: a pre-cancelled connection must never reach AuthState::Authenticated"
        );
    }

    // ── FI denial-invariant witnesses: shared harness ─────────────────────────
    //
    // Every post-upgrade AUTH denial with an FI assertion must queue exactly one
    // canonical Root NOTICE on the terminal channel, put nothing on the data or
    // ordinary ctrl channel, and cancel. Without an assertion, the legacy
    // `OK false` reply must be unchanged. [FI-TRACE-DENIAL-ORACLE]

    struct AuthHarness {
        conn: std::sync::Arc<crate::connection::ConnectionState>,
        key: Keys,
        challenge: String,
        send_rx: tokio::sync::mpsc::Receiver<WsMessage>,
        ctrl_rx: tokio::sync::mpsc::Receiver<WsMessage>,
        terminal_rx: tokio::sync::mpsc::Receiver<WsMessage>,
    }

    impl AuthHarness {
        /// A pending connection whose FI assertion (if any) names `key`, so
        /// pairing always passes and a later gate is the one that denies.
        fn new(with_fi_assertion: bool) -> Self {
            use tokio::sync::mpsc;
            let key = Keys::generate();
            let challenge = format!("fi-invariant-{}", uuid::Uuid::new_v4());
            let (send_tx, send_rx) = mpsc::channel(8);
            let (ctrl_tx, ctrl_rx) = mpsc::channel(8);
            let (terminal_ctrl_tx, terminal_rx) = mpsc::channel(1);
            let cancel = tokio_util::sync::CancellationToken::new();
            let assertion = with_fi_assertion.then(|| {
                buzz_auth::VerifiedAssertion::for_test(
                    Some(key.public_key()),
                    vec![chrono::Utc::now() + chrono::Duration::hours(1)],
                )
            });
            let conn = std::sync::Arc::new(crate::connection::ConnectionState {
                conn_id: uuid::Uuid::new_v4(),
                tenant: buzz_core::tenant::TenantContext::resolved(
                    buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::new_v4()),
                    "test.local".to_string(),
                ),
                remote_addr: "127.0.0.1:1234".parse().unwrap(),
                auth_state: std::sync::Mutex::new(pending(&challenge)),
                subscriptions: Default::default(),
                send_tx,
                ctrl_tx,
                terminal_ctrl_tx,
                cancel: cancel.clone(),
                backpressure_count: Default::default(),
                grace_limit: 3,
                nip_fi_assertion: assertion,
                session_deadline: None,
                nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel),
            });
            Self {
                conn,
                key,
                challenge,
                send_rx,
                ctrl_rx,
                terminal_rx,
            }
        }

        fn auth_event(&self) -> nostr::Event {
            EventBuilder::new(Kind::Authentication, "")
                .tag(Tag::parse(["relay", "ws://test.local"]).unwrap())
                .tag(Tag::parse(["challenge", &self.challenge]).unwrap())
                .sign_with_keys(&self.key)
                .unwrap()
        }

        async fn run(&self, event: nostr::Event, state: std::sync::Arc<crate::state::AppState>) {
            handle_auth(event, std::sync::Arc::clone(&self.conn), state).await;
        }

        fn assert_fi_terminal(mut self, class: buzz_auth::DenialClass) {
            assert_eq!(
                self.terminal_rx.try_recv().expect("terminal denial frame"),
                crate::nip_fi_session::root_denial_frame(class),
            );
            assert!(
                self.terminal_rx.try_recv().is_err(),
                "exactly one terminal frame"
            );
            assert!(
                self.send_rx.try_recv().is_err(),
                "no denial on the data channel"
            );
            assert!(
                self.ctrl_rx.try_recv().is_err(),
                "no denial on the ctrl channel"
            );
            assert!(
                self.conn.cancel.is_cancelled(),
                "FI denial must close the socket"
            );
            assert!(matches!(self.conn.auth_state_snapshot(), AuthState::Failed));
        }

        fn assert_off_mode_ok(mut self, reason: &str) {
            let frame = match self
                .send_rx
                .try_recv()
                .expect("off-mode OK on data channel")
            {
                WsMessage::Text(text) => serde_json::from_str::<serde_json::Value>(&text).unwrap(),
                other => panic!("expected text frame, got {other:?}"),
            };
            assert_eq!(frame[0], "OK");
            assert_eq!(frame[2], false);
            assert_eq!(frame[3], reason);
            assert!(
                self.terminal_rx.try_recv().is_err(),
                "off mode never uses terminal"
            );
            assert!(
                !self.conn.cancel.is_cancelled(),
                "off mode keeps the socket open"
            );
        }
    }

    async fn state_with_pool(
        pool: sqlx::PgPool,
        configure: impl FnOnce(&mut crate::config::Config),
    ) -> std::sync::Arc<crate::state::AppState> {
        use std::sync::Arc;
        let mut config = crate::config::Config::for_test();
        config.require_relay_membership = false;
        config.redis_url = "redis://127.0.0.1:1".to_string();
        configure(&mut config);
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
        let db = buzz_db::Db::from_pool(pool);
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

    /// NIP-42 `Err` arm: an invalid proof under FI is `evidence rejected`.
    #[tokio::test]
    async fn fi_invalid_nip42_proof_emits_terminal_evidence_rejected() {
        let state = auth_test_state().await;
        for with_fi in [true, false] {
            let harness = AuthHarness::new(with_fi);
            let mut event = harness.auth_event();
            event.content.push('x'); // breaks the id/signature
            harness.run(event, state.clone()).await;
            if with_fi {
                harness.assert_fi_terminal(buzz_auth::DenialClass::EvidenceRejected);
            } else {
                harness.assert_off_mode_ok("auth-required: verification failed");
            }
        }
    }

    /// Ban-lookup failure under FI is `authorization unavailable`, not denied.
    /// (Off mode keeps the ctrl-channel OK + close, covered by
    /// `handler_accounts_ban_check_database_error`.)
    #[tokio::test]
    async fn fi_ban_check_error_emits_terminal_authorization_unavailable() {
        let state = auth_test_state().await; // unreachable DB: ban lookup errors
        let harness = AuthHarness::new(true);
        harness.run(harness.auth_event(), state).await;
        harness.assert_fi_terminal(buzz_auth::DenialClass::AuthorizationUnavailable);
    }

    // ── W1 (auth barrier): expiry fired mid-flight blocks AUTH commit ─────────
    //
    // This test requires a real PostgreSQL instance. It lives in `postgres_tests`
    // and is gated with `#[ignore]` so it does not run in unit-test mode where no
    // DB is available. The postgres-ci nextest lane discovers it via the `ignore`
    // attribute — do not remove the ignore even if a local DB is reachable.
    // [Fix 8: FI-TRACE-ISOLATED-DB]
    mod postgres_tests {
        use super::*;

        async fn auth_test_state_real_db_expect() -> std::sync::Arc<crate::state::AppState> {
            use std::sync::Arc;
            let db_url = crate::test_support::database_url();
            // Fail hard on infrastructure errors — the postgres lane guarantees a DB.
            let pool = sqlx::PgPool::connect(&db_url)
                .await
                .expect("W1: PostgreSQL must be available — set BUZZ_TEST_DATABASE_URL");
            // Fix 5: use Config::for_test() which holds NIP_FI_ENV_LOCK internally. [FI-TRACE-ENV-RACE]
            let mut config = crate::config::Config::for_test();
            config.require_relay_membership = false;
            config.database_url = db_url.clone();
            config.redis_url = "redis://127.0.0.1:1".to_string();
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
            let media_storage =
                buzz_media::MediaStorage::new(&config.media).expect("media storage");
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

        /// W1 (auth barrier): expiry fired mid-flight blocks AUTH commit.
        ///
        /// Arms `before_auth_commit` — the hook immediately before `acquire_effect()`
        /// in the AUTH commit path. Dispatches `handle_auth` with a live (not-yet-
        /// expired) gate, waits for the hook to signal the handler reached the
        /// permit boundary, fires the gate expiry (cancel), then releases the hook.
        /// The handler tries `acquire_effect()` and gets `SessionExpired`, returns
        /// without committing `AuthState::Authenticated`.
        ///
        /// This is the real barrier test Paul requires: the handler runs through
        /// NIP-42 verification, pairing check, ban check, allowlist, and membership
        /// gates, then stalls at `before_auth_commit`. Expiry fires *in that async
        /// gap*. The permit acquisition fails, and no auth commit occurs.
        ///
        /// Hook location: `handlers/auth.rs`, immediately before `acquire_effect()`
        /// at the B2 AUTH commit seam.
        ///
        /// Mutation evidence:
        ///   A) Delete `#[cfg(test)] before_auth_commit(...)` from auth.rs → handler
        ///      never stalls at the hook → cancel fires before handler reaches
        ///      acquire_effect → handler completes auth before cancel is checked
        ///      (race) OR the gate denies anyway on cancel check. The test is
        ///      non-deterministic without the hook; WITH the hook the barrier is exact.
        ///   B) Remove `acquire_effect()` from auth.rs → handler commits
        ///      AuthState::Authenticated despite the cancel → assertion panics.
        ///   C) Change gate from deadline-with-cancel to off_mode → acquire_effect
        ///      succeeds even after cancel → handler commits auth → assertion panics.
        ///
        /// Requires a real DB (ban-check is fail-closed; lazy pool errors → deny
        /// before hook). DB call returns "not banned" for an unknown
        /// community/pubkey — a real result, not mocked.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn w1_auth_barrier_expiry_mid_flight_blocks_auth_commit() {
            use buzz_auth::VerifiedAssertion;
            use chrono::{Duration, Utc};
            use std::collections::HashMap;
            use std::sync::Arc;
            use tokio::sync::mpsc;
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
                tenant: buzz_core::tenant::TenantContext::resolved(
                    community,
                    "test.local".to_string(),
                ),
                remote_addr: "127.0.0.1:1234".parse().unwrap(),
                auth_state: std::sync::Mutex::new(AuthState::Pending {
                    challenge: challenge.clone(),
                    started_at: Instant::now(),
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

            let state = auth_test_state_real_db_expect().await;
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
                !matches!(conn.auth_state_snapshot(), AuthState::Authenticated(_)),
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

        /// Fix 4a witness: root allowlist denial with FI assertion emits
        /// `restricted: authorization denied` — not `auth-required: verification
        /// failed` — so the allowlist gate is not distinguishable from other
        /// local-policy denials when enforcement is active.
        ///
        /// Requires a real DB so `is_pubkey_allowed` can return `Ok(false)` for a
        /// key not in the allowlist. The community is freshly created so the key
        /// has never been allowlisted.
        ///
        /// Mutation oracle:
        ///   A) Remove the `if conn.nip_fi_assertion.is_some()` branch in the
        ///      allowlist denied arm → reply text is `auth-required: verification
        ///      failed` → assertion panics.
        ///   B) Change the FI-mode reply to any text other than `restricted:
        ///      authorization denied` → assertion panics.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn fix_4a_allowlist_denial_with_fi_assertion_emits_canonical_restricted_frame() {
            use buzz_auth::VerifiedAssertion;
            use chrono::{Duration, Utc};
            use std::collections::HashMap;
            use std::sync::Arc;
            use tokio::sync::mpsc;
            use tokio_util::sync::CancellationToken;
            use uuid::Uuid;

            // Build state with pubkey allowlist enabled + real DB.
            let db_url = crate::test_support::database_url();
            let pool = sqlx::PgPool::connect(&db_url)
                .await
                .expect("Fix4a: PostgreSQL must be available");
            // Fix 5: use Config::for_test() which holds NIP_FI_ENV_LOCK internally. [FI-TRACE-ENV-RACE]
            let mut config = crate::config::Config::for_test();
            config.require_relay_membership = false;
            config.pubkey_allowlist_enabled = true;
            config.database_url = db_url.clone();
            config.redis_url = "redis://127.0.0.1:1".to_string();
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
            let media_storage =
                buzz_media::MediaStorage::new(&config.media).expect("media storage");
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
            let state = Arc::new(state);

            // A matching key — pairing passes; the allowlist gate is the one that denies.
            let key = Keys::generate();
            let assertion = VerifiedAssertion::for_test(
                Some(key.public_key()),
                vec![Utc::now() + Duration::hours(1)],
            );

            let challenge = "fix-4a-allowlist-challenge".to_string();
            let (send_tx, mut send_rx) = mpsc::channel::<WsMessage>(8);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel::<WsMessage>(8);
            let (terminal_ctrl_tx, mut terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
            let cancel = CancellationToken::new();

            let conn = Arc::new(crate::connection::ConnectionState {
                conn_id: Uuid::new_v4(),
                tenant: buzz_core::tenant::TenantContext::resolved(
                    buzz_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                    "test.local".to_string(),
                ),
                remote_addr: "127.0.0.1:1234".parse().unwrap(),
                auth_state: std::sync::Mutex::new(AuthState::Pending {
                    challenge: challenge.clone(),
                    started_at: Instant::now(),
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

            let relay_url = "ws://test.local";
            let auth_event = EventBuilder::new(Kind::Authentication, "")
                .tag(Tag::parse(["relay", relay_url]).unwrap())
                .tag(Tag::parse(["challenge", &challenge]).unwrap())
                .sign_with_keys(&key)
                .unwrap();
            handle_auth(auth_event, Arc::clone(&conn), state).await;

            // Fix 4a: FI-present allowlist denial must use the canonical
            // NOTICE frame (byte-identical to expiry/pairing-mismatch denials),
            // NOT an OK envelope. The denial arrives on terminal_ctrl_tx.
            // The cancel token must be triggered (connection terminates).
            let expected_frame = crate::nip_fi_session::authorization_denied_frame(
                crate::nip_fi_session::NipFiWsRoute::Root,
            );
            // Ordinary channel must NOT contain the denial (no OK fallthrough).
            while let Ok(frame) = send_rx.try_recv() {
                if let WsMessage::Text(t) = &frame {
                    assert!(
                        !t.contains("restricted: authorization denied"),
                        "Fix 4a: FI allowlist denial must NOT reach ordinary send channel; got: {t}"
                    );
                    assert!(
                        !t.contains("auth-required: verification failed"),
                        "Fix 4a: non-FI text must not appear with FI assertion; got: {t}"
                    );
                }
            }
            // Terminal channel must contain the exact canonical frame.
            let terminal_frame = terminal_ctrl_rx
                .try_recv()
                .expect("Fix 4a: canonical denial must be on terminal_ctrl_rx");
            assert_eq!(
                terminal_frame, expected_frame,
                "Fix 4a: terminal frame must be the exact canonical authorization_denied_frame"
            );
            // Cancel must have fired — connection terminates.
            assert!(
                cancel.is_cancelled(),
                "Fix 4a: FI allowlist denial must cancel the connection token"
            );
        }
        /// A pool whose `search_path` is a fresh schema holding only
        /// `community_bans`: the ban gate succeeds, every later policy table
        /// is missing, so the next lookup fails as a dependency error.
        async fn ban_only_schema_pool() -> (sqlx::PgPool, sqlx::PgPool, String) {
            use sqlx::postgres::PgConnectOptions;
            let db_url = crate::test_support::database_url();
            let admin = sqlx::PgPool::connect(&db_url)
                .await
                .expect("PostgreSQL must be available");
            let schema = format!("fi_dep_{}", uuid::Uuid::new_v4().simple());
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "CREATE SCHEMA {schema}; \
                 CREATE TABLE {schema}.community_bans (LIKE public.community_bans INCLUDING ALL);"
            )))
            .execute(&admin)
            .await
            .expect("create ban-only schema");
            let options = db_url
                .parse::<PgConnectOptions>()
                .expect("database url")
                .options([("search_path", schema.as_str())]);
            let pool = sqlx::PgPool::connect_with(options)
                .await
                .expect("ban-only pool");
            (admin, pool, schema)
        }

        async fn drop_schema(admin: &sqlx::PgPool, schema: &str) {
            let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
                .execute(admin)
                .await;
        }

        /// Carl's P2: a matching-key non-member that passes the ban and
        /// allowlist gates gets the terminal `authorization denied` NOTICE and
        /// the socket closes; off mode keeps `OK false "not a relay member"`.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn fi_relay_membership_denial_emits_terminal_authorization_denied() {
            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .expect("PostgreSQL must be available");
            let state = state_with_pool(pool, |c| {
                c.require_relay_membership = true;
                c.pubkey_allowlist_enabled = true;
            })
            .await;
            for with_fi in [true, false] {
                let harness = AuthHarness::new(with_fi);
                let community = *harness.conn.tenant.community().as_uuid();
                sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                    .bind(community)
                    .bind(format!("fi-membership-{}.example", community.simple()))
                    .execute(state.db.pool())
                    .await
                    .expect("insert community");
                // Pass the allowlist so relay membership is the denying gate.
                sqlx::query("INSERT INTO pubkey_allowlist (community_id, pubkey) VALUES ($1, $2)")
                    .bind(harness.conn.tenant.community().as_uuid())
                    .bind(harness.key.public_key().to_bytes().to_vec())
                    .execute(state.db.pool())
                    .await
                    .expect("allowlist key");
                harness.run(harness.auth_event(), state.clone()).await;
                if with_fi {
                    harness.assert_fi_terminal(buzz_auth::DenialClass::AuthorizationDenied);
                } else {
                    harness.assert_off_mode_ok("restricted: not a relay member");
                }
            }
        }

        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn fi_allowlist_check_error_emits_terminal_authorization_unavailable() {
            let (admin, pool, schema) = ban_only_schema_pool().await;
            let state = state_with_pool(pool, |c| c.pubkey_allowlist_enabled = true).await;
            for with_fi in [true, false] {
                let harness = AuthHarness::new(with_fi);
                harness.run(harness.auth_event(), state.clone()).await;
                if with_fi {
                    harness.assert_fi_terminal(buzz_auth::DenialClass::AuthorizationUnavailable);
                } else {
                    harness.assert_off_mode_ok("error: internal error checking allowlist");
                }
            }
            drop_schema(&admin, &schema).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn fi_relay_membership_check_error_emits_terminal_authorization_unavailable() {
            let (admin, pool, schema) = ban_only_schema_pool().await;
            let state = state_with_pool(pool, |c| c.require_relay_membership = true).await;
            for with_fi in [true, false] {
                let harness = AuthHarness::new(with_fi);
                harness.run(harness.auth_event(), state.clone()).await;
                if with_fi {
                    harness.assert_fi_terminal(buzz_auth::DenialClass::AuthorizationUnavailable);
                } else {
                    harness.assert_off_mode_ok("error: internal error checking relay membership");
                }
            }
            drop_schema(&admin, &schema).await;
        }
    }

    #[test]
    fn ban_decisions_map_to_bounded_public_outcomes() {
        assert_eq!(ban_denial(BanOutcome::Clear), None);
        assert_eq!(
            ban_denial(BanOutcome::Banned),
            Some((
                "banned",
                "blocked: you are banned from this community",
                AuthOutcome::Banned,
            ))
        );
        assert_eq!(
            ban_denial(BanOutcome::DbError),
            Some((
                "ban_check_error",
                "error: internal error checking restriction state",
                AuthOutcome::BanCheckError,
            ))
        );
    }

    #[test]
    fn dependency_failures_are_distinct_from_policy_denials() {
        assert_eq!(
            classify_allowlist(Ok::<_, &str>(true)),
            PolicyCheck::Allowed(())
        );
        assert_eq!(
            classify_allowlist(Ok::<_, &str>(false)),
            PolicyCheck::Denied
        );
        assert_eq!(
            classify_allowlist(Err::<bool, _>("database unavailable")),
            PolicyCheck::DependencyError
        );

        let owner = Keys::generate().public_key();
        assert_eq!(
            classify_relay_membership(Ok(MembershipDecision::OpenRelay)),
            PolicyCheck::Allowed(None)
        );
        assert_eq!(
            classify_relay_membership(Ok(MembershipDecision::Member)),
            PolicyCheck::Allowed(None)
        );
        assert_eq!(
            classify_relay_membership(Ok(MembershipDecision::ViaOwner(owner))),
            PolicyCheck::Allowed(Some(owner))
        );
        assert_eq!(
            classify_relay_membership(Ok(MembershipDecision::Denied)),
            PolicyCheck::Denied
        );
        assert_eq!(
            classify_relay_membership(Err("database unavailable".to_owned())),
            PolicyCheck::DependencyError
        );
    }

    /// The handler owns retry classification, so drive its real terminal-state
    /// branches rather than calling the metric helper directly. A malformed
    /// signature also traverses the real NIP-42 verifier before terminalizing.
    #[tokio::test(flavor = "current_thread")]
    async fn handler_separates_post_terminal_frames_from_invalid_attempts() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _recorder_guard = metrics::set_default_local_recorder(&recorder);
        let state = crate::state::tests::test_state().await;

        let (authenticated, mut authenticated_rx) =
            test_conn_with_auth(crate::connection::tests::authenticated_state());
        let duplicate = signed_event_with_tags(Vec::new());
        handle_auth(duplicate, authenticated, state.clone()).await;
        let duplicate_frame = crate::connection::tests::read_frame(&mut authenticated_rx);
        assert_eq!(duplicate_frame[2], false);
        assert_eq!(duplicate_frame[3], "auth-required: already authenticated");

        let (failed, mut failed_rx) = test_conn_with_auth(AuthState::Failed);
        let after_failure = signed_event_with_tags(Vec::new());
        handle_auth(after_failure, failed, state.clone()).await;
        let failed_frame = crate::connection::tests::read_frame(&mut failed_rx);
        assert_eq!(failed_frame[2], false);
        assert_eq!(
            failed_frame[3],
            "auth-required: authentication already failed"
        );

        let challenge = "invalid-signature-challenge";
        let (invalid_conn, mut invalid_rx) = test_conn_with_auth(pending(challenge));
        crate::metrics::record_auth_attempt_started();
        let relay_url: RelayUrl = crate::api::bridge::nip42_expected_relay_url(
            &state.config.relay_url,
            &invalid_conn.tenant,
        )
        .parse()
        .expect("test relay URL");
        let mut invalid = EventBuilder::auth(challenge, relay_url)
            .sign_with_keys(&Keys::generate())
            .expect("sign auth event");
        invalid.content.push('x');
        handle_auth(invalid, invalid_conn.clone(), state).await;
        let invalid_frame = crate::connection::tests::read_frame(&mut invalid_rx);
        assert_eq!(invalid_frame[2], false);
        assert_eq!(invalid_frame[3], "auth-required: verification failed");
        assert!(matches!(
            invalid_conn.auth_state_snapshot(),
            AuthState::Failed
        ));

        let snapshot = snapshotter.snapshot().into_vec();
        let attempts = metric_counter(&snapshot, "buzz_auth_attempts_total", None);
        assert_eq!(attempts, 1);
        assert_eq!(
            metric_counter(
                &snapshot,
                "buzz_auth_outcomes_total",
                Some(AuthOutcome::Invalid.as_str()),
            ),
            1
        );
        for state in AuthPostTerminalState::ALL {
            let count = snapshot
                .iter()
                .find_map(|(key, _, _, value)| {
                    (key.key().name() == "buzz_auth_post_terminal_frames_total"
                        && key
                            .key()
                            .labels()
                            .any(|label| label.key() == "state" && label.value() == state.as_str()))
                    .then(|| match value {
                        DebugValue::Counter(value) => *value,
                        _ => panic!("post-terminal frame metric must be a counter"),
                    })
                })
                .unwrap_or_default();
            assert_eq!(count, 1, "{} post-terminal frame", state.as_str());
        }
    }

    /// A verified signature followed by an unavailable restriction database
    /// must deny fail-closed and expose a dependency error, not mislabel the
    /// principal as banned or let the attempt disappear.
    #[tokio::test(flavor = "current_thread")]
    async fn handler_accounts_ban_check_database_error() {
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _recorder_guard = metrics::set_default_local_recorder(&recorder);
        let state = crate::state::tests::test_state_with_database_url(
            "postgres://buzz:buzz_dev@127.0.0.1:1/buzz",
        )
        .await;

        let challenge = "ban-check-error-challenge";
        let (conn, _rx) = test_conn_with_auth(pending(challenge));
        crate::metrics::record_auth_attempt_started();
        let relay_url: RelayUrl =
            crate::api::bridge::nip42_expected_relay_url(&state.config.relay_url, &conn.tenant)
                .parse()
                .expect("test relay URL");
        let event = EventBuilder::auth(challenge, relay_url)
            .sign_with_keys(&Keys::generate())
            .expect("sign auth event");

        handle_auth(event, conn.clone(), state).await;

        assert!(matches!(conn.auth_state_snapshot(), AuthState::Failed));
        assert!(conn.cancel.is_cancelled());
        let snapshot = snapshotter.snapshot().into_vec();
        assert_eq!(
            metric_counter(
                &snapshot,
                "buzz_auth_outcomes_total",
                Some(AuthOutcome::BanCheckError.as_str()),
            ),
            1
        );
        assert_eq!(
            metric_counter(
                &snapshot,
                "buzz_auth_outcomes_total",
                Some(AuthOutcome::Banned.as_str()),
            ),
            0
        );
        assert_eq!(
            metric_counter(&snapshot, "buzz_auth_attempts_total", None),
            1
        );
    }
}
