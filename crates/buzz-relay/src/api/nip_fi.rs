//! NIP-FI admin disconnect endpoint — `POST /api/nip-fi/disconnect`.
//!
//! This module owns:
//!
//! * [`disconnect`] — the axum handler for `POST /api/nip-fi/disconnect`.
//! * [`build_nip_fi_command_components`] — startup initialization called by
//!   `main.rs` to wire the deny map and command verifier into `AppState`.
//!
//! ## Transport invariant
//!
//! The NIP-FI admin API is **not** a protected HTTP surface.  It MUST NOT be
//! subjected to the NIP-FI HTTP-ingress admission procedure.  It carries
//! `Nostr-Federated-Identity` for a command JWS, not an identity assertion.
//! [NIP-FI.md §HTTP ingress, protected surfaces note]
//!
//! Authentication is entirely by the signed command JWT verified inside
//! [`buzz_auth::CommandVerifier::verify`]; no NIP-98 or relay-membership check
//! is performed.
//!
//! ## Environment variables
//!
//! The command API is enabled when `BUZZ_NIP_FI_MODE` is `enforce` or
//! `shadow` (shadow keeps the deny entry but closes no session).  S4 fields
//! are read from the same `BUZZ_NIP_FI_ISSUERS` JSON array as the assertion
//! policy; in both modes every issuer entry must carry them:
//!
//! ```json
//! {
//!   "maximum_command_age_seconds": 30,
//!   "authorized_principals": ["service-account@issuer.example.com"],
//!   "deny_set_capacity": 50000
//! }
//! ```
//!
//! `maximum_command_age_seconds` (1–60) and a non-empty
//! `authorized_principals` are required on every issuer; startup fails if any
//! issuer lacks them.  `deny_set_capacity` is optional and defaults to
//! [`DEFAULT_DENY_SET_CAPACITY`].

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use tracing::{debug, warn};

use buzz_auth::{
    CommandError, CommandIssuerPolicy, CommandVerifier, IssuerCapacity, JwksFetcher, NipFiDenyMap,
    NipFiMode, ProductionJwksSource,
};

use crate::nip_fi_core::{extract_bearer_token, http_denial};
use crate::state::AppState;

/// Default per-issuer deny-set capacity when `deny_set_capacity` is absent.
/// 50_000 entries × ~128 bytes ≈ 6.4 MB per issuer.
pub const DEFAULT_DENY_SET_CAPACITY: usize = 50_000;

// ── Request / response shapes ────────────────────────────────────────────────

/// JSON body for `POST /api/nip-fi/disconnect`.
#[derive(Debug, Deserialize)]
pub struct DisconnectRequest {
    /// Lowercase hex encoding of the 32-byte target Nostr public key.
    pub pubkey: String,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// `POST /api/nip-fi/disconnect`
///
/// Executes `VerifyCommandJwt`, inserts the deny entry, and closes all live
/// sessions for the target pubkey across all communities.
///
/// Response contract (from NIP-FI spec):
///
/// | Condition | Status | Body |
/// |---|---|---|
/// | Authorized; action taken or no-op | `200` | `{"disconnected":true}` |
/// | Missing or invalid command JWT | `401`/`403` | per rejection table |
/// | Malformed request body or `until` exceeds ceiling | `400` | `"bad request\n"` |
/// | Deny set at capacity | `503` | `"deny set full\n"` |
///
/// The endpoint is NOT a protected HTTP surface.  [NIP-FI.md §HTTP ingress]
pub async fn disconnect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response<Body> {
    // ── Extract the command JWT from the header ────────────────────────────
    // Absent → 401 with `WWW-Authenticate: Nostr`; any other malformation
    // → 403. [NIP-FI.md §Rejection table]
    let token = match extract_bearer_token(&headers) {
        Ok(t) => t,
        Err(class) => return http_denial(class),
    };

    // ── Parse the JSON body ───────────────────────────────────────────────
    let req: DisconnectRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return command_denial(CommandError::MalformedRequest),
    };

    // body.pubkey must be lowercase hex of exactly 32 bytes.
    let body_pubkey = match parse_hex_pubkey(&req.pubkey) {
        Some(k) => k,
        None => return command_denial(CommandError::MalformedRequest),
    };

    // ── Command verifier ──────────────────────────────────────────────────
    let verifier = match &state.nip_fi_command_verifier {
        Some(v) => v.clone(),
        None => {
            // Mode is Off or not yet initialized.
            debug!("nip-fi disconnect: no command verifier configured");
            return command_denial(CommandError::AuthorizationUnavailable);
        }
    };

    let result = verifier
        .verify(
            token,
            "POST",
            "/api/nip-fi/disconnect",
            &body_pubkey,
            state.nip_fi_command_replay.as_ref(),
        )
        .await;

    match result {
        Ok(cmd) => {
            // ── Deny entry inserted; close sessions synchronously ─────────
            let pubkey_bytes = cmd.target_pubkey.to_bytes();
            // Issuer-scoped: the deny entry is keyed by (caller_iss, k), so only
            // sessions admitted under caller_iss are closed. [FI-TRACE-DENY-SET]
            // Shadow keeps the deny entry but closes nothing.
            // The command's scope, widened by any stronger active entry.
            let scope = state.nip_fi_deny_map.as_ref().map_or(cmd.scope, |map| {
                map.close_scope(
                    &cmd.caller_iss,
                    &cmd.target_pubkey,
                    cmd.scope,
                    chrono::Utc::now(),
                )
            });
            let closed = if !state.config.nip_fi.mode.observes_only() {
                state
                    .conn_manager
                    .disconnect_nip_fi(&cmd.caller_iss, &pubkey_bytes, scope)
                    + state.community_connections.disconnect_nip_fi(
                        &cmd.caller_iss,
                        &pubkey_bytes,
                        scope,
                    )
            } else {
                state
                    .nip_fi_shadow_sessions
                    .would_close(&cmd.caller_iss, &pubkey_bytes, scope);
                0
            };
            if closed > 0 {
                // [FI-TRACE-PRIVACY-NONPUBLIC]: raw `iss` MUST NOT appear in
                // logs, metrics, or traces.  Log only a count.
                debug!(closed, "nip-fi disconnect: closed sessions");
            }
            let mode = state.config.nip_fi.mode;
            count_disconnect_event(mode, "buzz_nip_fi_disconnect_total", "admin", "accepted", 1);
            if !mode.observes_only() {
                metrics::counter!(
                    "buzz_nip_fi_sessions_closed_total",
                    "reason" => "admin_disconnect"
                )
                .increment(closed as u64);
            }

            // Cross-pod propagation: publish to global NIP-FI Redis channel
            // so remote pods can merge the deny entry and close their sessions.
            // Asynchronous: HTTP response does not wait on remote delivery.
            {
                let pubsub = Arc::clone(&state.pubsub);
                let msg = nip_fi_disconnect_message(&cmd);
                let channel = disconnect_publish_channel(state.config.nip_fi.mode);
                state.nip_fi_publish_tasks.spawn(async move {
                    if let Err(e) = pubsub.publish_nip_fi_disconnect(channel, &msg).await {
                        // [FI-TRACE-PRIVACY-NONPUBLIC]: no iss or pubkey in logs
                        tracing::warn!("nip-fi: cross-pod propagation publish failed: {e}");
                        count_disconnect_event(
                            mode,
                            "buzz_nip_fi_disconnect_propagation_failures_total",
                            "admin",
                            "propagation_failure",
                            1,
                        );
                    }
                });
            }

            disconnected_response()
        }
        Err(err) => {
            if err == CommandError::DenySetFull {
                warn!("nip-fi disconnect: deny set full — command rejected, no sessions closed");
                count_disconnect_event(
                    state.config.nip_fi.mode,
                    "buzz_nip_fi_disconnect_capacity_rejections_total",
                    "admin",
                    "capacity",
                    1,
                );
            }
            command_denial(err)
        }
    }
}

// ── Startup component builder ─────────────────────────────────────────────────

/// Per-issuer command configuration parsed from the `BUZZ_NIP_FI_ISSUERS` JSON.
///
/// In enforce and shadow modes `maximum_command_age_seconds` and
/// `authorized_principals` are required on every issuer and startup
/// validation rejects any entry without them.  The fields remain `Option` for the off and `deny_protected`
/// modes, which do not require them.
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub struct CommandIssuerEnvConfig {
    /// Maximum command JWT age in seconds, in `[1, 60]`.  Required in enforce and shadow modes.
    pub maximum_command_age_seconds: Option<u64>,
    /// Non-empty list of authorized `sub` values, matched exactly
    /// (case-sensitive).  Required in enforce and shadow modes.
    pub authorized_principals: Option<Vec<String>>,
    /// Hard ceiling on live deny entries for this issuer; must be positive.
    /// Defaults to [`DEFAULT_DENY_SET_CAPACITY`] when absent.
    pub deny_set_capacity: Option<usize>,
}

/// The `NipFiDenyMap` + `CommandVerifier` pair built at startup.
pub struct NipFiCommandComponents<F: JwksFetcher = buzz_auth::HttpJwksFetcher> {
    /// The shared deny map consumed by WS admission and S5 HTTP admission.
    pub deny_map: Arc<NipFiDenyMap>,
    /// The command verifier for the `POST /api/nip-fi/disconnect` endpoint.
    pub command_verifier: Arc<CommandVerifier<Arc<ProductionJwksSource<F>>>>,
}

/// Channel this pod publishes accepted disconnects on.  Shadow uses its own
/// channel, so no enforce pod (of any build) ever acts on a shadow command.
pub fn disconnect_publish_channel(mode: NipFiMode) -> &'static str {
    if mode.observes_only() {
        buzz_pubsub::conn_control::NIP_FI_SHADOW_DISCONNECT_CHANNEL
    } else {
        buzz_pubsub::conn_control::NIP_FI_DISCONNECT_CHANNEL
    }
}

/// Channels this pod receives disconnects on.  Shadow also hears enforce's
/// real disconnects so its deny record matches.  An enforce pod hears shadow
/// disconnects only with `accept_shadow` (`BUZZ_NIP_FI_ACCEPT_SHADOW_DISCONNECTS`),
/// for the deploy that switches shadow → enforce.
pub fn disconnect_subscribe_channels(
    mode: NipFiMode,
    accept_shadow: bool,
) -> &'static [&'static str] {
    use buzz_pubsub::conn_control::{NIP_FI_DISCONNECT_CHANNEL, NIP_FI_SHADOW_DISCONNECT_CHANNEL};
    if mode.observes_only() || accept_shadow {
        &[NIP_FI_DISCONNECT_CHANNEL, NIP_FI_SHADOW_DISCONNECT_CHANNEL]
    } else {
        &[NIP_FI_DISCONNECT_CHANNEL]
    }
}

/// Stores this pod restores its deny map from, matching
/// [`disconnect_subscribe_channels`]: always the enforce store, plus the
/// shadow store on a shadow pod or with `accept_shadow`.
pub fn restore_sources(
    pool: buzz_pubsub::RedisConnection,
    mode: NipFiMode,
    accept_shadow: bool,
) -> Vec<Arc<dyn buzz_auth::CommandReplayGuard>> {
    let mut sources: Vec<Arc<dyn buzz_auth::CommandReplayGuard>> = vec![Arc::new(
        buzz_pubsub::RedisCommandReplayGuard::new(pool.clone()),
    )];
    if mode.observes_only() || accept_shadow {
        sources.push(Arc::new(buzz_pubsub::RedisCommandReplayGuard::shadow(pool)));
    }
    sources
}

/// Load every stored deny entry from `sources` into this pod's map.  `Ok`
/// only when every entry was recorded; a store error or a capacity miss is
/// `Err`, and the caller keeps admission closed.
///
/// Entries are merged first; sessions are closed afterwards, only for targets
/// the merge added or extended, on every exit, so a retry that finds an entry
/// unchanged never hides a close.  A capacity or poison miss gets the same
/// failsafe close as live delivery.  A failure does not end the pass: every
/// remaining entry, issuer and source is still merged or closed, and the
/// first failure is returned.
pub async fn restore_nip_fi_denies(
    state: &crate::state::AppState,
    sources: &[Arc<dyn buzz_auth::CommandReplayGuard>],
) -> Result<(), &'static str> {
    let Some(deny_map) = state.nip_fi_deny_map.as_deref() else {
        return Ok(());
    };
    let mut changed = Vec::new();
    let result = merge_stored_denies(state, deny_map, sources, &mut changed).await;
    for message in &changed {
        close_nip_fi_sessions(state, message, "restored");
    }
    result
}

async fn merge_stored_denies(
    state: &crate::state::AppState,
    deny_map: &buzz_auth::NipFiDenyMap,
    sources: &[Arc<dyn buzz_auth::CommandReplayGuard>],
    changed: &mut Vec<buzz_pubsub::NipFiDisconnect>,
) -> Result<(), &'static str> {
    use buzz_auth::CrossPodMergeResult;
    // Keep going past a failure so every reachable target is closed in this
    // pass; the first failure still fails the restore.
    let mut result = Ok(());
    for source in sources {
        for issuer in deny_map.issuers() {
            let now = chrono::Utc::now();
            let entries = match source.load_denies(&issuer, now).await {
                Ok(entries) => entries,
                Err(buzz_auth::LoadDeniesError::Store(_)) => {
                    result = result.and(Err("store"));
                    continue;
                }
                // Apply the records that decoded; the restore still fails.
                Err(buzz_auth::LoadDeniesError::Unreadable { readable, .. }) => {
                    result = result.and(Err("unreadable"));
                    readable
                }
            };
            for (key, entry) in entries {
                let message = buzz_pubsub::NipFiDisconnect {
                    issuer: issuer.clone(),
                    pubkey_bytes: key.to_bytes().to_vec(),
                    until_unix: entry.until.timestamp(),
                    until_unix_nanos: entry.until.timestamp_subsec_nanos(),
                    revoke_through: entry.scope.revoke_through(),
                };
                // Judge expiry at the merge, not at the read before it.
                match merge_nip_fi_disconnect(state, &message, chrono::Utc::now) {
                    NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Merged) => {
                        changed.push(message)
                    }
                    NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Unchanged) => {}
                    NipFiDisconnectApplyResult::Applied(merge) => {
                        close_after_merge(state, &message, merge);
                        result = result.and(Err("capacity"));
                    }
                    NipFiDisconnectApplyResult::Disabled | NipFiDisconnectApplyResult::Rejected => {
                        // Unreachable with consistent config; close defensively.
                        close_nip_fi_sessions(state, &message, "rejected");
                        result = result.and(Err("capacity"));
                    }
                }
            }
        }
    }
    result
}

/// The single owner of this pod's deny map restoration and cross-pod
/// consumer.
///
/// Admission (every `is_denied`) stays closed until a restore completes for
/// the current generation.  Each loss of the Redis subscription, and each
/// broadcast lag, opens a new generation and restores again, so a restore
/// overtaken by a newer subscription can never reopen admission.  Order per
/// generation: subscription confirmed → load stores → apply messages that
/// arrived meanwhile → reopen.  Never returns while the broadcast is open.
pub async fn run_nip_fi_disconnect_consumer(
    state: Arc<crate::state::AppState>,
    mut rx: tokio::sync::broadcast::Receiver<buzz_pubsub::NipFiDisconnect>,
    mut subscription: tokio::sync::watch::Receiver<buzz_pubsub::NipFiSubscription>,
    sources: Vec<Arc<dyn buzz_auth::CommandReplayGuard>>,
) {
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};
    let mode = state.config.nip_fi.mode;
    let Some(deny_map) = state.nip_fi_deny_map.clone() else {
        return;
    };
    let retry = std::time::Duration::from_secs(1);
    'generation: loop {
        let generation = deny_map.begin_restore();
        // Wait for a confirmed subscription.
        let epoch = loop {
            let current = *subscription.borrow_and_update();
            if current.live {
                break current.epoch;
            }
            if subscription.changed().await.is_err() {
                return;
            }
        };
        // Restore; retry while the subscription stays the same.
        while let Err(reason) = restore_nip_fi_denies(&state, &sources).await {
            metrics::counter!("buzz_nip_fi_deny_restore_failures_total", "reason" => reason)
                .increment(1);
            tracing::warn!(
                reason,
                "nip-fi: deny restore failed; admission stays closed"
            );
            // Keep closing revoked sessions while admission stays closed.
            // The deadline is fixed, so traffic cannot postpone the retry.
            let wait = tokio::time::sleep(retry);
            tokio::pin!(wait);
            loop {
                tokio::select! {
                    () = &mut wait => break,
                    msg = rx.recv() => match msg {
                        Ok(msg) => {
                            if !retained(apply_nip_fi_disconnect(&state, &msg, chrono::Utc::now)) {
                                continue 'generation;
                            }
                        }
                        Err(RecvError::Lagged(n)) => {
                            count_disconnect_event(mode, "buzz_nip_fi_disconnect_lag_total", "cross_pod", "lag", n);
                            continue 'generation;
                        }
                        Err(RecvError::Closed) => return,
                    },
                }
            }
            if subscription.borrow().epoch != epoch {
                continue 'generation;
            }
        }
        // Apply what arrived during the restore.
        loop {
            match rx.try_recv() {
                Ok(msg) => {
                    if !retained(apply_nip_fi_disconnect(&state, &msg, chrono::Utc::now)) {
                        continue 'generation;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(n)) => {
                    count_disconnect_event(
                        mode,
                        "buzz_nip_fi_disconnect_lag_total",
                        "cross_pod",
                        "lag",
                        n,
                    );
                    continue 'generation;
                }
                Err(TryRecvError::Closed) => return,
            }
        }
        if subscription.borrow().epoch != epoch || !deny_map.finish_restore(generation) {
            continue;
        }
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Ok(msg) => {
                        if !retained(apply_nip_fi_disconnect(&state, &msg, chrono::Utc::now)) {
                            tracing::warn!("NIP-FI deny not retained; closing admission and restoring");
                            continue 'generation;
                        }
                    }
                    Err(RecvError::Lagged(n)) => {
                        count_disconnect_event(mode, "buzz_nip_fi_disconnect_lag_total", "cross_pod", "lag", n);
                        tracing::warn!("NIP-FI disconnect consumer lagged by {n} messages; restoring");
                        continue 'generation;
                    }
                    Err(RecvError::Closed) => {
                        tracing::error!("NIP-FI disconnect broadcast channel closed");
                        return;
                    }
                },
                changed = subscription.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    continue 'generation;
                }
            }
        }
    }
}

/// Whether a consumed disconnect left the deny map complete.  A capacity miss
/// or poisoned shard did not record the deny, so admission must close and the
/// consumer restore until the active union fits.  A rejected message (unknown
/// issuer, bad key) carries no deny this pod could hold.
fn retained(result: NipFiDisconnectApplyResult) -> bool {
    !matches!(
        result,
        NipFiDisconnectApplyResult::Applied(
            buzz_auth::CrossPodMergeResult::CapacityExceeded
                | buzz_auth::CrossPodMergeResult::ShardPoisoned
        )
    )
}

/// Count a disconnect-path event on its `real` counter, or in shadow on
/// `buzz_nip_fi_shadow_disconnect_total{route, outcome}` instead, so a
/// shadow pod never moves an enforce disconnect metric.
pub fn count_disconnect_event(
    mode: NipFiMode,
    real: &'static str,
    route: &'static str,
    outcome: &'static str,
    n: u64,
) {
    if mode.observes_only() {
        let labels = [("route", route), ("outcome", outcome)];
        metrics::counter!("buzz_nip_fi_shadow_disconnect_total", &labels).increment(n);
    } else {
        metrics::counter!(real).increment(n);
    }
}

/// Command replay guard for this pod.  Shadow claims under its own key
/// prefix, so a shadow accept never uses up the enforce claim.
pub fn command_replay_guard(
    pool: buzz_pubsub::RedisConnection,
    mode: NipFiMode,
) -> Arc<dyn buzz_auth::CommandReplayGuard> {
    if mode.observes_only() {
        Arc::new(buzz_pubsub::RedisCommandReplayGuard::shadow(pool))
    } else {
        Arc::new(buzz_pubsub::RedisCommandReplayGuard::new(pool))
    }
}

/// Outcome of applying a cross-pod NIP-FI disconnect message.
///
/// Returned by [`apply_nip_fi_disconnect`]; used by the `main.rs` receive loop
/// and by tests to assert the production decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NipFiDisconnectApplyResult {
    /// Command API is not enabled on this pod (deny map absent); message ignored.
    Disabled,
    /// Message was rejected before reaching the map (invalid pubkey, unknown
    /// issuer, or unrepresentable timestamp).  An `until` beyond this pod's
    /// ceiling is not a rejection: it is clamped to the ceiling and applied.
    Rejected,
    /// Message was applied; carries the map's merge result.
    Applied(buzz_auth::CrossPodMergeResult),
}

/// Build the [`buzz_pubsub::NipFiDisconnect`] bus message from a successfully
/// verified command.
///
/// This is the single publisher mapping — the HTTP success path calls this
/// function to ensure the nanos precision is always captured correctly.
/// Reverting either the seconds or the nanos field must red the round-trip oracle.
pub fn nip_fi_disconnect_message(cmd: &buzz_auth::CommandResult) -> buzz_pubsub::NipFiDisconnect {
    buzz_pubsub::NipFiDisconnect {
        issuer: cmd.caller_iss.clone(),
        pubkey_bytes: cmd.target_pubkey.to_bytes().to_vec(),
        until_unix: cmd.until.timestamp(),
        until_unix_nanos: cmd.until.timestamp_subsec_nanos(),
        revoke_through: cmd.scope.revoke_through(),
    }
}

/// Apply a received cross-pod NIP-FI disconnect message against the local
/// deny map and connection registry.
///
/// This is the single consumer path — the `main.rs` receive loop calls this
/// function after receiving a message from the broadcast channel.  Extracting
/// the logic here allows tests to call the exact production path end-to-end
/// without driving a live Redis subscriber.
///
/// `until` is clamped to `now + skew_seconds + maximum_assertion_age_seconds`
/// for the message's issuer, so clock drift between pods cannot drop a command
/// the origin pod already accepted.
///
/// `now` is passed explicitly so tests can supply controlled timestamps.
pub fn apply_nip_fi_disconnect(
    state: &crate::state::AppState,
    message: &buzz_pubsub::NipFiDisconnect,
    now: impl Fn() -> chrono::DateTime<chrono::Utc>,
) -> NipFiDisconnectApplyResult {
    let result = merge_nip_fi_disconnect(state, message, now);
    if let NipFiDisconnectApplyResult::Applied(merge) = result {
        close_after_merge(state, message, merge);
    }
    result
}

/// Validate, clamp and merge one disconnect into the deny map; closes nothing.
fn merge_nip_fi_disconnect(
    state: &crate::state::AppState,
    message: &buzz_pubsub::NipFiDisconnect,
    now: impl Fn() -> chrono::DateTime<chrono::Utc>,
) -> NipFiDisconnectApplyResult {
    let deny_map = match state.nip_fi_deny_map.as_deref() {
        Some(m) => m,
        None => return NipFiDisconnectApplyResult::Disabled,
    };

    // Validate pubkey bytes.
    let pubkey = match nostr::PublicKey::from_slice(&message.pubkey_bytes) {
        Ok(k) => k,
        Err(_) => {
            tracing::warn!(
                len = message.pubkey_bytes.len(),
                "nip-fi cross-pod: malformed pubkey bytes — rejected"
            );
            return NipFiDisconnectApplyResult::Rejected;
        }
    };

    // Validate that the issuer is locally configured.
    let Some(policy) = state
        .config
        .nip_fi
        .registry
        .policy_for_issuer(&message.issuer)
    else {
        tracing::warn!("nip-fi cross-pod: unknown issuer (not locally configured) — rejected");
        return NipFiDisconnectApplyResult::Rejected;
    };

    // Validate timestamp representability.
    let mut until =
        match chrono::DateTime::from_timestamp(message.until_unix, message.until_unix_nanos) {
            Some(t) => t,
            None => {
                tracing::warn!(
                    until_unix = message.until_unix,
                    "nip-fi cross-pod: unrepresentable until timestamp — rejected"
                );
                return NipFiDisconnectApplyResult::Rejected;
            }
        };
    if let Some(c) = message.revoke_through {
        if chrono::DateTime::from_timestamp(c, 0).is_none() {
            tracing::warn!(
                revoke_through = c,
                "nip-fi cross-pod: unrepresentable revoke_through — rejected"
            );
            return NipFiDisconnectApplyResult::Rejected;
        }
    }

    // Clamp `until` to this pod's ceiling rather than rejecting it.  The origin
    // pod already verified `until` against its own clock and returned 200; a
    // pod whose clock trails the origin would otherwise drop a legitimate
    // at-ceiling command and leave the target's sessions open.
    let skew = chrono::Duration::seconds(policy.skew_seconds() as i64);
    let max_age = chrono::Duration::seconds(policy.maximum_assertion_age_seconds() as i64);
    if let Some(ceiling) = now()
        .checked_add_signed(skew)
        .and_then(|t| t.checked_add_signed(max_age))
    {
        until = until.min(ceiling);
    }

    let entry = buzz_auth::DenyEntry {
        until,
        scope: buzz_auth::DenyScope::from_revoke_through(message.revoke_through),
    };
    NipFiDisconnectApplyResult::Applied(deny_map.merge_cross_pod_deny(
        &message.issuer,
        &pubkey,
        entry,
        &now,
    ))
}

/// Close (or, in shadow, record would-close for) the sessions of the
/// message's target that its scope covers, widened by any stronger active
/// entry the deny map holds for it.
fn close_nip_fi_sessions(
    state: &crate::state::AppState,
    message: &buzz_pubsub::NipFiDisconnect,
    reason: &str,
) {
    let command = buzz_auth::DenyScope::from_revoke_through(message.revoke_through);
    let scope = match (
        state.nip_fi_deny_map.as_deref(),
        nostr::PublicKey::from_slice(&message.pubkey_bytes),
    ) {
        (Some(deny_map), Ok(pubkey)) => {
            deny_map.close_scope(&message.issuer, &pubkey, command, chrono::Utc::now())
        }
        _ => command,
    };
    if state.config.nip_fi.mode.observes_only() {
        let sessions = &state.nip_fi_shadow_sessions;
        return sessions.would_close(&message.issuer, &message.pubkey_bytes, scope);
    }
    let closed =
        state
            .conn_manager
            .disconnect_nip_fi(&message.issuer, &message.pubkey_bytes, scope)
            + state.community_connections.disconnect_nip_fi(
                &message.issuer,
                &message.pubkey_bytes,
                scope,
            );
    if closed > 0 {
        tracing::debug!(closed, reason = reason, "nip-fi cross-pod: closed sessions");
    }
}

/// Close the target's sessions for every merge outcome except
/// `UnknownIssuer`, and log/metric the failsafe outcomes.
fn close_after_merge(
    state: &crate::state::AppState,
    message: &buzz_pubsub::NipFiDisconnect,
    merge_result: buzz_auth::CrossPodMergeResult,
) {
    use buzz_auth::CrossPodMergeResult;
    let mode = state.config.nip_fi.mode;
    // What a capacity or poison failsafe does to targeted sessions here.
    let action = if mode.observes_only() {
        "would-close recorded (shadow)"
    } else {
        "targeted sessions closed"
    };
    match merge_result {
        CrossPodMergeResult::Merged | CrossPodMergeResult::Unchanged => {
            close_nip_fi_sessions(state, message, "merged");
        }
        CrossPodMergeResult::UnknownIssuer => {
            tracing::warn!("nip-fi cross-pod: merge returned UnknownIssuer — rejected");
        }
        CrossPodMergeResult::CapacityExceeded => {
            tracing::warn!(
                action,
                "nip-fi cross-pod: deny set full for issuer — no map entry (capacity miss; issuer re-push is the recovery path)"
            );
            close_nip_fi_sessions(state, message, "capacity-exceeded");
            let real = "buzz_nip_fi_cross_pod_capacity_exceeded_total";
            count_disconnect_event(mode, real, "cross_pod", "capacity", 1);
        }
        CrossPodMergeResult::ShardPoisoned => {
            tracing::error!(
                action,
                "nip-fi cross-pod: issuer shard is poisoned (fail-closed)"
            );
            close_nip_fi_sessions(state, message, "poisoned shard failsafe");
            let real = "buzz_nip_fi_cross_pod_shard_poison_total";
            count_disconnect_event(mode, real, "cross_pod", "poison", 1);
        }
    }
}

/// Build the NIP-FI command components from the issuer policies and key source.
///
/// Called by `install_nip_fi_command_components` (and transitively `main.rs`).
/// Returns `Err` when any command config is invalid.  In enforce mode, returns
/// `Err` when no command-capable issuers are present (assertion-only enforce is
/// not supported by this PR; every enforce issuer must carry command config).
///
/// `issuer_command_configs` must be in the same order as `registry.all_policies()`.
pub fn build_nip_fi_command_components<F: JwksFetcher>(
    mode: NipFiMode,
    registry: &buzz_auth::IssuerRegistry,
    key_source: Arc<ProductionJwksSource<F>>,
    issuer_command_configs: &[(String, CommandIssuerEnvConfig)],
) -> Result<Option<NipFiCommandComponents<F>>, String> {
    if mode.is_off() {
        return Ok(None);
    }

    // Build per-issuer command policies and capacity overrides.
    let mut command_policies: Vec<CommandIssuerPolicy> = Vec::new();
    let mut issuer_capacities: Vec<IssuerCapacity> = Vec::new();
    let mut default_capacity = DEFAULT_DENY_SET_CAPACITY;

    for (idx, (issuer, cmd_cfg)) in issuer_command_configs.iter().enumerate() {
        let age = match cmd_cfg.maximum_command_age_seconds {
            Some(a) => a,
            None => {
                // In enforce mode every issuer must be command-capable;
                // from_env() already guarantees this, but be defensive here too.
                if mode.evaluates() {
                    return Err(format!(
                        "nip-fi: enforce issuer [index {idx}] has no maximum_command_age_seconds — \
                         assertion-only issuers are not supported in enforce mode"
                    ));
                }
                continue; // non-enforce mode: skip issuers without command config
            }
        };
        let principals = match &cmd_cfg.authorized_principals {
            Some(p) if !p.is_empty() => p.clone(),
            _ => {
                // from_env() already rejects this; treat as a hard error here.
                return Err(format!(
                    "nip-fi: issuer [index {idx}] has maximum_command_age_seconds but no \
                     authorized_principals — startup validation should have caught this"
                ));
            }
        };
        let capacity = cmd_cfg
            .deny_set_capacity
            .unwrap_or(DEFAULT_DENY_SET_CAPACITY);

        // Validate and construct the command policy — no warn-and-skip.
        let policy = CommandIssuerPolicy::new(issuer.clone(), age, principals, capacity)
            .map_err(|e| format!("nip-fi: issuer [index {idx}] invalid command policy: {e}"))?;

        issuer_capacities.push(IssuerCapacity {
            issuer: issuer.clone(),
            capacity,
        });
        command_policies.push(policy);

        // Track the maximum capacity across issuers for the default slot.
        if capacity > default_capacity {
            default_capacity = capacity;
        }
    }

    if command_policies.is_empty() {
        if mode.evaluates() {
            // Enforce with no command-capable issuers is a misconfiguration:
            // from_env() guarantees every enforce issuer has command config, so
            // an empty set here means something was skipped or the configs are wrong.
            return Err(
                "nip-fi: enforce mode requires at least one command-capable issuer; \
                 no command policies were built — check issuer configuration"
                    .to_owned(),
            );
        }
        debug!("nip-fi: no command-capable issuers configured — command API disabled");
        return Ok(None);
    }

    let deny_map = Arc::new(NipFiDenyMap::new(default_capacity, issuer_capacities));

    let command_verifier = Arc::new(CommandVerifier::new(
        registry.clone(),
        key_source,
        command_policies,
        (*deny_map).clone(),
    ));

    Ok(Some(NipFiCommandComponents {
        deny_map,
        command_verifier,
    }))
}

/// Result of a successful [`install_nip_fi_command_components`] call.
#[derive(Debug)]
pub struct NipFiCommandStartupReport {
    /// Number of issuers wired into the command verifier.
    pub command_issuers: usize,
}

/// Install NIP-FI command components into the two `AppState` slots.
///
/// This is the single production startup seam that owns:
/// - enforce-mode pre-flight check (returns `Err` for incomplete config)
/// - `build_nip_fi_command_components` invocation
/// - assignment of both `nip_fi_deny_map` and `nip_fi_command_verifier`
///
/// It performs no JWKS I/O. `main.rs` owns the single warm + per-issuer
/// refresh lifecycle for the shared `key_source`; the command verifier reads
/// that cache lazily at verify time. `main.rs` passes
/// `&mut app_state.nip_fi_deny_map` and `&mut app_state.nip_fi_command_verifier`;
/// the slots are generic over the fetcher so tests can count fetches.
pub fn install_nip_fi_command_components<F: JwksFetcher>(
    deny_map_slot: &mut Option<Arc<NipFiDenyMap>>,
    command_verifier_slot: &mut Option<Arc<CommandVerifier<Arc<ProductionJwksSource<F>>>>>,
    mode: NipFiMode,
    registry: &buzz_auth::IssuerRegistry,
    key_source: Arc<ProductionJwksSource<F>>,
    command_configs: &[(String, CommandIssuerEnvConfig)],
) -> Result<NipFiCommandStartupReport, String> {
    // Pre-flight: enforce mode with no command configs is always an error.
    if mode.evaluates() && command_configs.is_empty() {
        return Err(
            "NIP-FI install: enforce mode requires at least one command-capable issuer".to_owned(),
        );
    }

    let components = build_nip_fi_command_components(mode, registry, key_source, command_configs)?;

    let command_issuers = if let Some(c) = components {
        let n = command_configs.len();
        *deny_map_slot = Some(c.deny_map);
        *command_verifier_slot = Some(c.command_verifier);
        tracing::info!("NIP-FI S4: command API enabled ({n} issuer(s))");
        n
    } else {
        0
    };

    Ok(NipFiCommandStartupReport { command_issuers })
}

/// Validate a command issuer config entry without constructing a policy.
///
/// Called by `nip_fi_config.rs` at startup before `build_nip_fi_command_components`
/// so that invalid config is rejected at `Config::from_env()`, not at serve time.
/// Returns `Err` with a non-sensitive message (no raw issuer URI).
pub fn validate_command_issuer_config(
    idx: usize,
    age_seconds: u64,
    principals: &[String],
    capacity: usize,
) -> Result<(), String> {
    CommandIssuerPolicy::new(
        // Use a sentinel issuer for validation only — no URI written to any log.
        format!("https://validate-sentinel-{idx}.internal"),
        age_seconds,
        principals.to_vec(),
        capacity,
    )
    .map(|_| ())
    .map_err(|e| format!("issuer [index {idx}] invalid command policy: {e}"))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_hex_pubkey(raw: &str) -> Option<nostr::PublicKey> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return None;
    }
    nostr::PublicKey::from_hex(raw).ok()
}

fn plain_response(status: StatusCode, body: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| status.into_response())
}

/// Render a command rejection with its spec-exact status and body.
fn command_denial(err: CommandError) -> Response<Body> {
    let status =
        StatusCode::from_u16(err.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    plain_response(status, err.response_body())
}

/// Spec-exact 200 success response.
///
/// The spec body is `{"disconnected": true}` (note the space after `:`).
/// `serde_json::to_vec` produces `{"disconnected":true}` without the space.
/// We produce the literal bytes directly to stay byte-exact.
fn disconnected_response() -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from("{\"disconnected\": true}"))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Hex pubkey parsing ────────────────────────────────────────────────

    #[test]
    fn uppercase_hex_rejected() {
        let upper = "A".repeat(64);
        assert!(parse_hex_pubkey(&upper).is_none());
    }

    #[test]
    fn wrong_length_rejected() {
        let short = "a".repeat(63);
        let long = "a".repeat(65);
        assert!(parse_hex_pubkey(&short).is_none());
        assert!(parse_hex_pubkey(&long).is_none());
    }

    // ── CommandIssuerEnvConfig default capacity ────────────────────────────

    // (The previous constant-assertion test was removed: asserting None and a constant
    // does not bind production behavior. The builder is now covered by route integration tests.)

    // ── HTTP response contract ─────────────────────────────────────────────

    /// The spec requires `{"disconnected": true}` (note the space after `:`).
    #[tokio::test]
    async fn disconnected_response_is_spec_exact() {
        let resp = disconnected_response();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(ct, "application/json");
        // Body bytes are verified directly — serde_json compact and the spec
        // literal are NOT the same (serde_json omits the space).
        let body_bytes = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(
            body_bytes.as_ref(),
            b"{\"disconnected\": true}",
            "success body must be byte-exact per spec"
        );
    }

    /// `403` error responses MUST NOT carry `WWW-Authenticate`.
    #[test]
    fn error_responses_have_no_www_authenticate() {
        for body in &["evidence rejected\n", "authorization denied\n"] {
            let resp = plain_response(StatusCode::FORBIDDEN, body);
            assert!(
                resp.headers().get("WWW-Authenticate").is_none(),
                "403 must not carry WWW-Authenticate"
            );
        }
    }

    /// `503` plain responses have the spec-exact body.
    #[test]
    fn deny_set_full_response_body_is_spec_exact() {
        use buzz_auth::CommandError;
        let body = CommandError::DenySetFull.response_body();
        assert_eq!(
            body, "deny set full\n",
            "FI-TRACE-DENY-SET: 503 body must be 'deny set full\\n'"
        );
    }
}

// ── Route integration tests ────────────────────────────────────────────────────
//
// Exercises `disconnect()` through the full axum router with a warmed
// ProductionJwksSource, a real CommandVerifier, and an AppState wired exactly
// as production does (nip_fi_command_verifier + nip_fi_deny_map both set).
//
// These tests call the route at POST /api/nip-fi/disconnect via oneshot and
// verify every spec response row: 401, 403 (evidence), 403 (authz), 400, 503
// (capacity), 200 exact bytes.  The startup-assembly invariant is also
// verified: the tests GO RED if either state field is absent (503 unavailable).

#[cfg(test)]
mod route_integration_tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use buzz_auth::{
        CommandIssuerPolicy, CommandVerifier, IssuerCapacity, IssuerRegistry, NipFiDenyMap,
        ProductionJwksSource, CLIENT_ATTACHED_HEADER,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    // ── Shared test key material ────────────────────────────────────────────

    // ES256 key pair — same material as command.rs tests, known-good.
    const TEST_PRIVATE_KEY_PEM: &str =
        "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgcnxDM4EiirH9dHUE\nWZc759TX4s5PAn8kO5ovXSnGxCWhRANCAARFb6ZnsfkqOOXyEhj3KBQphGKF4vTa\nzhebbavbZ1ZoklqkF1cGg+jTO7rONAVEzXvXUWtV6CdDV+rybiVmFP2w\n-----END PRIVATE KEY-----\n"; // sadscan:disable np.pem.1 kingfisher.privkey.2 -- unchanged public ES256 test fixture

    const TEST_ISS: &str = "https://idp.test.example.com";
    const TEST_AUD: &str = "https://relay.test.example.com";
    const TEST_SUB: &str = "admin-svc@test.example.com";
    const TEST_PATH: &str = "/api/nip-fi/disconnect";

    // Key ID used in both the JWKS and the JWT header.
    const TEST_KID: &str = "route-test-key-1";

    fn test_public_jwk() -> jsonwebtoken::jwk::Jwk {
        // Public-key coordinates extracted from TEST_PRIVATE_KEY_PEM (P-256),
        // which is the same key pair as command.rs TEST_JWK_X/Y constants.
        serde_json::from_value(serde_json::json!({
            "kty": "EC",
            "crv": "P-256",
            "x": "RW-mZ7H5Kjjl8hIY9ygUKYRiheL02s4Xm22r22dWaJI",
            "y": "WqQXVwaD6NM7us40BUTNe9dRa1XoJ0NX6vJuJWYU_bA",
            "alg": "ES256",
            "use": "sig",
            "kid": TEST_KID
        }))
        .expect("valid test JWK")
    }

    fn test_jwks() -> jsonwebtoken::jwk::JwkSet {
        jsonwebtoken::jwk::JwkSet {
            keys: vec![test_public_jwk()],
        }
    }

    fn test_issuer_policy() -> buzz_auth::IssuerPolicy {
        issuer_policy(TEST_ISS)
    }

    fn issuer_policy(iss: &str) -> buzz_auth::IssuerPolicy {
        use buzz_auth::{FreshnessClass, IssuerPolicy, JwksSourceContract, TokenClass};
        let contract = JwksSourceContract::new(format!("{iss}/.well-known/jwks.json"), 300, 86400)
            .expect("valid JWKS contract");
        IssuerPolicy::new(
            iss.to_owned(),
            vec![TEST_AUD.to_owned()],
            TokenClass::DedicatedNipFi,
            FreshnessClass::OfflineJwt,
            vec![jsonwebtoken::Algorithm::ES256],
            30,
            3600,
            None,
            contract,
        )
        .expect("valid issuer policy")
    }

    fn test_jwks_config() -> buzz_auth::IssuerJwksConfig {
        jwks_config(TEST_ISS)
    }

    fn jwks_config(iss: &str) -> buzz_auth::IssuerJwksConfig {
        use buzz_auth::{IssuerJwksConfig, JwksSourceContract};
        let contract = JwksSourceContract::new(format!("{iss}/.well-known/jwks.json"), 300, 86400)
            .expect("valid JWKS contract");
        IssuerJwksConfig {
            issuer: iss.to_owned(),
            contract,
        }
    }

    fn mint_token(target_hex: &str, until_offset_secs: i64, extra: serde_json::Value) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let now = chrono::Utc::now().timestamp();
        let mut claims = serde_json::json!({
            "iss": TEST_ISS,
            "aud": TEST_AUD,
            "sub": TEST_SUB,
            "iat": now,
            "exp": now + 60,
            "jti": uuid::Uuid::new_v4().to_string(),
            "method": "POST",
            "path": TEST_PATH,
            "cmd": "disconnect",
            "target_pubkey": target_hex,
            "until": now + until_offset_secs,
        });
        if let Some(obj) = extra.as_object() {
            for (k, v) in obj {
                claims[k] = v.clone();
            }
        }
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("nip-fi-command+jwt".to_owned());
        header.kid = Some(TEST_KID.to_owned());
        let key = EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("test EC key");
        encode(&header, &claims, &key).expect("sign test token")
    }

    async fn build_test_state(capacity: usize) -> Arc<crate::state::AppState> {
        Arc::new(build_test_app_state(capacity, crate::config::Config::for_test()).await)
    }

    async fn build_test_app_state(
        capacity: usize,
        config: crate::config::Config,
    ) -> crate::state::AppState {
        // Build a minimal AppState with NIP-FI S4 components wired.
        // Uses lazy/invalid DB+Redis — only nip_fi fields and conn_manager matter.
        use crate::state::AppState;

        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).expect("redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("pubsub"),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).expect("media storage");
        let (mut state, _audit_shutdown) = AppState::new(
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

        // Wire NIP-FI S4 components.
        let jwks_configs = vec![test_jwks_config()];
        let key_source = Arc::new(
            ProductionJwksSource::new(jwks_configs, buzz_auth::HttpJwksFetcher::new())
                .expect("key source"),
        );
        // Seed the snapshot without making an HTTP request.
        key_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;

        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());

        let deny_map = Arc::new(NipFiDenyMap::new(
            capacity,
            vec![IssuerCapacity {
                issuer: TEST_ISS.to_owned(),
                capacity,
            }],
        ));
        let policy =
            CommandIssuerPolicy::new(TEST_ISS.to_owned(), 30, vec![TEST_SUB.to_owned()], capacity)
                .expect("command policy");
        let verifier = Arc::new(CommandVerifier::new(
            registry,
            Arc::clone(&key_source),
            vec![policy],
            (*deny_map).clone(),
        ));

        state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
        state.nip_fi_command_verifier = Some(verifier);
        state.nip_fi_command_replay = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        state
    }

    fn target_hex() -> String {
        nostr::Keys::generate().public_key().to_hex()
    }

    async fn do_request(
        state: Arc<crate::state::AppState>,
        method: &str,
        headers: Vec<(&'static str, String)>,
        body: Option<serde_json::Value>,
    ) -> axum::response::Response {
        use crate::router::build_router;
        let body_bytes = match body {
            Some(v) => serde_json::to_vec(&v).unwrap().into(),
            None => axum::body::Bytes::new(),
        };
        let mut req = Request::builder().method(method).uri(TEST_PATH);
        for (k, v) in &headers {
            req = req.header(*k, v.as_str());
        }
        let req = req.body(Body::from(body_bytes)).unwrap();
        build_router(state).oneshot(req).await.unwrap()
    }

    // ── Test: no verifier → 503 (startup-assembly invariant) ─────────────────

    #[tokio::test]
    async fn absent_verifier_gives_503_unavailable() {
        // If nip_fi_command_verifier is not set, every request gets 503.
        // This tests the handler fallback path when the verifier is absent from
        // AppState.  The production startup assembly is covered separately by
        // `production_assembly_build_nip_fi_command_components_wires_both_fields`.
        // Build a state without the verifier.
        let no_verifier_state = {
            let config = crate::config::Config::for_test();
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (state, _) = crate::state::AppState::new(
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
            // nip_fi_command_verifier stays None.
            Arc::new(state)
        };
        let target = target_hex();
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            no_verifier_state,
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    // ── Test: absent header → 401 + WWW-Authenticate ─────────────────────────

    #[tokio::test]
    async fn absent_header_route_gives_401_with_www_authenticate() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        let resp = do_request(
            state,
            "POST",
            vec![("Content-Type", "application/json".into())],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let www_auth = resp
            .headers()
            .get("WWW-Authenticate")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(www_auth, "Nostr", "401 MUST carry WWW-Authenticate: Nostr");
    }

    // ── Test: bad signature → 403 evidence rejected ───────────────────────────

    #[tokio::test]
    async fn bad_signature_gives_403_evidence_rejected() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        // Tamper the token.
        let token = mint_token(&target, 300, serde_json::json!({}));
        let tampered = format!("{token}X");
        let resp = do_request(
            state,
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {tampered}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ── Test: capacity exceeded → 503 does NOT burn jti ──────────────────────

    #[tokio::test]
    async fn capacity_503_does_not_burn_jti_route_retry_succeeds() {
        // capacity=1, two distinct targets.
        let state = build_test_state(1).await;
        let target_a = target_hex();
        let target_b = target_hex();

        // First request fills the slot.
        let token_a = mint_token(&target_a, 300, serde_json::json!({}));
        let resp_a = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token_a}")),
            ],
            Some(serde_json::json!({"pubkey": target_a})),
        )
        .await;
        assert_eq!(resp_a.status(), StatusCode::OK);

        // Second request hits capacity → 503.  Jti NOT burned.
        let jti_b = uuid::Uuid::new_v4().to_string();
        let token_b = mint_token(&target_b, 300, serde_json::json!({"jti": jti_b}));
        let resp_b = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token_b}")),
            ],
            Some(serde_json::json!({"pubkey": target_b})),
        )
        .await;
        assert_eq!(
            resp_b.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "capacity exceeded must return 503"
        );
        let body = axum::body::to_bytes(resp_b.into_body(), 64).await.unwrap();
        assert_eq!(body.as_ref(), b"deny set full\n");
        // Jti was NOT burned: the same token_b can be reused once the slot frees.
        // (Route-level: we verify the 503 body; the jti non-burn is covered by
        // command.rs::capacity_503_does_not_burn_jti_retry_succeeds_after_slot_freed)
    }

    // ── Test: successful disconnect → 200 spec-exact bytes ───────────────────

    #[tokio::test]
    async fn success_response_is_spec_exact_bytes() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(ct, "application/json");
        let body = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(
            body.as_ref(),
            b"{\"disconnected\": true}",
            "200 body must be byte-exact per spec (note the space after ':')"
        );
    }

    // ── Test: count-independence (zero vs many sessions) ─────────────────────

    #[tokio::test]
    async fn success_body_identical_regardless_of_sessions_closed() {
        // Zero live sessions: response must still be {"disconnected": true}.
        let state = build_test_state(1000).await;
        let target = target_hex();
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            state,
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 64).await.unwrap();
        assert_eq!(
            body.as_ref(),
            b"{\"disconnected\": true}",
            "zero-sessions success must be byte-identical to many-sessions success [no count leak]"
        );
    }

    // ── Test: deny entry recorded after success ───────────────────────────────

    #[tokio::test]
    async fn success_records_deny_entry_visible_to_is_denied() {
        let state = build_test_state(1000).await;
        let target = target_hex();
        let target_pubkey = nostr::PublicKey::from_hex(&target).expect("valid hex pubkey");

        // Before disconnect: not denied.
        let deny_map = state.nip_fi_deny_map.as_deref().expect("deny map present");
        assert!(
            !deny_map.is_denied(TEST_ISS, &target_pubkey, chrono::Utc::now()),
            "must not be denied before disconnect"
        );

        // Execute disconnect.
        let token = mint_token(&target, 300, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        // After disconnect: denied.
        assert!(
            deny_map.is_denied(TEST_ISS, &target_pubkey, chrono::Utc::now()),
            "must be denied after successful disconnect"
        );
    }

    // ── Test: consumer capacity oracle (replacement per NIP-FI.md:306-336) ─────
    //
    // Drives apply_nip_fi_disconnect with a delivered target that encounters a
    // pre-filled capacity-1 map.  Asserts Applied(CapacityExceeded); the
    // pre-existing map key remains denied only until its TTL; the missed target
    // and unrelated key are NOT map-denied; targeted live sessions are closed;
    // unrelated live peers remain open.
    //
    // Mandatory reds:
    //  (a) consumer stops calling merge_cross_pod_deny → Applied(CapacityExceeded) missed;
    //      targeted session close assertion fails
    //  (b) reintroduce issuer-wide blocking → missed-target is_denied assertion fails
    //  (c) consumer skips close_sessions on CapacityExceeded → targeted cancel assertion fails

    #[tokio::test]
    async fn consumer_capacity_miss_closes_targeted_session_without_map_denial() {
        use super::apply_nip_fi_disconnect;
        use super::NipFiDisconnectApplyResult;
        use crate::state::CommunityConnectionControl;
        use buzz_auth::CrossPodMergeResult;
        use tokio_util::sync::CancellationToken;

        let state = {
            let mut config = crate::config::Config::for_test();
            // Wire TEST_ISS into the NIP-FI registry so the consumer seam accepts it.
            config.nip_fi.registry.insert(test_issuer_policy());
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (mut state, _) = crate::state::AppState::new(
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
            // Capacity=1, pre-filled with k_a so the second delivery (k_b) hits capacity.
            let deny_map = Arc::new(buzz_auth::NipFiDenyMap::new(
                1,
                vec![buzz_auth::IssuerCapacity {
                    issuer: TEST_ISS.to_owned(),
                    capacity: 1,
                }],
            ));
            state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
            Arc::new(state)
        };

        let now = chrono::Utc::now();
        let until_unix = (now + chrono::Duration::seconds(300)).timestamp();

        let k_a = nostr::Keys::generate().public_key();
        let k_b = nostr::Keys::generate().public_key();
        let k_unrelated = nostr::Keys::generate().public_key();

        // Pre-fill slot with k_a via the consumer seam.
        let msg_a = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: k_a.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
            revoke_through: None,
        };
        let result_a = apply_nip_fi_disconnect(&state, &msg_a, || now);
        assert_eq!(
            result_a,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Merged),
            "first consumer message must merge"
        );

        // Register live sessions for targeted (k_b) and unrelated (k_unrelated) peers.
        let cancel_b = CancellationToken::new();
        let cancel_unrelated = CancellationToken::new();
        let registry = &state.community_connections;
        let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::new_v4());

        let ctrl_b = CommunityConnectionControl::new(cancel_b.clone());
        ctrl_b.set_proven_identity(
            k_b.to_bytes().to_vec(),
            Some(crate::nip_fi_core::NipFiSession::new(TEST_ISS, None)),
        );
        let _guard_b = registry.register(uuid::Uuid::new_v4(), community, ctrl_b);

        let ctrl_unrelated = CommunityConnectionControl::new(cancel_unrelated.clone());
        ctrl_unrelated.set_proven_identity(
            k_unrelated.to_bytes().to_vec(),
            Some(crate::nip_fi_core::NipFiSession::new(TEST_ISS, None)),
        );
        let _guard_unrelated = registry.register(uuid::Uuid::new_v4(), community, ctrl_unrelated);

        // Deliver k_b — capacity exhausted, no map entry added.
        let msg_b = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: k_b.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
            revoke_through: None,
        };
        let result_b = apply_nip_fi_disconnect(&state, &msg_b, || now);
        assert_eq!(
            result_b,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::CapacityExceeded),
            "second consumer message must hit capacity"
        );

        // Targeted session (k_b) must be cancelled despite no map entry.
        assert!(
            cancel_b.is_cancelled(),
            "targeted session must be closed even on CapacityExceeded"
        );
        // Unrelated session must NOT be cancelled.
        assert!(
            !cancel_unrelated.is_cancelled(),
            "unrelated session must remain open after capacity miss"
        );

        // Map checks — no issuer-wide denial synthesized.
        let deny_map = state.nip_fi_deny_map.as_deref().expect("deny map present");

        // k_a is still denied (its entry was not evicted).
        assert!(
            deny_map.is_denied(TEST_ISS, &k_a, now),
            "pre-existing k_a entry must remain denied"
        );
        // k_b has no map entry — NOT denied via the map.
        assert!(
            !deny_map.is_denied(TEST_ISS, &k_b, now),
            "missed target k_b must NOT be map-denied after capacity miss"
        );
        // Unrelated key is not map-denied.
        assert!(
            !deny_map.is_denied(TEST_ISS, &k_unrelated, now),
            "unrelated key must NOT be denied after capacity miss"
        );
        // At exact equality with k_a's TTL, k_a is admitted.
        let at_ttl = chrono::DateTime::from_timestamp(until_unix, 0).unwrap();
        assert!(
            !deny_map.is_denied(TEST_ISS, &k_a, at_ttl),
            "k_a must be admitted at exact equality with its TTL"
        );
    }

    // ── Test: Item 5 — dual-transport registration witness ──────────────────────────────────────
    //
    // Proves that the production audio post-auth registration helper
    // (`audio_post_auth_register`) is the seam used to register audio connections
    // in the fan-out, and that apply_nip_fi_disconnect drives both connection
    // registries (ordinary WS via conn_manager and audio via community_connections).
    //
    // Four sub-claims verified:
    //  1. targeted ordinary WS is cancelled (conn_manager path)
    //  2. targeted audio is cancelled with AuthorizationDenied (community_connections path,
    //     registered via the production audio_post_auth_register helper)
    //  3. unrelated ordinary WS and audio peers remain open
    //  4. capacity-failure variant: the delivered target still closes both transports
    //     despite no map entry
    //
    // Mandatory reds:
    //  - no-op audio_post_auth_register leaves targeted audio open
    //  - removing community_connections from the fan-out leaves audio open
    //  - removing conn_manager from the fan-out leaves ordinary WS open
    //  - broad/non-key-exact matching would close unrelated peers (asserted absent)
    //  - skipping close on CapacityExceeded leaves targeted sessions open (capacity variant)

    #[tokio::test]
    async fn dual_transport_registration_witness() {
        use super::apply_nip_fi_disconnect;
        use super::NipFiDisconnectApplyResult;
        use crate::audio::handler::audio_post_auth_register;
        use crate::state::CommunityConnectionControl;
        use buzz_auth::CrossPodMergeResult;
        use tokio::sync::mpsc;
        use tokio_util::sync::CancellationToken;
        use uuid::Uuid;

        let community = buzz_core::tenant::CommunityId::from_uuid(Uuid::new_v4());

        // ── Helper: register an ordinary WS connection and return (conn_id, cancel). ──
        // Mirrors how connection.rs registers after NIP-42 auth: register first, then
        // call set_authenticated_pubkey.
        let register_ws =
            |state: &crate::state::AppState, pubkey_bytes: Vec<u8>| -> (Uuid, CancellationToken) {
                let conn_id = Uuid::new_v4();
                let (tx, _rx) = mpsc::channel(8);
                let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
                let cancel = CancellationToken::new();
                let bp = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
                state.conn_manager.register(
                    conn_id,
                    tx,
                    ctrl_tx,
                    mpsc::channel(1).0,
                    None,
                    cancel.clone(),
                    community,
                    bp,
                    std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
                    3,
                    crate::state::CommunityConnectionControl::new(cancel.clone()),
                );
                state.conn_manager.set_authenticated_identity(
                    conn_id,
                    pubkey_bytes,
                    Some(crate::nip_fi_core::NipFiSession::new(TEST_ISS, None)),
                );
                (conn_id, cancel)
            };

        // ── Build state: capacity 2 so the Merged case succeeds for the target. ──
        let state = {
            let mut config = crate::config::Config::for_test();
            // Wire TEST_ISS into the NIP-FI registry so apply_nip_fi_disconnect accepts it.
            config.nip_fi.registry.insert(test_issuer_policy());
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (mut state, _) = crate::state::AppState::new(
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
            let deny_map = Arc::new(buzz_auth::NipFiDenyMap::new(
                2,
                vec![buzz_auth::IssuerCapacity {
                    issuer: TEST_ISS.to_owned(),
                    capacity: 2,
                }],
            ));
            state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
            Arc::new(state)
        };

        let target = nostr::Keys::generate().public_key();
        let unrelated = nostr::Keys::generate().public_key();
        let now = chrono::Utc::now();
        let until_unix = (now + chrono::Duration::seconds(300)).timestamp();

        // Register targeted ordinary WS and audio controls.
        let (_target_ws_id, cancel_target_ws) = register_ws(&state, target.to_bytes().to_vec());

        // Register targeted audio via the production helper.
        // Guards must live until after the assertions — declared here, not in a sub-block.
        let audio_registry = &state.community_connections;
        let cancel_audio_target = CancellationToken::new();
        let _audio_target_guard = {
            let ctrl = CommunityConnectionControl::new(cancel_audio_target.clone());
            audio_post_auth_register(
                &ctrl,
                target.to_bytes().to_vec(),
                Some(crate::nip_fi_core::NipFiSession::new(TEST_ISS, None)),
            );
            audio_registry.register(Uuid::new_v4(), community, ctrl)
        };

        // Register unrelated ordinary WS and audio controls.
        let (_unrelated_ws_id, cancel_unrelated_ws) =
            register_ws(&state, unrelated.to_bytes().to_vec());
        let cancel_audio_unrelated = CancellationToken::new();
        let _audio_unrelated_guard = {
            let ctrl = CommunityConnectionControl::new(cancel_audio_unrelated.clone());
            audio_post_auth_register(
                &ctrl,
                unrelated.to_bytes().to_vec(),
                Some(crate::nip_fi_core::NipFiSession::new(TEST_ISS, None)),
            );
            audio_registry.register(Uuid::new_v4(), community, ctrl)
        };

        // Drive apply_nip_fi_disconnect for the target (Merged case).
        let msg = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: target.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
            revoke_through: None,
        };
        let result = apply_nip_fi_disconnect(&state, &msg, || now);
        assert_eq!(
            result,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Merged),
            "target disconnect must merge"
        );

        // Claim 1: targeted ordinary WS is cancelled.
        assert!(
            cancel_target_ws.is_cancelled(),
            "targeted ordinary WS must be cancelled by disconnect fan-out"
        );
        // Claim 2: targeted audio is cancelled (registered via audio_post_auth_register).
        assert!(
            cancel_audio_target.is_cancelled(),
            "targeted audio connection must be cancelled via community_connections fan-out"
        );
        // Claim 3a: unrelated ordinary WS remains open.
        assert!(
            !cancel_unrelated_ws.is_cancelled(),
            "unrelated ordinary WS must remain open"
        );
        // Claim 3b: unrelated audio remains open.
        assert!(
            !cancel_audio_unrelated.is_cancelled(),
            "unrelated audio connection must remain open"
        );

        // ── Capacity-failure variant ──────────────────────────────────────────────
        // Pre-fill the map to capacity with a different key, then deliver target2
        // (capacity exceeded). Assert target2's sessions still close despite no map entry.
        let target2 = nostr::Keys::generate().public_key();

        // Register target2 ordinary WS and audio.
        let (_t2_ws_id, cancel_t2_ws) = register_ws(&state, target2.to_bytes().to_vec());
        let cancel_t2_audio = CancellationToken::new();
        let _t2_audio_guard = {
            let ctrl = CommunityConnectionControl::new(cancel_t2_audio.clone());
            audio_post_auth_register(
                &ctrl,
                target2.to_bytes().to_vec(),
                Some(crate::nip_fi_core::NipFiSession::new(TEST_ISS, None)),
            );
            audio_registry.register(Uuid::new_v4(), community, ctrl)
        };

        // The map is now at capacity (target is in it from the Merged above, plus we need
        // one more to saturate cap=2). Pre-fill the second slot with a filler key.
        let filler = nostr::Keys::generate().public_key();
        let msg_fill = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: filler.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
            revoke_through: None,
        };
        let fill_result = apply_nip_fi_disconnect(&state, &msg_fill, || now);
        assert_eq!(
            fill_result,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Merged),
            "filler must merge to saturate capacity"
        );

        // Now deliver target2 — capacity exceeded, no map entry.
        let msg2 = buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: target2.to_bytes().to_vec(),
            until_unix,
            until_unix_nanos: 0,
            revoke_through: None,
        };
        let result2 = apply_nip_fi_disconnect(&state, &msg2, || now);
        assert_eq!(
            result2,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::CapacityExceeded),
            "second target must hit capacity"
        );

        // Claim 4a: targeted ordinary WS still closes despite CapacityExceeded.
        assert!(
            cancel_t2_ws.is_cancelled(),
            "target2 ordinary WS must close even on CapacityExceeded"
        );
        // Claim 4b: targeted audio still closes despite CapacityExceeded.
        assert!(
            cancel_t2_audio.is_cancelled(),
            "target2 audio must close even on CapacityExceeded"
        );
    }

    // ── Test: blocker 3 — fractional deadline survives publisher wire and consumer equality boundary ─
    //
    // Exercises the full publisher → encode → decode → apply_nip_fi_disconnect chain with a
    // non-zero nanos deadline. Proves denied immediately after T, admitted at exact T + nanos.
    //
    // Red mutations:
    //  - publisher writes zero nanos → until reconstructed as T+0 → equality boundary fails
    //  - encoder omits nanos field → decoder defaults to 0 → same failure
    //  - decoder defaults a present field to zero → same failure
    //  - consumer reconstructs with zero nanos → same failure
    //  - comparison changes from < to <= → admitted before exact boundary

    #[tokio::test]
    async fn fractional_deadline_survives_publisher_wire_and_consumer_equality_boundary() {
        use super::NipFiDisconnectApplyResult;
        use super::{apply_nip_fi_disconnect, nip_fi_disconnect_message};
        use buzz_auth::CrossPodMergeResult;

        // Build a state with TEST_ISS in the registry so apply_nip_fi_disconnect accepts it.
        let state = {
            let mut config = crate::config::Config::for_test();
            // Wire TEST_ISS into the NIP-FI registry so apply_nip_fi_disconnect accepts it.
            config.nip_fi.registry.insert(test_issuer_policy());
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let audit = buzz_audit::AuditService::new(pool.clone());
            let auth = buzz_auth::AuthService::new(config.auth.clone());
            let search = buzz_search::SearchService::new(pool.clone());
            let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
                db.clone(),
                buzz_workflow::WorkflowConfig::default(),
            ));
            let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
            let (mut state, _) = crate::state::AppState::new(
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
            let deny_map = Arc::new(buzz_auth::NipFiDenyMap::new(
                1000,
                vec![buzz_auth::IssuerCapacity {
                    issuer: TEST_ISS.to_owned(),
                    capacity: 1000,
                }],
            ));
            state.nip_fi_deny_map = Some(Arc::clone(&deny_map));
            Arc::new(state)
        };

        // Build a CommandResult with a fractional deadline: T + 500_000_000 ns.
        // We construct the CommandResult directly rather than going through the HTTP
        // handler so we can control the exact until timestamp.
        let target = nostr::Keys::generate().public_key();
        let t_whole = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let nanos: u32 = 500_000_000;
        let t_frac = chrono::DateTime::from_timestamp(1_800_000_000, nanos).unwrap();

        // Construct a synthetic CommandResult.
        let cmd = buzz_auth::CommandResult {
            caller_iss: TEST_ISS.to_owned(),
            caller_sub: TEST_SUB.to_owned(),
            target_pubkey: target,
            until: t_frac,
            scope: buzz_auth::DenyScope::WholeKey,
        };

        // Publisher seam → encode → decode.
        let msg = nip_fi_disconnect_message(&cmd);
        assert_eq!(
            msg.until_unix, 1_800_000_000,
            "publisher must capture whole-second"
        );
        assert_eq!(msg.until_unix_nanos, nanos, "publisher must capture nanos");

        let encoded = buzz_pubsub::encode_nip_fi_disconnect(&msg).expect("encode must succeed");
        let decoded = buzz_pubsub::decode_nip_fi_disconnect(&encoded).expect("decode must succeed");
        assert_eq!(decoded.until_unix_nanos, nanos, "decoded nanos must match");

        // Consumer seam: apply with now = t_frac - 1ns (just inside the deadline).
        let now_inside = t_frac - chrono::Duration::nanoseconds(1);
        let result = apply_nip_fi_disconnect(&state, &decoded, || now_inside);
        assert_eq!(
            result,
            NipFiDisconnectApplyResult::Applied(CrossPodMergeResult::Merged),
            "apply must succeed with now inside deadline"
        );

        let deny_map = state.nip_fi_deny_map.as_deref().expect("deny map present");

        // Denied immediately after T (now = T + 1ns, well inside the deadline T+500ms).
        let now_after_t = t_whole + chrono::Duration::nanoseconds(1);
        assert!(
            deny_map.is_denied(TEST_ISS, &target, now_after_t),
            "must be denied at T+1ns (deadline is T+500ms)"
        );

        // Denied at deadline minus 1ns (just before equality boundary).
        let now_before_boundary = t_frac - chrono::Duration::nanoseconds(1);
        assert!(
            deny_map.is_denied(TEST_ISS, &target, now_before_boundary),
            "must be denied at deadline - 1ns"
        );

        // Admitted at exact equality (now == until): contract is `now < until`,
        // so exact equality means admitted.
        assert!(
            !deny_map.is_denied(TEST_ISS, &target, t_frac),
            "must be admitted at exact equality (now < until fails at now == until)"
        );

        // Also admitted after (now = T + whole second).
        assert!(
            !deny_map.is_denied(TEST_ISS, &target, t_whole + chrono::Duration::seconds(1)),
            "must be admitted past the deadline"
        );
    }

    // ── Test: blocker 4b — production_install_populates_both_app_state_fields ─────
    //
    // Calls install_nip_fi_command_components() directly (the production seam that owns
    // both AppState assignments). Proves:
    // - command_issuers == 1
    // - both AppState fields are Some
    // - a valid signed command through the verifier creates a deny visible via the map
    //
    // Mandatory red mutations (proven by separate inline verification below):
    //  1. delete deny_map assignment → AppState.nip_fi_deny_map is None
    //  2. delete verifier assignment → AppState.nip_fi_command_verifier is None
    //  3. wire verifier to a different map → verify succeeds but state map denial fails

    #[tokio::test]
    async fn production_install_populates_both_app_state_fields() {
        use super::install_nip_fi_command_components;

        // Build a minimal AppState — same construction as build_test_state but without
        // the S4 fields so we can verify install_nip_fi_command_components populates them.
        let config = crate::config::Config::for_test();
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .unwrap(),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
        let (mut state, _) = crate::state::AppState::new(
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
        // Both fields start as None — we'll verify install populates them.
        assert!(state.nip_fi_deny_map.is_none());
        assert!(state.nip_fi_command_verifier.is_none());

        let jwks_configs = vec![test_jwks_config()];
        let key_source = Arc::new(
            ProductionJwksSource::new(jwks_configs.clone(), buzz_auth::HttpJwksFetcher::new())
                .expect("valid key source"),
        );
        // Seed the JWKS snapshot hermetically (no HTTP).
        key_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;

        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());

        let cmd_configs = vec![(
            TEST_ISS.to_owned(),
            CommandIssuerEnvConfig {
                maximum_command_age_seconds: Some(30),
                authorized_principals: Some(vec![TEST_SUB.to_owned()]),
                deny_set_capacity: Some(100),
            },
        )];

        let report = install_nip_fi_command_components(
            &mut state.nip_fi_deny_map,
            &mut state.nip_fi_command_verifier,
            buzz_auth::NipFiMode::Enforce,
            &registry,
            Arc::clone(&key_source),
            &cmd_configs,
        )
        .expect("install must succeed for valid config");

        assert_eq!(report.command_issuers, 1, "command_issuers must equal 1");

        // Both AppState fields must be populated.
        assert!(
            state.nip_fi_deny_map.is_some(),
            "nip_fi_deny_map must be Some after install"
        );
        assert!(
            state.nip_fi_command_verifier.is_some(),
            "nip_fi_command_verifier must be Some after install"
        );

        // Verify a valid command through the verifier and confirm the deny entry
        // is visible through the AppState's deny_map (proves shared map wiring).
        let target = nostr::Keys::generate().public_key();
        let token = mint_token(&target.to_hex(), 300, serde_json::json!({}));
        let verifier = state.nip_fi_command_verifier.as_ref().unwrap();
        let result = verifier.verify_at(&token, "POST", TEST_PATH, &target, chrono::Utc::now());
        assert!(
            result.is_ok(),
            "verifier must accept a valid command: {result:?}"
        );
        let deny_map = state.nip_fi_deny_map.as_deref().unwrap();
        assert!(
            deny_map.is_denied(TEST_ISS, &target, chrono::Utc::now()),
            "deny entry must be visible via AppState.nip_fi_deny_map after verify"
        );
    }

    // ── Single JWKS lifecycle owner witness ───────────────────────────────
    //
    // `main.rs` owns the only warm + per-issuer refresh of the shared JWKS
    // source. The installer must build and install components without any
    // fetch. Reintroducing a warm loop (or refresh spawn) inside the installer
    // makes `call_count` non-zero and reds this test.
    #[tokio::test(start_paused = true)]
    async fn installer_performs_no_jwks_fetch() {
        use super::install_nip_fi_command_components;

        let fetcher = buzz_auth::ScriptedJwksFetcher::new([]);
        let fetches = Arc::clone(&fetcher.call_count);
        let key_source = Arc::new(
            ProductionJwksSource::new(vec![test_jwks_config()], fetcher).expect("valid key source"),
        );
        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());
        let cmd_configs = vec![(
            TEST_ISS.to_owned(),
            CommandIssuerEnvConfig {
                maximum_command_age_seconds: Some(30),
                authorized_principals: Some(vec![TEST_SUB.to_owned()]),
                deny_set_capacity: Some(100),
            },
        )];
        let mut deny_map = None;
        let mut command_verifier = None;

        install_nip_fi_command_components(
            &mut deny_map,
            &mut command_verifier,
            buzz_auth::NipFiMode::Enforce,
            &registry,
            key_source,
            &cmd_configs,
        )
        .expect("install must succeed for valid config");
        tokio::time::sleep(std::time::Duration::from_secs(86_400)).await;

        assert!(deny_map.is_some() && command_verifier.is_some());
        assert_eq!(
            fetches.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "installer must not fetch JWKS; main.rs is the single lifecycle owner"
        );
    }

    // ── Startup-oracle: shared JWKS source wiring ──────────────────────────
    //
    // Proves that the installer, given the state's own JWKS source (as
    // `main.rs` passes it), wires the command verifier onto the EXACT Arc that
    // `nip_fi_verifier` holds: once that source is warm, both verify.
    //
    // Construction path mirrors `main.rs` exactly:
    //   1. Build AppState with an Enforce config that has jwks_configs populated —
    //      `build_nip_fi_components` runs and sets `nip_fi_verifier` and
    //      `nip_fi_jwks_source` on the state.
    //   2. Seed the state's own `nip_fi_jwks_source` (no HTTP).
    //   3. Call `install_nip_fi_command_components` with
    //      `state.nip_fi_jwks_source.clone()` — the same Arc the verifier holds.
    //   4. Assert both `nip_fi_verifier` and `nip_fi_command_verifier` verify.
    //
    // Mutation evidence:
    //   Pass a second, unseeded source to the installer → the command
    //   verifier's source stays cold → command verify fails.
    #[tokio::test]
    async fn installer_shares_jwks_source_with_assertion_verifier() {
        use super::install_nip_fi_command_components;
        use crate::nip_fi_config::NipFiRelayConfig;
        use buzz_auth::NipFiMode;

        // Build the config with an Enforce NIP-FI section so build_nip_fi_components
        // sets nip_fi_verifier + nip_fi_jwks_source on the AppState.
        let mut config = crate::config::Config::for_test();
        let jwks_configs = vec![test_jwks_config()];
        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());
        let cmd_configs = vec![(
            TEST_ISS.to_owned(),
            CommandIssuerEnvConfig {
                maximum_command_age_seconds: Some(30),
                authorized_principals: Some(vec![TEST_SUB.to_owned()]),
                deny_set_capacity: Some(100),
            },
        )];
        config.nip_fi = NipFiRelayConfig {
            profile_policies: Vec::new(),
            accept_shadow_disconnects: false,
            source: Default::default(),
            mode: NipFiMode::Enforce,
            registry: registry.clone(),
            jwks_configs: jwks_configs.clone(),
            command_configs: cmd_configs.clone(),
            max_connection_lifetime_secs: 3600,
            communities: crate::nip_fi_core::test_support::any_host(TEST_AUD),
        };

        let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .unwrap(),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
        // build_nip_fi_components runs here because mode == Enforce and jwks_configs
        // is non-empty; nip_fi_verifier and nip_fi_jwks_source are set on the state.
        let (mut state, _) = crate::state::AppState::new(
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
        assert!(
            state.nip_fi_verifier.is_some(),
            "nip_fi_verifier must be Some for Enforce config"
        );
        assert!(
            state.nip_fi_jwks_source.is_some(),
            "nip_fi_jwks_source must be Some for Enforce config"
        );

        let shared_source = state.nip_fi_jwks_source.clone().unwrap();
        install_nip_fi_command_components(
            &mut state.nip_fi_deny_map,
            &mut state.nip_fi_command_verifier,
            NipFiMode::Enforce,
            &registry,
            Arc::clone(&shared_source),
            &cmd_configs,
        )
        .expect("install must succeed for valid config");

        // Warm the state's own source after install — the step main.rs's
        // single lifecycle owner performs. Both verifiers must see it.
        shared_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;

        let key = nostr::Keys::generate();
        let token = mint_assertion_token(&key.public_key().to_hex());
        let verifier = state.nip_fi_verifier.as_deref().unwrap();
        let result =
            verifier.verify_assertion(&token, &crate::nip_fi_core::test_support::binding(TEST_AUD));
        assert!(
            result.is_ok(),
            "assertion verifier must read the warmed shared source; got: {result:?}"
        );

        let target = nostr::Keys::generate().public_key();
        let command = mint_token(&target.to_hex(), 300, serde_json::json!({}));
        let command_verifier = state.nip_fi_command_verifier.as_ref().unwrap();
        let result =
            command_verifier.verify_at(&command, "POST", TEST_PATH, &target, chrono::Utc::now());
        assert!(
            result.is_ok(),
            "command verifier must read the same warmed shared source; got: {result:?}"
        );
    }

    /// Shadow evaluates commands like enforce, so it shares enforce's
    /// refusal to start without a command-capable issuer.
    #[test]
    fn installer_rejects_shadow_without_command_issuers() {
        let key_source = Arc::new(
            ProductionJwksSource::new(
                vec![test_jwks_config()],
                buzz_auth::ScriptedJwksFetcher::new([]),
            )
            .expect("valid key source"),
        );
        let err = super::install_nip_fi_command_components(
            &mut None,
            &mut None,
            buzz_auth::NipFiMode::Shadow,
            &IssuerRegistry::new(),
            key_source,
            &[],
        )
        .unwrap_err();
        assert_eq!(
            err,
            "NIP-FI install: enforce mode requires at least one command-capable issuer"
        );
    }

    /// Mint a valid ES256 `nip-fi+jwt` assertion for `nostr_pubkey = key_hex`,
    /// signed by the route-integration-test key pair.
    /// Used by the startup-oracle test to verify the assertion verifier against
    /// its own warmed JWKS source.
    fn mint_assertion_token(key_hex: &str) -> String {
        mint_assertion_token_for(TEST_ISS, key_hex)
    }

    fn mint_assertion_token_for(iss: &str, key_hex: &str) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let now = chrono::Utc::now().timestamp();
        let claims = serde_json::json!({
            "iss": iss,
            "aud": TEST_AUD,
            "sub": TEST_SUB,
            "iat": now,
            "exp": now + 600,
            "nostr_pubkey": key_hex,
        });
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("nip-fi+jwt".to_owned());
        header.kid = Some(TEST_KID.to_owned());
        let key =
            EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).expect("valid test EC key");
        encode(&header, &claims, &key).expect("sign assertion-test token")
    }

    // ── Issuer scope: the close scans match (issuer, k) ──────────────────────
    //
    // A deny entry is keyed by (iss, k).  The same key K admitted under a
    // second issuer B is not blocked by A's entry, so neither the route nor the
    // cross-pod consumer may close B's sessions.  [FI-TRACE-DENY-SET]

    const OTHER_ISS: &str = "https://idp-b.test.example.com";

    /// Live root + audio sessions for one (issuer, key), with the tokens the
    /// close path cancels.  The guard keeps the audio socket registered.
    struct IssuerSessions {
        root: tokio_util::sync::CancellationToken,
        audio: tokio_util::sync::CancellationToken,
        _audio_guard: crate::state::CommunityConnectionGuard,
    }

    impl IssuerSessions {
        fn register(state: &crate::state::AppState, issuer: &str, key: &nostr::PublicKey) -> Self {
            Self::register_issued(state, issuer, key, chrono::Utc::now())
        }

        /// Sessions admitted by an assertion with verified `iat` `issued_at`.
        fn register_issued(
            state: &crate::state::AppState,
            issuer: &str,
            key: &nostr::PublicKey,
            issued_at: chrono::DateTime<chrono::Utc>,
        ) -> Self {
            use crate::state::CommunityConnectionControl;
            use tokio::sync::mpsc;
            use tokio_util::sync::CancellationToken;
            let community = buzz_core::tenant::CommunityId::from_uuid(uuid::Uuid::nil());
            let root = CancellationToken::new();
            let conn_id = uuid::Uuid::new_v4();
            state.conn_manager.register(
                conn_id,
                mpsc::channel(8).0,
                mpsc::channel(8).0,
                mpsc::channel(1).0,
                None,
                root.clone(),
                community,
                Arc::new(std::sync::atomic::AtomicU8::new(0)),
                Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
                3,
                CommunityConnectionControl::new(root.clone()),
            );
            state.conn_manager.set_authenticated_identity(
                conn_id,
                key.to_bytes().to_vec(),
                Some(crate::nip_fi_core::NipFiSession::issued_at(
                    issuer, None, issued_at,
                )),
            );
            let audio = CancellationToken::new();
            let ctrl = CommunityConnectionControl::new(audio.clone());
            crate::audio::handler::audio_post_auth_register(
                &ctrl,
                key.to_bytes().to_vec(),
                Some(crate::nip_fi_core::NipFiSession::issued_at(
                    issuer, None, issued_at,
                )),
            );
            let _audio_guard =
                state
                    .community_connections
                    .register(uuid::Uuid::new_v4(), community, ctrl);
            Self {
                root,
                audio,
                _audio_guard,
            }
        }

        fn assert_closed(&self, who: &str) {
            assert!(self.root.is_cancelled(), "{who}: root session must close");
            assert!(self.audio.is_cancelled(), "{who}: audio session must close");
        }

        fn assert_open(&self, who: &str) {
            assert!(
                !self.root.is_cancelled(),
                "{who}: root session must survive"
            );
            assert!(
                !self.audio.is_cancelled(),
                "{who}: audio session must survive"
            );
        }
    }

    #[tokio::test]
    async fn route_disconnect_closes_only_caller_issuer_sessions() {
        let state = build_test_state(1000).await;
        let key = nostr::Keys::generate().public_key();
        let under_a = IssuerSessions::register(&state, TEST_ISS, &key);
        let under_b = IssuerSessions::register(&state, OTHER_ISS, &key);

        let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": key.to_hex()})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        under_a.assert_closed("issuer A");
        under_b.assert_open("issuer B");
    }

    // ── revoke_through: cutoff-scoped close scans ────────────────────────────

    fn at(secs: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(secs, 0).unwrap()
    }

    fn cutoff_message(key: &nostr::PublicKey, c: i64) -> buzz_pubsub::NipFiDisconnect {
        buzz_pubsub::NipFiDisconnect {
            revoke_through: Some(c),
            ..cross_pod_message(key)
        }
    }

    #[tokio::test]
    async fn route_cutoff_closes_old_sessions_and_keeps_fresh_ones_across_redelivery() {
        let state = build_test_state(1000).await;
        let key = nostr::Keys::generate().public_key();
        let c = chrono::Utc::now().timestamp();
        let old = IssuerSessions::register_issued(&state, TEST_ISS, &key, at(c));
        let fresh = IssuerSessions::register_issued(&state, TEST_ISS, &key, at(c + 1));

        let token = mint_token(
            &key.to_hex(),
            300,
            serde_json::json!({ "revoke_through": c }),
        );
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": key.to_hex()})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        old.assert_closed("iat == C");
        fresh.assert_open("iat > C after the route");

        // The issuer's periodic redelivery, and a restore replaying the same
        // record, arrive as cross-pod messages: still cutoff-scoped.
        for _ in 0..3 {
            apply_nip_fi_disconnect(&state, &cutoff_message(&key, c), chrono::Utc::now);
        }
        fresh.assert_open("iat > C after redelivery");
        let deny_map = state.nip_fi_deny_map.as_ref().unwrap();
        assert!(deny_map.denies(TEST_ISS, &key, at(c), chrono::Utc::now()));
        assert!(!deny_map.denies(TEST_ISS, &key, at(c + 1), chrono::Utc::now()));
    }

    #[tokio::test]
    async fn cross_pod_cutoff_closes_only_old_sessions_on_another_pod() {
        let state = cross_pod_state(10).await;
        let key = nostr::Keys::generate().public_key();
        let c = chrono::Utc::now().timestamp();
        let old = IssuerSessions::register_issued(&state, TEST_ISS, &key, at(c - 30));
        let fresh = IssuerSessions::register_issued(&state, TEST_ISS, &key, at(c + 1));
        let result = apply_nip_fi_disconnect(&state, &cutoff_message(&key, c), chrono::Utc::now);
        assert_eq!(
            result,
            NipFiDisconnectApplyResult::Applied(buzz_auth::CrossPodMergeResult::Merged)
        );
        old.assert_closed("old session");
        fresh.assert_open("fresh session");
    }

    #[tokio::test]
    async fn cutoff_delivery_over_an_active_whole_key_entry_closes_every_session() {
        let state = cross_pod_state(10).await;
        let key = nostr::Keys::generate().public_key();
        let now = chrono::Utc::now();
        apply_nip_fi_disconnect(&state, &cross_pod_message(&key), || now);
        let c = now.timestamp();
        let fresh = IssuerSessions::register_issued(&state, TEST_ISS, &key, at(c + 1));
        apply_nip_fi_disconnect(&state, &cutoff_message(&key, c), || now);
        fresh.assert_closed("the stronger merged whole-key deny applies");
    }

    #[tokio::test]
    async fn past_until_cutoff_still_closes_its_scope_without_changing_the_deny() {
        let state = cross_pod_state(10).await;
        let key = nostr::Keys::generate().public_key();
        let now = chrono::Utc::now();
        let c = now.timestamp();
        let old = IssuerSessions::register_issued(&state, TEST_ISS, &key, at(c));
        let fresh = IssuerSessions::register_issued(&state, TEST_ISS, &key, at(c + 1));
        let message = buzz_pubsub::NipFiDisconnect {
            until_unix: c - 1,
            ..cutoff_message(&key, c)
        };
        apply_nip_fi_disconnect(&state, &message, || now);
        old.assert_closed("scoped close still runs");
        fresh.assert_open("fresh session");
        let deny_map = state.nip_fi_deny_map.as_ref().unwrap();
        assert!(
            !deny_map.is_denied(TEST_ISS, &key, now),
            "no active deny created"
        );
    }

    /// The route's published bus message carries the cutoff: another pod
    /// that decodes it closes only `iat <= C` and keeps the fresh session.
    #[tokio::test]
    async fn published_cutoff_reaches_another_pod_cutoff_scoped() {
        let key = nostr::Keys::generate().public_key();
        let c = chrono::Utc::now().timestamp();
        let cmd = buzz_auth::CommandResult {
            caller_iss: TEST_ISS.to_owned(),
            caller_sub: TEST_SUB.to_owned(),
            target_pubkey: key,
            until: chrono::Utc::now() + chrono::Duration::seconds(300),
            scope: buzz_auth::DenyScope::IssuedThrough(c),
        };
        let encoded = buzz_pubsub::encode_nip_fi_disconnect(&nip_fi_disconnect_message(&cmd))
            .expect("encode");
        let decoded = buzz_pubsub::decode_nip_fi_disconnect(&encoded).expect("decode");
        let other_pod = cross_pod_state(10).await;
        let old = IssuerSessions::register_issued(&other_pod, TEST_ISS, &key, at(c));
        let fresh = IssuerSessions::register_issued(&other_pod, TEST_ISS, &key, at(c + 1));
        apply_nip_fi_disconnect(&other_pod, &decoded, chrono::Utc::now);
        old.assert_closed("iat == C on the other pod");
        fresh.assert_open("iat > C on the other pod");
    }

    /// The route widens its close to a stronger active entry: a cutoff
    /// command over an active whole-key deny closes a fresh session too.
    #[tokio::test]
    async fn route_cutoff_over_an_active_whole_key_entry_closes_every_session() {
        let state = build_test_state(1000).await;
        let key = nostr::Keys::generate().public_key();
        let now = chrono::Utc::now();
        let c = now.timestamp();
        state
            .nip_fi_deny_map
            .as_ref()
            .unwrap()
            .merge_cross_pod_deny(
                TEST_ISS,
                &key,
                buzz_auth::DenyEntry::whole_key(now + chrono::Duration::seconds(300)),
                || now,
            );
        let fresh = IssuerSessions::register_issued(&state, TEST_ISS, &key, at(c + 1));
        let token = mint_token(
            &key.to_hex(),
            300,
            serde_json::json!({ "revoke_through": c }),
        );
        let resp = do_request(
            Arc::clone(&state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": key.to_hex()})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        fresh.assert_closed("the stronger active whole-key deny applies on the route");
    }

    #[test]
    fn legacy_bus_message_without_revoke_through_is_whole_key() {
        let json = r#"{"issuer":"i","pubkey_bytes":[],"until_unix":1}"#;
        let decoded: buzz_pubsub::NipFiDisconnect = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.revoke_through, None);
        assert_eq!(
            buzz_auth::DenyScope::from_revoke_through(decoded.revoke_through),
            buzz_auth::DenyScope::WholeKey
        );
    }

    /// State whose registry and deny map know only issuer A (`TEST_ISS`).
    async fn cross_pod_state(capacity: usize) -> Arc<crate::state::AppState> {
        let mut config = crate::config::Config::for_test();
        config.nip_fi.registry.insert(test_issuer_policy());
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .unwrap(),
        );
        let audit = buzz_audit::AuditService::new(pool.clone());
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage = buzz_media::MediaStorage::new(&config.media).unwrap();
        let (mut state, _) = crate::state::AppState::new(
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
        state.nip_fi_deny_map = Some(Arc::new(NipFiDenyMap::new(
            capacity,
            vec![IssuerCapacity {
                issuer: TEST_ISS.to_owned(),
                capacity,
            }],
        )));
        Arc::new(state)
    }

    fn cross_pod_message(key: &nostr::PublicKey) -> buzz_pubsub::NipFiDisconnect {
        buzz_pubsub::NipFiDisconnect {
            issuer: TEST_ISS.to_owned(),
            pubkey_bytes: key.to_bytes().to_vec(),
            until_unix: (chrono::Utc::now() + chrono::Duration::seconds(300)).timestamp(),
            until_unix_nanos: 0,
            revoke_through: None,
        }
    }

    /// Delivers an A-issued cross-pod message for K with A and B sessions
    /// live, and asserts the merge outcome and that only A's sessions close.
    fn assert_cross_pod_closes_only_issuer_a(
        state: &crate::state::AppState,
        expected: buzz_auth::CrossPodMergeResult,
    ) {
        let key = nostr::Keys::generate().public_key();
        let under_a = IssuerSessions::register(state, TEST_ISS, &key);
        let under_b = IssuerSessions::register(state, OTHER_ISS, &key);

        let result = apply_nip_fi_disconnect(state, &cross_pod_message(&key), chrono::Utc::now);

        assert_eq!(result, NipFiDisconnectApplyResult::Applied(expected));
        under_a.assert_closed("issuer A");
        under_b.assert_open("issuer B");
    }

    #[tokio::test]
    async fn cross_pod_merged_closes_only_message_issuer_sessions() {
        let state = cross_pod_state(10).await;
        assert_cross_pod_closes_only_issuer_a(&state, buzz_auth::CrossPodMergeResult::Merged);
    }

    #[tokio::test]
    async fn cross_pod_capacity_exceeded_closes_only_message_issuer_sessions() {
        let state = cross_pod_state(1).await;
        let filler = nostr::Keys::generate().public_key();
        assert_eq!(
            apply_nip_fi_disconnect(&state, &cross_pod_message(&filler), chrono::Utc::now),
            NipFiDisconnectApplyResult::Applied(buzz_auth::CrossPodMergeResult::Merged),
        );
        assert_cross_pod_closes_only_issuer_a(
            &state,
            buzz_auth::CrossPodMergeResult::CapacityExceeded,
        );
    }

    #[tokio::test]
    async fn cross_pod_shard_poisoned_closes_only_message_issuer_sessions() {
        let state = cross_pod_state(10).await;
        state
            .nip_fi_deny_map
            .as_deref()
            .expect("deny map present")
            .poison_shard_for_test(TEST_ISS);
        assert_cross_pod_closes_only_issuer_a(
            &state,
            buzz_auth::CrossPodMergeResult::ShardPoisoned,
        );
    }

    // Pins: the cross-pod capacity and poison failsafes count on the real
    // counters only in enforce; a shadow pod counts them on its own shadow
    // disconnect counter, never on an enforce series.
    // Mutation: reverting either site to its raw counter adds a real series
    // to the shadow run (or drops the shadow series).
    #[tokio::test(flavor = "current_thread")]
    async fn cross_pod_failsafe_counters_stay_off_enforce_series_in_shadow() {
        for (mode, expected) in [
            (
                buzz_auth::NipFiMode::Enforce,
                vec![
                    "buzz_nip_fi_cross_pod_capacity_exceeded_total".to_owned(),
                    "buzz_nip_fi_cross_pod_shard_poison_total".to_owned(),
                ],
            ),
            (
                buzz_auth::NipFiMode::Shadow,
                vec![
                    "buzz_nip_fi_shadow_disconnect_total cross_pod capacity".to_owned(),
                    "buzz_nip_fi_shadow_disconnect_total cross_pod poison".to_owned(),
                ],
            ),
        ] {
            let with_mode = |state: Arc<crate::state::AppState>| {
                let mut state = (*state).clone();
                Arc::make_mut(&mut state.config).nip_fi.mode = mode;
                state
            };
            let full = with_mode(cross_pod_state(1).await);
            let poisoned = with_mode(cross_pod_state(10).await);
            poisoned
                .nip_fi_deny_map
                .as_deref()
                .expect("deny map present")
                .poison_shard_for_test(TEST_ISS);
            let recorder = metrics_util::debugging::DebuggingRecorder::new();
            let snapshotter = recorder.snapshotter();
            let _guard = metrics::set_default_local_recorder(&recorder);
            for state in [&full, &full, &poisoned] {
                let key = nostr::Keys::generate().public_key();
                apply_nip_fi_disconnect(state, &cross_pod_message(&key), chrono::Utc::now);
            }
            let mut series: Vec<String> = snapshotter
                .snapshot()
                .into_vec()
                .into_iter()
                .map(|(key, ..)| {
                    let key = key.key();
                    let labels = key.labels().map(|l| format!(" {}", l.value()));
                    format!("{}{}", key.name(), labels.collect::<String>())
                })
                .filter(|name| name.starts_with("buzz_nip_fi"))
                .collect();
            series.sort();
            assert_eq!(series, expected, "{mode:?}");
        }
    }

    // ── Cross-pod consumer: rejection and ceiling clamp ──────────────────────
    //
    // Only malformed messages are rejected.  An `until` beyond the receiving
    // pod's ceiling is clamped, so clock drift between pods cannot drop a
    // command the origin pod already accepted.

    /// Fixed consumer clock: the tests below inject time, never sleep.
    fn cross_pod_now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_900_000_000, 0).expect("representable")
    }

    /// Delivers `message` for `key` with an issuer-A session live and asserts
    /// the consumer rejects it without recording a deny or closing the session.
    fn assert_cross_pod_rejected(
        state: &crate::state::AppState,
        key: &nostr::PublicKey,
        message: &buzz_pubsub::NipFiDisconnect,
    ) {
        let sessions = IssuerSessions::register(state, TEST_ISS, key);
        let now = cross_pod_now();

        let result = apply_nip_fi_disconnect(state, message, || now);

        assert_eq!(result, NipFiDisconnectApplyResult::Rejected);
        sessions.assert_open("rejected message");
        let deny_map = state.nip_fi_deny_map.as_deref().expect("deny map present");
        assert!(!deny_map.is_denied(TEST_ISS, key, now));
    }

    #[tokio::test]
    async fn cross_pod_malformed_pubkey_bytes_rejected() {
        let state = cross_pod_state(10).await;
        let key = nostr::Keys::generate().public_key();
        let message = buzz_pubsub::NipFiDisconnect {
            pubkey_bytes: key.to_bytes()[..31].to_vec(),
            ..cross_pod_message(&key)
        };
        assert_cross_pod_rejected(&state, &key, &message);
    }

    #[tokio::test]
    async fn cross_pod_unknown_issuer_rejected() {
        let state = cross_pod_state(10).await;
        let key = nostr::Keys::generate().public_key();
        let message = buzz_pubsub::NipFiDisconnect {
            issuer: OTHER_ISS.to_owned(),
            ..cross_pod_message(&key)
        };
        assert_cross_pod_rejected(&state, &key, &message);
    }

    #[tokio::test]
    async fn cross_pod_unrepresentable_until_rejected() {
        let state = cross_pod_state(10).await;
        let key = nostr::Keys::generate().public_key();
        let message = buzz_pubsub::NipFiDisconnect {
            until_unix: i64::MAX,
            ..cross_pod_message(&key)
        };
        assert_cross_pod_rejected(&state, &key, &message);
    }

    #[tokio::test]
    async fn cross_pod_unrepresentable_revoke_through_rejected() {
        let state = cross_pod_state(10).await;
        for c in [i64::MIN, i64::MAX] {
            let key = nostr::Keys::generate().public_key();
            let message = buzz_pubsub::NipFiDisconnect {
                revoke_through: Some(c),
                ..cross_pod_message(&key)
            };
            assert_cross_pod_rejected(&state, &key, &message);
        }
    }

    #[tokio::test]
    async fn cross_pod_without_deny_map_is_disabled() {
        let mut config = crate::config::Config::for_test();
        config.nip_fi.registry.insert(test_issuer_policy());
        let mut state = build_test_app_state(10, config).await;
        state.nip_fi_deny_map = None;
        let key = nostr::Keys::generate().public_key();
        let sessions = IssuerSessions::register(&state, TEST_ISS, &key);

        let result = apply_nip_fi_disconnect(&state, &cross_pod_message(&key), cross_pod_now);

        assert_eq!(result, NipFiDisconnectApplyResult::Disabled);
        sessions.assert_open("disabled consumer");
    }

    #[tokio::test]
    async fn cross_pod_until_beyond_ceiling_is_clamped_and_applied() {
        let state = cross_pod_state(10).await;
        let key = nostr::Keys::generate().public_key();
        let sessions = IssuerSessions::register(&state, TEST_ISS, &key);
        let now = cross_pod_now();
        let policy = test_issuer_policy();
        let ceiling = now
            + chrono::Duration::seconds(policy.skew_seconds() as i64)
            + chrono::Duration::seconds(policy.maximum_assertion_age_seconds() as i64);
        let message = buzz_pubsub::NipFiDisconnect {
            until_unix: (ceiling + chrono::Duration::days(365)).timestamp(),
            ..cross_pod_message(&key)
        };

        let result = apply_nip_fi_disconnect(&state, &message, || now);

        assert_eq!(
            result,
            NipFiDisconnectApplyResult::Applied(buzz_auth::CrossPodMergeResult::Merged)
        );
        sessions.assert_closed("clamped message");
        let deny_map = state.nip_fi_deny_map.as_deref().expect("deny map present");
        assert!(deny_map.is_denied(TEST_ISS, &key, now), "denied at now");
        assert!(
            deny_map.is_denied(TEST_ISS, &key, ceiling - chrono::Duration::nanoseconds(1)),
            "denied up to the local ceiling"
        );
        assert!(
            !deny_map.is_denied(TEST_ISS, &key, ceiling),
            "admitted once the clamped ceiling is reached"
        );
    }

    // ── HTTP admission reads the shared deny map ─────────────────────────────
    //
    // Deny entries are written only through `POST /api/nip-fi/disconnect`; HTTP
    // admission (`admit_nip_fi_http_on_state`) and WS admission must both see
    // them through `AppState::nip_fi_deny_map`.  [FI-TRACE-DENY-SET]

    /// `build_test_app_state` in `Enforce` mode with an assertion verifier that
    /// trusts issuer A (`TEST_ISS`) and issuer B (`OTHER_ISS`), both signed by
    /// the test key.  The command API and deny map know only issuer A.
    async fn http_enforce_state() -> Arc<crate::state::AppState> {
        let mut config = crate::config::Config::for_test();
        config.require_relay_membership = false;
        config.nip_fi.mode = buzz_auth::NipFiMode::Enforce;
        config.nip_fi.communities =
            crate::nip_fi_core::test_support::any_host("https://relay.test.example.com");
        config.nip_fi.registry.insert(issuer_policy(TEST_ISS));
        config.nip_fi.registry.insert(issuer_policy(OTHER_ISS));
        let mut state = build_test_app_state(1000, config).await;

        let key_source = Arc::new(
            ProductionJwksSource::new(
                vec![jwks_config(TEST_ISS), jwks_config(OTHER_ISS)],
                buzz_auth::HttpJwksFetcher::new(),
            )
            .expect("key source"),
        );
        key_source
            .seed_snapshot_for_test(TEST_ISS, test_jwks())
            .await;
        key_source
            .seed_snapshot_for_test(OTHER_ISS, test_jwks())
            .await;
        state.nip_fi_verifier = Some(Arc::new(buzz_auth::FederatedAssertionVerifier::new(
            state.config.nip_fi.registry.clone(),
            key_source,
        )));
        state.nip_fi_policy = Arc::new(crate::nip_fi_policy::OptionalPolicy);
        Arc::new(state)
    }

    struct RequiredPolicy;
    impl crate::nip_fi_policy::PolicySource for RequiredPolicy {
        fn lookup<'a>(
            &'a self,
            _host: &'a str,
        ) -> futures_util::future::BoxFuture<
            'a,
            Result<Option<buzz_db::community_auth::CommunityAuthPolicy>, ()>,
        > {
            Box::pin(async {
                Ok(Some(buzz_db::community_auth::CommunityAuthPolicy {
                    community_id: uuid::Uuid::nil(),
                    requires_nip_fi: true,
                    policy_generation: 1,
                }))
            })
        }
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)] // Real Axum admission closure.
    async fn required_mode_matrix_validates_crypto_only_when_enabled() {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        for mode in [
            buzz_auth::NipFiMode::Off,
            buzz_auth::NipFiMode::Shadow,
            buzz_auth::NipFiMode::Enforce,
        ] {
            let mut state = (*http_enforce_state().await).clone();
            Arc::make_mut(&mut state.config).nip_fi.mode = mode;
            Arc::make_mut(&mut state.config)
                .nip_fi
                .max_connection_lifetime_secs = 3600;
            state.nip_fi_policy = Arc::new(RequiredPolicy);
            let key = nostr::Keys::generate().public_key();
            let now = chrono::Utc::now().timestamp();
            for bad in ["valid", "issuer", "audience", "key", "expiry", "signature"] {
                let mut claims = serde_json::json!({"iss": TEST_ISS, "aud": TEST_AUD,
                    "sub": TEST_SUB, "iat": now, "exp": now + 600,
                    "nostr_pubkey": key.to_hex()});
                match bad {
                    "issuer" => claims["iss"] = serde_json::json!("https://untrusted.example"),
                    "audience" => claims["aud"] = serde_json::json!("https://other.example"),
                    "key" => {
                        claims["nostr_pubkey"] =
                            serde_json::json!(nostr::Keys::generate().public_key().to_hex())
                    }
                    "expiry" => {
                        claims["iat"] = serde_json::json!(now - 1200);
                        claims["exp"] = serde_json::json!(now - 600);
                    }
                    _ => {}
                }
                let mut header = Header::new(Algorithm::ES256);
                header.typ = Some("nip-fi+jwt".to_owned());
                header.kid = Some(TEST_KID.to_owned());
                let encoding = EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).unwrap();
                let mut token = encode(&header, &claims, &encoding).unwrap();
                if bad == "signature" {
                    let at = token.rfind('.').unwrap() + 1;
                    let replacement = if token.as_bytes()[at] == b'A' {
                        "B"
                    } else {
                        "A"
                    };
                    token.replace_range(at..at + 1, replacement);
                }
                let mut headers = crate::nip_fi_core::test_support::host_headers();
                headers.insert(
                    CLIENT_ATTACHED_HEADER,
                    format!("Bearer {token}").parse().unwrap(),
                );
                let result = crate::nip_fi_http::admit_nip_fi_http_on_state(
                    &state,
                    &headers,
                    &crate::access::AccessSlot::default(),
                    || Ok(crate::nip_fi_http::Nip98Proof::new(key, ())),
                )
                .await;
                if mode.is_off() || bad == "valid" {
                    assert!(result.is_ok(), "{mode:?} valid assertion");
                } else {
                    assert_eq!(
                        result.err().unwrap().status(),
                        StatusCode::FORBIDDEN,
                        "{mode:?} {bad}"
                    );
                }
            }
        }
    }

    /// Deny `key` under issuer A through the real disconnect route.
    async fn deny_via_route(
        state: &Arc<crate::state::AppState>,
        key: &nostr::PublicKey,
        until_offset_secs: i64,
    ) {
        let token = mint_token(&key.to_hex(), until_offset_secs, serde_json::json!({}));
        let resp = do_request(
            Arc::clone(state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": key.to_hex()})),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "disconnect route must succeed"
        );
    }

    /// HTTP admission for an `(iss, key)` assertion paired with a NIP-98 proof
    /// for `key`.  `Ok(())` = admitted; `Err` = the denial response.
    // Response<Body> is intentionally large (axum's design); see admit_nip_fi_http.
    #[allow(clippy::result_large_err)]
    async fn http_admit(
        state: &crate::state::AppState,
        iss: &str,
        key: &nostr::PublicKey,
    ) -> Result<(), axum::response::Response> {
        let mut headers = crate::nip_fi_core::test_support::host_headers();
        headers.insert(
            CLIENT_ATTACHED_HEADER,
            format!("Bearer {}", mint_assertion_token_for(iss, &key.to_hex()))
                .parse()
                .expect("header value"),
        );
        crate::nip_fi_http::admit_nip_fi_http_on_state(
            state,
            &headers,
            &crate::access::AccessSlot::default(),
            || Ok(crate::nip_fi_http::Nip98Proof::new(*key, ())),
        )
        .await
        .map(|_| ())
    }

    async fn assert_fixed_authorization_denied(resp: axum::response::Response, why: &str) {
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{why}");
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; charset=utf-8"),
            "{why}"
        );
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(&body[..], b"authorization denied\n", "{why}");
    }

    // A valid community-A assertion presented on community B's Host is
    // rejected on `aud`, even though B authorizes the same issuer.
    #[tokio::test]
    async fn http_admission_denies_community_a_assertion_on_community_b_host() {
        let key = nostr::Keys::generate().public_key();
        let mut state = (*http_enforce_state().await).clone();
        assert!(
            http_admit(&state, TEST_ISS, &key).await.is_ok(),
            "control: the assertion is admitted on its own community"
        );
        Arc::make_mut(&mut state.config).nip_fi.communities =
            crate::nip_fi_config::NipFiCommunities::for_test(
                "https://relay.example",
                &[TEST_ISS, OTHER_ISS],
            );
        let resp = http_admit(&state, TEST_ISS, &key)
            .await
            .expect_err("A's aud on B's Host");
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(&body[..], b"evidence rejected\n");
    }

    #[tokio::test]
    async fn http_admission_denies_key_denied_via_route() {
        let state = http_enforce_state().await;
        let key = nostr::Keys::generate().public_key();
        assert!(
            http_admit(&state, TEST_ISS, &key).await.is_ok(),
            "control: (iss-A, k) is admitted before any deny"
        );

        deny_via_route(&state, &key, 300).await;

        let resp = http_admit(&state, TEST_ISS, &key)
            .await
            .expect_err("(iss-A, k) must be denied at HTTP ingress after the route denies it");
        assert_fixed_authorization_denied(resp, "(iss-A, k) HTTP denial").await;
    }

    #[tokio::test]
    async fn http_admission_deny_is_issuer_scoped() {
        let state = http_enforce_state().await;
        let key = nostr::Keys::generate().public_key();
        deny_via_route(&state, &key, 300).await;

        assert!(
            http_admit(&state, TEST_ISS, &key).await.is_err(),
            "control: (iss-A, k) is denied"
        );
        assert!(
            http_admit(&state, OTHER_ISS, &key).await.is_ok(),
            "a deny for (iss-A, k) must not block (iss-B, k)"
        );
    }

    #[tokio::test]
    async fn one_route_deny_is_enforced_by_ws_and_http_admission() {
        use crate::router::build_router;
        let state = http_enforce_state().await;
        let key = nostr::Keys::generate().public_key();
        deny_via_route(&state, &key, 300).await;

        let ws_request = Request::get("/")
            .header(axum::http::header::HOST, "relay.example")
            .header("Upgrade", "websocket")
            .header("Connection", "Upgrade")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header(
                CLIENT_ATTACHED_HEADER,
                format!("Bearer {}", mint_assertion_token(&key.to_hex())),
            )
            .body(Body::empty())
            .expect("request");
        let ws_resp = build_router(Arc::clone(&state))
            .oneshot(ws_request)
            .await
            .expect("router response");
        assert_fixed_authorization_denied(ws_resp, "WS admission sees the route's deny").await;

        let http_resp = http_admit(&state, TEST_ISS, &key)
            .await
            .expect_err("HTTP admission sees the same deny");
        assert_fixed_authorization_denied(http_resp, "HTTP admission sees the route's deny").await;
    }

    #[tokio::test]
    // Response<Body> is intentionally large (axum's design); see admit_nip_fi_http.
    #[allow(clippy::result_large_err)]
    async fn off_mode_http_admission_ignores_route_deny_entries() {
        // Off mode: the command route still records the entry, but HTTP
        // admission never consults the deny map — the NIP-98 closure result is
        // returned unchanged, byte for byte.
        let mut state = (*build_test_state(1000).await).clone();
        state.nip_fi_policy = Arc::new(crate::nip_fi_policy::OptionalPolicy);
        let state = Arc::new(state);
        assert!(matches!(
            state.config.nip_fi.mode,
            buzz_auth::NipFiMode::Off
        ));
        let key = nostr::Keys::generate().public_key();
        deny_via_route(&state, &key, 300).await;
        assert!(
            state
                .nip_fi_deny_map
                .as_deref()
                .expect("deny map present")
                .is_denied(TEST_ISS, &key, chrono::Utc::now()),
            "control: the route recorded the deny entry"
        );

        let admitted = crate::nip_fi_http::admit_nip_fi_http_on_state(
            &state,
            &crate::nip_fi_core::test_support::host_headers(),
            &crate::access::AccessSlot::default(),
            || Ok(crate::nip_fi_http::Nip98Proof::new(key, 7u8)),
        )
        .await
        .expect("Off mode admits on NIP-98 success regardless of deny entries");
        assert_eq!(admitted.proven_pubkey(), &key);
        assert!(admitted.assertion().is_none());
        assert_eq!(*admitted.extra(), 7);

        let legacy = || {
            Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .header("Content-Type", "application/json")
                .body(Body::from(r#"{"error":"legacy"}"#))
                .expect("legacy response")
        };
        let resp = crate::nip_fi_http::admit_nip_fi_http_on_state::<(), _>(
            &state,
            &crate::nip_fi_core::test_support::host_headers(),
            &crate::access::AccessSlot::default(),
            || Err(legacy()),
        )
        .await
        .expect_err("Off mode propagates the NIP-98 failure");
        let expected = legacy();
        assert_eq!(resp.status(), expected.status());
        assert_eq!(resp.headers(), expected.headers());
        let got = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let want = axum::body::to_bytes(expected.into_body(), usize::MAX)
            .await
            .expect("body");
        assert_eq!(got, want, "Off-mode legacy response must be byte-identical");
    }

    // ── Characterization: exact disconnect-route rejection contract ──────────
    //
    // Pin status, Content-Type, WWW-Authenticate and body bytes for every
    // pre-success rejection, plus the observable check order
    // (header → JSON body → pubkey → verifier present → verify).

    const PLAIN: &str = "text/plain; charset=utf-8";

    async fn send_raw(
        state: Arc<crate::state::AppState>,
        header_values: Vec<axum::http::HeaderValue>,
        body: &[u8],
    ) -> axum::response::Response {
        let mut req = Request::builder()
            .method("POST")
            .uri(TEST_PATH)
            .header("Content-Type", "application/json");
        for v in header_values {
            req = req.header(CLIENT_ATTACHED_HEADER, v);
        }
        crate::router::build_router(state)
            .oneshot(req.body(Body::from(body.to_vec())).unwrap())
            .await
            .unwrap()
    }

    async fn assert_plain_denial(
        resp: axum::response::Response,
        status: StatusCode,
        body: &str,
        why: &str,
    ) {
        // Only the 401 carries a challenge. [NIP-FI.md §Rejection table]
        let challenge = (status == StatusCode::UNAUTHORIZED).then_some("Nostr");
        assert_eq!(resp.status(), status, "{why}");
        let header = |name| {
            resp.headers()
                .get(name)
                .map(|v: &axum::http::HeaderValue| v.to_str().unwrap().to_owned())
        };
        assert_eq!(header("Content-Type").as_deref(), Some(PLAIN), "{why}");
        assert_eq!(header("WWW-Authenticate").as_deref(), challenge, "{why}");
        let got = axum::body::to_bytes(resp.into_body(), 256).await.unwrap();
        assert_eq!(got.as_ref(), body.as_bytes(), "{why}");
    }

    fn valid_body() -> &'static [u8] {
        // A syntactically valid body: the pubkey is well-formed lowercase hex.
        br#"{"pubkey":"79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"}"#
    }

    #[tokio::test]
    async fn characterize_route_header_transport_rejections() {
        use axum::http::HeaderValue;
        let state = build_test_state(1000).await;
        let missing = send_raw(Arc::clone(&state), vec![], valid_body()).await;
        assert_plain_denial(
            missing,
            StatusCode::UNAUTHORIZED,
            "authentication required\n",
            "missing header",
        )
        .await;

        let rejected: Vec<(&str, Vec<HeaderValue>)> = vec![
            (
                "repeated header",
                vec![
                    HeaderValue::from_static("Bearer aaa.bbb.ccc"),
                    HeaderValue::from_static("Bearer ddd.eee.fff"),
                ],
            ),
            (
                "comma-joined value",
                vec![HeaderValue::from_static("Bearer aaa.bbb.ccc,ddd.eee.fff")],
            ),
            (
                "non-Bearer scheme",
                vec![HeaderValue::from_static("Token aaa.bbb.ccc")],
            ),
            ("empty token", vec![HeaderValue::from_static("Bearer ")]),
            (
                "space in token",
                vec![HeaderValue::from_static("Bearer aaa bbb")],
            ),
            (
                "tab in token",
                vec![HeaderValue::from_static("Bearer aaa\tbbb")],
            ),
            // U+00A0 is `char::is_whitespace` but not ASCII whitespace; the
            // obs-text bytes fail `HeaderValue::to_str`, so it rejects before
            // any whitespace predicate runs.
            (
                "non-ASCII whitespace in token",
                vec![HeaderValue::from_bytes(b"Bearer aaa\xc2\xa0bbb").unwrap()],
            ),
        ];
        // A malformed body makes a dropped transport check observable: it
        // would surface as 400 from body parsing, not the same 403 the
        // verifier would give.
        for (why, values) in rejected {
            let resp = send_raw(Arc::clone(&state), values, b"not json").await;
            assert_plain_denial(resp, StatusCode::FORBIDDEN, "evidence rejected\n", why).await;
        }
    }

    #[tokio::test]
    async fn characterize_route_check_order_before_verify() {
        use axum::http::HeaderValue;
        let state = build_test_state(1000).await;
        let bearer = || vec![HeaderValue::from_static("Bearer aaa.bbb.ccc")];

        // Header is checked before the JSON body.
        let resp = send_raw(Arc::clone(&state), vec![], b"not json").await;
        assert_plain_denial(
            resp,
            StatusCode::UNAUTHORIZED,
            "authentication required\n",
            "missing header + malformed JSON",
        )
        .await;
        let resp = send_raw(
            Arc::clone(&state),
            vec![HeaderValue::from_static("Token x")],
            br#"{"pubkey":"NOT-HEX"}"#,
        )
        .await;
        assert_plain_denial(
            resp,
            StatusCode::FORBIDDEN,
            "evidence rejected\n",
            "bad scheme + bad pubkey",
        )
        .await;

        // Body shape is checked before the (garbage) token is verified.
        for (why, body) in [
            ("malformed JSON", b"not json".as_slice()),
            ("missing pubkey field", br#"{}"#.as_slice()),
            (
                "uppercase pubkey",
                br#"{"pubkey":"79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798"}"#
                    .as_slice(),
            ),
            ("short pubkey", br#"{"pubkey":"abcd"}"#.as_slice()),
        ] {
            let resp = send_raw(Arc::clone(&state), bearer(), body).await;
            assert_plain_denial(resp, StatusCode::BAD_REQUEST, "bad request\n", why).await;
        }

        // No verifier: body/pubkey still precede the verifier check, and the
        // verifier check precedes verification (the token is garbage).
        let mut no_verifier = build_test_app_state(1000, crate::config::Config::for_test()).await;
        no_verifier.nip_fi_command_verifier = None;
        let no_verifier = Arc::new(no_verifier);
        let resp = send_raw(Arc::clone(&no_verifier), bearer(), br#"{"pubkey":"abcd"}"#).await;
        assert_plain_denial(
            resp,
            StatusCode::BAD_REQUEST,
            "bad request\n",
            "bad pubkey precedes verifier presence",
        )
        .await;
        let resp = send_raw(no_verifier, bearer(), valid_body()).await;
        assert_plain_denial(
            resp,
            StatusCode::SERVICE_UNAVAILABLE,
            "authorization unavailable\n",
            "no command verifier configured",
        )
        .await;
    }

    #[tokio::test]
    async fn characterize_route_command_error_arms() {
        let post = |state: Arc<crate::state::AppState>, token: String, target: String| async move {
            do_request(
                state,
                "POST",
                vec![
                    ("Content-Type", "application/json".into()),
                    (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
                ],
                Some(serde_json::json!({ "pubkey": target })),
            )
            .await
        };
        let state = build_test_state(1).await;

        let t = target_hex();
        let tampered = format!("{}X", mint_token(&t, 300, serde_json::json!({})));
        let resp = post(Arc::clone(&state), tampered, t).await;
        assert_plain_denial(
            resp,
            StatusCode::FORBIDDEN,
            "evidence rejected\n",
            "EvidenceRejected",
        )
        .await;

        let t = target_hex();
        let token = mint_token(&t, 300, serde_json::json!({"sub": "intruder@example.com"}));
        let resp = post(Arc::clone(&state), token, t).await;
        assert_plain_denial(
            resp,
            StatusCode::FORBIDDEN,
            "authorization denied\n",
            "AuthorizationDenied",
        )
        .await;

        let t = target_hex();
        let token = mint_token(&t, 10 * 365 * 24 * 3600, serde_json::json!({}));
        let resp = post(Arc::clone(&state), token, t).await;
        assert_plain_denial(
            resp,
            StatusCode::BAD_REQUEST,
            "bad request\n",
            "UntilExceedsCeiling",
        )
        .await;

        // Fill the capacity-1 deny set, then the next distinct target is full.
        let t = target_hex();
        let resp = post(
            Arc::clone(&state),
            mint_token(&t, 300, serde_json::json!({})),
            t,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK, "control: slot filled");
        let t = target_hex();
        let resp = post(
            Arc::clone(&state),
            mint_token(&t, 300, serde_json::json!({})),
            t,
        )
        .await;
        assert_plain_denial(
            resp,
            StatusCode::SERVICE_UNAVAILABLE,
            "deny set full\n",
            "DenySetFull",
        )
        .await;

        // An unseeded key source has no key set for the issuer.
        let mut unseeded = build_test_app_state(1000, crate::config::Config::for_test()).await;
        let key_source = Arc::new(
            ProductionJwksSource::new(vec![test_jwks_config()], buzz_auth::HttpJwksFetcher::new())
                .expect("key source"),
        );
        let mut registry = IssuerRegistry::new();
        registry.insert(test_issuer_policy());
        let deny_map = NipFiDenyMap::new(
            1000,
            vec![IssuerCapacity {
                issuer: TEST_ISS.to_owned(),
                capacity: 1000,
            }],
        );
        let policy =
            CommandIssuerPolicy::new(TEST_ISS.to_owned(), 30, vec![TEST_SUB.to_owned()], 1000)
                .expect("command policy");
        unseeded.nip_fi_command_verifier = Some(Arc::new(CommandVerifier::new(
            registry,
            key_source,
            vec![policy],
            deny_map,
        )));
        let t = target_hex();
        let token = mint_token(&t, 300, serde_json::json!({}));
        let resp = post(Arc::new(unseeded), token, t).await;
        assert_plain_denial(
            resp,
            StatusCode::SERVICE_UNAVAILABLE,
            "authorization unavailable\n",
            "AuthorizationUnavailable",
        )
        .await;
    }

    /// Pins: an admin disconnect counts its accept and its capacity rejection
    /// on the real counters only in enforce; a shadow pod answers the same
    /// way but counts both on its shadow disconnect counter.
    /// Mutation: reverting either admin site to its raw counter puts a real
    /// series in the shadow run.
    #[test]
    fn admin_disconnect_counters_stay_off_enforce_series_in_shadow() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");
        for (mode, expected) in [
            (
                NipFiMode::Enforce,
                vec![
                    "buzz_nip_fi_disconnect_capacity_rejections_total",
                    "buzz_nip_fi_disconnect_total",
                    "buzz_nip_fi_sessions_closed_total admin_disconnect",
                ],
            ),
            (
                NipFiMode::Shadow,
                vec![
                    "buzz_nip_fi_shadow_disconnect_total admin accepted",
                    "buzz_nip_fi_shadow_disconnect_total admin capacity",
                ],
            ),
        ] {
            let mut config = crate::config::Config::for_test();
            config.nip_fi.mode = mode;
            config.nip_fi.registry.insert(test_issuer_policy());
            let state = Arc::new(rt.block_on(build_test_app_state(1, config)));
            let recorder = metrics_util::debugging::DebuggingRecorder::new();
            let snapshotter = recorder.snapshotter();
            let statuses = metrics::with_local_recorder(&recorder, || {
                rt.block_on(async {
                    let mut statuses = Vec::new();
                    for _ in 0..2 {
                        let t = target_hex();
                        let token = mint_token(&t, 300, serde_json::json!({}));
                        statuses.push(post_command(&state, &token, &t).await.0);
                    }
                    statuses
                })
            });
            assert_eq!(
                statuses,
                [StatusCode::OK, StatusCode::SERVICE_UNAVAILABLE],
                "{mode:?}"
            );
            let mut series: Vec<String> = snapshotter
                .snapshot()
                .into_vec()
                .into_iter()
                .map(|(key, ..)| {
                    let key = key.key();
                    let labels = key.labels().map(|l| format!(" {}", l.value()));
                    format!("{}{}", key.name(), labels.collect::<String>())
                })
                .filter(|name| name.starts_with("buzz_nip_fi"))
                .collect();
            series.sort();
            assert_eq!(series, expected, "{mode:?}");
        }
    }

    /// `buzz_nip_fi_disconnect_capacity_rejections_total` counts exactly the
    /// `DenySetFull` rejections and no other command-error arm.
    ///
    /// `#[test]` with a current-thread runtime, not `#[tokio::test]`:
    /// `metrics::with_local_recorder` is thread-local and takes a sync closure,
    /// so each request runs to completion on this thread inside its own
    /// recorder's scope.
    #[test]
    fn capacity_rejection_counter_counts_only_deny_set_full() {
        use axum::body::Bytes;

        fn capacity_rejections_during(
            rt: &tokio::runtime::Runtime,
            request: impl std::future::Future<Output = (StatusCode, Bytes)>,
        ) -> ((StatusCode, Bytes), u64) {
            let recorder = metrics_util::debugging::DebuggingRecorder::new();
            let snapshotter = recorder.snapshotter();
            let response = metrics::with_local_recorder(&recorder, || rt.block_on(request));
            let count = snapshotter
                .snapshot()
                .into_vec()
                .into_iter()
                .find(|(key, ..)| {
                    key.key().name() == "buzz_nip_fi_disconnect_capacity_rejections_total"
                })
                .map_or(0, |(.., value)| match value {
                    metrics_util::debugging::DebugValue::Counter(n) => n,
                    other => panic!("capacity rejections must be a counter, got {other:?}"),
                });
            (response, count)
        }

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current_thread runtime");
        let state = rt.block_on(build_test_state(1));

        let t = target_hex();
        let tampered = format!("{}X", mint_token(&t, 300, serde_json::json!({})));
        let intruder = mint_token(&t, 300, serde_json::json!({"sub": "intruder@example.com"}));
        let far_until = mint_token(&t, 10 * 365 * 24 * 3600, serde_json::json!({}));
        for (why, token, status, body) in [
            (
                "EvidenceRejected",
                tampered,
                StatusCode::FORBIDDEN,
                "evidence rejected\n",
            ),
            (
                "AuthorizationDenied",
                intruder,
                StatusCode::FORBIDDEN,
                "authorization denied\n",
            ),
            (
                "UntilExceedsCeiling",
                far_until,
                StatusCode::BAD_REQUEST,
                "bad request\n",
            ),
        ] {
            let (response, count) =
                capacity_rejections_during(&rt, post_command(&state, &token, &t));
            assert_eq!(
                response,
                (status, Bytes::from_static(body.as_bytes())),
                "{why}"
            );
            assert_eq!(count, 0, "{why} must not count as a capacity rejection");
        }

        // Fill the capacity-1 deny set, then the next distinct target is full.
        let t = target_hex();
        let (filled, _) = rt.block_on(post_command(
            &state,
            &mint_token(&t, 300, serde_json::json!({})),
            &t,
        ));
        assert_eq!(filled, StatusCode::OK, "control: slot filled");
        let t = target_hex();
        let token = mint_token(&t, 300, serde_json::json!({}));
        let (response, count) = capacity_rejections_during(&rt, post_command(&state, &token, &t));
        assert_eq!(
            response,
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Bytes::from_static(b"deny set full\n")
            ),
            "DenySetFull"
        );
        assert_eq!(count, 1, "DenySetFull must count one capacity rejection");
    }

    // ── Cross-pod command replay fence ───────────────────────────────────────
    //
    // Each "pod" is its own AppState with its own deny map and sessions; the
    // pods share only the command replay guard, as production pods share Redis.

    async fn pod(capacity: usize, replay: Arc<dyn buzz_auth::CommandReplayGuard>) -> Arc<AppState> {
        let mut state = build_test_app_state(capacity, crate::config::Config::for_test()).await;
        state.nip_fi_command_replay = replay;
        Arc::new(state)
    }

    async fn post_command(
        state: &Arc<AppState>,
        token: &str,
        target: &str,
    ) -> (StatusCode, axum::body::Bytes) {
        let resp = do_request(
            Arc::clone(state),
            "POST",
            vec![
                ("Content-Type", "application/json".into()),
                (CLIENT_ATTACHED_HEADER, format!("Bearer {token}")),
            ],
            Some(serde_json::json!({"pubkey": target})),
        )
        .await;
        let status = resp.status();
        (
            status,
            axum::body::to_bytes(resp.into_body(), 64).await.unwrap(),
        )
    }

    fn is_denied(state: &AppState, key: &nostr::PublicKey) -> bool {
        state
            .nip_fi_deny_map
            .as_deref()
            .expect("deny map")
            .is_denied(TEST_ISS, key, chrono::Utc::now())
    }

    /// Counts cross-pod disconnect publications for one target on the Redis
    /// channel the pods publish to.  Tests filter by target, so concurrent
    /// tests sharing Redis do not interfere.
    struct PublishObserver {
        rx: tokio::sync::mpsc::UnboundedReceiver<buzz_pubsub::NipFiDisconnect>,
        target: Vec<u8>,
        seen: usize,
    }

    impl PublishObserver {
        async fn subscribe(state: &AppState, target: &nostr::PublicKey) -> Self {
            use futures::StreamExt;
            let client = redis::Client::open(state.config.redis_url.as_str()).expect("redis url");
            let mut conn = client.get_async_pubsub().await.expect("redis pubsub");
            // Acknowledged by Redis before returning, so no later publish is missed.
            conn.subscribe(buzz_pubsub::conn_control::NIP_FI_DISCONNECT_CHANNEL)
                .await
                .expect("subscribe");
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(async move {
                let mut messages = conn.into_on_message();
                // A decode panic or stream end drops `tx`, which `count` reports.
                while let Some(msg) = messages.next().await {
                    let payload: String = msg.get_payload().expect("payload");
                    let cmd = buzz_pubsub::decode_nip_fi_disconnect(&payload).expect("decode");
                    if tx.send(cmd).is_err() {
                        break;
                    }
                }
            });
            Self {
                rx,
                target: target.to_bytes().to_vec(),
                seen: 0,
            }
        }

        /// Publications for the target so far.
        ///
        /// Barrier: wait until every publish spawned by each pod in `drain`
        /// (every pod that could have published) has finished, so each
        /// earlier `PUBLISH` is complete.  Then send an accepted
        /// sentinel command and read up to its publication.  It is published
        /// strictly after the earlier ones and Redis delivers in publish
        /// order, so the read drains them all; it is also a positive control
        /// that the publisher and this collector both work.
        async fn count(&mut self, drain: &[&Arc<AppState>], via: &Arc<AppState>) -> usize {
            for state in drain {
                drain_publishes(state).await;
            }
            let sentinel = nostr::Keys::generate().public_key();
            let token = mint_token(&sentinel.to_hex(), 300, serde_json::json!({}));
            let (status, _) = post_command(via, &token, &sentinel.to_hex()).await;
            assert_eq!(status, StatusCode::OK, "sentinel command must be accepted");
            let sentinel = sentinel.to_bytes().to_vec();
            let deadline = std::time::Duration::from_secs(10);
            loop {
                let cmd = tokio::time::timeout(deadline, self.rx.recv())
                    .await
                    .expect("sentinel publication did not arrive within 10s")
                    .expect("publish collector stopped");
                if cmd.pubkey_bytes == self.target {
                    self.seen += 1;
                } else if cmd.pubkey_bytes == sentinel {
                    return self.seen;
                }
            }
        }
    }

    /// Wait until every publish `state` has spawned so far has finished.
    async fn drain_publishes(state: &AppState) {
        let tasks = &state.nip_fi_publish_tasks;
        tasks.close();
        tokio::time::timeout(std::time::Duration::from_secs(10), tasks.wait())
            .await
            .expect("spawned publishes did not finish within 10s");
        tasks.reopen();
    }

    async fn observe(
        enabled: bool,
        state: &AppState,
        target: &nostr::PublicKey,
    ) -> Option<PublishObserver> {
        match enabled {
            true => Some(PublishObserver::subscribe(state, target).await),
            false => None,
        }
    }

    async fn assert_publishes(
        observer: &mut Option<PublishObserver>,
        drain: &[&Arc<AppState>],
        via: &Arc<AppState>,
        expected: usize,
        what: &str,
    ) {
        if let Some(observer) = observer {
            assert_eq!(
                observer.count(drain, via).await,
                expected,
                "publications: {what}"
            );
        }
    }

    async fn command_replayed_on_second_pod_is_denied_without_effect(
        replay_a: Arc<dyn buzz_auth::CommandReplayGuard>,
        replay_b: Arc<dyn buzz_auth::CommandReplayGuard>,
        observe_publishes: bool,
    ) {
        let pod_a = pod(1000, replay_a).await;
        let pod_b = pod(1000, replay_b).await;
        let key = nostr::Keys::generate().public_key();
        let on_b = IssuerSessions::register(&pod_b, TEST_ISS, &key);
        let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
        let mut publishes = observe(observe_publishes, &pod_a, &key).await;

        let (status, _) = post_command(&pod_a, &token, &key.to_hex()).await;
        assert_eq!(status, StatusCode::OK, "first use is accepted on pod A");
        assert_publishes(
            &mut publishes,
            &[&pod_a, &pod_b],
            &pod_a,
            1,
            "accepted command",
        )
        .await;

        let (status, body) = post_command(&pod_b, &token, &key.to_hex()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "replay on pod B is denied");
        assert_eq!(body.as_ref(), b"authorization denied\n");
        assert!(
            !is_denied(&pod_b, &key),
            "replay must not insert a deny entry on B"
        );
        on_b.assert_open("pod B after rejected replay");
        assert_publishes(
            &mut publishes,
            &[&pod_a, &pod_b],
            &pod_b,
            1,
            "rejected replay adds none",
        )
        .await;
    }

    #[tokio::test]
    async fn command_replay_on_second_pod_is_denied() {
        let shared: Arc<dyn buzz_auth::CommandReplayGuard> =
            Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        command_replayed_on_second_pod_is_denied_without_effect(Arc::clone(&shared), shared, false)
            .await;
    }

    /// Delegates to a real guard and counts `try_claim` calls.
    struct CountingReplayGuard {
        inner: buzz_auth::InMemoryCommandReplayGuard,
        claims: std::sync::atomic::AtomicUsize,
    }

    impl buzz_auth::CommandReplayGuard for CountingReplayGuard {
        fn try_claim<'a>(
            &'a self,
            claim: buzz_auth::CommandClaim<'a>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<buzz_auth::ClaimOutcome, buzz_auth::AuthError>,
                    > + Send
                    + 'a,
            >,
        > {
            self.claims
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.try_claim(claim)
        }

        fn load_denies<'a>(
            &'a self,
            issuer: &'a str,
            now: chrono::DateTime<chrono::Utc>,
        ) -> buzz_auth::LoadDenies<'a> {
            self.inner.load_denies(issuer, now)
        }
    }

    /// A command whose signature does not verify but which carries a real
    /// command's `(iss, jti)` must not consume that command's shared claim.
    async fn forged_command_leaves_shared_claim_unconsumed(
        replay: Arc<dyn buzz_auth::CommandReplayGuard>,
        claims: Option<&CountingReplayGuard>,
        observe_publishes: bool,
    ) {
        let pod_a = pod(1000, Arc::clone(&replay)).await;
        let pod_b = pod(1000, replay).await;
        let key = nostr::Keys::generate().public_key();
        let on_a = IssuerSessions::register(&pod_a, TEST_ISS, &key);
        let jti = serde_json::json!({ "jti": uuid::Uuid::new_v4().to_string() });
        let legit = mint_token(&key.to_hex(), 300, jti.clone());
        // Legitimate header and payload, with a well-formed signature over a
        // different payload.
        let other = mint_token(&target_hex(), 300, jti);
        let (signed_part, _) = legit.rsplit_once('.').expect("compact JWS");
        let (_, foreign_sig) = other.rsplit_once('.').expect("compact JWS");
        let forged = format!("{signed_part}.{foreign_sig}");
        let mut publishes = observe(observe_publishes, &pod_a, &key).await;

        let (status, body) = post_command(&pod_a, &forged, &key.to_hex()).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "forged signature is rejected"
        );
        assert_eq!(body.as_ref(), b"evidence rejected\n");
        assert!(!is_denied(&pod_a, &key), "forgery inserts no deny entry");
        on_a.assert_open("pod A after forgery");
        assert_publishes(&mut publishes, &[&pod_a, &pod_b], &pod_a, 0, "forgery").await;
        if let Some(guard) = claims {
            assert_eq!(
                guard.claims.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "invalid evidence must never reach the shared guard"
            );
        }

        let (status, _) = post_command(&pod_b, &legit, &key.to_hex()).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the legitimate command's jti was not burned by the forgery"
        );
        assert!(is_denied(&pod_b, &key));
        assert_publishes(
            &mut publishes,
            &[&pod_a, &pod_b],
            &pod_b,
            1,
            "accepted command",
        )
        .await;
    }

    #[tokio::test]
    async fn forged_command_does_not_consume_shared_claim() {
        let guard = Arc::new(CountingReplayGuard {
            inner: buzz_auth::InMemoryCommandReplayGuard::default(),
            claims: std::sync::atomic::AtomicUsize::new(0),
        });
        forged_command_leaves_shared_claim_unconsumed(guard.clone(), Some(&guard), false).await;
    }

    // ── Durable deny store: restore and unknown outcomes ────────────────────

    /// A pod whose registry holds the test issuer (so restores apply), sharing
    /// `store` as production pods share Redis.
    async fn store_pod(
        capacity: usize,
        store: Arc<dyn buzz_auth::CommandReplayGuard>,
    ) -> Arc<AppState> {
        let mut config = crate::config::Config::for_test();
        config.nip_fi.registry.insert(test_issuer_policy());
        let mut state = build_test_app_state(capacity, config).await;
        state.nip_fi_command_replay = store;
        Arc::new(state)
    }

    /// Restart: a fresh pod is closed until it restores, then denies exactly
    /// what other pods accepted.
    #[tokio::test]
    async fn restarted_pod_restores_denies_from_the_shared_store() {
        let store: Arc<dyn buzz_auth::CommandReplayGuard> =
            Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let pod_a = store_pod(1000, Arc::clone(&store)).await;
        let key = nostr::Keys::generate().public_key();
        let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
        assert_eq!(
            post_command(&pod_a, &token, &key.to_hex()).await.0,
            StatusCode::OK
        );

        let pod_b = store_pod(1000, Arc::clone(&store)).await;
        let deny_map = pod_b.nip_fi_deny_map.as_deref().unwrap();
        let generation = deny_map.begin_restore();
        let other = nostr::Keys::generate().public_key();
        assert!(is_denied(&pod_b, &other), "closed before restore");
        restore_nip_fi_denies(&pod_b, &[store])
            .await
            .expect("restore");
        assert!(deny_map.finish_restore(generation));
        assert!(is_denied(&pod_b, &key), "restored deny");
        assert!(!is_denied(&pod_b, &other), "only restored keys are denied");
    }

    /// Redis down at startup: the subscription never confirms, so the
    /// consumer never restores and admission stays closed.  A store that
    /// fails to load keeps it closed too.
    #[tokio::test]
    async fn unavailable_store_keeps_admission_closed() {
        let pod = store_pod(
            1000,
            Arc::new(buzz_auth::InMemoryCommandReplayGuard::default()),
        )
        .await;
        let key = nostr::Keys::generate().public_key();
        pod.nip_fi_deny_map.as_deref().unwrap().begin_restore();
        let (sub_tx, sub_rx) =
            tokio::sync::watch::channel(buzz_pubsub::NipFiSubscription::default());
        let consumer = tokio::spawn(run_nip_fi_disconnect_consumer(
            Arc::clone(&pod),
            pod.pubsub.subscribe_nip_fi_disconnect(),
            sub_rx,
            vec![Arc::new(FailingReplayGuard {
                load_fails: true,
                ..Default::default()
            })],
        ));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(is_denied(&pod, &key), "no subscription: closed");
        sub_tx.send_modify(|s| {
            s.epoch += 1;
            s.live = true;
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(is_denied(&pod, &key), "store unreadable: closed");
        assert_eq!(
            restore_nip_fi_denies(
                &pod,
                &[Arc::new(FailingReplayGuard {
                    load_fails: true,
                    ..Default::default()
                })]
            )
            .await,
            Err("store")
        );
        consumer.abort();
    }

    /// The consumer reopens admission once subscribed and restored, and
    /// closes it again when the subscription is lost.
    #[tokio::test]
    async fn consumer_reopens_only_for_a_live_restored_subscription() {
        let store: Arc<dyn buzz_auth::CommandReplayGuard> =
            Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let pod = store_pod(1000, Arc::clone(&store)).await;
        let key = nostr::Keys::generate().public_key();
        pod.nip_fi_deny_map.as_deref().unwrap().begin_restore();
        let (sub_tx, sub_rx) =
            tokio::sync::watch::channel(buzz_pubsub::NipFiSubscription::default());
        let consumer = tokio::spawn(run_nip_fi_disconnect_consumer(
            Arc::clone(&pod),
            pod.pubsub.subscribe_nip_fi_disconnect(),
            sub_rx,
            vec![store],
        ));
        let set = |live| {
            sub_tx.send_modify(|s| {
                s.epoch += 1;
                s.live = live;
            })
        };
        let settle = || tokio::time::sleep(std::time::Duration::from_millis(50));
        set(true);
        settle().await;
        assert!(!is_denied(&pod, &key), "open once restored");
        set(false);
        settle().await;
        assert!(
            is_denied(&pod, &key),
            "closed after losing the subscription"
        );
        set(true);
        settle().await;
        assert!(!is_denied(&pod, &key), "open again after a fresh restore");
        consumer.abort();
    }

    /// A store whose first load misses `hidden` (a deny accepted while that
    /// snapshot was being read), and which counts its loads.
    struct StaleFirstLoad {
        inner: Arc<buzz_auth::InMemoryCommandReplayGuard>,
        hidden: std::sync::Mutex<Option<nostr::PublicKey>>,
        loads: tokio::sync::watch::Sender<usize>,
    }

    impl StaleFirstLoad {
        fn new(
            inner: Arc<buzz_auth::InMemoryCommandReplayGuard>,
            hidden: Option<nostr::PublicKey>,
        ) -> Arc<Self> {
            Arc::new(Self {
                inner,
                hidden: std::sync::Mutex::new(hidden),
                loads: tokio::sync::watch::channel(0).0,
            })
        }

        /// Waits until the consumer has started its `n`th restore load.
        async fn loaded(&self, n: usize) {
            let mut loads = self.loads.subscribe();
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                loads.wait_for(|loads| *loads >= n),
            )
            .await
            .expect("consumer progress")
            .unwrap();
        }
    }

    /// Waits until `pod`'s consumer has reopened admission.
    async fn restored(pod: &AppState) {
        let map = pod.nip_fi_deny_map.as_deref().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !map.is_restored() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("restored");
    }

    impl buzz_auth::CommandReplayGuard for StaleFirstLoad {
        fn try_claim<'a>(
            &'a self,
            claim: buzz_auth::CommandClaim<'a>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<buzz_auth::ClaimOutcome, buzz_auth::AuthError>,
                    > + Send
                    + 'a,
            >,
        > {
            self.inner.try_claim(claim)
        }

        fn load_denies<'a>(
            &'a self,
            issuer: &'a str,
            now: chrono::DateTime<chrono::Utc>,
        ) -> buzz_auth::LoadDenies<'a> {
            Box::pin(async move {
                self.loads.send_modify(|loads| *loads += 1);
                let hidden = self.hidden.lock().unwrap().take();
                let mut denies = self.inner.load_denies(issuer, now).await?;
                denies.retain(|(key, _)| Some(*key) != hidden);
                Ok(denies)
            })
        }
    }

    /// Reads from `inner`, reports the read on `read`, then holds the answer
    /// until `release`, so a test controls the time between read and merge.
    struct GatedLoad {
        inner: buzz_auth::InMemoryCommandReplayGuard,
        read: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }

    impl buzz_auth::CommandReplayGuard for GatedLoad {
        fn try_claim<'a>(
            &'a self,
            claim: buzz_auth::CommandClaim<'a>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<buzz_auth::ClaimOutcome, buzz_auth::AuthError>,
                    > + Send
                    + 'a,
            >,
        > {
            self.inner.try_claim(claim)
        }

        fn load_denies<'a>(
            &'a self,
            issuer: &'a str,
            now: chrono::DateTime<chrono::Utc>,
        ) -> buzz_auth::LoadDenies<'a> {
            Box::pin(async move {
                let denies = self.inner.load_denies(issuer, now).await;
                self.read.notify_one();
                self.release.notified().await;
                denies
            })
        }
    }

    /// Restore judges expiry when it merges, not when it started reading: a
    /// whole key that lapses during a slow load no longer overrides the
    /// stored cutoff.
    #[tokio::test]
    async fn restore_judges_expiry_at_the_merge() {
        use buzz_auth::CommandReplayGuard as _;
        let source = Arc::new(GatedLoad {
            inner: buzz_auth::InMemoryCommandReplayGuard::default(),
            read: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        let key = nostr::Keys::generate().public_key();
        let now = chrono::Utc::now();
        let c = now.timestamp();
        let outcome = source
            .try_claim(buzz_auth::CommandClaim {
                issuer: TEST_ISS,
                jti: "cutoff",
                ttl_secs: 60,
                target: &key,
                until: now + chrono::Duration::seconds(120),
                scope: buzz_auth::DenyScope::IssuedThrough(c),
                capacity: 10,
                now,
            })
            .await
            .unwrap();
        assert_eq!(outcome, buzz_auth::ClaimOutcome::Claimed);
        let dyn_source: Arc<dyn buzz_auth::CommandReplayGuard> = source.clone();
        let pod = store_pod(10, Arc::clone(&dyn_source)).await;
        let deny_map = pod.nip_fi_deny_map.as_deref().unwrap();
        let lapse = chrono::Utc::now() + chrono::Duration::milliseconds(200);
        deny_map.merge_cross_pod_deny(
            TEST_ISS,
            &key,
            buzz_auth::DenyEntry::whole_key(lapse),
            chrono::Utc::now,
        );
        let generation = deny_map.begin_restore();
        let sources = [dyn_source];
        let restore = restore_nip_fi_denies(&pod, &sources);
        tokio::pin!(restore);
        tokio::select! {
            _ = &mut restore => panic!("restore finished before the gated read"),
            () = source.read.notified() => {}
        }
        assert!(
            chrono::Utc::now() < lapse,
            "the whole key is live at the read"
        );
        let wait = (lapse - chrono::Utc::now()).to_std().unwrap_or_default();
        tokio::time::sleep(wait + std::time::Duration::from_millis(50)).await;
        source.release.notify_one();
        assert_eq!(restore.await, Ok(()));
        assert!(deny_map.finish_restore(generation));
        let after = chrono::DateTime::from_timestamp(c, 0).unwrap() + chrono::Duration::seconds(1);
        assert!(
            !deny_map.denies(TEST_ISS, &key, after, chrono::Utc::now()),
            "the whole key lapsed before the merge and gave way to the cutoff"
        );
    }

    /// Records a deny for `target` in `store` as another pod's accept would.
    async fn store_deny(
        store: &buzz_auth::InMemoryCommandReplayGuard,
        target: &nostr::PublicKey,
    ) -> buzz_pubsub::NipFiDisconnect {
        store_deny_for(store, TEST_ISS, target).await
    }

    async fn store_deny_for(
        store: &buzz_auth::InMemoryCommandReplayGuard,
        issuer: &str,
        target: &nostr::PublicKey,
    ) -> buzz_pubsub::NipFiDisconnect {
        use buzz_auth::CommandReplayGuard as _;
        let until = chrono::Utc::now() + chrono::Duration::seconds(300);
        let jti = uuid::Uuid::new_v4().to_string();
        let outcome = store
            .try_claim(buzz_auth::CommandClaim {
                issuer,
                jti: &jti,
                ttl_secs: 60,
                target,
                until,
                scope: buzz_auth::DenyScope::WholeKey,
                capacity: 10,
                now: chrono::Utc::now(),
            })
            .await
            .unwrap();
        assert_eq!(outcome, buzz_auth::ClaimOutcome::Claimed);
        buzz_pubsub::NipFiDisconnect {
            issuer: issuer.to_owned(),
            pubkey_bytes: target.to_bytes().to_vec(),
            until_unix: until.timestamp(),
            until_unix_nanos: until.timestamp_subsec_nanos(),
            revoke_through: None,
        }
    }

    /// Runs the consumer for a capacity-one pod over `source`, fed by the
    /// returned broadcast sender, with a live subscription.
    async fn capacity_one_consumer(
        source: Arc<dyn buzz_auth::CommandReplayGuard>,
    ) -> (
        Arc<AppState>,
        tokio::sync::broadcast::Sender<buzz_pubsub::NipFiDisconnect>,
        tokio::sync::watch::Sender<buzz_pubsub::NipFiSubscription>,
        tokio::task::JoinHandle<()>,
    ) {
        let pod = store_pod(1, Arc::clone(&source)).await;
        pod.nip_fi_deny_map.as_deref().unwrap().begin_restore();
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        let (sub_tx, sub_rx) =
            tokio::sync::watch::channel(buzz_pubsub::NipFiSubscription::default());
        let consumer = tokio::spawn(run_nip_fi_disconnect_consumer(
            Arc::clone(&pod),
            rx,
            sub_rx,
            vec![source],
        ));
        (pod, tx, sub_tx, consumer)
    }

    fn go_live(sub_tx: &tokio::sync::watch::Sender<buzz_pubsub::NipFiSubscription>) {
        sub_tx.send_modify(|s| {
            s.epoch += 1;
            s.live = true;
        });
    }

    /// A partly unreadable load on a shadow pod applies the readable records
    /// through the shadow close path, which closes no real session, and
    /// still fails, so the generation never finishes.
    #[tokio::test]
    async fn shadow_partial_restore_observes_and_stays_unrestored() {
        let source = Arc::new(FailingReplayGuard {
            load_unreadable: true.into(),
            ..Default::default()
        });
        let key = nostr::Keys::generate().public_key();
        let message = store_deny(&source.inner, &key).await;
        let mut config = crate::config::Config::for_test();
        config.nip_fi.mode = NipFiMode::Shadow;
        config.nip_fi.registry.insert(test_issuer_policy());
        let mut state = build_test_app_state(10, config).await;
        state.nip_fi_command_replay = Arc::clone(&source) as _;
        let pod = Arc::new(state);
        let deny_map = pod.nip_fi_deny_map.as_deref().unwrap();
        let session = IssuerSessions::register(&pod, TEST_ISS, &key);
        deny_map.begin_restore();
        assert_eq!(
            restore_nip_fi_denies(&pod, &[source as _]).await,
            Err("unreadable")
        );
        session.assert_open("shadow only observes");
        assert!(
            !deny_map.is_restored(),
            "a partial load leaves it restoring"
        );
        assert_eq!(
            merge_nip_fi_disconnect(&pod, &message, chrono::Utc::now),
            NipFiDisconnectApplyResult::Applied(buzz_auth::CrossPodMergeResult::Unchanged),
            "the readable record was applied"
        );
    }

    /// The consumer's own timer retries a partly unreadable restore: the
    /// first attempt applies the readable record and stays closed; once the
    /// bad record is gone the next attempt reopens.
    #[tokio::test]
    async fn consumer_retries_a_partly_unreadable_restore() {
        let source = Arc::new(FailingReplayGuard {
            load_unreadable: true.into(),
            ..Default::default()
        });
        let (revoked, bystander) = (
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
        );
        store_deny(&source.inner, &revoked).await;
        let pod = store_pod(10, Arc::clone(&source) as _).await;
        let deny_map = pod.nip_fi_deny_map.as_deref().unwrap();
        deny_map.begin_restore();
        let sessions = [&revoked, &bystander].map(|k| IssuerSessions::register(&pod, TEST_ISS, k));
        let (_tx, rx) = tokio::sync::broadcast::channel(16);
        let (sub_tx, sub_rx) =
            tokio::sync::watch::channel(buzz_pubsub::NipFiSubscription::default());
        let consumer = tokio::spawn(run_nip_fi_disconnect_consumer(
            Arc::clone(&pod),
            rx,
            sub_rx,
            vec![Arc::clone(&source) as _],
        ));
        tokio::time::pause();
        go_live(&sub_tx);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(source.loads.load(std::sync::atomic::Ordering::SeqCst), 1);
        sessions[0].assert_closed("readable record past a bad one");
        sessions[1].assert_open("not revoked");
        assert!(!deny_map.is_restored(), "unreadable keeps admission closed");

        source
            .load_unreadable
            .store(false, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        assert_eq!(source.loads.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(deny_map.is_restored(), "the timer's retry reopens");
        assert!(!is_denied(&pod, &bystander));
        assert!(is_denied(&pod, &revoked));
        consumer.abort();
    }

    /// While restores keep failing, delivered disconnects still close their
    /// sessions, admission stays closed, and steady traffic does not postpone
    /// the next restore attempt.
    #[tokio::test]
    async fn failing_restore_still_closes_revoked_sessions() {
        let store = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let failing = Arc::new(FailingReplayGuard {
            load_fails: true,
            ..Default::default()
        });
        let pod = store_pod(10, Arc::clone(&failing) as _).await;
        pod.nip_fi_deny_map.as_deref().unwrap().begin_restore();
        let (tx, rx) = tokio::sync::broadcast::channel(16);
        let (sub_tx, sub_rx) =
            tokio::sync::watch::channel(buzz_pubsub::NipFiSubscription::default());
        let consumer = tokio::spawn(run_nip_fi_disconnect_consumer(
            Arc::clone(&pod),
            rx,
            sub_rx,
            vec![Arc::clone(&failing) as _],
        ));
        let (a, b, bystander, other) = (
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
        );
        let sessions = [&a, &b, &bystander].map(|k| IssuerSessions::register(&pod, TEST_ISS, k));
        // Controlled time from here on: the retry deadline, not scheduler
        // pauses, decides how many loads happen.
        tokio::time::pause();
        go_live(&sub_tx);
        // Steady traffic, a message every 100ms for 2.5s, spanning two retry
        // deadlines.
        let messages = [store_deny(&store, &a).await, store_deny(&store, &b).await];
        let start = tokio::time::Instant::now();
        let mut i = 0;
        while start.elapsed() < std::time::Duration::from_millis(2500) {
            tx.send(messages[i % 2].clone()).unwrap();
            i += 1;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        sessions[0].assert_closed("revoked during a failing restore");
        sessions[1].assert_closed("revoked during a failing restore");
        sessions[2].assert_open("not revoked");
        assert!(is_denied(&pod, &other), "admission stays closed");
        assert!(
            failing.loads.load(std::sync::atomic::Ordering::SeqCst) >= 3,
            "traffic must not postpone the retry"
        );
        consumer.abort();
    }

    /// A deny that arrives during the restore and does not fit keeps the pod
    /// closed: it restores again and finds the union over capacity.
    #[tokio::test]
    async fn capacity_miss_during_handoff_keeps_admission_closed() {
        let store = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let (a, b, other) = (
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
        );
        store_deny(&store, &a).await;
        let message = store_deny(&store, &b).await;
        let source = StaleFirstLoad::new(Arc::clone(&store), Some(b));
        let (pod, tx, sub_tx, consumer) = capacity_one_consumer(source.clone()).await;
        tx.send(message).unwrap();
        go_live(&sub_tx);
        // The second load is the restore the missed deny forced.
        source.loaded(2).await;
        assert!(
            !pod.nip_fi_deny_map.as_deref().unwrap().is_restored(),
            "a missed deny never reopens admission"
        );
        assert!(is_denied(&pod, &other));
        consumer.abort();
    }

    /// A live deny that does not fit after a successful restore closes
    /// admission again until the active union fits.
    #[tokio::test]
    async fn capacity_miss_after_restore_closes_admission() {
        let store = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let (a, b, other) = (
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
        );
        store_deny(&store, &a).await;
        let source = StaleFirstLoad::new(Arc::clone(&store), None);
        let (pod, tx, sub_tx, consumer) = capacity_one_consumer(source.clone()).await;
        go_live(&sub_tx);
        restored(&pod).await;
        assert!(!is_denied(&pod, &other), "open once restored");
        tx.send(store_deny(&store, &b).await).unwrap();
        source.loaded(2).await;
        assert!(
            !pod.nip_fi_deny_map.as_deref().unwrap().is_restored(),
            "closed after the miss"
        );
        assert!(is_denied(&pod, &other));
        consumer.abort();
    }

    /// A Redis error before the script ran: 503, and the same signed command
    /// succeeds on retry.
    #[tokio::test]
    async fn store_error_before_apply_returns_503_and_the_command_retries() {
        let store = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let failing = store_pod(
            1000,
            Arc::new(FailingReplayGuard {
                inner: Arc::clone(&store),
                ..Default::default()
            }),
        )
        .await;
        let healthy = store_pod(1000, store).await;
        let key = nostr::Keys::generate().public_key();
        let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
        assert_eq!(
            post_command(&failing, &token, &key.to_hex()).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(!is_denied(&failing, &key));
        assert_eq!(
            post_command(&healthy, &token, &key.to_hex()).await.0,
            StatusCode::OK
        );
    }

    /// The script ran but its reply was lost: 503; the same command is then a
    /// replay (403), a fresh command succeeds, and the stored deny restores.
    #[tokio::test]
    async fn lost_reply_returns_503_and_needs_a_fresh_command() {
        let store = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let lossy = store_pod(
            1000,
            Arc::new(FailingReplayGuard {
                inner: Arc::clone(&store),
                applies: true,
                ..Default::default()
            }),
        )
        .await;
        let healthy = store_pod(
            1000,
            Arc::clone(&store) as Arc<dyn buzz_auth::CommandReplayGuard>,
        )
        .await;
        let key = nostr::Keys::generate().public_key();
        let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
        assert_eq!(
            post_command(&lossy, &token, &key.to_hex()).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            post_command(&healthy, &token, &key.to_hex()).await.0,
            StatusCode::FORBIDDEN,
            "same jti is a replay"
        );

        let store: Arc<dyn buzz_auth::CommandReplayGuard> = store;
        let restarted = store_pod(1000, Arc::clone(&store)).await;
        restore_nip_fi_denies(&restarted, std::slice::from_ref(&store))
            .await
            .expect("restore");
        assert!(is_denied(&restarted, &key), "the applied deny was stored");

        let fresh = mint_token(&key.to_hex(), 300, serde_json::json!({}));
        assert_eq!(
            post_command(&healthy, &fresh, &key.to_hex()).await.0,
            StatusCode::OK
        );
    }

    /// Capacity holds across pods: each pod's local map has room, but the
    /// shared store refuses the second distinct key.
    #[tokio::test]
    async fn deny_set_capacity_is_enforced_across_pods() {
        let store: Arc<dyn buzz_auth::CommandReplayGuard> =
            Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let pod_a = store_pod(1, Arc::clone(&store)).await;
        let pod_b = store_pod(1, store).await;
        let (a, b) = (
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
        );
        let token_a = mint_token(&a.to_hex(), 300, serde_json::json!({}));
        let token_b = mint_token(&b.to_hex(), 300, serde_json::json!({}));
        assert_eq!(
            post_command(&pod_a, &token_a, &a.to_hex()).await.0,
            StatusCode::OK
        );
        assert_eq!(
            post_command(&pod_b, &token_b, &b.to_hex()).await.0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(!is_denied(&pod_b, &b), "a full shared set records nothing");
    }

    /// A restore that does not fit the local map is a failure, never a
    /// truncated success.
    #[tokio::test]
    async fn over_capacity_restore_is_not_complete() {
        let store: Arc<dyn buzz_auth::CommandReplayGuard> =
            Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let writer = store_pod(10, Arc::clone(&store)).await;
        for _ in 0..2 {
            let key = nostr::Keys::generate().public_key();
            let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
            assert_eq!(
                post_command(&writer, &token, &key.to_hex()).await.0,
                StatusCode::OK
            );
        }
        let small = store_pod(1, Arc::clone(&store)).await;
        assert_eq!(
            restore_nip_fi_denies(&small, &[store]).await,
            Err("capacity")
        );
    }

    /// A restore closes sessions only for targets it added or extended; a
    /// target whose held entry already covers the stored one stays open.
    #[tokio::test]
    async fn restore_closes_only_added_or_extended_targets() {
        let store = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let keys: Vec<_> = (0..3)
            .map(|_| nostr::Keys::generate().public_key())
            .collect();
        let (held, extended) = (&keys[0], &keys[2]);
        for key in &keys {
            store_deny(&store, key).await;
        }
        let source: Arc<dyn buzz_auth::CommandReplayGuard> = store;
        let pod = store_pod(10, Arc::clone(&source)).await;
        let map = pod.nip_fi_deny_map.as_deref().unwrap();
        let now = chrono::Utc::now();
        map.merge_cross_pod_deny(TEST_ISS, held, now + chrono::Duration::hours(1), || now);
        map.merge_cross_pod_deny(
            TEST_ISS,
            extended,
            now + chrono::Duration::seconds(5),
            || now,
        );
        let sessions: Vec<_> = keys
            .iter()
            .map(|k| IssuerSessions::register(&pod, TEST_ISS, k))
            .collect();
        map.begin_restore();
        restore_nip_fi_denies(&pod, &[source])
            .await
            .expect("restore");
        sessions[0].assert_open("held entry unchanged");
        sessions[1].assert_closed("entry added");
        sessions[2].assert_closed("entry extended");
    }

    /// A restore that fails part-way still closes every target it can reach:
    /// a failure ends neither the pass over entries, nor over issuers and
    /// sources, and the first failure still fails the restore.
    #[tokio::test]
    async fn failed_restore_still_closes_targets_it_added() {
        let store = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let keys = [(); 3].map(|()| nostr::Keys::generate().public_key());
        for key in &keys {
            store_deny(&store, key).await;
        }
        let closed = |pod: &AppState| {
            assert!(
                is_denied(pod, &nostr::Keys::generate().public_key()),
                "admission stays closed"
            );
        };

        // Capacity: the first entry fits, the next two miss.
        let full = store_pod(1, Arc::clone(&store) as _).await;
        let sessions = keys
            .each_ref()
            .map(|k| IssuerSessions::register(&full, TEST_ISS, k));
        full.nip_fi_deny_map.as_deref().unwrap().begin_restore();
        assert_eq!(
            restore_nip_fi_denies(&full, &[Arc::clone(&store) as _]).await,
            Err("capacity")
        );
        for session in &sessions {
            session.assert_closed("target past a capacity miss");
        }
        closed(&full);

        // Store: a failing source, then a healthy one.
        let pod = store_pod(10, Arc::clone(&store) as _).await;
        let sessions = keys
            .each_ref()
            .map(|k| IssuerSessions::register(&pod, TEST_ISS, k));
        pod.nip_fi_deny_map.as_deref().unwrap().begin_restore();
        let failing = Arc::new(FailingReplayGuard {
            load_fails: true,
            ..Default::default()
        });
        assert_eq!(
            restore_nip_fi_denies(&pod, &[failing, Arc::clone(&store) as _]).await,
            Err("store")
        );
        for session in &sessions {
            session.assert_closed("source after a store error");
        }
        closed(&pod);

        // Store: the first issuer the map visits fails to load; the healthy
        // issuer after it still closes.
        const OTHER_ISS: &str = "https://other-issuer.example";
        let mut config = crate::config::Config::for_test();
        config.nip_fi.registry.insert(test_issuer_policy());
        config.nip_fi.registry.insert(issuer_policy(OTHER_ISS));
        let mut state = build_test_app_state(10, config).await;
        let capacity = |issuer: &str| IssuerCapacity {
            issuer: issuer.to_owned(),
            capacity: 10,
        };
        let map = NipFiDenyMap::new(10, vec![capacity(TEST_ISS), capacity(OTHER_ISS)]);
        let mut order = map.issuers().into_iter();
        let (failed, healthy) = (order.next().unwrap(), order.next().unwrap());
        state.nip_fi_deny_map = Some(Arc::new(map));
        let pod = Arc::new(state);
        let inner = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let (a, b) = (
            nostr::Keys::generate().public_key(),
            nostr::Keys::generate().public_key(),
        );
        store_deny_for(&inner, &failed, &a).await;
        store_deny_for(&inner, &healthy, &b).await;
        let source = Arc::new(FailingReplayGuard {
            inner,
            fail_issuer: Some(failed),
            ..Default::default()
        });
        let session = IssuerSessions::register(&pod, &healthy, &b);
        pod.nip_fi_deny_map.as_deref().unwrap().begin_restore();
        assert_eq!(
            restore_nip_fi_denies(&pod, &[source as _]).await,
            Err("store")
        );
        session.assert_closed("issuer after a store error");
        closed(&pod);

        // Unreadable: a bad stored record is reported apart from an outage.
        let pod = store_pod(10, Arc::clone(&store) as _).await;
        let sessions = keys
            .each_ref()
            .map(|k| IssuerSessions::register(&pod, TEST_ISS, k));
        pod.nip_fi_deny_map.as_deref().unwrap().begin_restore();
        let unreadable = Arc::new(FailingReplayGuard {
            load_unreadable: true.into(),
            ..Default::default()
        });
        assert_eq!(
            restore_nip_fi_denies(&pod, &[unreadable, Arc::clone(&store) as _]).await,
            Err("unreadable")
        );
        for session in &sessions {
            session.assert_closed("source after an unreadable record");
        }
        closed(&pod);
    }

    /// Live delivery still closes sessions when the entry is unchanged.
    #[tokio::test]
    async fn live_unchanged_delivery_closes_sessions() {
        let store = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let key = nostr::Keys::generate().public_key();
        let message = store_deny(&store, &key).await;
        let pod = store_pod(10, store).await;
        let now = chrono::Utc::now();
        pod.nip_fi_deny_map
            .as_deref()
            .unwrap()
            .merge_cross_pod_deny(TEST_ISS, &key, now + chrono::Duration::hours(1), || now);
        let sessions = IssuerSessions::register(&pod, TEST_ISS, &key);
        assert_eq!(
            apply_nip_fi_disconnect(&pod, &message, || now),
            NipFiDisconnectApplyResult::Applied(buzz_auth::CrossPodMergeResult::Unchanged)
        );
        sessions.assert_closed("unchanged live delivery");
    }

    /// A broadcast lag after a restore restores again: overflow the channel
    /// with retained messages while a newer store entry no longer fits, and
    /// the pod stays closed.
    #[tokio::test]
    async fn lag_after_restore_restores_again() {
        let store = Arc::new(buzz_auth::InMemoryCommandReplayGuard::default());
        let a = nostr::Keys::generate().public_key();
        let message = store_deny(&store, &a).await;
        let source = StaleFirstLoad::new(Arc::clone(&store), None);
        let (pod, tx, sub_tx, consumer) = capacity_one_consumer(source.clone()).await;
        go_live(&sub_tx);
        restored(&pod).await;
        store_deny(&store, &nostr::Keys::generate().public_key()).await;
        for _ in 0..17 {
            tx.send(message.clone()).unwrap();
        }
        source.loaded(2).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !pod.nip_fi_deny_map.as_deref().unwrap().is_restored(),
            "a lag restores again"
        );
        consumer.abort();
    }

    #[test]
    fn subscribe_channels_and_restore_sources_follow_mode_and_bridge() {
        use buzz_pubsub::conn_control::{
            NIP_FI_DISCONNECT_CHANNEL as E, NIP_FI_SHADOW_DISCONNECT_CHANNEL as S,
        };
        let pool = || buzz_pubsub::redis_connection("redis://127.0.0.1:6379").expect("lazy pool");
        for (mode, bridge, channels) in [
            (NipFiMode::Enforce, false, &[E][..]),
            (NipFiMode::Enforce, true, &[E, S][..]),
            (NipFiMode::Shadow, false, &[E, S][..]),
        ] {
            assert_eq!(
                disconnect_subscribe_channels(mode, bridge),
                channels,
                "{mode:?} {bridge}"
            );
            assert_eq!(
                restore_sources(pool(), mode, bridge).len(),
                channels.len(),
                "{mode:?} {bridge}"
            );
        }
    }

    /// Fails every claim.  With `applies`, the claim is applied to `inner`
    /// first and only the reply is lost.
    #[derive(Default)]
    struct FailingReplayGuard {
        inner: Arc<buzz_auth::InMemoryCommandReplayGuard>,
        applies: bool,
        load_fails: bool,
        /// Fails loads as an undecodable stored record, still returning
        /// `inner`'s records as the readable ones.
        load_unreadable: std::sync::atomic::AtomicBool,
        /// Fails loads for this issuer only.
        fail_issuer: Option<String>,
        loads: std::sync::atomic::AtomicUsize,
    }

    impl buzz_auth::CommandReplayGuard for FailingReplayGuard {
        fn try_claim<'a>(
            &'a self,
            claim: buzz_auth::CommandClaim<'a>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<buzz_auth::ClaimOutcome, buzz_auth::AuthError>,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                if self.applies {
                    self.inner.try_claim(claim).await?;
                }
                Err(buzz_auth::AuthError::Internal(
                    "simulated Redis outage".into(),
                ))
            })
        }

        fn load_denies<'a>(
            &'a self,
            issuer: &'a str,
            now: chrono::DateTime<chrono::Utc>,
        ) -> buzz_auth::LoadDenies<'a> {
            self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.load_fails || self.fail_issuer.as_deref() == Some(issuer) {
                return Box::pin(async {
                    Err(buzz_auth::AuthError::Internal("simulated Redis outage".into()).into())
                });
            }
            if self
                .load_unreadable
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Box::pin(async move {
                    Err(buzz_auth::LoadDeniesError::Unreadable {
                        record: "simulated bad record".into(),
                        readable: self.inner.load_denies(issuer, now).await?,
                    })
                });
            }
            self.inner.load_denies(issuer, now)
        }
    }

    async fn guard_error_fails_closed_without_effect(observe_publishes: bool) {
        let state = pod(1000, Arc::new(FailingReplayGuard::default())).await;
        let key = nostr::Keys::generate().public_key();
        let sessions = IssuerSessions::register(&state, TEST_ISS, &key);
        let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
        let mut publishes = observe(observe_publishes, &state, &key).await;

        let (status, body) = post_command(&state, &token, &key.to_hex()).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.as_ref(), b"authorization unavailable\n");
        assert!(!is_denied(&state, &key), "no deny entry on guard error");
        sessions.assert_open("guard error");
        // The failing guard accepts nothing, so the sentinel goes through a
        // healthy pod publishing to the same Redis channel.
        let healthy = pod(
            1000,
            Arc::new(buzz_auth::InMemoryCommandReplayGuard::default()),
        )
        .await;
        assert_publishes(
            &mut publishes,
            &[&state, &healthy],
            &healthy,
            0,
            "guard error",
        )
        .await;
    }

    #[tokio::test]
    async fn command_replay_guard_error_fails_closed_without_effect() {
        guard_error_fails_closed_without_effect(false).await;
    }

    async fn deny_set_full_leaves_command_retryable(
        replay: Arc<dyn buzz_auth::CommandReplayGuard>,
    ) {
        let state = pod(1, replay).await;
        let filler = target_hex();
        let target = target_hex();
        // `until` is a whole second, so an offset of 2 keeps the filler alive
        // for at least one second and frees the only slot before the retry.
        let filler_token = mint_token(&filler, 2, serde_json::json!({}));
        assert_eq!(
            post_command(&state, &filler_token, &filler).await.0,
            StatusCode::OK
        );

        let token = mint_token(&target, 300, serde_json::json!({}));
        let (status, body) = post_command(&state, &token, &target).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.as_ref(), b"deny set full\n");

        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        let (status, _) = post_command(&state, &token, &target).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the 503 must not burn the shared jti claim"
        );
    }

    #[tokio::test]
    async fn command_deny_set_full_leaves_shared_claim_retryable() {
        deny_set_full_leaves_command_retryable(Arc::new(
            buzz_auth::InMemoryCommandReplayGuard::default(),
        ))
        .await;
    }

    mod external_infra_redis {
        use super::*;

        /// A store on its own connection under `namespace`: pods built with
        /// the same namespace share one store; other cases never share its
        /// capacity.
        fn redis_guard_in(namespace: &str) -> Arc<dyn buzz_auth::CommandReplayGuard> {
            let url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let pool = buzz_pubsub::redis_connection(&url).expect("redis pool");
            Arc::new(buzz_pubsub::RedisCommandReplayGuard::namespaced(
                pool, namespace,
            ))
        }

        fn test_namespace() -> String {
            format!("buzz:test:nip-fi:{}", uuid::Uuid::new_v4())
        }

        /// A store private to one case.
        fn redis_guard() -> Arc<dyn buzz_auth::CommandReplayGuard> {
            redis_guard_in(&test_namespace())
        }

        async fn pod_in(mode: NipFiMode) -> Arc<AppState> {
            let mut config = crate::config::Config::for_test();
            config.nip_fi.mode = mode;
            config.nip_fi.registry.insert(test_issuer_policy());
            let mut state = build_test_app_state(1000, config).await;
            state.nip_fi_command_replay =
                command_replay_guard(state.redis_pool.clone(), state.config.nip_fi.mode);
            Arc::new(state)
        }

        /// Shadow startup goes through the same `AppState::new` as production:
        /// it builds the assertion verifier, installs a command verifier on
        /// the shared key source, and claims commands under the shadow replay
        /// prefix, leaving the enforce claim for an enforce pod.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn app_state_new_in_shadow_builds_verifiers_and_shadow_replay_guard() {
            use crate::nip_fi_config::NipFiRelayConfig;
            use buzz_auth::CommandReplayGuard as _;

            let mut config = crate::config::Config::for_test();
            config.redis_url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let mut registry = IssuerRegistry::new();
            registry.insert(test_issuer_policy());
            let command_configs = vec![(
                TEST_ISS.to_owned(),
                CommandIssuerEnvConfig {
                    maximum_command_age_seconds: Some(30),
                    authorized_principals: Some(vec![TEST_SUB.to_owned()]),
                    deny_set_capacity: Some(100),
                },
            )];
            config.nip_fi = NipFiRelayConfig {
                profile_policies: Vec::new(),
                accept_shadow_disconnects: false,
                source: Default::default(),
                mode: NipFiMode::Shadow,
                registry: registry.clone(),
                jwks_configs: vec![test_jwks_config()],
                command_configs: command_configs.clone(),
                max_connection_lifetime_secs: 3600,
                communities: crate::nip_fi_core::test_support::any_host(TEST_AUD),
            };
            let pool = sqlx::PgPool::connect_lazy(&config.database_url).unwrap();
            let db = buzz_db::Db::from_pool(pool.clone());
            let redis_pool = buzz_pubsub::redis_connection(&config.redis_url).unwrap();
            let pubsub = Arc::new(
                buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                    .await
                    .unwrap(),
            );
            let (mut state, _) = crate::state::AppState::new(
                config.clone(),
                db.clone(),
                redis_pool.clone(),
                buzz_audit::AuditService::new(pool.clone()),
                pubsub,
                buzz_auth::AuthService::new(config.auth.clone()),
                buzz_search::SearchService::new(pool),
                Arc::new(buzz_workflow::WorkflowEngine::new(
                    db,
                    buzz_workflow::WorkflowConfig::default(),
                )),
                nostr::Keys::generate(),
                buzz_media::MediaStorage::new(&config.media).unwrap(),
            );
            assert!(
                state.nip_fi_verifier.is_some(),
                "shadow must build the verifier"
            );
            let source = state.nip_fi_jwks_source.clone().expect("shared key source");
            super::super::install_nip_fi_command_components(
                &mut state.nip_fi_deny_map,
                &mut state.nip_fi_command_verifier,
                NipFiMode::Shadow,
                &registry,
                Arc::clone(&source),
                &command_configs,
            )
            .expect("shadow installs the command verifier");
            source.seed_snapshot_for_test(TEST_ISS, test_jwks()).await;
            let target = nostr::Keys::generate().public_key();
            let command = mint_token(&target.to_hex(), 300, serde_json::json!({}));
            let verifier = state.nip_fi_command_verifier.as_ref().unwrap();
            let verified =
                verifier.verify_at(&command, "POST", TEST_PATH, &target, chrono::Utc::now());
            assert!(verified.is_ok(), "shadow verifies commands: {verified:?}");

            let jti = uuid::Uuid::new_v4().to_string();
            let claim = buzz_auth::CommandClaim {
                issuer: TEST_ISS,
                jti: &jti,
                ttl_secs: 60,
                target: &target,
                until: chrono::Utc::now() + chrono::Duration::seconds(60),
                scope: buzz_auth::DenyScope::WholeKey,
                capacity: 100,
                now: chrono::Utc::now(),
            };
            use buzz_auth::ClaimOutcome::{Claimed, Replay};
            assert_eq!(
                state.nip_fi_command_replay.try_claim(claim).await.unwrap(),
                Claimed
            );
            let shadow = buzz_pubsub::RedisCommandReplayGuard::shadow(redis_pool.clone());
            let enforce = buzz_pubsub::RedisCommandReplayGuard::new(redis_pool);
            assert_eq!(
                shadow.try_claim(claim).await.unwrap(),
                Replay,
                "claim is shadow-prefixed"
            );
            assert_eq!(
                enforce.try_claim(claim).await.unwrap(),
                Claimed,
                "enforce claim untouched"
            );
        }

        fn claim<'a>(
            issuer: &'a str,
            jti: &'a str,
            target: &'a nostr::PublicKey,
            until: chrono::DateTime<chrono::Utc>,
            capacity: usize,
        ) -> buzz_auth::CommandClaim<'a> {
            buzz_auth::CommandClaim {
                issuer,
                jti,
                ttl_secs: 60,
                target,
                until,
                scope: buzz_auth::DenyScope::WholeKey,
                capacity,
                now: chrono::Utc::now(),
            }
        }

        /// An expired entry frees its slot, and a shorter later command never
        /// lowers the index score, so a later prune keeps the longer deny.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_store_prunes_expired_entries_and_never_lowers_a_score() {
            use buzz_auth::ClaimOutcome::Claimed;
            let store = redis_guard();
            let issuer = format!("https://{}.example", uuid::Uuid::new_v4());
            let keys: Vec<_> = (0..3)
                .map(|_| nostr::Keys::generate().public_key())
                .collect();
            let (a, b, c) = (&keys[0], &keys[1], &keys[2]);
            // Redis judges expiry on its own clock, so entries expire for real.
            let t = chrono::Utc::now();
            let s = chrono::Duration::seconds;
            let ms = chrono::Duration::milliseconds;
            let at = |jti: &'static str, target, until, capacity, now| buzz_auth::CommandClaim {
                issuer: &issuer,
                jti,
                ttl_secs: 60,
                target,
                until,
                scope: buzz_auth::DenyScope::WholeKey,
                capacity,
                now,
            };
            assert_eq!(
                store
                    .try_claim(at("p1", a, t + ms(300), 1, t))
                    .await
                    .unwrap(),
                Claimed
            );
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            assert_eq!(
                store
                    .try_claim(at("p2", b, t + s(120), 1, t + s(2)))
                    .await
                    .unwrap(),
                Claimed,
                "an expired entry frees its slot"
            );
            assert_eq!(
                store
                    .try_claim(at("p3", b, t + s(1), 2, t + s(2)))
                    .await
                    .unwrap(),
                Claimed
            );
            // A later accept prunes entries whose score is below its now.
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            assert_eq!(
                store
                    .try_claim(at("p4", c, t + s(90), 2, t + s(2)))
                    .await
                    .unwrap(),
                Claimed
            );
            let mut loaded = store.load_denies(&issuer, t + s(2)).await.unwrap();
            loaded.sort_by_key(|(k, _)| k.to_hex());
            let w = buzz_auth::DenyEntry::whole_key;
            let mut expected = vec![(*b, w(t + s(120))), (*c, w(t + s(90)))];
            expected.sort_by_key(|(k, _)| k.to_hex());
            assert_eq!(loaded, expected, "the longer deny for b survives the prune");
        }

        /// A deny record restore cannot read fails the load, so the restore
        /// fails and admission stays closed rather than dropping it.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_store_load_fails_on_unreadable_record() {
            let url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let mut conn = buzz_pubsub::redis_connection(&url).expect("redis pool");
            let key = || nostr::Keys::generate().public_key().to_hex();
            let exact = |t: chrono::DateTime<chrono::Utc>| {
                format!("{}:{}", t.timestamp(), t.timestamp_subsec_nanos())
            };
            let now = chrono::Utc::now();
            let live = (key(), exact(now + chrono::Duration::seconds(120)));
            let expired = (key(), exact(now - chrono::Duration::seconds(1)));
            let bytes = |(k, v): (String, String)| (k.into_bytes(), v.into_bytes());
            let cases = [
                ("valid and expired", None, true),
                (
                    "invalid key",
                    Some(bytes(("not-a-key".to_owned(), live.1.clone()))),
                    false,
                ),
                (
                    "invalid until",
                    Some(bytes((key(), "garbage".to_owned()))),
                    false,
                ),
                (
                    "until with extra field",
                    Some(bytes((key(), format!("{}:x", live.1)))),
                    false,
                ),
                (
                    "until not UTF-8",
                    Some((key().into_bytes(), vec![0xff, b':', b'0'])),
                    false,
                ),
                (
                    "key not UTF-8",
                    Some((vec![0xff; 64], live.1.clone().into_bytes())),
                    false,
                ),
            ];
            for (case, extra, ok) in cases {
                let namespace = test_namespace();
                let store = redis_guard_in(&namespace);
                let issuer = format!("https://{}.example", uuid::Uuid::new_v4());
                let (_, hash) = buzz_pubsub::nip_fi_command_replay::deny_store_keys(
                    &format!("{namespace}:deny"),
                    &issuer,
                );
                // Written first; HGETALL doesn't promise to return it first.
                let mut fields: Vec<_> = extra.into_iter().collect();
                fields.extend([bytes(live.clone()), bytes(expired.clone())]);
                // One transaction, so the fixture always expires.
                let _: () = redis::pipe()
                    .atomic()
                    .cmd("HSET")
                    .arg(&hash)
                    .arg(&fields)
                    .ignore()
                    .cmd("EXPIRE")
                    .arg(&hash)
                    .arg(60)
                    .ignore()
                    .query_async(&mut conn)
                    .await
                    .unwrap();
                let loaded = match store.load_denies(&issuer, now).await {
                    Ok(loaded) => {
                        assert!(ok, "{case}: an unreadable record must fail the load");
                        loaded
                    }
                    Err(buzz_auth::LoadDeniesError::Unreadable { readable, .. }) => {
                        assert!(!ok, "{case}: a readable hash must load");
                        readable
                    }
                    Err(e) => panic!("{case}: an unreadable record is not a store error: {e}"),
                };
                let keys: Vec<_> = loaded.into_iter().map(|(k, _)| k.to_hex()).collect();
                assert_eq!(
                    keys,
                    vec![live.0.clone()],
                    "{case}: every readable active record still loads"
                );
            }
        }

        /// One bad record does not hide the issuer's good ones: restore
        /// applies the good record and closes its sessions, leaves unrelated
        /// sessions open, and still fails as `unreadable`, so admission stays
        /// closed until the bad record is removed and a retry restores.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_restore_applies_readable_records_past_a_bad_one() {
            let url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let mut conn = buzz_pubsub::redis_connection(&url).expect("redis pool");
            let namespace = test_namespace();
            let store = redis_guard_in(&namespace);
            let (revoked, bystander) = (
                nostr::Keys::generate().public_key(),
                nostr::Keys::generate().public_key(),
            );
            let now = chrono::Utc::now();
            let outcome = store
                .try_claim(buzz_auth::CommandClaim {
                    issuer: TEST_ISS,
                    jti: "good",
                    ttl_secs: 60,
                    target: &revoked,
                    until: now + chrono::Duration::seconds(120),
                    scope: buzz_auth::DenyScope::WholeKey,
                    capacity: 10,
                    now,
                })
                .await
                .unwrap();
            assert_eq!(outcome, buzz_auth::ClaimOutcome::Claimed);
            let (_, hash) = buzz_pubsub::nip_fi_command_replay::deny_store_keys(
                &format!("{namespace}:deny"),
                TEST_ISS,
            );
            let _: () = redis::cmd("HSET")
                .arg(&hash)
                .arg("not-a-key")
                .arg("1:0")
                .query_async(&mut conn)
                .await
                .unwrap();

            let pod = store_pod(10, Arc::clone(&store)).await;
            let deny_map = pod.nip_fi_deny_map.as_deref().unwrap();
            let revoked_session = IssuerSessions::register(&pod, TEST_ISS, &revoked);
            let bystander_session = IssuerSessions::register(&pod, TEST_ISS, &bystander);
            let generation = deny_map.begin_restore();
            assert_eq!(
                restore_nip_fi_denies(&pod, &[Arc::clone(&store)]).await,
                Err("unreadable")
            );
            revoked_session.assert_closed("good record past a bad one");
            bystander_session.assert_open("unrelated key");
            assert_eq!(
                deny_map.deny_status(TEST_ISS, &bystander, chrono::Utc::now(), chrono::Utc::now()),
                buzz_auth::DenyStatus::Unavailable,
                "admission stays closed while a record is unreadable"
            );

            // The runbook's repair; the same generation's retry then reopens.
            let _: () = redis::cmd("HDEL")
                .arg(&hash)
                .arg("not-a-key")
                .query_async(&mut conn)
                .await
                .unwrap();
            assert_eq!(restore_nip_fi_denies(&pod, &[store]).await, Ok(()));
            assert!(deny_map.finish_restore(generation));
            assert!(!is_denied(&pod, &bystander), "reopened after the repair");
            assert!(is_denied(&pod, &revoked), "the good record still denies");
        }

        /// Past a bad record, restore applies a readable deny at its cutoff,
        /// not as the whole key, and admission stays fenced.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_partial_restore_keeps_a_readable_cutoff() {
            let url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let mut conn = buzz_pubsub::redis_connection(&url).expect("redis pool");
            let namespace = test_namespace();
            let store = redis_guard_in(&namespace);
            let (revoked, bystander) = (
                nostr::Keys::generate().public_key(),
                nostr::Keys::generate().public_key(),
            );
            let now = chrono::Utc::now();
            let c = now.timestamp();
            let outcome = store
                .try_claim(buzz_auth::CommandClaim {
                    issuer: TEST_ISS,
                    jti: "good",
                    ttl_secs: 60,
                    target: &revoked,
                    until: now + chrono::Duration::seconds(120),
                    scope: buzz_auth::DenyScope::IssuedThrough(c),
                    capacity: 10,
                    now,
                })
                .await
                .unwrap();
            assert_eq!(outcome, buzz_auth::ClaimOutcome::Claimed);
            let (_, hash) = buzz_pubsub::nip_fi_command_replay::deny_store_keys(
                &format!("{namespace}:deny"),
                TEST_ISS,
            );
            let _: () = redis::cmd("HSET")
                .arg(&hash)
                .arg("not-a-key")
                .arg("1:0")
                .query_async(&mut conn)
                .await
                .unwrap();

            let pod = store_pod(10, Arc::clone(&store)).await;
            let deny_map = pod.nip_fi_deny_map.as_deref().unwrap();
            let at = |secs| chrono::DateTime::from_timestamp(secs, 0).unwrap();
            let after = at(c) + chrono::Duration::seconds(1);
            let old = IssuerSessions::register_issued(&pod, TEST_ISS, &revoked, at(c));
            let fresh = IssuerSessions::register_issued(&pod, TEST_ISS, &revoked, after);
            deny_map.begin_restore();
            assert_eq!(
                restore_nip_fi_denies(&pod, &[store]).await,
                Err("unreadable")
            );
            assert!(
                !deny_map.is_restored(),
                "a partial load leaves it restoring"
            );
            old.assert_closed("issued at the cutoff");
            fresh.assert_open("the readable deny is scoped to its cutoff, not the whole key");
            assert_eq!(
                deny_map.deny_status(TEST_ISS, &bystander, after, chrono::Utc::now()),
                buzz_auth::DenyStatus::Unavailable,
                "admission stays fenced while a record is unreadable"
            );
        }

        /// The deny-store writer and reader before cutoffs, from #9860's
        /// `02627cce` (the script verbatim, the reader with its logging
        /// dropped and errors as text): the code a rolled-back or
        /// not-yet-upgraded pod runs.
        mod pre_cutoff {
            use chrono::{DateTime, Utc};
            use nostr::PublicKey;

            pub const ACCEPT_SCRIPT: &str = r"
if redis.call('EXISTS', KEYS[1]) == 1 then return 'replay' end
local active = ARGV[8] == '1'
if active then
  local expired = redis.call('ZRANGEBYSCORE', KEYS[2], '-inf', '(' .. ARGV[7])
  for _, member in ipairs(expired) do
    redis.call('HDEL', KEYS[3], member)
    redis.call('HDEL', KEYS[4], member)
  end
  redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', '(' .. ARGV[7])
  if not redis.call('ZSCORE', KEYS[2], ARGV[2])
     and redis.call('ZCARD', KEYS[2]) >= tonumber(ARGV[6]) then
    return 'full'
  end
end
redis.call('SET', KEYS[1], '1', 'EX', ARGV[1])
if active then
  redis.call('ZADD', KEYS[2], 'GT', ARGV[3], ARGV[2])
  local cur = redis.call('HGET', KEYS[3], ARGV[2])
  local keep = false
  if cur then
    local s, n = string.match(cur, '^(%-?%d+):(%d+)$')
    s, n = tonumber(s), tonumber(n)
    local ns, nn = tonumber(ARGV[4]), tonumber(ARGV[5])
    keep = s > ns or (s == ns and n >= nn)
  end
  if not keep then redis.call('HSET', KEYS[3], ARGV[2], ARGV[4] .. ':' .. ARGV[5]) end
  redis.call('HDEL', KEYS[4], ARGV[2])
  local top = redis.call('ZRANGE', KEYS[2], -1, -1, 'WITHSCORES')
  redis.call('PEXPIREAT', KEYS[2], top[2])
  redis.call('PEXPIREAT', KEYS[3], top[2])
  redis.call('PEXPIREAT', KEYS[4], top[2])
end
return 'claimed'
";

            /// Writes a whole-key deny as the old writer's `try_claim` did.
            pub async fn accept(
                conn: &mut buzz_pubsub::RedisConnection,
                prefix: &str,
                issuer: &str,
                jti: &str,
                target: &PublicKey,
                until: DateTime<Utc>,
                now: DateTime<Utc>,
            ) -> String {
                let (index, exact) =
                    buzz_pubsub::nip_fi_command_replay::deny_store_keys(prefix, issuer);
                let ceil_ms = until.timestamp_millis()
                    + i64::from(!until.timestamp_subsec_nanos().is_multiple_of(1_000_000));
                redis::Script::new(ACCEPT_SCRIPT)
                    .key(format!("{prefix}:claim:{issuer}:{jti}"))
                    .key(index)
                    .key(exact)
                    .key(buzz_pubsub::nip_fi_command_replay::revoke_through_key(
                        prefix, issuer,
                    ))
                    .arg(60)
                    .arg(target.to_hex())
                    .arg(ceil_ms)
                    .arg(until.timestamp())
                    .arg(until.timestamp_subsec_nanos())
                    .arg(10)
                    .arg(now.timestamp_millis())
                    .arg(u8::from(until > now))
                    .invoke_async(conn)
                    .await
                    .unwrap()
            }

            /// The old `load_denies` body: the exact-`until` hash only, applying
            /// every record past a bad one but still failing.
            pub async fn load_denies(
                conn: &mut buzz_pubsub::RedisConnection,
                exact: String,
                now: DateTime<Utc>,
            ) -> Result<Vec<(PublicKey, DateTime<Utc>)>, String> {
                let entries: Vec<(Vec<u8>, Vec<u8>)> = redis::cmd("HGETALL")
                    .arg(exact)
                    .query_async(conn)
                    .await
                    .map_err(|e| e.to_string())?;
                let mut denies = Vec::with_capacity(entries.len());
                let mut bad = None;
                for (key, until) in entries {
                    match decode(key, until) {
                        Ok((key, until)) if until > now => denies.push((key, until)),
                        Ok(_) => {}
                        Err(record) => {
                            bad.get_or_insert(record);
                        }
                    }
                }
                match bad {
                    None => Ok(denies),
                    Some(record) => Err(record),
                }
            }

            fn decode(key: Vec<u8>, until: Vec<u8>) -> Result<(PublicKey, DateTime<Utc>), String> {
                let (Ok(key), Ok(until)) = (String::from_utf8(key), String::from_utf8(until))
                else {
                    return Err("deny record that is not UTF-8".into());
                };
                let key = PublicKey::from_hex(&key).map_err(|_| format!("deny key {key}"))?;
                let until =
                    parse_until(&until).ok_or_else(|| format!("deny until {until} for {key}"))?;
                Ok((key, until))
            }

            fn parse_until(value: &str) -> Option<DateTime<Utc>> {
                let (secs, nanos) = value.split_once(':')?;
                DateTime::from_timestamp(secs.parse().ok()?, nanos.parse().ok()?)
            }
        }

        /// A cutoff a command stores in Redis survives a restart as a cutoff:
        /// a restored pod refuses an assertion issued at C and admits one
        /// issued after it, while the exact-`until` hash alone (the whole of
        /// an older relay's reader) still decodes and reads it as a whole-key
        /// deny.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_cutoff_survives_restore_and_old_reader_sees_whole_key() {
            let url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let mut conn = buzz_pubsub::redis_connection(&url).expect("redis pool");
            let namespace = test_namespace();
            let store = redis_guard_in(&namespace);
            let key = nostr::Keys::generate().public_key();
            let now = chrono::Utc::now();
            let until = now + chrono::Duration::seconds(120);
            let c = now.timestamp();
            let outcome = store
                .try_claim(buzz_auth::CommandClaim {
                    issuer: TEST_ISS,
                    jti: "cutoff",
                    ttl_secs: 60,
                    target: &key,
                    until,
                    scope: buzz_auth::DenyScope::IssuedThrough(c),
                    capacity: 10,
                    now,
                })
                .await
                .unwrap();
            assert_eq!(outcome, buzz_auth::ClaimOutcome::Claimed);

            let pod = store_pod(10, Arc::clone(&store)).await;
            let deny_map = pod.nip_fi_deny_map.as_deref().unwrap();
            let generation = deny_map.begin_restore();
            assert_eq!(restore_nip_fi_denies(&pod, &[store]).await, Ok(()));
            assert!(deny_map.finish_restore(generation));
            let at = |secs| chrono::DateTime::from_timestamp(secs, 0).unwrap();
            let fraction = at(c) + chrono::Duration::milliseconds(500);
            assert!(deny_map.denies(TEST_ISS, &key, at(c), chrono::Utc::now()));
            assert!(
                !deny_map.denies(TEST_ISS, &key, fraction, chrono::Utc::now()),
                "restored as a cutoff, not a whole-key deny"
            );

            let (_, exact) = buzz_pubsub::nip_fi_command_replay::deny_store_keys(
                &format!("{namespace}:deny"),
                TEST_ISS,
            );
            let old = pre_cutoff::load_denies(&mut conn, exact, now)
                .await
                .expect("the old reader decodes new data");
            assert_eq!(old, vec![(key, until)], "read as a whole-key deny");
        }

        /// The Redis merge matches [`buzz_auth::DenyEntry::merge`] in both
        /// write orders: a whole key beats a cutoff, two cutoffs keep the
        /// later C even under a shorter `until`, and an expired entry
        /// contributes no scope.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_scope_merge_matches_the_deny_map() {
            use buzz_auth::{DenyEntry, DenyScope};
            let s = chrono::Duration::seconds;
            let cut = DenyScope::IssuedThrough;
            let whole = DenyScope::WholeKey;
            // (first, second, milliseconds between the writes)
            let cases = [
                ((s(60), cut(10)), (s(60), whole), 0, "whole over cutoff"),
                ((s(60), whole), (s(60), cut(10)), 0, "cutoff over whole"),
                (
                    (s(60), cut(10)),
                    (s(30), cut(20)),
                    0,
                    "shorter later cutoff",
                ),
                (
                    (s(60), cut(20)),
                    (s(90), cut(10)),
                    0,
                    "longer earlier cutoff",
                ),
                (
                    (chrono::Duration::milliseconds(300), whole),
                    (s(60), cut(10)),
                    400,
                    "expired whole key",
                ),
            ];
            for ((d1, s1), (d2, s2), wait, case) in cases {
                let store = redis_guard();
                let key = nostr::Keys::generate().public_key();
                let t = chrono::Utc::now();
                let claim = |jti, until, scope| buzz_auth::CommandClaim {
                    issuer: TEST_ISS,
                    jti,
                    ttl_secs: 60,
                    target: &key,
                    until,
                    scope,
                    capacity: 10,
                    now: t,
                };
                let first = DenyEntry {
                    until: t + d1,
                    scope: s1,
                };
                let second = DenyEntry {
                    until: t + d2,
                    scope: s2,
                };
                store.try_claim(claim("a", first.until, s1)).await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                store.try_claim(claim("b", second.until, s2)).await.unwrap();
                let now = chrono::Utc::now();
                let expected = DenyEntry::merge(Some(first), second, now);
                let loaded = store.load_denies(TEST_ISS, now).await.unwrap();
                assert_eq!(loaded, vec![(key, expected)], "{case}");
            }
        }

        /// A pod that predates cutoffs writes over a cutoff with an equal or
        /// shorter `until`, and a fresh upgraded pod restores what it wrote as a whole-key deny.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_pre_cutoff_writer_then_fresh_restore_is_whole_key() {
            let url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let mut conn = buzz_pubsub::redis_connection(&url).expect("redis pool");
            let namespace = test_namespace();
            let prefix = format!("{namespace}:deny");
            let store = redis_guard_in(&namespace);
            let [equal, shorter, fresh] =
                std::array::from_fn(|_| nostr::Keys::generate().public_key());
            let now = chrono::Utc::now();
            let until = now + chrono::Duration::seconds(120);
            let c = now.timestamp();
            for (jti, target) in [("cutoff-a", &equal), ("cutoff-b", &shorter)] {
                let outcome = store
                    .try_claim(buzz_auth::CommandClaim {
                        issuer: TEST_ISS,
                        jti,
                        ttl_secs: 60,
                        target,
                        until,
                        scope: buzz_auth::DenyScope::IssuedThrough(c),
                        capacity: 10,
                        now,
                    })
                    .await
                    .unwrap();
                assert_eq!(outcome, buzz_auth::ClaimOutcome::Claimed);
            }
            let short = until - chrono::Duration::seconds(60);
            for (jti, target, old_until) in [
                ("old-a", &equal, until),
                ("old-b", &shorter, short),
                ("old-c", &fresh, until),
            ] {
                let reply =
                    pre_cutoff::accept(&mut conn, &prefix, TEST_ISS, jti, target, old_until, now)
                        .await;
                assert_eq!(reply, "claimed");
            }

            let pod = store_pod(10, Arc::clone(&store)).await;
            let deny_map = pod.nip_fi_deny_map.as_deref().unwrap();
            let generation = deny_map.begin_restore();
            assert_eq!(restore_nip_fi_denies(&pod, &[store]).await, Ok(()));
            assert!(deny_map.finish_restore(generation));
            let after =
                chrono::DateTime::from_timestamp(c, 0).unwrap() + chrono::Duration::seconds(1);
            for key in [equal, shorter, fresh] {
                assert!(
                    deny_map.denies(TEST_ISS, &key, after, chrono::Utc::now()),
                    "the old writer's full revoke, equal or shorter, restores as the whole key"
                );
            }
        }

        /// Expiry is judged by Redis as the script runs, not by the clock the
        /// command was authenticated under: a full revoke whose `until` passes
        /// before its write changes nothing, and a stored full revoke that
        /// expired meanwhile no longer overrides a cutoff.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_accept_judges_expiry_at_the_write() {
            use buzz_auth::{DenyEntry, DenyScope};
            let store = redis_guard();
            let (key, other) = (
                nostr::Keys::generate().public_key(),
                nostr::Keys::generate().public_key(),
            );
            let authenticated = chrono::Utc::now();
            let ms = chrono::Duration::milliseconds;
            let claim = |jti, target, until, scope| buzz_auth::CommandClaim {
                issuer: TEST_ISS,
                jti,
                ttl_secs: 60,
                target,
                until,
                scope,
                capacity: 10,
                now: authenticated,
            };
            let until = authenticated + chrono::Duration::seconds(60);
            let cut = DenyScope::IssuedThrough(10);
            let lapsing = authenticated + ms(300);
            store
                .try_claim(claim("cutoff", &key, until, cut))
                .await
                .unwrap();
            store
                .try_claim(claim("lapsing", &other, lapsing, DenyScope::WholeKey))
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            store
                .try_claim(claim("lapsed", &key, lapsing, DenyScope::WholeKey))
                .await
                .unwrap();
            store
                .try_claim(claim("over", &other, until, cut))
                .await
                .unwrap();
            let mut loaded = store
                .load_denies(TEST_ISS, chrono::Utc::now())
                .await
                .unwrap();
            loaded.sort_by_key(|(k, _)| k.to_hex());
            let mut expected = vec![
                (key, DenyEntry { until, scope: cut }),
                (other, DenyEntry { until, scope: cut }),
            ];
            expected.sort_by_key(|(k, _)| k.to_hex());
            assert_eq!(
                loaded, expected,
                "an expired full revoke neither widens nor overrides a cutoff"
            );
        }

        /// A cutoff that doesn't decode, or names no stored deny, fails the
        /// load as unreadable.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_store_load_fails_on_unreadable_cutoff() {
            let url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let mut conn = buzz_pubsub::redis_connection(&url).expect("redis pool");
            let key = nostr::Keys::generate().public_key();
            let now = chrono::Utc::now();
            let until = now + chrono::Duration::seconds(120);
            for (case, field, cutoff) in [
                ("not a number", key.to_hex(), "soon".to_owned()),
                ("out of range", key.to_hex(), i64::MAX.to_string()),
                (
                    "no deny",
                    nostr::Keys::generate().public_key().to_hex(),
                    "10".to_owned(),
                ),
            ] {
                let namespace = test_namespace();
                let store = redis_guard_in(&namespace);
                let prefix = format!("{namespace}:deny");
                let (_, exact) =
                    buzz_pubsub::nip_fi_command_replay::deny_store_keys(&prefix, TEST_ISS);
                let cutoffs =
                    buzz_pubsub::nip_fi_command_replay::revoke_through_key(&prefix, TEST_ISS);
                let _: () = redis::pipe()
                    .atomic()
                    .cmd("HSET")
                    .arg(&exact)
                    .arg(key.to_hex())
                    .arg(format!(
                        "{}:{}",
                        until.timestamp(),
                        until.timestamp_subsec_nanos()
                    ))
                    .ignore()
                    .cmd("HSET")
                    .arg(&cutoffs)
                    .arg(field)
                    .arg(cutoff)
                    .ignore()
                    .cmd("EXPIRE")
                    .arg(&exact)
                    .arg(60)
                    .ignore()
                    .cmd("EXPIRE")
                    .arg(&cutoffs)
                    .arg(60)
                    .ignore()
                    .query_async(&mut conn)
                    .await
                    .unwrap();
                assert!(
                    matches!(
                        store.load_denies(TEST_ISS, now).await,
                        Err(buzz_auth::LoadDeniesError::Unreadable { .. })
                    ),
                    "{case}"
                );
            }
        }

        /// A full revoke clears any cutoff stored for its target, whether the
        /// stored `until` grows, stays equal or is kept over a shorter one, and
        /// pruning an expired target clears its cutoff too.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_store_accept_and_prune_clear_revoke_through() {
            let url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let conn = buzz_pubsub::redis_connection(&url).expect("redis pool");
            let namespace = test_namespace();
            let store = redis_guard_in(&namespace);
            let issuer = format!("https://{}.example", uuid::Uuid::new_v4());
            let cutoffs = buzz_pubsub::nip_fi_command_replay::revoke_through_key(
                &format!("{namespace}:deny"),
                &issuer,
            );
            let (a, b) = (
                nostr::Keys::generate().public_key(),
                nostr::Keys::generate().public_key(),
            );
            let t = chrono::Utc::now();
            let s = chrono::Duration::seconds;
            let claim = |jti, target, until, now| buzz_auth::CommandClaim {
                issuer: &issuer,
                jti,
                ttl_secs: 60,
                target,
                until,
                scope: buzz_auth::DenyScope::WholeKey,
                capacity: 10,
                now,
            };
            let seed = |key: &nostr::PublicKey| {
                let mut conn = conn.clone();
                let (cutoffs, field) = (cutoffs.clone(), key.to_hex());
                async move {
                    let _: () = redis::pipe()
                        .atomic()
                        .cmd("HSET")
                        .arg(&cutoffs)
                        .arg(field)
                        .arg(1)
                        .ignore()
                        .cmd("EXPIRE")
                        .arg(&cutoffs)
                        .arg(60)
                        .ignore()
                        .query_async(&mut conn)
                        .await
                        .unwrap();
                }
            };
            let held = |key: &nostr::PublicKey| {
                let mut conn = conn.clone();
                let (cutoffs, field) = (cutoffs.clone(), key.to_hex());
                async move {
                    let held: bool = redis::cmd("HEXISTS")
                        .arg(cutoffs)
                        .arg(field)
                        .query_async(&mut conn)
                        .await
                        .unwrap();
                    held
                }
            };
            store
                .try_claim(claim("c0", &a, t + s(60), t))
                .await
                .unwrap();
            for (jti, until, case) in [("c1", t + s(60), "equal"), ("c2", t + s(30), "shorter")] {
                seed(&a).await;
                store.try_claim(claim(jti, &a, until, t)).await.unwrap();
                assert!(!held(&a).await, "{case} until clears the cutoff");
            }
            // b's entry already expired when the next accept runs, so it is pruned.
            let (index, exact) = buzz_pubsub::nip_fi_command_replay::deny_store_keys(
                &format!("{namespace}:deny"),
                &issuer,
            );
            let mut seed_conn = conn.clone();
            let _: () = redis::pipe()
                .atomic()
                .cmd("ZADD")
                .arg(&index)
                .arg(1)
                .arg(b.to_hex())
                .ignore()
                .cmd("HSET")
                .arg(&exact)
                .arg(b.to_hex())
                .arg("0:1000000")
                .ignore()
                .cmd("HSET")
                .arg(&cutoffs)
                .arg(b.to_hex())
                .arg(1)
                .ignore()
                .query_async(&mut seed_conn)
                .await
                .unwrap();
            store
                .try_claim(claim("c4", &a, t + s(120), t + s(10)))
                .await
                .unwrap();
            assert!(!held(&b).await, "pruning clears the cutoff");
        }

        /// A disconnect over an active deny whose stored cutoff restore would
        /// reject is refused with nothing written: the claim stays unused and the deny state is
        /// unchanged.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_accept_refuses_an_unreadable_stored_cutoff() {
            /// Index members with scores, then the exact and cutoff hashes.
            type DenyStoreSnapshot = (
                Vec<(String, f64)>,
                Vec<(String, String)>,
                Vec<(String, String)>,
            );
            let url = std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
            let mut conn = buzz_pubsub::redis_connection(&url).expect("redis pool");
            let namespace = test_namespace();
            let prefix = format!("{namespace}:deny");
            let store = redis_guard_in(&namespace);
            let issuer = format!("https://{}.example", uuid::Uuid::new_v4());
            let (index, exact) =
                buzz_pubsub::nip_fi_command_replay::deny_store_keys(&prefix, &issuer);
            let cutoffs = buzz_pubsub::nip_fi_command_replay::revoke_through_key(&prefix, &issuer);
            let now = chrono::Utc::now();
            for (case, bad) in [
                ("fractional", "12.5"),
                ("out of range", "9223372036854775807"),
            ] {
                let key = nostr::Keys::generate().public_key();
                let base = buzz_auth::CommandClaim {
                    issuer: &issuer,
                    jti: "base",
                    ttl_secs: 60,
                    target: &key,
                    until: now + chrono::Duration::seconds(120),
                    scope: buzz_auth::DenyScope::IssuedThrough(now.timestamp()),
                    capacity: 10,
                    now,
                };
                let base_jti = format!("base-{key}");
                let outcome = store
                    .try_claim(buzz_auth::CommandClaim {
                        jti: &base_jti,
                        ..base
                    })
                    .await
                    .unwrap();
                assert_eq!(outcome, buzz_auth::ClaimOutcome::Claimed);
                let _: () = redis::cmd("HSET")
                    .arg(&cutoffs)
                    .arg(key.to_hex())
                    .arg(bad)
                    .query_async(&mut conn)
                    .await
                    .unwrap();
                let snapshot = |conn: &mut buzz_pubsub::RedisConnection| {
                    let mut conn = conn.clone();
                    let (index, exact, cutoffs) = (index.clone(), exact.clone(), cutoffs.clone());
                    async move {
                        let state: DenyStoreSnapshot = redis::pipe()
                            .cmd("ZRANGE")
                            .arg(&index)
                            .arg(0)
                            .arg(-1)
                            .arg("WITHSCORES")
                            .cmd("HGETALL")
                            .arg(&exact)
                            .cmd("HGETALL")
                            .arg(&cutoffs)
                            .query_async(&mut conn)
                            .await
                            .unwrap();
                        state
                    }
                };
                let before = snapshot(&mut conn).await;
                let jti = format!("bad-cutoff-{key}");
                let claim = buzz_auth::CommandClaim {
                    issuer: &issuer,
                    jti: &jti,
                    ttl_secs: 60,
                    target: &key,
                    until: now + chrono::Duration::seconds(60),
                    scope: buzz_auth::DenyScope::IssuedThrough(now.timestamp()),
                    capacity: 10,
                    now,
                };
                assert!(
                    store.try_claim(claim).await.is_err(),
                    "{case} cutoff is refused"
                );
                assert_eq!(
                    snapshot(&mut conn).await,
                    before,
                    "{case} leaves the deny state"
                );
                let _: () = redis::cmd("HDEL")
                    .arg(&cutoffs)
                    .arg(key.to_hex())
                    .query_async(&mut conn)
                    .await
                    .unwrap();
                assert_eq!(
                    store.try_claim(claim).await.unwrap(),
                    buzz_auth::ClaimOutcome::Claimed,
                    "{case} left the claim unused"
                );
            }
        }

        /// The Redis script checks replay and shared capacity before writing,
        /// keeps the exact maximum `until`, and records no entry for a past
        /// `until`.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_store_checks_before_writing_and_keeps_exact_until() {
            use buzz_auth::ClaimOutcome::{Claimed, Full, Replay};
            let store = redis_guard();
            let issuer = format!("https://{}.example", uuid::Uuid::new_v4());
            let (a, b) = (
                nostr::Keys::generate().public_key(),
                nostr::Keys::generate().public_key(),
            );
            let base = chrono::Utc::now() + chrono::Duration::seconds(120);
            // Two deadlines inside the same millisecond: the later must win.
            let later = base + chrono::Duration::nanoseconds(900);
            let earlier = base + chrono::Duration::nanoseconds(100);

            assert_eq!(
                store
                    .try_claim(claim(&issuer, "j1", &a, later, 1))
                    .await
                    .unwrap(),
                Claimed
            );
            assert_eq!(
                store
                    .try_claim(claim(&issuer, "j1", &a, later, 1))
                    .await
                    .unwrap(),
                Replay
            );
            assert_eq!(
                store
                    .try_claim(claim(&issuer, "j2", &a, earlier, 1))
                    .await
                    .unwrap(),
                Claimed,
                "an existing key needs no new slot"
            );
            assert_eq!(
                store
                    .try_claim(claim(&issuer, "j3", &b, later, 1))
                    .await
                    .unwrap(),
                Full
            );
            // `Full` wrote no claim: the same jti succeeds once there is room.
            assert_eq!(
                store
                    .try_claim(claim(&issuer, "j3", &b, later, 2))
                    .await
                    .unwrap(),
                Claimed
            );
            let past = chrono::Utc::now() - chrono::Duration::seconds(1);
            let c = nostr::Keys::generate().public_key();
            assert_eq!(
                store
                    .try_claim(claim(&issuer, "j4", &c, past, 2))
                    .await
                    .unwrap(),
                Claimed
            );
            assert_eq!(
                store
                    .try_claim(claim(&issuer, "j4", &c, past, 2))
                    .await
                    .unwrap(),
                Replay
            );

            let mut loaded = store
                .load_denies(&issuer, chrono::Utc::now())
                .await
                .unwrap();
            loaded.sort_by_key(|(k, _)| k.to_hex());
            let w = buzz_auth::DenyEntry::whole_key;
            let mut expected = vec![(a, w(later)), (b, w(later))];
            expected.sort_by_key(|(k, _)| k.to_hex());
            assert_eq!(
                loaded, expected,
                "exact maximum until; no entry for a past until"
            );
        }

        /// The bridge: an enforce pod restores denies accepted by shadow pods
        /// only with `BUZZ_NIP_FI_ACCEPT_SHADOW_DISCONNECTS`; without it the
        /// shadow store is never read.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn bridge_restores_shadow_period_denies_on_enforce_pods() {
            let shadow = pod_in(NipFiMode::Shadow).await;
            let key = nostr::Keys::generate().public_key();
            let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
            assert_eq!(
                post_command(&shadow, &token, &key.to_hex()).await.0,
                StatusCode::OK
            );

            for (bridge, denied) in [(false, false), (true, true)] {
                let enforce = pod_in(NipFiMode::Enforce).await;
                let sources =
                    restore_sources(enforce.redis_pool.clone(), NipFiMode::Enforce, bridge);
                restore_nip_fi_denies(&enforce, &sources)
                    .await
                    .expect("restore");
                assert_eq!(is_denied(&enforce, &key), denied, "bridge={bridge}");
            }
        }

        /// Run `state`'s production disconnect subscriber and apply what it
        /// hears through the production consumer, like `main.rs`.  Yields
        /// the target of each applied message.
        pub(super) fn listen(
            state: &Arc<AppState>,
        ) -> tokio::sync::mpsc::UnboundedReceiver<Vec<u8>> {
            let mut rx = state.pubsub.subscribe_nip_fi_disconnect();
            let channels = disconnect_subscribe_channels(state.config.nip_fi.mode, false);
            tokio::spawn(Arc::clone(&state.pubsub).run_nip_fi_disconnect_subscriber(channels));
            let consumer = Arc::clone(state);
            let (seen_tx, seen) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(async move {
                while let Ok(msg) = rx.recv().await {
                    apply_nip_fi_disconnect(&consumer, &msg, chrono::Utc::now);
                    let _ = seen_tx.send(msg.pubkey_bytes);
                }
            });
            seen
        }

        /// Return once a probe that `prober` publishes has been applied via
        /// `seen`: the subscription is live, and anything published earlier
        /// on the probe's channel has been applied.
        pub(super) async fn await_probe(
            seen: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
            prober: &Arc<AppState>,
        ) {
            for _ in 0..50 {
                let probe = nostr::Keys::generate().public_key();
                let token = mint_token(&probe.to_hex(), 300, serde_json::json!({}));
                post_command(prober, &token, &probe.to_hex()).await;
                drain_publishes(prober).await;
                let wait = std::time::Duration::from_millis(200);
                while let Ok(Some(k)) = tokio::time::timeout(wait, seen.recv()).await {
                    if k == probe.to_bytes().to_vec() {
                        return;
                    }
                }
            }
            panic!("disconnect subscriber never received a probe");
        }

        /// Return once `seen` has applied a message for `key`.
        pub(super) async fn await_target(
            seen: &mut tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
            key: &nostr::PublicKey,
        ) {
            let wait = std::time::Duration::from_secs(5);
            while let Ok(Some(k)) = tokio::time::timeout(wait, seen.recv()).await {
                if k == key.to_bytes().to_vec() {
                    return;
                }
            }
            panic!("disconnect subscriber never received the target");
        }

        // Pins finding 1 (disconnect half): a shadow disconnect goes out only
        // on the shadow channel, so an enforce pod denies and closes nothing.
        // Other shadow pods receive it and record the deny; no shadow pod
        // closes a session, whether the command came from shadow or enforce.
        // Mutation: publishing shadow commands on `NIP_FI_DISCONNECT_CHANNEL`
        // closes the enforce session; letting a shadow pod close on a
        // received command fails `assert_open` on the shadow pods.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn shadow_disconnect_never_reaches_an_enforce_pod() {
            let shadow = pod_in(NipFiMode::Shadow).await;
            let peer = pod_in(NipFiMode::Shadow).await;
            let enforce = pod_in(NipFiMode::Enforce).await;
            let mut enforce_seen = listen(&enforce);
            let mut shadow_seen = listen(&shadow);
            let mut peer_seen = listen(&peer);
            await_probe(&mut enforce_seen, &enforce).await;
            await_probe(&mut shadow_seen, &enforce).await;
            await_probe(&mut peer_seen, &enforce).await;
            let key = nostr::Keys::generate().public_key();
            let on_shadow = IssuerSessions::register(&shadow, TEST_ISS, &key);
            let on_peer = IssuerSessions::register(&peer, TEST_ISS, &key);
            let on_enforce = IssuerSessions::register(&enforce, TEST_ISS, &key);

            let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));
            let (status, body) = post_command(&shadow, &token, &key.to_hex()).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(&body[..], br#"{"disconnected": true}"#);
            assert!(is_denied(&shadow, &key), "shadow records its deny");
            on_shadow.assert_open("shadow pod");
            drain_publishes(&shadow).await;
            // Anything the enforce pod would hear from that publish has
            // arrived once a later enforce-channel probe has.
            await_probe(&mut enforce_seen, &enforce).await;
            assert!(!is_denied(&enforce, &key), "enforce records no shadow deny");
            on_enforce.assert_open("enforce pod");
            await_target(&mut peer_seen, &key).await;
            assert!(
                is_denied(&peer, &key),
                "a second shadow pod records the deny"
            );
            on_peer.assert_open("second shadow pod");

            // An enforce disconnect reaching a shadow pod records the deny
            // and closes nothing there.
            let target = nostr::Keys::generate().public_key();
            let target_on_shadow = IssuerSessions::register(&shadow, TEST_ISS, &target);
            let token = mint_token(&target.to_hex(), 300, serde_json::json!({}));
            let (status, _) = post_command(&enforce, &token, &target.to_hex()).await;
            assert_eq!(status, StatusCode::OK);
            drain_publishes(&enforce).await;
            await_target(&mut shadow_seen, &target).await;
            assert!(is_denied(&shadow, &target), "shadow records enforce's deny");
            target_on_shadow.assert_open("shadow pod, enforce command");
        }

        // Pins finding 1 (replay half): shadow and enforce share one Redis
        // but claim under disjoint prefixes, so a shadow accept leaves the
        // command usable by enforce, while shadow still rejects its own
        // replay.  Mutation: a shadow guard on the enforce prefix makes the
        // enforce use a 403.
        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn shadow_command_claim_never_uses_up_enforce_claim() {
            let shadow = pod_in(NipFiMode::Shadow).await;
            let enforce = pod_in(NipFiMode::Enforce).await;
            let key = nostr::Keys::generate().public_key();
            let token = mint_token(&key.to_hex(), 300, serde_json::json!({}));

            let (status, _) = post_command(&shadow, &token, &key.to_hex()).await;
            assert_eq!(status, StatusCode::OK, "shadow accepts first use");
            let (status, _) = post_command(&enforce, &token, &key.to_hex()).await;
            assert_eq!(status, StatusCode::OK, "enforce still accepts it");
            let (status, body) = post_command(&shadow, &token, &key.to_hex()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "shadow rejects its replay");
            assert_eq!(body.as_ref(), b"authorization denied\n");
        }

        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_command_replay_on_second_pod_is_denied() {
            let namespace = test_namespace();
            command_replayed_on_second_pod_is_denied_without_effect(
                redis_guard_in(&namespace),
                redis_guard_in(&namespace),
                true,
            )
            .await;
        }

        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_forged_command_does_not_consume_shared_claim() {
            forged_command_leaves_shared_claim_unconsumed(redis_guard(), None, true).await;
        }

        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_guard_error_publishes_nothing() {
            guard_error_fails_closed_without_effect(true).await;
        }

        #[tokio::test]
        #[ignore = "requires Redis"]
        async fn redis_deny_set_full_leaves_shared_claim_retryable() {
            deny_set_full_leaves_command_retryable(redis_guard()).await;
        }
    }

    /// Plan criterion 10 over real infrastructure: PostgreSQL holds the two
    /// communities, Redis carries the disconnect between pods, and every
    /// admission runs the production path end to end.
    mod postgres_tests {
        use super::external_infra_redis::{await_probe, await_target, listen};
        use super::*;
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, Message};

        const DENY_BODY: &[u8] = b"authorization denied\n";

        fn redis_url() -> String {
            std::env::var("BUZZ_TEST_REDIS_URL")
                .or_else(|_| std::env::var("REDIS_URL"))
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".into())
        }

        /// An enforce pod serving `hosts` as two exact communities over the
        /// shared PostgreSQL and Redis, listening on a real socket and
        /// running the production disconnect subscriber.
        struct Pod {
            state: Arc<AppState>,
            addr: std::net::SocketAddr,
            seen: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
            server: tokio::task::JoinHandle<()>,
        }

        impl Drop for Pod {
            fn drop(&mut self) {
                self.server.abort();
            }
        }

        async fn pod(hosts: &[String; 2]) -> Pod {
            let mut config = crate::config::Config::for_test();
            config.database_url = crate::test_support::database_url();
            config.redis_url = redis_url();
            config.require_relay_membership = false;
            config.nip_fi.mode = NipFiMode::Enforce;
            config.nip_fi.registry.insert(test_issuer_policy());
            let uris = hosts.clone().map(|host| format!("https://{host}"));
            config.nip_fi.communities = crate::nip_fi_config::NipFiCommunities::for_test_many(
                &[&uris[0], &uris[1]],
                &[TEST_ISS],
            );
            let mut state = build_test_app_state(1000, config).await;
            state.nip_fi_command_replay =
                command_replay_guard(state.redis_pool.clone(), state.config.nip_fi.mode);
            let key_source = Arc::new(
                ProductionJwksSource::new(
                    vec![test_jwks_config()],
                    buzz_auth::HttpJwksFetcher::new(),
                )
                .expect("key source"),
            );
            key_source
                .seed_snapshot_for_test(TEST_ISS, test_jwks())
                .await;
            state.nip_fi_verifier = Some(Arc::new(buzz_auth::FederatedAssertionVerifier::new(
                state.config.nip_fi.registry.clone(),
                key_source,
            )));
            let state = Arc::new(state);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let addr = listener.local_addr().expect("addr");
            let app = crate::router::build_router(Arc::clone(&state))
                .into_make_service_with_connect_info::<std::net::SocketAddr>();
            let server = tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            let seen = listen(&state);
            Pod {
                state,
                addr,
                seen,
                server,
            }
        }

        /// Both pods, with each subscription confirmed live.
        async fn pair(hosts: &[String; 2]) -> [Pod; 2] {
            let mut pods = [pod(hosts).await, pod(hosts).await];
            for p in &mut pods {
                await_probe(&mut p.seen, &p.state).await;
            }
            pods
        }

        /// A login assertion for `key` whose audience is `host`'s community.
        /// Minted once and replayed as the same bytes in every phase.
        fn retained_assertion(key: &nostr::Keys, host: &str) -> String {
            use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
            let now = chrono::Utc::now().timestamp();
            let claims = serde_json::json!({
                "iss": TEST_ISS, "aud": format!("https://{host}"), "sub": "restart-test",
                "iat": now, "exp": now + 600, "nostr_pubkey": key.public_key().to_hex(),
            });
            let mut header = Header::new(Algorithm::ES256);
            header.typ = Some("nip-fi+jwt".to_owned());
            header.kid = Some(TEST_KID.to_owned());
            let signing = EncodingKey::from_ec_pem(TEST_PRIVATE_KEY_PEM.as_bytes()).unwrap();
            encode(&header, &claims, &signing).unwrap()
        }

        /// A key and its retained assertion for each community.
        struct Retained {
            keys: nostr::Keys,
            assertions: [(String, String); 2],
        }

        impl Retained {
            fn mint(hosts: &[String; 2]) -> Self {
                let keys = nostr::Keys::generate();
                let assertions = hosts.clone().map(|host| {
                    let token = retained_assertion(&keys, &host);
                    (host, token)
                });
                Self { keys, assertions }
            }
        }

        /// How one admission ended; allowed cases must reach real admission.
        #[derive(Debug, PartialEq, Eq, Clone, Copy)]
        enum Outcome {
            Admitted,
            Denied,
        }
        use Outcome::{Admitted, Denied};

        /// A root socket past the upgrade, holding its NIP-42 challenge.
        struct RootSession {
            socket: tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            challenge: String,
            relay: String,
        }

        /// Upgrade the root socket at `host` with `token`. A deny at upgrade
        /// is the 403 deny body; anything else that is not an upgrade fails.
        async fn open_root(pod: &Pod, host: &str, token: &str) -> Result<RootSession, Outcome> {
            let mut req = format!("ws://{}/", pod.addr)
                .into_client_request()
                .expect("request");
            req.headers_mut().insert("host", host.parse().unwrap());
            req.headers_mut().insert(
                "nostr-federated-identity",
                format!("Bearer {token}").parse().unwrap(),
            );
            let mut socket = match tokio_tungstenite::connect_async(req).await {
                Ok((socket, _)) => socket,
                Err(tungstenite::Error::Http(resp)) => {
                    let body = resp.body().clone().unwrap_or_default();
                    assert_eq!(
                        (resp.status().as_u16(), &body[..]),
                        (403, DENY_BODY),
                        "root upgrade refused other than by the deny body"
                    );
                    return Err(Denied);
                }
                Err(e) => panic!("root upgrade failed: {e}"),
            };
            let challenge = loop {
                let frame = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
                    .await
                    .expect("AUTH challenge in time");
                if let Some(Ok(Message::Text(text))) = frame {
                    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if json[0] == "AUTH" {
                        break json[1].as_str().unwrap().to_owned();
                    }
                } else {
                    panic!("expected the AUTH challenge, got {frame:?}");
                }
            };
            let relay = format!(
                "{}://{host}",
                if pod.state.config.relay_url.starts_with("wss://") {
                    "wss"
                } else {
                    "ws"
                }
            );
            Ok(RootSession {
                socket,
                challenge,
                relay,
            })
        }

        impl RootSession {
            /// Complete NIP-42 as `keys`: `Admitted` on OK true, `Denied` on
            /// the authorization-denied frame followed by the close.
            async fn authenticate(mut self, keys: &nostr::Keys) -> Outcome {
                let auth = nostr::EventBuilder::new(nostr::Kind::Authentication, "")
                    .tag(nostr::Tag::parse(["relay", &self.relay]).unwrap())
                    .tag(nostr::Tag::parse(["challenge", &self.challenge]).unwrap())
                    .sign_with_keys(keys)
                    .unwrap();
                let id = auth.id.to_hex();
                let frame = serde_json::json!(["AUTH", auth]).to_string();
                self.socket.send(Message::Text(frame.into())).await.unwrap();
                let denial = buzz_auth::DenialClass::AuthorizationDenied.nostr_text();
                let mut denied = false;
                loop {
                    let frame =
                        tokio::time::timeout(std::time::Duration::from_secs(5), self.socket.next())
                            .await
                            .expect("AUTH outcome in time");
                    match frame {
                        Some(Ok(Message::Text(text))) => {
                            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
                            if json[0] == "OK" && json[1] == id.as_str() && json[2] == true {
                                assert!(!denied, "admitted after a denial frame");
                                return Admitted;
                            }
                            denied |= text.contains(denial);
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                            assert!(denied, "root closed without the denial frame");
                            return Denied;
                        }
                        Some(Ok(_)) => {}
                    }
                }
            }
        }

        /// A full root session: upgrade, then NIP-42.
        async fn root(pod: &Pod, keys: &nostr::Keys, host: &str, token: &str) -> Outcome {
            match open_root(pod, host, token).await {
                Ok(session) => session.authenticate(keys).await,
                Err(denied) => denied,
            }
        }

        /// A signed NIP-98 `POST /query` through the real handler: 200 when
        /// admitted, the deny body when denied.
        async fn http(pod: &Pod, keys: &nostr::Keys, host: &str, token: &str) -> Outcome {
            use base64::Engine as _;
            use sha2::Digest as _;
            let body = br#"[{"kinds":[1],"limit":1}]"#;
            let scheme = if pod.state.config.relay_url.starts_with("wss://") {
                "https"
            } else {
                "http"
            };
            let proof = nostr::EventBuilder::new(nostr::Kind::HttpAuth, "")
                .tags([
                    nostr::Tag::parse(["u", &format!("{scheme}://{host}/query")]).unwrap(),
                    nostr::Tag::parse(["method", "POST"]).unwrap(),
                    // A fresh proof per request: the replay guard is shared.
                    nostr::Tag::parse(["nonce", &uuid::Uuid::new_v4().to_string()]).unwrap(),
                    nostr::Tag::parse(["payload", &hex::encode(sha2::Sha256::digest(body))])
                        .unwrap(),
                ])
                .sign_with_keys(keys)
                .unwrap();
            let proof = base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_vec(&proof).unwrap());
            let request = Request::post("/query")
                .header(axum::http::header::HOST, host)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .header(axum::http::header::AUTHORIZATION, format!("Nostr {proof}"))
                .header(CLIENT_ATTACHED_HEADER, format!("Bearer {token}"))
                .body(Body::from(&body[..]))
                .unwrap();
            let resp = crate::router::build_router(Arc::clone(&pod.state))
                .oneshot(request)
                .await
                .unwrap();
            let status = resp.status();
            let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
                .await
                .unwrap();
            match status {
                StatusCode::OK => Admitted,
                StatusCode::FORBIDDEN if &body[..] == DENY_BODY => Denied,
                other => panic!(
                    "unexpected HTTP outcome {other}: {}",
                    String::from_utf8_lossy(&body)
                ),
            }
        }

        /// The audio deny-set check after a real NIP-42 session.
        async fn audio(pod: &Pod, keys: &nostr::Keys, host: &str, token: &str) -> Outcome {
            let binding = pod
                .state
                .config
                .nip_fi
                .communities
                .resolve(host)
                .expect("served");
            let assertion = pod
                .state
                .nip_fi_verifier
                .as_deref()
                .expect("verifier")
                .verify_assertion(token, binding)
                .expect("the retained assertion is still valid");
            if crate::audio::handler::test_support::deny_set_refuses(
                Arc::clone(&pod.state),
                assertion,
                keys,
            )
            .await
            {
                Denied
            } else {
                Admitted
            }
        }

        /// Asserts every retained assertion of K (expected `key`) and the
        /// control key (always admitted), on both pods and in both
        /// communities, at root, audio and HTTP.
        async fn check(pods: &[Pod; 2], [k, control]: [&Retained; 2], key: Outcome, phase: &str) {
            for (n, pod) in pods.iter().enumerate() {
                for (who, retained, expected) in [("K", k, key), ("control", control, Admitted)] {
                    for (host, token) in &retained.assertions {
                        let keys = &retained.keys;
                        assert_eq!(
                            [
                                root(pod, keys, host, token).await,
                                audio(pod, keys, host, token).await,
                                http(pod, keys, host, token).await,
                            ],
                            [expected; 3],
                            "{phase}: {who} at {host} on P{} [root, audio, HTTP]",
                            n + 1
                        );
                    }
                }
            }
        }

        /// Root sessions for every retained assertion on both pods, upgraded
        /// but not yet authenticated.
        async fn open_all(pods: &[Pod; 2], retained: &Retained) -> Vec<(String, RootSession)> {
            let mut sessions = Vec::new();
            for (n, pod) in pods.iter().enumerate() {
                for (host, token) in &retained.assertions {
                    let session = open_root(pod, host, token)
                        .await
                        .unwrap_or_else(|_| panic!("{host} on P{} must upgrade", n + 1));
                    sessions.push((format!("{host} on P{}", n + 1), session));
                }
            }
            sessions
        }

        /// Sessions K upgraded before a disconnect landed, now finishing
        /// NIP-42: only the post-registration deny-set check refuses them.
        /// The control key's open sessions are admitted.
        async fn authenticate_all(
            phase: &str,
            k: (&nostr::Keys, Vec<(String, RootSession)>),
            control: (&nostr::Keys, Vec<(String, RootSession)>),
        ) {
            for (who, (keys, sessions), expected) in
                [("K", k, Denied), ("control", control, Admitted)]
            {
                for (at, session) in sessions {
                    assert_eq!(
                        session.authenticate(keys).await,
                        expected,
                        "{phase}: {who}'s session upgraded before it, at {at}"
                    );
                }
            }
        }

        // Plan criterion 10: restart recovery across pods. K and a control
        // key each hold one assertion per community (two exact host bindings
        // in PostgreSQL and config), minted before the revocation and replayed
        // as the same bytes in every phase, on both pods, through a real root
        // NIP-42 session, the audio deny-set check and a signed NIP-98 request
        // to a real handler. A disconnect sent to one pod denies K on both. A
        // root session K upgraded before the disconnect is refused at NIP-42
        // by the post-registration deny-set check. Restarting both pods
        // empties their deny maps, so the retained assertions are admitted
        // again until the issuer re-sends; that is the exposure window,
        // recorded from restart to restored denial. The control key is
        // admitted throughout. Mutations: dropping the post-registration
        // check admits K's pre-upgraded sessions; dropping cross-pod
        // application leaves K admitted on the other pod.
        #[tokio::test]
        #[ignore = "requires Postgres and Redis"]
        async fn re_sent_disconnect_restores_the_deny_on_every_pod_after_restart() {
            let tag = uuid::Uuid::new_v4().simple().to_string();
            let hosts = [
                format!("restart-a-{tag}.example"),
                format!("restart-b-{tag}.example"),
            ];
            let db = sqlx::PgPool::connect(&crate::test_support::database_url())
                .await
                .expect("PostgreSQL must be available");
            for host in &hosts {
                sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                    .bind(uuid::Uuid::new_v4())
                    .bind(host)
                    .execute(&db)
                    .await
                    .expect("seed community");
            }
            let k = Retained::mint(&hosts);
            let control = Retained::mint(&hosts);
            let minted: Vec<String> = [&k, &control]
                .iter()
                .flat_map(|r| r.assertions.iter().map(|(_, t)| t.clone()))
                .collect();
            let both = [&k, &control];
            let key = k.keys.public_key();
            let until = chrono::Utc::now().timestamp() + 300;
            let send = |pod: Arc<AppState>| async move {
                let token = mint_token(&key.to_hex(), 0, serde_json::json!({ "until": until }));
                let (status, body) = post_command(&pod, &token, &key.to_hex()).await;
                assert_eq!(status, StatusCode::OK);
                assert_eq!(&body[..], br#"{"disconnected": true}"#);
                drain_publishes(&pod).await;
            };

            let mut pods = pair(&hosts).await;
            check(&pods, both, Admitted, "before the disconnect").await;
            let (k_open, control_open) =
                (open_all(&pods, &k).await, open_all(&pods, &control).await);
            send(Arc::clone(&pods[0].state)).await;
            await_target(&mut pods[1].seen, &key).await;
            authenticate_all(
                "after the disconnect",
                (&k.keys, k_open),
                (&control.keys, control_open),
            )
            .await;
            check(&pods, both, Denied, "after the disconnect").await;

            // Restart: fresh pods over the same database, empty deny maps.
            drop(pods);
            let mut pods = pair(&hosts).await;
            let restarted = std::time::Instant::now();
            let (first_host, first_token) = &k.assertions[0];
            assert_eq!(
                root(&pods[0], &k.keys, first_host, first_token).await,
                Admitted,
                "restarted: K's retained assertion completes a root session on P1"
            );
            let first_admitted = restarted.elapsed();
            check(&pods, both, Admitted, "restarted, before the re-send").await;

            let (k_open, control_open) =
                (open_all(&pods, &k).await, open_all(&pods, &control).await);
            send(Arc::clone(&pods[1].state)).await;
            let [p1, p2] = &mut pods;
            await_target(&mut p1.seen, &key).await;
            await_target(&mut p2.seen, &key).await;
            authenticate_all(
                "after the re-send",
                (&k.keys, k_open),
                (&control.keys, control_open),
            )
            .await;
            check(&pods, both, Denied, "after the re-send").await;
            let restored = restarted.elapsed();

            let replayed: Vec<String> = [&k, &control]
                .iter()
                .flat_map(|r| r.assertions.iter().map(|(_, t)| t.clone()))
                .collect();
            assert_eq!(replayed, minted, "assertion bytes unchanged");
            eprintln!(
                "criterion 10 exposure: first retained root admission {first_admitted:?} after \
                 restart; denial restored on every pod {restored:?} after restart"
            );
            assert!(first_admitted < restored);
        }
    }
}
