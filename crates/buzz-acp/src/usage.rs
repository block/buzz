//! Usage tracking for NIP-AM agent turn metrics.
//!
//! Agents that support usage reporting emit a `_goose/unstable/session/update`
//! notification (with `sessionUpdate: "usage_update"`) at the end of every
//! turn.  Both goose and buzz-agent use this same wire format.  The payload
//! carries session-cumulative token counts from which we derive per-turn
//! deltas.
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
//! Goose may emit **multiple** `usage_update` notifications per turn. The
//! tracker handles this correctly: the committed baseline (and `turn_seq`)
//! advance only when `take()` is called (i.e. at publish time), never on
//! individual notifications. Within a turn all notifications measure their
//! delta from the same frozen baseline — the end of the previous published
//! turn — so the final `pending` record always reflects the full
//! previous-published→current-final delta regardless of how many
//! intermediate notifications arrived.
//!
//! The `TurnUsage` produced after each turn is consumed by the
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
///
/// `used` and `contextLimit` are optional because buzz-agent does not track a
/// context window limit; the fields are present when goose emits them.
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
    UsageUpdate(UsageUpdatePayload),
    #[serde(other)]
    Other,
}

/// The `usage_update` payload.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageUpdatePayload {
    /// Total tokens used (context-usage proxy). Optional — buzz-agent omits
    /// this field or sends 0 because it does not track a context window limit.
    #[serde(default)]
    #[allow(dead_code)]
    pub used: u64,
    /// Context window size. Optional — buzz-agent omits this field.
    #[serde(default)]
    #[allow(dead_code)]
    pub context_limit: u64,
    pub accumulated_input_tokens: u64,
    pub accumulated_output_tokens: u64,
    /// The cache-served subset of `accumulated_input_tokens`.
    ///
    /// `None` when the harness did not include the field (e.g. goose, which
    /// never emits it). `Some(0)` when the harness explicitly reported zero
    /// cache hits. The distinction matters: `None` means "we don't know",
    /// while `Some(0)` means "provider confirmed no cache was used".
    ///
    /// Do NOT use `#[serde(default)]` here — that would collapse the absent
    /// case into `Some(0)` and destroy provenance in the append-only archive.
    pub accumulated_cached_input_tokens: Option<u64>,
    pub accumulated_cost: Option<f64>,
    /// Session-cumulative genuine provider total tokens. Optional — only
    /// emitted by buzz-agent when every turn in the session so far supplied a
    /// provider-reported total. Absent for goose (field ignore-if-absent for
    /// backward compat), for Anthropic-backed turns, and for sessions where any
    /// turn lacked a provider total. NIP-AM forbids deriving this by summing
    /// categories, so the UI must approximate when this field is absent.
    #[serde(default)]
    pub accumulated_total_tokens: Option<u64>,
    /// Effective model id for this turn. Optional — goose payloads that
    /// predate this field deserialize cleanly as `None`.
    #[serde(default)]
    pub model: Option<String>,
}

/// Per-session normalization state: the last cumulative snapshot we saw.
#[derive(Debug, Clone)]
struct SessionState {
    /// Per-session turn counter for the LAST PUBLISHED metric (1-based).
    /// Advanced only when `take()` drains a pending record — not on every
    /// `record()` call. This ensures `turnSeq` counts published metrics, not
    /// usage-update notifications.
    published_seq: u64,
    /// Cumulative input tokens at the end of the LAST PUBLISHED turn.
    /// Advanced only on publish (i.e. in `take()`), not on every notification.
    last_input: u64,
    /// Cumulative output tokens at the end of the LAST PUBLISHED turn.
    last_output: u64,
    /// Cumulative cost at the end of the LAST PUBLISHED turn.
    last_cost: Option<f64>,
    /// Cumulative total tokens at the end of the LAST PUBLISHED turn.
    /// `None` when the session has never emitted a provider total (Unseen) or
    /// when any prior turn lacked one (poisoned).
    last_total: Option<u64>,
    /// Cumulative cache-read input tokens at the end of the LAST PUBLISHED turn.
    /// `None` when the harness has never reported this field (e.g. goose).
    /// `Some(n)` when at least one payload included the field. Field-local:
    /// a decrease in this counter taints only the cache-read delta, not
    /// `delta_reliable` or the input/output deltas.
    last_cached_input: Option<u64>,
}

/// Per-turn usage record exposed to `TurnCompletionGuard` for NIP-AM publishing.
///
/// `turn_*` fields are `None` when delta is unreliable (first turn or counter
/// decrease). `cumulative_*` fields are always present when the agent reports them.
#[derive(Debug, Clone)]
pub struct TurnUsage {
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
    /// Per-turn total token delta; `None` when the cumulative total is
    /// unavailable (no baseline, non-monotonic, or either snapshot was absent).
    /// Field-local: a missing total never flips `delta_reliable` or invalidates
    /// `turn_input_tokens`/`turn_output_tokens`.
    pub turn_total_tokens: Option<u64>,
    /// Per-turn cost delta (`current − previous`); `None` when unreliable or
    /// either snapshot is missing.
    pub turn_cost_usd: Option<f64>,
    /// Per-turn cache-read token delta (`current − previous`); `None` when no
    /// baseline exists, either snapshot is `None` (harness did not report it),
    /// or the cumulative counter decreased (field-local taint). Field-local:
    /// a decrease here never flips `delta_reliable` or invalidates the
    /// input/output deltas.
    pub turn_cache_read_tokens: Option<u64>,
    /// Session-cumulative input tokens as reported by goose at end of turn.
    pub cumulative_input_tokens: u64,
    /// Session-cumulative output tokens as reported by goose at end of turn.
    pub cumulative_output_tokens: u64,
    /// Session-cumulative genuine provider total tokens as reported by buzz-agent;
    /// `None` when the session has never emitted one or any turn lacked one.
    pub cumulative_total_tokens: Option<u64>,
    /// Session-cumulative estimated cost in USD; `None` if goose did not report it.
    pub cumulative_cost_usd: Option<f64>,
    /// Session-cumulative cache-read input tokens as reported by buzz-agent.
    /// `None` when the harness has never reported this field (e.g. goose or
    /// any harness that omits `accumulatedCachedInputTokens`).
    /// `Some(0)` when the harness reported zero cache hits.
    pub cumulative_cache_read_tokens: Option<u64>,
    /// Effective model id for this turn (maps to NIP-AM `model`). `None` if the
    /// harness did not include the model in its usage notification.
    pub model: Option<String>,
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
///    `_goose/unstable/session/update` notification. When in-flight, updates
///    `pending` with the latest cumulative values and a delta measured from
///    the committed baseline (end of the previous published turn). Multiple
///    notifications per turn are fine — the last one wins and `turn_seq` stays
///    constant within the turn. When not in-flight, advances the committed
///    baseline so the next turn can compute a correct delta.
/// 3. **`take()`** — called at turn completion by `TurnCompletionGuard`.
///    Drains and returns the pending record (or `None` if no usage was emitted
///    for this turn), clears the in-flight marker, and advances the committed
///    baseline so the next `record()` call measures from here.
#[derive(Debug, Default)]
pub(crate) struct UsageTracker {
    /// One entry per goose `sessionId` ever seen in this process.
    sessions: HashMap<String, SessionState>,
    /// The session that currently has an in-flight `session/prompt`.
    /// `None` means no prompt is in flight; `record()` will still update
    /// the baseline but will not set `pending`.
    in_flight_session: Option<String>,
    /// The most recently computed turn usage, ready for `take()`.
    pending: Option<TurnUsage>,
}

impl UsageTracker {
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
    /// Behavior depends on which session (if any) is currently in-flight; see
    /// the three explicit cases below. Only a notification for the in-flight
    /// session produces a publishable `pending` record. A notification that
    /// arrives outside any turn (e.g. during `session/new` setup) advances the
    /// committed baseline so the next in-flight turn computes a correct delta.
    /// A notification for a *different* in-flight session is ignored entirely.
    ///
    /// When multiple notifications arrive during the same turn, the **last one
    /// wins** on the cumulative totals, and the delta is always measured from
    /// the baseline at the end of the **previous published turn** — not from an
    /// intermediate notification within the current turn. `turn_seq` stays
    /// constant across all notifications within one turn and only increments
    /// when a record is actually published (i.e. when `take()` is called).
    ///
    /// Three cases:
    /// 1. **In-flight-match** (`in_flight_session == Some(session_id)`): updates
    ///    `pending`. Baseline NOT advanced (that happens on `take()`).
    /// 2. **Not in-flight at all** (`in_flight_session == None`): advances the
    ///    committed baseline (setup notification path).
    /// 3. **In-flight for another session** (`in_flight_session == Some(other)`):
    ///    ignored entirely — touching this session's baseline while another is
    ///    in-flight would undercount this session's next published delta.
    pub(crate) fn record(&mut self, session_id: &str, payload: &UsageUpdatePayload) {
        let current_input = payload.accumulated_input_tokens;
        let current_output = payload.accumulated_output_tokens;
        let current_cost = payload.accumulated_cost;
        let current_total = payload.accumulated_total_tokens;
        let current_cached_input = payload.accumulated_cached_input_tokens;

        // Determine whether this session is currently in-flight so we know
        // whether to set `pending`. We compute the delta regardless so that
        // setup notifications (no in-flight turn) still advance the baseline.
        let is_in_flight = self.in_flight_session.as_deref() == Some(session_id);

        let (delta_reliable, turn_input, turn_output, turn_cost, turn_seq) =
            match self.sessions.get(session_id) {
                None => {
                    // First notification for this session — no baseline yet.
                    (false, None, None, None, 1u64)
                }
                Some(prev) => {
                    // turn_seq for this pending record is one above the last
                    // *published* seq — constant for all notifications in this
                    // turn, advanced only on publish.
                    let seq = prev.published_seq + 1;
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

        // Total-token delta: field-local — never affects `delta_reliable` or
        // the input/output deltas. Null when: no baseline exists, either
        // snapshot is absent, or cumulative total decreased.
        let turn_total = match self.sessions.get(session_id) {
            Some(prev) => match (current_total, prev.last_total) {
                (Some(cur), Some(p)) if cur >= p => Some(cur - p),
                _ => None, // no baseline, absent on either side, or decrease
            },
            None => None, // no baseline yet
        };

        // Cache-read token delta: field-local — never affects `delta_reliable`
        // or the input/output deltas. Null when: no baseline exists, either
        // snapshot is None (harness did not report the field), or the cumulative
        // counter decreased (harness restart, overflow).
        // Some(0) is a valid result when both snapshots are Some(0) — it means
        // the harness confirmed zero cache hits this turn, not that data is absent.
        let turn_cache_read = match self.sessions.get(session_id) {
            Some(prev) => match (current_cached_input, prev.last_cached_input) {
                (Some(cur), Some(p)) if cur >= p => Some(cur - p),
                (Some(_), Some(_)) => None, // decrease → field-local taint
                _ => None,                  // either snapshot absent → no delta
            },
            None => None, // no baseline yet
        };

        if is_in_flight {
            // In-flight-match: update pending with the latest cumulative values.
            // Baseline is NOT advanced here — it advances only on take().
            self.pending = Some(TurnUsage {
                session_id: session_id.to_string(),
                turn_seq,
                delta_reliable,
                turn_input_tokens: turn_input,
                turn_output_tokens: turn_output,
                turn_total_tokens: turn_total,
                turn_cost_usd: turn_cost,
                turn_cache_read_tokens: turn_cache_read,
                cumulative_input_tokens: current_input,
                cumulative_output_tokens: current_output,
                cumulative_total_tokens: current_total,
                cumulative_cost_usd: current_cost,
                cumulative_cache_read_tokens: current_cached_input,
                model: payload.model.clone(),
            });
        } else if self.in_flight_session.is_none() {
            // Not in-flight at all: advance the committed baseline so the next
            // in-flight turn computes its delta from this notification.
            // This handles setup notifications that fire during `session/new`
            // before the first `begin_turn`.
            self.sessions.insert(
                session_id.to_string(),
                SessionState {
                    published_seq: match self.sessions.get(session_id) {
                        Some(s) => s.published_seq,
                        None => 0,
                    },
                    last_input: current_input,
                    last_output: current_output,
                    last_cost: current_cost,
                    last_total: current_total,
                    last_cached_input: current_cached_input,
                },
            );
        }
        // else: in-flight-for-another-session — ignore. A late notification
        // for session X while session Y is in-flight must NOT advance X's
        // committed baseline; doing so would undercount X's next published delta.
    }

    /// Consume and return the most recently computed turn usage record, then
    /// clear the in-flight marker and advance the committed baseline.
    ///
    /// Returns `None` if no `usage_update` arrived during the current in-flight
    /// turn (the agent did not emit usage, or no `begin_turn` was called). The
    /// caller (`TurnCompletionGuard`) must handle `None`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn take(&mut self) -> Option<TurnUsage> {
        self.in_flight_session = None;
        let record = self.pending.take()?;
        // Advance the committed baseline to this published record so the
        // *next* turn measures its delta from here.
        self.sessions.insert(
            record.session_id.clone(),
            SessionState {
                published_seq: record.turn_seq,
                last_input: record.cumulative_input_tokens,
                last_output: record.cumulative_output_tokens,
                last_cost: record.cumulative_cost_usd,
                last_total: record.cumulative_total_tokens,
                last_cached_input: record.cumulative_cache_read_tokens,
            },
        );
        Some(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The camelCase key buzz-agent actually puts on the wire must land on the
    /// field. A rename mismatch here would deserialize to None, and every trial
    /// would be treated as "not reported" — the exact silent failure this field
    /// was added to remove.
    #[test]
    fn cached_input_tokens_deserialize_from_the_wire_key() {
        let p: UsageUpdatePayload = serde_json::from_value(serde_json::json!({
            "used": 15_247,
            "contextLimit": 0,
            "accumulatedInputTokens": 15_091,
            "accumulatedOutputTokens": 156,
            "accumulatedCachedInputTokens": 5_033,
        }))
        .expect("payload must deserialize");
        assert_eq!(p.accumulated_cached_input_tokens, Some(5_033));
        assert!(p.accumulated_cached_input_tokens.unwrap() <= p.accumulated_input_tokens);
    }

    /// goose does not send the field; its payloads must deserialize with None —
    /// not zero — so that "not reported" is preserved distinct from "reported zero".
    #[test]
    fn a_payload_without_the_cache_field_deserializes_as_none() {
        let p: UsageUpdatePayload = serde_json::from_value(serde_json::json!({
            "used": 500,
            "contextLimit": 200_000,
            "accumulatedInputTokens": 400,
            "accumulatedOutputTokens": 100,
        }))
        .expect("payload must deserialize without the cache field");
        assert!(
            p.accumulated_cached_input_tokens.is_none(),
            "absent field must be None, not Some(0)"
        );
    }

    /// A harness that explicitly reports zero cache hits must produce Some(0),
    /// not None — so downstream analytics can distinguish "confirmed zero" from
    /// "not reported".
    #[test]
    fn a_payload_with_explicit_zero_cache_field_deserializes_as_some_zero() {
        let p: UsageUpdatePayload = serde_json::from_value(serde_json::json!({
            "accumulatedInputTokens": 400,
            "accumulatedOutputTokens": 100,
            "accumulatedCachedInputTokens": 0,
        }))
        .expect("payload must deserialize with zero cache field");
        assert_eq!(
            p.accumulated_cached_input_tokens,
            Some(0),
            "explicit zero must be Some(0), not None"
        );
    }

    fn payload(input: u64, output: u64, cost: Option<f64>) -> UsageUpdatePayload {
        UsageUpdatePayload {
            used: input + output,
            context_limit: 200_000,
            accumulated_input_tokens: input,
            accumulated_output_tokens: output,
            accumulated_cached_input_tokens: None,
            accumulated_cost: cost,
            accumulated_total_tokens: None,
            model: None,
        }
    }

    fn payload_no_context(input: u64, output: u64, cost: Option<f64>) -> UsageUpdatePayload {
        UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: input,
            accumulated_output_tokens: output,
            accumulated_cached_input_tokens: None,
            accumulated_cost: cost,
            accumulated_total_tokens: None,
            model: None,
        }
    }

    // ── Turn scoping: setup notifications must not pollute the first real turn ─

    #[test]
    fn setup_notification_before_begin_turn_returns_none() {
        // Regression: setup notifications fire during session/new (before any
        // prompt). They must update the baseline but must NOT produce a
        // publishable record for the next turn.
        let mut tracker = UsageTracker::default();

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
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-a");
        tracker.record("sess-a", &payload(1000, 200, None));

        // Notification for a different session — should not touch pending.
        tracker.record("sess-b", &payload(9000, 3000, None));

        let usage = tracker.take().expect("sess-a pending must survive");
        assert_eq!(usage.session_id, "sess-a");
    }

    #[test]
    fn cross_session_notification_does_not_corrupt_other_sessions_delta() {
        // Regression: A publishes at 1000/100 (turn 1). A late A notification at
        // 1500/150 arrives while session B is in-flight. Under the old `else`
        // branch this would advance A's committed baseline to 1500/150 without
        // publishing a metric, so A's next turn (2000/250) would see a delta of
        // only 500/100 instead of the correct 1000/150.
        //
        // With the fixed three-way branch, the cross-session notification is
        // ignored entirely and A's baseline stays at its last published state.
        let mut tracker = UsageTracker::default();

        // ── Turn A1 — establish A's committed baseline at 1000/100, seq=1 ──
        tracker.begin_turn("sess-a");
        tracker.record("sess-a", &payload(1000, 100, None));
        let a1 = tracker.take().expect("A turn 1");
        assert_eq!(a1.turn_seq, 1);
        assert!(!a1.delta_reliable, "first turn is unreliable");
        assert_eq!(a1.cumulative_input_tokens, 1000);

        // ── B is now in-flight; A late notification arrives ──
        tracker.begin_turn("sess-b");
        // Late A notification while B is in-flight — must NOT advance A's baseline.
        tracker.record("sess-a", &payload(1500, 150, None));
        // B gets its own notification and completes.
        tracker.record("sess-b", &payload(200, 50, None));
        let b1 = tracker.take().expect("B turn 1");
        assert_eq!(b1.session_id, "sess-b");

        // ── Turn A2 — delta must be measured from A's last PUBLISHED baseline ──
        // If the cross-session fix is correct: committed A baseline = 1000/100
        // (from take() after A turn 1), so delta = 2000-1000 = 1000 / 250-100 = 150.
        // If broken (old code): committed A baseline = 1500/150 (wrongly advanced),
        // so delta = 500/100 — the undercount Eva+Wren and Thufir both flagged.
        tracker.begin_turn("sess-a");
        tracker.record("sess-a", &payload(2000, 250, None));
        let a2 = tracker.take().expect("A turn 2");

        assert_eq!(a2.session_id, "sess-a");
        assert_eq!(
            a2.turn_seq, 2,
            "seq must increment per publish, not per notification"
        );
        assert!(a2.delta_reliable, "A turn 2 must have a reliable delta");
        assert_eq!(
            a2.turn_input_tokens,
            Some(1000),
            "A turn 2 delta must be from A's last published baseline (1000), not the \
             late cross-session advance (500)"
        );
        assert_eq!(a2.turn_output_tokens, Some(150));
        assert_eq!(a2.cumulative_input_tokens, 2000);
        assert_eq!(a2.cumulative_output_tokens, 250);
    }

    // ── Delta computation: non-happy paths ─────────────────────────────────

    #[test]
    fn first_turn_no_prior_delta_unreliable() {
        let mut tracker = UsageTracker::default();
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
        let mut tracker = UsageTracker::default();
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
        let mut tracker = UsageTracker::default();
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
        let mut tracker = UsageTracker::default();
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
        let mut tracker = UsageTracker::default();
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
        let mut tracker = UsageTracker::default();
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
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-4");
        tracker.record("sess-4", &payload(100, 20, None));
        let _ = tracker.take();
        assert!(tracker.take().is_none(), "take after drain must be None");
    }

    #[test]
    fn last_update_wins_multiple_updates_same_turn() {
        // Goose emits multiple usage_update notifications per turn. The tracker
        // must:
        // (a) use the LAST notification's cumulative values,
        // (b) measure the delta from the baseline at the END OF THE PREVIOUS
        //     PUBLISHED TURN (not from intermediate notifications), and
        // (c) keep turn_seq constant across all notifications within the turn
        //     (incrementing only on publish, not on each notification).
        let mut tracker = UsageTracker::default();
        // Turn 1 — establish baseline. After take(), committed baseline = 1000/100.
        tracker.begin_turn("sess-5");
        tracker.record("sess-5", &payload(1000, 100, None));
        let t1 = tracker.take().expect("turn 1");
        assert_eq!(t1.turn_seq, 1);

        // Turn 2 — two notifications arrive before take(). The second overwrites
        // the first in pending; delta is measured from the committed baseline
        // (1000/100), not from the intermediate snapshot (1500/150).
        tracker.begin_turn("sess-5");
        tracker.record("sess-5", &payload(1500, 150, None));
        tracker.record("sess-5", &payload(2000, 250, None));
        let usage = tracker.take().expect("turn 2");

        // Cumulative from the last notification.
        assert_eq!(usage.cumulative_input_tokens, 2000);
        assert_eq!(usage.cumulative_output_tokens, 250);
        // Delta is from committed baseline (1000, 100) → (2000, 250) = 1000/150.
        assert_eq!(usage.turn_input_tokens, Some(1000));
        assert_eq!(usage.turn_output_tokens, Some(150));
        // seq increments once per publish, not once per notification.
        assert_eq!(usage.turn_seq, 2);

        // Turn 3 — prove seq continues to increment per publish, not per notification.
        tracker.begin_turn("sess-5");
        tracker.record("sess-5", &payload(2300, 290, None));
        let t3 = tracker.take().expect("turn 3");
        assert_eq!(t3.turn_seq, 3);
        // Delta from turn-2 committed baseline (2000, 250).
        assert_eq!(t3.turn_input_tokens, Some(300));
        assert_eq!(t3.turn_output_tokens, Some(40));
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
    fn notification_deserializes_without_used_and_context_limit() {
        // buzz-agent emits usage_update without used/contextLimit.
        let raw = serde_json::json!({
            "sessionId": "buzz-sess",
            "update": {
                "sessionUpdate": "usage_update",
                "accumulatedInputTokens": 500,
                "accumulatedOutputTokens": 100
            }
        });
        let notif: GooseSessionUpdateNotification =
            serde_json::from_value(raw).expect("deserialization");
        match notif.update {
            GooseSessionUpdateVariant::UsageUpdate(p) => {
                assert_eq!(p.accumulated_input_tokens, 500);
                assert_eq!(p.accumulated_output_tokens, 100);
                assert_eq!(p.used, 0);
                assert_eq!(p.context_limit, 0);
                assert!(p.accumulated_cost.is_none());
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

    #[test]
    fn buzz_agent_notification_flows_through_tracker() {
        // End-to-end: a buzz-agent-shaped usage_update (no used/contextLimit)
        // deserializes and flows through UsageTracker to produce correct TurnUsage.
        let raw1 = serde_json::json!({
            "sessionId": "buzz-s1",
            "update": {
                "sessionUpdate": "usage_update",
                "accumulatedInputTokens": 300,
                "accumulatedOutputTokens": 80
            }
        });
        let raw2 = serde_json::json!({
            "sessionId": "buzz-s1",
            "update": {
                "sessionUpdate": "usage_update",
                "accumulatedInputTokens": 700,
                "accumulatedOutputTokens": 150
            }
        });

        let mut tracker = UsageTracker::default();

        // Turn 1 — first turn, delta unreliable.
        tracker.begin_turn("buzz-s1");
        let notif1: GooseSessionUpdateNotification = serde_json::from_value(raw1).expect("deser");
        if let GooseSessionUpdateVariant::UsageUpdate(p) = notif1.update {
            tracker.record("buzz-s1", &p);
        }
        let t1 = tracker.take().expect("turn 1");
        assert!(!t1.delta_reliable, "first turn: unreliable");
        assert_eq!(t1.cumulative_input_tokens, 300);

        // Turn 2 — delta reliable.
        tracker.begin_turn("buzz-s1");
        let notif2: GooseSessionUpdateNotification = serde_json::from_value(raw2).expect("deser");
        if let GooseSessionUpdateVariant::UsageUpdate(p) = notif2.update {
            tracker.record("buzz-s1", &p);
        }
        let t2 = tracker.take().expect("turn 2");
        assert!(t2.delta_reliable, "second turn: reliable");
        assert_eq!(t2.turn_input_tokens, Some(400)); // 700 - 300
        assert_eq!(t2.turn_output_tokens, Some(70)); // 150 - 80
    }

    #[test]
    fn buzz_agent_payload_no_context_fields_processes_correctly() {
        // UsageTracker handles payloads with used=0 / context_limit=0 correctly.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("s");
        tracker.record("s", &payload_no_context(1000, 200, None));
        let _ = tracker.take();

        tracker.begin_turn("s");
        tracker.record("s", &payload_no_context(1500, 300, None));
        let usage = tracker.take().expect("pending");

        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, Some(500));
        assert_eq!(usage.turn_output_tokens, Some(100));
    }

    #[test]
    fn begin_turn_then_take_without_record_returns_none() {
        // A turn cancelled before the provider emits any tokens: begin_turn is
        // called but no record() arrives before take(). take() must return None.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-precancel");
        let result = tracker.take();
        assert!(
            result.is_none(),
            "take() without any record() must return None (pre-response cancel path)"
        );
    }

    // ── model field threading ────────────────────────────────────────────────

    fn payload_with_model(
        input: u64,
        output: u64,
        cost: Option<f64>,
        model: Option<&str>,
    ) -> UsageUpdatePayload {
        UsageUpdatePayload {
            used: input + output,
            context_limit: 200_000,
            accumulated_input_tokens: input,
            accumulated_output_tokens: output,
            accumulated_cached_input_tokens: None,
            accumulated_cost: cost,
            accumulated_total_tokens: None,
            model: model.map(str::to_string),
        }
    }

    #[test]
    fn model_threads_from_payload_to_turn_usage() {
        // When a `usage_update` payload includes a `model` field, TurnUsage
        // must carry it through so pool.rs can populate the 44200 payload.
        let mut tracker = UsageTracker::default();
        let p = payload_with_model(1000, 200, None, Some("claude-sonnet-4-5"));
        tracker.begin_turn("sess-model");
        tracker.record("sess-model", &p);
        let usage = tracker.take().expect("pending");
        assert_eq!(
            usage.model.as_deref(),
            Some("claude-sonnet-4-5"),
            "model must pass through record() → pending → take()"
        );
    }

    #[test]
    fn model_none_when_payload_omits_model_field() {
        // Goose payloads that predate the `model` field must deserialize cleanly
        // and produce TurnUsage with model = None (no deserialization error,
        // no panic — goose-parity / fail-soft contract).
        let json = r#"{
            "sessionUpdate": "usage_update",
            "accumulatedInputTokens": 500,
            "accumulatedOutputTokens": 100,
            "accumulatedCost": 0.005
        }"#;
        let variant: GooseSessionUpdateVariant =
            serde_json::from_str(json).expect("must deserialize without model field");
        let payload = match variant {
            GooseSessionUpdateVariant::UsageUpdate(p) => p,
            _ => panic!("expected UsageUpdate variant"),
        };
        assert!(
            payload.model.is_none(),
            "model must be None when absent from wire payload"
        );

        // And it should produce a TurnUsage with model = None.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-goose-compat");
        tracker.record("sess-goose-compat", &payload);
        let usage = tracker.take().expect("pending");
        assert!(
            usage.model.is_none(),
            "TurnUsage.model must be None when payload omits the field"
        );
    }

    // ── accumulatedTotalTokens: field-local delta, session poisoning ───────

    fn payload_with_total(input: u64, output: u64, total: Option<u64>) -> UsageUpdatePayload {
        UsageUpdatePayload {
            used: input + output,
            context_limit: 200_000,
            accumulated_input_tokens: input,
            accumulated_output_tokens: output,
            accumulated_cached_input_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: total,
            model: None,
        }
    }

    #[test]
    fn first_update_without_baseline_turn_total_is_none() {
        // No baseline exists → turn total null, but delta_reliable/input/output
        // follow the normal first-turn rule (delta_reliable = false).
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-t1");
        tracker.record("sess-t1", &payload_with_total(100, 20, Some(120)));
        let usage = tracker.take().expect("pending");

        assert!(!usage.delta_reliable, "first turn: delta unreliable");
        assert!(
            usage.turn_total_tokens.is_none(),
            "no baseline → turn total must be None"
        );
        assert_eq!(
            usage.cumulative_total_tokens,
            Some(120),
            "cumulative total passes through even on first turn"
        );
    }

    #[test]
    fn second_turn_with_totals_produces_turn_delta() {
        let mut tracker = UsageTracker::default();
        // Turn 1 — establish baseline.
        tracker.begin_turn("sess-t2");
        tracker.record("sess-t2", &payload_with_total(100, 20, Some(120)));
        let _ = tracker.take();

        // Turn 2 — delta is computable.
        tracker.begin_turn("sess-t2");
        tracker.record("sess-t2", &payload_with_total(200, 50, Some(250)));
        let usage = tracker.take().expect("pending");

        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_total_tokens, Some(130)); // 250 - 120
        assert_eq!(usage.cumulative_total_tokens, Some(250));
    }

    #[test]
    fn cumulative_total_decrease_leaves_turn_total_null_without_affecting_reliability() {
        // Cumulative total decreases (e.g. counter reset) → turn total null,
        // but delta_reliable and input/output are NOT affected (field-local).
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-t3");
        tracker.record("sess-t3", &payload_with_total(500, 100, Some(600)));
        let _ = tracker.take();

        tracker.begin_turn("sess-t3");
        // Cumulative total decreased: 600 → 50.
        tracker.record("sess-t3", &payload_with_total(600, 150, Some(50)));
        let usage = tracker.take().expect("pending");

        assert!(
            usage.delta_reliable,
            "input/output decrease would flip reliability; total decrease must not"
        );
        assert_eq!(usage.turn_input_tokens, Some(100));
        assert_eq!(usage.turn_output_tokens, Some(50));
        assert!(
            usage.turn_total_tokens.is_none(),
            "cumulative total decrease → turn total null (field-local)"
        );
        assert_eq!(
            usage.cumulative_total_tokens,
            Some(50),
            "cumulative total from payload still passes through"
        );
    }

    #[test]
    fn cumulative_total_absent_on_current_turn_leaves_turn_total_null() {
        // Goose-shaped payload: no accumulatedTotalTokens field at all.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-t4");
        tracker.record("sess-t4", &payload_with_total(100, 20, Some(120)));
        let _ = tracker.take();

        // Second turn: goose omits the total field entirely.
        tracker.begin_turn("sess-t4");
        tracker.record("sess-t4", &payload_with_total(200, 50, None));
        let usage = tracker.take().expect("pending");

        assert!(usage.delta_reliable, "input/output delta unaffected");
        assert_eq!(usage.turn_input_tokens, Some(100));
        assert_eq!(usage.turn_output_tokens, Some(30));
        assert!(
            usage.turn_total_tokens.is_none(),
            "absent field → null turn total"
        );
        assert!(
            usage.cumulative_total_tokens.is_none(),
            "absent cumulative total passes through as None"
        );
    }

    #[test]
    fn goose_shaped_payload_without_accumulated_total_deserializes_correctly() {
        // goose payloads lack accumulatedTotalTokens; the field must default
        // to None without a deserialization error (ignore-if-absent contract).
        let json = r#"{
            "sessionUpdate": "usage_update",
            "accumulatedInputTokens": 1000,
            "accumulatedOutputTokens": 200,
            "accumulatedCost": 0.01
        }"#;
        let variant: GooseSessionUpdateVariant =
            serde_json::from_str(json).expect("must deserialize without accumulatedTotalTokens");
        let payload = match variant {
            GooseSessionUpdateVariant::UsageUpdate(p) => p,
            _ => panic!("expected UsageUpdate"),
        };
        assert!(
            payload.accumulated_total_tokens.is_none(),
            "absent accumulatedTotalTokens must default to None"
        );

        // And it must flow through the tracker correctly.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-goose-nototal");
        tracker.record("sess-goose-nototal", &payload);
        let usage = tracker.take().expect("pending");
        assert!(
            usage.cumulative_total_tokens.is_none(),
            "goose-shaped payload must produce None cumulative_total_tokens"
        );
    }

    #[test]
    fn cumulative_total_absent_on_baseline_leaves_turn_total_null_on_second_turn() {
        // Baseline was set without a total (e.g. first goose turn); second
        // turn reports a total. No baseline to diff against → turn total None.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-t5");
        tracker.record("sess-t5", &payload_with_total(100, 20, None)); // no total
        let _ = tracker.take();

        tracker.begin_turn("sess-t5");
        tracker.record("sess-t5", &payload_with_total(200, 50, Some(250)));
        let usage = tracker.take().expect("pending");

        assert!(usage.delta_reliable, "input/output delta unaffected");
        assert!(
            usage.turn_total_tokens.is_none(),
            "absent baseline total → turn total null even when current has a total"
        );
        assert_eq!(usage.cumulative_total_tokens, Some(250));
    }

    // ── cache-read token threading ──────────────────────────────────────────

    fn payload_with_cache(
        input: u64,
        output: u64,
        cached_input: Option<u64>,
    ) -> UsageUpdatePayload {
        UsageUpdatePayload {
            used: input + output,
            context_limit: 200_000,
            accumulated_input_tokens: input,
            accumulated_output_tokens: output,
            accumulated_cached_input_tokens: cached_input,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
        }
    }

    #[test]
    fn cache_read_first_turn_produces_none_turn_delta_and_passes_cumulative_through() {
        // First turn has no baseline → turn cache delta must be None, but
        // cumulative_cache_read_tokens must carry the reported value through.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-c1");
        tracker.record("sess-c1", &payload_with_cache(1000, 200, Some(500)));
        let usage = tracker.take().expect("pending");

        assert!(
            usage.turn_cache_read_tokens.is_none(),
            "first turn: no baseline → cache delta must be None"
        );
        assert_eq!(
            usage.cumulative_cache_read_tokens,
            Some(500),
            "cumulative cache read passes through on first turn"
        );
        assert!(!usage.delta_reliable, "first turn is unreliable");
    }

    #[test]
    fn cache_read_second_turn_delta_computed_correctly() {
        // Second turn: cumulative cached 500 → 1200, delta = 700.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-c2");
        tracker.record("sess-c2", &payload_with_cache(1000, 200, Some(500)));
        let _ = tracker.take();

        tracker.begin_turn("sess-c2");
        tracker.record("sess-c2", &payload_with_cache(2000, 350, Some(1200)));
        let usage = tracker.take().expect("pending");

        assert!(usage.delta_reliable);
        assert_eq!(
            usage.turn_cache_read_tokens,
            Some(700),
            "cache delta = 1200 - 500 = 700"
        );
        assert_eq!(
            usage.cumulative_cache_read_tokens,
            Some(1200),
            "cumulative cache passes through"
        );
    }

    #[test]
    fn cache_read_decrease_nulls_turn_cache_but_leaves_delta_reliable() {
        // Cache counter decrease → cache delta None (field-local taint), but
        // delta_reliable and input/output deltas are NOT affected.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-c3");
        tracker.record("sess-c3", &payload_with_cache(1000, 200, Some(800)));
        let _ = tracker.take();

        tracker.begin_turn("sess-c3");
        // Cache counter decreased: 800 → 50.
        tracker.record("sess-c3", &payload_with_cache(1500, 300, Some(50)));
        let usage = tracker.take().expect("pending");

        assert!(
            usage.delta_reliable,
            "cache decrease must NOT flip delta_reliable — field-local"
        );
        assert_eq!(
            usage.turn_input_tokens,
            Some(500),
            "input/output delta unaffected by cache decrease"
        );
        assert_eq!(usage.turn_output_tokens, Some(100));
        assert!(
            usage.turn_cache_read_tokens.is_none(),
            "cache counter decrease → turn_cache_read_tokens None (field-local taint)"
        );
        assert_eq!(
            usage.cumulative_cache_read_tokens,
            Some(50),
            "cumulative still passes through from payload even on decrease"
        );
    }

    #[test]
    fn cache_read_explicit_zero_payload_after_explicit_zero_baseline_produces_some_zero_delta() {
        // When both baseline and current are Some(0), turn_cache_read_tokens must
        // be Some(0) — confirmed zero, not absent.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-c4");
        tracker.record("sess-c4", &payload_with_cache(1000, 200, Some(0)));
        let _ = tracker.take();

        tracker.begin_turn("sess-c4");
        tracker.record("sess-c4", &payload_with_cache(1500, 300, Some(0)));
        let usage = tracker.take().expect("pending");

        assert!(usage.delta_reliable);
        assert_eq!(
            usage.turn_cache_read_tokens,
            Some(0),
            "explicit zero on both sides → Some(0), not None"
        );
        assert_eq!(usage.cumulative_cache_read_tokens, Some(0));
    }

    #[test]
    fn cache_read_threads_through_setup_notification_baseline() {
        // A setup notification (before begin_turn) with a nonzero cache count
        // must update the committed baseline so the first real turn gets a
        // correct delta from that starting point.
        let mut tracker = UsageTracker::default();

        // Setup notification: cumulative cache = 300.
        tracker.record("sess-c5", &payload_with_cache(1000, 200, Some(300)));

        tracker.begin_turn("sess-c5");
        tracker.record("sess-c5", &payload_with_cache(1500, 350, Some(700)));
        let usage = tracker.take().expect("pending");

        assert!(usage.delta_reliable, "baseline from setup: reliable");
        assert_eq!(
            usage.turn_cache_read_tokens,
            Some(400),
            "cache delta from setup baseline: 700 - 300 = 400"
        );
        assert_eq!(usage.cumulative_cache_read_tokens, Some(700));
    }

    #[test]
    fn cache_read_omitted_field_produces_none_cumulative_and_no_turn_delta() {
        // A harness that omits accumulatedCachedInputTokens (e.g. goose) must
        // produce None cumulative_cache_read_tokens — not Some(0) — and the
        // turn delta must also be None even on the second turn.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-c6");
        // payload() uses None for accumulated_cached_input_tokens.
        tracker.record("sess-c6", &payload(1000, 200, None));
        let t1 = tracker.take().expect("turn 1");

        assert!(
            t1.cumulative_cache_read_tokens.is_none(),
            "goose-shaped payload: cumulative must be None, not Some(0)"
        );
        assert!(
            t1.turn_cache_read_tokens.is_none(),
            "first turn always has no turn delta"
        );

        tracker.begin_turn("sess-c6");
        tracker.record("sess-c6", &payload(1500, 300, None));
        let t2 = tracker.take().expect("turn 2");

        assert!(
            t2.cumulative_cache_read_tokens.is_none(),
            "continued goose session: cumulative must remain None"
        );
        assert!(
            t2.turn_cache_read_tokens.is_none(),
            "absent field on both sides → no turn delta invented"
        );
        assert!(
            t2.delta_reliable,
            "input/output reliability unaffected by absent cache field"
        );
    }

    #[test]
    fn cache_read_baseline_absent_then_present_produces_no_delta() {
        // If the first turn omits the cache field (baseline stored as None) and
        // the second turn reports a value, no delta can be computed — we have no
        // baseline to subtract from. The cumulative value should still pass through.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-c7");
        tracker.record("sess-c7", &payload(1000, 200, None)); // no cache field
        let _ = tracker.take();

        tracker.begin_turn("sess-c7");
        tracker.record("sess-c7", &payload_with_cache(1500, 300, Some(400)));
        let usage = tracker.take().expect("turn 2");

        assert!(
            usage.turn_cache_read_tokens.is_none(),
            "absent baseline → no turn delta even when current has a value"
        );
        assert_eq!(
            usage.cumulative_cache_read_tokens,
            Some(400),
            "cumulative from current payload passes through"
        );
        assert!(usage.delta_reliable, "input/output reliability unaffected");
    }

    #[test]
    fn cache_read_baseline_present_then_absent_produces_no_delta() {
        // If the first turn reports the cache field but the second omits it
        // (harness switched), no delta should be produced and cumulative is None.
        let mut tracker = UsageTracker::default();
        tracker.begin_turn("sess-c8");
        tracker.record("sess-c8", &payload_with_cache(1000, 200, Some(300)));
        let _ = tracker.take();

        tracker.begin_turn("sess-c8");
        tracker.record("sess-c8", &payload(1500, 300, None)); // no cache field
        let usage = tracker.take().expect("turn 2");

        assert!(
            usage.turn_cache_read_tokens.is_none(),
            "absent current → no turn delta"
        );
        assert!(
            usage.cumulative_cache_read_tokens.is_none(),
            "absent field: cumulative must be None"
        );
        assert!(usage.delta_reliable, "input/output reliability unaffected");
    }

    #[test]
    fn pool_omitted_cache_field_publishes_no_cache_read_tokens_in_kind44200() {
        // End-to-end: a buzz-agent or goose payload that omits the cache field
        // must NOT publish cacheReadTokens in the kind:44200 event — neither
        // in turn nor cumulative counts.
        //
        // This is the core acceptance test for Thufir's finding: the old code
        // would publish cacheReadTokens: 0 for every harness regardless of
        // whether the field was reported.
        use crate::pool::build_turn_metric_counts;

        let usage = TurnUsage {
            session_id: "sess-pool-none".into(),
            turn_seq: 2,
            delta_reliable: true,
            turn_input_tokens: Some(400),
            turn_output_tokens: Some(100),
            turn_total_tokens: None,
            turn_cost_usd: None,
            turn_cache_read_tokens: None,
            cumulative_input_tokens: 700,
            cumulative_output_tokens: 200,
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None, // harness did not report the field
            model: None,
        };

        let (turn_counts, cumulative_counts) = build_turn_metric_counts(&usage);

        let turn = turn_counts.expect("turn counts must be present (delta reliable)");
        assert!(
            turn.cache_read_tokens.is_none(),
            "omitted cache field: turn cacheReadTokens must be absent from kind:44200"
        );

        let cumulative = cumulative_counts.expect("cumulative counts always present");
        assert!(
            cumulative.cache_read_tokens.is_none(),
            "omitted cache field: cumulative cacheReadTokens must be absent from kind:44200"
        );
    }

    #[test]
    fn pool_reported_cache_field_publishes_nonzero_cache_read_tokens_in_kind44200() {
        // End-to-end: a buzz-agent payload with a nonzero cache count must
        // publish cacheReadTokens in both turn and cumulative counts.
        use crate::pool::build_turn_metric_counts;

        let usage = TurnUsage {
            session_id: "sess-pool-some".into(),
            turn_seq: 2,
            delta_reliable: true,
            turn_input_tokens: Some(400),
            turn_output_tokens: Some(100),
            turn_total_tokens: None,
            turn_cost_usd: None,
            turn_cache_read_tokens: Some(300),
            cumulative_input_tokens: 700,
            cumulative_output_tokens: 200,
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: Some(600),
            model: None,
        };

        let (turn_counts, cumulative_counts) = build_turn_metric_counts(&usage);

        let turn = turn_counts.expect("turn counts present");
        assert_eq!(
            turn.cache_read_tokens,
            Some(300),
            "nonzero turn cache: must appear in kind:44200 turn counts"
        );

        let cumulative = cumulative_counts.expect("cumulative counts present");
        assert_eq!(
            cumulative.cache_read_tokens,
            Some(600),
            "nonzero cumulative cache: must appear in kind:44200 cumulative counts"
        );
    }
}
