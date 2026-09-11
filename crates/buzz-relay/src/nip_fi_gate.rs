//! `SessionAdmissionGate` — per-connection lifetime authority for NIP-FI sessions.
//!
//! Every WS connection that carries a NIP-FI assertion gets one gate. The gate
//! owns three orthogonal concerns:
//!
//! * **Effect permit**: any handler that performs an irreversible side effect
//!   (AUTH state commit, EVENT persistence, REQ subscription registration,
//!   COUNT query, `48101` commit) must acquire a [`SessionEffectPermit`] before
//!   the first irreversible operation. The permit is a `Tokio` fair read lock
//!   guard — expiry cannot start until all pre-expiry permits are dropped.
//!
//! * **Expiry**: at the session deadline, [`SessionAdmissionGate::expire`]
//!   queues the terminal denial frame, cancels the socket immediately, then
//!   acquires the write guard to record [`SessionPhase::Expired`]. Acquiring the
//!   write guard blocks until all outstanding read guards (live effect permits)
//!   are dropped, making the lock a quiescence barrier: post-expiry teardown
//!   (subscription removal, peer cleanup) cannot start until all permitted
//!   effects have finished.
//!
//! * **Deadline check**: `acquire_effect` checks cancellation AND the wall-clock
//!   deadline *under* the read guard, so a permit can never be obtained after
//!   expiry has been queued or the deadline has passed.
//!
//! ## Ordering guarantees
//!
//! ```text
//! expire()  : terminal()       → cancel.cancel()   → write guard → Expired
//! acquire() : obtain read guard → check cancel/deadline → Ok(permit) or Err
//! ```
//!
//! An effect holding a permit before `cancel.cancel()` fires **wins**: the
//! permit prevents the write guard, and the effect may complete its bounded
//! commit/fan-out. An effect that cannot obtain a permit after `cancel.cancel()`
//! **loses**: the cancel check inside the read guard fails, and the effect is
//! rejected before any side effect occurs.
//!
//! ## Off-mode
//!
//! When `deadline` is `None`, `acquire_effect` always succeeds (no cancel is ever
//! issued by the gate itself, and `None` deadline is treated as infinite). The
//! gate has zero overhead in off-mode: one arc read per effect acquire.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use tokio::sync::{OwnedRwLockReadGuard, RwLock};
use tokio_util::sync::CancellationToken;

// ── Phase ─────────────────────────────────────────────────────────────────────

/// Connection phase from the gate's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionPhase {
    Active,
    Expired,
}

// ── Permit ────────────────────────────────────────────────────────────────────

/// A live effect permit. While this value is held, expiry cannot transition to
/// `Expired` — the read lock prevents the write guard in `expire()`.
///
/// Drop the permit as soon as the effect's irreversible work is done. Holding it
/// across long-lived awaits that are not part of the bounded effect is incorrect.
#[must_use = "effect permit must be held through the bounded effect and then dropped"]
#[cfg_attr(test, derive(Debug))]
pub(crate) struct SessionEffectPermit {
    /// Holds the Tokio read lock, keeping expiry from transitioning until drop.
    _guard: OwnedRwLockReadGuard<SessionPhase>,
}

// ── Error ─────────────────────────────────────────────────────────────────────

/// Returned by [`SessionAdmissionGate::acquire_effect`] when the session has
/// already expired or the deadline has passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionExpired;

// ── Gate ─────────────────────────────────────────────────────────────────────

/// Per-connection session lifetime authority.
///
/// Create one per WS connection via [`SessionAdmissionGate::new`] (with a
/// deadline) or [`SessionAdmissionGate::off_mode`] (no deadline, never expires
/// on its own). Root and audio connections use the same type.
#[derive(Debug)]
pub(crate) struct SessionAdmissionGate {
    /// UTC deadline after which new effect permits are rejected.
    ///
    /// `None` means off-mode: no deadline, gate never self-expires.
    pub deadline: Option<DateTime<Utc>>,
    phase: Arc<RwLock<SessionPhase>>,
    cancel: CancellationToken,
}

impl SessionAdmissionGate {
    /// Create a gate with the given deadline.
    pub(crate) fn new(deadline: DateTime<Utc>, cancel: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            deadline: Some(deadline),
            phase: Arc::new(RwLock::new(SessionPhase::Active)),
            cancel,
        })
    }

    /// Create an off-mode gate (no deadline, never self-expires).
    #[allow(dead_code)] // used in nip_fi_gate unit tests and forthcoming B1/B2 witnesses
    pub(crate) fn off_mode(cancel: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            deadline: None,
            phase: Arc::new(RwLock::new(SessionPhase::Active)),
            cancel,
        })
    }

    /// Acquire an effect permit.
    ///
    /// Obtains the fair Tokio read lock, then checks:
    /// 1. `cancel.is_cancelled()` — expiry has already been queued.
    /// 2. `deadline` is past (equality is expired).
    ///
    /// Returns `Ok(SessionEffectPermit)` only when both checks pass.
    /// Returns `Err(SessionExpired)` otherwise, without performing any side effect.
    pub(crate) async fn acquire_effect(
        self: &Arc<Self>,
    ) -> Result<SessionEffectPermit, SessionExpired> {
        // Obtain the fair read lock. This blocks if expiry holds the write guard
        // (quiescence window) but that is bounded — expire() holds the write guard
        // only long enough to set the phase field.
        let guard = Arc::clone(&self.phase).read_owned().await;

        // Check cancellation and deadline under the read guard. Once we hold the
        // guard, expiry cannot transition until we release it. A cancelled token
        // or a past deadline means expiry has already been queued (or is guaranteed
        // to fire before any new socket I/O completes).
        if self.cancel.is_cancelled() {
            return Err(SessionExpired);
        }
        if let Some(deadline) = self.deadline {
            // Equality is expired per spec [FI-TRACE-LEASE-BOUND].
            if Utc::now() >= deadline {
                return Err(SessionExpired);
            }
        }

        Ok(SessionEffectPermit { _guard: guard })
    }

    /// Returns a future that resolves when the gate's cancellation token fires.
    ///
    /// Use in `tokio::select!` to exit early when the connection closes from
    /// outside the expiry path (e.g., the client disconnects before the deadline).
    pub(crate) fn cancelled(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
        self.cancel.cancelled()
    }

    /// Cheaply test whether the session is expired or past its deadline.
    ///
    /// This is a **defense-in-depth** check at dispatch time, not a substitute
    /// for acquiring a permit. Handler permits are authoritative; this check
    /// merely avoids spawning obviously-dead work.
    #[allow(dead_code)] // used in nip_fi_gate unit tests and forthcoming B1/B2 witnesses
    pub(crate) fn is_expired_or_past_deadline(&self) -> bool {
        if self.cancel.is_cancelled() {
            return true;
        }
        if let Some(deadline) = self.deadline {
            if Utc::now() >= deadline {
                return true;
            }
        }
        false
    }

    /// Expire the session.
    ///
    /// Ordering (per contract):
    /// 1. Call `terminal()` — queues the denial frame before any lock is held.
    ///    Socket cancellation starts immediately; the send loop delivers the
    ///    terminal frame and then `Close`.
    /// 2. Call `cancel.cancel()` — socket termination starts at the deadline;
    ///    never waits for any permit.
    /// 3. Acquire the write guard — blocks until all outstanding read guards
    ///    (live effect permits) are dropped. This is the **quiescence barrier**:
    ///    teardown cannot start until all pre-expiry effects have finished their
    ///    bounded commits.
    /// 4. Record `SessionPhase::Expired`.
    /// 5. Release the write guard — the expiry task's `await` on this call
    ///    completes, and the task returns. Connection teardown (which awaits the
    ///    expiry task handle) then proceeds.
    ///
    /// `terminal` is called exactly once, before any lock is held, so it cannot
    /// deadlock and cannot be delayed by in-flight permits.
    pub(crate) async fn expire(&self, terminal: impl FnOnce()) {
        // Step 1: queue the denial frame (terminal delivery, no lock held).
        terminal();
        // Step 2: cancel the socket immediately — never waits for a permit.
        self.cancel.cancel();
        // Steps 3–5: quiescence barrier.
        let mut phase = self.phase.write().await;
        *phase = SessionPhase::Expired;
        // Write guard released here on drop — expiry task's await completes.
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn gate_with_far_deadline() -> Arc<SessionAdmissionGate> {
        let cancel = CancellationToken::new();
        let deadline = Utc::now() + chrono::Duration::hours(1);
        SessionAdmissionGate::new(deadline, cancel)
    }

    // ── acquire_effect passes in normal operation ──────────────────────────────

    #[tokio::test]
    async fn acquire_effect_succeeds_when_active_and_within_deadline() {
        let gate = gate_with_far_deadline();
        let permit = gate.acquire_effect().await;
        assert!(
            permit.is_ok(),
            "acquire_effect must succeed when gate is active and deadline is in the future"
        );
    }

    // ── cancel causes acquire_effect to fail ──────────────────────────────────

    #[tokio::test]
    async fn acquire_effect_fails_after_cancel() {
        let cancel = CancellationToken::new();
        let gate =
            SessionAdmissionGate::new(Utc::now() + chrono::Duration::hours(1), cancel.clone());
        cancel.cancel();
        let result = gate.acquire_effect().await;
        assert!(
            matches!(result, Err(SessionExpired)),
            "acquire_effect must return Err(SessionExpired) after cancel"
        );
    }

    // ── past deadline causes acquire_effect to fail ──────────────────────────

    #[tokio::test]
    async fn acquire_effect_fails_when_past_deadline() {
        let cancel = CancellationToken::new();
        let past = Utc::now() - chrono::Duration::seconds(1);
        let gate = SessionAdmissionGate::new(past, cancel);
        let result = gate.acquire_effect().await;
        assert!(
            matches!(result, Err(SessionExpired)),
            "acquire_effect must return Err(SessionExpired) when deadline has passed"
        );
    }

    // ── off-mode gate never self-cancels ──────────────────────────────────────

    #[tokio::test]
    async fn off_mode_gate_always_succeeds() {
        let cancel = CancellationToken::new();
        let gate = SessionAdmissionGate::off_mode(cancel);
        let permit = gate.acquire_effect().await;
        assert!(
            permit.is_ok(),
            "off-mode gate must always grant permits when cancel has not fired"
        );
    }

    // ── expire() ordering: terminal fires before cancel, write guard acquired after ──

    #[tokio::test]
    async fn expire_calls_terminal_then_cancels_then_quiesces() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;

        let cancel = CancellationToken::new();
        let gate =
            SessionAdmissionGate::new(Utc::now() + chrono::Duration::hours(1), cancel.clone());

        let sequence = StdArc::new(AtomicUsize::new(0));

        // Hold a permit — expire() must block on the write guard until we drop it.
        let permit = gate.acquire_effect().await.expect("permit before expiry");

        let gate2 = Arc::clone(&gate);
        let seq2 = StdArc::clone(&sequence);
        let seq3 = StdArc::clone(&sequence);
        let expire_task = tokio::spawn(async move {
            gate2
                .expire(|| {
                    // terminal() fires before cancel.cancel() and before write guard.
                    seq2.fetch_add(1, Ordering::SeqCst); // step 1
                })
                .await;
            seq3.fetch_add(10, Ordering::SeqCst); // step 3 (after write guard released)
        });

        // Yield so expire_task can start and reach the write guard wait.
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }

        // expire_task should have called terminal() (seq += 1) and cancel.cancel()
        // but be blocked on the write guard (seq should be 1, not 11).
        assert!(
            cancel.is_cancelled(),
            "cancel must fire before the write guard is acquired"
        );
        let seq_before_drop = sequence.load(Ordering::SeqCst);
        assert_eq!(
            seq_before_drop, 1,
            "terminal() must have run (seq=1) but write guard must not yet be released (seq<11)"
        );

        // Drop the permit — expire_task can now obtain the write guard.
        drop(permit);

        tokio::time::timeout(std::time::Duration::from_secs(2), expire_task)
            .await
            .expect("expire must complete within timeout")
            .expect("expire task must not panic");

        assert_eq!(
            sequence.load(Ordering::SeqCst),
            11,
            "expire must complete fully after permit is dropped (seq = 1 + 10 = 11)"
        );

        // After expiry, no new permit can be obtained.
        let post_expire = gate.acquire_effect().await;
        assert!(
            matches!(post_expire, Err(SessionExpired)),
            "acquire_effect must fail after expire() completes"
        );
    }

    // ── is_expired_or_past_deadline ───────────────────────────────────────────

    #[tokio::test]
    async fn is_expired_false_when_active() {
        let gate = gate_with_far_deadline();
        assert!(
            !gate.is_expired_or_past_deadline(),
            "active gate must not report expired"
        );
    }

    #[tokio::test]
    async fn is_expired_true_after_cancel() {
        let cancel = CancellationToken::new();
        let gate =
            SessionAdmissionGate::new(Utc::now() + chrono::Duration::hours(1), cancel.clone());
        cancel.cancel();
        assert!(
            gate.is_expired_or_past_deadline(),
            "cancelled gate must report expired"
        );
    }
}
