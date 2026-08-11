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
    UsageUpdate(Box<UsageUpdatePayload>),
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
    /// Session-cumulative inclusive input tokens.
    ///
    /// `None` when buzz-agent omitted the field — this happens when the
    /// session-cumulative sum overflowed `u64::MAX`.  Goose always emits this
    /// field, so `None` from goose is not expected; `#[serde(default)]` keeps
    /// backward compatibility with any producer that omits it.
    #[serde(default)]
    pub accumulated_input_tokens: Option<u64>,
    /// Session-cumulative output tokens.
    ///
    /// Same overflow-omit contract as `accumulated_input_tokens`.
    #[serde(default)]
    pub accumulated_output_tokens: Option<u64>,
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
    /// The cache-written subset of `accumulated_input_tokens`.
    ///
    /// `None` when the harness did not include the field (e.g. goose or any
    /// provider that does not report cache-write tokens). `Some(0)` when the
    /// harness explicitly reported zero cache writes. Same absence-vs-zero
    /// semantics as `accumulated_cached_input_tokens` above.
    ///
    /// Do NOT use `#[serde(default)]` here for the same reason.
    pub accumulated_cache_write_tokens: Option<u64>,
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
    /// Billing identity as stamped by the publisher. Optional — absent when
    /// the publisher could not prove applicability (unrecognised endpoint,
    /// mixed identities within the turn, etc.). Old harnesses that do not emit
    /// this field deserialise to `None` cleanly via `#[serde(default)]`.
    ///
    /// Do NOT use this value directly to advance the session-cumulative
    /// baseline: it is per-turn only and must not persist to `SessionState`.
    #[serde(default)]
    pub pricing_identity: Option<buzz_core::agent_turn_metric::PricingIdentity>,
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
    /// `None` when the publisher omitted the field in a prior turn.
    last_input: Option<u64>,
    /// Cumulative output tokens at the end of the LAST PUBLISHED turn.
    /// `None` when the publisher omitted the field in a prior turn.
    last_output: Option<u64>,
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
    /// Cumulative cache-write tokens at the end of the LAST PUBLISHED turn.
    /// `None` when the harness has never reported this field. Field-local:
    /// a decrease taints only the cache-write delta, not `delta_reliable`.
    last_cache_write: Option<u64>,
    /// Sticky poison flag for the input field: set the first time ACP observes
    /// an absent `accumulated_input_tokens` snapshot for this session and never
    /// cleared.  Once true, `delta_reliable` stays false for every subsequent
    /// turn regardless of whether the publisher later resumes emitting the
    /// field.  ACP cannot trust the producer's permanence guarantee.
    input_ever_poisoned: bool,
    /// Sticky poison flag for the output field: same contract as
    /// `input_ever_poisoned` but for `accumulated_output_tokens`.
    output_ever_poisoned: bool,
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
    /// Per-turn cache-write token delta (`current − previous`); `None` when no
    /// baseline exists, either snapshot is `None`, or the counter decreased.
    /// Field-local — same contract as `turn_cache_read_tokens`.
    pub turn_cache_write_tokens: Option<u64>,
    /// Session-cumulative input tokens as reported by goose at end of turn.
    /// `None` when the publisher omitted the field (overflow-poisoned session).
    pub cumulative_input_tokens: Option<u64>,
    /// Session-cumulative output tokens as reported by goose at end of turn.
    /// `None` when the publisher omitted the field (overflow-poisoned session).
    pub cumulative_output_tokens: Option<u64>,
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
    /// Session-cumulative cache-write tokens as reported by buzz-agent.
    /// `None` when the harness has never reported this field.
    /// `Some(0)` when the harness reported zero cache writes.
    pub cumulative_cache_write_tokens: Option<u64>,
    /// Effective model id for this turn (maps to NIP-AM `model`). `None` if the
    /// harness did not include the model in its usage notification.
    pub model: Option<String>,
    /// Billing identity for this turn, as received from the publisher.
    /// `None` when the publisher omitted it (unrecognised endpoint, mixed
    /// identities, old harness). Per-turn only — not session-cumulative.
    pub pricing_identity: Option<buzz_core::agent_turn_metric::PricingIdentity>,
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
    /// Per-in-flight-turn identity accumulator — three-state:
    ///   `None`             = no usage notification yet (initial / after begin_turn)
    ///   `Some(Some(pi))`   = all notifications so far carry the same proven identity
    ///   `Some(None)`       = poisoned (mismatch, absent on a token-advancing
    ///                        notification, or first notification had no identity)
    ///
    /// Folded on every in-flight `record()` call (last-update-wins is the
    /// wrong contract for cumulative-snapshot notifications — a later
    /// notification that carries A after an unproven/absent one must NOT
    /// resurrect the identity). Reset to `None` in `begin_turn()` and `take()`.
    pending_identity: Option<Option<buzz_core::agent_turn_metric::PricingIdentity>>,
    /// Per-in-flight-turn fold accumulator for input-field absence.
    ///
    /// Set to `true` the first time any in-flight `record()` call for the
    /// current turn observes `accumulated_input_tokens: None`. Monotonically
    /// grows (never cleared mid-turn); reset to `false` by `begin_turn()`.
    /// At `take()` this value is OR-ed into the session's `input_ever_poisoned`
    /// flag, creating or updating the session entry as needed. If `take()` is
    /// never called before the next `begin_turn()`, the flush is performed at
    /// `begin_turn()` time instead, so no observed absence is ever discarded.
    input_absence_observed: bool,
    /// Per-in-flight-turn fold accumulator for output-field absence.
    /// Symmetric contract to `input_absence_observed`.
    output_absence_observed: bool,
}

impl UsageTracker {
    /// Mark the start of a new prompt turn for `session_id`.
    ///
    /// Clears any leftover `pending` record and records which session is
    /// in-flight. Must be called before the corresponding `session/prompt`
    /// request is sent so that setup notifications received before this call
    /// do not become publishable for this turn.
    ///
    /// If the previous turn's fold accumulators hold observed absences and
    /// `take()` was never called (e.g. the initial-message path calls
    /// `begin_turn` twice without a `take()` between them), those absences are
    /// committed here into the previous in-flight session's sticky
    /// `*_ever_poisoned` state before the accumulators are reset.  This
    /// prevents the "take-skipped turn" escape: observed absences are
    /// impossible to discard regardless of whether `take()` was called.
    pub(crate) fn begin_turn(&mut self, session_id: &str) {
        // Flush any outstanding fold state from the previous in-flight turn
        // into the previous session's entry BEFORE resetting the accumulators.
        //
        // This closes two discard points:
        //   1. **Take-skipped same-session** — `begin_turn("s")` called twice
        //      without a `take()` in between (the initial-message path in
        //      pool.rs does exactly this).
        //   2. **Cross-session** — session A's turn observed absences, then
        //      `begin_turn("B")` runs next.  A's poison must survive.
        //
        // The fold accumulators can only be non-false when `in_flight_session`
        // is Some, because only in-flight `record()` calls set them.  The outer
        // guard is a performance short-circuit (skip the map lookup on the
        // common no-absence path); correctness does not depend on it.
        if self.input_absence_observed || self.output_absence_observed {
            if let Some(ref prev_session) = self.in_flight_session {
                let key = prev_session.clone();
                let existing = self.sessions.get(key.as_str());
                let input_ever_poisoned =
                    existing.is_some_and(|s| s.input_ever_poisoned) || self.input_absence_observed;
                let output_ever_poisoned = existing.is_some_and(|s| s.output_ever_poisoned)
                    || self.output_absence_observed;
                let (published_seq, last_input, last_output, last_cost, last_total, lci, lcw) =
                    match existing {
                        Some(s) => (
                            s.published_seq,
                            s.last_input,
                            s.last_output,
                            s.last_cost,
                            s.last_total,
                            s.last_cached_input,
                            s.last_cache_write,
                        ),
                        None => (0, None, None, None, None, None, None),
                    };
                self.sessions.insert(
                    key,
                    SessionState {
                        published_seq,
                        last_input,
                        last_output,
                        last_cost,
                        last_total,
                        last_cached_input: lci,
                        last_cache_write: lcw,
                        input_ever_poisoned,
                        output_ever_poisoned,
                    },
                );
            }
        }
        self.in_flight_session = Some(session_id.to_string());
        self.pending = None;
        self.pending_identity = None;
        self.input_absence_observed = false;
        self.output_absence_observed = false;
    }

    /// Process a `usage_update` notification payload.
    ///
    /// Behavior depends on which session (if any) is currently in-flight; see
    /// the three explicit cases below. Only a notification for the in-flight
    /// session produces a publishable `pending` record. A notification that
    /// arrives outside any turn (e.g. during `session/new` setup) advances the
    /// committed baseline so the next in-flight turn computes a correct delta.
    /// A notification for a *different* in-flight session drops its counters
    /// (advancing the baseline would undercount that session's next turn) but
    /// latches any observed input/output absence into that session's committed
    /// `*_ever_poisoned` state — the sticky-absence contract has no cross-session
    /// exemption.
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
    ///    counters are dropped (advancing this session's baseline would undercount
    ///    its next published delta), but any observed input/output absence is
    ///    latched into this session's `*_ever_poisoned` state — the sticky-absence
    ///    contract has no cross-session exemption.
    pub(crate) fn record(&mut self, session_id: &str, payload: &UsageUpdatePayload) {
        let current_input = payload.accumulated_input_tokens;
        let current_output = payload.accumulated_output_tokens;
        let current_cost = payload.accumulated_cost;
        let current_total = payload.accumulated_total_tokens;
        let current_cached_input = payload.accumulated_cached_input_tokens;
        let current_cache_write = payload.accumulated_cache_write_tokens;

        // Determine whether this session is currently in-flight so we know
        // whether to set `pending`. We compute the delta regardless so that
        // setup notifications (no in-flight turn) still advance the baseline.
        let is_in_flight = self.in_flight_session.as_deref() == Some(session_id);

        // For in-flight notifications, fold the absence of each field into the
        // per-turn accumulators BEFORE computing the delta.  This ensures the
        // second `record()` call in a turn sees the absence observed by the first,
        // even when no session entry exists yet (un-baselined path) and even when
        // the second notification reintroduces the field.  The fold is monotonic
        // (OR — never cleared mid-turn); it is reset by `begin_turn()` and
        // committed to the session's sticky flags in `take()`.
        //
        // Case 3 (in-flight for another session) is handled separately in the
        // `else` branch below: counters are dropped, but any observed absence is
        // latched directly into that session's committed `*_ever_poisoned` state.
        if is_in_flight {
            if current_input.is_none() {
                self.input_absence_observed = true;
            }
            if current_output.is_none() {
                self.output_absence_observed = true;
            }
        }

        let (delta_reliable, turn_input, turn_output, turn_cost, turn_seq) = match self
            .sessions
            .get(session_id)
        {
            None => {
                // First notification for this session — no baseline yet.
                (false, None, None, None, 1u64)
            }
            Some(prev) => {
                // turn_seq for this pending record is one above the last
                // *published* seq — constant for all notifications in this
                // turn, advanced only on publish.
                let seq = prev.published_seq + 1;
                // Sticky-poison check: if ACP ever observed an absent input
                // or output snapshot for this session, delta_reliable is
                // permanently false.  A later reintroduced value must NOT
                // heal the reliability — ACP cannot trust the producer's
                // permanence guarantee; the prefix delta is irrecoverably
                // unknown.
                //
                // Three sources of poison — all monotonic (OR):
                //   1. The session's committed flag from prior turns.
                //   2. The per-turn fold accumulator (captures absences seen
                //      earlier in THIS turn before take() commits them).
                //   3. Whether THIS notification is itself absent.
                let this_input_absent = current_input.is_none();
                let this_output_absent = current_output.is_none();
                let input_poisoned =
                    prev.input_ever_poisoned || self.input_absence_observed || this_input_absent;
                let output_poisoned =
                    prev.output_ever_poisoned || self.output_absence_observed || this_output_absent;
                if input_poisoned || output_poisoned {
                    (false, None, None, None, seq)
                } else {
                    match (
                        current_input,
                        current_output,
                        prev.last_input,
                        prev.last_output,
                    ) {
                        (Some(ci), Some(co), Some(pi), Some(po)) => {
                            // Token counter decrease → unreliable delta.
                            if ci < pi || co < po {
                                (false, None, None, None, seq)
                            } else {
                                let di = ci - pi;
                                let dout = co - po;
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
                        // One or both sides absent (no prior baseline) → unreliable.
                        _ => (false, None, None, None, seq),
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

        // Cache-write token delta: same field-local contract as cache-read.
        let turn_cache_write = match self.sessions.get(session_id) {
            Some(prev) => match (current_cache_write, prev.last_cache_write) {
                (Some(cur), Some(p)) if cur >= p => Some(cur - p),
                (Some(_), Some(_)) => None, // decrease → field-local taint
                _ => None,                  // either snapshot absent → no delta
            },
            None => None, // no baseline yet
        };

        if is_in_flight {
            // In-flight-match: update pending with the latest cumulative values.
            // Baseline is NOT advanced here — it advances only on take().
            //
            // Fold the per-notification identity into the per-turn accumulator.
            // Last-update-wins is wrong for cumulative-snapshot notifications: a
            // later notification that carries a proven identity A after an
            // absent/unproven one must NOT resurrect the identity.
            //
            // Fold contract (mirrors the publisher-side `fold_pricing_identity`):
            // - `None` acc (first notification): adopt whatever the payload carries.
            // - `Some(Some(pi))` acc: if this notification matches exactly, keep;
            //   otherwise poison to `Some(None)`.
            // - `Some(None)` acc (poisoned): stays poisoned, no healing.
            let incoming = payload.pricing_identity.clone();
            self.pending_identity = match self.pending_identity.take() {
                // First in-flight notification: adopt the payload identity.
                None => Some(incoming),
                // Already consistent: keep only if this notification matches exactly.
                Some(Some(ref existing)) => {
                    if Some(existing) == incoming.as_ref() {
                        Some(incoming)
                    } else {
                        // Mismatch (different identity, absent, or unproven) → poison.
                        Some(None)
                    }
                }
                // Already poisoned: stays poisoned regardless of this notification.
                poisoned @ Some(None) => poisoned,
            };
            self.pending = Some(TurnUsage {
                session_id: session_id.to_string(),
                turn_seq,
                delta_reliable,
                turn_input_tokens: turn_input,
                turn_output_tokens: turn_output,
                turn_total_tokens: turn_total,
                turn_cost_usd: turn_cost,
                turn_cache_read_tokens: turn_cache_read,
                turn_cache_write_tokens: turn_cache_write,
                cumulative_input_tokens: current_input,
                cumulative_output_tokens: current_output,
                cumulative_total_tokens: current_total,
                cumulative_cost_usd: current_cost,
                cumulative_cache_read_tokens: current_cached_input,
                cumulative_cache_write_tokens: current_cache_write,
                model: payload.model.clone(),
                // The folded identity is written in take() — use a placeholder
                // here and replace it before returning the record.
                pricing_identity: None,
            });
        } else if self.in_flight_session.is_none() {
            // Not in-flight at all: advance the committed baseline so the next
            // in-flight turn computes its delta from this notification.
            // This handles setup notifications that fire during `session/new`
            // before the first `begin_turn`.
            //
            // Carry forward any existing sticky-poison flags (they only grow).
            let existing = self.sessions.get(session_id);
            let input_ever_poisoned =
                existing.is_some_and(|s| s.input_ever_poisoned) || current_input.is_none();
            let output_ever_poisoned =
                existing.is_some_and(|s| s.output_ever_poisoned) || current_output.is_none();
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
                    last_cache_write: current_cache_write,
                    input_ever_poisoned,
                    output_ever_poisoned,
                },
            );
        } else {
            // In-flight-for-another-session — counters are dropped; absence is
            // latched.  Advancing X's baseline while Y is in-flight would
            // undercount X's next published delta, so counters stay unchanged.
            // But the sticky-absence contract has no cross-session exemption: if
            // this notification is absent, that observation must survive into X's
            // next in-flight turn even though the record is otherwise discarded.
            let input_absent = current_input.is_none();
            let output_absent = current_output.is_none();
            if input_absent || output_absent {
                let existing = self.sessions.get(session_id);
                let input_ever_poisoned =
                    existing.is_some_and(|s| s.input_ever_poisoned) || input_absent;
                let output_ever_poisoned =
                    existing.is_some_and(|s| s.output_ever_poisoned) || output_absent;
                let (published_seq, last_input, last_output, last_cost, last_total, lci, lcw) =
                    match existing {
                        Some(s) => (
                            s.published_seq,
                            s.last_input,
                            s.last_output,
                            s.last_cost,
                            s.last_total,
                            s.last_cached_input,
                            s.last_cache_write,
                        ),
                        None => (0, None, None, None, None, None, None),
                    };
                self.sessions.insert(
                    session_id.to_string(),
                    SessionState {
                        published_seq,
                        last_input,
                        last_output,
                        last_cost,
                        last_total,
                        last_cached_input: lci,
                        last_cache_write: lcw,
                        input_ever_poisoned,
                        output_ever_poisoned,
                    },
                );
            }
        }
    }

    /// Seed a zero baseline for a session that buzz-acp just spawned.
    ///
    /// When buzz-acp creates a session itself via `session/new`, the session's
    /// prior token usage is zero by definition — no provider calls have been
    /// made yet.  Seeding a zero baseline here means the first usage
    /// notification for this session will see `current − 0 == cumulative` and
    /// can emit `delta_reliable: true` with `turn.* == cumulative.*`.
    ///
    /// This must be called **only** from the code path that issues `session/new`
    /// (i.e. `create_session_and_apply_model` in `pool.rs`).  It must **not** be
    /// called when attaching to a pre-existing session whose prior usage is
    /// genuinely unknown — that case correctly stays fail-closed with the
    /// existing no-baseline behavior.
    ///
    /// No-op if a baseline for this session already exists (guards against
    /// accidental double-seeding across session rotation).
    pub(crate) fn seed_zero_baseline(&mut self, session_id: &str) {
        self.sessions
            .entry(session_id.to_string())
            .or_insert(SessionState {
                published_seq: 0,
                last_input: Some(0),
                last_output: Some(0),
                last_cost: Some(0.0),
                // At spawn all counters are zero — seed known-zero baselines so
                // the first real turn delta is computed exactly, not discarded as
                // "no prior baseline".  Cache values use the same argument as
                // input/output: a freshly-spawned session has accumulated nothing,
                // so the provider-reported cumulative IS the turn delta.
                last_total: Some(0),
                last_cached_input: Some(0),
                last_cache_write: Some(0),
                // A freshly-spawned session has no prior absence — poison flags
                // start clear and are set only if a subsequent snapshot is absent.
                input_ever_poisoned: false,
                output_ever_poisoned: false,
            });
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
        // Consume the folded identity accumulator: emit the proven identity when
        // every in-flight notification carried the same one; emit `None` when
        // any notification was absent/unproven or they disagreed.
        let folded_identity = self.pending_identity.take().and_then(|inner| inner);
        // Consume and reset the per-turn fold accumulators before returning.
        // These must be reset even on the None path (no pending record) so a
        // subsequent begin_turn/take cycle starts clean.
        let input_absence_this_turn = std::mem::replace(&mut self.input_absence_observed, false);
        let output_absence_this_turn = std::mem::replace(&mut self.output_absence_observed, false);
        let mut record = self.pending.take()?;
        record.pricing_identity = folded_identity;
        // Advance the committed baseline to this published record so the
        // *next* turn measures its delta from here.
        //
        // Compute sticky-poison flags by combining three sources — all monotonic:
        //   1. Any prior session-level flag (from a previous turn).
        //   2. `input_absence_this_turn` / `output_absence_this_turn` — whether any
        //      in-flight notification this turn observed an absent field.  This is
        //      the fold accumulator that closes the un-baselined escape: for sessions
        //      on the attach-to-existing path (no `seed_zero_baseline`), no session
        //      entry exists yet, so a `get_mut`-based latch would be a no-op — the
        //      fold captures the absence regardless and commits it here.
        //   3. Whether the final published record's cumulative field is None (the
        //      last-notification check that was already present).
        let existing = self.sessions.get(&record.session_id);
        let input_ever_poisoned = existing.is_some_and(|s| s.input_ever_poisoned)
            || input_absence_this_turn
            || record.cumulative_input_tokens.is_none();
        let output_ever_poisoned = existing.is_some_and(|s| s.output_ever_poisoned)
            || output_absence_this_turn
            || record.cumulative_output_tokens.is_none();
        self.sessions.insert(
            record.session_id.clone(),
            SessionState {
                published_seq: record.turn_seq,
                last_input: record.cumulative_input_tokens,
                last_output: record.cumulative_output_tokens,
                last_cost: record.cumulative_cost_usd,
                last_total: record.cumulative_total_tokens,
                last_cached_input: record.cumulative_cache_read_tokens,
                last_cache_write: record.cumulative_cache_write_tokens,
                input_ever_poisoned,
                output_ever_poisoned,
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
        assert!(p.accumulated_cached_input_tokens.unwrap() <= p.accumulated_input_tokens.unwrap());
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
            accumulated_input_tokens: Some(input),
            accumulated_output_tokens: Some(output),
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: cost,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
        }
    }

    fn payload_no_context(input: u64, output: u64, cost: Option<f64>) -> UsageUpdatePayload {
        UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: Some(input),
            accumulated_output_tokens: Some(output),
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: cost,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
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
        // With the fixed three-way branch, the cross-session notification drops
        // its counters (advancing A's baseline would undercount A's next turn)
        // and latches any observed absence into A's committed poison flags.
        let mut tracker = UsageTracker::default();

        // ── Turn A1 — establish A's committed baseline at 1000/100, seq=1 ──
        tracker.begin_turn("sess-a");
        tracker.record("sess-a", &payload(1000, 100, None));
        let a1 = tracker.take().expect("A turn 1");
        assert_eq!(a1.turn_seq, 1);
        assert!(!a1.delta_reliable, "first turn is unreliable");
        assert_eq!(a1.cumulative_input_tokens, Some(1000));

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
        assert_eq!(a2.cumulative_input_tokens, Some(2000));
        assert_eq!(a2.cumulative_output_tokens, Some(250));
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
        assert_eq!(usage.cumulative_input_tokens, Some(1000));
        assert_eq!(usage.cumulative_output_tokens, Some(200));
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
        assert_eq!(usage.cumulative_input_tokens, Some(1500));
        assert_eq!(usage.cumulative_output_tokens, Some(350));
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
        assert_eq!(usage.cumulative_input_tokens, Some(1800));
        assert_eq!(usage.cumulative_output_tokens, Some(450));
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
        assert_eq!(usage.cumulative_input_tokens, Some(2000));
        assert_eq!(usage.cumulative_output_tokens, Some(250));
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
                assert_eq!(p.accumulated_input_tokens, Some(40000));
                assert_eq!(p.accumulated_output_tokens, Some(10000));
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
                assert_eq!(p.accumulated_input_tokens, Some(500));
                assert_eq!(p.accumulated_output_tokens, Some(100));
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
        assert_eq!(t1.cumulative_input_tokens, Some(300));

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
            accumulated_input_tokens: Some(input),
            accumulated_output_tokens: Some(output),
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: cost,
            accumulated_total_tokens: None,
            model: model.map(str::to_string),
            pricing_identity: None,
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
            accumulated_input_tokens: Some(input),
            accumulated_output_tokens: Some(output),
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: total,
            model: None,
            pricing_identity: None,
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
            accumulated_input_tokens: Some(input),
            accumulated_output_tokens: Some(output),
            accumulated_cached_input_tokens: cached_input,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
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
            turn_cache_write_tokens: None,
            cumulative_input_tokens: Some(700),
            cumulative_output_tokens: Some(200),
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None, // harness did not report the field
            cumulative_cache_write_tokens: None,
            model: None,
            pricing_identity: None,
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
            turn_cache_write_tokens: None,
            cumulative_input_tokens: Some(700),
            cumulative_output_tokens: Some(200),
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: Some(600),
            cumulative_cache_write_tokens: None,
            model: None,
            pricing_identity: None,
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

    // ── seed_zero_baseline / first-turn fix ─────────────────────────────────

    /// (a) Self-spawned session: first notification must be delta_reliable=true,
    /// turn deltas equal to the cumulative values (baseline was zero).
    #[test]
    fn spawned_session_first_turn_is_reliable_with_zero_baseline() {
        let mut tracker = UsageTracker::default();
        // Simulate what pool.rs does immediately after create_session_and_apply_model.
        tracker.seed_zero_baseline("sess-spawned");

        tracker.begin_turn("sess-spawned");
        tracker.record("sess-spawned", &payload(1000, 200, Some(0.01)));
        let usage = tracker.take().expect("pending");

        assert!(
            usage.delta_reliable,
            "spawned session first turn must be reliable"
        );
        assert_eq!(usage.turn_seq, 1);
        // Turn deltas == cumulative (baseline was zero).
        assert_eq!(usage.turn_input_tokens, Some(1000));
        assert_eq!(usage.turn_output_tokens, Some(200));
        let dc = usage.turn_cost_usd.expect("cost delta present");
        assert!((dc - 0.01).abs() < 1e-9, "cost delta: {dc}");
        assert_eq!(usage.cumulative_input_tokens, Some(1000));
        assert_eq!(usage.cumulative_output_tokens, Some(200));
    }

    /// (b) Re-attach session (no seed): first notification must remain
    /// fail-closed (delta_reliable=false, turn.*=None).
    #[test]
    fn reattach_session_first_turn_stays_fail_closed() {
        let mut tracker = UsageTracker::default();
        // No seed_zero_baseline call — simulates re-attach to pre-existing session.

        tracker.begin_turn("sess-reattach");
        tracker.record("sess-reattach", &payload(5000, 1000, Some(0.05)));
        let usage = tracker.take().expect("pending");

        assert!(
            !usage.delta_reliable,
            "re-attach first turn must remain fail-closed (delta_reliable=false)"
        );
        assert_eq!(usage.turn_seq, 1);
        assert!(
            usage.turn_input_tokens.is_none(),
            "no turn delta on re-attach"
        );
        assert!(usage.turn_output_tokens.is_none());
        assert!(usage.turn_cost_usd.is_none());
        // Cumulative still passes through.
        assert_eq!(usage.cumulative_input_tokens, Some(5000));
        assert_eq!(usage.cumulative_output_tokens, Some(1000));
    }

    /// (c) Second turn and beyond are unaffected in both modes.
    #[test]
    fn second_turn_reliable_in_both_spawned_and_reattach_paths() {
        // Spawned path: turn 2 must be reliable (baseline from turn 1's take()).
        let mut spawned = UsageTracker::default();
        spawned.seed_zero_baseline("sess-s");
        spawned.begin_turn("sess-s");
        spawned.record("sess-s", &payload(1000, 100, None));
        let _ = spawned.take();

        spawned.begin_turn("sess-s");
        spawned.record("sess-s", &payload(1800, 250, None));
        let t2_s = spawned.take().expect("spawned turn 2");
        assert!(t2_s.delta_reliable, "spawned path: turn 2 reliable");
        assert_eq!(t2_s.turn_seq, 2);
        assert_eq!(t2_s.turn_input_tokens, Some(800));
        assert_eq!(t2_s.turn_output_tokens, Some(150));

        // Re-attach path: turn 2 must also be reliable.
        let mut reattach = UsageTracker::default();
        reattach.begin_turn("sess-r");
        reattach.record("sess-r", &payload(5000, 1000, None));
        let _ = reattach.take(); // turn 1: unreliable (no baseline), but take() seeds it

        reattach.begin_turn("sess-r");
        reattach.record("sess-r", &payload(6000, 1200, None));
        let t2_r = reattach.take().expect("reattach turn 2");
        assert!(t2_r.delta_reliable, "re-attach path: turn 2 reliable");
        assert_eq!(t2_r.turn_seq, 2);
        assert_eq!(t2_r.turn_input_tokens, Some(1000));
        assert_eq!(t2_r.turn_output_tokens, Some(200));
    }

    /// (d) Wire-frame assertions on the emitted TurnUsage payload fields for
    /// the spawned-session first turn (not just internal delta_reliable).
    #[test]
    fn spawned_session_first_turn_payload_fields_are_correct() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-wire");

        tracker.begin_turn("sess-wire");
        tracker.record(
            "sess-wire",
            &UsageUpdatePayload {
                used: 12345,
                context_limit: 200_000,
                accumulated_input_tokens: Some(10000),
                accumulated_output_tokens: Some(2345),
                accumulated_cached_input_tokens: Some(500),
                accumulated_cache_write_tokens: None,
                accumulated_cost: Some(0.042),
                accumulated_total_tokens: Some(12345),
                model: Some("claude-opus-4-5".to_string()),
                pricing_identity: None,
            },
        );
        let usage = tracker.take().expect("pending");

        // Wire payload fields — every field checked.
        assert_eq!(usage.session_id, "sess-wire");
        assert_eq!(usage.turn_seq, 1);
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, Some(10000));
        assert_eq!(usage.turn_output_tokens, Some(2345));
        // turn_total: cumulative_total(12345) - baseline_total(Some(0)) = 12345.
        // seed_zero_baseline now seeds last_total = Some(0) — same known-zero
        // argument as input/output: a freshly-spawned session has accumulated nothing.
        assert_eq!(
            usage.turn_total_tokens,
            Some(12345),
            "turn_total_tokens must be Some(12345) on seeded first turn (baseline = Some(0))"
        );
        let dc = usage.turn_cost_usd.expect("cost delta present");
        assert!((dc - 0.042).abs() < 1e-9, "cost delta: {dc}");
        assert_eq!(usage.cumulative_input_tokens, Some(10000));
        assert_eq!(usage.cumulative_output_tokens, Some(2345));
        assert_eq!(usage.cumulative_total_tokens, Some(12345));
        assert_eq!(usage.cumulative_cost_usd, Some(0.042));
        assert_eq!(usage.model.as_deref(), Some("claude-opus-4-5"));
        // Cache: baseline seeded with last_cached_input = Some(0), so first turn
        // delta = snapshot(500) - baseline(0) = Some(500).
        assert_eq!(
            usage.turn_cache_read_tokens,
            Some(500),
            "turn_cache_read_tokens: seeded baseline = Some(0) → delta = Some(500)"
        );
        assert_eq!(
            usage.cumulative_cache_read_tokens,
            Some(500),
            "cumulative_cache_read_tokens passes through from payload"
        );
    }

    /// seed_zero_baseline is a no-op when a baseline already exists — guards
    /// against accidental double-seeding across session rotation.
    #[test]
    fn seed_zero_baseline_is_noop_when_baseline_already_exists() {
        let mut tracker = UsageTracker::default();
        // Establish a real baseline via turn 1.
        tracker.seed_zero_baseline("sess-noop");
        tracker.begin_turn("sess-noop");
        tracker.record("sess-noop", &payload(1000, 200, None));
        let _ = tracker.take();

        // A second seed call (e.g. a bug in pool.rs) must not reset the baseline.
        tracker.seed_zero_baseline("sess-noop");

        // Turn 2 delta must still measure from the real baseline (1000/200), not zero.
        tracker.begin_turn("sess-noop");
        tracker.record("sess-noop", &payload(1500, 300, None));
        let usage = tracker.take().expect("pending");

        assert!(usage.delta_reliable);
        assert_eq!(
            usage.turn_input_tokens,
            Some(500),
            "baseline must not have been reset to zero by the second seed call"
        );
        assert_eq!(usage.turn_output_tokens, Some(100));
    }

    // ── PricingIdentity wire threading ──────────────────────────────────────

    fn make_pricing_identity_payload(
        input: u64,
        output: u64,
        authority: &str,
        model: &str,
    ) -> UsageUpdatePayload {
        UsageUpdatePayload {
            used: input + output,
            context_limit: 200_000,
            accumulated_input_tokens: Some(input),
            accumulated_output_tokens: Some(output),
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: Some(model.to_string()),
            pricing_identity: Some(buzz_core::agent_turn_metric::PricingIdentity {
                authority: authority.to_string(),
                model: model.to_string(),
                cache_class: None,
            }),
        }
    }

    /// A payload with a well-formed `pricingIdentity` field must thread
    /// it through to `TurnUsage.pricing_identity`. The field is per-turn
    /// only (not session-cumulative) and must not affect other deltas.
    #[test]
    fn pricing_identity_threads_from_payload_to_turn_usage() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-pi");

        tracker.begin_turn("sess-pi");
        tracker.record(
            "sess-pi",
            &make_pricing_identity_payload(1000, 200, "api.anthropic.com", "claude-opus-4-5"),
        );
        let usage = tracker.take().expect("pending");

        let pi = usage
            .pricing_identity
            .expect("pricing_identity must be Some");
        assert_eq!(pi.authority, "api.anthropic.com");
        assert_eq!(pi.model, "claude-opus-4-5");
        assert!(pi.cache_class.is_none());
        // Other fields must be unaffected.
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, Some(1000));
    }

    /// Old harnesses (goose, older buzz-agent) that do not emit `pricingIdentity`
    /// must produce `TurnUsage.pricing_identity = None` — no default injection.
    #[test]
    fn old_harness_no_pricing_identity_field_yields_none() {
        // Deserialize a payload with no pricingIdentity field.
        let raw = serde_json::json!({
            "used": 1200,
            "contextLimit": 200_000,
            "accumulatedInputTokens": 1000,
            "accumulatedOutputTokens": 200,
            "model": "claude-opus-4",
        });
        let p: UsageUpdatePayload =
            serde_json::from_value(raw).expect("must deserialize without pricingIdentity field");
        assert!(
            p.pricing_identity.is_none(),
            "old harness compat: absent field must deserialize to None, not inject a default"
        );

        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-old");
        tracker.begin_turn("sess-old");
        tracker.record("sess-old", &p);
        let usage = tracker.take().expect("pending");

        assert!(
            usage.pricing_identity.is_none(),
            "old harness: pricing_identity must be None in TurnUsage"
        );
    }

    /// `pricingIdentity` in the JSON wire format uses camelCase keys as
    /// required by the NIP-AM wire contract (`#[serde(rename_all = "camelCase")]`).
    #[test]
    fn pricing_identity_deserializes_from_camel_case_wire_key() {
        let raw = serde_json::json!({
            "used": 1500,
            "contextLimit": 0,
            "accumulatedInputTokens": 1200,
            "accumulatedOutputTokens": 300,
            "pricingIdentity": {
                "authority": "api.openai.com",
                "model": "gpt-4o",
            }
        });
        let p: UsageUpdatePayload = serde_json::from_value(raw).expect("payload must deserialize");
        let pi = p.pricing_identity.expect("pricingIdentity must parse");
        assert_eq!(pi.authority, "api.openai.com");
        assert_eq!(pi.model, "gpt-4o");
        assert!(pi.cache_class.is_none());
    }

    /// `pricingIdentity` with a `cacheClass` field threads through correctly.
    #[test]
    fn pricing_identity_cache_class_threads_through() {
        let raw = serde_json::json!({
            "used": 800,
            "contextLimit": 0,
            "accumulatedInputTokens": 600,
            "accumulatedOutputTokens": 200,
            "pricingIdentity": {
                "authority": "api.anthropic.com",
                "model": "claude-3-5-haiku",
                "cacheClass": "ephemeral",
            }
        });
        let p: UsageUpdatePayload = serde_json::from_value(raw).expect("payload must deserialize");
        let pi = p.pricing_identity.expect("pricingIdentity must parse");
        assert_eq!(pi.cache_class.as_deref(), Some("ephemeral"));
    }

    /// `pricing_identity` is per-turn only — it must NOT be stored in or
    /// influence the session-cumulative baseline (`SessionState`). A second
    /// turn must still carry its own `pricing_identity` from the latest payload.
    #[test]
    fn pricing_identity_is_not_session_cumulative() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-perturn");

        // Turn 1: identity present.
        tracker.begin_turn("sess-perturn");
        tracker.record(
            "sess-perturn",
            &make_pricing_identity_payload(1000, 200, "api.anthropic.com", "claude-opus-4-5"),
        );
        let t1 = tracker.take().expect("turn 1");
        assert!(
            t1.pricing_identity.is_some(),
            "turn 1 must carry pricing_identity"
        );

        // Turn 2: no identity in payload.
        tracker.begin_turn("sess-perturn");
        tracker.record("sess-perturn", &payload(1500, 300, None));
        let t2 = tracker.take().expect("turn 2");
        assert!(
            t2.pricing_identity.is_none(),
            "turn 2 must NOT inherit turn 1's pricing_identity"
        );
    }

    // ── First-turn cache deltas via seed_zero_baseline ───────────────────────
    //
    // When `seed_zero_baseline` is called before the first turn (the normal path
    // for freshly-spawned sessions), the zero-seeded baselines mean the first
    // snapshot's cumulative values equal the per-turn deltas — no data is lost.

    #[test]
    fn seed_zero_baseline_first_turn_cache_read_and_write_produce_exact_deltas() {
        // A freshly-spawned session seeds last_cached_input = Some(0) and
        // last_cache_write = Some(0).  The first turn's snapshot values ARE the
        // deltas; both categories must surface as exact non-None values.
        let mut tracker = UsageTracker::default();

        // Simulate session spawn: seed before the first turn.
        tracker.seed_zero_baseline("sess-seed1");
        tracker.begin_turn("sess-seed1");

        // First snapshot: cache-read = 500, cache-write = 120.
        let payload = UsageUpdatePayload {
            used: 1200,
            context_limit: 200_000,
            accumulated_input_tokens: Some(1000),
            accumulated_output_tokens: Some(200),
            accumulated_cached_input_tokens: Some(500),
            accumulated_cache_write_tokens: Some(120),
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
        };
        tracker.record("sess-seed1", &payload);
        let usage = tracker
            .take()
            .expect("first seeded turn must produce a record");

        // Input/output deltas are always reliable on the seeded path.
        assert!(
            usage.delta_reliable,
            "seeded first turn must be delta_reliable"
        );
        assert_eq!(
            usage.turn_cache_read_tokens,
            Some(500),
            "seeded first turn: cache-read delta must equal the snapshot value"
        );
        assert_eq!(
            usage.turn_cache_write_tokens,
            Some(120),
            "seeded first turn: cache-write delta must equal the snapshot value"
        );
        assert_eq!(
            usage.cumulative_cache_read_tokens,
            Some(500),
            "seeded first turn: cumulative cache-read must pass through"
        );
        assert_eq!(
            usage.cumulative_cache_write_tokens,
            Some(120),
            "seeded first turn: cumulative cache-write must pass through"
        );
    }

    // ── ACP identity fold across multiple notifications ──────────────────────
    //
    // The publisher fold (agent.rs `fold_pricing_identity`) covers the case where
    // a single cumulative snapshot has no provable identity within the agent loop.
    // The ACP tracker has a DISTINCT multi-notification path: buzz-agent sends
    // multiple `usage_update` notifications per turn (one per round), and the ACP
    // tracker must fold identity across those notifications — not last-update-wins.
    //
    // Three acceptance tests per the dispatch contract (Paul event 4ad5390e):

    fn pi_payload(input: u64, output: u64, authority: &str, model: &str) -> UsageUpdatePayload {
        make_pricing_identity_payload(input, output, authority, model)
    }

    fn no_identity_payload(input: u64, output: u64) -> UsageUpdatePayload {
        payload(input, output, None)
    }

    /// ACP identity fold — case A→B: two notifications with different proven
    /// identities in one turn must poison; published `AgentTurnMetricPayload`
    /// (i.e. `TurnUsage.pricing_identity`) must be absent.
    #[test]
    fn acp_identity_fold_different_identities_poisons() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-acp-ab");
        tracker.begin_turn("sess-acp-ab");

        // Notification 1: identity A (anthropic / claude-opus).
        tracker.record(
            "sess-acp-ab",
            &pi_payload(1000, 200, "api.anthropic.com", "claude-opus-4-5"),
        );
        // Notification 2: identity B (openai / gpt-4o).
        tracker.record(
            "sess-acp-ab",
            &pi_payload(2000, 400, "api.openai.com", "gpt-4o"),
        );
        let usage = tracker.take().expect("pending");

        assert!(
            usage.pricing_identity.is_none(),
            "A→B in one turn: pricing_identity must be absent (poisoned by mismatch)"
        );
    }

    /// ACP identity fold — case A→absent: proven identity followed by a
    /// notification with no identity must poison; published identity must be absent.
    #[test]
    fn acp_identity_fold_absent_notification_poisons() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-acp-aabs");
        tracker.begin_turn("sess-acp-aabs");

        // Notification 1: identity A.
        tracker.record(
            "sess-acp-aabs",
            &pi_payload(1000, 200, "api.anthropic.com", "claude-3-5-haiku"),
        );
        // Notification 2: no identity (e.g. unpairable cumulative snapshot).
        tracker.record("sess-acp-aabs", &no_identity_payload(2000, 400));

        let usage = tracker.take().expect("pending");

        assert!(
            usage.pricing_identity.is_none(),
            "A→absent in one turn: pricing_identity must be absent (poisoned by missing identity)"
        );
    }

    /// ACP identity fold — case A→absent→A: proven identity, then an absent
    /// notification, then the original identity again — must NOT heal;
    /// published identity must still be absent.
    #[test]
    fn acp_identity_fold_never_heals_after_poison() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-acp-heal");
        tracker.begin_turn("sess-acp-heal");

        // Notification 1: identity A.
        tracker.record(
            "sess-acp-heal",
            &pi_payload(1000, 200, "api.openai.com", "gpt-4o"),
        );
        // Notification 2: absent identity — poisons.
        tracker.record("sess-acp-heal", &no_identity_payload(2000, 400));
        // Notification 3: identity A again — must NOT resurrect it.
        tracker.record(
            "sess-acp-heal",
            &pi_payload(3000, 600, "api.openai.com", "gpt-4o"),
        );

        let usage = tracker.take().expect("pending");

        assert!(
            usage.pricing_identity.is_none(),
            "A→absent→A in one turn: pricing_identity must remain absent (no healing after poison)"
        );
    }

    // ── Overflow-poison ACP consumer tests ───────────────────────────────────

    /// A payload with absent `accumulatedInputTokens` (publisher overflow-poisoned)
    /// must produce `delta_reliable: false`, null turn fields, and null cumulative
    /// input/output in `TurnUsage`.
    #[test]
    fn absent_input_tokens_produces_unreliable_delta_and_null_cumulative() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-poison-in");
        tracker.begin_turn("sess-poison-in");

        // Publisher omitted accumulatedInputTokens (overflow-poisoned).
        let p = UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: None, // overflow-poisoned
            accumulated_output_tokens: Some(200),
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
        };
        tracker.record("sess-poison-in", &p);
        let usage = tracker.take().expect("pending");

        assert!(
            !usage.delta_reliable,
            "absent input: delta_reliable must be false"
        );
        assert!(
            usage.turn_input_tokens.is_none(),
            "absent input: turn_input_tokens must be None"
        );
        assert!(
            usage.turn_output_tokens.is_none(),
            "absent input: turn_output_tokens must be None"
        );
        assert!(
            usage.cumulative_input_tokens.is_none(),
            "absent input: cumulative_input_tokens must be None"
        );
        // cumulative_output_tokens passes through as-is (it's separate).
        assert_eq!(usage.cumulative_output_tokens, Some(200));
    }

    /// A payload with absent `accumulatedOutputTokens` (publisher overflow-poisoned)
    /// must produce `delta_reliable: false`, null turn fields, and null cumulative output.
    #[test]
    fn absent_output_tokens_produces_unreliable_delta_and_null_cumulative() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-poison-out");
        tracker.begin_turn("sess-poison-out");

        let p = UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: Some(1000),
            accumulated_output_tokens: None, // overflow-poisoned
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
        };
        tracker.record("sess-poison-out", &p);
        let usage = tracker.take().expect("pending");

        assert!(
            !usage.delta_reliable,
            "absent output: delta_reliable must be false"
        );
        assert!(usage.turn_input_tokens.is_none());
        assert!(usage.turn_output_tokens.is_none());
        assert_eq!(usage.cumulative_input_tokens, Some(1000));
        assert!(
            usage.cumulative_output_tokens.is_none(),
            "absent output: cumulative_output_tokens must be None"
        );
    }

    /// A goose-shaped payload with both input and output present must produce
    /// the same behavior as before — delta_reliable true on seeded sessions,
    /// cumulative values passed through exactly.
    #[test]
    fn goose_shaped_payload_both_present_unchanged_behavior() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-goose");
        tracker.begin_turn("sess-goose");

        tracker.record("sess-goose", &payload(1500, 300, None));
        let usage = tracker.take().expect("pending");

        assert!(
            usage.delta_reliable,
            "goose payload: delta_reliable must be true"
        );
        assert_eq!(usage.turn_input_tokens, Some(1500));
        assert_eq!(usage.turn_output_tokens, Some(300));
        assert_eq!(usage.cumulative_input_tokens, Some(1500));
        assert_eq!(usage.cumulative_output_tokens, Some(300));
    }

    /// Once a session emits a poisoned snapshot (absent fields), subsequent turns
    /// stay unknown — not advancing is correct since publisher poison is permanent.
    #[test]
    fn poison_mid_session_subsequent_turns_stay_unknown() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-poison-mid");

        // Turn 1: normal.
        tracker.begin_turn("sess-poison-mid");
        tracker.record("sess-poison-mid", &payload(1000, 200, None));
        let t1 = tracker.take().expect("t1");
        assert!(t1.delta_reliable);
        assert_eq!(t1.cumulative_input_tokens, Some(1000));

        // Turn 2: overflow-poisoned (publisher omits input).
        tracker.begin_turn("sess-poison-mid");
        let poisoned = UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: None,
            accumulated_output_tokens: Some(500),
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
        };
        tracker.record("sess-poison-mid", &poisoned);
        let t2 = tracker.take().expect("t2");
        assert!(!t2.delta_reliable, "poisoned turn: delta_reliable false");
        assert!(t2.cumulative_input_tokens.is_none());

        // Turn 3: subsequent snapshot also absent → still unreliable.
        tracker.begin_turn("sess-poison-mid");
        let also_poisoned = UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: None,
            accumulated_output_tokens: Some(700),
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
        };
        tracker.record("sess-poison-mid", &also_poisoned);
        let t3 = tracker.take().expect("t3");
        assert!(
            !t3.delta_reliable,
            "turn after poison: delta_reliable still false"
        );
        assert!(
            t3.cumulative_input_tokens.is_none(),
            "turn after poison: cumulative_input_tokens stays None"
        );
    }

    /// Wes's P1 reproducer: once ACP has observed an absent input cumulative,
    /// a later turn that resumes emitting the field must NOT heal
    /// `delta_reliable`.  The prefix delta is irrecoverably unknown; sticky
    /// poison persists for the rest of the session.
    #[test]
    fn sticky_poison_input_absent_then_present_stays_unreliable() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-sticky-input");

        // Turn 1: normal — establishes a baseline.
        tracker.begin_turn("sess-sticky-input");
        tracker.record("sess-sticky-input", &payload(500, 100, None));
        let t1 = tracker.take().expect("t1");
        assert!(t1.delta_reliable, "pre-poison turn must be reliable");

        // Turn 2: publisher poisons (absent input).
        tracker.begin_turn("sess-sticky-input");
        let poisoned = UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: None,
            accumulated_output_tokens: Some(300),
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
        };
        tracker.record("sess-sticky-input", &poisoned);
        let t2 = tracker.take().expect("t2");
        assert!(!t2.delta_reliable, "poisoned turn must be unreliable");

        // Turn 3: publisher resumes emitting input — but poison must be sticky.
        tracker.begin_turn("sess-sticky-input");
        tracker.record("sess-sticky-input", &payload(100, 400, None));
        let t3 = tracker.take().expect("t3");
        assert!(
            !t3.delta_reliable,
            "turn after absent→present must stay unreliable (sticky poison)"
        );
        assert!(
            t3.turn_input_tokens.is_none(),
            "turn_input_tokens must be None after sticky poison"
        );
        assert!(
            t3.turn_output_tokens.is_none(),
            "turn_output_tokens must be None after sticky poison"
        );

        // Turn 4: publisher continues emitting — poison persists.
        tracker.begin_turn("sess-sticky-input");
        tracker.record("sess-sticky-input", &payload(150, 500, None));
        let t4 = tracker.take().expect("t4");
        assert!(
            !t4.delta_reliable,
            "delta_reliable stays false for the remainder of the session"
        );
    }

    /// Symmetric to the input test: once ACP has observed an absent *output*
    /// cumulative, subsequent turns that resume emitting output must NOT heal
    /// `delta_reliable`.
    #[test]
    fn sticky_poison_output_absent_then_present_stays_unreliable() {
        let mut tracker = UsageTracker::default();
        tracker.seed_zero_baseline("sess-sticky-output");

        // Turn 1: normal.
        tracker.begin_turn("sess-sticky-output");
        tracker.record("sess-sticky-output", &payload(500, 100, None));
        let t1 = tracker.take().expect("t1");
        assert!(t1.delta_reliable);

        // Turn 2: absent output poisons the session.
        tracker.begin_turn("sess-sticky-output");
        let poisoned = UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: Some(600),
            accumulated_output_tokens: None, // <-- absent output
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
        };
        tracker.record("sess-sticky-output", &poisoned);
        let t2 = tracker.take().expect("t2");
        assert!(
            !t2.delta_reliable,
            "absent output must make delta unreliable"
        );

        // Turn 3: output resumes — sticky poison holds.
        tracker.begin_turn("sess-sticky-output");
        tracker.record("sess-sticky-output", &payload(700, 200, None));
        let t3 = tracker.take().expect("t3");
        assert!(
            !t3.delta_reliable,
            "output absent→present must stay unreliable (sticky poison)"
        );
        assert!(t3.turn_input_tokens.is_none());
        assert!(t3.turn_output_tokens.is_none());

        // Turn 4: persists.
        tracker.begin_turn("sess-sticky-output");
        tracker.record("sess-sticky-output", &payload(800, 250, None));
        let t4 = tracker.take().expect("t4");
        assert!(
            !t4.delta_reliable,
            "delta_reliable stays false for the remainder of the session"
        );
    }

    /// Convenience helper: build a payload with optional input and output.
    /// Used by within-turn sticky-poison tests that need to inject absence
    /// mid-turn without building the full struct every time.
    fn payload_opt(input: Option<u64>, output: Option<u64>) -> UsageUpdatePayload {
        UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: input,
            accumulated_output_tokens: output,
            accumulated_cached_input_tokens: None,
            accumulated_cache_write_tokens: None,
            accumulated_cost: None,
            accumulated_total_tokens: None,
            model: None,
            pricing_identity: None,
        }
    }

    /// Rich payload for cross-session baseline-preservation tests.
    ///
    /// Carries all six counter fields so that `take()` commits a fully-populated
    /// `SessionState` with every baseline `Some(…)` and distinct.  The
    /// before/after comparisons then exercise every field in the preservation
    /// assertion, not just the input/output pair.
    fn rich_payload(
        input: Option<u64>,
        output: Option<u64>,
        cost: Option<f64>,
        total: Option<u64>,
        cached_input: Option<u64>,
        cache_write: Option<u64>,
    ) -> UsageUpdatePayload {
        UsageUpdatePayload {
            used: 0,
            context_limit: 0,
            accumulated_input_tokens: input,
            accumulated_output_tokens: output,
            accumulated_cached_input_tokens: cached_input,
            accumulated_cache_write_tokens: cache_write,
            accumulated_cost: cost,
            accumulated_total_tokens: total,
            model: None,
            pricing_identity: None,
        }
    }

    /// Once ACP observes an absent *input* snapshot mid-turn, a later
    /// notification in the SAME turn that reintroduces the field must NOT
    /// heal `delta_reliable`.  The poison must also persist to subsequent turns.
    ///
    /// Wes's finding: his reproducer was stated at snapshot level ("a later
    /// producer snapshot reintroduces the field"), not turn level.  This test
    /// pins the within-turn case that the turn-boundary latch missed.
    #[test]
    fn within_turn_input_absent_then_present_stays_unreliable() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("wt-input");
        t.begin_turn("wt-input");
        // First notification is normal — establishes a seeded baseline turn.
        t.record("wt-input", &payload_opt(Some(50), Some(10)));
        let t0 = t.take().expect("t0");
        assert!(t0.delta_reliable, "pre-poison turn must be reliable");

        t.begin_turn("wt-input");
        t.record("wt-input", &payload_opt(None, Some(10))); // poison: input absent
        t.record("wt-input", &payload_opt(Some(100), Some(20))); // reintroduced
        let t1 = t.take().expect("t1");
        assert!(
            !t1.delta_reliable,
            "within-turn absent→present must stay unreliable (input)"
        );

        t.begin_turn("wt-input");
        t.record("wt-input", &payload_opt(Some(150), Some(30)));
        let t2 = t.take().expect("t2");
        assert!(
            !t2.delta_reliable,
            "poison must persist to next turn (input)"
        );
    }

    /// Symmetric to the input case: once ACP observes an absent *output*
    /// snapshot mid-turn, subsequent same-turn reintroductions and subsequent
    /// turns must both stay unreliable.
    #[test]
    fn within_turn_output_absent_then_present_stays_unreliable() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("wt-output");
        t.begin_turn("wt-output");
        t.record("wt-output", &payload_opt(Some(50), Some(10)));
        let t0 = t.take().expect("t0");
        assert!(t0.delta_reliable, "pre-poison turn must be reliable");

        t.begin_turn("wt-output");
        t.record("wt-output", &payload_opt(Some(60), None)); // poison: output absent
        t.record("wt-output", &payload_opt(Some(100), Some(20))); // reintroduced
        let t1 = t.take().expect("t1");
        assert!(
            !t1.delta_reliable,
            "within-turn absent→present must stay unreliable (output)"
        );

        t.begin_turn("wt-output");
        t.record("wt-output", &payload_opt(Some(150), Some(30)));
        let t2 = t.take().expect("t2");
        assert!(
            !t2.delta_reliable,
            "poison must persist to next turn (output)"
        );
    }

    /// Un-baselined session (attach-to-existing path, no seed_zero_baseline):
    /// an absent input snapshot observed mid-turn must poison the session even
    /// though no session entry exists yet — a later reintroduced value must not
    /// heal delta_reliable in the next turn.
    ///
    /// This is Paul's probe that FAILED at eb24590e2e — the get_mut latch was a
    /// no-op for un-baselined sessions.  The fold accumulator on UsageTracker
    /// captures the absence and commits it at take() regardless of whether a
    /// session entry already exists.
    #[test]
    fn unbaselined_within_turn_input_absence_poisons_next_turn() {
        let mut t = UsageTracker::default();
        // NO seed_zero_baseline — attach-to-existing path
        t.begin_turn("s");
        t.record("s", &payload_opt(None, Some(10))); // poisoned snapshot
        t.record("s", &payload_opt(Some(100), Some(20))); // reintroduced same turn
        let t1 = t.take().expect("t1");
        assert!(!t1.delta_reliable, "t1: no baseline — must be unreliable");
        t.begin_turn("s");
        t.record("s", &payload_opt(Some(150), Some(30)));
        let t2 = t.take().expect("t2");
        assert!(
            !t2.delta_reliable,
            "t2: absence was observed in t1 — sticky poison must hold"
        );
    }

    /// Symmetric output-field case for the un-baselined escape:
    /// absent output snapshot mid-turn must poison the session and persist to
    /// the next turn, even when no session entry existed at record() time.
    #[test]
    fn unbaselined_within_turn_output_absence_poisons_next_turn() {
        let mut t = UsageTracker::default();
        // NO seed_zero_baseline — attach-to-existing path
        t.begin_turn("s");
        t.record("s", &payload_opt(Some(10), None)); // poisoned snapshot: output absent
        t.record("s", &payload_opt(Some(100), Some(20))); // reintroduced same turn
        let t1 = t.take().expect("t1");
        assert!(!t1.delta_reliable, "t1: no baseline — must be unreliable");
        t.begin_turn("s");
        t.record("s", &payload_opt(Some(150), Some(30)));
        let t2 = t.take().expect("t2");
        assert!(
            !t2.delta_reliable,
            "t2: absence was observed in t1 — sticky poison must hold"
        );
    }

    /// Take-skipped same-session: `begin_turn("s")` is called twice without
    /// a `take()` in between (the initial-message path in pool.rs does this).
    /// An absence observed in the skipped turn must NOT be discarded — the
    /// next real turn must stay unreliable.
    ///
    /// This is Paul's probe that FAILED at 762e47bd31.  The fold accumulators
    /// were only committed in `take()`, so a skipped `take()` silently dropped
    /// the observed absence.  The fix flushes in `begin_turn()` instead.
    #[test]
    fn take_skipped_turn_input_absence_survives_to_next_turn() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("s");
        t.begin_turn("s");
        t.record("s", &payload_opt(None, Some(10))); // absence observed in init turn
                                                     // NO take() — init-message path goes straight to the next begin_turn
        t.begin_turn("s");
        t.record("s", &payload_opt(Some(100), Some(20)));
        let t2 = t.take().expect("t2");
        assert!(
            !t2.delta_reliable,
            "absence must survive a skipped take() (input)"
        );
    }

    /// Symmetric output-field case for the take-skipped escape.
    #[test]
    fn take_skipped_turn_output_absence_survives_to_next_turn() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("s");
        t.begin_turn("s");
        t.record("s", &payload_opt(Some(10), None)); // absence observed in init turn: output absent
                                                     // NO take() — init-message path goes straight to the next begin_turn
        t.begin_turn("s");
        t.record("s", &payload_opt(Some(100), Some(20)));
        let t2 = t.take().expect("t2");
        assert!(
            !t2.delta_reliable,
            "absence must survive a skipped take() (output)"
        );
    }

    /// Cross-session take-skipped: session A's turn observed an absence, then
    /// `begin_turn("B")` runs next (no take() for A).  A's poison must survive
    /// — when A is next in-flight its delta must still be unreliable.
    #[test]
    fn cross_session_take_skipped_input_absence_survives() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("a");
        t.seed_zero_baseline("b");
        // Session A's turn: observe absence (no take)
        t.begin_turn("a");
        t.record("a", &payload_opt(None, Some(10))); // input absence observed for A
                                                     // Session B starts — no take() for A
        t.begin_turn("b");
        t.record("b", &payload_opt(Some(50), Some(5)));
        let tb = t.take().expect("tb");
        assert!(tb.delta_reliable, "session B must still be reliable");
        // Session A resumes — poison must hold
        t.begin_turn("a");
        t.record("a", &payload_opt(Some(100), Some(20)));
        let ta = t.take().expect("ta");
        assert!(
            !ta.delta_reliable,
            "session A: absence observed before cross-session begin_turn must hold"
        );
    }

    /// Symmetric output-field cross-session case.
    #[test]
    fn cross_session_take_skipped_output_absence_survives() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("a");
        t.seed_zero_baseline("b");
        // Session A's turn: observe output absence (no take)
        t.begin_turn("a");
        t.record("a", &payload_opt(Some(10), None)); // output absence observed for A
                                                     // Session B starts — no take() for A
        t.begin_turn("b");
        t.record("b", &payload_opt(Some(50), Some(5)));
        let tb = t.take().expect("tb");
        assert!(tb.delta_reliable, "session B must still be reliable");
        // Session A resumes — poison must hold
        t.begin_turn("a");
        t.record("a", &payload_opt(Some(100), Some(20)));
        let ta = t.take().expect("ta");
        assert!(
            !ta.delta_reliable,
            "session A: output absence observed before cross-session begin_turn must hold"
        );
    }

    /// Wes's reproducer (round-5 review): a cross-session absent notification
    /// arrives while a different session is in-flight.  The absence must latch
    /// into the notified session's `*_ever_poisoned` state even though the
    /// notification's counters are otherwise dropped.
    ///
    /// Round-7 upgrade: rich fixture with all six baselines populated and
    /// distinct so every "must not change" assertion is discriminating.  The
    /// cross-session payload carries different values for every present counter
    /// (including non-target output) so advance-to-current corruption is also
    /// visible.  Pre-flag false, post-flag true is explicitly asserted.
    ///
    /// Scenario:
    ///   - A publishes a reliable turn with full counters (turn 1).
    ///   - B becomes in-flight.
    ///   - A late A notification: input=None, all other counters present but
    ///     with different values from A's committed baseline.
    ///   - B's turn publishes normally (must be unaffected).
    ///   - A's turn 2 must be `!delta_reliable`.
    #[test]
    fn cross_session_absent_notification_latches_poison_input() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("a");
        t.seed_zero_baseline("b");

        // ── A turn 1: commit a fully-populated baseline with all six fields ──
        //
        // Values chosen to be distinct so every baseline field is discriminating:
        //   last_input=50, last_output=10, last_cost=1.5, last_total=70,
        //   last_cached_input=5, last_cache_write=3.
        t.begin_turn("a");
        t.record(
            "a",
            &rich_payload(Some(50), Some(10), Some(1.5), Some(70), Some(5), Some(3)),
        );
        let a1 = t.take().expect("a1");
        assert!(a1.delta_reliable, "A turn 1 must be reliable");

        // ── Snapshot A's SessionState BEFORE the cross-session record ──
        let state_before = t
            .sessions
            .get("a")
            .expect("A entry must exist after turn 1")
            .clone();
        // Sanity: all six baselines are populated and have the expected values.
        assert_eq!(state_before.last_input, Some(50));
        assert_eq!(state_before.last_output, Some(10));
        assert_eq!(state_before.last_cost, Some(1.5));
        assert_eq!(state_before.last_total, Some(70));
        assert_eq!(state_before.last_cached_input, Some(5));
        assert_eq!(state_before.last_cache_write, Some(3));
        // Pre-flag: input_ever_poisoned must be false before the latch.
        assert!(
            !state_before.input_ever_poisoned,
            "input_ever_poisoned must be false before the cross-session record"
        );

        // ── B in-flight; late A notification: input absent, all other fields
        //    present with DIFFERENT values from A's committed baseline ──
        //    (output=20, cost=2.5, total=90, cached_input=8, cache_write=6)
        t.begin_turn("b");
        t.record(
            "a",
            &rich_payload(None, Some(20), Some(2.5), Some(90), Some(8), Some(6)),
        );

        // ── Snapshot A's SessionState AFTER the cross-session record ──
        let state_after = t
            .sessions
            .get("a")
            .expect("A entry must still exist")
            .clone();

        // Every non-poison field must be byte-for-byte unchanged.
        assert_eq!(
            state_after.published_seq, state_before.published_seq,
            "published_seq must not be advanced by a dropped cross-session notification"
        );
        assert_eq!(
            state_after.last_input, state_before.last_input,
            "last_input baseline must not change"
        );
        assert_eq!(
            state_after.last_output, state_before.last_output,
            "last_output baseline must not change"
        );
        assert_eq!(
            state_after.last_cost, state_before.last_cost,
            "last_cost baseline must not change"
        );
        assert_eq!(
            state_after.last_total, state_before.last_total,
            "last_total baseline must not change"
        );
        assert_eq!(
            state_after.last_cached_input, state_before.last_cached_input,
            "last_cached_input baseline must not change"
        );
        assert_eq!(
            state_after.last_cache_write, state_before.last_cache_write,
            "last_cache_write baseline must not change"
        );
        // Poison: input flag grows from false (asserted above) to true.
        assert!(
            state_after.input_ever_poisoned,
            "input_ever_poisoned must be latched by the cross-session absent notification"
        );
        assert_eq!(
            state_after.output_ever_poisoned, state_before.output_ever_poisoned,
            "output_ever_poisoned must not change when only input is absent"
        );

        t.record(
            "b",
            &rich_payload(
                Some(200),
                Some(30),
                Some(4.0),
                Some(250),
                Some(15),
                Some(10),
            ),
        );
        let b1 = t.take().expect("b1");
        assert!(
            b1.delta_reliable,
            "B turn 1 must be unaffected by the late A notification"
        );
        assert_eq!(b1.session_id, "b");

        // ── A turn 2: record at 100/20 ──
        t.begin_turn("a");
        t.record("a", &payload_opt(Some(100), Some(20)));
        let a2 = t.take().expect("a2");
        assert!(
            !a2.delta_reliable,
            "A turn 2 must be poisoned: input absence observed in dropped cross-session notification"
        );
    }

    /// Symmetric output-absent case for the cross-session absence latch.
    ///
    /// Round-7 upgrade: same rich-fixture approach as the input variant — all
    /// six baselines populated with distinct values, cross-session payload
    /// carries different present counters (including non-target input) while
    /// output is absent, pre-flag false → post-flag true explicitly asserted.
    #[test]
    fn cross_session_absent_notification_latches_poison_output() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("a");
        t.seed_zero_baseline("b");

        // ── A turn 1: commit a fully-populated baseline ──
        //   last_input=50, last_output=10, last_cost=1.5, last_total=70,
        //   last_cached_input=5, last_cache_write=3.
        t.begin_turn("a");
        t.record(
            "a",
            &rich_payload(Some(50), Some(10), Some(1.5), Some(70), Some(5), Some(3)),
        );
        let a1 = t.take().expect("a1");
        assert!(a1.delta_reliable, "A turn 1 must be reliable");

        // ── Snapshot A's SessionState BEFORE the cross-session record ──
        let state_before = t
            .sessions
            .get("a")
            .expect("A entry must exist after turn 1")
            .clone();
        // Sanity: all six baselines populated with expected values.
        assert_eq!(state_before.last_input, Some(50));
        assert_eq!(state_before.last_output, Some(10));
        assert_eq!(state_before.last_cost, Some(1.5));
        assert_eq!(state_before.last_total, Some(70));
        assert_eq!(state_before.last_cached_input, Some(5));
        assert_eq!(state_before.last_cache_write, Some(3));
        // Pre-flag: output_ever_poisoned must be false before the latch.
        assert!(
            !state_before.output_ever_poisoned,
            "output_ever_poisoned must be false before the cross-session record"
        );

        // ── B in-flight; late A notification: output absent, all other fields
        //    present with DIFFERENT values from A's committed baseline ──
        //    (input=80, cost=2.5, total=90, cached_input=8, cache_write=6)
        t.begin_turn("b");
        t.record(
            "a",
            &rich_payload(Some(80), None, Some(2.5), Some(90), Some(8), Some(6)),
        );

        // ── Snapshot A's SessionState AFTER the cross-session record ──
        let state_after = t
            .sessions
            .get("a")
            .expect("A entry must still exist")
            .clone();

        // Every non-poison field must be byte-for-byte unchanged.
        assert_eq!(
            state_after.published_seq, state_before.published_seq,
            "published_seq must not be advanced"
        );
        assert_eq!(
            state_after.last_input, state_before.last_input,
            "last_input baseline must not change"
        );
        assert_eq!(
            state_after.last_output, state_before.last_output,
            "last_output baseline must not change"
        );
        assert_eq!(
            state_after.last_cost, state_before.last_cost,
            "last_cost baseline must not change"
        );
        assert_eq!(
            state_after.last_total, state_before.last_total,
            "last_total baseline must not change"
        );
        assert_eq!(
            state_after.last_cached_input, state_before.last_cached_input,
            "last_cached_input baseline must not change"
        );
        assert_eq!(
            state_after.last_cache_write, state_before.last_cache_write,
            "last_cache_write baseline must not change"
        );
        // Poison: output flag grows from false (asserted above) to true.
        assert!(
            state_after.output_ever_poisoned,
            "output_ever_poisoned must be latched by the cross-session absent notification"
        );
        assert_eq!(
            state_after.input_ever_poisoned, state_before.input_ever_poisoned,
            "input_ever_poisoned must not change when only output is absent"
        );

        t.record(
            "b",
            &rich_payload(
                Some(200),
                Some(30),
                Some(4.0),
                Some(250),
                Some(15),
                Some(10),
            ),
        );
        let b1 = t.take().expect("b1");
        assert!(b1.delta_reliable, "B turn 1 must be unaffected");
        assert_eq!(b1.session_id, "b");

        // ── A turn 2 ──
        t.begin_turn("a");
        t.record("a", &payload_opt(Some(100), Some(20)));
        let a2 = t.take().expect("a2");
        assert!(
            !a2.delta_reliable,
            "A turn 2 must be poisoned: output absence observed in dropped cross-session notification"
        );
    }

    /// Un-baselined variant (input-absent): A has NO session entry when the
    /// cross-session absent notification arrives.  The latch must CREATE an entry
    /// with only `input_ever_poisoned = true` and zero-baseline fields (all six
    /// `last_*` baselines remain `None`, `published_seq` = 0).  The poison must
    /// then survive into A's second real turn after A establishes its own baseline.
    ///
    /// Round-7 upgrade: the cross-session payload carries nonzero cost/total/
    /// cached-input/cache-write values (plus present non-target output) so the
    /// created-entry shape assertions actually prove the latch did NOT initialize
    /// baselines from the incoming payload.
    ///
    /// (A's first turn is unreliable regardless because it has no prior baseline;
    /// the second turn is where the latch matters — without it, `take()` would
    /// see `input_ever_poisoned: false` and flip `delta_reliable: true`.)
    #[test]
    fn cross_session_absent_notification_latches_poison_unbaselined() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("b");
        // A has NO entry at all.
        assert!(
            !t.sessions.contains_key("a"),
            "A must have no entry before the cross-session record"
        );

        // ── B in-flight; A notification: input absent, but ALL other counter
        //    fields present with nonzero values ──
        //    output=15, cost=3.0, total=80, cached_input=7, cache_write=4.
        //    A has no prior entry, so the latch must CREATE one with all six
        //    baselines None — not initialized from these payload values.
        t.begin_turn("b");
        t.record(
            "a",
            &rich_payload(None, Some(15), Some(3.0), Some(80), Some(7), Some(4)),
        );

        // Entry must now exist with exactly the right shape.
        let created = t
            .sessions
            .get("a")
            .expect("latch must create an entry for A");
        assert_eq!(created.published_seq, 0, "created entry has zero seq");
        assert!(
            created.last_input.is_none(),
            "created entry must have no input baseline"
        );
        assert!(
            created.last_output.is_none(),
            "created entry must have no output baseline"
        );
        assert!(
            created.last_cost.is_none(),
            "created entry must have no cost baseline"
        );
        assert!(
            created.last_total.is_none(),
            "created entry must have no total baseline"
        );
        assert!(
            created.last_cached_input.is_none(),
            "created entry must have no cache-read baseline"
        );
        assert!(
            created.last_cache_write.is_none(),
            "created entry must have no cache-write baseline"
        );
        assert!(
            created.input_ever_poisoned,
            "input_ever_poisoned must be set on the newly created entry"
        );
        assert!(
            !created.output_ever_poisoned,
            "output_ever_poisoned must NOT be set (only input was absent)"
        );

        t.record("b", &payload_opt(Some(100), Some(20)));
        let b1 = t.take().expect("b1");
        assert!(b1.delta_reliable, "B must be unaffected");

        // ── A's first real turn (unreliable regardless — no prior baseline) ──
        t.begin_turn("a");
        t.record("a", &payload_opt(Some(80), Some(15)));
        let _a1 = t.take().expect("a1");

        // ── A's second turn: with the fix, `input_ever_poisoned` was committed
        // by take() above; without it, the flag would be false and delta heals. ──
        t.begin_turn("a");
        t.record("a", &payload_opt(Some(150), Some(25)));
        let a2 = t.take().expect("a2");
        assert!(
            !a2.delta_reliable,
            "A second turn must be poisoned: input absence from cross-session notification must hold even with no prior entry"
        );
    }

    /// Un-baselined variant (output-absent): symmetric mirror of the input-absent
    /// case above.  A has no entry; a cross-session notification with output absent
    /// (and all other counters present and nonzero) creates an entry with only
    /// `output_ever_poisoned = true`; the poison holds through A's second real turn.
    #[test]
    fn cross_session_absent_notification_latches_poison_unbaselined_output() {
        let mut t = UsageTracker::default();
        t.seed_zero_baseline("b");
        assert!(
            !t.sessions.contains_key("a"),
            "A must have no entry before the cross-session record"
        );

        // ── B in-flight; A notification: output absent, ALL other counter
        //    fields present with nonzero values ──
        //    input=15, cost=3.0, total=80, cached_input=7, cache_write=4.
        //    A has no prior entry, so the latch must CREATE one with all six
        //    baselines None — not initialized from these payload values.
        t.begin_turn("b");
        t.record(
            "a",
            &rich_payload(Some(15), None, Some(3.0), Some(80), Some(7), Some(4)),
        );

        // Entry must now exist with exactly the right shape.
        let created = t
            .sessions
            .get("a")
            .expect("latch must create an entry for A");
        assert_eq!(created.published_seq, 0, "created entry has zero seq");
        assert!(created.last_input.is_none());
        assert!(created.last_output.is_none());
        assert!(created.last_cost.is_none());
        assert!(created.last_total.is_none());
        assert!(created.last_cached_input.is_none());
        assert!(created.last_cache_write.is_none());
        assert!(
            !created.input_ever_poisoned,
            "input_ever_poisoned must NOT be set (only output was absent)"
        );
        assert!(
            created.output_ever_poisoned,
            "output_ever_poisoned must be set on the newly created entry"
        );

        t.record("b", &payload_opt(Some(100), Some(20)));
        let b1 = t.take().expect("b1");
        assert!(b1.delta_reliable, "B must be unaffected");

        // ── A first and second real turns ──
        t.begin_turn("a");
        t.record("a", &payload_opt(Some(80), Some(15)));
        let _a1 = t.take().expect("a1");

        t.begin_turn("a");
        t.record("a", &payload_opt(Some(150), Some(25)));
        let a2 = t.take().expect("a2");
        assert!(
            !a2.delta_reliable,
            "A second turn must be poisoned: output absence from cross-session notification must hold even with no prior entry"
        );
    }
}
