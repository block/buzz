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
            *conn.auth_state.write().await = crate::connection::AuthState::Failed;
            // Publish reason first-writer-wins: set AuthorizationDenied only
            // when the slot is still None — a concurrent CommunityDeleted must
            // not be clobbered, and vice versa. [FI-TRACE-CLOSE-CODE]
            let _ = conn
                .nip_fi_reason_tx
                .send_if_modified(|current| match current {
                    None => {
                        *current =
                            Some(crate::state::CommunityDisconnectReason::AuthorizationDenied);
                        true
                    }
                    Some(_) => false,
                });
            // Use the dedicated terminal channel — guaranteed one free slot even
            // when ctrl_tx (capacity 8) is saturated by ordinary control traffic.
            let _ = conn
                .terminal_ctrl_tx
                .try_send(authorization_denied_frame(NipFiWsRoute::Root));
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
            let _ = ws_send
                .send(authorization_denied_frame(NipFiWsRoute::Audio))
                .await;
            // Send explicit 1008 POLICY close frame before dropping. The audio
            // handler owns ws_send directly here (send_loop not yet started).
            // [FI-TRACE-CLOSE-CODE]
            let _ = ws_send
                .send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                    code: axum::extract::ws::close_code::POLICY,
                    reason: axum::extract::ws::Utf8Bytes::from_static("authorization denied"),
                })))
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
    deny_reason_tx: tokio::sync::watch::Sender<Option<crate::state::CommunityDisconnectReason>>,
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
                    // Publish reason first-writer-wins so the send loop's
                    // cancel branch reads AuthorizationDenied and emits 1008;
                    // a concurrent CommunityDeleted must not be clobbered.
                    // [FI-TRACE-CLOSE-CODE]
                    let _ = deny_reason_tx.send_if_modified(|current| match current {
                        None => {
                            *current = Some(
                                crate::state::CommunityDisconnectReason::AuthorizationDenied,
                            );
                            true
                        }
                        Some(_) => false,
                    });
                    let _ = terminal_ctrl_tx.try_send(authorization_denied_frame(route));
                    metrics::counter!("buzz_nip_fi_lease_expirations_total").increment(1);
                    warn!(
                        route = ?route,
                        "NIP-FI session lease expired — closing connection"
                    );
                })
                .await;
            }
            _ = gate.cancelled() => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use nostr::Keys;
    use std::sync::Arc;
    use tokio::sync::{mpsc, RwLock};
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

        let conn = Arc::new(crate::connection::ConnectionState {
            conn_id: Uuid::new_v4(),
            tenant: buzz_core::tenant::TenantContext::resolved(
                buzz_core::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(crate::connection::AuthState::Pending {
                challenge: "test-challenge".to_string(),
            }),
            subscriptions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            send_tx,
            ctrl_tx,
            terminal_ctrl_tx,
            cancel: CancellationToken::new(),
            backpressure_count: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            grace_limit: 3,
            nip_fi_assertion: Some(assertion),
            session_deadline: None,
            nip_fi_gate: crate::nip_fi_gate::SessionAdmissionGate::off_mode(
                CancellationToken::new(),
            ),
            nip_fi_reason_tx: tokio::sync::watch::channel(None).0,
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
        let handle = spawn_nip_fi_expiry_task(
            already_expired,
            gate,
            terminal_tx,
            NipFiWsRoute::Root,
            tokio::sync::watch::channel(None).0,
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
}
