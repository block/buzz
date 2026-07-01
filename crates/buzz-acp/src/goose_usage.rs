//! Goose-specific usage tracking for NIP-AM agent turn metrics.
//!
//! Goose emits a `_goose/unstable/session/update` notification (with
//! `sessionUpdate: "usage_update"`) at the end of every turn when the client
//! has advertised `clientCapabilities._meta.goose.customNotifications: true`.
//! The payload carries session-cumulative token counts from which we derive
//! per-turn deltas.
//!
//! # Delta computation
//!
//! Because goose only reports cumulative counters, the per-turn counts are
//! computed as `current − previous`. Three cases require special handling per
//! NIP-AM:
//!
//! 1. **First turn (no prior baseline):** delta unknown → `null` counts,
//!    `delta_reliable: false`.
//! 2. **Counter decrease** (harness restart, overflow): delta would be
//!    negative → `null` counts, `delta_reliable: false`.
//! 3. **Session restart** (caller supplies a new `session_id` not seen
//!    before): treated as case 1 — fresh baseline, no delta for this turn.
//!
//! The `GooseTurnUsage` produced after each turn is consumed by the
//! `TurnCompletionGuard` in `pool.rs` to publish a kind 44200 relay event.

use std::collections::HashMap;

/// Wire-format deserialization for `_goose/unstable/session/update` params.
///
/// Method: `_goose/unstable/session/update`
/// Shape (camelCase on the wire):
/// ```json
/// {
///   "sessionId": "...",
///   "update": {
///     "sessionUpdate": "usage_update",
///     "used": 12345,
///     "contextLimit": 200000,
///     "accumulatedInputTokens": 10000,
///     "accumulatedOutputTokens": 2345,
///     "accumulatedCost": 0.0234
///   }
/// }
/// ```
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GooseSessionUpdateNotification {
    pub session_id: String,
    pub update: GooseSessionUpdateVariant,
}

/// Discriminated union matching goose's `GooseSessionUpdate` enum on the wire.
/// We only care about `usage_update`; other variants are ignored.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub(crate) enum GooseSessionUpdateVariant {
    UsageUpdate(GooseUsageUpdatePayload),
    #[serde(other)]
    Other,
}

/// The `usage_update` payload from goose.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GooseUsageUpdatePayload {
    #[allow(dead_code)]
    pub used: u64,
    #[allow(dead_code)]
    pub context_limit: u64,
    pub accumulated_input_tokens: u64,
    pub accumulated_output_tokens: u64,
    pub accumulated_cost: Option<f64>,
}

/// Per-session normalization state: the last cumulative snapshot we saw.
#[derive(Debug, Clone)]
struct SessionState {
    /// Monotonically increasing per-session turn counter (1-based, incremented
    /// on every recorded update).
    turn_seq: u64,
    /// Cumulative input tokens at the end of the previous turn.
    last_input: u64,
    /// Cumulative output tokens at the end of the previous turn.
    last_output: u64,
    /// Cumulative cost at the end of the previous turn.
    last_cost: Option<f64>,
}

/// Per-turn usage record exposed to `TurnCompletionGuard` for NIP-AM publishing.
///
/// `turn_*` fields are `None` when delta is unreliable (first turn or counter
/// decrease). `cumulative_*` fields are always present when goose reports them.
#[derive(Debug, Clone)]
pub struct GooseTurnUsage {
    /// Goose session id (maps to NIP-AM `sessionId`).
    pub session_id: String,
    /// Per-session monotonic sequence number for this turn (maps to NIP-AM `turnSeq`).
    pub turn_seq: u64,
    /// Whether the `turn_*` delta fields are reliable.
    pub delta_reliable: bool,
    /// Per-turn input token delta; `None` when unreliable.
    pub turn_input_tokens: Option<u64>,
    /// Per-turn output token delta; `None` when unreliable.
    pub turn_output_tokens: Option<u64>,
    /// Per-turn cost delta (`current − previous`); `None` when unreliable or
    /// either snapshot is missing.
    pub turn_cost_usd: Option<f64>,
    /// Session-cumulative input tokens as reported by goose at end of turn.
    pub cumulative_input_tokens: u64,
    /// Session-cumulative output tokens as reported by goose at end of turn.
    pub cumulative_output_tokens: u64,
    /// Session-cumulative estimated cost in USD; `None` if goose did not report it.
    pub cumulative_cost_usd: Option<f64>,
}

/// Tracks per-session cumulative usage state across turns.
///
/// Cheap to construct. Usage lifecycle per turn:
///
/// 1. **`begin_turn(session_id)`** — call this immediately before sending
///    `session/prompt`. Marks the tracker as in-flight for the given session
///    and clears any leftover pending record from a previous turn. Setup
///    notifications that arrive *before* the first `begin_turn` (e.g. during
///    `session/new` setup) will still update the cumulative baseline but will
///    NOT produce a publishable record.
/// 2. **`record(session_id, payload)`** — called for each
///    `_goose/unstable/session/update` notification. Always updates the
///    cumulative baseline; only produces a publishable record when a turn is
///    currently in-flight for the matching session.
/// 3. **`take()`** — called at turn completion by `TurnCompletionGuard`.
///    Drains and returns the pending record (or `None` if goose did not emit
///    usage for this turn) and clears the in-flight marker.
#[derive(Debug, Default)]
pub(crate) struct GooseUsageTracker {
    /// One entry per goose `sessionId` ever seen in this process.
    sessions: HashMap<String, SessionState>,
    /// The session that currently has an in-flight `session/prompt`.
    /// `None` means no prompt is in flight; `record()` will still update
    /// the baseline but will not set `pending`.
    in_flight_session: Option<String>,
    /// The most recently computed turn usage, ready for `take()`.
    pending: Option<GooseTurnUsage>,
}

impl GooseUsageTracker {
    /// Mark the start of a new prompt turn for `session_id`.
    ///
    /// Clears any leftover `pending` record and records which session is
    /// in-flight. Must be called before the corresponding `session/prompt`
    /// request is sent so that setup notifications received before this call
    /// do not become publishable for this turn.
    pub(crate) fn begin_turn(&mut self, session_id: &str) {
        self.in_flight_session = Some(session_id.to_string());
        self.pending = None;
    }

    /// Process a `usage_update` notification payload.
    ///
    /// **Always** updates the cumulative baseline for `session_id` so that the
    /// next in-flight turn can compute a correct delta even if this notification
    /// arrived outside a turn (e.g. during `session/new` setup).
    ///
    /// Only produces a publishable `pending` record when a turn is currently
    /// in-flight for the matching `session_id`. If `in_flight_session` is
    /// `None` or refers to a different session, the baseline is updated but
    /// `pending` is left unchanged.
    ///
    /// When multiple notifications arrive during the same turn, the last one
    /// wins (goose may emit several per turn; each increments `turn_seq`).
    pub(crate) fn record(&mut self, session_id: &str, payload: &GooseUsageUpdatePayload) {
        let current_input = payload.accumulated_input_tokens;
        let current_output = payload.accumulated_output_tokens;
        let current_cost = payload.accumulated_cost;

        let (delta_reliable, turn_input, turn_output, turn_cost, turn_seq) =
            match self.sessions.get(session_id) {
                None => {
                    // First notification for this session — no baseline yet.
                    (false, None, None, None, 1u64)
                }
                Some(prev) => {
                    let seq = prev.turn_seq + 1;
                    // Token counter decrease → unreliable delta.
                    if current_input < prev.last_input || current_output < prev.last_output {
                        (false, None, None, None, seq)
                    } else {
                        let di = current_input - prev.last_input;
                        let dout = current_output - prev.last_output;
                        // Cost delta: only when both snapshots have cost.
                        // A cost *decrease* is also unreliable (NIP-AM: negative
                        // delta ⇒ delta_reliable false, null all turn fields).
                        let (dc, cost_reliable) = match (current_cost, prev.last_cost) {
                            (Some(c), Some(p)) if c >= p => (Some(c - p), true),
                            (Some(_), Some(_)) => {
                                // Both present but current < prev — counter decreased.
                                (None, false)
                            }
                            _ => (None, true), // absent on either side: null cost, reliable tokens
                        };
                        if cost_reliable {
                            (true, Some(di), Some(dout), dc, seq)
                        } else {
                            // Cost decrease overrides the whole record to unreliable.
                            (false, None, None, None, seq)
                        }
                    }
                }
            };

        // Always advance the session baseline so the next in-flight turn can
        // compute a correct delta even if this notification is from setup.
        self.sessions.insert(
            session_id.to_string(),
            SessionState {
                turn_seq,
                last_input: current_input,
                last_output: current_output,
                last_cost: current_cost,
            },
        );

        // Only publish a pending record if this session is currently in-flight.
        if self.in_flight_session.as_deref() == Some(session_id) {
            self.pending = Some(GooseTurnUsage {
                session_id: session_id.to_string(),
                turn_seq,
                delta_reliable,
                turn_input_tokens: turn_input,
                turn_output_tokens: turn_output,
                turn_cost_usd: turn_cost,
                cumulative_input_tokens: current_input,
                cumulative_output_tokens: current_output,
                cumulative_cost_usd: current_cost,
            });
        }
    }

    /// Consume and return the most recently computed turn usage record, then
    /// clear the in-flight marker.
    ///
    /// Returns `None` if no `usage_update` arrived during the current in-flight
    /// turn (goose did not emit usage, or no `begin_turn` was called). The
    /// caller (`TurnCompletionGuard`) must handle `None`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn take(&mut self) -> Option<GooseTurnUsage> {
        self.in_flight_session = None;
        self.pending.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(input: u64, output: u64, cost: Option<f64>) -> GooseUsageUpdatePayload {
        GooseUsageUpdatePayload {
            used: input + output,
            context_limit: 200_000,
            accumulated_input_tokens: input,
            accumulated_output_tokens: output,
            accumulated_cost: cost,
        }
    }

    // ── Turn scoping: setup notifications must not pollute the first real turn ─

    #[test]
    fn setup_notification_before_begin_turn_returns_none() {
        // Regression: setup notifications fire during session/new (before any
        // prompt). They must update the baseline but must NOT produce a
        // publishable record for the next turn.
        let mut tracker = GooseUsageTracker::default();

        // Simulate a setup notification (no begin_turn called yet).
        tracker.record("sess-setup", &payload(500, 100, Some(0.005)));
        // No turn is in-flight — pending must stay None.
        assert!(
            tracker.pending.is_none(),
            "setup notification must not set pending before begin_turn"
        );

        // The zero-update turn: begin_turn, no notification during prompt, take.
        tracker.begin_turn("sess-setup");
        let result = tracker.take();
        assert!(
            result.is_none(),
            "zero-update turn after setup must return None"
        );

        // Baseline was still updated: the next real turn gets a correct delta.
        tracker.begin_turn("sess-setup");
        tracker.record("sess-setup", &payload(1200, 300, Some(0.012)));
        let usage = tracker.take().expect("second turn must have usage");

        assert!(
            usage.delta_reliable,
            "baseline fed by setup: delta reliable"
        );
        assert_eq!(usage.turn_input_tokens, Some(700)); // 1200 - 500
        assert_eq!(usage.turn_output_tokens, Some(200)); // 300 - 100
        let dc = usage.turn_cost_usd.expect("cost delta present");
        assert!((dc - 0.007).abs() < 1e-9, "cost delta: {dc}");
    }

    #[test]
    fn record_outside_in_flight_does_not_clobber_pending() {
        // A notification for a different session_id while another is in-flight
        // must not overwrite the pending record.
        let mut tracker = GooseUsageTracker::default();
        tracker.begin_turn("sess-a");
        tracker.record("sess-a", &payload(1000, 200, None));

        // Notification for a different session — should not touch pending.
        tracker.record("sess-b", &payload(9000, 3000, None));

        let usage = tracker.take().expect("sess-a pending must survive");
        assert_eq!(usage.session_id, "sess-a");
    }

    // ── Delta computation: non-happy paths ─────────────────────────────────

    #[test]
    fn first_turn_no_prior_delta_unreliable() {
        let mut tracker = GooseUsageTracker::default();
        tracker.begin_turn("sess-1");
        tracker.record("sess-1", &payload(1000, 200, Some(0.01)));
        let usage = tracker.take().expect("should have pending usage");

        assert_eq!(usage.session_id, "sess-1");
        assert_eq!(usage.turn_seq, 1);
        assert!(
            !usage.delta_reliable,
            "first turn: delta must be unreliable"
        );
        assert!(usage.turn_input_tokens.is_none());
        assert!(usage.turn_output_tokens.is_none());
        assert!(usage.turn_cost_usd.is_none());
        // Cumulative is still populated.
        assert_eq!(usage.cumulative_input_tokens, 1000);
        assert_eq!(usage.cumulative_output_tokens, 200);
        assert_eq!(usage.cumulative_cost_usd, Some(0.01));
    }

    #[test]
    fn counter_decrease_delta_unreliable_no_negatives() {
        let mut tracker = GooseUsageTracker::default();
        // Turn 1 — establish baseline.
        tracker.begin_turn("sess-2");
        tracker.record("sess-2", &payload(5000, 1000, Some(0.05)));
        let _ = tracker.take();

        // Turn 2 — counter decreased (harness restart simulation).
        tracker.begin_turn("sess-2");
        tracker.record("sess-2", &payload(100, 50, Some(0.001)));
        let usage = tracker.take().expect("pending");

        assert_eq!(usage.turn_seq, 2);
        assert!(
            !usage.delta_reliable,
            "counter decrease: delta must be unreliable"
        );
        assert!(usage.turn_input_tokens.is_none(), "no negative delta");
        assert!(usage.turn_output_tokens.is_none(), "no negative delta");
        assert!(usage.turn_cost_usd.is_none());
    }

    #[test]
    fn cost_decrease_sets_delta_unreliable_and_nulls_all_turn_fields() {
        // Regression for Thufir fix 2: cost counter decrease must set
        // delta_reliable = false and null all turn fields (not just cost).
        // turn_seq still increments (NIP-AM: seq advances even on unreliable).
        let mut tracker = GooseUsageTracker::default();
        // Turn 1 — establish baseline with cost.
        tracker.begin_turn("sess-cost");
        tracker.record("sess-cost", &payload(1000, 200, Some(0.10)));
        let t1 = tracker.take().expect("t1");
        assert_eq!(t1.turn_seq, 1);

        // Turn 2 — tokens monotone, but cost decreased.
        tracker.begin_turn("sess-cost");
        tracker.record("sess-cost", &payload(1500, 350, Some(0.05)));
        let usage = tracker.take().expect("t2");

        assert_eq!(usage.turn_seq, 2, "turn_seq must still increment");
        assert!(
            !usage.delta_reliable,
            "cost decrease: delta must be unreliable"
        );
        assert!(
            usage.turn_input_tokens.is_none(),
            "all turn fields null on unreliable"
        );
        assert!(usage.turn_output_tokens.is_none());
        assert!(usage.turn_cost_usd.is_none());
        // Cumulative values are unaffected.
        assert_eq!(usage.cumulative_input_tokens, 1500);
        assert_eq!(usage.cumulative_output_tokens, 350);
        assert_eq!(usage.cumulative_cost_usd, Some(0.05));
    }

    #[test]
    fn cost_absent_on_one_side_leaves_tokens_reliable() {
        // Cost merely absent on either side: null cost, reliable tokens.
        let mut tracker = GooseUsageTracker::default();
        tracker.begin_turn("sess-nocost");
        tracker.record("sess-nocost", &payload(1000, 200, Some(0.01)));
        let _ = tracker.take();

        // Turn 2 — no cost reported this time.
        tracker.begin_turn("sess-nocost");
        tracker.record("sess-nocost", &payload(1800, 450, None));
        let usage = tracker.take().expect("pending");

        assert!(
            usage.delta_reliable,
            "absent cost must not make delta unreliable"
        );
        assert_eq!(usage.turn_input_tokens, Some(800));
        assert_eq!(usage.turn_output_tokens, Some(250));
        assert!(
            usage.turn_cost_usd.is_none(),
            "cost null when absent on either side"
        );
    }

    #[test]
    fn session_restart_new_session_id_treated_as_first_turn() {
        let mut tracker = GooseUsageTracker::default();
        // Original session.
        tracker.begin_turn("sess-a");
        tracker.record("sess-a", &payload(8000, 2000, None));
        let _ = tracker.take();

        // New session_id — restart. Must behave like a first turn.
        tracker.begin_turn("sess-b");
        tracker.record("sess-b", &payload(500, 100, None));
        let usage = tracker.take().expect("pending");

        assert_eq!(usage.session_id, "sess-b");
        assert_eq!(usage.turn_seq, 1);
        assert!(
            !usage.delta_reliable,
            "new session: delta must be unreliable"
        );
        assert!(usage.turn_input_tokens.is_none());
    }

    // ── Happy path ─────────────────────────────────────────────────────────

    #[test]
    fn second_turn_delta_computed_correctly() {
        let mut tracker = GooseUsageTracker::default();
        tracker.begin_turn("sess-3");
        tracker.record("sess-3", &payload(1000, 200, Some(0.01)));
        let _ = tracker.take();

        tracker.begin_turn("sess-3");
        tracker.record("sess-3", &payload(1800, 450, Some(0.018)));
        let usage = tracker.take().expect("pending");

        assert_eq!(usage.turn_seq, 2);
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, Some(800));
        assert_eq!(usage.turn_output_tokens, Some(250));
        // cost delta: 0.018 - 0.01 = 0.008 (floating-point; use approx check)
        let dc = usage.turn_cost_usd.expect("cost delta present");
        assert!((dc - 0.008).abs() < 1e-9, "cost delta: {dc}");
        assert_eq!(usage.cumulative_input_tokens, 1800);
        assert_eq!(usage.cumulative_output_tokens, 450);
    }

    #[test]
    fn take_returns_none_after_drain() {
        let mut tracker = GooseUsageTracker::default();
        tracker.begin_turn("sess-4");
        tracker.record("sess-4", &payload(100, 20, None));
        let _ = tracker.take();
        assert!(tracker.take().is_none(), "take after drain must be None");
    }

    #[test]
    fn last_update_wins_multiple_updates_same_turn() {
        let mut tracker = GooseUsageTracker::default();
        // Turn 1 — baseline.
        tracker.begin_turn("sess-5");
        tracker.record("sess-5", &payload(1000, 100, None));
        let _ = tracker.take();

        // Two updates arrive before take() — each advances state independently;
        // the second delta is computed from the first update's snapshot.
        tracker.begin_turn("sess-5");
        tracker.record("sess-5", &payload(1500, 150, None));
        tracker.record("sess-5", &payload(2000, 250, None));
        let usage = tracker.take().expect("pending");

        // Cumulative from the last update.
        assert_eq!(usage.cumulative_input_tokens, 2000);
        assert_eq!(usage.cumulative_output_tokens, 250);
        // Delta is from the previous intermediate snapshot (1500, 150) → (2000, 250).
        assert_eq!(usage.turn_input_tokens, Some(500));
        assert_eq!(usage.turn_output_tokens, Some(100));
    }

    // ── Wire deserialization ────────────────────────────────────────────────

    #[test]
    fn notification_deserializes_from_wire_json() {
        let raw = serde_json::json!({
            "sessionId": "abc-123",
            "update": {
                "sessionUpdate": "usage_update",
                "used": 50000,
                "contextLimit": 200000,
                "accumulatedInputTokens": 40000,
                "accumulatedOutputTokens": 10000,
                "accumulatedCost": 0.42
            }
        });
        let notif: GooseSessionUpdateNotification =
            serde_json::from_value(raw).expect("deserialization");
        assert_eq!(notif.session_id, "abc-123");
        match notif.update {
            GooseSessionUpdateVariant::UsageUpdate(p) => {
                assert_eq!(p.accumulated_input_tokens, 40000);
                assert_eq!(p.accumulated_output_tokens, 10000);
                assert_eq!(p.accumulated_cost, Some(0.42));
            }
            GooseSessionUpdateVariant::Other => panic!("expected UsageUpdate"),
        }
    }

    #[test]
    fn other_variant_deserializes_without_error() {
        let raw = serde_json::json!({
            "sessionId": "xyz",
            "update": {
                "sessionUpdate": "status_message",
                "status": { "type": "notice", "message": "hi" }
            }
        });
        let notif: GooseSessionUpdateNotification =
            serde_json::from_value(raw).expect("deserialization");
        assert!(matches!(notif.update, GooseSessionUpdateVariant::Other));
    }

    #[test]
    fn missing_accumulated_cost_is_none() {
        let raw = serde_json::json!({
            "sessionId": "s",
            "update": {
                "sessionUpdate": "usage_update",
                "used": 100,
                "contextLimit": 200000,
                "accumulatedInputTokens": 80,
                "accumulatedOutputTokens": 20
            }
        });
        let notif: GooseSessionUpdateNotification =
            serde_json::from_value(raw).expect("deserialization");
        match notif.update {
            GooseSessionUpdateVariant::UsageUpdate(p) => {
                assert!(p.accumulated_cost.is_none());
            }
            _ => panic!("expected UsageUpdate"),
        }
    }
}
