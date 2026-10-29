//! Lock-free latency histograms and counters shared by every simulated client.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

/// A measured operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    WsConnect,
    History,
    PublishEphemeral,
    PublishMessage,
    Deliver,
    HttpQuery,
    HttpEvents,
    Seed,
    Subscribe,
}

impl Op {
    pub const ALL: [Op; 9] = [
        Op::WsConnect,
        Op::History,
        Op::PublishEphemeral,
        Op::PublishMessage,
        Op::Deliver,
        Op::HttpQuery,
        Op::HttpEvents,
        Op::Seed,
        Op::Subscribe,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Op::WsConnect => "ws_connect_auth",
            Op::History => "history_eose",
            Op::PublishEphemeral => "publish_ephemeral_ok",
            Op::PublishMessage => "publish_message_ok",
            Op::Deliver => "deliver",
            Op::HttpQuery => "http_query",
            Op::HttpEvents => "http_events",
            Op::Seed => "seed",
            Op::Subscribe => "subscribe_eose",
        }
    }
}

/// Why a step crossed the abort thresholds, if it did. Every operation but
/// seeding counts toward the latency limit; delivery has no error signal of
/// its own, so it is left out of the error-rate limit. Operations in flight
/// when the window ends are reported but not judged: they have not failed
/// yet. The run stops and each one settles, so a stalled operation shows up
/// as an error in the shutdown window.
pub fn abort_reason(
    summaries: &[(Op, OpSummary)],
    max_error_rate: f64,
    max_p99_ms: f64,
) -> Option<String> {
    summaries
        .iter()
        .filter(|(op, s)| *op != Op::Seed && s.count + s.errors > 0)
        .find_map(|(op, s)| {
            if *op != Op::Deliver && s.error_rate() > max_error_rate {
                Some(format!(
                    "{} error rate {:.3} > {max_error_rate}",
                    op.name(),
                    s.error_rate()
                ))
            } else if s.p99_ms > max_p99_ms {
                Some(format!(
                    "{} p99 {:.1} ms > {max_p99_ms} ms",
                    op.name(),
                    s.p99_ms
                ))
            } else {
                None
            }
        })
}

/// Bucket `i` holds latencies up to `GROWTH^i` microseconds (2% resolution).
const GROWTH: f64 = 1.02;
const BUCKETS: usize = 1024;

fn bucket(d: Duration) -> usize {
    let us = d.as_micros().max(1) as f64;
    ((us.ln() / GROWTH.ln()).ceil() as usize).min(BUCKETS - 1)
}

fn bucket_ms(i: usize) -> f64 {
    GROWTH.powi(i as i32) / 1_000.0
}

struct OpStats {
    buckets: Vec<AtomicU64>,
    errors: AtomicU64,
    throttled: AtomicU64,
    /// Operations begun with [`Stats::begin`] and not yet finished.
    in_flight: AtomicU64,
}

impl OpStats {
    fn new() -> Self {
        Self {
            buckets: (0..BUCKETS).map(|_| AtomicU64::new(0)).collect(),
            errors: AtomicU64::new(0),
            throttled: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
        }
    }
}

/// Cumulative counters for every [`Op`].
pub struct Stats {
    ops: Vec<OpStats>,
    reasons: Mutex<HashMap<String, u64>>,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            ops: Op::ALL.iter().map(|_| OpStats::new()).collect(),
            reasons: Mutex::new(HashMap::new()),
        }
    }
}

impl Stats {
    fn op(&self, op: Op) -> &OpStats {
        &self.ops[Op::ALL
            .iter()
            .position(|o| *o == op)
            .expect("op listed in ALL")]
    }

    pub fn ok(&self, op: Op, latency: Duration) {
        self.op(op).buckets[bucket(latency)].fetch_add(1, Ordering::Relaxed);
    }

    /// Starts timing an operation. The guard counts as in flight until it is
    /// finished; one dropped unfinished, which only happens when its client is
    /// cancelled, records an error.
    pub fn begin(&self, op: Op) -> Pending<'_> {
        self.op(op).in_flight.fetch_add(1, Ordering::Relaxed);
        Pending {
            stats: self,
            op,
            start: Instant::now(),
            finished: false,
        }
    }

    /// Records a failure. Quota rejections count as throttled, not errors, so a
    /// run can tell relay limits apart from relay failures.
    pub fn err(&self, op: Op, reason: &str) {
        let throttled = is_throttle(reason);
        let stats = self.op(op);
        if throttled {
            stats.throttled.fetch_add(1, Ordering::Relaxed);
        } else {
            stats.errors.fetch_add(1, Ordering::Relaxed);
        }
        let key = format!("{}: {}", op.name(), normalize(reason));
        *self
            .reasons
            .lock()
            .expect("reasons lock")
            .entry(key)
            .or_default() += 1;
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            ops: self
                .ops
                .iter()
                .map(|s| OpSnapshot {
                    buckets: s
                        .buckets
                        .iter()
                        .map(|b| b.load(Ordering::Relaxed))
                        .collect(),
                    errors: s.errors.load(Ordering::Relaxed),
                    throttled: s.throttled.load(Ordering::Relaxed),
                    in_flight: s.in_flight.load(Ordering::Relaxed),
                })
                .collect(),
            reasons: self.reasons.lock().expect("reasons lock").clone(),
        }
    }
}

/// An operation started with [`Stats::begin`].
#[must_use = "an unfinished guard records an error when dropped"]
pub struct Pending<'a> {
    stats: &'a Stats,
    op: Op,
    start: Instant,
    finished: bool,
}

impl Pending<'_> {
    /// Records success with the latency since [`Stats::begin`].
    pub fn ok(mut self) {
        self.stats.ok(self.op, self.start.elapsed());
        self.finish();
    }

    /// Records a failure.
    pub fn err(mut self, reason: &str) {
        self.stats.err(self.op, reason);
        self.finish();
    }

    /// Ends the operation without recording it, for a failure the caller
    /// accounts for elsewhere.
    pub fn forget(mut self) {
        self.finish();
    }

    /// Leaves the in-flight gauge after the outcome is recorded, so a
    /// snapshot between the two sees the operation twice, never not at all.
    fn finish(&mut self) {
        self.finished = true;
        self.stats
            .op(self.op)
            .in_flight
            .fetch_sub(1, Ordering::Relaxed);
    }
}

impl Drop for Pending<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.stats.err(self.op, "unfinished at stop");
            self.finish();
        }
    }
}

/// Start of a reason built by `net::http_reason` for an HTTP 429.
const HTTP_TOO_MANY_REQUESTS: &str = "HTTP 429 ";
/// The relay's quota rejection, over WebSocket (`OK`, `CLOSED`) and in HTTP bodies.
const QUOTA_EXCEEDED: &str = "rate-limited: quota exceeded";

/// Quota rejections only: an HTTP 429 status or the relay's quota text. Other
/// `rate-limited:` rejections, such as `shared admission unavailable` when the
/// relay's admission store is down, are relay failures and count as errors.
fn is_throttle(reason: &str) -> bool {
    reason.starts_with(HTTP_TOO_MANY_REQUESTS) || reason.contains(QUOTA_EXCEEDED)
}

/// Collapses ids and numbers so equal failures aggregate under one reason.
fn normalize(reason: &str) -> String {
    let mut out: String = reason
        .split_whitespace()
        .map(|w| {
            if w.chars().any(|c| c.is_ascii_digit()) && w.len() > 3 {
                "<n>"
            } else {
                w
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if let Some((cut, _)) = out.char_indices().nth(120) {
        out.truncate(cut);
    }
    out
}

#[derive(Clone, Default)]
struct OpSnapshot {
    buckets: Vec<u64>,
    errors: u64,
    throttled: u64,
    in_flight: u64,
}

/// Point-in-time copy of [`Stats`]; subtract two to report one interval.
#[derive(Clone, Default)]
pub struct Snapshot {
    ops: Vec<OpSnapshot>,
    reasons: HashMap<String, u64>,
}

/// Summary of one operation over an interval.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpSummary {
    pub count: u64,
    pub errors: u64,
    pub throttled: u64,
    /// Operations in flight when the interval ended.
    pub pending: u64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

impl OpSummary {
    pub fn error_rate(&self) -> f64 {
        let total = self.count + self.errors;
        if total == 0 {
            0.0
        } else {
            self.errors as f64 / total as f64
        }
    }
}

impl Snapshot {
    /// Per-op summaries for the interval `earlier..self`.
    pub fn since(&self, earlier: &Snapshot) -> Vec<(Op, OpSummary)> {
        Op::ALL
            .iter()
            .enumerate()
            .map(|(i, op)| {
                let now = &self.ops[i];
                let then = earlier.ops.get(i).cloned().unwrap_or_default();
                let buckets: Vec<u64> = now
                    .buckets
                    .iter()
                    .enumerate()
                    .map(|(b, n)| n - then.buckets.get(b).copied().unwrap_or(0))
                    .collect();
                (
                    *op,
                    summarize(
                        &buckets,
                        now.errors - then.errors,
                        now.throttled - then.throttled,
                        now.in_flight,
                    ),
                )
            })
            .collect()
    }

    /// Failure reasons counted during `earlier..self`, most frequent first.
    pub fn reasons_since(&self, earlier: &Snapshot, limit: usize) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> = self
            .reasons
            .iter()
            .map(|(k, v)| (k.clone(), v - earlier.reasons.get(k).copied().unwrap_or(0)))
            .filter(|(_, v)| *v > 0)
            .collect();
        out.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        out.truncate(limit);
        out
    }
}

fn summarize(buckets: &[u64], errors: u64, throttled: u64, pending: u64) -> OpSummary {
    let count: u64 = buckets.iter().sum();
    let pct = |q: f64| {
        if count == 0 {
            return 0.0;
        }
        let rank = ((count as f64) * q).ceil().max(1.0) as u64;
        let mut seen = 0;
        for (i, n) in buckets.iter().enumerate() {
            seen += n;
            if seen >= rank {
                return bucket_ms(i);
            }
        }
        bucket_ms(BUCKETS - 1)
    };
    OpSummary {
        count,
        errors,
        throttled,
        pending,
        p50_ms: pct(0.50),
        p90_ms: pct(0.90),
        p99_ms: pct(0.99),
        max_ms: buckets
            .iter()
            .rposition(|n| *n > 0)
            .map(bucket_ms)
            .unwrap_or(0.0),
    }
}

/// JSON object of the non-empty op summaries in an interval of `secs` seconds.
pub fn ops_json(summaries: &[(Op, OpSummary)], secs: f64) -> Value {
    let mut map = Map::new();
    for (op, s) in summaries {
        if s.count + s.errors + s.throttled + s.pending == 0 {
            continue;
        }
        map.insert(
            op.name().into(),
            json!({
                "count": s.count,
                "errors": s.errors,
                "throttled": s.throttled,
                "pending": s.pending,
                "per_sec": round(s.count as f64 / secs.max(1e-9)),
                "p50_ms": round(s.p50_ms),
                "p90_ms": round(s.p90_ms),
                "p99_ms": round(s.p99_ms),
                "max_ms": round(s.max_ms),
            }),
        );
    }
    Value::Object(map)
}

fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_are_within_bucket_resolution() {
        let stats = Stats::default();
        for ms in 1..=100 {
            stats.ok(Op::HttpQuery, Duration::from_millis(ms));
        }
        let base = Snapshot::default();
        let summary = stats.snapshot().since(&base)[5].1;
        assert_eq!(summary.count, 100);
        assert!((summary.p50_ms - 50.0).abs() <= 1.1, "{summary:?}");
        assert!((summary.p99_ms - 99.0).abs() <= 2.0, "{summary:?}");
        assert!((summary.max_ms - 100.0).abs() <= 2.0, "{summary:?}");
    }

    #[test]
    fn intervals_subtract_and_throttles_are_not_errors() {
        let stats = Stats::default();
        stats.ok(Op::PublishMessage, Duration::from_millis(5));
        let first = stats.snapshot();
        stats.ok(Op::PublishMessage, Duration::from_millis(500));
        stats.err(
            Op::PublishMessage,
            "rate-limited: quota exceeded; retry in 12345s",
        );
        stats.err(Op::PublishMessage, "error: internal 98765");
        let interval = stats.snapshot().since(&first);
        let s = interval
            .iter()
            .find(|(o, _)| *o == Op::PublishMessage)
            .unwrap()
            .1;
        assert_eq!((s.count, s.errors, s.throttled), (1, 1, 1));
        assert!(s.p50_ms > 400.0);
        let reasons = stats.snapshot().reasons_since(&first, 10);
        assert!(reasons
            .iter()
            .any(|(r, _)| r == "publish_message_ok: error: internal <n>"));
    }

    #[test]
    fn long_reasons_truncate_on_char_boundaries() {
        let stats = Stats::default();
        for reason in ["a".repeat(119) + "é tail", "é".repeat(200), "x".repeat(300)] {
            stats.err(Op::HttpEvents, &reason);
        }
        let reasons = stats.snapshot().reasons_since(&Snapshot::default(), 10);
        assert_eq!(reasons.len(), 3);
        assert!(reasons
            .iter()
            .all(|(r, _)| r.chars().count() <= "http_events: ".len() + 120));
    }

    #[test]
    fn abort_judges_error_rate_and_every_latency_including_delivery() {
        let stats = Stats::default();
        stats.ok(Op::PublishMessage, Duration::from_millis(2));
        stats.ok(Op::Deliver, Duration::from_millis(900));
        let base = Snapshot::default();
        let slow_delivery = stats.snapshot().since(&base);
        let reason = abort_reason(&slow_delivery, 0.05, 500.0).expect("slow delivery aborts");
        assert!(reason.starts_with("deliver p99"), "{reason}");
        assert_eq!(abort_reason(&slow_delivery, 0.05, 5_000.0), None);

        stats.err(Op::PublishMessage, "error: boom");
        let failing = stats.snapshot().since(&base);
        let reason = abort_reason(&failing, 0.05, 5_000.0).expect("errors abort");
        assert!(
            reason.starts_with("publish_message_ok error rate"),
            "{reason}"
        );

        let seeding = Stats::default();
        seeding.err(Op::Seed, "error: boom");
        assert_eq!(
            abort_reason(&seeding.snapshot().since(&base), 0.05, 1.0),
            None
        );
    }

    fn summary(stats: &Stats, op: Op) -> OpSummary {
        let window = stats.snapshot().since(&Snapshot::default());
        window.iter().find(|(o, _)| *o == op).unwrap().1
    }

    #[test]
    fn begun_operations_count_in_flight_until_finished_and_unfinished_drops_are_errors() {
        let stats = Stats::default();
        let waiting = stats.begin(Op::WsConnect);
        let succeeds = stats.begin(Op::WsConnect);
        let fails = stats.begin(Op::WsConnect);
        let handled_elsewhere = stats.begin(Op::WsConnect);
        assert_eq!(summary(&stats, Op::WsConnect).pending, 4);

        succeeds.ok();
        fails.err("connect timeout");
        handled_elsewhere.forget();
        let s = summary(&stats, Op::WsConnect);
        assert_eq!((s.count, s.errors, s.pending), (1, 1, 1));

        // A guard dropped unfinished, as when its task is aborted, is an
        // error, not a vanished sample.
        drop(waiting);
        let s = summary(&stats, Op::WsConnect);
        assert_eq!((s.count, s.errors, s.pending), (1, 2, 0));
        let reasons = stats.snapshot().reasons_since(&Snapshot::default(), 10);
        assert!(reasons
            .iter()
            .any(|(r, n)| r == "ws_connect_auth: unfinished at stop" && *n == 1));
    }

    #[test]
    fn in_flight_operations_are_reported_but_not_judged() {
        let stats = Stats::default();
        let straddling = stats.begin(Op::PublishEphemeral);
        let window = stats.snapshot().since(&Snapshot::default());
        // One healthy operation still in flight is not a 100% error rate.
        assert_eq!(abort_reason(&window, 0.0, 5_000.0), None);
        assert_eq!(summary(&stats, Op::PublishEphemeral).error_rate(), 0.0);
        assert_eq!(ops_json(&window, 1.0)["publish_ephemeral_ok"]["pending"], 1);

        // Once it settles as a failure, it counts.
        straddling.err("OK timeout");
        let window = stats.snapshot().since(&Snapshot::default());
        let reason = abort_reason(&window, 0.05, 5_000.0).expect("a settled error aborts");
        assert!(
            reason.starts_with("publish_ephemeral_ok error rate 1.000"),
            "{reason}"
        );
    }

    #[test]
    fn only_quota_rejections_are_throttles() {
        use crate::loadgen::net::http_reason;
        use reqwest::StatusCode;
        let throttles = [
            // WebSocket OK and CLOSED text from `rejection.rs`.
            "rate-limited: quota exceeded; retry in 7s",
            "CLOSED live: rate-limited: quota exceeded; retry in 7s",
            // HTTP 429 from `api/bridge.rs`, with and without its body.
            &http_reason(
                StatusCode::TOO_MANY_REQUESTS,
                r#"{"error":"rate-limited: quota exceeded; retry in 3s"}"#,
            ),
            &http_reason(StatusCode::TOO_MANY_REQUESTS, "slow down"),
        ];
        let errors = [
            // The admission store is down: an outage, not a quota.
            "rate-limited: shared admission unavailable",
            &http_reason(
                StatusCode::SERVICE_UNAVAILABLE,
                r#"{"error":"rate-limited: shared admission unavailable"}"#,
            ),
            "rate-limited: too many concurrent requests",
            // Numbers that merely contain 429, and prose that mentions limits.
            "invalid: created_at 1793242912 is too far in the past",
            "error: event 429abc is not valid",
            "rate limit configuration is invalid",
            &http_reason(StatusCode::BAD_REQUEST, "created_at 1793242912 rejected"),
            &http_reason(StatusCode::INTERNAL_SERVER_ERROR, "HTTP 429 upstream"),
        ];
        let stats = Stats::default();
        for reason in throttles.iter().chain(errors.iter()) {
            stats.err(Op::HttpEvents, reason);
        }
        let s = summary(&stats, Op::HttpEvents);
        assert_eq!(
            (s.throttled, s.errors),
            (throttles.len() as u64, errors.len() as u64)
        );
    }
}
