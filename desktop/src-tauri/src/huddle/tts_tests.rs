//! Tests for huddle/tts.rs — split into a sibling file to keep tts.rs
//! focused. These exercise the pure helpers and the lifecycle contracts:
//! remote-interrupt frame counting, cancel consumption, the worker idle
//! branch, buffer building, fades, and playback gain.

use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

// ── Remote interrupt tracker ──────────────────────────────────────────────
//
// Models the per-peer frame counting logic in the recv task of
// relay_api.rs. The contract (must match the production implementation):
//
//   - Frame counting is GATED on tts_active — counters only increment
//     while TTS is playing.
//   - On TTS session start (false→true transition), all counters and
//     the window timer are reset. Prevents stale pre-playback speech
//     from tripping a cancel.
//   - In production, counting happens after successful Opus decode
//     (Ok(n) if n > 0). We can't model decode in unit tests, so
//     on_frame() represents a successfully-decoded audio frame.
//   - Each remote peer has an independent frame counter.
//   - Counters use saturating_add (overflow-safe).
//   - When a peer's counter crosses REMOTE_SPEECH_THRESHOLD, set
//     tts_cancel = true.
//   - Counters reset on the 500ms window (Instant-based in production,
//     on_tick() in tests — logically equivalent).
//   - Uses Acquire for tts_active reads, Release for tts_cancel writes.
//
use crate::huddle::relay_api::REMOTE_SPEECH_THRESHOLD;

/// Test-side model of the per-peer frame counting logic in the recv task.
struct RemoteInterruptTracker {
    frame_counts: HashMap<u8, u16>,
    tts_active: Arc<AtomicBool>,
    tts_cancel: Arc<AtomicBool>,
    /// Tracks the previous tts_active state to detect false→true transitions.
    /// Mirrors `tts_was_active` in the production recv task.
    tts_was_active: bool,
}

impl RemoteInterruptTracker {
    fn new(tts_active: Arc<AtomicBool>, tts_cancel: Arc<AtomicBool>) -> Self {
        Self {
            frame_counts: HashMap::new(),
            tts_active,
            tts_cancel,
            tts_was_active: false,
        }
    }

    /// Called when a successfully-decoded audio frame arrives from a
    /// remote peer. Mirrors the production logic in relay_api.rs:
    ///   1. Check tts_active — skip if inactive
    ///   2. On false→true transition, clear all counters (new TTS session)
    ///   3. Increment peer counter (saturating)
    ///   4. Fire cancel if threshold crossed
    fn on_frame(&mut self, peer_idx: u8) {
        let tts_now = self.tts_active.load(Ordering::Acquire);

        // Detect TTS session start — clear stale counters.
        if tts_now && !self.tts_was_active {
            self.frame_counts.clear();
        }
        self.tts_was_active = tts_now;

        if !tts_now {
            return; // Not counting while TTS is inactive.
        }

        let count = self.frame_counts.entry(peer_idx).or_insert(0);
        *count = count.saturating_add(1);
        if *count >= REMOTE_SPEECH_THRESHOLD {
            self.tts_cancel.store(true, Ordering::Release);
        }
    }

    /// Called on the 500ms window boundary — resets all frame counters.
    /// In production this is Instant-based (starvation-proof); in tests
    /// we call it explicitly since there's no async event loop.
    fn on_tick(&mut self) {
        self.frame_counts.clear();
    }

    fn count_for(&self, peer_idx: u8) -> u16 {
        *self.frame_counts.get(&peer_idx).unwrap_or(&0)
    }
}

/// Simulate the TTS worker's cancel-handling logic (from handle_cancel_or_shutdown).
/// Returns true if cancel was processed (mirrors the real function's return value).
fn simulate_cancel_consumption(
    cancel: &AtomicBool,
    shutdown: &AtomicBool,
    tts_active: &AtomicBool,
    text_rx: &mpsc::Receiver<String>,
) -> bool {
    if shutdown.load(Ordering::Acquire) {
        tts_active.store(false, Ordering::Release);
        return true;
    }
    if cancel.load(Ordering::Acquire) {
        while text_rx.try_recv().is_ok() {}
        cancel.store(false, Ordering::Release);
        tts_active.store(false, Ordering::Release);
        return true;
    }
    false
}

/// Simulate the drained-player check in the TTS worker's main loop. It
/// runs in TWO places — the recv-timeout (idle) arm, and on item receipt
/// before synthesis begins: `tts_active` is released and the lead-in
/// re-armed only when the player has fully drained AND audio was actually
/// queued since the last idle period. Must match the production logic in
/// `tts_worker`.
fn simulate_idle_check(player_empty: bool, first_append: &mut bool, tts_active: &AtomicBool) {
    if player_empty && !*first_append {
        tts_active.store(false, Ordering::Release);
        *first_append = true;
    }
}

// ── Threshold tests ───────────────────────────────────────────────────────

/// Real speech above threshold during TTS → cancel fires.
#[test]
fn speech_above_threshold_sets_cancel() {
    let tts_active = Arc::new(AtomicBool::new(true));
    let tts_cancel = Arc::new(AtomicBool::new(false));
    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    // Send exactly REMOTE_SPEECH_THRESHOLD frames from peer 1.
    for _ in 0..REMOTE_SPEECH_THRESHOLD {
        tracker.on_frame(1);
    }

    assert!(
        tts_cancel.load(Ordering::Acquire),
        "tts_cancel should be true after {} frames from a peer during active TTS",
        REMOTE_SPEECH_THRESHOLD,
    );
}

/// DTX comfort noise below threshold during TTS → cancel does NOT fire.
#[test]
fn comfort_noise_below_threshold_no_cancel() {
    let tts_active = Arc::new(AtomicBool::new(true));
    let tts_cancel = Arc::new(AtomicBool::new(false));
    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    // DTX comfort noise: ~2-3 frames per 500ms tick. Send fewer than threshold.
    for _ in 0..(REMOTE_SPEECH_THRESHOLD - 1) {
        tracker.on_frame(1);
    }

    assert!(
        !tts_cancel.load(Ordering::Acquire),
        "tts_cancel should remain false with only {} frames (below threshold of {})",
        REMOTE_SPEECH_THRESHOLD - 1,
        REMOTE_SPEECH_THRESHOLD,
    );
}

/// Frames arrive while tts_active=false → no cancel regardless of count.
#[test]
fn frames_without_tts_active_no_cancel() {
    let tts_active = Arc::new(AtomicBool::new(false));
    let tts_cancel = Arc::new(AtomicBool::new(false));
    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    // Send many frames — well above threshold.
    for _ in 0..50 {
        tracker.on_frame(1);
    }

    assert!(
        !tts_cancel.load(Ordering::Acquire),
        "tts_cancel should remain false when TTS is not active",
    );
}

/// Counter reset on tick → peer must re-accumulate frames to trigger cancel.
#[test]
fn tick_resets_counters() {
    let tts_active = Arc::new(AtomicBool::new(true));
    let tts_cancel = Arc::new(AtomicBool::new(false));
    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    // Send frames just below threshold.
    for _ in 0..(REMOTE_SPEECH_THRESHOLD - 1) {
        tracker.on_frame(1);
    }
    assert!(!tts_cancel.load(Ordering::Acquire));

    // Tick resets counters.
    tracker.on_tick();
    assert_eq!(tracker.count_for(1), 0, "counter should be zero after tick");

    // Send same number again — still below threshold because counter was reset.
    for _ in 0..(REMOTE_SPEECH_THRESHOLD - 1) {
        tracker.on_frame(1);
    }
    assert!(
        !tts_cancel.load(Ordering::Acquire),
        "cancel should not fire: counter was reset by tick, frames still below threshold",
    );
}

/// Per-peer isolation: peer A's silence does not reset peer B's accumulation.
#[test]
fn per_peer_counters_are_independent() {
    let tts_active = Arc::new(AtomicBool::new(true));
    let tts_cancel = Arc::new(AtomicBool::new(false));
    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    // Peer 1 sends frames below threshold (DTX comfort noise).
    for _ in 0..2 {
        tracker.on_frame(1);
    }

    // Peer 2 sends frames above threshold (real speech).
    for _ in 0..REMOTE_SPEECH_THRESHOLD {
        tracker.on_frame(2);
    }

    assert!(
        tts_cancel.load(Ordering::Acquire),
        "peer 2 should trigger cancel independently of peer 1's low count",
    );
    assert_eq!(
        tracker.count_for(1),
        2,
        "peer 1 counter should be untouched"
    );
    assert_eq!(
        tracker.count_for(2),
        REMOTE_SPEECH_THRESHOLD,
        "peer 2 counter should be at threshold",
    );
}

/// Multiple peers both above threshold → cancel fires (idempotent).
#[test]
fn multiple_peers_above_threshold_idempotent() {
    let tts_active = Arc::new(AtomicBool::new(true));
    let tts_cancel = Arc::new(AtomicBool::new(false));
    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    // Both peers send above threshold.
    for _ in 0..REMOTE_SPEECH_THRESHOLD {
        tracker.on_frame(1);
        tracker.on_frame(2);
    }

    assert!(
        tts_cancel.load(Ordering::Acquire),
        "cancel should be set when multiple peers exceed threshold",
    );
    // tts_active should still be true — only the TTS worker resets it.
    assert!(
        tts_active.load(Ordering::Acquire),
        "tts_active should remain true (only TTS worker clears it)",
    );
}

/// tts_cancel already true → setting again is harmless (AtomicBool store).
#[test]
fn cancel_already_true_is_harmless() {
    let tts_active = Arc::new(AtomicBool::new(true));
    let tts_cancel = Arc::new(AtomicBool::new(true)); // Already cancelled.
    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    for _ in 0..REMOTE_SPEECH_THRESHOLD {
        tracker.on_frame(1);
    }

    assert!(
        tts_cancel.load(Ordering::Acquire),
        "tts_cancel should still be true (idempotent store)",
    );
}

// ── Regression: local-only interrupt still works ──────────────────────────

/// The existing local barge-in path (STT detects speech → sets tts_cancel)
/// must continue to work independently of remote frame counting.
#[test]
fn local_barge_in_still_works_without_remote_frames() {
    let _tts_active = AtomicBool::new(true);
    let tts_cancel = AtomicBool::new(false);

    // Simulate local STT barge-in (stt.rs after BARGE_IN_DEBOUNCE_FRAMES).
    tts_cancel.store(true, Ordering::Release);

    assert!(
        tts_cancel.load(Ordering::Acquire),
        "local barge-in should set tts_cancel",
    );
}

// ── Cancel consumption tests (TTS worker side) ────────────────────────────

/// TTS worker correctly resets both tts_cancel and tts_active after cancel.
#[test]
fn cancel_consumption_resets_flags() {
    let cancel = AtomicBool::new(true);
    let shutdown = AtomicBool::new(false);
    let tts_active = AtomicBool::new(true);
    let (_tx, rx) = mpsc::channel::<String>();

    let handled = simulate_cancel_consumption(&cancel, &shutdown, &tts_active, &rx);

    assert!(handled, "should return true when cancel is set");
    assert!(
        !cancel.load(Ordering::Acquire),
        "cancel should be reset to false after consumption",
    );
    assert!(
        !tts_active.load(Ordering::Acquire),
        "tts_active should be reset to false after cancel",
    );
}

/// TTS worker drains the text queue on cancel.
#[test]
fn cancel_consumption_drains_queue() {
    let cancel = AtomicBool::new(true);
    let shutdown = AtomicBool::new(false);
    let tts_active = AtomicBool::new(true);
    let (tx, rx) = mpsc::channel::<String>();

    tx.send("sentence one".to_string()).unwrap();
    tx.send("sentence two".to_string()).unwrap();
    tx.send("sentence three".to_string()).unwrap();

    let handled = simulate_cancel_consumption(&cancel, &shutdown, &tts_active, &rx);

    assert!(handled);
    assert!(
        rx.try_recv().is_err(),
        "text queue should be drained after cancel",
    );
}

/// No cancel, no shutdown → TTS worker continues (returns false).
#[test]
fn no_cancel_no_shutdown_returns_false() {
    let cancel = AtomicBool::new(false);
    let shutdown = AtomicBool::new(false);
    let tts_active = AtomicBool::new(true);
    let (_tx, rx) = mpsc::channel::<String>();

    let handled = simulate_cancel_consumption(&cancel, &shutdown, &tts_active, &rx);

    assert!(
        !handled,
        "should return false when neither cancel nor shutdown is set",
    );
    assert!(
        tts_active.load(Ordering::Acquire),
        "tts_active should remain true",
    );
}

/// Shutdown takes priority over cancel — cancel flag is NOT reset.
#[test]
fn shutdown_takes_priority_over_cancel() {
    let cancel = AtomicBool::new(true);
    let shutdown = AtomicBool::new(true);
    let tts_active = AtomicBool::new(true);
    let (_tx, rx) = mpsc::channel::<String>();

    let handled = simulate_cancel_consumption(&cancel, &shutdown, &tts_active, &rx);

    assert!(handled, "should return true on shutdown");
    assert!(
        !tts_active.load(Ordering::Acquire),
        "tts_active should be false after shutdown",
    );
    assert!(
        cancel.load(Ordering::Acquire),
        "cancel should remain true (shutdown path doesn't reset it)",
    );
}

// ── Full lifecycle tests ──────────────────────────────────────────────────

/// Idle branch: nothing queued + player drained + audio was played →
/// release `tts_active` and re-arm the lead-in for the next utterance.
#[test]
fn idle_check_releases_mic_gate_after_drain() {
    let tts_active = AtomicBool::new(true);
    let mut first_append = false; // audio was appended this utterance

    simulate_idle_check(true, &mut first_append, &tts_active);

    assert!(
        !tts_active.load(Ordering::Acquire),
        "tts_active should be released once playback drains",
    );
    assert!(
        first_append,
        "lead-in should be re-armed for next utterance"
    );
}

/// Idle branch: nothing queued but audio is STILL DRAINING → `tts_active`
/// stays set. This is the cross-item pipelining contract: the per-item
/// drain barrier is gone, so the mic gate must hold across the gap
/// between item N finishing synthesis and its audio finishing playback.
#[test]
fn idle_check_holds_mic_gate_while_audio_drains() {
    let tts_active = AtomicBool::new(true);
    let mut first_append = false;

    simulate_idle_check(false, &mut first_append, &tts_active);

    assert!(
        tts_active.load(Ordering::Acquire),
        "tts_active must stay set while queued audio is still playing",
    );
    assert!(!first_append, "lead-in must not re-arm mid-playback");
}

/// Idle branch: player empty but nothing was ever appended (e.g. worker
/// just started, or every item preprocessed to empty) → no spurious
/// store, lead-in stays armed.
#[test]
fn idle_check_noop_when_nothing_was_queued() {
    let tts_active = AtomicBool::new(false);
    let mut first_append = true;

    simulate_idle_check(true, &mut first_append, &tts_active);

    assert!(!tts_active.load(Ordering::Acquire));
    assert!(first_append, "lead-in should remain armed");
}

/// On-receipt check (PR #997 review blocker): item N's audio drains, then
/// item N+1 arrives BEFORE the recv timeout fires. The same drained-player
/// check must run on receipt so `tts_active` is released before the
/// synthesis pass — otherwise STT discards human speech as "echo" for the
/// whole synthesis window while the agent is actually silent.
#[test]
fn on_receipt_check_releases_mic_gate_before_synthesis() {
    let tts_active = AtomicBool::new(true);
    let mut first_append = false; // item N appended audio, now fully drained

    // Item N+1 received with the player already empty: release + re-arm.
    simulate_idle_check(true, &mut first_append, &tts_active);

    assert!(
        !tts_active.load(Ordering::Acquire),
        "tts_active must be released before synthesizing the next item \
         when playback has already drained",
    );
    assert!(
        first_append,
        "lead-in must re-arm so the next utterance gets a fresh cushion"
    );
}

// ── Barge-in monitor ──────────────────────────────────────────────────────
//
// Models one tick of the tts-barge-in-monitor thread in `tts_worker`. The
// contract (must match production):
//   - cancel set   → take `player_ops`, RE-CHECK cancel under the lock; if
//     still set: silence player + release tts_active, flag NOT consumed
//     (the worker owns consumption: queue drain + lead-in reset)
//   - cancel cleared by the time the lock is held → no-op (stale branch)
//   - cancel clear → no-op
// Not consuming the flag is what makes the monitor idempotent across ticks
// and closes the race where the worker appends a sentence after the
// monitor's clear but before consuming the flag. The under-lock re-check
// closes the converse race (PR #997 review blocker): worker consumes the
// cancel and appends a fresh post-cancel utterance between the monitor's
// initial load and its clear — the stale branch must not delete that audio.

/// Test-side model of one monitor tick. `player_cleared` stands in for the
/// `clear()+play()` pair on the real Player; `player_ops` is the mutex
/// serializing monitor clears with worker player mutations.
fn simulate_monitor_tick(
    cancel: &AtomicBool,
    tts_active: &AtomicBool,
    player_ops: &Mutex<()>,
    player_cleared: &mut bool,
) {
    if cancel.load(Ordering::Acquire) {
        let _ops = player_ops.lock().unwrap();
        if cancel.load(Ordering::Acquire) {
            *player_cleared = true;
            tts_active.store(false, Ordering::Release);
        }
    }
}

/// Cancel set → monitor silences playback and releases the mic gate, but
/// leaves the flag for the worker to consume.
#[test]
fn monitor_tick_silences_and_releases_without_consuming_cancel() {
    let cancel = AtomicBool::new(true);
    let tts_active = AtomicBool::new(true);
    let player_ops = Mutex::new(());
    let mut player_cleared = false;

    simulate_monitor_tick(&cancel, &tts_active, &player_ops, &mut player_cleared);

    assert!(player_cleared, "monitor must silence in-flight audio");
    assert!(
        !tts_active.load(Ordering::Acquire),
        "monitor must release the mic gate immediately on barge-in",
    );
    assert!(
        cancel.load(Ordering::Acquire),
        "monitor must NOT consume the cancel flag — the worker owns \
         queue drain and lead-in reset",
    );
}

/// No cancel → monitor is a pure no-op.
#[test]
fn monitor_tick_noop_without_cancel() {
    let cancel = AtomicBool::new(false);
    let tts_active = AtomicBool::new(true);
    let player_ops = Mutex::new(());
    let mut player_cleared = false;

    simulate_monitor_tick(&cancel, &tts_active, &player_ops, &mut player_cleared);

    assert!(!player_cleared);
    assert!(
        tts_active.load(Ordering::Acquire),
        "monitor must not touch the mic gate while no barge-in is pending",
    );
}

/// Stale-branch race (PR #997 review blocker): monitor observes
/// `cancel == true`, then the worker — under `player_ops` — consumes the
/// cancel and appends a fresh post-cancel utterance before the monitor
/// reaches its clear. The monitor's under-lock re-check must see
/// `cancel == false` and no-op, leaving the fresh audio and its mic gate
/// intact.
#[test]
fn monitor_stale_cancel_branch_must_not_clear_fresh_audio() {
    let cancel = AtomicBool::new(true);
    let tts_active = AtomicBool::new(true);
    let player_ops = Mutex::new(());
    let mut player_cleared = false;

    // Monitor's initial (pre-lock) load observes the cancel…
    let stale_observation = cancel.load(Ordering::Acquire);
    assert!(stale_observation);

    // …then the worker wins the lock: consumes the cancel, appends a fresh
    // utterance, sets tts_active (mirrors handle_cancel_or_shutdown +
    // the locked append in the sentence loop).
    {
        let _ops = player_ops.lock().unwrap();
        cancel.store(false, Ordering::Release);
        tts_active.store(true, Ordering::Release);
    }

    // Monitor resumes from its stale branch — the re-check under the lock
    // must turn it into a no-op.
    if stale_observation {
        let _ops = player_ops.lock().unwrap();
        if cancel.load(Ordering::Acquire) {
            player_cleared = true;
            tts_active.store(false, Ordering::Release);
        }
    }

    assert!(
        !player_cleared,
        "stale monitor branch must not clear a fresh post-cancel utterance",
    );
    assert!(
        tts_active.load(Ordering::Acquire),
        "stale monitor branch must not release the mic gate while fresh \
         audio is playing",
    );
}

/// Full cycle: remote speech → cancel → TTS consumption → new TTS → cancel again.
/// Validates the cancel mechanism is reusable across TTS sessions.
/// The false→true transition on tts_active auto-clears counters — no
/// explicit on_tick() needed between sessions.
#[test]
fn full_cancel_cycle_is_reusable() {
    let tts_active = Arc::new(AtomicBool::new(false));
    let tts_cancel = Arc::new(AtomicBool::new(false));
    let shutdown = AtomicBool::new(false);
    let (_tx, rx) = mpsc::channel::<String>();

    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    // Cycle 1: TTS starts, remote speech triggers cancel, TTS consumes.
    tts_active.store(true, Ordering::Release);
    for _ in 0..REMOTE_SPEECH_THRESHOLD {
        tracker.on_frame(1);
    }
    assert!(tts_cancel.load(Ordering::Acquire));

    let handled = simulate_cancel_consumption(&tts_cancel, &shutdown, &tts_active, &rx);
    assert!(handled);
    assert!(!tts_cancel.load(Ordering::Acquire));
    assert!(!tts_active.load(Ordering::Acquire));

    // No on_tick() needed — the false→true transition auto-clears counters.

    // Cycle 2: New TTS starts, another remote speech triggers cancel.
    tts_active.store(true, Ordering::Release);
    for _ in 0..REMOTE_SPEECH_THRESHOLD {
        tracker.on_frame(1);
    }
    assert!(tts_cancel.load(Ordering::Acquire));

    let handled = simulate_cancel_consumption(&tts_cancel, &shutdown, &tts_active, &rx);
    assert!(handled);
    assert!(!tts_cancel.load(Ordering::Acquire));
    assert!(!tts_active.load(Ordering::Acquire));
}

/// TTS session transition (false→true) clears stale counters.
/// Prevents pre-existing speech from tripping a cancel on TTS restart.
#[test]
fn tts_session_transition_clears_counters() {
    let tts_active = Arc::new(AtomicBool::new(true));
    let tts_cancel = Arc::new(AtomicBool::new(false));
    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    // Accumulate frames just below threshold during first TTS session.
    for _ in 0..(REMOTE_SPEECH_THRESHOLD - 1) {
        tracker.on_frame(1);
    }
    assert_eq!(tracker.count_for(1), REMOTE_SPEECH_THRESHOLD - 1);
    assert!(!tts_cancel.load(Ordering::Acquire));

    // TTS stops (cancel consumed).
    tts_active.store(false, Ordering::Release);
    // Send a frame while inactive — triggers tts_was_active transition tracking.
    tracker.on_frame(1);

    // TTS restarts — false→true transition should clear counters.
    tts_active.store(true, Ordering::Release);
    tracker.on_frame(1); // First frame of new session triggers clear + count.
    assert_eq!(
        tracker.count_for(1),
        1,
        "counter should be 1 (cleared on session transition, then incremented)",
    );

    // Need full threshold again from scratch.
    for _ in 1..REMOTE_SPEECH_THRESHOLD {
        tracker.on_frame(1);
    }
    assert!(
        tts_cancel.load(Ordering::Acquire),
        "cancel should fire after full threshold in new session",
    );
}

/// Concurrent remote cancel + local barge-in → both safe, one cancel processed.
#[test]
fn concurrent_remote_and_local_cancel() {
    let tts_active = Arc::new(AtomicBool::new(true));
    let tts_cancel = Arc::new(AtomicBool::new(false));

    // Remote path: frame counting above threshold.
    let cancel_remote = Arc::clone(&tts_cancel);
    let active_remote = Arc::clone(&tts_active);
    let remote = std::thread::spawn(move || {
        // Simulate threshold crossing — directly set cancel (the tracker
        // would do this after REMOTE_SPEECH_THRESHOLD frames).
        if active_remote.load(Ordering::Acquire) {
            cancel_remote.store(true, Ordering::Release);
        }
    });

    // Local path: STT barge-in.
    let cancel_local = Arc::clone(&tts_cancel);
    let local = std::thread::spawn(move || {
        cancel_local.store(true, Ordering::Release);
    });

    remote.join().unwrap();
    local.join().unwrap();

    assert!(
        tts_cancel.load(Ordering::Acquire),
        "tts_cancel should be true regardless of which path set it",
    );
}

/// Frames while TTS is inactive are NOT counted. The peer must accumulate
/// the full threshold AFTER tts_active becomes true.
#[test]
fn frames_while_tts_inactive_are_not_counted() {
    let tts_active = Arc::new(AtomicBool::new(false));
    let tts_cancel = Arc::new(AtomicBool::new(false));
    let mut tracker = RemoteInterruptTracker::new(Arc::clone(&tts_active), Arc::clone(&tts_cancel));

    // Send frames while TTS is inactive — should be ignored entirely.
    for _ in 0..50 {
        tracker.on_frame(1);
    }
    assert_eq!(
        tracker.count_for(1),
        0,
        "no frames should accumulate while TTS inactive"
    );
    assert!(!tts_cancel.load(Ordering::Acquire));

    // TTS starts. Peer must now accumulate from zero.
    tts_active.store(true, Ordering::Release);

    // Send frames below threshold — not enough yet.
    for _ in 0..(REMOTE_SPEECH_THRESHOLD - 1) {
        tracker.on_frame(1);
    }
    assert!(
        !tts_cancel.load(Ordering::Acquire),
        "cancel should not fire: only {} frames since TTS became active",
        REMOTE_SPEECH_THRESHOLD - 1,
    );

    // One more frame crosses the threshold.
    tracker.on_frame(1);
    assert!(
        tts_cancel.load(Ordering::Acquire),
        "cancel should fire after full threshold accumulated post-activation",
    );
}

// ── apply_fade_out tests ──────────────────────────────────────────────────

/// The fade-out half of the old `apply_fades`: last sample is silenced
/// and the ramp is monotonic. Mid-buffer must be untouched.
#[test]
fn apply_fade_out_short_buffer() {
    let mut samples = vec![1.0f32; 10];
    apply_fade_out(&mut samples);
    assert_eq!(samples[9], 0.0, "last sample should be silenced");
    assert!(samples[5] > 0.5, "mid-buffer should be near-untouched");
}

/// REGRESSION (2026-05-18): the *first* samples must NOT be attenuated.
/// An earlier `apply_fades` symmetrically faded in over 8 ms which
/// swallowed the consonant onset of every sentence.
/// Lock in: samples[0..FADE_OUT_SAMPLES] are byte-equal to input.
#[test]
fn apply_fade_out_does_not_touch_leading_samples() {
    // Input long enough that fade window doesn't overlap (≫ 2× fade).
    let n = FADE_OUT_SAMPLES * 4;
    let input: Vec<f32> = (0..n).map(|i| 0.5 + (i as f32) * 1e-4).collect();
    let mut samples = input.clone();
    apply_fade_out(&mut samples);
    for i in 0..FADE_OUT_SAMPLES {
        assert_eq!(
            samples[i], input[i],
            "leading sample {i} must not be attenuated (was {} → {})",
            input[i], samples[i]
        );
    }
    // And the trailing fade still works.
    assert_eq!(samples[n - 1], 0.0);
}

#[test]
fn apply_fade_out_empty_buffer() {
    let mut samples: Vec<f32> = vec![];
    apply_fade_out(&mut samples);
    assert!(samples.is_empty());
}

#[test]
fn apply_fade_out_single_sample() {
    // fade = min(FADE_OUT_SAMPLES, len/2) = 0, so nothing changes.
    let mut samples = vec![1.0f32];
    apply_fade_out(&mut samples);
    assert_eq!(samples[0], 1.0);
}

/// Sanity-check the per-sentence cushion length: 20 ms at 24 kHz must
/// land at exactly 480 samples. This is a const computation, so the
/// real value of this test is documenting *why* 20 ms was chosen — it
/// covers a typical CoreAudio buffer turnover (256–1024 samples)
/// without being audible as user-facing latency.
#[test]
fn sentence_lead_in_is_sane() {
    assert_eq!(SENTENCE_LEAD_IN_SAMPLES, 480, "20 ms × 24 kHz");
}

// ── build_sentence_append_buffer tests ───────────────────────────────────

/// REGRESSION: every chunk needs an onset cushion; synthesized chunks
/// can start with speech energy within the first millisecond.
#[test]
fn lead_in_pad_is_present_for_every_sentence_chunk() {
    const SENTENCE_AUDIO_LEN: usize = 1000;
    const SILENCE_BUF_LEN: usize = 2400; // 100 ms at 24 kHz, like production
    const N_SENTENCES: usize = 5;

    let mut first = true;

    for _ in 0..N_SENTENCES {
        let buf = build_sentence_append_buffer(
            &mut first,
            vec![0.5_f32; SENTENCE_AUDIO_LEN],
            SILENCE_BUF_LEN,
        );

        assert_eq!(buf.len(), SENTENCE_AUDIO_LEN + SILENCE_BUF_LEN);
        assert!(
            buf[..SENTENCE_LEAD_IN_SAMPLES].iter().all(|&s| s == 0.0),
            "lead-in pad must be pure silence"
        );
        assert!(
            buf[SENTENCE_LEAD_IN_SAMPLES..SENTENCE_LEAD_IN_SAMPLES + SENTENCE_AUDIO_LEN]
                .iter()
                .all(|&s| s == 0.5),
            "sentence audio must immediately follow the lead-in"
        );
        assert!(
            buf[SENTENCE_LEAD_IN_SAMPLES + SENTENCE_AUDIO_LEN..]
                .iter()
                .all(|&s| s == 0.0),
            "trailing gap must be pure silence"
        );
    }

    assert!(!first, "first_append flag must be cleared after first call");
}

/// `first_append` still flips on the first call for `tts_active` gating.
#[test]
fn build_sentence_append_buffer_flips_first_append() {
    let mut first = true;
    let _ = build_sentence_append_buffer(&mut first, vec![0.5; 100], 2400);
    assert!(!first, "first call must flip the flag");

    // Subsequent call: still has a per-sentence lead-in, flag stays false.
    let buf = build_sentence_append_buffer(&mut first, vec![0.5; 100], 2400);
    assert!(buf[..SENTENCE_LEAD_IN_SAMPLES].iter().all(|&s| s == 0.0));
    assert!(!first);
}

/// Leading silence is exactly the lead-in; no pre-audio gap is double-counted.
#[test]
fn first_sentence_leading_silence_is_exactly_lead_in() {
    let mut first = true;
    let buf = build_sentence_append_buffer(&mut first, vec![0.5; 100], 2400);
    assert!(buf[..SENTENCE_LEAD_IN_SAMPLES].iter().all(|&s| s == 0.0));
    assert_eq!(buf[SENTENCE_LEAD_IN_SAMPLES], 0.5);
}

/// Tail silence plus the next lead-in preserves the 100 ms sentence gap.
#[test]
fn sentence_gap_budget_is_preserved() {
    let mut first = true;
    let silence_buf_len = 2400;
    let first_buf = build_sentence_append_buffer(&mut first, vec![0.5; 100], silence_buf_len);
    let second_buf = build_sentence_append_buffer(&mut first, vec![0.5; 100], silence_buf_len);

    let first_tail = &first_buf[SENTENCE_LEAD_IN_SAMPLES + 100..];
    let second_lead = &second_buf[..SENTENCE_LEAD_IN_SAMPLES];
    assert_eq!(first_tail.len(), silence_buf_len - SENTENCE_LEAD_IN_SAMPLES);
    assert_eq!(second_lead.len(), SENTENCE_LEAD_IN_SAMPLES);
    assert_eq!(first_tail.len() + second_lead.len(), silence_buf_len);
}

/// Regression guard: one contiguous rodio source per synthesized sentence.
#[test]
fn sentence_append_buffer_is_one_contiguous_source() {
    let mut first = true;
    let buf = build_sentence_append_buffer(&mut first, vec![0.5; 100], 2400);

    assert_eq!(buf.len(), 2400 + 100);
    assert!(buf[..SENTENCE_LEAD_IN_SAMPLES].iter().all(|&s| s == 0.0));
    assert!(
        buf[SENTENCE_LEAD_IN_SAMPLES..SENTENCE_LEAD_IN_SAMPLES + 100]
            .iter()
            .all(|&s| s == 0.5)
    );
}

// ── clamp_to_full_scale tests ─────────────────────────────────────────────

/// In-range speech audio passes through bit-exact — no gain is applied.
/// (Pocket output is already at speech level; see the fn doc-comment for
/// the history of the two gain-stage regressions this replaced.)
#[test]
fn clamp_to_full_scale_passes_speech_audio_unchanged() {
    let input = vec![0.42_f32, -0.97, 0.076, 0.0];
    let out = clamp_to_full_scale(input.clone());
    assert_eq!(out, input);
}

/// Outlier transients beyond full scale are hard-clamped to ±1.0 rather
/// than wrapping.
#[test]
fn clamp_to_full_scale_clamps_outliers() {
    let input = vec![1.5_f32, -2.0, 0.5];
    let out = clamp_to_full_scale(input);
    assert_eq!(out, vec![1.0, -1.0, 0.5]);
}

/// Empty input round-trips to empty output.
#[test]
fn clamp_to_full_scale_empty_buffer() {
    let out = clamp_to_full_scale(Vec::new());
    assert!(out.is_empty());
}

// ── group_sentences_into_chunks tests ─────────────────────────────────────

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// The first sentence always stands alone — it bounds time-to-first-audio.
/// Even when the whole message would fit in one chunk, sentence one must
/// not wait on synthesis of the rest.
#[test]
fn chunk_grouping_first_sentence_is_always_alone() {
    let chunks = group_sentences_into_chunks(&s(&["Hi there.", "Short.", "Tiny."]), 200);
    assert_eq!(chunks[0], "Hi there.");
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[1], "Short. Tiny.");
}

/// Sentences after the first pack greedily up to the char budget, then
/// spill into a new chunk. Fewer generate() calls = fewer prosody seams.
#[test]
fn chunk_grouping_packs_up_to_budget_then_spills() {
    let a = "A".repeat(50) + ".";
    let b = "B".repeat(50) + ".";
    let c = "C".repeat(50) + ".";
    let d = "D".repeat(50) + ".";
    // Budget of 110: b+c fits (51+1+51 = 103), adding d (103+1+51) does not.
    let chunks = group_sentences_into_chunks(&s(&[&a, &b, &c, &d]), 110);
    assert_eq!(chunks.len(), 3, "chunks: {chunks:?}");
    assert_eq!(chunks[0], a);
    assert_eq!(chunks[1], format!("{b} {c}"));
    assert_eq!(chunks[2], d);
}

/// A single sentence longer than the budget is passed through unsplit —
/// long single sentences are fine (the LM cap bounds runaway); only seams
/// are being minimized.
#[test]
fn chunk_grouping_oversized_sentence_passes_through() {
    let long = "word ".repeat(60).trim_end().to_string() + ".";
    assert!(long.len() > 200);
    let chunks = group_sentences_into_chunks(&s(&["First.", &long]), 200);
    assert_eq!(chunks, vec!["First.".to_string(), long]);
}

/// Single-sentence messages — the common huddle case, since agents are
/// prompted to send one sentence per message — are unaffected by grouping.
#[test]
fn chunk_grouping_single_sentence_unchanged() {
    let chunks = group_sentences_into_chunks(&s(&["Just one sentence here."]), 200);
    assert_eq!(chunks, vec!["Just one sentence here.".to_string()]);
}

/// Empty and whitespace-only entries are dropped, and never produce
/// empty chunks (which would synthesize as garbage).
#[test]
fn chunk_grouping_skips_blank_sentences() {
    let chunks = group_sentences_into_chunks(&s(&["", "  ", "Real sentence.", "   ", "Two."]), 200);
    assert_eq!(chunks[0], "Real sentence.");
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[1], "Two.");
}

/// Empty input produces no chunks (the worker loop then synthesizes nothing).
#[test]
fn chunk_grouping_empty_input() {
    assert!(group_sentences_into_chunks(&[], 200).is_empty());
}

/// Chunks joined with a single space preserve each sentence's terminal
/// punctuation — the model sees natural multi-sentence prose, matching the
/// shape upstream's ~50-token chunker produces.
#[test]
fn chunk_grouping_preserves_punctuation_at_joins() {
    let chunks = group_sentences_into_chunks(&s(&["Lead.", "Really?", "Yes!", "Good."]), 200);
    assert_eq!(chunks[1], "Really? Yes! Good.");
}
