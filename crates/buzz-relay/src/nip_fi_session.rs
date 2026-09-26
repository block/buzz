//! Shared NIP-FI post-upgrade session seams.
//!
//! This module owns:
//!
//! * [`NipFiWsRoute`] — route discriminant for frame construction and logging.
//! * [`enforce_nip_fi_key_pairing`] — the single production function that owns
//!   the full NIP-FI key-pairing verdict, denial frame delivery, metric,
//!   auth-state transition (Root), and cancellation for both ingresses.
//! * [`spawn_nip_fi_expiry_task`] — the shared session-lifetime enforcement
//!   constructor used by both root and audio routes.
//! * [`authorization_denied_frame`] — route-specific frame builder used by
//!   both the pairing seam and the expiry seam.
//!
//! **Invariant**: both production call sites call `enforce_nip_fi_key_pairing`
//! and `spawn_nip_fi_expiry_task` from this module; no caller may re-implement
//! these side effects.

use axum::extract::ws::Message as WsMessage;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use crate::connection::ConnectionState;

// ── Route discriminant ────────────────────────────────────────────────────────

/// Which ingress a session is on. Governs denial frame format and log labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NipFiWsRoute {
    Root,
    Audio,
}

// ── Pairing seam ──────────────────────────────────────────────────────────────

/// Outcome of [`enforce_nip_fi_key_pairing`].
///
/// Callers MUST return immediately on `Denied`; all denial side-effects have
/// already been performed inside the function.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PairingOutcome {
    Paired,
    Denied,
}

/// Route-specific resources needed to deliver the pairing denial.
pub(crate) enum PairingDenialTarget<'a> {
    Root(&'a ConnectionState),
    Audio {
        ws_send: &'a mut futures_util::stream::SplitSink<
            axum::extract::ws::WebSocket,
            axum::extract::ws::Message,
        >,
        cancel: &'a CancellationToken,
        channel_id: Uuid,
    },
}

/// Enforce the NIP-FI key-pairing invariant [FI-INV-05].
///
/// When an assertion was presented at upgrade, the proven NIP-42 key MUST equal
/// the assertion's `nostr_pubkey` claim; a claimless assertion is also a denial.
///
/// This function owns the **entire denial path**: verdict, route-specific denial
/// frame delivery, `buzz_auth_failures_total{reason="nip_fi_key_mismatch"}`,
/// a route-labelled warning (no `iss`/`sub`/raw-assertion fields), auth-state
/// transition (Root only), and cancellation. Callers must not repeat any of
/// those effects.
///
/// Returns [`PairingOutcome::Paired`] when:
/// * no assertion is present (off-mode), or
/// * the assertion's `nostr_pubkey` claim matches `proven_pubkey`.
///
/// Returns [`PairingOutcome::Denied`] after performing all denial side-effects.
pub(crate) async fn enforce_nip_fi_key_pairing(
    assertion: Option<&buzz_auth::VerifiedAssertion>,
    proven_pubkey: nostr::PublicKey,
    target: PairingDenialTarget<'_>,
) -> PairingOutcome {
    // No assertion → off-mode; pass unconditionally.
    let Some(assertion) = assertion else {
        return PairingOutcome::Paired;
    };

    // Matching key → pass.
    if matches!(assertion.asserted_key(), Some(k) if k == proven_pubkey) {
        return PairingOutcome::Paired;
    }

    // Mismatch or claimless assertion — single shared denial branch.
    metrics::counter!(
        "buzz_auth_failures_total",
        "reason" => "nip_fi_key_mismatch"
    )
    .increment(1);

    match target {
        PairingDenialTarget::Root(conn) => {
            warn!(
                conn_id = %conn.conn_id,
                route = "root",
                proven_pubkey = %proven_pubkey.to_hex(),
                "NIP-FI key pairing mismatch — closing connection"
            );
            *conn.auth_state.lock().unwrap() = crate::connection::AuthState::Failed;
            // Serialize through the terminal-transition lock: reason publication +
            // winner-only frame enqueue happen while the lock is held, so a
            // concurrent disconnect_community cannot fire cancel.cancel() before
            // the winning payload is enqueued.  The auth_state write uses the sync
            // StdMutex and completes before acquiring the sync transition lock.
            // [FI-TRACE-CANCEL-RACE]
            conn.community_control
                .pairing_deny_terminal(&conn.terminal_ctrl_tx, NipFiWsRoute::Root);
            conn.cancel.cancel();
        }
        PairingDenialTarget::Audio {
            ws_send,
            cancel,
            channel_id,
        } => {
            warn!(
                %channel_id,
                route = "audio",
                proven_pubkey = %proven_pubkey.to_hex(),
                "NIP-FI key pairing mismatch — closing connection"
            );
            use futures_util::SinkExt as _;
            // Bounded sends: pre-writer audio exits must not block on a stalled
            // sink indefinitely.  Same 1-second terminal/close policy as every
            // other pre-writer error exit.  [R2: bounded I/O invariant]
            let _deny_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
            let _ = tokio::time::timeout_at(
                _deny_deadline,
                ws_send.send(authorization_denied_frame(NipFiWsRoute::Audio)),
            )
            .await;
            // Send explicit 1008 POLICY close frame before dropping. The audio
            // handler owns ws_send directly here (send_loop not yet started).
            // [FI-TRACE-CLOSE-CODE]
            let _ = tokio::time::timeout_at(
                _deny_deadline,
                ws_send.send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::POLICY,
                    reason: axum::extract::ws::Utf8Bytes::from_static("authorization denied"),
                }))),
            )
            .await;
            cancel.cancel();
        }
    }

    PairingOutcome::Denied
}

// ── Shared frame constructor ───────────────────────────────────────────────────

/// Build the exact NIP-FI authorization-denied frame for the given route.
///
/// * Root: a Nostr NOTICE — `["NOTICE","restricted: authorization denied"]`.
/// * Audio: `{"type":"restricted","message":"restricted: authorization denied"}`.
pub(crate) fn authorization_denied_frame(route: NipFiWsRoute) -> WsMessage {
    use buzz_auth::DenialClass;
    let text = DenialClass::AuthorizationDenied.nostr_text();
    WsMessage::Text(match route {
        NipFiWsRoute::Root => crate::protocol::RelayMessage::notice(text).into(),
        NipFiWsRoute::Audio => serde_json::json!({"type": "restricted", "message": text})
            .to_string()
            .into(),
    })
}

// ── Shared expiry task constructor ────────────────────────────────────────────

/// Spawn the NIP-FI session-lifetime enforcement task for either route.
///
/// At `deadline`, the task:
/// 1. Calls `gate.expire(terminal)` with the route-specific terminal closure.
///    Inside `gate.expire()`:
///    a. The terminal closure sets `deny_reason_tx` to `AuthorizationDenied`
///    (so the send loop's cancel branch emits a 1008 POLICY close frame),
///    enqueues the denial frame on `terminal_ctrl_tx`, and increments the
///    lease-expiration metric. [FI-TRACE-CLOSE-CODE]
///    b. `cancel.cancel()` — socket termination starts immediately.
///    c. The gate acquires the write guard (quiescence barrier) — blocks until
///    all outstanding effect permits are released, then records `Expired`.
/// 2. The task then returns, allowing connection teardown to proceed.
///
/// Equality at deadline is expired; already-expired deadlines fire immediately.
/// No in-band renewal is added. [FI-TRACE-LEASE-BOUND]
pub(crate) fn spawn_nip_fi_expiry_task(
    deadline: chrono::DateTime<chrono::Utc>,
    gate: std::sync::Arc<crate::nip_fi_gate::SessionAdmissionGate>,
    terminal_ctrl_tx: mpsc::Sender<WsMessage>,
    route: NipFiWsRoute,
    control: crate::state::CommunityConnectionControl,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let now = chrono::Utc::now();
        // Equality at deadline is expired: strict less-than.
        let remaining = if now < deadline {
            (deadline - now)
                .to_std()
                .unwrap_or(std::time::Duration::ZERO)
        } else {
            std::time::Duration::ZERO
        };
        tokio::select! {
            _ = tokio::time::sleep(remaining) => {
                // F4 elapsed-deadline schedule witness: fires AFTER sleep (wall-clock
                // past deadline) but BEFORE gate.expire() (cancel not yet set).
                // This lets the test confirm that acquire_effect() returns
                // SessionExpired via the deadline arm (nip_fi_gate.rs:139-142),
                // not the cancellation arm (136-137).  No-op in production.
                // [nip_fi_session::expiry_pre_expire_hook, F4]
                #[cfg(test)]
                expiry_pre_expire_hook::fire(control.hook_key).await;
                // gate.expire() ordering (per contract [6d3b75a5]):
                //   1. terminal() — queues denial frame before any lock is held.
                //   2. cancel.cancel() — socket termination at the deadline.
                //   3. write guard — quiescence barrier; blocks until all pre-expiry
                //      effect permits are released, then records Expired.
                // The task's await on gate.expire() completes only after the write
                // guard is released, so connection teardown (which awaits this task
                // handle before remove_connection) cannot start until pre-expiry
                // effects have finished their bounded commits.
                gate.expire(|| {
                    // Acquire the transition lock so a concurrent disconnect_community
                    // that loses reason publication cannot fire cancel.cancel() until
                    // this call's winning try_send completes.  Without the lock,
                    // community's cancel could wake the consumer before the denial
                    // frame is enqueued — reproducing the original close-only defect
                    // for an expiry/delete race.  [FI-TRACE-CANCEL-RACE]
                    control.expiry_deny_terminal(&terminal_ctrl_tx, route);
                    metrics::counter!("buzz_nip_fi_lease_expirations_total").increment(1);
                    warn!(
                        route = ?route,
                        "NIP-FI session lease expired — closing connection"
                    );
                })
                .await;
            }
            _ = gate.cancelled() => {
                // F5: admin-cancel quiescence barrier.
                //
                // When cancel fires from outside (admin disconnect, deny-set scan,
                // or community delete), the denial frame has already been enqueued
                // by the caller that cancelled the token.  We must NOT produce a
                // second terminal frame — so we do NOT call expire() here.
                //
                // However, we DO need to wait for any in-flight effect permits
                // before the task returns.  The connection teardown in connection.rs
                // awaits this task handle before running remove_connection and
                // deregister.  Without quiesce(), a REQ handler that acquired a
                // permit (read guard) before the cancel can finish its sub_registry
                // registration AFTER teardown has already run remove_connection —
                // leaving orphan subscription entries and retained topic references
                // that accumulate across repeated admin-cancel/REQ-resume cycles.
                //
                // gate.quiesce() acquires the write guard, blocking until all
                // outstanding permits are released, then records Expired — the
                // same quiescence barrier used by expire(), without the terminal
                // or cancel calls.  [FI-TRACE-LEASE-BOUND, F5]
                //
                // Signal the test hook BEFORE quiesce() so the test can observe
                // the task reaching the quiescence wait point — the explicit
                // rendezvous that replaces the 50 ms timing window.  No-op in
                // production.  [F5: quiesce-entry rendezvous]
                #[cfg(test)]
                quiesce_cancel_arm_hook::fire(control.hook_key);
                gate.quiesce().await;
            }
        }
    })
}

// ── Test-only hook: quiesce-entry rendezvous ──────────────────────────────────
//
// Fires a oneshot in the cancel arm of `spawn_nip_fi_expiry_task` immediately
// before `gate.quiesce().await`.  The test awaits this signal to know that the
// task has entered the cancel arm and is blocked at the quiescence barrier —
// replacing the 50 ms timing window with an explicit rendezvous.
//
// Keyed by `control.hook_key` (per-control UUID) so concurrent tests never share
// a slot.  Zero-cost in production: the module is `#[cfg(test)]` only.
#[cfg(test)]
pub(crate) mod quiesce_cancel_arm_hook {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    use tokio::sync::oneshot;
    use uuid::Uuid;

    static HOOKS: OnceLock<Mutex<HashMap<Uuid, oneshot::Sender<()>>>> = OnceLock::new();

    fn hook_map() -> &'static Mutex<HashMap<Uuid, oneshot::Sender<()>>> {
        HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Arm a one-shot signal for `key`. Returns the receiver the test awaits.
    pub(crate) fn arm(key: Uuid) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        hook_map().lock().unwrap().insert(key, tx);
        rx
    }

    /// Disarm the hook for `key` (call after the test to prevent interference).
    #[allow(dead_code)]
    pub(crate) fn disarm(key: Uuid) {
        hook_map().lock().unwrap().remove(&key);
    }

    /// Called by `spawn_nip_fi_expiry_task` in the cancel arm before quiesce().
    /// No-op when not armed for this key.
    pub(crate) fn fire(key: Uuid) {
        let tx = hook_map().lock().unwrap().remove(&key);
        if let Some(t) = tx {
            let _ = t.send(());
        }
    }
}

// ── Test-only hook: expiry-task pre-expire rendezvous (F4 elapsed-deadline) ──
//
// Fires a two-phase gate in the sleep arm of `spawn_nip_fi_expiry_task`,
// AFTER `tokio::time::sleep(remaining)` completes (wall-clock deadline elapsed)
// but BEFORE `gate.expire()` is called (cancel token not yet set).
//
// This creates the exact precondition for exercising the elapsed-deadline arm
// at `nip_fi_gate.rs:139-142`:
//   - `Utc::now() >= deadline`  — true: sleep fired after real wall-clock time
//   - `cancel.is_cancelled()`  — false: gate.expire() hasn't called cancel.cancel() yet
//
// Keyed by `control.hook_key` (per-control UUID) so concurrent tests never share
// a slot.  Zero-cost in production: the module is `#[cfg(test)]` only.
#[cfg(test)]
pub(crate) mod expiry_pre_expire_hook {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    use tokio::sync::{oneshot, Notify};
    use uuid::Uuid;

    struct Gate {
        arrived: oneshot::Sender<()>,
        release: Arc<Notify>,
    }

    static HOOKS: OnceLock<Mutex<HashMap<Uuid, Gate>>> = OnceLock::new();

    fn hook_map() -> &'static Mutex<HashMap<Uuid, Gate>> {
        HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Arm a two-phase barrier for `key`.
    ///
    /// Returns `(arrived_rx, release)`. Await `arrived_rx` to know when the
    /// expiry task has paused before `gate.expire()`; call `release.notify_one()`
    /// to let it proceed.
    pub(crate) fn arm(key: Uuid) -> (oneshot::Receiver<()>, Arc<Notify>) {
        let (tx, rx) = oneshot::channel();
        let release = Arc::new(Notify::new());
        hook_map().lock().unwrap().insert(
            key,
            Gate {
                arrived: tx,
                release: release.clone(),
            },
        );
        (rx, release)
    }

    /// Disarm the hook for `key` (clean up if test completes before expiry fires).
    #[allow(dead_code)]
    pub(crate) fn disarm(key: Uuid) {
        hook_map().lock().unwrap().remove(&key);
    }

    /// Called by `spawn_nip_fi_expiry_task` in the sleep arm before gate.expire().
    /// Signals arrived, then waits for release.  No-op when not armed.
    pub(crate) async fn fire(key: Uuid) {
        let gate = hook_map().lock().unwrap().remove(&key);
        if let Some(g) = gate {
            let _ = g.arrived.send(());
            g.release.notified().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use nostr::Keys;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    // ── B3: terminal denial frame survives saturated ctrl_tx ──────────────────
    //
    // Root pairing and expiry both write the denial frame to `terminal_ctrl_tx`
    // (capacity 1) instead of `ctrl_tx` (capacity 8). These tests saturate
    // ctrl_tx completely, then fire the denial path and assert the frame arrives
    // on the terminal channel regardless.
    //
    // Mutation evidence:
    //   A) Switch `enforce_nip_fi_key_pairing` back to `ctrl_tx.try_send` →
    //      terminal_rx is empty → recv assertion panics.
    //   B) Switch `spawn_nip_fi_expiry_task` back to `ctrl_tx.try_send` →
    //      terminal_rx is empty → recv assertion panics.

    #[tokio::test]
    async fn b3_root_pairing_denial_delivered_when_ctrl_queue_saturated() {
        let keys = Keys::generate();
        let deadline = Utc::now() + chrono::Duration::hours(1);
        let assertion =
            buzz_auth::VerifiedAssertion::for_test(Some(keys.public_key()), vec![deadline]);

        let (send_tx, _send_rx) = mpsc::channel(4);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel::<WsMessage>(8);
        let (terminal_ctrl_tx, mut terminal_rx) = mpsc::channel::<WsMessage>(1);

        // Saturate ctrl_tx to capacity 8.
        for i in 0..8u8 {
            ctrl_tx
                .try_send(WsMessage::Text(format!("ordinary-{i}").into()))
                .expect("ctrl_tx has capacity 8");
        }
        assert!(
            ctrl_tx
                .try_send(WsMessage::Text("overflow".into()))
                .is_err(),
            "ctrl_tx must be full before the test exercises the denial path"
        );

        let b3_cancel = CancellationToken::new();
        let b3_control = crate::state::CommunityConnectionControl::new(b3_cancel.clone());
        let conn = Arc::new(crate::connection::ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(
                buzz_core::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: std::sync::Mutex::new(crate::connection::AuthState::Pending {
                challenge: "test-challenge".to_string(),
                started_at: std::time::Instant::now(),
            }),
            subscriptions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: b3_cancel.clone(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: Some(assertion),
            session_deadline: None,
            nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(b3_cancel.clone()),
            community_control: b3_control,
        });
        // Use a different key as the proven pubkey → forced mismatch.
        let wrong_pubkey = Keys::generate().public_key();
        let outcome = enforce_nip_fi_key_pairing(
            conn.nip_fi_assertion.as_ref(),
            wrong_pubkey,
            PairingDenialTarget::Root(conn.as_ref()),
        )
        .await;

        assert_eq!(outcome, PairingOutcome::Denied, "mismatch must be Denied");
        assert!(
            conn.cancel.is_cancelled(),
            "cancel must be called on denial"
        );

        // Terminal channel must have the denial frame despite ctrl_tx being full.
        let frame = terminal_rx
            .try_recv()
            .expect("denial frame must arrive on terminal channel even when ctrl_tx is full");
        match frame {
            WsMessage::Text(t) => {
                let v: serde_json::Value =
                    serde_json::from_str(&t).expect("denial frame is valid JSON");
                assert!(
                    v.get(1)
                        .and_then(|c| c.as_str())
                        .map(|s| s.contains("authorization denied"))
                        .unwrap_or(false),
                    "root denial frame must contain 'authorization denied': {t}"
                );
            }
            other => panic!("expected Text denial frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn b3_expiry_denial_delivered_when_ctrl_queue_saturated() {
        // Saturate a separate ctrl channel to prove the expiry task doesn't
        // depend on it being available.
        let (ctrl_tx, _ctrl_rx) = mpsc::channel::<WsMessage>(8);
        for i in 0..8u8 {
            ctrl_tx
                .try_send(WsMessage::Text(format!("ordinary-{i}").into()))
                .expect("ctrl_tx has capacity 8");
        }
        drop(ctrl_tx); // expiry task never touches ctrl_tx; drop proves it

        let (terminal_tx, mut terminal_rx) = mpsc::channel::<WsMessage>(1);
        let cancel = CancellationToken::new();
        let already_expired = Utc::now() - chrono::Duration::seconds(1);

        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(already_expired, cancel.clone());
        // Build a minimal control for the expiry task — its transition lock
        // serializes the terminal enqueue vs. concurrent community deletes.
        let control = crate::state::CommunityConnectionControl::new(cancel.clone());
        let handle = spawn_nip_fi_expiry_task(
            already_expired,
            gate,
            terminal_tx,
            NipFiWsRoute::Root,
            control,
        );
        handle.await.expect("expiry task must complete");

        assert!(
            cancel.is_cancelled(),
            "cancel must be called by expiry task"
        );

        // Terminal channel must have the denial frame.
        let frame = terminal_rx
            .try_recv()
            .expect("expiry denial frame must be in terminal channel");
        match frame {
            WsMessage::Text(t) => {
                assert!(
                    t.contains("authorization denied"),
                    "expiry denial frame must contain 'authorization denied': {t}"
                );
            }
            other => panic!("expected Text denial frame, got {other:?}"),
        }
    }

    // ── W_f5: admin-cancel arm waits for held permits before task exit ────────
    //
    // Proves that when the expiry task's cancellation arm fires (external admin
    // cancel), the task calls `gate.quiesce()` and does NOT return until all
    // outstanding effect permits are released.
    //
    // This is the F5 integration-level witness.  Connection teardown in
    // `connection.rs` awaits the expiry task handle before calling
    // `remove_connection`.  Without quiescence, a REQ handler that acquired a
    // permit before the cancel could register subscriptions AFTER `remove_connection`
    // already ran — leaving orphan entries.  With `quiesce()` in the cancellation
    // arm, the task blocks until the permit is released, guaranteeing registration
    // completes before teardown.
    //
    // Schedule:
    //   1. Create a gate with a future deadline (so the sleep arm won't fire
    //      during the test) and acquire a permit.
    //   2. Spawn the expiry task.  It blocks in the select, waiting for either
    //      the sleep (far future) or cancellation.
    //   3. Cancel the token — the cancellation arm fires, calls quiesce().
    //      quiesce() blocks on the write guard because the permit (read guard)
    //      is still held.
    //   4. Yield several times — assert the task has NOT completed.
    //   5. Drop the permit — quiesce() acquires the write guard and finishes.
    //      The task completes.
    //   6. Assert the task completed and the terminal channel is EMPTY (no second
    //      denial frame from the cancellation arm).
    //
    // Mutation evidence:
    //   A) Remove `gate.quiesce().await` from the cancellation arm →
    //      the task completes before the permit is dropped →
    //      "task must not have finished" assertion panics.
    //   B) Replace `quiesce()` with a bare `return` in the cancellation arm →
    //      same as (A).
    //   C) Remove the `gate.quiesce().await` and insert a `gate.expire()` call →
    //      a second terminal frame is enqueued → terminal channel is non-empty →
    //      "terminal channel must be empty" assertion panics.

    // ── W_f4_add_peer / W_f4_commit: delayed-expiry-task wire witnesses ─────────
    //
    // Both witnesses prove that the audio rejection paths (add-peer and commit)
    // call `control.expiry_deny_terminal` BEFORE self-cancelling even when a
    // spawned expiry task is running but hasn't yet fired its timer.
    //
    // Production scenario:
    //   `acquire_effect()` returns `SessionExpired` via the wall-clock path
    //   (Utc::now() >= deadline) before the expiry task's sleep fires.  Without
    //   the fix, both rejection paths called raw `lifecycle_cancel()` first; the
    //   expiry task's select takes the cancellation arm and calls only quiesce(),
    //   NOT expiry_deny_terminal.  Result: terminal channel empty → no restricted
    //   JSON, no 1008 policy close.
    //
    // With the fix: the rejection path calls `expiry_deny_terminal` first (wins
    // the reason slot and enqueues the frame), then calls `lifecycle_cancel()`.
    // The expiry task's cancel arm runs quiesce() — no second frame.
    //
    // Binding: both witnesses use a REAL gate with a past deadline (so
    // acquire_effect actually returns SessionExpired) and a REAL spawned expiry
    // task (far-future deadline, waiting in its sleep arm).  These tests call
    // `expiry_deny_terminal` DIRECTLY in the test body — they prove the primitive's
    // ordering invariant (enqueue-before-cancel), not the handler's invocation of
    // it.  Handler-bound wire delivery is proven in
    // `postgres_tests::f4_add_peer_expired_delivers_denial_before_close` and
    // `postgres_tests::f4_commit_expired_delivers_denial_before_close`.
    //
    // Mutation evidence (for both witnesses):
    //   A) Remove the `expiry_deny_terminal` call from the rejection sequence →
    //      channel stays empty → "denial frame must be in terminal channel" panics.
    //   B) Swap order (lifecycle_cancel BEFORE expiry_deny_terminal) → if
    //      lifecycle_cancel wins first and the task takes the cancel arm (quiesce
    //      only), the frame is never enqueued → same panic.
    //   C) Produce a second expiry_deny_terminal call → second try_send returns
    //      Err(Full) (capacity 1), only one frame present — no double-frame.

    #[tokio::test]
    async fn w_f4_add_peer_rejection_wire_delivers_denial_before_cancel() {
        // Primitive-level ordering proof for the add-peer rejection sequence.
        //
        // What this proves: when `expiry_deny_terminal` runs BEFORE `lifecycle_cancel`,
        // the terminal channel is populated before any cancel wakes a consumer.
        //
        // Production path (audio/handler.rs SessionExpired arm, line ~959):
        //   1. acquire_effect() → Err(SessionExpired)
        //   2. control.expiry_deny_terminal(&terminal_ctrl_tx, Audio)  ← this call
        //   3. control.lifecycle_cancel()
        //
        // This test calls `expiry_deny_terminal` directly (same call as the handler).
        // Mutation within this test:
        //   Reorder steps 2 and 3 (call lifecycle_cancel before expiry_deny_terminal)
        //   → consumer wakes on empty channel → try_recv returns Err → assertion panics.
        //
        // Handler-bound wire delivery is proven in
        //   `postgres_tests::f4_add_peer_expired_delivers_denial_before_close`.
        let (terminal_tx, mut terminal_rx) = tokio::sync::mpsc::channel::<WsMessage>(1);
        let cancel = CancellationToken::new();
        let control = crate::state::CommunityConnectionControl::new(cancel.clone());

        // Gate with a past deadline → acquire_effect returns SessionExpired immediately.
        let past_deadline = Utc::now() - chrono::Duration::seconds(1);
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(past_deadline, cancel.clone());

        // Spawn expiry task with a FAR-FUTURE deadline — it is waiting in its
        // sleep arm and will never fire the timer arm during this test.
        // When lifecycle_cancel() fires, the task takes the cancel arm (quiesce)
        // NOT the timer arm (expiry_deny_terminal) — so no frame from the task.
        let far_future = Utc::now() + chrono::Duration::hours(1);
        let gate_for_task =
            crate::nip_fi_gate::SessionAdmissionGate::new(far_future, cancel.clone());
        let expiry_task = spawn_nip_fi_expiry_task(
            far_future,
            Arc::clone(&gate_for_task),
            terminal_tx.clone(),
            NipFiWsRoute::Audio,
            control.clone(),
        );

        // Verify acquire_effect returns SessionExpired (wall-clock path, past deadline).
        let result = gate.acquire_effect().await;
        assert!(
            matches!(result, Err(crate::nip_fi_gate::SessionExpired)),
            "W_f4_add_peer: gate with past deadline must return SessionExpired"
        );

        // Production rejection sequence — EXACTLY what audio/handler.rs does:
        control.expiry_deny_terminal(&terminal_tx, NipFiWsRoute::Audio);

        // Frame must be in the channel BEFORE lifecycle_cancel fires.
        let frame = terminal_rx.try_recv().expect(
            "W_f4_add_peer: denial frame must be in terminal channel after expiry_deny_terminal \
             and before lifecycle_cancel (proves rejection path enqueues before self-cancel). \
             Mutation: reorder steps 2 and 3 → cancel fires first → Err(Empty) → RED.",
        );
        let expected = authorization_denied_frame(NipFiWsRoute::Audio);
        assert_eq!(
            frame, expected,
            "W_f4_add_peer: queued frame must be the canonical Audio authorization-denied frame"
        );

        // lifecycle_cancel fires — task takes the cancel arm (quiesce only, no frame).
        control.lifecycle_cancel();
        assert!(cancel.is_cancelled(), "W_f4_add_peer: cancel must be set");

        // Await the expiry task — must complete (quiesce with no held permits).
        tokio::time::timeout(std::time::Duration::from_secs(2), expiry_task)
            .await
            .expect("W_f4_add_peer: expiry task must complete within 2s after cancel")
            .expect("W_f4_add_peer: expiry task must not panic");

        // Task's cancel arm (quiesce) must NOT have enqueued a second frame.
        assert!(
            terminal_rx.try_recv().is_err(),
            "W_f4_add_peer: terminal channel must have exactly one frame \
             (expiry task cancel arm must not produce a duplicate via quiesce)"
        );

        // Reason slot must be AuthorizationDenied (not overwritten by task).
        let reason = *control.disconnect_reason().borrow();
        assert_eq!(
            reason,
            Some(crate::state::CommunityDisconnectReason::AuthorizationDenied),
            "W_f4_add_peer: reason slot must be AuthorizationDenied after expiry_deny_terminal"
        );
    }

    #[tokio::test]
    async fn w_f4_commit_rejection_wire_delivers_denial_before_cancel() {
        // Primitive-level ordering proof for the commit rejection sequence.
        //
        // What this proves: when `expiry_deny_terminal` runs BEFORE `lifecycle_cancel`,
        // the terminal channel is populated before any cancel wakes a consumer.
        //
        // Production path (audio/handler.rs JoinCommitError::Expired arm, line ~1306):
        //   1. commit_participant_join() → Err(JoinCommitError::Expired)
        //   2. control.expiry_deny_terminal(&terminal_ctrl_tx, Audio)  ← this call
        //   3. control.lifecycle_cancel()
        //
        // This test calls `expiry_deny_terminal` directly (same call as the handler).
        // Mutation within this test:
        //   Reorder steps 2 and 3 → cancel fires first → try_recv returns Err → RED.
        //
        // Handler-bound wire delivery is proven in
        //   `postgres_tests::f4_commit_expired_delivers_denial_before_close`.
        let (terminal_tx, mut terminal_rx) = tokio::sync::mpsc::channel::<WsMessage>(1);
        let cancel = CancellationToken::new();
        let control = crate::state::CommunityConnectionControl::new(cancel.clone());

        let past_deadline = Utc::now() - chrono::Duration::seconds(1);
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(past_deadline, cancel.clone());

        let far_future = Utc::now() + chrono::Duration::hours(1);
        let gate_for_task =
            crate::nip_fi_gate::SessionAdmissionGate::new(far_future, cancel.clone());
        let expiry_task = spawn_nip_fi_expiry_task(
            far_future,
            Arc::clone(&gate_for_task),
            terminal_tx.clone(),
            NipFiWsRoute::Audio,
            control.clone(),
        );

        // Verify acquire_effect returns SessionExpired.
        let result = gate.acquire_effect().await;
        assert!(
            matches!(result, Err(crate::nip_fi_gate::SessionExpired)),
            "W_f4_commit: gate with past deadline must return SessionExpired"
        );

        // Production commit rejection path:
        control.expiry_deny_terminal(&terminal_tx, NipFiWsRoute::Audio);

        let frame = terminal_rx.try_recv().expect(
            "W_f4_commit: denial frame must be in terminal channel after expiry_deny_terminal \
             and before lifecycle_cancel (proves commit rejection enqueues before self-cancel). \
             Mutation: reorder steps 2 and 3 → cancel fires first → Err(Empty) → RED.",
        );
        let expected = authorization_denied_frame(NipFiWsRoute::Audio);
        assert_eq!(
            frame, expected,
            "W_f4_commit: queued frame must be the canonical Audio authorization-denied frame"
        );

        control.lifecycle_cancel();
        assert!(cancel.is_cancelled(), "W_f4_commit: cancel must be set");

        tokio::time::timeout(std::time::Duration::from_secs(2), expiry_task)
            .await
            .expect("W_f4_commit: expiry task must complete within 2s after cancel")
            .expect("W_f4_commit: expiry task must not panic");

        assert!(
            terminal_rx.try_recv().is_err(),
            "W_f4_commit: terminal channel must have exactly one frame"
        );

        let reason = *control.disconnect_reason().borrow();
        assert_eq!(
            reason,
            Some(crate::state::CommunityDisconnectReason::AuthorizationDenied),
            "W_f4_commit: reason slot must be AuthorizationDenied after expiry_deny_terminal"
        );
    }

    #[tokio::test]
    async fn w_f5_quiescence_cancel_arm_blocks_until_permits_released() {
        let cancel = CancellationToken::new();
        let far_future = Utc::now() + chrono::Duration::hours(1);
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(far_future, cancel.clone());

        // Acquire a permit BEFORE spawning the task — simulates a REQ handler
        // that obtained its permit before the admin disconnect fired.
        let permit = gate.acquire_effect().await.expect("permit before cancel");

        let (terminal_tx, mut terminal_rx) = mpsc::channel::<WsMessage>(1);
        let control = crate::state::CommunityConnectionControl::new(cancel.clone());

        let task_handle = spawn_nip_fi_expiry_task(
            far_future,
            Arc::clone(&gate),
            terminal_tx,
            NipFiWsRoute::Root,
            control,
        );

        // Simulate admin disconnect: cancel the connection token.
        cancel.cancel();

        // Task is now in its cancellation arm, blocked in quiesce() awaiting
        // the write guard (which permit holds as a read guard).
        //
        // task_handle.is_finished() is cheap — just check the JoinHandle.
        // Yield several times to let the async runtime run the task.
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }

        assert!(
            !task_handle.is_finished(),
            "W_f5: expiry task must not finish while a permit is still held \
             (quiesce() must block on the write guard)"
        );

        // Drop the permit — quiesce() can now acquire the write guard.
        drop(permit);

        // Task should complete shortly after the permit is released.
        tokio::time::timeout(std::time::Duration::from_secs(2), task_handle)
            .await
            .expect("W_f5: expiry task must complete within 2s after permit is released")
            .expect("W_f5: expiry task must not panic");

        // The cancellation arm must NOT enqueue a second terminal frame —
        // the denial was already enqueued by the caller that cancelled the token.
        assert!(
            terminal_rx.try_recv().is_err(),
            "W_f5: cancellation arm must not enqueue a terminal frame \
             (deny was already queued by the admin-cancel path; quiesce only waits, \
             never enqueues)"
        );
    }

    // ── W_f5_registry: production-bound admin-disconnect path ────────────────
    //
    // Proves the full F5 invariant using the real admin-disconnect path:
    //   `CommunityConnectionRegistry::disconnect_nip_fi(&pubkey)` (the actual call
    //   made by `AppState::disconnect_nip_fi` during admin-driven session removal).
    //
    // This witness goes through the production code path — not raw `cancel.cancel()`.
    // `disconnect_nip_fi` on the registry:
    //   1. Iterates connections, finds the one with the matching proven pubkey.
    //   2. Calls `CommunityConnectionControl::disconnect_nip_fi` → acquires the
    //      transition lock, wins the reason slot, enqueues the denial frame on
    //      the terminal channel, releases the lock, then calls `cancel.cancel()`.
    //   3. The expiry task's cancellation arm fires, calls `gate.quiesce()`, blocks
    //      until the held permit is released (proving no orphan-effects escape).
    //
    // Proved by this witness:
    //   - Exactly one denial frame is enqueued (by the admin path, not by quiesce).
    //   - The task's cancellation arm does NOT produce a second terminal frame.
    //   - The task waits for an outstanding effect permit before completing,
    //     verified via an explicit bounded rendezvous (quiesce_cancel_arm_hook)
    //     instead of a timing window.
    //
    // Mutation evidence:
    //   A) Remove `set_terminal_frame_sender` call → terminal_frame_tx slot is None →
    //      disconnect_nip_fi enqueues nothing → terminal_rx is empty → first
    //      `try_recv` assertion panics.
    //   B) Replace `gate.quiesce().await` with `gate.expire(|| {...}).await` in the
    //      cancel arm → `expire()` enqueues a second terminal frame → `try_recv`
    //      after task completion sees a frame → "must be empty" assertion panics.
    //   C) Remove `gate.quiesce().await` from the cancel arm → task exits the cancel
    //      arm without waiting → `quiesce_cancel_arm_hook::fire()` is still called
    //      (it is before the quiesce call), so `arrived_rx` does NOT time out; but
    //      the task completes while the permit is still held → the `is_finished`
    //      assertion panics.
    #[tokio::test]
    async fn w_f5_registry_admin_disconnect_enqueues_frame_quiesces_no_dup() {
        use crate::state::{CommunityConnectionControl, CommunityConnectionRegistry};
        use buzz_core::tenant::CommunityId;
        use uuid::Uuid;

        let community = CommunityId::from_uuid(Uuid::new_v4());
        let target_pubkey = vec![0xf5u8; 32];

        let cancel = CancellationToken::new();
        let far_future = Utc::now() + chrono::Duration::hours(1);
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(far_future, cancel.clone());

        // Acquire a permit BEFORE the admin disconnect fires — simulates a REQ
        // handler that holds an effect permit when the admin cancel arrives.
        let permit = gate
            .acquire_effect()
            .await
            .expect("W_f5_registry: permit must be available before cancel");

        let (terminal_tx, mut terminal_rx) = mpsc::channel::<WsMessage>(1);
        let control = CommunityConnectionControl::new(cancel.clone());

        // Register the proven pubkey so registry.disconnect_nip_fi can find this
        // control by pubkey (same as audio_post_auth_register does in production).
        control.set_proven_identity(target_pubkey.clone(), Some("test-issuer".to_owned()));

        // Register the terminal sender so disconnect_nip_fi can enqueue the denial
        // frame on the correct channel (same as set_terminal_frame_sender does in
        // handle_active_audio_connection before the first check_cancel!).
        control.set_terminal_frame_sender(terminal_tx.clone());

        // Register the control in the registry.
        let registry = Arc::new(CommunityConnectionRegistry::new());
        let _guard = registry.register(Uuid::new_v4(), community, control.clone());

        // Arm the quiesce-entry hook BEFORE spawning the task so the signal is
        // ready when the cancel arm fires.
        let quiesce_arrived_rx = quiesce_cancel_arm_hook::arm(control.hook_key);

        let task_handle = spawn_nip_fi_expiry_task(
            far_future,
            Arc::clone(&gate),
            terminal_tx,
            NipFiWsRoute::Root,
            control.clone(),
        );

        // ── Admin disconnect: call the real production path. ──────────────────
        // `CommunityConnectionRegistry::disconnect_nip_fi` finds the control by
        // pubkey, calls `CommunityConnectionControl::disconnect_nip_fi` which
        // enqueues the denial frame on `terminal_frame_tx`, publishes
        // AuthorizationDenied, then calls `cancel.cancel()`.
        let closed = registry.disconnect_nip_fi("test-issuer", &target_pubkey);
        assert_eq!(
            closed, 1,
            "W_f5_registry: registry scan must find exactly 1 connection \
             (proves set_proven_pubkey ran before disconnect_nip_fi)"
        );
        assert!(
            cancel.is_cancelled(),
            "W_f5_registry: cancel must be set after disconnect_nip_fi"
        );

        // The denial frame must be present immediately (enqueued by the admin path
        // before cancel.cancel() fires — the transition lock guarantees this).
        let frame = terminal_rx.try_recv().expect(
            "W_f5_registry: denial frame must be in terminal channel immediately after \
             registry.disconnect_nip_fi (admin path enqueues before cancel). \
             Mutation A: remove set_terminal_frame_sender → frame absent → this panics.",
        );
        let expected = authorization_denied_frame(NipFiWsRoute::Audio);
        assert_eq!(
            frame, expected,
            "W_f5_registry: queued frame must be the canonical Audio authorization-denied frame \
             (CommunityConnectionControl::disconnect_nip_fi hard-codes NipFiWsRoute::Audio)"
        );

        // ── Explicit bounded rendezvous: task has entered the cancel arm. ─────
        // Await the quiesce-entry hook signal — this proves the task reached the
        // cancel arm and is about to call quiesce().  The permit is still held,
        // so quiesce() will block; the task must not be finished yet.
        //
        // Mutation C: remove `gate.quiesce().await` from the cancel arm → task
        // returns immediately after the hook fires → task IS finished before
        // `is_finished` check → assertion panics.
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            quiesce_arrived_rx,
        )
        .await
        .expect(
            "W_f5_registry: quiesce-entry hook must fire within 2s of cancel \
             (proves task entered cancel arm and reached quiesce() call site). \
             Mutation C: remove gate.quiesce() → hook fires but task finishes → is_finished panics.",
        )
        .expect("W_f5_registry: quiesce-entry hook sender must not be dropped");

        assert!(
            !task_handle.is_finished(),
            "W_f5_registry: expiry task must not finish while a permit is still held \
             (quiesce() must block on the write guard). \
             Mutation C: remove quiesce() from cancel arm → task finishes early → this panics."
        );
        // Cleanup: hook already consumed (oneshot); nothing to disarm.

        // Release the permit — quiesce() can now acquire the write guard.
        drop(permit);

        tokio::time::timeout(std::time::Duration::from_secs(2), task_handle)
            .await
            .expect("W_f5_registry: expiry task must complete within 2s after permit released")
            .expect("W_f5_registry: expiry task must not panic");

        // The cancellation arm must NOT produce a second terminal frame —
        // quiesce() only waits, never enqueues.
        // Note: replacing quiesce() with expire() MAY or MAY NOT produce a second
        // frame depending on whether the admin path has already won the reason slot
        // (expire() is winner-only: if the reason is already set by the admin path
        // it skips enqueue).  The assertion here proves that quiesce() on its own
        // does not enqueue — it makes no claim about expire() behaviour.
        assert!(
            terminal_rx.try_recv().is_err(),
            "W_f5_registry: terminal channel must be empty after task completion \
             (quiesce does not enqueue — proved by admin path having already won \
             the reason slot before quiesce ran, and quiesce() containing no \
             enqueue call of its own)"
        );
    }

    // ── W_f5_cleanup: sub_registry has zero orphan entries after teardown ─────
    //
    // Proves the complete F5 invariant through the subscription-registry cleanup
    // seam: quiescence ensures that a REQ handler which acquired a permit
    // BEFORE the admin cancel fires can still register its subscription in
    // `sub_registry` WHILE the expiry task is blocked in quiesce().  Only after
    // the permit is dropped (handler done) does quiesce() finish and the task
    // return — giving production's `remove_connection` (which runs after
    // `task.await` in connection.rs) the full subscription set to clean up.
    //
    // Without quiesce():
    //   cancel fires → task exits immediately → remove_connection runs with zero
    //   entries → REQ handler then registers its subscription → orphan entry persists
    //   forever (no cleanup runs after teardown).
    //
    // With quiesce():
    //   cancel fires → task blocks on write guard → REQ handler finishes and
    //   registers its subscription → drops permit → quiesce completes → task exits
    //   → remove_connection runs and finds the entry → zero orphans.
    //
    // This test models the full sequence: permit held before cancel, registration
    // happens while quiesce is blocked, cleanup is complete and zero orphans remain.
    //
    // Mutation evidence:
    //   A) Remove `gate.quiesce().await` from the cancel arm → task exits before
    //      `sub_registry.register_scoped` runs → `remove_connection` finds nothing
    //      → orphan: the subscription registered AFTER teardown is never removed
    //      → the "removed count" assertion (zero orphans in registry after cleanup)
    //      would still pass, but the registration-before-cleanup seam is broken;
    //      the test covers this via hook ordering: quiesce hook fires, registration
    //      runs, permit drops, task finishes — this sequence is impossible without
    //      quiesce blocking.
    //   B) Move the registration to AFTER `drop(permit)` → registration happens
    //      after quiesce, not while it is blocked → not the in-flight-REQ scenario
    //      → the test models the wrong schedule; here registration is inside the
    //      permit window (lines ordered: arm hook → permit held → disconnect →
    //      await hook → register → drop permit).
    #[tokio::test]
    async fn w_f5_cleanup_sub_registry_zero_orphans_after_disconnect() {
        use crate::state::{CommunityConnectionControl, CommunityConnectionRegistry};
        use crate::subscription::SubscriptionRegistry;
        use buzz_core::tenant::CommunityId;
        use nostr::Filter;
        use uuid::Uuid;

        let community = CommunityId::from_uuid(Uuid::new_v4());
        let target_pubkey = vec![0xf5u8; 32];
        let conn_id = Uuid::new_v4();

        let cancel = CancellationToken::new();
        let far_future = Utc::now() + chrono::Duration::hours(1);
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(far_future, cancel.clone());

        // ── Sub-registry (shared with the REQ handler simulation). ───────────
        let sub_registry = Arc::new(SubscriptionRegistry::new());

        // ── Effect permit: acquired BEFORE admin disconnect fires. ─────────
        // This represents a REQ handler that is mid-flight when the admin cancel
        // arrives — it holds a permit and will register a subscription before
        // dropping it, exactly the scenario quiesce() must protect.
        let permit = gate
            .acquire_effect()
            .await
            .expect("W_f5_cleanup: permit must be available before cancel");

        let (terminal_tx, _terminal_rx) = mpsc::channel::<WsMessage>(1);
        let control = CommunityConnectionControl::new(cancel.clone());
        control.set_proven_identity(target_pubkey.clone(), Some("test-issuer".to_owned()));
        control.set_terminal_frame_sender(terminal_tx.clone());

        let registry = Arc::new(CommunityConnectionRegistry::new());
        let _guard = registry.register(Uuid::new_v4(), community, control.clone());

        // Arm the quiesce-entry hook before spawning.
        let quiesce_arrived_rx = quiesce_cancel_arm_hook::arm(control.hook_key);

        let task_handle = spawn_nip_fi_expiry_task(
            far_future,
            Arc::clone(&gate),
            terminal_tx,
            NipFiWsRoute::Root,
            control.clone(),
        );

        // ── Admin disconnect via the real production path. ────────────────────
        let closed = registry.disconnect_nip_fi("test-issuer", &target_pubkey);
        assert_eq!(
            closed, 1,
            "W_f5_cleanup: registry must find exactly 1 connection"
        );

        // ── Wait for the task to enter the cancel arm (quiesce blocked). ─────
        tokio::time::timeout(std::time::Duration::from_secs(2), quiesce_arrived_rx)
            .await
            .expect("W_f5_cleanup: quiesce-entry hook must fire within 2s")
            .expect("W_f5_cleanup: quiesce-entry hook sender must not be dropped");

        // Task is blocked in quiesce() — permit is still held.
        assert!(
            !task_handle.is_finished(),
            "W_f5_cleanup: expiry task must be blocked in quiesce() while permit is held"
        );

        // ── Simulate in-flight REQ registration while quiesce is blocked. ─────
        // This is the critical sequence: the permit holder finishes its bounded
        // work (subscription registration) WHILE the expiry task is waiting.
        // Without quiesce(), the task would have already exited and remove_connection
        // would have run — so this registration would become an orphan.
        let sub_id = "test-sub-f5".to_string();
        let filters = vec![Filter::new()];
        sub_registry.register_scoped(community, conn_id, sub_id, filters, None);

        // Verify the subscription is visible in the registry while quiesce blocks.
        assert_eq!(
            sub_registry.total_subscriptions(),
            1,
            "W_f5_cleanup: subscription must be registered while quiesce is blocked"
        );

        // ── Release the permit — quiesce() unblocks and the task completes. ──
        drop(permit);

        tokio::time::timeout(std::time::Duration::from_secs(2), task_handle)
            .await
            .expect("W_f5_cleanup: expiry task must complete within 2s after permit released")
            .expect("W_f5_cleanup: expiry task must not panic");

        // ── Production teardown simulation: remove_connection runs AFTER task. ─
        // In connection.rs: `let _ = nip_fi_expiry_task.await` (line ~628) then
        // `for removed in state.sub_registry.remove_connection(conn.conn_id)`.
        // Because quiesce guaranteed the REQ handler finished before the task
        // returned, remove_connection finds the registered subscription.
        let removed = sub_registry.remove_connection(conn_id);
        assert_eq!(
            removed.len(),
            1,
            "W_f5_cleanup: remove_connection must find exactly 1 subscription — \
             the one registered while quiesce was blocked.  Zero means the \
             registration happened AFTER teardown (orphan scenario — quiesce broken)."
        );

        // After cleanup: no subscriptions remain.
        assert_eq!(
            sub_registry.total_subscriptions(),
            0,
            "W_f5_cleanup: zero subscriptions must remain after remove_connection \
             (complete cleanup, no orphan entries)"
        );
    }

    // ── W_f5_teardown: production teardown sequence (connection.rs:627-631) ──
    //
    // Models the exact teardown sequence from `connection.rs:627-631`:
    //   1. `nip_fi_expiry_task.await` — task must complete only after all
    //      pre-cancel permit-holders finish (quiescence).
    //   2. `sub_registry.remove_connection(conn_id)` — zero orphan subscriptions.
    //
    // The previous W_f5_cleanup test establishes the invariant with a REQ
    // handler modelled by direct registry calls.  This test drives the same
    // sequence through the named production functions to make the connection
    // explicit: admin `registry.disconnect_nip_fi` → expiry task quiesces →
    // task completes → teardown removes subscriptions.
    //
    // ── Scope and known limits ─────────────────────────────────────────────
    //
    // This test exercises the quiescence invariant via the production components
    // (`CommunityConnectionRegistry`, `spawn_nip_fi_expiry_task`,
    // `registry.disconnect_nip_fi`) and the sub_registry primitives directly.
    // It deliberately does NOT run `handle_req` or `pubsub.release_topic`.
    //
    // The full production teardown (including `release_topic` per removed scope
    // and `conn_manager.deregister`) lives in `handle_active_connection`'s
    // epilogue (connection.rs:631–648).  That path is now covered by
    // `f5_loopback_teardown_epilogue_removes_subscription_and_topic_refcount`
    // in connection.rs, which drives a real `handle_active_connection` over
    // a loopback WebSocket — no TLS or external relay required.  This simulated
    // test retains complementary low-level coverage of the quiescence barrier
    // and sub_registry primitives; it is not the primary F5 witness.
    //
    // What this test verifies:
    //   - The quiescence barrier (`gate.quiesce()`) blocks the expiry task
    //     until the last pre-cancel permit-holder drops.
    //   - Admin disconnect via the real `CommunityConnectionRegistry` path
    //     enqueues exactly one terminal frame (Audio-format denial frame —
    //     `CommunityConnectionControl` always uses Audio JSON).
    //   - `sub_registry.remove_connection` finds and removes the subscription
    //     registered while quiescence was blocked — zero orphans.
    //
    // What is NOT verified here (requires live-socket integration test):
    //   - `pubsub.release_topic` called for each removed scope.
    //   - `conn_manager.deregister` called after task completion.
    //
    // Contract:
    //   - Exactly one terminal frame enqueued by the admin path (not by quiesce).
    //   - Task blocked until permit is released (quiescence proof).
    //   - Zero subscription orphans after teardown remove_connection.
    //
    // Mutation evidence:
    //   A) Remove `gate.quiesce().await` → task can exit before step 5's
    //      `register_scoped` in this test, but note: this test calls
    //      register_scoped unconditionally before remove_connection, so the
    //      ordering within the test is always correct.
    //      The mutation's real falsifiable claim is production-level: in the
    //      real handler a REQ arrives WHILE the expiry task is running; without
    //      quiescence the task exits before registration and the subscription
    //      orphans permanently.  This test proves quiescence BLOCKS the task
    //      until the permit is released — an insertion after permit-drop would
    //      be lost, but step 5 occurs while the task is blocked, so the remove
    //      finds exactly 1.  Remove quiesce → task_handle.is_finished() before
    //      step 5 → the is_finished assertion at step 4 panics.
    //   B) Replace admin `registry.disconnect_nip_fi` with raw `cancel.cancel()`
    //      → no terminal frame enqueued → first try_recv assertion panics.
    #[tokio::test]
    async fn w_f5_teardown_matches_connection_teardown_sequence() {
        use crate::state::{CommunityConnectionControl, CommunityConnectionRegistry};
        use crate::subscription::SubscriptionRegistry;
        use buzz_core::tenant::CommunityId;
        use nostr::Filter;
        use uuid::Uuid;

        let community = CommunityId::from_uuid(Uuid::new_v4());
        let conn_id = Uuid::new_v4();
        let target_pubkey = vec![0xf5u8; 32];

        let cancel = CancellationToken::new();
        let far_future = Utc::now() + chrono::Duration::hours(1);
        let gate = crate::nip_fi_gate::SessionAdmissionGate::new(far_future, cancel.clone());

        // ── Step 1: acquire effect permit before admin disconnect (simulates an
        //    in-flight REQ handler).  This is the window quiescence must protect.
        let permit = gate
            .acquire_effect()
            .await
            .expect("W_f5_teardown: permit must be available before cancel");

        // ── Step 2: wire up the production components.
        let (terminal_tx, mut terminal_rx) = mpsc::channel::<WsMessage>(1);
        let control = CommunityConnectionControl::new(cancel.clone());
        control.set_proven_identity(target_pubkey.clone(), Some("test-issuer".to_owned()));
        control.set_terminal_frame_sender(terminal_tx.clone());

        let registry = Arc::new(CommunityConnectionRegistry::new());
        let sub_registry = Arc::new(SubscriptionRegistry::new());
        let _guard = registry.register(Uuid::new_v4(), community, control.clone());

        // Arm quiesce hook to observe task entering the quiescence barrier.
        let quiesce_arrived_rx = quiesce_cancel_arm_hook::arm(control.hook_key);

        // Spawn the expiry task — mirrors `nip_fi_expiry_task` in connection.rs.
        let task_handle = spawn_nip_fi_expiry_task(
            far_future,
            Arc::clone(&gate),
            terminal_tx,
            NipFiWsRoute::Root,
            control.clone(),
        );

        // ── Step 3: admin disconnect via the real production path.
        //    This is `registry.disconnect_nip_fi("test-issuer", &pubkey)` — the call made by
        //    `AppState::disconnect_nip_fi` in the admin HTTP handler.
        let closed = registry.disconnect_nip_fi("test-issuer", &target_pubkey);
        assert_eq!(
            closed, 1,
            "W_f5_teardown: registry must find exactly 1 connection"
        );

        // Exactly one terminal frame enqueued by the admin path.
        let frame = terminal_rx
            .try_recv()
            .expect("W_f5_teardown: admin disconnect must enqueue exactly one terminal frame");
        // `CommunityConnectionRegistry::disconnect_nip_fi` delegates to
        // `CommunityConnectionControl::disconnect_nip_fi`, which always enqueues
        // the Audio-route denial frame (audio JSON, not NOTICE).  Root-path
        // connections use `ConnectionManager::disconnect_nip_fi` instead.
        let expected = crate::nip_fi_session::authorization_denied_frame(NipFiWsRoute::Audio);
        assert_eq!(
            frame, expected,
            "W_f5_teardown: terminal frame must be the canonical Audio denial frame \
             (CommunityConnectionRegistry path always sends Audio format)"
        );

        // ── Step 4: task is now in its cancel arm, blocked in quiesce().
        //    Wait for the task to enter quiescence (it holds the write guard).
        tokio::time::timeout(std::time::Duration::from_secs(2), quiesce_arrived_rx)
            .await
            .expect("W_f5_teardown: quiesce-entry hook must fire within 2s")
            .expect("W_f5_teardown: quiesce-entry hook sender must not be dropped");

        assert!(
            !task_handle.is_finished(),
            "W_f5_teardown: task must be blocked in quiesce() while permit is held"
        );

        // ── Step 5: simulate in-flight REQ handler registering a subscription
        //    WHILE quiesce is blocking (the production race quiescence protects).
        let sub_id = "test-sub-f5-teardown".to_string();
        let filters = vec![Filter::new()];
        sub_registry.register_scoped(community, conn_id, sub_id, filters, None);

        assert_eq!(
            sub_registry.total_subscriptions(),
            1,
            "W_f5_teardown: subscription must be registered while quiesce is blocked"
        );

        // ── Step 6: drop permit → quiesce() acquires write guard → task completes.
        drop(permit);

        // ── Step 7: await task — mirrors `task.await` in connection.rs:628.
        tokio::time::timeout(std::time::Duration::from_secs(2), task_handle)
            .await
            .expect("W_f5_teardown: task must complete within 2s after permit released")
            .expect("W_f5_teardown: task must not panic");

        // ── Step 8: remove_connection — mirrors connection.rs:631.
        let removed = sub_registry.remove_connection(conn_id);
        assert_eq!(
            removed.len(),
            1,
            "W_f5_teardown: remove_connection must find exactly 1 subscription — \
             the one registered in step 5 while quiesce was blocked. \
             (Note: in this test register_scoped is unconditionally called before \
             remove_connection; the quiescence proof is the task.is_finished() \
             check above, not this assertion)"
        );

        // Zero orphans remain — complete cleanup.
        assert_eq!(
            sub_registry.total_subscriptions(),
            0,
            "W_f5_teardown: zero subscriptions must remain after remove_connection"
        );

        // No second terminal frame — quiesce does not enqueue.
        assert!(
            terminal_rx.try_recv().is_err(),
            "W_f5_teardown: terminal channel must be empty after task completion \
             (quiesce does not enqueue; admin path already enqueued exactly one frame)"
        );
    }
}
