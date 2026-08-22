//! Phase B causal audit trail — local, append-only, metadata-only, fail-open.
//!
//! Single sink for the nine MVP causal boundaries defined in
//! `HIVE_PHASE_B_EMIT_POINT_SPEC_FIZZ_HONEY.md`: `EVENT_PUBLISHED`,
//! `RELAY_ACCEPTED`, `EVENT_RECEIVED`, `FILTER_DECISION`, `QUEUE_DECISION`,
//! `WAKE_DECISION`, `ACP_SESSION_STARTED`, `RESPONSE_PUBLISHED`,
//! `RECIPIENT_EVIDENCE_OBSERVED`.
//!
//! No hook lives in this file — it defines the schema and the sink only.
//! Call sites are added elsewhere, one per boundary, each an additive
//! `audit::record(...)` statement next to an existing decision.
//!
//! # Design constraints (Schema V3 + sink architecture, approved after
//! causality and hot-path review)
//!
//! - **Local only by default.** `$HOME/.buzz/audit/buzz-acp-audit.jsonl` —
//!   no network call, no relay publish, no database. This is deliberately
//!   *not* [`crate::observer`], which exists to publish encrypted telemetry
//!   frames to the relay. `BUZZ_ACP_AUDIT_PATH`, when set, is used verbatim
//!   and is entirely operator-controlled: nothing here asserts an explicit
//!   override can't point at a mounted or network filesystem — only the
//!   *default* (no override) is guaranteed strictly local.
//! - **Metadata only, redaction by construction.** [`AuditFields`] and
//!   [`EventDetail`] have no field that can hold raw event content, prompts,
//!   secrets, or arbitrary relay-supplied text.
//! - **Global, opt-in, default OFF.** Controlled by `BUZZ_ACP_AUDIT`, read
//!   once at startup — outside [`crate::config::Config`]'s clap/env surface
//!   (per-agent business configuration), and unrelated to the Phase A
//!   per-agent `RUST_LOG` experiment.
//! - **Non-blocking producer, fail-open sink.** `record()` never performs
//!   filesystem I/O and never awaits: it builds a JSON line in memory and
//!   hands it to a bounded `std::sync::mpsc::sync_channel` via `try_send`
//!   only. A dedicated named OS thread is the sole owner of the `File` and
//!   does all actual disk I/O off the causal hot path. Backpressure
//!   (channel full or the writer thread gone) drops the record, counts it,
//!   and logs a rate-limited warning — it never blocks or retries. A write
//!   error inside the writer thread is likewise counted, rate-limited, and
//!   never propagates or panics.
//! - **No invented business state.** There is no `handoff_id` and no
//!   `source_event_id`. Two separately-signed events cannot be proven to be
//!   "the same logical handoff, retried" from transport data alone.
//!   `workflow_id` is a verbatim tag pass-through, populated only when the
//!   Hive protocol layer actually sets it. `attempt` is populated only
//!   where [`crate::queue::EventQueue`] has an authoritative retry counter
//!   (`QUEUE_DECISION` / `WAKE_DECISION`), `None` everywhere else.
//! - **Honest batch cardinality.** `WAKE_DECISION` and `ACP_SESSION_STARTED`
//!   can each concern more than one physical event (a `FlushBatch` may hold
//!   several). The scalar `AuditFields::event_id` is `None` at both — the
//!   full triggering set lives in `EventDetail::WakeDecision` /
//!   `EventDetail::AcpSession` instead. The two are paired by `turn_id`
//!   (reused verbatim from `dispatch_pending`'s existing `Uuid::new_v4()`
//!   local, not invented here) and independently verifiable by equal
//!   `triggering_event_ids`.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::OnceLock;

use serde::Serialize;

/// Environment variable that globally enables the audit sink for this
/// process. Accepts `1` / `true` (case-insensitive); anything else —
/// including unset — is disabled. The only switch: no per-agent override,
/// no config-file key, no relationship to `RUST_LOG`.
const AUDIT_ENABLE_ENV: &str = "BUZZ_ACP_AUDIT";

/// Optional override for the JSONL sink path. When unset, defaults to
/// `$HOME/.buzz/audit/` + [`DEFAULT_AUDIT_FILENAME`].
const AUDIT_PATH_ENV: &str = "BUZZ_ACP_AUDIT_PATH";

const DEFAULT_AUDIT_FILENAME: &str = "buzz-acp-audit.jsonl";

/// Bounded producer→writer channel capacity. Fixed — guarantees bounded
/// memory regardless of audit volume; excess records are dropped, not
/// queued without limit.
const CHANNEL_CAPACITY: usize = 4096;

/// Schema iteration. Bump on any wire-format-breaking change to
/// [`AuditFields`] / [`EventDetail`].
const SCHEMA_VERSION: u32 = 1;

/// Log a backpressure/write-error warning only every Nth occurrence, so a
/// sustained overload can't turn the warning itself into a second source of
/// load. The first occurrence (n == 1) always logs immediately.
const WARN_LOG_MODULUS: u64 = 100;

/// The nine MVP causal boundaries. Serializes as the `event_type` field.
///
/// `TransportPublished` / `TransportAccepted` stand in for the spec's
/// `EVENT_PUBLISHED` / `RELAY_ACCEPTED` slots, but are deliberately NOT
/// named that: source inspection confirmed genuine agent handoff/chat
/// content (kind:9, sent via `buzz messages send`) is published by the
/// separate `buzz-cli` binary over its own REST connection, never through
/// buzz-acp's WS `HarnessRelay`. buzz-acp's WS publish path only ever
/// carries its own internal traffic (typing indicators, presence,
/// membership, observer frames) — real, useful transport-health telemetry,
/// but not the causal handoff/response fact `EVENT_PUBLISHED` /
/// `RELAY_ACCEPTED` are meant to prove. Those two spec-canonical names are
/// reserved for a future buzz-cli-side implementation and must not be
/// reused here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    /// buzz-acp's own WS publish path (`relay.rs::execute_connected_command`,
    /// `RelayCommand::PublishEvent`). Does NOT represent real agent
    /// handoff/chat publication and does NOT cover kind:9 messages sent via
    /// `buzz messages send` — those go out through buzz-cli's independent
    /// REST client, never through this process's relay connection.
    TransportPublished,
    /// Relay OK acknowledgment for buzz-acp's own WS-published traffic
    /// (`relay.rs::handle_ws_message`, `RelayMessage::Ok`). Does NOT
    /// represent acceptance of real agent chat messages — those are
    /// published through buzz-cli's REST path and acknowledged via that
    /// HTTP response, not this WS OK frame.
    TransportAccepted,
    EventReceived,
    FilterDecision,
    QueueDecision,
    WakeDecision,
    AcpSessionStarted,
    ResponsePublished,
    RecipientEvidenceObserved,
}

/// Outcome of a `QUEUE_DECISION`. Sourced only in
/// `queue.rs::EventQueue::push`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum QueueOutcome {
    Accepted,
    /// [`DedupMode::Drop`](crate::config::DedupMode) discarded the event for
    /// an in-flight channel.
    DroppedInFlight,
    /// Per-channel depth cap evicted the oldest queued event to make room.
    CapEvicted,
}

/// Outcome of a `WAKE_DECISION`. Sourced only in `lib.rs::dispatch_pending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WakeOutcome {
    Claimed,
    PoolExhausted,
}

/// Boundary-specific extras. Only the fields a boundary can actually source
/// appear on its variant — e.g. `rule_index` cannot be set on a
/// `WAKE_DECISION` record because that variant has no such field.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventDetail {
    /// `TransportPublished`, `EVENT_RECEIVED`, `RESPONSE_PUBLISHED`,
    /// `RECIPIENT_EVIDENCE_OBSERVED` — nothing beyond the shared envelope.
    Empty,
    FilterDecision {
        rule_index: Option<usize>,
        fail_closed: bool,
    },
    QueueDecision {
        outcome: QueueOutcome,
    },
    /// `TransportAccepted` — accept/reject only. Deliberately no
    /// relay-supplied message text: that string is arbitrary,
    /// externally-controlled content and has no place in a metadata-only,
    /// redaction-by-construction schema.
    RelayAck {
        accepted: bool,
    },
    WakeDecision {
        /// Exact event set from the `FlushBatch` this decision concerns —
        /// same computation `pool.rs::run_prompt_task` already performs
        /// (`batch.events.iter().map(|be| be.event.id.to_hex())`), not a
        /// new derivation. May hold more than one id.
        triggering_event_ids: Vec<String>,
        outcome: WakeOutcome,
    },
    /// `ACP_SESSION_STARTED`. Same event set as the `WakeDecision` sharing
    /// this record's `turn_id`, by construction (same moved `FlushBatch`).
    AcpSession {
        triggering_event_ids: Vec<String>,
    },
}

/// Shared envelope fields, common to all nine boundaries.
///
/// No `content`, `prompt`, `handoff_id`, or `source_event_id` field exists
/// on this type — the first two because a hook site has nothing to pass if
/// it wanted to leak a raw message body; the latter two because neither is
/// derivable from transport data without either fabricating a value or
/// conflating two distinct concepts (see the causality-review addenda).
#[derive(Debug, Default, Clone, Serialize)]
pub struct AuditFields {
    /// The single physical Nostr event id (hex) this record concerns.
    /// Populated at `TransportPublished`, `TransportAccepted` (bare acked
    /// id), `EVENT_RECEIVED`, `FILTER_DECISION`, `QUEUE_DECISION`,
    /// `RESPONSE_PUBLISHED`, `RECIPIENT_EVIDENCE_OBSERVED` — all
    /// structurally one-event-at-a-time boundaries. Always `None` at
    /// `WAKE_DECISION` / `ACP_SESSION_STARTED`, which are batch-cardinality
    /// (see `EventDetail::WakeDecision` / `EventDetail::AcpSession`).
    pub event_id: Option<String>,
    /// NIP-10 direct-reply parent event id.
    pub direct_parent_event_id: Option<String>,
    /// NIP-10 thread-root event id.
    pub thread_root_event_id: Option<String>,
    /// `thread_root_event_id`, falling back to the event's own id when it
    /// has no parent (it IS the root). Thread-level grouping only — NOT a
    /// unique per-handoff identity.
    pub correlation_id: Option<String>,
    /// Verbatim pass-through of a `workflow_id` tag, ONLY when the Hive
    /// protocol layer above buzz-acp actually set one. Never interpreted
    /// here, never derived, never defaulted.
    pub workflow_id: Option<String>,
    pub sender_pubkey: Option<String>,
    /// Provenance is boundary-dependent — this field is NOT one consistent
    /// kind of evidence across `event_type`s, and must always be read
    /// together with it:
    /// - At `FILTER_DECISION` (`filter.rs`), the value is the authoritative
    ///   local agent pubkey supplied to `match_event` as `agent_pubkey_hex`:
    ///   this process genuinely IS the recipient there.
    /// - At `EVENT_RECEIVED` (`relay.rs`), the value is the authoritative
    ///   local recipient process pubkey already in scope as
    ///   `agent_pubkey_hex` — same reasoning, different file.
    /// - At `QUEUE_DECISION` (`queue.rs`), and any other boundary populated
    ///   via [`first_mentioned_pubkey`], the value is merely the first
    ///   signed `p` tag found on the event — mention/target evidence only.
    ///   A `p` tag records who the event's author addressed; it is NOT a
    ///   confirmation that any particular process received or claimed the
    ///   role of recipient.
    /// Consumers MUST NOT interpret `target_pubkey` generically as proof of
    /// process-recipient identity — its evidentiary weight depends entirely
    /// on which boundary (`event_type`) produced the record.
    pub target_pubkey: Option<String>,
    pub channel_id: Option<String>,
    pub agent_session_id: Option<String>,
    /// Dispatch/turn identity spanning exactly one `WAKE_DECISION` and its
    /// paired `ACP_SESSION_STARTED`. `Some(_)` ONLY at `WAKE_DECISION`
    /// (`Claimed` outcome — `PoolExhausted` never reaches the point a
    /// turn_id is assigned) and `ACP_SESSION_STARTED`. Sourced verbatim
    /// from `dispatch_pending`'s existing `turn_id` local, not invented.
    pub turn_id: Option<String>,
    /// `EventQueue`'s per-channel dispatch-retry counter. `Some(_)` ONLY at
    /// `QUEUE_DECISION` (`queue.rs::push`, `&self.retry_counts`) and
    /// `WAKE_DECISION` (`lib.rs::dispatch_pending`, via a new
    /// `EventQueue::retry_count_for` accessor). This is NOT a count of
    /// business-level (Nostr republish) retries, which buzz-acp cannot
    /// observe — no default-0 fallback exists anywhere in this module.
    pub attempt: Option<u32>,
}

/// Thread-level correlation key for an event: its NIP-10 thread-root id, or
/// its own id when it has no parent (it IS the root). Reuses
/// [`crate::queue::parse_thread_tags`] rather than re-implementing NIP-10
/// parsing. Metadata only — reads `.id` / `.tags`, never `.content`.
pub fn correlation_id_for(event: &nostr::Event) -> String {
    crate::queue::parse_thread_tags(event)
        .root_event_id
        .unwrap_or_else(|| event.id.to_hex())
}

/// First mentioned pubkey (`p` tag) on an event — used as `target_pubkey`
/// where no more authoritative source (e.g. this process's own identity via
/// an `agent_pubkey_hex` parameter already in scope) is available. Metadata
/// only.
pub fn first_mentioned_pubkey(event: &nostr::Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.len() >= 2 && parts[0] == "p").then(|| parts[1].clone())
    })
}

/// Best-effort, verbatim pass-through of a `workflow_id` tag. buzz-acp does
/// not define, require, or interpret this tag — Hive workflow semantics are
/// out of scope here; this only forwards what's already signed on the wire.
pub fn workflow_id_for(event: &nostr::Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.len() >= 2 && parts[0] == "workflow_id").then(|| parts[1].clone())
    })
}

/// Handle to the background writer. The producer side only ever holds a
/// cheap, cloneable `SyncSender` — the `File` itself belongs exclusively to
/// the writer thread.
struct AuditSink {
    tx: SyncSender<String>,
    /// Records dropped due to a full or disconnected channel.
    dropped: AtomicU64,
    seq: AtomicU64,
}

static SINK: OnceLock<Option<AuditSink>> = OnceLock::new();

/// Initialize the global audit sink from the environment. Idempotent — call
/// once at process startup; later calls are no-ops against the
/// already-initialized [`OnceLock`].
///
/// Fail-open: any error resolving `HOME`, creating the audit directory,
/// opening the file, or spawning the writer thread disables auditing for
/// this process. Never panics, never blocks startup.
pub fn init() {
    SINK.get_or_init(build_sink);
}

fn enabled_from_env() -> bool {
    std::env::var(AUDIT_ENABLE_ENV)
        .map(|v| matches!(v.trim(), "1" | "true" | "TRUE" | "True"))
        .unwrap_or(false)
}

/// `$HOME/.buzz/audit` — the default sink directory. `None` when `HOME` is
/// unset or empty, in which case the location would no longer be
/// deterministic and the caller fails open (disables auditing) rather than
/// guessing a CWD-relative fallback.
fn default_audit_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join(".buzz").join("audit"))
}

/// Resolve the JSONL sink path. `BUZZ_ACP_AUDIT_PATH`, when set to a
/// non-empty value, is used verbatim — it is operator-controlled and may
/// point anywhere the operator chooses, local or otherwise; this function
/// makes no claim about it. Only the no-override default is guaranteed
/// local, and only after successfully creating `$HOME/.buzz/audit/` (itself
/// fail-open on failure).
fn resolve_audit_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var(AUDIT_PATH_ENV) {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    let dir = default_audit_dir()?;
    if let Err(error) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            target: "audit",
            path = %dir.display(),
            %error,
            "failed to create default audit directory; auditing disabled for this process"
        );
        return None;
    }
    Some(dir.join(DEFAULT_AUDIT_FILENAME))
}

fn build_sink() -> Option<AuditSink> {
    if !enabled_from_env() {
        return None;
    }
    let path = resolve_audit_path()?;
    let file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => file,
        Err(error) => {
            tracing::warn!(
                target: "audit",
                path = %path.display(),
                %error,
                "failed to open audit sink; auditing disabled for this process"
            );
            return None;
        }
    };

    let (tx, rx) = sync_channel::<String>(CHANNEL_CAPACITY);
    let spawned = std::thread::Builder::new()
        .name("buzz-acp-audit-writer".to_string())
        .spawn(move || {
            // Single-threaded by construction — a plain local counter is
            // enough, no atomics needed on this side.
            let mut file = file;
            let mut write_errors: u64 = 0;
            while let Ok(line) = rx.recv() {
                if let Err(error) = writeln!(file, "{line}") {
                    write_errors += 1;
                    if write_errors % WARN_LOG_MODULUS == 1 {
                        tracing::warn!(
                            target: "audit",
                            %error,
                            count = write_errors,
                            "audit write failed (rate-limited log)"
                        );
                    }
                    // Never panic on a transient I/O error — keep draining
                    // so a temporarily-unwritable sink doesn't wedge the
                    // channel; the caller already moved on either way.
                }
            }
        });

    match spawned {
        Ok(_handle) => {
            // Detached deliberately: the writer runs for the process
            // lifetime, fed only by `SINK`'s `SyncSender`. No explicit
            // shutdown/flush is required — fail-open allows dropped
            // records, and each successful write is its own syscall (no
            // extra application-level buffering to flush).
            tracing::info!(target: "audit", path = %path.display(), "buzz-acp audit sink enabled");
            Some(AuditSink {
                tx,
                dropped: AtomicU64::new(0),
                seq: AtomicU64::new(0),
            })
        }
        Err(error) => {
            tracing::warn!(
                target: "audit",
                %error,
                "failed to spawn audit writer thread; auditing disabled for this process"
            );
            None
        }
    }
}

/// True when the audit sink is enabled for this process. Cheap — callers on
/// hot paths may use this to skip building [`AuditFields`] entirely when
/// disabled, though [`record`] itself is already a no-op in that case.
pub fn is_enabled() -> bool {
    SINK.get().is_some_and(Option::is_some)
}

/// One JSONL line. Assembled only inside [`record`] — callers never
/// construct this directly, so `schema_version` / `ts` / `seq` can never be
/// forgotten or spoofed by a call site.
#[derive(Serialize)]
struct AuditRecord<'a> {
    schema_version: u32,
    ts: String,
    seq: u64,
    event_type: EventType,
    #[serde(flatten)]
    fields: &'a AuditFields,
    detail: &'a EventDetail,
}

/// Record one causal-boundary event.
///
/// No-op when auditing is disabled or uninitialized. Never awaits, never
/// performs filesystem I/O itself, never panics: this function only builds
/// a JSON string in memory and hands it to the writer thread via
/// [`SyncSender::try_send`]. A serialization failure, a full channel, or a
/// disconnected writer are all handled the same way — the record is
/// dropped, counted, and (rate-limited) logged; the caller is never blocked
/// and never sees an error.
pub fn record(event_type: EventType, fields: AuditFields, detail: EventDetail) {
    let Some(sink) = SINK.get().and_then(|s| s.as_ref()) else {
        return;
    };
    let seq = sink.seq.fetch_add(1, Ordering::Relaxed);
    let record = AuditRecord {
        schema_version: SCHEMA_VERSION,
        ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true),
        seq,
        event_type,
        fields: &fields,
        detail: &detail,
    };
    let line = match serde_json::to_string(&record) {
        Ok(line) => line,
        Err(error) => {
            tracing::warn!(target: "audit", %error, "failed to serialize audit record; dropping");
            return;
        }
    };
    match sink.tx.try_send(line) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
            let n = sink.dropped.fetch_add(1, Ordering::Relaxed) + 1;
            if n % WARN_LOG_MODULUS == 1 {
                tracing::warn!(
                    target: "audit",
                    dropped = n,
                    "audit channel backpressure — dropping records (rate-limited log)"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Env-var-touching tests must run serially — env vars are process-
    /// global. Mirrors `lib.rs`'s `build_mcp_servers_tests::ENV_LOCK` idiom
    /// exactly. No other file in this crate reads `BUZZ_ACP_AUDIT`,
    /// `BUZZ_ACP_AUDIT_PATH`, or `HOME` (confirmed by grep before writing
    /// these tests), so this lock only needs to protect tests within this
    /// module from each other.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard: sets (or unsets) an env var for the duration of a test
    /// and restores its exact prior value on drop — `Some(v)` restores `v`,
    /// `None` removes the var entirely. Runs on the panic/unwind path too
    /// (a failed `assert!` still drops locals), so a test failure can never
    /// leak mutated env state to a later test in the same binary. Must be
    /// held only while `ENV_LOCK` is also held.
    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// No test in this module ever calls [`init`]. `SINK` therefore stays
    /// uninitialized for the entire test binary (nothing else in the crate
    /// calls `audit::init()` either — it's only reached from
    /// `lib.rs::run()`'s production entry point). This is deliberate: `SINK`
    /// is a `OnceLock` and can only be populated once per process, so tests
    /// must not race to be "the one" that initializes it. Every test below
    /// instead calls the pure, stateless functions `build_sink()` /
    /// `resolve_audit_path()` / `enabled_from_env()` directly, or exercises
    /// `record()`/`is_enabled()` only in the (safe, permanent-for-this-
    /// binary) uninitialized state.

    // ---- T1: schema / envelope shape ----------------------------------

    const EXPECTED_ENVELOPE_KEYS: &[&str] = &[
        "schema_version",
        "ts",
        "seq",
        "event_type",
        "event_id",
        "direct_parent_event_id",
        "thread_root_event_id",
        "correlation_id",
        "workflow_id",
        "sender_pubkey",
        "target_pubkey",
        "channel_id",
        "agent_session_id",
        "turn_id",
        "attempt",
        "detail",
    ];

    const OBSOLETE_FIELD_NAMES: &[&str] = &[
        "handoff_id",
        "source_event_id",
        "runtime_id",
        "pid",
        "buzz_acp_version",
        "stage",
        "latency_ms",
    ];

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn envelope_serializes_expected_keys_and_omits_obsolete_fields() {
        let fields = AuditFields {
            event_id: Some("deadbeef".into()),
            ..Default::default()
        };
        let record = AuditRecord {
            schema_version: SCHEMA_VERSION,
            ts: "2026-01-01T00:00:00.000000000Z".into(),
            seq: 0,
            event_type: EventType::EventReceived,
            fields: &fields,
            detail: &EventDetail::Empty,
        };
        let value = serde_json::to_value(&record).expect("serialize");
        let obj = value.as_object().expect("record must be a JSON object");

        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected: Vec<&str> = EXPECTED_ENVELOPE_KEYS.to_vec();
        expected.sort_unstable();
        assert_eq!(keys, expected, "envelope key set changed unexpectedly");

        for obsolete in OBSOLETE_FIELD_NAMES {
            assert!(
                !obj.contains_key(*obsolete),
                "obsolete field {obsolete:?} must never reappear in the schema"
            );
        }

        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["event_type"], "EVENT_RECEIVED");
        assert_eq!(value["event_id"], "deadbeef");
    }

    #[test]
    fn event_type_variants_serialize_screaming_snake_case() {
        let cases = [
            (EventType::TransportPublished, "TRANSPORT_PUBLISHED"),
            (EventType::TransportAccepted, "TRANSPORT_ACCEPTED"),
            (EventType::EventReceived, "EVENT_RECEIVED"),
            (EventType::FilterDecision, "FILTER_DECISION"),
            (EventType::QueueDecision, "QUEUE_DECISION"),
            (EventType::WakeDecision, "WAKE_DECISION"),
            (EventType::AcpSessionStarted, "ACP_SESSION_STARTED"),
            (EventType::ResponsePublished, "RESPONSE_PUBLISHED"),
            (
                EventType::RecipientEvidenceObserved,
                "RECIPIENT_EVIDENCE_OBSERVED",
            ),
        ];
        for (variant, expected) in cases {
            let value = serde_json::to_value(variant).expect("serialize");
            assert_eq!(value, expected);
        }
    }

    #[test]
    fn filter_decision_detail_shape() {
        let detail = EventDetail::FilterDecision {
            rule_index: Some(2),
            fail_closed: true,
        };
        let value = serde_json::to_value(&detail).expect("serialize");
        assert_eq!(value["kind"], "FILTER_DECISION");
        assert_eq!(value["rule_index"], 2);
        assert_eq!(value["fail_closed"], true);
    }

    #[test]
    fn queue_decision_detail_shape() {
        let detail = EventDetail::QueueDecision {
            outcome: QueueOutcome::CapEvicted,
        };
        let value = serde_json::to_value(&detail).expect("serialize");
        assert_eq!(value["kind"], "QUEUE_DECISION");
        assert_eq!(value["outcome"], "CAP_EVICTED");
    }

    #[test]
    fn relay_ack_detail_has_no_message_field() {
        let detail = EventDetail::RelayAck { accepted: false };
        let value = serde_json::to_value(&detail).expect("serialize");
        let obj = value.as_object().expect("must be an object");
        assert_eq!(value["kind"], "RELAY_ACK");
        assert_eq!(value["accepted"], false);
        assert_eq!(
            obj.len(),
            2,
            "RelayAck must carry only kind+accepted, never a relay message string"
        );
        assert!(!obj.contains_key("message"));
    }

    #[test]
    fn wake_decision_detail_shape() {
        let detail = EventDetail::WakeDecision {
            triggering_event_ids: vec!["ev1".into(), "ev2".into()],
            outcome: WakeOutcome::PoolExhausted,
        };
        let value = serde_json::to_value(&detail).expect("serialize");
        assert_eq!(value["kind"], "WAKE_DECISION");
        assert_eq!(value["outcome"], "POOL_EXHAUSTED");
        assert_eq!(value["triggering_event_ids"], serde_json::json!(["ev1", "ev2"]));
    }

    #[test]
    fn acp_session_detail_shape() {
        let detail = EventDetail::AcpSession {
            triggering_event_ids: vec!["ev1".into()],
        };
        let value = serde_json::to_value(&detail).expect("serialize");
        assert_eq!(value["kind"], "ACP_SESSION");
        assert_eq!(value["triggering_event_ids"], serde_json::json!(["ev1"]));
    }

    // ---- T2: metadata / redaction-by-construction ----------------------

    #[test]
    fn sentinel_populated_fields_serialize_as_plain_strings_no_extra_keys() {
        // Synthetic, obviously-fake sentinel values only — never real
        // secrets, keys, or content.
        let fields = AuditFields {
            event_id: Some("SENTINEL_EVENT_ID".into()),
            direct_parent_event_id: Some("SENTINEL_PARENT_ID".into()),
            thread_root_event_id: Some("SENTINEL_ROOT_ID".into()),
            correlation_id: Some("SENTINEL_CORRELATION_ID".into()),
            workflow_id: Some("SENTINEL_WORKFLOW_ID".into()),
            sender_pubkey: Some("SENTINEL_SENDER_PUBKEY".into()),
            target_pubkey: Some("SENTINEL_TARGET_PUBKEY".into()),
            channel_id: Some("SENTINEL_CHANNEL_ID".into()),
            agent_session_id: Some("SENTINEL_SESSION_ID".into()),
            turn_id: Some("SENTINEL_TURN_ID".into()),
            attempt: Some(9),
        };
        let record = AuditRecord {
            schema_version: SCHEMA_VERSION,
            ts: "2026-01-01T00:00:00.000000000Z".into(),
            seq: 42,
            event_type: EventType::FilterDecision,
            fields: &fields,
            detail: &EventDetail::FilterDecision {
                rule_index: Some(0),
                fail_closed: false,
            },
        };
        let value = serde_json::to_value(&record).expect("serialize");
        let obj = value.as_object().expect("must be an object");

        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected: Vec<&str> = EXPECTED_ENVELOPE_KEYS.to_vec();
        expected.sort_unstable();
        assert_eq!(
            keys, expected,
            "fully-populated record must not gain or lose keys"
        );

        // Every sentinel must round-trip unchanged as a plain string — no
        // hidden expansion, parsing, or restructuring of caller-supplied
        // metadata values.
        assert_eq!(value["event_id"], "SENTINEL_EVENT_ID");
        assert_eq!(value["sender_pubkey"], "SENTINEL_SENDER_PUBKEY");
        assert_eq!(value["target_pubkey"], "SENTINEL_TARGET_PUBKEY");
        assert_eq!(value["turn_id"], "SENTINEL_TURN_ID");
        assert_eq!(value["attempt"], 9);

        // No field anywhere in this type is named/shaped for raw content —
        // this is a structural guarantee, re-asserted here as a regression
        // check on the field list itself.
        for forbidden in ["content", "prompt", "body", "message", "text"] {
            assert!(
                !obj.contains_key(forbidden),
                "AuditFields must never gain a field named {forbidden:?}"
            );
        }
    }

    // ---- T7: audit OFF ---------------------------------------------------

    #[test]
    fn enabled_from_env_true_variants() {
        let _guard = ENV_LOCK.lock().unwrap();
        for v in ["1", "true", "True", "TRUE"] {
            let _env = EnvVarGuard::set(AUDIT_ENABLE_ENV, v);
            assert!(enabled_from_env(), "expected enabled for {v:?}");
        }
    }

    #[test]
    fn enabled_from_env_false_for_unset_and_other_values() {
        let _guard = ENV_LOCK.lock().unwrap();
        {
            let _env = EnvVarGuard::unset(AUDIT_ENABLE_ENV);
            assert!(!enabled_from_env(), "unset must be disabled");
        }
        for v in ["0", "false", "yes", "enabled", ""] {
            let _env = EnvVarGuard::set(AUDIT_ENABLE_ENV, v);
            assert!(!enabled_from_env(), "expected disabled for {v:?}");
        }
    }

    #[test]
    fn record_is_noop_and_does_not_panic_without_init() {
        // Deliberately never calls audit::init() anywhere in this module —
        // see the module-level doc comment above.
        let fields = AuditFields {
            event_id: Some("noop-test-event".into()),
            ..Default::default()
        };
        record(EventType::EventReceived, fields, EventDetail::Empty);
        // No panic = pass. There is no observable side channel from outside
        // this module without touching the real filesystem sink, which this
        // test deliberately avoids.
    }

    #[test]
    fn is_enabled_false_without_init() {
        assert!(!is_enabled());
    }

    // ---- T8: fail-open ---------------------------------------------------

    #[test]
    fn build_sink_returns_none_when_disabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvVarGuard::unset(AUDIT_ENABLE_ENV);
        assert!(build_sink().is_none());
    }

    #[test]
    fn resolve_audit_path_uses_explicit_override_verbatim() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvVarGuard::set(AUDIT_PATH_ENV, "/tmp/some-explicit-audit-path.jsonl");
        let resolved = resolve_audit_path();
        assert_eq!(
            resolved,
            Some(PathBuf::from("/tmp/some-explicit-audit-path.jsonl"))
        );
    }

    /// Builds a fresh, unique temp directory containing a plain FILE named
    /// `.buzz` (not a directory). Any attempt to `create_dir_all(".buzz/
    /// audit")` underneath it must fail deterministically on every OS — no
    /// chmod/permission trick required.
    fn make_home_with_blocked_dot_buzz() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "buzz-acp-audit-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&base).expect("create temp base dir");
        std::fs::write(base.join(".buzz"), b"not a directory").expect("write blocking file");
        base
    }

    #[test]
    fn resolve_audit_path_fails_open_when_home_dot_buzz_is_a_regular_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let base = make_home_with_blocked_dot_buzz();

        let _path_env = EnvVarGuard::unset(AUDIT_PATH_ENV);
        let _home_env = EnvVarGuard::set("HOME", base.to_str().expect("temp path is valid UTF-8"));
        let resolved = resolve_audit_path();

        let _ = std::fs::remove_dir_all(&base);
        assert!(
            resolved.is_none(),
            "resolve_audit_path must fail open (None) when $HOME/.buzz is a regular file"
        );
    }

    #[test]
    fn build_sink_fails_open_when_default_dir_blocked() {
        let _guard = ENV_LOCK.lock().unwrap();
        let base = make_home_with_blocked_dot_buzz();

        let _enable_env = EnvVarGuard::set(AUDIT_ENABLE_ENV, "1");
        let _path_env = EnvVarGuard::unset(AUDIT_PATH_ENV);
        let _home_env = EnvVarGuard::set("HOME", base.to_str().expect("temp path is valid UTF-8"));
        let sink = build_sink();

        let _ = std::fs::remove_dir_all(&base);
        assert!(
            sink.is_none(),
            "build_sink must fail open (None), never panic, when the default \
             audit directory cannot be created"
        );
    }
}
