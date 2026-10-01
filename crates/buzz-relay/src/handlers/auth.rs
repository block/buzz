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
pub(crate) enum BanOutcome {
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

/// Community-ban verdict for an authenticating principal: shared by every
/// socket auth seam (root and audio) so they cannot drift.
///
/// Fails closed: a DB error is `DbError`, never `Clear`. NIP-OA cascade: a ban
/// on the principal blocks it directly; if the principal is clear, a ban on
/// its proven owner (extracted from the self-proving auth tag) blocks it too.
pub(crate) async fn community_ban_outcome(
    state: &AppState,
    community: buzz_core::CommunityId,
    pubkey: nostr::PublicKey,
    auth_tag_json: Option<&str>,
    signed_auth_created_at: Option<u64>,
) -> BanOutcome {
    async fn lookup(
        state: &AppState,
        community: buzz_core::CommunityId,
        key: &nostr::PublicKey,
    ) -> BanOutcome {
        match state
            .db
            .moderation_restriction_state(community, key.as_bytes())
            .await
        {
            Ok(restriction) if restriction.banned => BanOutcome::Banned,
            Ok(_) => BanOutcome::Clear,
            Err(e) => {
                warn!(pubkey = %key.to_hex(), error = %e, "ban-state DB lookup failed, denying (fail-closed)");
                BanOutcome::DbError
            }
        }
    }
    let outcome = lookup(state, community, &pubkey).await;
    if outcome != BanOutcome::Clear {
        return outcome;
    }
    match crate::api::relay_members::extract_nip_oa_owner(
        pubkey.as_bytes(),
        auth_tag_json,
        signed_auth_created_at,
    ) {
        Some(owner) => lookup(state, community, &owner).await,
        None => BanOutcome::Clear,
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

/// Why a bound socket is refused at the final admission check.
pub(crate) struct AdmissionDenial {
    pub(crate) metric: &'static str,
    pub(crate) reason: &'static str,
    pub(crate) outcome: AuthOutcome,
    pub(crate) class: buzz_auth::DenialClass,
}

/// Final ban and relay-membership verdict for an authenticating socket.
///
/// Callers bind the socket to `pubkey` first (the registry a ban's or
/// removal's disconnect searches), then call this. Either that disconnect
/// runs after the bind and cancels the socket, or it ran before, so its
/// committed ban or removal is visible to these fresh reads. Checking before
/// binding leaves a gap where both are missed. Callers must also refuse a
/// socket whose cancellation token fired. Both reads fail closed.
pub(crate) async fn final_admission_denial(
    state: &AppState,
    community: buzz_core::CommunityId,
    pubkey: nostr::PublicKey,
    auth_tag_json: Option<&str>,
    signed_auth_created_at: Option<u64>,
) -> Option<AdmissionDenial> {
    let ban = community_ban_outcome(
        state,
        community,
        pubkey,
        auth_tag_json,
        signed_auth_created_at,
    )
    .await;
    if let Some((metric, reason, outcome)) = ban_denial(ban) {
        let class = match ban {
            BanOutcome::DbError => buzz_auth::DenialClass::AuthorizationUnavailable,
            _ => buzz_auth::DenialClass::AuthorizationDenied,
        };
        return Some(AdmissionDenial {
            metric,
            reason,
            outcome,
            class,
        });
    }
    match crate::api::relay_members::check_relay_membership_authoritative(
        state,
        community,
        pubkey.as_bytes(),
        auth_tag_json,
        signed_auth_created_at,
    )
    .await
    {
        Ok(crate::api::relay_members::MembershipDecision::Denied) => Some(AdmissionDenial {
            metric: "not_relay_member",
            reason: "restricted: not a relay member",
            outcome: AuthOutcome::NotRelayMember,
            class: buzz_auth::DenialClass::AuthorizationDenied,
        }),
        Ok(_) => None,
        Err(e) => {
            warn!(pubkey = %pubkey.to_hex(), error = %e, "relay membership recheck failed, denying (fail-closed)");
            Some(AdmissionDenial {
                metric: "relay_membership_check_error",
                reason: "error: internal error checking relay membership",
                outcome: AuthOutcome::RelayMembershipCheckError,
                class: buzz_auth::DenialClass::AuthorizationUnavailable,
            })
        }
    }
}

/// Owner to record on an admitted socket: the proven NIP-OA owner, else the
/// stored `users.agent_owner_pubkey` (first-write-wins, so fixed for the
/// socket's life). Revoking that owner then closes the socket with no
/// database read. A failed read refuses admission.
pub(crate) async fn admitted_owner(
    state: &AppState,
    community: buzz_core::CommunityId,
    pubkey: nostr::PublicKey,
    nip_oa_owner: Option<nostr::PublicKey>,
) -> Result<Option<[u8; 32]>, AdmissionDenial> {
    if let Some(owner) = nip_oa_owner {
        return Ok(Some(owner.to_bytes()));
    }
    match state
        .db
        .get_agent_channel_policy(community, pubkey.as_bytes())
        .await
    {
        Ok(row) => Ok(row
            .and_then(|(_, owner)| owner)
            .and_then(|owner| owner.try_into().ok())),
        Err(e) => {
            warn!(pubkey = %pubkey.to_hex(), error = %e, "stored agent owner lookup failed, denying");
            Err(AdmissionDenial {
                metric: "agent_owner_link_error",
                reason: OWNER_LINK_ERROR,
                outcome: AuthOutcome::RelayMembershipCheckError,
                class: buzz_auth::DenialClass::AuthorizationUnavailable,
            })
        }
    }
}

/// Refusal when a proven agent's owner link cannot be recorded. Without the
/// link, revoking the owner cannot find the agent's live sockets.
pub(crate) const OWNER_LINK_ERROR: &str = "error: internal error recording agent owner";

/// NIP-FI class for a failed NIP-42 proof: a bad proof is client evidence
/// (`evidence rejected`); only a relay-internal verifier failure is
/// `authorization unavailable`.
pub(crate) fn nip42_denial_class(error: &buzz_auth::AuthError) -> buzz_auth::DenialClass {
    match error {
        buzz_auth::AuthError::Internal(_) => buzz_auth::DenialClass::AuthorizationUnavailable,
        _ => buzz_auth::DenialClass::EvidenceRejected,
    }
}

/// NIP-FI post-upgrade AUTH denial: queue the canonical Root NOTICE for
/// `class` on the terminal channel, then close. Callers invoke this only when
/// `conn.nip_fi_assertion` is present, so every FI denial is uniform in frame
/// type, body, and close behaviour. `authorization_denied` goes through the
/// shared first-writer-wins transition, so it closes with the same 1008 as a
/// deny-set hit or admin disconnect. [FI-TRACE-DENIAL-ORACLE]
fn deny_nip_fi_auth(conn: &ConnectionState, class: buzz_auth::DenialClass) {
    if class == buzz_auth::DenialClass::AuthorizationDenied {
        conn.community_control.deny_authorization(
            &conn.terminal_ctrl_tx,
            crate::nip_fi_session::NipFiWsRoute::Root,
        );
    } else {
        let _ = conn
            .terminal_ctrl_tx
            .try_send(crate::nip_fi_session::denial_frame(
                crate::nip_fi_session::NipFiWsRoute::Root,
                class,
            ));
    }
    conn.cancel.cancel();
}

/// Refuse a root AUTH that passed the early gates but failed admission: record
/// the outcome, send the denial (the canonical NOTICE under NIP-FI, else an
/// OK false on the control channel so it drains before the Close), and close.
fn deny_admission(conn: &ConnectionState, event_id_hex: &str, denial: AdmissionDenial) {
    metrics::counter!("buzz_auth_failures_total", "reason" => denial.metric).increment(1);
    if !conn.reject_auth(denial.outcome) {
        return;
    }
    if conn.nip_fi_assertion.is_some() {
        deny_nip_fi_auth(conn, denial.class);
        return;
    }
    let _ = conn.ctrl_tx.try_send(WsMessage::Text(
        RelayMessage::ok(event_id_hex, false, denial.reason).into(),
    ));
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
                let outcome = community_ban_outcome(
                    &state,
                    conn.tenant.community(),
                    pubkey,
                    auth_tag_json.as_deref(),
                    Some(signed_auth_created_at),
                )
                .await;

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

            // B2: acquire a session effect permit after the last policy read
            // and before the first persistent write — NIP-OA materialization
            // (users + agent-owner rows) — and hold it through the auth commit.
            //
            // Gate ordering: acquire_effect() obtains the fair read lock, then
            // checks cancel and deadline. A permit is returned only when the
            // session is still active — expiry cannot transition to Expired
            // while any permit is held (the permit IS the read lock). This
            // replaces the old "acquire write_lock → check cancel" fence with
            // a stronger bound: no AUTH commit can start after the gate's
            // deadline passes or after the expiry task's cancel.cancel() fires,
            // and expiry's quiescence waits for this permit. The permit's
            // lifetime is bounded by the caller (connection.rs), which races
            // this whole handler against cancellation and drops it, permit
            // included, when expiry cancels; that fence is load-bearing.
            //
            // Off-mode: the gate has no deadline, but an externally cancelled
            // AUTH still stops here, before materialization.
            // [FI-TRACE-LEASE-BOUND, B2 seam: AUTH commit]
            //
            // Test hook: fires immediately before acquire_effect so a test can
            // arm expiry after the policy reads and before the permit and
            // materialization. This is the exact async gap W1 (auth barrier witness)
            // exercises. No-op in production (cfg(test) only, Mutex<None> unless
            // armed). [nip_fi_test_hooks::auth_commit_hook]
            #[cfg(test)]
            crate::nip_fi_test_hooks::before_auth_commit(conn.tenant.community()).await;
            let _auth_permit = match conn.nip_fi_gate.acquire_effect().await {
                Ok(permit) => permit,
                Err(crate::nip_fi_gate::SessionExpired) => return,
            };

            // Record the NIP-OA owner link before admission: revoking the
            // owner finds the agent's sockets through it, so an agent whose
            // link cannot be recorded is refused rather than admitted
            // unrevocable.
            if let Some(owner) = nip_oa_owner {
                if !crate::api::relay_members::materialize_nip_oa_owner(
                    &state,
                    &conn.tenant,
                    &pubkey,
                    &owner,
                )
                .await
                {
                    warn!(
                        conn_id = %conn_id,
                        agent = %pubkey.to_hex(),
                        nip_oa_owner = %owner.to_hex(),
                        "NIP-OA owner could not be materialized, denying"
                    );
                    deny_admission(
                        &conn,
                        &event_id_hex,
                        AdmissionDenial {
                            metric: "agent_owner_link_error",
                            reason: OWNER_LINK_ERROR,
                            outcome: AuthOutcome::RelayMembershipCheckError,
                            class: buzz_auth::DenialClass::AuthorizationUnavailable,
                        },
                    );
                    return;
                }
                auth_ctx.agent_owner_pubkey = Some(owner);
            }

            // Bind, then take the final ban/membership decision (see
            // `final_admission_denial`), then refuse if a disconnect already
            // cancelled this socket. No await separates the last check from
            // `authenticate`.
            //
            // The bind carries the admitting NIP-FI issuer, so a concurrent
            // `disconnect_nip_fi` scan that sees the pubkey also sees its
            // issuer; the deny-set check after `authenticate` closes the rest.
            let owner =
                match admitted_owner(&state, conn.tenant.community(), pubkey, nip_oa_owner).await {
                    Ok(owner) => owner,
                    Err(denial) => {
                        deny_admission(&conn, &event_id_hex, denial);
                        return;
                    }
                };
            state.conn_manager.set_authenticated_identity(
                conn_id,
                pubkey.to_bytes().to_vec(),
                conn.nip_fi_assertion
                    .as_ref()
                    .map(|a| a.identity().issuer().to_owned()),
            );
            if let Some(owner) = owner {
                state.conn_manager.set_admitted_owner(conn_id, owner);
            }
            let denial = match final_admission_denial(
                &state,
                conn.tenant.community(),
                pubkey,
                auth_tag_json.as_deref(),
                Some(signed_auth_created_at),
            )
            .await
            {
                None if conn.cancel.is_cancelled() => Some(AdmissionDenial {
                    metric: "revoked_during_auth",
                    reason: "blocked: access revoked",
                    outcome: AuthOutcome::Banned,
                    class: buzz_auth::DenialClass::AuthorizationDenied,
                }),
                denial => denial,
            };
            if let Some(denial) = denial {
                warn!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), reason = denial.reason, "denied at final admission check");
                deny_admission(&conn, &event_id_hex, denial);
                return;
            }

            info!(conn_id = %conn_id, pubkey = %pubkey.to_hex(), "NIP-42 auth successful");
            if !conn.authenticate(auth_ctx) {
                return;
            }
            // The permit is held through the deny-set check and the OK send so
            // the auth commit is atomic with respect to expiry.
            //
            // The proven key was registered with its NIP-FI issuer before the
            // final admission check: a concurrent disconnect either finds this
            // session in its close scan or this check finds its deny entry.
            #[cfg(test)]
            crate::nip_fi_test_hooks::before_deny_set_check(conn.tenant.community()).await;
            // [FI-TRACE-DENY-SET]
            if let Some(assertion) = &conn.nip_fi_assertion {
                if let (Some(asserted_key), Some(deny_map)) =
                    (assertion.asserted_key(), state.nip_fi_deny_map.as_deref())
                {
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
                        deny_nip_fi_auth(&conn, buzz_auth::DenialClass::AuthorizationDenied);
                        return;
                    }
                }
            }
            // A concurrent `disconnect_nip_fi` may have closed this session
            // after the deny-set check; its denial is the terminal reply.
            if conn.cancel.is_cancelled() {
                return;
            }
            conn.send(RelayMessage::ok(&event_id_hex, true, ""));
            // _auth_permit drops here — expiry's write guard may proceed.
        }
        Err(e) => {
            warn!(conn_id = %conn_id, error = %e, "NIP-42 auth failed");
            metrics::counter!("buzz_auth_failures_total", "reason" => "nip42_invalid").increment(1);
            if !conn.reject_auth(AuthOutcome::Invalid) {
                return;
            }
            if conn.nip_fi_assertion.is_some() {
                deny_nip_fi_auth(&conn, nip42_denial_class(&e));
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
        handle_auth, nip42_denial_class, BanOutcome, PolicyCheck,
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
            community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
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
        // Same public class as a deny-set hit, so the send loop closes 1008.
        assert_eq!(
            *conn.community_control.disconnect_reason().borrow(),
            Some(crate::state::CommunityDisconnectReason::AuthorizationDenied),
            "pairing mismatch must publish AuthorizationDenied"
        );
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
            community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
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
                nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone()),
                community_control: crate::state::CommunityConnectionControl::new(cancel),
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
            self.auth_event_signed_by(&self.key)
        }

        /// An AUTH over the issued challenge signed by `key`; a key other
        /// than the asserted one is a pairing mismatch.
        fn auth_event_signed_by(&self, key: &Keys) -> nostr::Event {
            EventBuilder::new(Kind::Authentication, "")
                .tag(Tag::parse(["relay", "ws://test.local"]).unwrap())
                .tag(Tag::parse(["challenge", &self.challenge]).unwrap())
                .sign_with_keys(key)
                .unwrap()
        }

        /// The frames the production root send loop writes for this
        /// (already denied) connection.
        async fn wire_frames(self) -> Vec<WsMessage> {
            crate::connection::tests::root_wire_frames(&self.conn, self.terminal_rx).await
        }

        async fn run(&self, event: nostr::Event, state: std::sync::Arc<crate::state::AppState>) {
            handle_auth(event, std::sync::Arc::clone(&self.conn), state).await;
        }

        fn assert_fi_terminal(mut self, class: buzz_auth::DenialClass) {
            assert_eq!(
                self.terminal_rx.try_recv().expect("terminal denial frame"),
                crate::nip_fi_session::denial_frame(
                    crate::nip_fi_session::NipFiWsRoute::Root,
                    class
                ),
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
            // `authorization_denied` closes 1008 like a deny-set hit; the
            // other classes keep the bare close.
            let expected_reason = (class == buzz_auth::DenialClass::AuthorizationDenied)
                .then_some(crate::state::CommunityDisconnectReason::AuthorizationDenied);
            assert_eq!(
                *self.conn.community_control.disconnect_reason().borrow(),
                expected_reason
            );
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
    /// A relay-internal verifier failure is `authorization unavailable`;
    /// every client-evidence NIP-42 failure is `evidence rejected`.
    #[test]
    fn nip42_denial_class_separates_internal_failure_from_bad_evidence() {
        use buzz_auth::{AuthError, DenialClass};
        assert_eq!(
            nip42_denial_class(&AuthError::Internal("spawn_blocking panicked".into())),
            DenialClass::AuthorizationUnavailable
        );
        for evidence in [
            AuthError::InvalidSignature,
            AuthError::ChallengeMismatch,
            AuthError::RelayUrlMismatch,
            AuthError::EventExpired,
        ] {
            assert_eq!(nip42_denial_class(&evidence), DenialClass::EvidenceRejected);
        }
    }

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

    /// Root wire frames for an FI session whose NIP-42 key is not the
    /// asserted one, driven through the production `handle_auth`.
    async fn root_key_mismatch_wire_frames(
        state: std::sync::Arc<crate::state::AppState>,
    ) -> Vec<WsMessage> {
        let harness = AuthHarness::new(true);
        harness
            .run(harness.auth_event_signed_by(&Keys::generate()), state)
            .await;
        harness.wire_frames().await
    }

    /// FI-TRACE-DENIAL-ORACLE: an assertion–key mismatch, an admin disconnect
    /// (deny-set entry), and a lease expiry are all `authorization_denied`, so
    /// the root socket must see byte-identical frames: the NOTICE, then 1008.
    /// (The handler's own deny-set hit is compared against the same mismatch
    /// frames in the Postgres lane's `w_deny_pre_registration_denied_by_handler_check`.)
    ///
    /// Mutation: route the pairing or expiry denial around the shared
    /// transition (bare terminal enqueue) → its close is `Close(None)` and the
    /// sequences differ.
    #[tokio::test]
    async fn fi_root_authorization_denied_rows_emit_identical_frames() {
        let state = auth_test_state().await;
        let mismatch = root_key_mismatch_wire_frames(std::sync::Arc::clone(&state)).await;

        let harness = AuthHarness::new(true);
        let conn = &harness.conn;
        state.conn_manager.register(
            conn.conn_id,
            conn.send_tx.clone(),
            conn.ctrl_tx.clone(),
            conn.terminal_ctrl_tx.clone(),
            None,
            conn.cancel.clone(),
            conn.tenant.community(),
            std::sync::Arc::clone(&conn.backpressure_count),
            std::sync::Arc::clone(&conn.subscriptions),
            conn.grace_limit,
            conn.community_control.clone(),
        );
        let pubkey = harness.key.public_key().to_bytes().to_vec();
        state.conn_manager.set_authenticated_identity(
            conn.conn_id,
            pubkey.clone(),
            Some("test-issuer".to_owned()),
        );
        assert_eq!(
            state.conn_manager.disconnect_nip_fi("test-issuer", &pubkey),
            1
        );
        let admin_disconnect = harness.wire_frames().await;

        let harness = AuthHarness::new(true);
        let expired = chrono::Utc::now() - chrono::Duration::seconds(1);
        crate::nip_fi_session::spawn_nip_fi_expiry_task(
            expired,
            crate::nip_fi_gate::SessionAdmissionGate::new(expired, harness.conn.cancel.clone()),
            harness.conn.community_control.clone(),
            harness.conn.terminal_ctrl_tx.clone(),
            crate::nip_fi_session::NipFiWsRoute::Root,
        )
        .await
        .expect("expiry task");
        let expiry = harness.wire_frames().await;

        assert_eq!(mismatch, admin_disconnect);
        assert_eq!(mismatch, expiry);
        assert_eq!(
            mismatch,
            [
                crate::nip_fi_session::denial_frame(
                    crate::nip_fi_session::NipFiWsRoute::Root,
                    buzz_auth::DenialClass::AuthorizationDenied,
                ),
                crate::state::CommunityDisconnectReason::AuthorizationDenied.close_message(),
            ]
        );
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

        /// A community whose row exists, so bans and users can reference it.
        async fn seeded_community(
            state: &crate::state::AppState,
        ) -> buzz_core::tenant::CommunityId {
            let id = uuid::Uuid::new_v4();
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(id)
                .bind(format!("admission-{}.example", id.simple()))
                .execute(state.db.pool())
                .await
                .expect("insert community");
            buzz_core::tenant::CommunityId::from_uuid(id)
        }

        /// A pending, non-NIP-FI root socket registered with the connection
        /// manager, the registry a ban's disconnect searches.
        fn registered_pending_conn(
            state: &crate::state::AppState,
            community: buzz_core::tenant::CommunityId,
            challenge: &str,
        ) -> (
            std::sync::Arc<crate::connection::ConnectionState>,
            tokio::sync::mpsc::Receiver<WsMessage>,
        ) {
            use std::collections::HashMap;
            use std::sync::Arc;
            let (send_tx, _send_rx) = tokio::sync::mpsc::channel::<WsMessage>(8);
            let (ctrl_tx, ctrl_rx) = tokio::sync::mpsc::channel::<WsMessage>(8);
            let (terminal_ctrl_tx, _terminal_rx) = tokio::sync::mpsc::channel::<WsMessage>(1);
            let cancel = tokio_util::sync::CancellationToken::new();
            let backpressure = Arc::new(std::sync::atomic::AtomicU8::new(0));
            let subscriptions = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
            let conn = Arc::new(crate::connection::ConnectionState {
                conn_id: uuid::Uuid::new_v4(),
                tenant: buzz_core::tenant::TenantContext::resolved(
                    community,
                    "test.local".to_string(),
                ),
                remote_addr: "127.0.0.1:1234".parse().unwrap(),
                auth_state: std::sync::Mutex::new(AuthState::Pending {
                    challenge: challenge.to_string(),
                    started_at: Instant::now(),
                }),
                subscriptions: Arc::clone(&subscriptions),
                send_tx: send_tx.clone(),
                ctrl_tx: ctrl_tx.clone(),
                terminal_ctrl_tx,
                cancel: cancel.clone(),
                backpressure_count: Arc::clone(&backpressure),
                grace_limit: 3,
                nip_fi_assertion: None,
                session_deadline: None,
                nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(cancel.clone()),
                community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
            });
            state.conn_manager.register(
                conn.conn_id,
                send_tx,
                ctrl_tx,
                conn.terminal_ctrl_tx.clone(),
                None,
                cancel,
                community,
                backpressure,
                subscriptions,
                3,
                conn.community_control.clone(),
            );
            (conn, ctrl_rx)
        }

        fn signed_auth(
            keys: &Keys,
            challenge: &str,
            auth_tag: Option<Vec<String>>,
        ) -> nostr::Event {
            let mut builder = EventBuilder::new(Kind::Authentication, "")
                .tag(Tag::parse(["relay", "ws://test.local"]).unwrap())
                .tag(Tag::parse(["challenge", challenge]).unwrap());
            if let Some(tag) = auth_tag {
                builder = builder.tag(Tag::parse(tag).unwrap());
            }
            builder.sign_with_keys(keys).unwrap()
        }

        /// A ban whose disconnect lands after AUTH's policy reads but before it
        /// binds the socket must still refuse admission: the final check runs
        /// after the bind. A user in another community, paused at the same
        /// point, is still admitted.
        ///
        /// Mutation: remove the `final_admission_denial` call from
        /// `handle_auth` → the banned socket is admitted → RED.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn ban_committed_during_auth_refuses_admission() {
            use std::sync::Arc;
            let state = auth_test_state_real_db_expect().await;
            let (banned_community, control_community) = (
                seeded_community(&state).await,
                seeded_community(&state).await,
            );
            let (member, bystander) = (Keys::generate(), Keys::generate());
            let (banned_conn, mut banned_ctrl) =
                registered_pending_conn(&state, banned_community, "race-banned");
            let (control_conn, _control_ctrl) =
                registered_pending_conn(&state, control_community, "race-control");

            let mut paused = Vec::new();
            for (conn, keys, challenge) in [
                (&banned_conn, &member, "race-banned"),
                (&control_conn, &bystander, "race-control"),
            ] {
                let (arrived, release) =
                    crate::nip_fi_test_hooks::auth_commit_hook::arm(conn.tenant.community());
                let handle = tokio::spawn(handle_auth(
                    signed_auth(keys, challenge, None),
                    Arc::clone(conn),
                    Arc::clone(&state),
                ));
                tokio::time::timeout(std::time::Duration::from_secs(5), arrived)
                    .await
                    .expect("AUTH must pause after its policy reads")
                    .expect("hook channel closed");
                paused.push((handle, release));
            }

            // The ban commits and its disconnect runs while AUTH is paused.
            state
                .db
                .ban_community_member(
                    banned_community,
                    &member.public_key().to_bytes(),
                    &Keys::generate().public_key().to_bytes(),
                    None,
                    None,
                )
                .await
                .expect("ban");
            let tenant = banned_conn.tenant.clone();
            state
                .revoke_live_access(
                    &tenant,
                    &member.public_key().to_bytes(),
                    "race-ban",
                    "blocked: you are banned from this community",
                )
                .await
                .expect("revoke");

            for (handle, release) in paused {
                release.notify_one();
                tokio::time::timeout(std::time::Duration::from_secs(5), handle)
                    .await
                    .expect("AUTH must finish")
                    .expect("AUTH must not panic");
            }

            assert!(
                !matches!(
                    banned_conn.auth_state_snapshot(),
                    AuthState::Authenticated(_)
                ),
                "a ban committed during AUTH must refuse admission"
            );
            assert!(
                banned_conn.cancel.is_cancelled(),
                "the refused socket closes"
            );
            let refusal = banned_ctrl.try_recv().expect("refusal frame");
            assert!(
                matches!(&refusal, WsMessage::Text(t) if t.contains("banned")),
                "refusal names the ban, got {refusal:?}"
            );
            assert!(
                matches!(
                    control_conn.auth_state_snapshot(),
                    AuthState::Authenticated(_)
                ),
                "an unaffected community's user is still admitted"
            );
        }

        /// A roster removal whose disconnect runs while AUTH is paused must
        /// refuse admission even when a lagging replica still lists the
        /// member: the final check reads membership from the writer. A user
        /// in another community, paused at the same point, is still admitted.
        ///
        /// Mutation: make `final_admission_denial` call the replica-routed
        /// `check_relay_membership` → the removed socket is admitted → RED.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn removal_during_auth_is_not_undone_by_a_stale_replica() {
            use sqlx::postgres::PgConnectOptions;
            use std::sync::Arc;
            let base = auth_test_state_real_db_expect().await;
            let (removed_community, control_community) =
                (seeded_community(&base).await, seeded_community(&base).await);
            let (member, bystander) = (Keys::generate(), Keys::generate());

            // The "replica": a schema whose relay_members still lists both.
            let db_url = crate::test_support::database_url();
            let schema = format!("stale_replica_{}", uuid::Uuid::new_v4().simple());
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "CREATE SCHEMA {schema}; \
                 CREATE TABLE {schema}.relay_members (LIKE public.relay_members INCLUDING ALL);"
            )))
            .execute(base.db.pool())
            .await
            .expect("create replica schema");
            let replica = sqlx::PgPool::connect_with(
                db_url
                    .parse::<PgConnectOptions>()
                    .expect("database url")
                    .options([("search_path", format!("{schema},public").as_str())]),
            )
            .await
            .expect("replica pool");
            for (community, keys) in [
                (removed_community, &member),
                (control_community, &bystander),
            ] {
                for pool in [base.db.pool(), &replica] {
                    buzz_db::relay_members::add_relay_member(
                        pool,
                        community,
                        &keys.public_key().to_hex(),
                        "member",
                        None,
                    )
                    .await
                    .expect("seed member");
                }
            }
            let mut db = buzz_db::Db::from_pools(base.db.pool().clone(), replica.clone());
            db.fence().force_open_for_tests(chrono::Utc::now());
            db.set_replica_read_max_age_for_tests(Some(std::time::Duration::from_secs(60)));
            let mut config = (*base.config).clone();
            config.require_relay_membership = true;
            let mut state = (*base).clone();
            state.db = db;
            state.config = Arc::new(config);
            let state = Arc::new(state);

            let (removed_conn, _removed_ctrl) =
                registered_pending_conn(&state, removed_community, "stale-removed");
            let (control_conn, _control_ctrl) =
                registered_pending_conn(&state, control_community, "stale-control");
            let mut paused = Vec::new();
            for (conn, keys, challenge) in [
                (&removed_conn, &member, "stale-removed"),
                (&control_conn, &bystander, "stale-control"),
            ] {
                let (arrived, release) =
                    crate::nip_fi_test_hooks::auth_commit_hook::arm(conn.tenant.community());
                let handle = tokio::spawn(handle_auth(
                    signed_auth(keys, challenge, None),
                    Arc::clone(conn),
                    Arc::clone(&state),
                ));
                tokio::time::timeout(std::time::Duration::from_secs(5), arrived)
                    .await
                    .expect("AUTH must pause after its policy reads")
                    .expect("hook channel closed");
                paused.push((handle, release));
            }

            // The removal commits on the writer only, and its disconnect runs
            // while AUTH is paused.
            state
                .db
                .remove_relay_member(removed_community, &member.public_key().to_hex())
                .await
                .expect("remove member");
            assert!(
                state
                    .db
                    .is_relay_member(removed_community, &member.public_key().to_hex())
                    .await
                    .expect("routed read"),
                "precondition: the routed read still sees the stale replica row"
            );
            state
                .revoke_live_access(
                    &removed_conn.tenant.clone(),
                    &member.public_key().to_bytes(),
                    "stale-removal",
                    "restricted: you were removed from this relay",
                )
                .await
                .expect("revoke");

            for (handle, release) in paused {
                release.notify_one();
                tokio::time::timeout(std::time::Duration::from_secs(5), handle)
                    .await
                    .expect("AUTH must finish")
                    .expect("AUTH must not panic");
            }
            let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
                .execute(base.db.pool())
                .await;

            assert!(
                !matches!(
                    removed_conn.auth_state_snapshot(),
                    AuthState::Authenticated(_)
                ),
                "a removal committed during AUTH must refuse admission"
            );
            assert!(
                removed_conn.cancel.is_cancelled(),
                "the refused socket closes"
            );
            assert!(
                matches!(
                    control_conn.auth_state_snapshot(),
                    AuthState::Authenticated(_)
                ),
                "an unaffected community's member is still admitted"
            );
        }

        /// An agent whose owner link cannot be recorded (here: the agent is
        /// already linked to a different owner) is refused, because revoking
        /// its NIP-OA owner could not find it.
        ///
        /// Mutation: restore "continue on a failed owner-link write" in
        /// `handle_auth` → the agent is admitted → RED.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn root_auth_refuses_agent_whose_owner_link_fails() {
            use std::sync::Arc;
            let state = auth_test_state_real_db_expect().await;
            let community = seeded_community(&state).await;
            let (agent, owner, prior_owner) =
                (Keys::generate(), Keys::generate(), Keys::generate());
            for key in [&agent, &prior_owner] {
                state
                    .db
                    .ensure_user_for_authorization(community, key.public_key().as_bytes())
                    .await
                    .expect("seed user");
            }
            assert!(state
                .db
                .set_agent_owner_for_authorization(
                    community,
                    agent.public_key().as_bytes(),
                    prior_owner.public_key().as_bytes(),
                )
                .await
                .expect("seed prior owner"));
            let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "")
                .expect("sign NIP-OA credential");
            let auth_tag: Vec<String> = serde_json::from_str(&auth_tag).expect("tag JSON");

            let (conn, mut ctrl) = registered_pending_conn(&state, community, "owner-link");
            handle_auth(
                signed_auth(&agent, "owner-link", Some(auth_tag)),
                Arc::clone(&conn),
                state,
            )
            .await;

            assert!(!matches!(
                conn.auth_state_snapshot(),
                AuthState::Authenticated(_)
            ));
            let refusal = ctrl.try_recv().expect("refusal frame");
            assert!(
                matches!(&refusal, WsMessage::Text(t) if t.contains(crate::handlers::auth::OWNER_LINK_ERROR)),
                "refusal names the owner-link failure, got {refusal:?}"
            );
        }

        /// Root AUTH records each agent's owner on its socket — the proven
        /// NIP-OA owner, or the stored owner link when the agent signs in
        /// without a credential — so revoking the owner closes both agents
        /// even after the stored links are gone. A bystander stays admitted.
        ///
        /// Mutations: drop `set_admitted_owner` from `handle_auth` → both
        /// agents stay open → RED; ignore the stored link in
        /// `admitted_owner` → the credential-less agent stays open → RED.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn root_auth_records_owner_for_lookup_free_revoke() {
            use std::sync::Arc;
            let state = auth_test_state_real_db_expect().await;
            let community = seeded_community(&state).await;
            let (owner, tagged, stored, bystander) = (
                Keys::generate(),
                Keys::generate(),
                Keys::generate(),
                Keys::generate(),
            );
            for key in [&owner, &stored] {
                state
                    .db
                    .ensure_user_for_authorization(community, key.public_key().as_bytes())
                    .await
                    .expect("seed user");
            }
            assert!(state
                .db
                .set_agent_owner_for_authorization(
                    community,
                    stored.public_key().as_bytes(),
                    owner.public_key().as_bytes(),
                )
                .await
                .expect("seed stored owner link"));
            let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &tagged.public_key(), "")
                .expect("sign NIP-OA credential");
            let auth_tag: Vec<String> = serde_json::from_str(&auth_tag).expect("tag JSON");

            let mut conns = Vec::new();
            for (keys, tag, challenge) in [
                (&tagged, Some(auth_tag), "owner-tagged"),
                (&stored, None, "owner-stored"),
                (&bystander, None, "owner-bystander"),
            ] {
                let (conn, _ctrl) = registered_pending_conn(&state, community, challenge);
                handle_auth(
                    signed_auth(keys, challenge, tag),
                    Arc::clone(&conn),
                    Arc::clone(&state),
                )
                .await;
                assert!(
                    matches!(conn.auth_state_snapshot(), AuthState::Authenticated(_)),
                    "{challenge} is admitted"
                );
                conns.push(conn);
            }
            sqlx::query("UPDATE users SET agent_owner_pubkey = NULL WHERE community_id = $1")
                .bind(community.as_uuid())
                .execute(state.db.pool())
                .await
                .expect("clear stored owner links");

            let tenant = conns[0].tenant.clone();
            state
                .revoke_live_access(
                    &tenant,
                    &owner.public_key().to_bytes(),
                    "owner-revoke",
                    "blocked: you are banned from this community",
                )
                .await
                .expect("revoke");
            assert!(conns[0].cancel.is_cancelled(), "the NIP-OA agent closes");
            assert!(
                conns[1].cancel.is_cancelled(),
                "the stored-link agent closes"
            );
            assert!(!conns[2].cancel.is_cancelled(), "the bystander stays");
        }

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

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn w_deny_straddle_entry_inserted_between_registration_and_check_is_caught() {
            w_deny_straddle_entry_inserted_between_registration_and_check_is_caught_body().await;
        }

        // ── W_deny_straddle: deny entry inserted in window between registration and check
        //
        // Arms `before_deny_set_check` — the hook immediately AFTER
        // `set_authenticated_pubkey` (registration) and BEFORE the `is_denied` call.
        // The key starts absent from the deny map. Once registration occurs the
        // handler stalls at the hook. At the hook, the test:
        //   1. Inserts the deny entry into the live map.
        //   2. Executes the real ConnectionManager::disconnect_nip_fi (close-scan side):
        //      asserts it finds exactly 1 registered session — proving registration is
        //      visible to a concurrent disconnect in this exact window.
        //   3. Releases the hook — the handler resumes and calls is_denied() (check side).
        // Both sides are exercised; neither can miss. The connection is cancelled and
        // the exact `authorization_denied` NOTICE frame is asserted; no OK(true) sent.
        //
        // Mutation evidence (executed on green baseline):
        //   A) Delete `#[cfg(test)] before_deny_set_check(...)` from auth.rs →
        //      handler never stalls → deny entry inserted AFTER check runs and
        //      missed → close_scan returns 0 (session deregistered) → assertion panics.
        //   B) Remove the `is_denied` check entirely → same outcome as (A).
        //   C) Move hook to before `set_authenticated_pubkey` (registration) →
        //      handler stalls before registration → close-scan `disconnect_nip_fi`
        //      returns 0 (not yet registered) → "exactly 1 session" assertion panics.
        //      Causally falsifies the registration-before-check invariant.
        //
        // Runs in PG lane on wrapper DB (same constraint as W1: ban-check is fail-closed).
        async fn w_deny_straddle_entry_inserted_between_registration_and_check_is_caught_body() {
            use buzz_auth::{IssuerCapacity, NipFiDenyMap, VerifiedAssertion};
            use chrono::{Duration, Utc};
            use std::collections::HashMap;
            use std::sync::Arc;
            use tokio::sync::mpsc;
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

            // Use a unique community UUID so this test's deny_set_check_hook slot
            // does not collide with other concurrent tests (audio-active uses Uuid::nil()).
            let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::new_v4());
            let deny_straddle_control =
                crate::state::CommunityConnectionControl::new(cancel.clone());

            let conn = Arc::new(crate::connection::ConnectionState {
                conn_id: Uuid::new_v4(),
                tenant: buzz_core::tenant::TenantContext::resolved(
                    community,
                    "test.local".to_string(),
                ),
                remote_addr: "127.0.0.1:1234".parse().unwrap(),
                auth_state: std::sync::Mutex::new(AuthState::Pending {
                    challenge: challenge.clone(),
                    started_at: std::time::Instant::now(),
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
                community_control: deny_straddle_control,
            });

            // Real DB required (ban-check is fail-closed; lazy pool denies before hook).
            let mut state = Arc::try_unwrap(auth_test_state_real_db_expect().await)
                .unwrap_or_else(|arc| (*arc).clone());

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

            // Register the connection with conn_manager so set_authenticated_pubkey
            // (called by handle_auth after NIP-42 succeeds) stores the pubkey — the
            // close-scan side calls disconnect_nip_fi which iterates over registered
            // connections. Without this registration, set_authenticated_pubkey is a
            // no-op and disconnect_nip_fi always returns 0.
            state.conn_manager.register(
                conn.conn_id,
                conn.send_tx.clone(),
                conn.ctrl_tx.clone(),
                conn.terminal_ctrl_tx.clone(),
                None, // no restart_tx for this unit-test fixture
                cancel.clone(),
                community,
                Arc::clone(&conn.backpressure_count),
                Arc::clone(&conn.subscriptions),
                conn.grace_limit,
                conn.community_control.clone(),
            );

            let relay_url = "ws://test.local";
            let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
                .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
                .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
                .sign_with_keys(&key)
                .unwrap();

            // Arm the barrier: fires when handle_auth reaches before_deny_set_check.
            let (arrived_rx, release) =
                crate::nip_fi_test_hooks::deny_set_check_hook::arm(community);

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

            // Close-scan side: run the real ConnectionManager::disconnect_nip_fi now
            // that the connection is registered. This proves the registration is visible
            // to the concurrent close scan — the normative invariant [FI-TRACE-DENY-SET].
            // With the deny entry live, the scan finds exactly one session matching this
            // pubkey and closes it.
            let pubkey_bytes = key.public_key().to_bytes().to_vec();
            let closed = state
                .conn_manager
                .disconnect_nip_fi("test-issuer", &pubkey_bytes);
            assert_eq!(
                closed, 1,
                "W_deny_straddle: close scan must find exactly 1 registered session \
             (proves registration is visible between the hook and the check)"
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

            // The denial frame must be on the terminal channel (authorization_denied).
            // Both the close-scan side (manager_disconnect_nip_fi) and the check side
            // (deny_authorization) enqueue on terminal_ctrl_tx, which has capacity-1
            // and first-writer-wins semantics — exactly one frame lands there.
            let terminal_frame = terminal_ctrl_rx
                .try_recv()
                .expect("W_deny_straddle: terminal channel must contain the denial frame");
            if let WsMessage::Text(t) = &terminal_frame {
                let expected = crate::protocol::RelayMessage::notice(
                    buzz_auth::DenialClass::AuthorizationDenied.nostr_text(),
                );
                assert_eq!(
                t.as_str(),
                expected.as_str(),
                "W_deny_straddle: terminal frame must be exact authorization_denied NOTICE; got: {t}"
            );
            } else {
                panic!(
                    "W_deny_straddle: terminal frame must be Text(NOTICE); got {terminal_frame:?}"
                );
            }
            // ctrl channel must be empty — denial goes to terminal only.
            assert!(
                ctrl_rx.try_recv().is_err(),
                "W_deny_straddle: ctrl channel must be empty (denial goes to terminal channel)"
            );

            // No OK(true) on the data channel.
            while let Ok(WsMessage::Text(t)) = send_rx.try_recv() {
                assert!(
                    !is_ok_true(t.as_str()),
                    "W_deny_straddle: no OK(true) must be sent when deny-set catches \
                     the entry inserted between registration and check; got: {t}"
                );
            }
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn w_deny_other_issuer_straddle_leaves_session_admitted() {
            w_deny_other_issuer_straddle_leaves_session_admitted_body().await;
        }

        // ── W_deny_other_issuer_straddle: a deny for (A, K) in the registration→check
        // window neither closes nor refuses a session admitted under B with key K.
        //
        // Same seam as W_deny_straddle.  The session's assertion is issued by B
        // ("test-issuer"); at the hook the test inserts a deny entry for issuer A
        // and runs A's close scan (root + audio), which must match nothing.  After
        // release the B session completes admission: OK(true), not cancelled.
        // Mutation: a pubkey-only scan closes the B session → `closed == 0` fails.
        // [FI-TRACE-DENY-SET]
        //
        // Runs in PG lane on wrapper DB (ban-check is fail-closed).
        async fn w_deny_other_issuer_straddle_leaves_session_admitted_body() {
            use buzz_auth::{IssuerCapacity, NipFiDenyMap, VerifiedAssertion};
            use chrono::{Duration, Utc};
            use std::collections::HashMap;
            use std::sync::Arc;
            use tokio::sync::mpsc;
            use tokio_util::sync::CancellationToken;
            use uuid::Uuid;

            const ISSUER_A: &str = "https://issuer-a.example";
            const ISSUER_B: &str = "test-issuer"; // VerifiedAssertion::for_test

            let key = Keys::generate();
            let deadline = Utc::now() + Duration::hours(1);
            let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);
            assert_eq!(assertion.identity().issuer(), ISSUER_B);

            let challenge = "w-deny-other-issuer-challenge".to_string();
            let (send_tx, mut send_rx) = mpsc::channel::<WsMessage>(8);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel::<WsMessage>(8);
            let (terminal_ctrl_tx, mut terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
            let cancel = CancellationToken::new();
            let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());
            let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::new_v4());

            let conn = Arc::new(crate::connection::ConnectionState {
                conn_id: Uuid::new_v4(),
                tenant: buzz_core::tenant::TenantContext::resolved(
                    community,
                    "test.local".to_string(),
                ),
                remote_addr: "127.0.0.1:1234".parse().unwrap(),
                auth_state: std::sync::Mutex::new(AuthState::Pending {
                    challenge: challenge.clone(),
                    started_at: std::time::Instant::now(),
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
                community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
            });

            let mut state = Arc::try_unwrap(auth_test_state_real_db_expect().await)
                .unwrap_or_else(|arc| (*arc).clone());
            let deny_map = Arc::new(NipFiDenyMap::new(
                16,
                [ISSUER_A, ISSUER_B]
                    .into_iter()
                    .map(|issuer| IssuerCapacity {
                        issuer: issuer.to_owned(),
                        capacity: 16,
                    })
                    .collect(),
            ));
            state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
            let state = Arc::new(state);
            state.conn_manager.register(
                conn.conn_id,
                conn.send_tx.clone(),
                conn.ctrl_tx.clone(),
                conn.terminal_ctrl_tx.clone(),
                None,
                cancel.clone(),
                community,
                Arc::clone(&conn.backpressure_count),
                Arc::clone(&conn.subscriptions),
                conn.grace_limit,
                conn.community_control.clone(),
            );

            let relay_url = "ws://test.local";
            let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
                .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
                .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
                .sign_with_keys(&key)
                .unwrap();

            let (arrived_rx, release) =
                crate::nip_fi_test_hooks::deny_set_check_hook::arm(community);
            let handle = tokio::spawn(handle_auth(
                auth_event,
                Arc::clone(&conn),
                Arc::clone(&state),
            ));
            tokio::time::timeout(std::time::Duration::from_secs(5), arrived_rx)
                .await
                .expect("handler must reach before_deny_set_check within 5s")
                .expect("arrived channel closed");

            // Issuer A's command lands in the window: entry + both close scans.
            assert!(matches!(
                deny_map.merge_cross_pod_deny(
                    ISSUER_A,
                    &key.public_key(),
                    Utc::now() + Duration::seconds(3600),
                    Utc::now(),
                ),
                buzz_auth::CrossPodMergeResult::Merged
            ));
            let pubkey_bytes = key.public_key().to_bytes().to_vec();
            let closed = state
                .conn_manager
                .disconnect_nip_fi(ISSUER_A, &pubkey_bytes)
                + state
                    .community_connections
                    .disconnect_nip_fi(ISSUER_A, &pubkey_bytes);
            assert_eq!(
                closed, 0,
                "issuer A's close scan must not match a session admitted under B"
            );

            release.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(5), handle)
                .await
                .expect("handle_auth must return within 5s after hook release")
                .expect("handle_auth task must not panic");

            assert!(
                !cancel.is_cancelled(),
                "B session must not be cancelled by A's deny"
            );
            assert!(
                terminal_ctrl_rx.try_recv().is_err(),
                "B session must get no denial frame"
            );
            assert!(
                matches!(
                    *conn.auth_state.lock().unwrap(),
                    AuthState::Authenticated(_)
                ),
                "B session must complete admission"
            );
            let mut ok_true = false;
            while let Ok(WsMessage::Text(t)) = send_rx.try_recv() {
                ok_true |= is_ok_true(t.as_str());
            }
            assert!(ok_true, "B session must receive OK(true)");
        }

        /// True iff `frame` is a NIP-01 `["OK", <id>, true, ...]` acceptance.
        fn is_ok_true(frame: &str) -> bool {
            serde_json::from_str::<serde_json::Value>(frame)
                .ok()
                .and_then(|v| v.as_array().cloned())
                .is_some_and(|a| {
                    a.first().and_then(|v| v.as_str()) == Some("OK")
                        && a.get(2) == Some(&serde_json::Value::Bool(true))
                })
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn w_deny_pre_registration_denied_by_handler_check() {
            w_deny_pre_registration_body(true).await;
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn w_deny_pre_registration_clean_key_admitted() {
            w_deny_pre_registration_body(false).await;
        }

        // ── W_deny_pre_registration: the deny entry exists BEFORE the connection
        // proves its identity, so an issuer-scoped close scan run at that point
        // misses the unproven connection (returns 0). Only the handler's own
        // post-registration deny check can refuse it. With `deny_self == false`
        // the entry targets a different key (positive control): the session must
        // be admitted with OK(true).
        //
        // Mutation: bypass the `[FI-TRACE-DENY-SET]` check in `handle_auth` →
        // the denied case sends OK(true), stays uncancelled, enqueues no frame.
        async fn w_deny_pre_registration_body(deny_self: bool) {
            use buzz_auth::{IssuerCapacity, NipFiDenyMap, VerifiedAssertion};
            use chrono::{Duration, Utc};
            use std::collections::HashMap;
            use std::sync::Arc;
            use tokio::sync::mpsc;
            use tokio_util::sync::CancellationToken;
            use uuid::Uuid;

            let key = Keys::generate();
            let deadline = Utc::now() + Duration::hours(1);
            // `for_test` produces issuer = "test-issuer".
            let assertion = VerifiedAssertion::for_test(Some(key.public_key()), vec![deadline]);

            let challenge = "w-deny-pre-registration-challenge".to_string();
            let (send_tx, mut send_rx) = mpsc::channel::<WsMessage>(8);
            let (ctrl_tx, mut ctrl_rx) = mpsc::channel::<WsMessage>(8);
            let (terminal_ctrl_tx, mut terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
            let cancel = CancellationToken::new();
            let gate = crate::nip_fi_gate::SessionAdmissionGate::new(deadline, cancel.clone());
            let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::new_v4());

            let conn = Arc::new(crate::connection::ConnectionState {
                conn_id: Uuid::new_v4(),
                tenant: buzz_core::tenant::TenantContext::resolved(
                    community,
                    "test.local".to_string(),
                ),
                remote_addr: "127.0.0.1:1234".parse().unwrap(),
                auth_state: std::sync::Mutex::new(AuthState::Pending {
                    challenge: challenge.clone(),
                    started_at: std::time::Instant::now(),
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
                community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
            });

            let mut state = Arc::try_unwrap(auth_test_state_real_db_expect().await)
                .unwrap_or_else(|arc| (*arc).clone());
            let deny_map = Arc::new(NipFiDenyMap::new(
                16,
                vec![IssuerCapacity {
                    issuer: "test-issuer".to_owned(),
                    capacity: 16,
                }],
            ));
            let denied_key = if deny_self {
                key.public_key()
            } else {
                Keys::generate().public_key()
            };
            let merge = deny_map.merge_cross_pod_deny(
                "test-issuer",
                &denied_key,
                Utc::now() + Duration::hours(1),
                Utc::now(),
            );
            assert!(matches!(merge, buzz_auth::CrossPodMergeResult::Merged));
            state.nip_fi_deny_map = Some(deny_map);
            let state = Arc::new(state);

            state.conn_manager.register(
                conn.conn_id,
                conn.send_tx.clone(),
                conn.ctrl_tx.clone(),
                conn.terminal_ctrl_tx.clone(),
                None,
                cancel.clone(),
                community,
                Arc::clone(&conn.backpressure_count),
                Arc::clone(&conn.subscriptions),
                conn.grace_limit,
                conn.community_control.clone(),
            );

            // The close scan runs while the connection is still unproven: it
            // must miss it, leaving the handler's check as the only defence.
            let closed = state
                .conn_manager
                .disconnect_nip_fi("test-issuer", &key.public_key().to_bytes());
            assert_eq!(closed, 0, "close scan must miss the unproven connection");
            assert!(!cancel.is_cancelled());

            let relay_url = "ws://test.local";
            let auth_event = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
                .tag(nostr::Tag::parse(["relay", relay_url]).unwrap())
                .tag(nostr::Tag::parse(["challenge", &challenge]).unwrap())
                .sign_with_keys(&key)
                .unwrap();
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                handle_auth(auth_event, Arc::clone(&conn), Arc::clone(&state)),
            )
            .await
            .expect("handle_auth must return within 5s");

            let mut ok_true = false;
            while let Ok(WsMessage::Text(t)) = send_rx.try_recv() {
                ok_true |= is_ok_true(t.as_str());
            }
            assert!(ctrl_rx.try_recv().is_err(), "ctrl channel must stay empty");

            if deny_self {
                assert!(!ok_true, "denied key must not receive OK(true)");
                assert!(
                    cancel.is_cancelled(),
                    "handler check must cancel the session"
                );
                // The deny-set row is byte-identical on the wire to the
                // assertion–key mismatch row.  [FI-TRACE-DENIAL-ORACLE]
                let frames =
                    crate::connection::tests::root_wire_frames(&conn, terminal_ctrl_rx).await;
                assert_eq!(
                    frames,
                    super::root_key_mismatch_wire_frames(Arc::clone(&state)).await
                );
            } else {
                assert!(ok_true, "clean key must receive OK(true)");
                assert!(!cancel.is_cancelled(), "clean key must not be cancelled");
                assert!(
                    terminal_ctrl_rx.try_recv().is_err(),
                    "clean key gets no frame"
                );
                assert!(matches!(
                    *conn.auth_state.lock().unwrap(),
                    AuthState::Authenticated(_)
                ));
            }
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
                community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
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

        /// Permit-before-write witness: expiry armed after the policy reads
        /// and before NIP-OA materialization leaves no `users` / agent-owner
        /// row and no committed auth.
        ///
        /// Mutation oracle: move `acquire_effect()` back after
        /// `materialize_nip_oa_owner` → the agent and owner rows are written
        /// before the permit is refused → the row-count assertion goes RED.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn expiry_before_nip_oa_materialize_writes_no_rows_and_commits_no_auth() {
            use buzz_auth::VerifiedAssertion;
            use chrono::{Duration, Utc};
            use std::collections::HashMap;
            use std::sync::Arc;
            use tokio::sync::mpsc;
            use tokio_util::sync::CancellationToken;
            use uuid::Uuid;

            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .expect("PostgreSQL must be available");
            let community_id = Uuid::new_v4();
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(community_id)
                .bind(format!("materialize-{}.example", community_id.simple()))
                .execute(&pool)
                .await
                .expect("insert community");
            let community = buzz_core::tenant::CommunityId::from_uuid(community_id);

            let agent = Keys::generate();
            let owner = Keys::generate();
            let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "")
                .expect("sign NIP-OA credential");
            let auth_tag: Vec<String> = serde_json::from_str(&auth_tag).expect("tag JSON");

            let deadline = Utc::now() + Duration::hours(1);
            let assertion = VerifiedAssertion::for_test(Some(agent.public_key()), vec![deadline]);
            let challenge = "materialize-barrier-challenge".to_string();
            let (send_tx, mut send_rx) = mpsc::channel::<WsMessage>(8);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel::<WsMessage>(8);
            let (terminal_ctrl_tx, _terminal_ctrl_rx) = mpsc::channel::<WsMessage>(1);
            let cancel = CancellationToken::new();
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
                nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::new(
                    deadline,
                    cancel.clone(),
                ),
                community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
            });

            let state = auth_test_state_real_db_expect().await;
            let auth_event = EventBuilder::new(Kind::Authentication, "")
                .tag(Tag::parse(["relay", "ws://test.local"]).unwrap())
                .tag(Tag::parse(["challenge", &challenge]).unwrap())
                .tag(Tag::parse(auth_tag).unwrap())
                .sign_with_keys(&agent)
                .unwrap();

            let (arrived_rx, release) = crate::nip_fi_test_hooks::auth_commit_hook::arm(community);
            let handle = tokio::spawn(handle_auth(auth_event, Arc::clone(&conn), state));
            tokio::time::timeout(std::time::Duration::from_secs(5), arrived_rx)
                .await
                .expect("handler must reach the pre-materialize hook within 5s")
                .expect("arrived channel closed");
            cancel.cancel();
            release.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(5), handle)
                .await
                .expect("handle_auth must return within 5s")
                .expect("handle_auth must not panic");

            let rows: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM users WHERE community_id = $1 AND pubkey = ANY($2)",
            )
            .bind(community_id)
            .bind(vec![
                agent.public_key().to_bytes().to_vec(),
                owner.public_key().to_bytes().to_vec(),
            ])
            .fetch_one(&pool)
            .await
            .expect("count users");
            assert_eq!(
                rows, 0,
                "no users / agent-owner row may be written after expiry"
            );
            assert!(
                !matches!(conn.auth_state_snapshot(), AuthState::Authenticated(_)),
                "auth must not be committed after expiry"
            );
            assert!(
                send_rx.try_recv().is_err(),
                "no OK may be sent after expiry"
            );
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
                community_control: crate::state::CommunityConnectionControl::new(cancel.clone()),
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
            let expected_frame = crate::nip_fi_session::denial_frame(
                crate::nip_fi_session::NipFiWsRoute::Root,
                buzz_auth::DenialClass::AuthorizationDenied,
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
                "Fix 4a: terminal frame must be the exact canonical denial_frame"
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
                 CREATE TABLE {schema}.community_bans (LIKE public.community_bans INCLUDING ALL); \
                 CREATE TABLE {schema}.users (LIKE public.users INCLUDING ALL);"
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

        /// Carl's coverage gap: a matching-key FI connection whose pubkey is
        /// banned gets the terminal `authorization denied` NOTICE and the
        /// socket closes; off mode keeps `OK false` on ctrl, then closes.
        #[tokio::test]
        #[ignore = "requires Postgres — runs in postgres-ci nextest lane"]
        async fn fi_ban_denial_emits_terminal_authorization_denied() {
            let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .expect("PostgreSQL must be available");
            let state = state_with_pool(pool, |_| {}).await;
            for with_fi in [true, false] {
                let mut harness = AuthHarness::new(with_fi);
                let community = *harness.conn.tenant.community().as_uuid();
                sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                    .bind(community)
                    .bind(format!("fi-ban-{}.example", community.simple()))
                    .execute(state.db.pool())
                    .await
                    .expect("insert community");
                sqlx::query(
                    "INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey) \
                     VALUES ($1, $2, TRUE, $3)",
                )
                .bind(community)
                .bind(harness.key.public_key().to_bytes().to_vec())
                .bind(Keys::generate().public_key().to_bytes().to_vec())
                .execute(state.db.pool())
                .await
                .expect("ban key");
                harness.run(harness.auth_event(), state.clone()).await;
                if with_fi {
                    harness.assert_fi_terminal(buzz_auth::DenialClass::AuthorizationDenied);
                    continue;
                }
                let frame = match harness.ctrl_rx.try_recv().expect("off-mode OK on ctrl") {
                    WsMessage::Text(text) => {
                        serde_json::from_str::<serde_json::Value>(&text).unwrap()
                    }
                    other => panic!("expected text frame, got {other:?}"),
                };
                assert_eq!(frame[0], "OK");
                assert_eq!(frame[2], false);
                assert_eq!(frame[3], "blocked: you are banned from this community");
                assert!(
                    harness.terminal_rx.try_recv().is_err(),
                    "off mode never uses terminal"
                );
                assert!(
                    harness.send_rx.try_recv().is_err(),
                    "no OK on the data channel"
                );
                assert!(
                    harness.conn.cancel.is_cancelled(),
                    "a ban closes the socket"
                );
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
