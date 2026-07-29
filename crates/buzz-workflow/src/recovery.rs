//! Crash-window classification for workflow recovery.
//!
//! The database lease and idempotency records are the source of truth. This
//! small, side-effect-free classifier documents the expected decision at each
//! failure boundary and keeps that policy executable in tests.

/// The point at which a worker stopped making progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashWindow {
    /// The event/run was persisted, but no worker claimed it.
    BeforeClaim,
    /// A lease was claimed, but the step attempt was not started.
    AfterClaimBeforeStep,
    /// A step attempt was persisted and the worker stopped during execution.
    DuringStep,
    /// The remote side effect may have succeeded, but its durable receipt was
    /// not written yet.
    AfterEffectBeforeReceipt,
    /// The run reached a terminal result, but finalization was interrupted.
    BeforeFinalize,
}

/// Durable facts available to the recovery decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryFacts {
    /// Whether the worker lease has expired (or the worker disappeared).
    pub lease_expired: bool,
    /// Whether a durable action-effect receipt exists.
    pub effect_receipt_exists: bool,
    /// Whether the run already has a terminal status.
    pub run_is_terminal: bool,
}

/// Recovery action selected by the worker after a crash/restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Claim or reclaim the run and continue from its durable step cursor.
    ReclaimAndResume,
    /// Resume the durable step attempt using its idempotency key.
    ResumeIdempotentStep,
    /// Do not repeat the remote effect; reconcile the existing receipt.
    ReconcileEffect,
    /// Leave the terminal result immutable.
    PreserveTerminal,
    /// Keep waiting for the current lease to expire.
    WaitForLeaseExpiry,
}

/// Classify a crash window from the records persisted by Buzz.
pub fn classify(window: CrashWindow, facts: RecoveryFacts) -> RecoveryAction {
    if facts.run_is_terminal {
        return RecoveryAction::PreserveTerminal;
    }
    if !facts.lease_expired {
        return RecoveryAction::WaitForLeaseExpiry;
    }

    match window {
        CrashWindow::BeforeClaim | CrashWindow::AfterClaimBeforeStep => {
            RecoveryAction::ReclaimAndResume
        }
        CrashWindow::DuringStep => RecoveryAction::ResumeIdempotentStep,
        CrashWindow::AfterEffectBeforeReceipt => {
            if facts.effect_receipt_exists {
                RecoveryAction::ReconcileEffect
            } else {
                // The action sink must reserve by idempotency key before
                // retrying; the sink decides whether the remote effect is
                // safely replayable or requires operator reconciliation.
                RecoveryAction::ResumeIdempotentStep
            }
        }
        CrashWindow::BeforeFinalize => RecoveryAction::ReclaimAndResume,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPIRED: RecoveryFacts = RecoveryFacts {
        lease_expired: true,
        effect_receipt_exists: false,
        run_is_terminal: false,
    };

    #[test]
    fn crash_windows_are_recoverable_after_lease_expiry() {
        let cases = [
            (CrashWindow::BeforeClaim, RecoveryAction::ReclaimAndResume),
            (
                CrashWindow::AfterClaimBeforeStep,
                RecoveryAction::ReclaimAndResume,
            ),
            (
                CrashWindow::DuringStep,
                RecoveryAction::ResumeIdempotentStep,
            ),
            (
                CrashWindow::AfterEffectBeforeReceipt,
                RecoveryAction::ResumeIdempotentStep,
            ),
            (
                CrashWindow::BeforeFinalize,
                RecoveryAction::ReclaimAndResume,
            ),
        ];
        for (window, expected) in cases {
            assert_eq!(classify(window, EXPIRED), expected);
        }
    }

    #[test]
    fn existing_effect_receipt_is_reconciled_without_replay() {
        let facts = RecoveryFacts {
            effect_receipt_exists: true,
            ..EXPIRED
        };
        assert_eq!(
            classify(CrashWindow::AfterEffectBeforeReceipt, facts),
            RecoveryAction::ReconcileEffect
        );
    }

    #[test]
    fn live_lease_is_never_stolen() {
        assert_eq!(
            classify(
                CrashWindow::DuringStep,
                RecoveryFacts {
                    lease_expired: false,
                    ..EXPIRED
                }
            ),
            RecoveryAction::WaitForLeaseExpiry
        );
    }

    #[test]
    fn terminal_runs_are_immutable_even_if_replayed() {
        assert_eq!(
            classify(
                CrashWindow::BeforeFinalize,
                RecoveryFacts {
                    run_is_terminal: true,
                    ..EXPIRED
                }
            ),
            RecoveryAction::PreserveTerminal
        );
    }
}
