//! ACP client module — manages communication with an AI agent subprocess over stdio
//! using JSON-RPC 2.0 (newline-delimited / NDJSON).
//!
//! # Lifecycle
//! 1. [`AcpClient::spawn`] — launch agent binary as subprocess
//! 2. [`AcpClient::initialize`] — protocol version negotiation
//! 3. [`AcpClient::session_new`] — create session with MCP server config
//! 4. [`AcpClient::session_prompt_with_idle_timeout`] — send prompt with idle/hard deadline, return stop reason
//! 5. [`AcpClient::session_cancel`] / [`AcpClient::cancel_with_cleanup`] — cancel in-flight turn

use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

use nostr::{EventBuilder, Keys, Kind, PublicKey, Tag};
use uuid::Uuid;

use crate::config::{PermissionMode, PermissionPolicy, ResolvedPermissionConfig};
use crate::observer::{AuthorizationEnvelope, ObserverContext, ObserverEvent, ObserverHandle};
use crate::relay::RelayEventPublisher;
use crate::usage::{
    PromptResponseUsage, StandardAdapterKind, StandardUsageTracker, TurnUsage, UsageTracker,
};
use buzz_core::observer::OBSERVER_MAX_PLAINTEXT_LEN;

/// Maximum allowed size of a single NDJSON line from the agent's stdout.
/// Lines exceeding this limit are rejected to prevent OOM from rogue agents.
const MAX_LINE_SIZE: usize = 10_000_000; // 10 MB

/// Maximum number of `session/request_permission` requests that may be
/// simultaneously pending under the `ask` policy. New requests beyond this
/// cap are denied immediately (fail closed) so the map remains bounded.
pub const PERMISSION_MAP_CAP: usize = 8;

/// Maximum number of options in a single `session/request_permission` request.
/// Requests with more options are denied immediately (admission preflight).
const PERMISSION_OPTIONS_MAX: usize = 16;

/// Per-request timeout under the `ask` policy. The desktop has at most this
/// long to deliver a `permission_decision` control frame before the harness
/// fails closed with the denial response.
const PERMISSION_ASK_TIMEOUT_SECS: u64 = 300;

/// Maximum time to wait for a relay `OK` after publishing the kind-9 sentinel
/// card. If the relay does not acknowledge within this window the request is
/// denied immediately (fail closed). The publish deadline is
/// `min(now + SENTINEL_PUBLISH_TIMEOUT_SECS, expiresAt)`.
pub(crate) const SENTINEL_PUBLISH_TIMEOUT_SECS: u64 = 10;

/// Delay between resolved kind-40003 edit retransmission attempts.
///
/// The permission decision is already irreversible by the time the resolved
/// edit publishes, so the edit must reach the relay to retire the UI card.
/// While the relay is disconnected an acked publish resolves as `Uncertain`
/// immediately; this backoff paces retransmission of the same signed event
/// across a reconnect instead of busy-looping. Bounded overall by the card's
/// expiry.
const RESOLVED_RETRANSMIT_BACKOFF: std::time::Duration = std::time::Duration::from_secs(2);

/// Delivery window for the resolved-edit retransmit loop, measured from the
/// instant the decision is resolved (ACP response written). Independent of the
/// original click/card deadline: an ordinary timeout resolves at expiry (when
/// the card deadline is already past), so using the click deadline as the retry
/// bound means the loop exits before the first attempt.
///
/// Aligned with the relay's 300 s card-maximum and the per-request
/// `PERMISSION_ASK_TIMEOUT_SECS` admission window. Using 300 s here means
/// the retransmit task can span the full TLS reconnect ladder (typically
/// ≤60 s) plus any additional relay backpressure, ensuring the resolved edit
/// always reaches the relay before the card naturally expires. The first
/// publish attempt is unconditional (deadline is future at spawn time); only
/// retries consult this bound.
const RESOLVED_DELIVERY_WINDOW_SECS: u64 = 300;

/// An MCP server configuration passed to `session/new`.
///
/// Corresponds to the `McpServerStdio` variant in the ACP schema.
/// All four fields are **required** by the schema (`args` and `env` may be empty arrays).
#[derive(Debug, Clone, serde::Serialize)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<EnvVar>,
}

/// A single environment variable for an MCP server.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// Stop reason returned by `session/prompt` when the agent finishes a turn.
///
/// Maps to the `stopReason` field in the `SessionPromptResponse`.
#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    /// Agent completed the turn normally (`"end_turn"`).
    EndTurn,
    /// Turn was cancelled via `session/cancel` (`"cancelled"`).
    Cancelled,
    /// Agent hit its token limit (`"max_tokens"`).
    MaxTokens,
    /// Agent hit its per-turn request limit (`"max_turn_requests"`).
    MaxTurnRequests,
    /// Agent refused the prompt (`"refusal"`).
    /// Note: refused turns are dropped from history by the agent.
    Refusal,
}

impl StopReason {
    /// Parse a `stopReason` string from the ACP wire format.
    ///
    /// Matching is case-insensitive so agents that send `"END_TURN"` or
    /// `"Cancelled"` are handled correctly without a protocol error.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "end_turn" => Some(Self::EndTurn),
            "cancelled" => Some(Self::Cancelled),
            "max_tokens" => Some(Self::MaxTokens),
            "max_turn_requests" => Some(Self::MaxTurnRequests),
            "refusal" => Some(Self::Refusal),
            _ => None,
        }
    }
}

/// Errors that can occur in the ACP client.
#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Agent process exited unexpectedly")]
    AgentExited,

    #[error("Idle timeout — no agent activity for {0:?}")]
    IdleTimeout(std::time::Duration),

    #[error("Hard turn timeout exceeded (silence {silence:?})")]
    HardTimeout { silence: std::time::Duration },

    #[error("Agent did not stop within {0:?} after cancellation")]
    CancelDrainTimeout(std::time::Duration),

    #[error("Request timeout — agent did not respond within {0:?}")]
    Timeout(std::time::Duration),

    #[error("Write timeout — agent stopped reading stdin (blocked for {0:?})")]
    WriteTimeout(std::time::Duration),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Agent reported error (code {code}): {message}")]
    AgentError { code: i64, message: String },

    /// A permission response write was interrupted mid-flight by a cancel.
    ///
    /// The process may have received the response bytes but may not have acted
    /// on them — state is irrecoverably uncertain. The agent process MUST be
    /// replaced (not returned to the pool) after this error. The cancel path
    /// surfaces this through `cancel_with_cleanup_grace` so
    /// `classify_control_cancel_failure` in `pool.rs` triggers respawn.
    #[error("Permission response write was interrupted — process state uncertain")]
    PermissionPoisoned,
}

/// Build an [`AcpError::AgentError`] from a JSON-RPC error object,
/// preserving the numeric code. When the `message` field is missing or
/// non-string, fall back to the full JSON object so provider-specific
/// detail (e.g. a `data` field) is not lost.
fn agent_error_from_json(error: &serde_json::Value) -> AcpError {
    let code = error.get("code").and_then(|c| c.as_i64()).unwrap_or(-32000);
    let message = match error.get("message").and_then(|m| m.as_str()) {
        Some(m) => m.to_string(),
        None => error.to_string(),
    };
    AcpError::AgentError { code, message }
}

fn build_initialize_params() -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": 2,
        "clientCapabilities": build_client_capabilities(),
        "clientInfo": {
            "name": "buzz-acp",
            "version": env!("CARGO_PKG_VERSION")
        },
    })
}

/// A decision delivered by the desktop via a `permission_decision` control frame.
#[derive(Debug, Clone)]
pub struct PermissionDecision {
    /// The nonce that was advertised in the `authorization` envelope of the
    /// `acp_read` frame for this request.
    pub request_nonce: String,
    /// The `optionId` the owner chose. Must exactly match one of the options in
    /// the original request.
    pub option_id: String,
}

/// Lifecycle state of a single `session/request_permission` request under
/// the `ask` policy.
#[derive(Debug, Clone)]
enum PermissionEntryState {
    /// Kind-9 sentinel published; waiting for relay `OK accepted=true`.
    /// An authorized early decision arriving in this state is buffered in
    /// `PermissionEntry::early_decision` and applied on admission.
    Publishing,
    /// Relay confirmed the sentinel (`OK accepted=true`). Waiting for an
    /// owner decision via the `permission_decision` control channel.
    Pending,
    /// A decision arrived; we are in the process of writing the response.
    /// Cancel during this state → `PermissionPoisoned`.
    Writing,
}

/// Per-request state tracked in `AcpClient::pending_permissions` under `ask`.
///
/// Entries are **removed** from the map on every terminal transition
/// (applied/timed_out/cancelled). The absence of a nonce from the map is the
/// replay guard — no `Resolved` tombstone is kept, so capacity measures only
/// live (Publishing, Pending, or Writing) requests.
#[derive(Debug)]
struct PermissionEntry {
    /// Nonce bound to this request — must match the desktop's decision.
    nonce: String,
    /// The exact options snapshot from the original request.
    options_snapshot: Vec<serde_json::Value>,
    /// The two validated card actions (allow_once / reject_once) surfaced for
    /// this request. The read loop accepts an owner decision only when its
    /// `optionId` matches one of these two — never a forbidden option (e.g.
    /// `allow_always`) that the adapter offered but the card never showed.
    card_actions: CardActions,
    /// Current lifecycle state.
    state: PermissionEntryState,
    /// Per-request hard deadline: `min(registered_at + 300s, turn hard deadline)`.
    /// Expiry → fail closed (denial + `timed_out` outcome).
    deadline: tokio::time::Instant,
    /// Unix timestamp of `expiresAt` included in both the pending and resolved
    /// sentinel payloads. Stored once at build time so the resolved edit reuses
    /// the exact same value (no recompute drift).
    expiry_unix_secs: u64,
    /// Event ID of the kind-9 sentinel card published into the thread.
    /// `None` while still in `Publishing` state (set on `Accepted`).
    /// The kind-40003 edit is skipped when this is `None`.
    sentinel_event_id: Option<String>,
    /// An authorized decision that arrived while the entry was still in
    /// `Publishing` state. Applied immediately on `Accepted`; discarded on
    /// any non-accepted outcome (entry is denied instead).
    early_decision: Option<PermissionDecision>,
    /// Human-readable description of the requested operation, extracted from
    /// the ACP `session/request_permission` message via `description_from_request_permission`.
    /// Tries `params.title`, `params.subject.toolCall.title`, `params.toolCall.title`,
    /// `params.toolCall.rawInput.command`, and `params._meta.codex.params.reason` in order.
    /// Truncated to `SENTINEL_STRING_MAX_BYTES`. `None` when no path yields a non-empty string.
    description: Option<String>,
}

/// ACP client that owns an agent subprocess and communicates over its stdio.
///
/// One `AcpClient` per agent process. Multiple sessions can be created on the
/// same client via repeated calls to [`session_new`](AcpClient::session_new).
pub struct AcpClient {
    /// The agent child process (kept alive to prevent zombie).
    child: Child,
    /// Write end of the agent's stdin pipe.
    stdin: ChildStdin,
    /// Framed reader over the agent's stdout pipe (line-oriented, bounded).
    /// Uses `LinesCodec::new_with_max_length` to enforce MAX_LINE_SIZE at the
    /// read level — prevents OOM from rogue agents writing infinite non-newline bytes.
    reader: FramedRead<ChildStdout, LinesCodec>,
    /// Monotonically increasing JSON-RPC request id counter.
    /// Harness-generated IDs are always numeric.
    next_id: u64,
    /// The id of a `session/request_permission` request that has been received
    /// but not yet responded to. Stored as `serde_json::Value` because JSON-RPC 2.0
    /// permits both numeric and string IDs from the agent.
    /// Used by [`cancel_with_cleanup`](AcpClient::cancel_with_cleanup) to send
    /// a `cancelled` outcome before the agent returns from `session/prompt`.
    ///
    /// Under `reject` and `allow` policies only one request can be in-flight
    /// (synchronous handling), so a single Option suffices.
    /// Under `ask` the full map is `pending_permissions` below.
    pending_permission_id: Option<serde_json::Value>,
    /// Whether we have already sent a response to the pending permission request.
    /// Guards against double-response if a timeout fires after the rejection
    /// response was written but before `pending_permission_id` was cleared.
    permission_responded: bool,
    /// Pending `session/request_permission` entries under the `ask` policy.
    ///
    /// Keyed by request id (as JSON Value). Bounded at `PERMISSION_MAP_CAP`.
    /// Entries transition: `Pending → Writing`. On any terminal outcome
    /// (applied/timed_out/cancelled) the entry is **removed** — the absence of
    /// a nonce is the replay guard. Capacity is live count only (no tombstones).
    /// Cleared at turn end as a safety net.
    pending_permissions: std::collections::HashMap<String, PermissionEntry>,
    /// Whether this process is poisoned due to a cancel-during-write.
    ///
    /// When `true` the process MUST NOT be returned to the pool — it must be
    /// respawned. The cancel path surfaces this via `PermissionPoisoned`.
    permission_poisoned: bool,
    /// Resolved permission configuration. Determines how `handle_permission_request`
    /// answers ACP `session/request_permission` frames.
    permission_config: ResolvedPermissionConfig,
    /// Whether an agent owner pubkey was resolved at startup.
    ///
    /// Used by the `ask` availability gate: `ask` without a known owner downgrades
    /// to `reject` (the desktop needs an owner to route the permission card to).
    owner_pubkey_known: bool,
    /// Channel for delivering `permission_decision` control frames from the
    /// observer dispatch loop into the read loop's decision arm.
    /// Installed by `install_permission_decision_rx`; consumed by the read loop.
    permission_decision_rx: Option<tokio::sync::mpsc::Receiver<PermissionDecision>>,
    /// Publisher for kind-9 sentinel cards and kind-40003 edits.
    /// Set via `set_relay_publisher`. When `None`, sentinel publishing is skipped
    /// (permission flow continues without a UI card).
    relay_publisher: Option<RelayEventPublisher>,
    /// Agent signing keys for building sentinel Nostr events.
    /// Set via `set_agent_relay_keys`. Must be set alongside `relay_publisher`.
    agent_relay_keys: Option<Keys>,
    /// Agent owner pubkey (hex). p-tagged on the kind-9 sentinel so the
    /// desktop routes the card to the correct viewer. Set via `set_agent_owner_pubkey_hex`.
    agent_owner_pubkey_hex: Option<String>,
    /// Pubkey of the first event in the current turn's batch.
    /// Used by the D7-final admission check: `ask` only proceeds for turns
    /// initiated by the agent owner. Set per-turn by `set_turn_initiator_pubkey`.
    turn_initiator_pubkey: Option<PublicKey>,
    /// Channel UUID for the `h` tag on the kind-9 sentinel.
    /// Set per-turn by `set_turn_channel_context`.
    sentinel_channel_id: Option<Uuid>,
    /// Event ID of the triggering turn event for the kind-9 sentinel reply tag.
    /// Set per-turn by `set_turn_channel_context`.
    sentinel_thread_reply_id: Option<String>,
    /// In-flight ACK receiver for the currently-publishing sentinel.
    ///
    /// Set by `handle_permission_request` when a kind-9 is sent via
    /// `register_publish_ack`. The read loop's select! arm polls this until
    /// the relay responds or the publish deadline fires. At most one entry can
    /// be in `Publishing` state at a time — the admission preflight denies a
    /// new Ask request while a publish is in flight, so this single slot is
    /// never overwritten with an unacknowledged receiver still live.
    ///
    /// A background task awaits the `oneshot::Receiver<AckOutcome>` and forwards
    /// the `(entry_id, outcome)` pair here via mpsc, decoupling the borrow from
    /// the read loop's `self` reference. The relay background task owns deadline
    /// enforcement — it sweeps expired waiters with `Uncertain`, so `ack_rx`
    /// always resolves before the deadline without any caller-side timeout.
    sentinel_ack_result_rx: Option<tokio::sync::mpsc::Receiver<(String, crate::relay::AckOutcome)>>,
    /// The JSON-RPC id of the most recently sent `session/prompt` request.
    /// Used by [`cancel_with_cleanup`] to drain the correct response.
    /// Set in [`session_prompt_with_idle_timeout`]; consumed in [`cancel_with_cleanup`].
    last_prompt_id: Option<u64>,
    /// Hard deadline for the current turn, set by `session_prompt_with_idle_timeout`.
    /// Inherited by `cancel_with_cleanup` so the drain loop shares the same budget
    /// rather than starting a fresh timer (prevents double-jeopardy).
    current_hard_deadline: Option<tokio::time::Instant>,
    /// Optional local observer feed used by the desktop app.
    observer: Option<ObserverHandle>,
    /// Pool slot index for this agent process.
    observer_agent_index: Option<usize>,
    /// Best-effort context attached to raw ACP wire events.
    observer_context: ObserverContext,
    /// Most recently observed `_meta.goose.activeRunId` from a
    /// `session/update` notification of kind `session_info_update`.
    ///
    /// Both goose and buzz-agent emit `session_info_update` with this field;
    /// goose emits it whenever it starts or clears an active prompt run
    /// (`crates/goose/src/acp/server.rs:2277` `send_active_run_update`).
    /// Required as `expectedRunId` when calling the non-standard
    /// `_goose/unstable/session/steer` method to inject a message into an
    /// in-flight turn without cancelling it.
    ///
    /// `None` until the first `session_info_update` arrives, or after the
    /// run clears (goose/buzz-agent emit `activeRunId: null` at end of turn).
    /// Other agents may leave this unset — readers must treat `None` as
    /// "no active run to steer into" and fall back to cancel+merge.
    active_run_id: Option<String>,
    /// Whether the agent advertised `_meta.steering.supported: true` in its
    /// `initialize` response, meaning it implements the cross-adapter
    /// [`ACP_STEER_METHOD`] extension.
    ///
    /// Set once by [`initialize`](Self::initialize); `false` for agents that
    /// omit the key. This is the **only** gate on writing an
    /// [`ACP_STEER_METHOD`] request. It must never be replaced by error-code
    /// probing: codex-acp answers unrecognized extension methods with `{}` —
    /// a JSON-RPC *success*, not `-32601` — which the main loop would read as
    /// a delivered steer and drop the user's message from the queue.
    steering_supported: bool,
    /// Per-turn channel for receiving goose-native non-cancelling steer
    /// requests from the main loop. Installed by
    /// [`install_steer_rx`](Self::install_steer_rx) at dispatch and
    /// consumed (via `take()`) by `session_prompt_with_idle_timeout` so it
    /// is dropped at scope exit alongside the turn it served. `None`
    /// outside of a goose-native turn — the read loop's steer arm is
    /// disabled in that case.
    steer_rx: Option<tokio::sync::mpsc::Receiver<crate::pool::SteerRequest>>,
    /// Usage tracker for goose/buzz-agent's cumulative notification format.
    goose_usage: UsageTracker,
    /// Test-only: count every write attempt (before the actual I/O). Incremented
    /// at the top of `write_ndjson_inner` so callers can assert "exactly N attempts"
    /// independently of whether the writes succeeded.
    #[cfg(test)]
    write_attempt_count: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
    /// Per-turn prompt-response usage and Claude's optional cumulative cost.
    standard_usage: StandardUsageTracker,
    /// Known adapter identity for prompt-response usage mapping.
    standard_adapter: Option<StandardAdapterKind>,
}

/// Recursively merge `overlay` into `base`, with `overlay` winning on scalar/shape
/// collisions.  When both sides have an object for the same key, the merge recurses so
/// unrelated nested keys from `base` are preserved.
fn deep_merge(
    base: &mut serde_json::Map<String, serde_json::Value>,
    overlay: serde_json::Map<String, serde_json::Value>,
) {
    for (k, overlay_val) in overlay {
        match base.get_mut(&k) {
            Some(serde_json::Value::Object(base_obj))
                if matches!(overlay_val, serde_json::Value::Object(_)) =>
            {
                // Both sides are objects — recurse to preserve unrelated nested keys.
                if let serde_json::Value::Object(overlay_obj) = overlay_val {
                    deep_merge(base_obj, overlay_obj);
                }
            }
            _ => {
                // Scalar, array, type mismatch, or new key — overlay wins.
                base.insert(k, overlay_val);
            }
        }
    }
}

/// Build the merged `CODEX_CONFIG` environment-variable value for a Codex agent spawn.
///
/// Returns `Some(json_string)` when `has_generated_codex_config` is true (Buzz injected a
/// `CODEX_CONFIG` entry via `codex_network_env()`), `None` otherwise.
///
/// # Merge contract (when `has_generated_codex_config` is true)
///
/// 1. **Persona base** — the first `CODEX_CONFIG` value in `extra_env` is taken as
///    the base object (all keys preserved, recursively).  When there is no persona entry,
///    the generated entry serves as the base.
/// 2. **Generated overlay** — all subsequent `CODEX_CONFIG` entries are deep-merged into
///    the base so unrelated nested persona keys survive.
/// 3. **Parent-env precedence** — if `parent_codex_config` is `Some`, its keys are
///    deep-merged into the result (parent wins on colliding keys at every nesting level;
///    unrelated keys from either side survive).
/// 4. **Forced overlay** — `sandbox_workspace_write.network_access = true` is applied
///    last so relay access is guaranteed regardless of operator / persona config.
///
/// When `has_generated_codex_config` is false, the function returns `None` and the
/// caller handles any persona-supplied `CODEX_CONFIG` with ordinary operator-wins
/// semantics (no merging, no sandbox widening).
///
/// # Errors
///
/// Returns `Err(AcpError::Protocol)` when `has_generated_codex_config` is true and any
/// `CODEX_CONFIG` value is not valid JSON or is not a JSON object, or when
/// `sandbox_workspace_write` is present but not an object after all merges.
pub(crate) fn build_codex_config_env(
    extra_env: &[(String, String)],
    parent_codex_config: Option<&str>,
    has_generated_codex_config: bool,
) -> Result<Option<String>, AcpError> {
    // Without an explicit Buzz-generated overlay signal, skip the merge entirely.
    // Any persona CODEX_CONFIG is handled by the caller with operator-wins semantics.
    if !has_generated_codex_config {
        return Ok(None);
    }

    // Collect all CODEX_CONFIG entries from extra_env in order.
    let codex_entries: Vec<&str> = extra_env
        .iter()
        .filter(|(k, _)| k == "CODEX_CONFIG")
        .map(|(_, v)| v.as_str())
        .collect();

    if codex_entries.is_empty() {
        // has_generated_codex_config is true but no entry in extra_env — shouldn't
        // happen in practice, but treat as no-op rather than panic.
        return Ok(None);
    }

    // Parse all entries; first one is the persona base (or the generated entry if no
    // persona CODEX_CONFIG was set), rest are additional generated entries.
    let mut parsed_entries: Vec<serde_json::Map<String, serde_json::Value>> = Vec::new();
    for (i, raw) in codex_entries.iter().enumerate() {
        match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(serde_json::Value::Object(obj)) => parsed_entries.push(obj),
            Ok(_) => {
                let source = if i == 0 { "persona" } else { "generated" };
                return Err(AcpError::Protocol(format!(
                    "CODEX_CONFIG {source} value is valid JSON but not an object"
                )));
            }
            Err(e) => {
                let source = if i == 0 { "persona" } else { "generated" };
                return Err(AcpError::Protocol(format!(
                    "CODEX_CONFIG {source} value is not valid JSON: {e}"
                )));
            }
        }
    }

    // Start from first entry, deep-merge remaining entries.
    let mut base = parsed_entries.remove(0);
    for overlay in parsed_entries {
        deep_merge(&mut base, overlay);
    }

    // Deep-merge parent env (parent wins on colliding keys at every nesting level).
    if let Some(parent_raw) = parent_codex_config {
        match serde_json::from_str::<serde_json::Value>(parent_raw) {
            Ok(serde_json::Value::Object(parent_obj)) => {
                deep_merge(&mut base, parent_obj);
            }
            Ok(_) => {
                return Err(AcpError::Protocol(
                    "CODEX_CONFIG in parent environment is valid JSON but not an object".into(),
                ));
            }
            Err(e) => {
                return Err(AcpError::Protocol(format!(
                    "CODEX_CONFIG in parent environment is not valid JSON: {e}"
                )));
            }
        }
    }

    // Force sandbox_workspace_write.network_access = true (our invariant, always wins).
    let sws_entry = base
        .entry("sandbox_workspace_write")
        .or_insert_with(|| serde_json::json!({}));
    match sws_entry {
        serde_json::Value::Object(sws_obj) => {
            sws_obj.insert("network_access".to_string(), serde_json::Value::Bool(true));
        }
        other => {
            return Err(AcpError::Protocol(format!(
                "CODEX_CONFIG sandbox_workspace_write is not an object (got {}); \
                 cannot set network_access=true",
                other
            )));
        }
    }

    Ok(Some(serde_json::Value::Object(base).to_string()))
}

/// goose's non-standard mid-turn steer method. Requires `expectedRunId`, so it
/// is only usable once a `session_info_update` has supplied
/// `_meta.goose.activeRunId`. Emitted by goose and buzz-agent only.
const GOOSE_STEER_METHOD: &str = "_goose/unstable/session/steer";

/// The cross-adapter mid-turn steer method, shipped by claude-agent-acp
/// (`src/acp-agent.ts:200`) and codex-acp (`src/AcpExtensions.ts:11`).
/// Params are `{sessionId, prompt}` — no run id — and the result is
/// `{outcome}`. Gated on [`AcpClient::steering_supported`].
const ACP_STEER_METHOD: &str = "_session/steering";

/// `outcome` value meaning the steer was applied to the turn Buzz is waiting
/// on, which therefore keeps running.
const STEER_OUTCOME_INJECTED: &str = "injected";

/// `outcome` value meaning the turn Buzz was steering had already finished, so
/// the adapter began a fresh turn carrying the message. Still a delivery
/// success, but the awaited turn is over — see the steer-response arm for why
/// this must not renew the hard deadline.
const STEER_OUTCOME_STARTED_NEW_TURN: &str = "startedNewTurn";

/// Which wire method carried an in-flight steer request, recorded so the
/// response arm decodes the shape that method actually returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SteerTransport {
    /// [`GOOSE_STEER_METHOD`] — any success result is a delivered steer.
    Goose,
    /// [`ACP_STEER_METHOD`] — success carries an `outcome` that must be
    /// positively recognized before the steer counts as delivered.
    AcpExtension,
}

fn build_client_capabilities() -> serde_json::Value {
    serde_json::json!({
        // Signal to ACP adapters that Buzz can hand users to terminal-native
        // auth flows. Adapters decide which auth methods to expose; Buzz does
        // not hardcode vendor login commands from this capability.
        "auth": {
            "terminal": true
        },
        // Signal to goose that we handle `_goose/unstable/session/update`
        // notifications. Without this the custom notification is suppressed
        // on goose's side and usage data is never emitted.
        "_meta": {
            "goose": {
                "customNotifications": true
            },
            // Non-standard extension used by claude-agent-acp to advertise the
            // exact terminal login argv for subscription auth. Unknown `_meta`
            // keys are ignored by other adapters.
            "terminal-auth": true
        }
    })
}

impl AcpClient {
    /// Kill the agent subprocess and wait for it to exit (no zombies).
    ///
    /// `Drop` only calls `start_kill()` (sends SIGKILL but doesn't reap).
    /// Call this when you need guaranteed cleanup — e.g., in `run_models`
    /// before process exit.
    pub async fn shutdown(&mut self) {
        // Kill the entire process group when possible. The child was spawned
        // with process_group(0), so its PID == its PGID. Killing the group
        // ensures subprocesses (MCP servers, tool processes) are cleaned up
        // rather than orphaned to init.
        //
        // Falls back to start_kill() (direct child only) on non-Unix or if
        // the child has been polled to completion (id() returns None).
        match self.child.id() {
            Some(pid) if kill_process_group(pid) => {}
            _ => {
                let _ = self.child.start_kill();
            }
        }
        // Bounded wait: if the child doesn't exit within 5s after SIGKILL,
        // give up and let Drop/OS handle it. An unbounded wait here would
        // wedge the harness during respawn or shutdown if a child is stuck.
        match tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::debug!("child wait error after kill: {e}"),
            Err(_) => tracing::warn!("child did not exit within 5s after SIGKILL — abandoning"),
        }
    }

    /// Spawn the agent binary as a subprocess and connect to its stdio pipes.
    ///
    /// `has_generated_codex_config` must be true when `codex_network_env()` successfully
    /// injected a `CODEX_CONFIG` entry into `extra_env`.  The spawn path uses it to
    /// trigger the recursive merge + forced `network_access=true` in
    /// `build_codex_config_env`.  Pass `false` for test spawns and non-Codex agents.
    ///
    /// After spawning, call [`initialize`](Self::initialize) before any other method.
    pub async fn spawn(
        command: &str,
        args: &[String],
        extra_env: &[(String, String)],
        has_generated_codex_config: bool,
    ) -> Result<Self, AcpError> {
        use std::process::Stdio;

        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Inherit stderr so agent logs are visible in the harness terminal.
            .stderr(Stdio::inherit())
            // Ensure the child is killed when the AcpClient is dropped (best-effort).
            // Callers MUST still call shutdown().await for guaranteed cleanup.
            .kill_on_drop(true);

        // Per-persona env vars (e.g., GOOSE_PROVIDER, BUZZ_AGENT_PROVIDER).
        // For most keys, operator precedence wins: skip injection if already set
        // in the parent environment.
        //
        // CODEX_CONFIG is handled specially via build_codex_config_env:
        //   • has_generated_codex_config=true: merge all CODEX_CONFIG entries + parent
        //     recursively and force network_access=true.
        //   • has_generated_codex_config=false: return None; any persona-supplied
        //     CODEX_CONFIG falls through to the normal operator-wins loop below.
        let has_codex_config = extra_env.iter().any(|(k, _)| k == "CODEX_CONFIG");
        let parent_codex_config = if has_generated_codex_config && has_codex_config {
            std::env::var("CODEX_CONFIG").ok()
        } else {
            None
        };
        let codex_config_value = build_codex_config_env(
            extra_env,
            parent_codex_config.as_deref(),
            has_generated_codex_config,
        )?;
        // When the merge path was not taken (None returned), any persona CODEX_CONFIG
        // entry falls through to the standard operator-wins treatment below.
        let codex_merge_active = codex_config_value.is_some();

        // Per-runtime environment defaults (e.g. Hermes MCP-startup isolation).
        // Applied first so both persona `extra_env` (below, via `Command::env`
        // key replacement) and inherited parent env (via the parent-presence
        // check) override them.
        for &(key, value) in crate::config::default_agent_env(command) {
            if std::env::var_os(key).is_none() {
                cmd.env(key, value);
            }
        }

        for (key, value) in extra_env {
            if key == "CODEX_CONFIG" && codex_merge_active {
                // Handled by build_codex_config_env; skip here to avoid double-setting.
                continue;
            }
            if std::env::var_os(key).is_none() {
                cmd.env(key, value);
            }
        }
        if let Some(merged) = codex_config_value {
            cmd.env("CODEX_CONFIG", merged);
        }

        // Spawn the agent in its own process group so SIGKILL doesn't propagate
        // to the harness's own process group on Unix.
        // tokio::process::Command::process_group is a stable tokio API (no extra imports needed).
        #[cfg(unix)]
        cmd.process_group(0);

        // Suppress the console window that Windows otherwise allocates for every
        // console-subsystem child process spawned from a GUI/non-console parent.
        configure_no_window(&mut cmd);

        let standard_adapter =
            match crate::config::normalize_agent_command_identity(command).as_str() {
                "claude-agent-acp" | "claude-code-acp" | "claude-code" | "claudecode" => {
                    Some(StandardAdapterKind::Claude)
                }
                "codex" | "codex-acp" => Some(StandardAdapterKind::Codex),
                _ => None,
            };
        let mut child = cmd.spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::Protocol("failed to open agent stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::Protocol("failed to open agent stdout".into()))?;

        Ok(Self {
            child,
            stdin,
            reader: FramedRead::new(stdout, LinesCodec::new_with_max_length(MAX_LINE_SIZE)),
            next_id: 0,
            pending_permission_id: None,
            permission_responded: false,
            pending_permissions: std::collections::HashMap::new(),
            permission_poisoned: false,
            permission_config: ResolvedPermissionConfig {
                policy: crate::config::PermissionPolicy::Reject,
                effective_mode: PermissionMode::DontAsk,
                mode_source: crate::config::ModeSource::Derived,
                transmit_mode: true,
            },
            owner_pubkey_known: false,
            permission_decision_rx: None,
            relay_publisher: None,
            agent_relay_keys: None,
            agent_owner_pubkey_hex: None,
            turn_initiator_pubkey: None,
            sentinel_channel_id: None,
            sentinel_thread_reply_id: None,
            sentinel_ack_result_rx: None,
            last_prompt_id: None,
            current_hard_deadline: None,
            observer: None,
            observer_agent_index: None,
            observer_context: ObserverContext::default(),
            active_run_id: None,
            steering_supported: false,
            steer_rx: None,
            goose_usage: UsageTracker::default(),
            #[cfg(test)]
            write_attempt_count: None,
            standard_usage: StandardUsageTracker::default(),
            standard_adapter,
        })
    }

    /// Attach a local observer feed to this ACP client.
    pub fn set_observer(&mut self, observer: Option<ObserverHandle>, agent_index: usize) {
        self.observer = observer;
        self.observer_agent_index = Some(agent_index);
    }

    /// Set the resolved permission configuration for this agent process.
    ///
    /// Called once after spawn (like `set_observer`) by `pool_lifecycle`.
    pub fn set_permission_config(&mut self, config: ResolvedPermissionConfig) {
        self.permission_config = config;
    }

    /// Record whether the agent owner pubkey is known at startup.
    ///
    /// The `ask` availability gate downgrades to `reject` when the owner is
    /// unknown — the desktop needs an owner to route the permission card.
    pub fn set_owner_pubkey_known(&mut self, known: bool) {
        self.owner_pubkey_known = known;
    }

    /// Install the per-session `permission_decision` receiver.
    ///
    /// The matching `Sender` is held by `handle_observer_control` in `lib.rs`
    /// and delivers `permission_decision` control frames into the read loop's
    /// decision arm. Idempotent — replaces any previously installed receiver.
    pub fn install_permission_decision_rx(
        &mut self,
        rx: tokio::sync::mpsc::Receiver<PermissionDecision>,
    ) {
        self.permission_decision_rx = Some(rx);
    }

    /// Install the relay publisher and agent signing keys for sentinel card publishing.
    ///
    /// Both must be set together. When either is absent, sentinel publishing is
    /// skipped; the permission flow continues without a UI card.
    pub fn set_relay_publisher(&mut self, publisher: RelayEventPublisher, keys: Keys) {
        self.relay_publisher = Some(publisher);
        self.agent_relay_keys = Some(keys);
    }

    /// Set the agent owner pubkey hex for the sentinel p-tag.
    pub fn set_agent_owner_pubkey_hex(&mut self, hex: Option<String>) {
        self.agent_owner_pubkey_hex = hex;
    }

    /// Set the turn initiator pubkey for the D7-final admission check.
    ///
    /// Must be called at the start of each turn (before `session_prompt_with_idle_timeout`).
    /// The `ask` policy rejects requests for turns NOT initiated by the agent owner.
    pub fn set_turn_initiator_pubkey(&mut self, pubkey: Option<PublicKey>) {
        self.turn_initiator_pubkey = pubkey;
    }

    /// Set the per-turn channel context for sentinel card routing.
    ///
    /// `channel_id` — the `h` tag on the kind-9.
    /// `thread_reply_event_id` — the `e` reply tag (triggering turn event).
    pub fn set_turn_channel_context(
        &mut self,
        channel_id: Option<Uuid>,
        thread_reply_event_id: Option<String>,
    ) {
        self.sentinel_channel_id = channel_id;
        self.sentinel_thread_reply_id = thread_reply_event_id;
    }

    /// Update metadata that will be attached to subsequent raw wire events.
    pub fn set_observer_context(&mut self, context: ObserverContext) {
        self.observer_context = context;
    }

    /// Install a write-attempt counter for tests.
    ///
    /// When set, every call to `write_ndjson_inner` (regardless of success or failure)
    /// atomically increments the counter before attempting the I/O. Tests can use this
    /// to assert "exactly one attempt was made" even when the write fails.
    #[cfg(test)]
    pub fn set_write_attempt_count(
        &mut self,
        counter: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) {
        self.write_attempt_count = Some(counter);
    }

    /// Return a clone of the observer handle, if attached.
    pub(crate) fn observer_handle(&self) -> Option<ObserverHandle> {
        self.observer.clone()
    }

    /// Return the pool slot index for this agent process.
    pub(crate) fn observer_agent_index(&self) -> Option<usize> {
        self.observer_agent_index
    }

    /// Emit a semantic event to the local observer feed, if enabled.
    pub fn observe(&self, kind: impl Into<String>, payload: serde_json::Value) {
        if let Some(observer) = &self.observer {
            observer.emit(
                kind,
                self.observer_agent_index,
                &self.observer_context,
                payload,
            );
        }
    }

    /// Emit a semantic event with an authorization envelope, if observer enabled.
    fn observe_authorized(
        &self,
        kind: impl Into<String>,
        authorization: AuthorizationEnvelope,
        payload: serde_json::Value,
    ) {
        if let Some(observer) = &self.observer {
            observer.emit_authorized(
                kind,
                self.observer_agent_index,
                &self.observer_context,
                authorization,
                payload,
            );
        }
    }

    /// Send the `initialize` request and return the agent's response result value.
    ///
    /// Must be called exactly once, before any other ACP method.
    /// The caller may inspect `agentCapabilities` in the returned value.
    ///
    /// Records `_meta.steering.supported` into
    /// [`steering_supported`](Self::steering_supported) so the read loop's steer
    /// arm can choose [`ACP_STEER_METHOD`] for adapters that implement it.
    /// Parsed here rather than at each call site so no caller can forget it.
    pub async fn initialize(&mut self) -> Result<serde_json::Value, AcpError> {
        // Requesting version 2 is an intentional temporary pin — we are squatting
        // on ACP v2 ahead of the upstream ACP RFD. Revisit when that RFD merges.
        let params = build_initialize_params();
        let result = self.send_request("initialize", params).await?;
        self.steering_supported = result
            .pointer("/_meta/steering/supported")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        tracing::debug!(target: "acp::init", "initialize response: {result}");
        Ok(result)
    }

    /// Send the ACP `authenticate` request for an adapter-advertised method.
    pub async fn authenticate(&mut self, method_id: &str) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            "methodId": method_id,
        });
        self.send_request("authenticate", params).await
    }

    /// Send `session/new` and return the full response alongside the session ID.
    ///
    /// `cwd` must be an absolute path. `mcp_servers` may be empty.
    ///
    /// `system_prompt` controls how the prompt text is delivered:
    ///
    /// - `None` — no system-prompt field in the request (legacy framing).
    /// - `Some(SystemPromptTransport::Field(text))` — bare `systemPrompt` field
    ///   (ACP protocol v2, buzz-agent, goose unused).
    /// - `Some(SystemPromptTransport::ClaudeMeta(text))` — `_meta.systemPrompt`
    ///   as `{"append": text}`, keeping claude-agent-acp's native preset intact.
    ///
    /// `session_title` rides in `_meta.sessionTitle` when `Some`; `_meta` is
    /// omitted entirely otherwise, since adapters may distinguish an absent
    /// member from a null one. When both `ClaudeMeta` and `session_title` are
    /// present the two `_meta` members are merged into a single object.
    ///
    /// Callers use [`extract_model_config_options`] and [`extract_model_state`]
    /// to pull model info from the raw result.
    pub async fn session_new_full(
        &mut self,
        cwd: &str,
        mcp_servers: Vec<McpServer>,
        system_prompt: Option<SystemPromptTransport<'_>>,
        session_title: Option<&str>,
    ) -> Result<SessionNewResponse, AcpError> {
        let mut params = serde_json::json!({
            "cwd": cwd,
            "mcpServers": mcp_servers,
        });
        match system_prompt {
            Some(SystemPromptTransport::Field(sp)) => {
                params["systemPrompt"] = serde_json::Value::String(sp.to_owned());
            }
            Some(SystemPromptTransport::ClaudeMeta(sp)) => {
                // Merge into _meta so sessionTitle (set below) is not clobbered.
                params["_meta"]["systemPrompt"] = serde_json::json!({ "append": sp });
            }
            None => {}
        }
        if let Some(title) = session_title {
            // Merge — _meta may already carry systemPrompt from ClaudeMeta above.
            params["_meta"]["sessionTitle"] = serde_json::Value::String(title.to_owned());
        }
        let result = self.send_request("session/new", params).await?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("session/new response missing sessionId".into()))?
            .to_owned();
        tracing::info!(target: "acp::session", "session created: {session_id}");
        Ok(SessionNewResponse {
            session_id,
            raw: result,
        })
    }

    /// Send `session/new` and return only the `sessionId` string.
    ///
    /// Convenience wrapper around [`session_new_full`].
    #[allow(dead_code)] // Public API — callers outside the harness may use this.
    pub async fn session_new(
        &mut self,
        cwd: &str,
        mcp_servers: Vec<McpServer>,
        system_prompt: Option<SystemPromptTransport<'_>>,
        session_title: Option<&str>,
    ) -> Result<String, AcpError> {
        Ok(self
            .session_new_full(cwd, mcp_servers, system_prompt, session_title)
            .await?
            .session_id)
    }

    /// Replace Goose's native system prompt after `session/new`.
    pub async fn session_set_goose_system_prompt(
        &mut self,
        session_id: &str,
        text: &str,
    ) -> Result<serde_json::Value, AcpError> {
        self.send_request(
            "_goose/unstable/session/system-prompt/set",
            serde_json::json!({
                "sessionId": session_id,
                "mode": "set",
                "key": "buzz",
                "text": text,
            }),
        )
        .await
    }

    /// Send `session/set_config_option` (stable ACP path).
    pub async fn session_set_config_option(
        &mut self,
        session_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
            "configId": config_id,
            "value": value,
        });
        self.send_request("session/set_config_option", params).await
    }

    /// Send `session/set_model` (unstable ACP path).
    pub async fn session_set_model(
        &mut self,
        session_id: &str,
        model_id: &str,
    ) -> Result<serde_json::Value, AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
            "modelId": model_id,
        });
        self.send_request("session/set_model", params).await
    }

    /// Send `session/prompt` with idle-based timeout instead of wall-clock.
    ///
    /// The idle deadline resets on any stdout activity from the agent. The hard
    /// deadline is an absolute wall-clock cap (safety valve).
    pub async fn session_prompt_with_idle_timeout(
        &mut self,
        session_id: &str,
        prompt_text: &str,
        idle_timeout: std::time::Duration,
        max_duration: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        self.session_prompt_blocks_with_idle_timeout(
            session_id,
            std::slice::from_ref(&prompt_text),
            idle_timeout,
            max_duration,
        )
        .await
    }

    /// Like [`session_prompt_with_idle_timeout`](Self::session_prompt_with_idle_timeout),
    /// but sends each entry in `prompt_blocks` as a separate text content block.
    ///
    /// Used for slash-command pass-through: ACP connectors detect commands via
    /// the **first** block's text starting with `/`, so the harness sends
    /// `["/cmd args", "<buzz context>"]` instead of one wrapped block.
    pub async fn session_prompt_blocks_with_idle_timeout(
        &mut self,
        session_id: &str,
        prompt_blocks: &[&str],
        idle_timeout: std::time::Duration,
        max_duration: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        let params = build_prompt_params(session_id, prompt_blocks);
        let hard_deadline = tokio::time::Instant::now() + max_duration;
        self.current_hard_deadline = Some(hard_deadline);

        // Mark the usage tracker as in-flight for this turn BEFORE sending the
        // prompt so that any setup notifications recorded earlier are not
        // misattributed to this turn.
        self.goose_usage.begin_turn(session_id);
        self.standard_usage.begin_turn(session_id);

        self.last_prompt_id = Some(self.next_id);
        let id = self.next_id;
        self.next_id += 1;

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/prompt",
            "params": params,
        });

        tracing::debug!(target: "acp::wire", "→ {}", &serde_json::to_string(&msg).unwrap_or_default());
        if let Err(e) = self.write_ndjson(&msg).await {
            self.last_prompt_id = None;
            self.current_hard_deadline = None;
            return Err(e);
        }

        let result = self
            .read_until_response_with_idle_timeout(
                session_id,
                id,
                idle_timeout,
                hard_deadline,
                max_duration,
            )
            .await;

        // On timeout errors, leave current_hard_deadline set so cancel_with_cleanup
        // can inherit the remaining budget. Clear it on all other outcomes.
        match &result {
            Ok(_) => {
                self.last_prompt_id = None;
                self.current_hard_deadline = None;
                // Turn completed normally — drain resolved/expired permission entries.
                // Pending entries are unexpected here (should be Resolved or expired),
                // but drain unconditionally to guarantee the map never leaks across turns.
                self.pending_permissions.clear();
            }
            Err(AcpError::IdleTimeout(_) | AcpError::HardTimeout { .. }) => {
                // Leave last_prompt_id and current_hard_deadline set —
                // caller will invoke cancel_with_cleanup.
            }
            Err(_) => {
                self.last_prompt_id = None;
                self.current_hard_deadline = None;
                // Non-recoverable error — drain the map to prevent capacity leak
                // if the pool reuses this process (poisoned processes are respawned,
                // but clean error exits may be returned to the pool).
                self.pending_permissions.clear();
            }
        }
        self.parse_prompt_response(session_id, &result?)
    }

    /// Send a `session/cancel` **notification** (no `id` field, no response expected).
    ///
    /// After calling this, the agent will eventually respond to the in-flight
    /// `session/prompt` with `stopReason: "cancelled"`. Use
    /// [`cancel_with_cleanup`](Self::cancel_with_cleanup) if you need to drain
    /// that response.
    ///
    /// Note: async because writing to stdin requires async I/O.
    pub async fn session_cancel(&mut self, session_id: &str) -> Result<(), AcpError> {
        let params = serde_json::json!({
            "sessionId": session_id,
        });
        self.send_notification("session/cancel", params).await
    }

    /// Returns `true` if a `session/prompt` request is currently in flight.
    pub fn has_in_flight_prompt(&self) -> bool {
        self.last_prompt_id.is_some()
    }

    /// Most recently observed goose `_meta.goose.activeRunId` from a
    /// `session_info_update`, if any.
    ///
    /// Both goose and buzz-agent emit `session_info_update`; other agents
    /// leave this `None` for the lifetime of the client. Read directly by
    /// `read_until_response_with_idle_timeout`'s
    /// steer arm at write time (see [`crate::pool::SteerRequest`] for
    /// why the read loop owns this); production callers do not need this
    /// accessor. Kept as `pub` so tests can introspect the field.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn active_run_id(&self) -> Option<&str> {
        self.active_run_id.as_deref()
    }

    /// Whether the agent advertised the [`ACP_STEER_METHOD`] extension at
    /// `initialize` time (`_meta.steering.supported`).
    ///
    /// The read loop's steer arm reads the field directly; this accessor exists
    /// for the supervisor's post-initialize log line.
    pub fn steering_supported(&self) -> bool {
        self.steering_supported
    }

    /// Consume per-turn usage for NIP-AM publishing. Goose/buzz-agent is an
    /// exclusive cumulative path; standard ACP prompt usage is used only when
    /// goose emitted nothing for this turn.
    pub fn take_turn_usage(&mut self) -> Option<TurnUsage> {
        let goose_usage = self.goose_usage.take();
        let standard_usage = self.standard_usage.take();
        goose_usage.or(standard_usage)
    }

    /// Notify the usage tracker that buzz-acp just spawned a new session.
    ///
    /// Seeds a zero baseline so the first usage notification for `session_id`
    /// produces `delta_reliable: true` (turn delta == cumulative from zero).
    /// Must be called only when buzz-acp created the session via `session/new`;
    /// never when attaching to a pre-existing session.
    pub(crate) fn notify_session_spawned(&mut self, session_id: &str) {
        self.goose_usage.seed_zero_baseline(session_id);
        self.standard_usage.seed_zero_baseline(session_id);
    }

    /// Install a per-turn steer request channel for goose-native
    /// non-cancelling mid-turn delivery.
    ///
    /// Called by the dispatch path immediately before
    /// [`session_prompt_with_idle_timeout`] for all prompt tasks.
    /// The matching `Sender` is stored in `TaskMeta.steer_tx` for the
    /// main loop's mode-gate fork to drive.
    ///
    /// Panics if a receiver is already installed — there is exactly one
    /// turn per `AcpClient` at a time, and stacking receivers would
    /// silently misroute steer requests across turns. The previous
    /// turn's receiver must have been consumed by the read loop and
    /// dropped at scope exit before the next turn dispatches.
    pub fn install_steer_rx(&mut self, rx: tokio::sync::mpsc::Receiver<crate::pool::SteerRequest>) {
        assert!(
            self.steer_rx.is_none(),
            "install_steer_rx: previous turn's receiver was not consumed — \
             stacking receivers would misroute steer requests across turns"
        );
        self.steer_rx = Some(rx);
    }

    /// Clear any installed steer receiver without consuming it.
    ///
    /// Called by `send_prompt_result` on every exit path of `run_prompt_task`
    /// so that `install_steer_rx`'s `is_none()` invariant holds for the next
    /// dispatch even when the turn ended before the read loop ran `take()`.
    /// Idempotent — safe to call when `steer_rx` is already `None`.
    pub fn clear_steer_rx(&mut self) {
        self.steer_rx = None;
    }

    /// Returns `true` if no steer receiver is currently installed.
    ///
    /// Test-only: used by `pool` tests to assert the post-return invariant
    /// without exposing the private field directly.
    #[cfg(test)]
    pub fn steer_rx_is_none(&self) -> bool {
        self.steer_rx.is_none()
    }

    /// Cancel a turn cleanly, handling any pending permission request first.
    ///
    /// Steps:
    /// 1. If there is a pending `session/request_permission` that hasn't been
    ///    responded to yet, respond with `outcome: "cancelled"`.
    /// 2. Send `session/cancel` notification (no id).
    /// 3. Continue reading until the `session/prompt` response arrives with `stopReason: "cancelled"`.
    ///
    /// Returns the final [`StopReason`] (almost always [`StopReason::Cancelled`]).
    pub async fn cancel_with_cleanup(
        &mut self,
        session_id: &str,
        _idle_timeout: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        // Inherit the hard deadline from the timed-out turn so the drain loop
        // doesn't start a fresh timer (prevents double-jeopardy). If the original
        // deadline is already expired or near-expired, grant a 30s floor so the
        // cancel notification has time to propagate and the agent can respond.
        let stored_deadline = self.current_hard_deadline.take();
        let min_cleanup_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let hard_deadline = match stored_deadline {
            Some(d) if d > min_cleanup_deadline => d,
            Some(_) => {
                tracing::debug!(
                    "original hard deadline expired or near-expired — using 30s cleanup grace"
                );
                min_cleanup_deadline
            }
            None => {
                tracing::warn!(
                    "cancel_with_cleanup called without current_hard_deadline — using 30s fallback"
                );
                min_cleanup_deadline
            }
        };

        self.cancel_with_cleanup_until(session_id, hard_deadline)
            .await
    }

    /// Cancel a user-interrupted turn with a bounded grace window.
    ///
    /// Some ACP servers currently keep streaming after `session/cancel`. For an
    /// explicit Stop button, waiting until the original turn deadline can make
    /// cancellation look broken. This variant gives the agent a short chance to
    /// acknowledge cancellation, then returns a timeout so the caller can respawn
    /// the agent process and actually stop the work.
    ///
    /// The `grace` window is a cleanup deadline, not the turn's real max-turn
    /// wall clock — a bounded drain that expires maps to
    /// [`AcpError::CancelDrainTimeout`], never [`AcpError::HardTimeout`], so
    /// callers can distinguish "agent didn't stop in time" from a genuine
    /// configured hard-cap breach.
    pub async fn cancel_with_cleanup_grace(
        &mut self,
        session_id: &str,
        grace: std::time::Duration,
    ) -> Result<StopReason, AcpError> {
        let _ = self.current_hard_deadline.take();
        let hard_deadline = tokio::time::Instant::now() + grace;
        match self
            .cancel_with_cleanup_until(session_id, hard_deadline)
            .await
        {
            Err(AcpError::HardTimeout { .. }) => Err(AcpError::CancelDrainTimeout(grace)),
            other => other,
        }
    }

    async fn cancel_with_cleanup_until(
        &mut self,
        session_id: &str,
        hard_deadline: tokio::time::Instant,
    ) -> Result<StopReason, AcpError> {
        // Validate precondition before any side effects — fail fast if there's
        // no in-flight prompt (prevents writing permission responses or cancel
        // notifications to the agent when no prompt is active).
        let prompt_id = self.last_prompt_id.take().ok_or_else(|| {
            AcpError::Protocol("cancel_with_cleanup called with no in-flight prompt".into())
        })?;

        // Check for poisoning first: if a permission write is in progress we
        // must not send any more bytes to this process — return the dedicated
        // error so `classify_control_cancel_failure` triggers respawn.
        if self.permission_poisoned {
            tracing::error!(
                target: "acp::cancel",
                "cancel on poisoned process — triggering respawn"
            );
            return Err(AcpError::PermissionPoisoned);
        }

        // Step 1: respond to any pending permission request with "cancelled".
        //
        // Under `ask` policy: collect entry ids, peek without pre-removal, and
        // route each through `finish_permission()`. The first write failure poisons
        // the process and stops immediately; Writing-state entries poison immediately.
        //
        // Under `reject`/`allow` policy: use the old single-id path below.
        let ids_to_cancel: Vec<String> = self.pending_permissions.keys().cloned().collect();
        for req_id_str in ids_to_cancel {
            // Peek at state without removing — finish_permission removes on success.
            let state = self
                .pending_permissions
                .get(&req_id_str)
                .map(|e| e.state.clone());
            match state {
                Some(PermissionEntryState::Publishing) => {
                    // Cancel during Publishing: drop the ACK receiver and deny with
                    // cancelled outcome. finish_permission will attempt a kind-40003
                    // edit if sentinel_event_id is set (it is — stored at build time).
                    self.sentinel_ack_result_rx = None; // drop background task receiver
                    let perm_id: serde_json::Value = serde_json::from_str(&req_id_str)
                        .unwrap_or_else(|_| serde_json::Value::String(req_id_str.clone()));
                    let nonce = self
                        .pending_permissions
                        .get(&req_id_str)
                        .map(|e| e.nonce.clone())
                        .unwrap_or_default();
                    let response = permission_response_cancelled(&perm_id);
                    let ok = self
                        .finish_permission(
                            (&req_id_str, &perm_id),
                            (&nonce, "cancelled", response),
                            None,
                            None,
                        )
                        .await;
                    if !ok {
                        return Err(AcpError::PermissionPoisoned);
                    }
                }
                Some(PermissionEntryState::Writing) => {
                    let entry = self.pending_permissions.remove(&req_id_str).unwrap();
                    tracing::error!(
                        target: "acp::cancel",
                        "cancel during permission write for req_id={req_id_str} — poisoning process"
                    );
                    // Emit uncertain terminal so Desktop retires the card.
                    self.observe_authorized(
                        "permission_terminal",
                        AuthorizationEnvelope {
                            request_nonce: entry.nonce.clone(),
                            actionable: false,
                            reason: Some("uncertain".to_string()),
                            expires_at: None,
                        },
                        serde_json::json!({ "id": req_id_str }),
                    );
                    self.permission_poisoned = true;
                    return Err(AcpError::PermissionPoisoned);
                }
                Some(PermissionEntryState::Pending) => {
                    // Parse id back to JSON value for the wire response.
                    let perm_id: serde_json::Value = serde_json::from_str(&req_id_str)
                        .unwrap_or_else(|_| serde_json::Value::String(req_id_str.clone()));
                    let nonce = self
                        .pending_permissions
                        .get(&req_id_str)
                        .map(|e| e.nonce.clone())
                        .unwrap_or_default();
                    let response = permission_response_cancelled(&perm_id);
                    // finish_permission removes the entry and poisons on write failure.
                    // The cancel path has no loop-owned idle state to re-arm.
                    let ok = self
                        .finish_permission(
                            (&req_id_str, &perm_id),
                            (&nonce, "cancelled", response),
                            None,
                            None, // no idle re-arm in cancel path
                        )
                        .await;
                    if !ok {
                        // Write failed → process is already poisoned; stop immediately.
                        return Err(AcpError::PermissionPoisoned);
                    }
                }
                None => {
                    // Entry was concurrently removed (shouldn't happen, but be safe).
                }
            }
        }

        // Old single-id path (reject/allow policy).
        if let Some(perm_id) = self.pending_permission_id.clone() {
            if !self.permission_responded {
                let response = permission_response_cancelled(&perm_id);
                self.write_ndjson(&response).await?;
                tracing::debug!(
                    target: "acp::cancel",
                    "responded cancelled to pending permission id={perm_id}"
                );
            }
            self.pending_permission_id = None;
            self.permission_responded = false;
        }

        // Step 2: send session/cancel notification (no id)
        self.session_cancel(session_id).await?;
        tracing::info!(target: "acp::cancel", "sent session/cancel for {session_id}");
        // Use a fixed 30s idle timeout during cleanup — the cancel notification
        // needs time to propagate and the agent may go silent while winding down.
        // The separate hard_deadline bounds agents that keep producing output
        // but ignore cancellation.
        let cleanup_idle = std::time::Duration::from_secs(30);
        let remaining = hard_deadline
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or_default();
        let result = self
            .read_until_response_with_idle_timeout(
                session_id,
                prompt_id,
                cleanup_idle,
                hard_deadline,
                remaining,
            )
            .await?;
        // Cancel completed — drain any remaining entries (safety net).
        self.pending_permissions.clear();
        self.parse_prompt_response(session_id, &result)
    }

    /// Serialize `value` as a single NDJSON line and flush to the agent's stdin.
    ///
    /// Bounded by a 30-second write timeout. If the agent stops reading stdin
    /// (e.g., it's stuck or dead), the write would otherwise block forever.
    ///
    /// Emits a generic `acp_write` observer event. For permission response paths
    /// that emit their own authorized event, use `write_ndjson_no_observe`.
    async fn write_ndjson(&mut self, value: &serde_json::Value) -> Result<(), AcpError> {
        self.write_ndjson_inner(value, true).await
    }

    /// Write NDJSON without emitting a generic `acp_write` observer event.
    ///
    /// Used for permission response paths that emit a single authorized event
    /// themselves — prevents duplicate generic+authorized telemetry.
    async fn write_ndjson_no_observe(&mut self, value: &serde_json::Value) -> Result<(), AcpError> {
        self.write_ndjson_inner(value, false).await
    }

    async fn write_ndjson_inner(
        &mut self,
        value: &serde_json::Value,
        emit_observe: bool,
    ) -> Result<(), AcpError> {
        #[cfg(test)]
        if let Some(counter) = &self.write_attempt_count {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        let line = serde_json::to_string(value)?;
        tokio::time::timeout(WRITE_TIMEOUT, async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.write_all(b"\n").await?;
            self.stdin.flush().await?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(|_| AcpError::WriteTimeout(WRITE_TIMEOUT))?
        .map_err(AcpError::Io)?;
        if emit_observe {
            self.observe("acp_write", value.clone());
        }
        Ok(())
    }

    /// Default timeout for non-prompt RPCs (initialize, session/new, etc.).
    const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

    /// Terminal helper: write `response` for a permission request, emit one
    /// authorized `acp_write` with `reason`, remove the entry from the map,
    /// and re-arm the idle deadline if no live (Pending|Writing) entries remain.
    ///
    /// On any write failure the process is poisoned — no further bytes are
    /// sent; an observer-only `permission_terminal` event is emitted so Desktop
    /// can retire the card.
    ///
    /// Returns `true` if the write succeeded (terminal outcome delivered),
    /// `false` if the write failed and the process is now poisoned.
    ///
    /// `entry`: `(id_str, id_val)` — map key + JSON-RPC id value for logging.
    /// `outcome`: `(nonce, reason, response)` — what to write and observe.
    /// `write_deadline`: optional absolute deadline bounding the write.
    /// `idle_deadline_and_timeout`: optional `(&mut Instant, Duration)` for
    ///   re-arming the idle window. Pass `None` for synchronous policy paths
    ///   (reject/allow/preflight-denial) that have no loop-owned idle state.
    async fn finish_permission(
        &mut self,
        entry: (&str, &serde_json::Value),
        outcome: (&str, &str, serde_json::Value),
        write_deadline: Option<tokio::time::Instant>,
        idle_deadline_and_timeout: Option<(&mut tokio::time::Instant, std::time::Duration)>,
    ) -> bool {
        let (id_str, id_val) = entry;
        let (nonce, reason, response) = outcome;
        // Write the response. Use a bounded timeout when one is provided.
        let write_result = if let Some(deadline) = write_deadline {
            tokio::time::timeout_at(deadline, self.write_ndjson_no_observe(&response))
                .await
                .unwrap_or(Err(AcpError::WriteTimeout(std::time::Duration::from_secs(
                    30,
                ))))
        } else {
            self.write_ndjson_no_observe(&response).await
        };

        match write_result {
            Ok(()) => {
                // Emit single authorized acp_write correlated by nonce.
                self.observe_authorized(
                    "acp_write",
                    AuthorizationEnvelope {
                        request_nonce: nonce.to_string(),
                        actionable: false,
                        reason: Some(reason.to_string()),
                        expires_at: None,
                    },
                    response.clone(),
                );
                // Extract sentinel data before removing the entry — used to
                // publish the kind-40003 edit that resolves the UI card.
                let sentinel_context = self.pending_permissions.get(id_str).map(|e| {
                    (
                        e.sentinel_event_id.clone(),
                        e.card_actions.clone(),
                        e.nonce.clone(),
                        e.expiry_unix_secs,
                        e.description.clone(),
                    )
                });
                // Remove entry — absence of the nonce is the replay guard.
                self.pending_permissions.remove(id_str);
                // Re-arm idle if no live (Publishing|Pending|Writing) entries remain.
                if let Some((idle_deadline, idle_timeout)) = idle_deadline_and_timeout {
                    let live = self.pending_permissions.values().any(|e| {
                        matches!(
                            e.state,
                            PermissionEntryState::Publishing
                                | PermissionEntryState::Pending
                                | PermissionEntryState::Writing
                        )
                    });
                    if !live {
                        *idle_deadline = tokio::time::Instant::now() + idle_timeout;
                    }
                }
                // Publish the kind-40003 resolved edit if a sentinel was published.
                // Best-effort: a failure here is logged but does not fail the permission
                // resolution — the agent has already received the ACP response.
                if let Some((
                    Some(original_event_id),
                    card_actions,
                    entry_nonce,
                    expiry_unix_secs,
                    entry_description,
                )) = sentinel_context
                {
                    // Clone all relay context upfront to avoid holding &mut self borrows
                    // across the async publish call.
                    let keys_opt = self.agent_relay_keys.clone();
                    let channel_id_opt = self.sentinel_channel_id;
                    let publisher_opt = self.relay_publisher.clone();
                    let session_id_owned = self.observer_context.session_id.clone();
                    let turn_id = self.observer_context.turn_id.clone().unwrap_or_default();

                    if let (Some(keys), Some(channel_id), Some(publisher)) =
                        (keys_opt, channel_id_opt, publisher_opt)
                    {
                        // `reason` maps directly to the schema's `outcome` field.
                        let chosen_option_id: Option<String> = if reason == "applied" {
                            response
                                .pointer("/result/outcome/optionId")
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                        } else {
                            None
                        };
                        // Use the stored wire expiry_unix_secs — no recompute.
                        if let Some(content) = build_sentinel_resolved_payload(
                            &entry_nonce,
                            &original_event_id,
                            &card_actions,
                            expiry_unix_secs,
                            session_id_owned.as_deref(),
                            &turn_id,
                            reason,
                            chosen_option_id.as_deref(),
                            entry_description.as_deref(),
                        ) {
                            if let Some(event) = build_kind40003_sentinel(
                                &keys,
                                channel_id,
                                &original_event_id,
                                &content,
                            ) {
                                // The decision is already irreversible (ACP
                                // response written, entry removed above). Publish
                                // via the acked lane with bounded retransmission
                                // so a socket failure at this instant doesn't
                                // permanently strand the card as "Timed out" —
                                // the same signed event is idempotently resent on
                                // Uncertain until the relay accepts it or the
                                // delivery window closes. The delivery window
                                // starts now (resolution time), independent of
                                // the original card/click deadline, so ordinary
                                // timeouts (where entry_deadline is already past)
                                // still get at least one publish attempt.
                                // Detached so the read loop is never blocked.
                                let delivery_deadline = tokio::time::Instant::now()
                                    + std::time::Duration::from_secs(RESOLVED_DELIVERY_WINDOW_SECS);
                                tokio::spawn(retransmit_resolved_edit(
                                    publisher,
                                    event,
                                    delivery_deadline,
                                ));
                            }
                        }
                    }
                }
                tracing::debug!(
                    target: "acp::permission",
                    "permission id={id_val} finished: reason={reason}"
                );
                true
            }
            Err(e) => {
                tracing::error!(
                    target: "acp::permission",
                    "permission write failed for id={id_val} reason={reason}: {e} — poisoning process"
                );
                self.permission_poisoned = true;
                // Remove entry so cancel doesn't attempt a second write.
                self.pending_permissions.remove(id_str);
                // Emit an observer-only `permission_terminal` so Desktop can retire the card
                // even though no ACP response was confirmed.
                self.observe_authorized(
                    "permission_terminal",
                    AuthorizationEnvelope {
                        request_nonce: nonce.to_string(),
                        actionable: false,
                        reason: Some("uncertain".to_string()),
                        expires_at: None,
                    },
                    serde_json::json!({ "id": id_val }),
                );
                false
            }
        }
    }

    /// Terminal helper for synchronous policy paths (`reject`, `allow`,
    /// preflight denial). Unlike `finish_permission`, this does not manage
    /// `pending_permissions` — these paths are resolved inline before the
    /// entry is inserted.
    ///
    /// Writes `response`, then emits an authorized `acp_write` observer event
    /// correlated by `nonce` with the given `reason`. On write failure the
    /// process is poisoned and `Err(AcpError::PermissionPoisoned)` is returned.
    ///
    /// Standardized `reason` values for policy terminals:
    /// - `"rejected"` — `reject` policy or preflight denial.
    /// - `"allowed"` — `allow` policy auto-approval.
    /// - `"allow_failed_closed"` — `allow` policy with no unique allow_once option.
    async fn finish_permission_sync(
        &mut self,
        id_val: &serde_json::Value,
        nonce: &str,
        reason: &str,
        response: serde_json::Value,
    ) -> Result<(), AcpError> {
        match self.write_ndjson_no_observe(&response).await {
            Ok(()) => {
                self.observe_authorized(
                    "acp_write",
                    AuthorizationEnvelope {
                        request_nonce: nonce.to_string(),
                        actionable: false,
                        reason: Some(reason.to_string()),
                        expires_at: None,
                    },
                    response,
                );
                tracing::debug!(
                    target: "acp::permission",
                    "synchronous permission id={id_val} finished: reason={reason}"
                );
                Ok(())
            }
            Err(e) => {
                tracing::error!(
                    target: "acp::permission",
                    "synchronous permission write failed for id={id_val} reason={reason}: {e} — poisoning process"
                );
                self.permission_poisoned = true;
                Err(AcpError::PermissionPoisoned)
            }
        }
    }

    /// Send a JSON-RPC request and wait for the matching response.
    ///
    /// Assigns the next available id, writes the NDJSON line to stdin,
    /// then calls [`read_until_response`](Self::read_until_response).
    ///
    /// The write phase is bounded by `WRITE_TIMEOUT` (30s) and the read phase
    /// by `REQUEST_TIMEOUT` (60s), so worst-case wall clock is ~90s. Non-prompt
    /// RPCs like `initialize` and `session/new` should complete in seconds;
    /// if they don't, the agent is likely stuck and we must not block forever.
    async fn send_request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let id = self.next_id;
        self.next_id += 1;

        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        tracing::debug!(target: "acp::wire", "→ {}", &serde_json::to_string(&msg).unwrap_or_default());

        // Wrap write + read in a single timeout so a hung agent can't block forever.
        // We cannot use an async block that borrows `self` mutably across two awaits
        // inside timeout(), so we sequence them with early-return on timeout.
        let timeout = Self::REQUEST_TIMEOUT;
        match tokio::time::timeout(timeout, self.write_ndjson(&msg)).await {
            Ok(result) => result?,
            Err(_) => return Err(AcpError::Timeout(timeout)),
        }

        match tokio::time::timeout(timeout, self.read_until_response(id)).await {
            Ok(result) => result,
            Err(_) => Err(AcpError::Timeout(timeout)),
        }
    }

    /// Drain any buffered lines from the agent's stdout without blocking.
    ///
    /// After a [`AcpError::Timeout`] from [`send_request`], the agent may
    /// eventually send the late response. That stale message will sit in the
    /// `BufReader` buffer and be silently skipped by the next `read_until_response`
    /// call (ID mismatch). However, if the caller wants a clean slate — e.g.
    /// before retrying the same method — they can call this to consume any
    /// buffered data with a short deadline.
    ///
    /// This is a best-effort drain: it reads until the buffer is empty or
    /// `drain_timeout` elapses, whichever comes first. Errors are ignored.
    #[allow(dead_code)] // Scaffolding for future model-switch timeout cleanup; not yet wired.
    pub async fn drain_stale_responses(&mut self, drain_timeout: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + drain_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let read_result = tokio::time::timeout(remaining, self.reader.next()).await;
            match read_result {
                // Timeout or stream ended — buffer is empty or agent exited.
                Err(_) | Ok(None) => break,
                Ok(Some(Ok(_))) => {
                    // Consumed one buffered line; loop to drain more.
                    tracing::debug!(target: "acp::wire", "drained stale buffered line");
                }
                Ok(Some(Err(_))) => break,
            }
        }
    }

    /// Send a JSON-RPC **notification** — no `id` field, no response expected.
    ///
    /// Used for `session/cancel`. The absence of `id` is the JSON-RPC 2.0
    /// distinguisher between requests and notifications.
    async fn send_notification(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), AcpError> {
        // Notifications deliberately have NO "id" field.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        tracing::debug!(target: "acp::wire", "→ (notification) {}", &serde_json::to_string(&msg).unwrap_or_default());
        self.write_ndjson(&msg).await?;
        Ok(())
    }

    /// Core message loop: read NDJSON lines until we get a response matching `expected_id`.
    ///
    /// While waiting, handles:
    /// - `session/update` notifications → logged via tracing
    /// - `session/request_permission` requests → rejected unless an owner has
    ///   already selected a non-interactive permission mode at session setup
    /// - Any other messages → debug-logged and ignored; if they carry an `id`
    ///   (i.e. they are requests, not notifications), a JSON-RPC -32601 error is sent.
    ///
    /// Compares the incoming `id` field as a `serde_json::Value` against
    /// `json!(expected_id)` so that both numeric and string IDs work correctly.
    async fn read_until_response(
        &mut self,
        expected_id: u64,
    ) -> Result<serde_json::Value, AcpError> {
        loop {
            // LinesCodec::new_with_max_length enforces MAX_LINE_SIZE at the
            // read level — the buffer never grows beyond the limit, preventing
            // OOM from rogue agents writing infinite non-newline bytes.
            let line = match self.reader.next().await {
                None => return Err(AcpError::AgentExited),
                Some(Err(LinesCodecError::MaxLineLengthExceeded)) => {
                    return Err(AcpError::Protocol(
                        "agent stdout line exceeded 10MB limit".into(),
                    ));
                }
                Some(Err(e)) => {
                    return Err(AcpError::Io(std::io::Error::other(e)));
                }
                Some(Ok(line)) => line,
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Only log and reset idle after we have a valid non-empty line.
            tracing::debug!(target: "acp::wire", "← {trimmed}");

            let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    self.observe(
                        "acp_parse_error",
                        serde_json::json!({
                            "line": trimmed,
                            "error": e.to_string(),
                        }),
                    );
                    tracing::warn!(
                        target: "acp::wire",
                        "failed to parse line as JSON: {e} — skipping"
                    );
                    continue;
                }
            };
            self.observe("acp_read", msg.clone());

            // Check if this is a response to our expected request (has matching id
            // AND no `method` field — a `method` field means it's an agent-initiated
            // request, not a response, even if the id happens to match).
            if let Some(id) = msg.get("id") {
                if *id == serde_json::json!(expected_id) && msg.get("method").is_none() {
                    if let Some(error) = msg.get("error") {
                        return Err(agent_error_from_json(error));
                    }
                    return Ok(msg["result"].clone());
                }
            }

            // Dispatch by method name (notifications and agent-initiated requests).
            if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
                match method {
                    "session/update" => {
                        let _ = self.handle_session_update(&msg);
                    }
                    "_goose/unstable/session/update" => {
                        self.handle_goose_usage_update(&msg);
                    }
                    "session/request_permission" => {
                        // Pre-turn (session/new) path: no decision arm installed.
                        // Force reject regardless of policy — ask requests would
                        // register map entries that can never be resolved without
                        // the turn reader's decision arm.
                        let saved_policy = self.permission_config.policy;
                        if matches!(saved_policy, PermissionPolicy::Ask) {
                            // Temporarily downgrade to reject for this request only.
                            let saved = std::mem::replace(
                                &mut self.permission_config.policy,
                                PermissionPolicy::Reject,
                            );
                            let deadline = tokio::time::Instant::now()
                                + std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS);
                            let _ = self.handle_permission_request(&msg, deadline).await;
                            self.permission_config.policy = saved;
                        } else {
                            let deadline = tokio::time::Instant::now()
                                + std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS);
                            self.handle_permission_request(&msg, deadline).await?;
                        }
                    }
                    other => {
                        // If the unknown message has an id, it's a request expecting a reply.
                        // Silence would cause the agent to hang waiting for a response.
                        // Send a JSON-RPC -32601 "Method not found" error.
                        if msg.get("id").is_some() {
                            let err_resp = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": msg["id"],
                                "error": {"code": -32601, "message": format!("Method not found: {other}")}
                            });
                            // Surface write failures — a broken pipe means the
                            // agent process is dead and continuing would hang.
                            self.write_ndjson(&err_resp).await?;
                        }
                        tracing::debug!(target: "acp::wire", "ignoring unknown method: {other}");
                    }
                }
            }
        }
    }

    /// Idle-aware message loop: like [`read_until_response`] but resets an idle
    /// deadline on every stdout line. Fires [`AcpError::IdleTimeout`] on silence
    /// or [`AcpError::HardTimeout`] on absolute wall-clock cap.
    ///
    /// `hard_deadline` is an absolute `Instant` (pre-computed by the caller) so
    /// that `cancel_with_cleanup` can inherit the remaining budget from the
    /// original turn rather than starting a fresh timer.
    /// Read agent messages until the response with `expected_id` arrives, or
    /// either of two timeouts fires. Returns `Result<value, IdleTimeout |
    /// HardTimeout | other>`.
    ///
    /// - `idle_timeout`: silent-agent guard, **reset on every line of valid
    ///   JSON** (and explicitly on `session/update` notifications).
    /// - `hard_deadline`: absolute wall-clock cap on the whole call, passed
    ///   in so that `cancel_with_cleanup` can inherit the remaining budget
    ///   from the original turn rather than starting a fresh timer.
    ///
    /// While reading, the loop interleaves goose-native non-cancelling steer
    /// requests via `tokio::select!`. The select uses `biased` for
    /// reader-first throughput, with a pre-select deadline check at the top
    /// of every loop iteration so a continuously-ready reader arm cannot
    /// starve the hard deadline (Max's review gate). The steer arm is
    /// guarded by `pending_steer.is_none()` so at most one steer is in
    /// flight at a time; a successful steer response is routed to the
    /// caller's oneshot ack instead of being returned as the prompt result.
    ///
    /// `session_id` is threaded in lexically by callers so the goose-native
    /// steer arm can complete `sessionId` in the steer JSON-RPC params at
    /// write time without needing access to outer state. See
    /// [`crate::pool::SteerRequest`] for why params are built here and not
    /// in the main loop.
    async fn read_until_response_with_idle_timeout(
        &mut self,
        session_id: &str,
        expected_id: u64,
        idle_timeout: std::time::Duration,
        hard_deadline: tokio::time::Instant,
        max_duration: std::time::Duration,
    ) -> Result<serde_json::Value, AcpError> {
        use tokio::time::Instant;

        // Take the per-turn steer receiver into a local so it can be
        // borrowed independently of `self.reader` inside `select!`.
        // Dropped at scope exit (return paths drain `pending_steer` first
        // so the ack_tx oneshot is never leaked silently).
        let mut steer_rx = self.steer_rx.take();

        // Take the per-session permission decision receiver into a local for
        // the same reason: `self.reader` and `decision_rx` cannot both be
        // borrowed inside `select!` via `self`.
        let mut decision_rx = self.permission_decision_rx.take();

        // Receiver for sentinel publish ACK results. Set after
        // `handle_permission_request` installs a sentinel; moved here from
        // `self.sentinel_ack_result_rx` at the top of each loop iteration so
        // it can be polled inside `select!` independently of `self`.
        let mut ack_result_rx: Option<
            tokio::sync::mpsc::Receiver<(String, crate::relay::AckOutcome)>,
        > = None;

        // Tracks the in-flight steer write: `(request_id, transport, ack_tx)`.
        // While `Some`, the steer arm is gated off so we don't stack writes,
        // and a response matching `id` is routed to the ack_tx instead
        // of being treated as the prompt result. `transport` records which
        // method was written so the response arm decodes the result shape
        // that method actually returns. Drained on every return path with
        // `PromptCompletedNeutral` so callers are never left hanging.
        let mut pending_steer: Option<(
            u64,
            SteerTransport,
            tokio::sync::oneshot::Sender<crate::pool::SteerAck>,
        )> = None;

        let now = Instant::now();
        let mut idle_deadline = now + idle_timeout;
        let mut hard_deadline = hard_deadline;
        let mut last_activity_at = now;

        loop {
            // Move any newly-set sentinel ACK receiver from self to the local,
            // so it can be polled inside select! without conflicting with self.
            if ack_result_rx.is_none() {
                if let Some(rx) = self.sentinel_ack_result_rx.take() {
                    ack_result_rx = Some(rx);
                }
            }

            // If the process was poisoned by a cancel-during-write, surface the
            // error immediately so the caller can respawn.
            if self.permission_poisoned {
                if let Some((_, _, ack_tx)) = pending_steer.take() {
                    let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                }
                return Err(AcpError::PermissionPoisoned);
            }

            // Determine which deadline fires first BEFORE sleeping — this is
            // the classification we'll use on timeout, immune to scheduler jitter.
            //
            // Deadline logic:
            // - When any Pending permission entries exist, suspend the idle
            //   deadline (owner is deciding; agent silence is expected) and
            //   wake on the earliest permission deadline instead.
            // - Otherwise wake on min(idle, hard) as normal.
            let has_pending_permissions = self.pending_permissions.values().any(|e| {
                matches!(
                    e.state,
                    PermissionEntryState::Publishing | PermissionEntryState::Pending
                )
            });
            let next_deadline;
            let idle_fires_first;
            if has_pending_permissions {
                // Suspend idle; find earliest permission deadline (capped by hard).
                // Publishing entries use their publish_deadline (in sentinel_ack_rx)
                // or their entry deadline — we use entry.deadline for both states.
                let earliest_perm = self
                    .pending_permissions
                    .values()
                    .filter(|e| {
                        matches!(
                            e.state,
                            PermissionEntryState::Publishing | PermissionEntryState::Pending
                        )
                    })
                    .map(|e| e.deadline)
                    .min()
                    .unwrap_or(hard_deadline);
                // Also factor in the publish deadline for the in-flight ACK.
                // The background task enforces publish_deadline itself; for the
                // select! wakeup we rely on earliest_perm (the entry.deadline).
                next_deadline = earliest_perm.min(hard_deadline);
                idle_fires_first = false; // hard deadline governs if we wake
            } else {
                idle_fires_first = idle_deadline < hard_deadline;
                next_deadline = if idle_fires_first {
                    idle_deadline
                } else {
                    hard_deadline
                };
            }

            // Pre-select deadline check — required by Max's review. Under
            // `biased`, a continuously-ready reader arm wins every poll and
            // `sleep_until(next_deadline)` is never reached, silently
            // defeating the hard-deadline guarantee for agents that keep
            // producing output (see `acp.rs:608` for why the hard deadline
            // exists). Check the classified deadline here so a steady-
            // stream agent is still bounded.
            if Instant::now() >= next_deadline {
                // When pending permission entries exist (including when
                // entry.deadline == hard_deadline), fall through to let the
                // expiry block process timed-out entries first.
                // We return HardTimeout after the expiry block in that case.
                if !has_pending_permissions {
                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                        // Prompt is timing out — release the withheld event via
                        // PromptCompletedNeutral (no fallback signal: there is
                        // no in-flight turn to signal once we return, and
                        // normal dispatch handles redelivery).
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    if idle_fires_first {
                        tracing::warn!("idle timeout ({idle_timeout:?}) — no agent activity");
                        return Err(AcpError::IdleTimeout(idle_timeout));
                    } else {
                        let silence = Instant::now().saturating_duration_since(last_activity_at);
                        tracing::warn!("hard turn timeout exceeded (silence {silence:?})");
                        return Err(AcpError::HardTimeout { silence });
                    }
                }
            }

            // Expire any pending `ask` permission entries whose per-request
            // deadline has passed. Fail closed: write denial response for each
            // expired entry. `finish_permission` removes the entry on success
            // and emits `permission_terminal` + poisons on write failure.
            {
                let now = Instant::now();

                // Publishing entries whose publish deadline has passed: the
                // background task handles the publish timeout and sends an
                // Uncertain outcome via sentinel_ack_result_rx. No action
                // needed here — the select! arm will process it on next iteration.
                // However, if the entry deadline (300s) has also passed while
                // still in Publishing (very unusual), deny it directly.
                {
                    let publishing_expired: Vec<_> = self
                        .pending_permissions
                        .iter()
                        .filter(|(_, e)| {
                            matches!(e.state, PermissionEntryState::Publishing) && now >= e.deadline
                        })
                        .map(|(k, e)| {
                            (
                                k.clone(),
                                serde_json::from_str(k)
                                    .unwrap_or_else(|_| serde_json::Value::String(k.clone())),
                                e.options_snapshot.clone(),
                                e.nonce.clone(),
                            )
                        })
                        .collect();
                    for (id_str, id_val, opts, nonce) in publishing_expired {
                        tracing::warn!(
                            target: "acp::permission",
                            "Publishing entry hard deadline for id={id_val} — failing closed"
                        );
                        // Drop the ACK result channel if it matches.
                        if self
                            .sentinel_ack_result_rx
                            .as_ref()
                            .map(|_| true)
                            .unwrap_or(false)
                        {
                            self.sentinel_ack_result_rx = None;
                        }
                        if let Ok(response) = permission_denial_response(&id_val, &opts) {
                            let ok = self
                                .finish_permission(
                                    (&id_str, &id_val),
                                    (&nonce, "timed_out", response),
                                    None,
                                    Some((&mut idle_deadline, idle_timeout)),
                                )
                                .await;
                            if !ok {
                                return Err(AcpError::PermissionPoisoned);
                            }
                        }
                    }
                }

                let expired: Vec<(String, serde_json::Value, Vec<serde_json::Value>, String)> =
                    self.pending_permissions
                        .iter()
                        .filter(|(_, e)| {
                            matches!(e.state, PermissionEntryState::Pending) && now >= e.deadline
                        })
                        .map(|(id_str, e)| {
                            (
                                id_str.clone(),
                                serde_json::from_str(id_str)
                                    .unwrap_or_else(|_| serde_json::Value::String(id_str.clone())),
                                e.options_snapshot.clone(),
                                e.nonce.clone(),
                            )
                        })
                        .collect();
                for (id_str, id_val, opts, nonce) in expired {
                    tracing::warn!(
                        target: "acp::permission",
                        "ask timeout for permission id={id_val} — failing closed"
                    );
                    if let Ok(response) = permission_denial_response(&id_val, &opts) {
                        let ok = self
                            .finish_permission(
                                (&id_str, &id_val),
                                (&nonce, "timed_out", response),
                                None,
                                Some((&mut idle_deadline, idle_timeout)),
                            )
                            .await;
                        if !ok {
                            // Write failed → process is poisoned; stop immediately.
                            return Err(AcpError::PermissionPoisoned);
                        }
                    }
                }
            }

            // After processing expired permission entries, check if the hard
            // deadline has now been reached — this handles the deadline-equality
            // case where entry.deadline == hard_deadline: we wrote the fail-closed
            // response above, now exit with HardTimeout.
            if Instant::now() >= hard_deadline
                && !self.pending_permissions.values().any(|e| {
                    matches!(
                        e.state,
                        PermissionEntryState::Publishing | PermissionEntryState::Pending
                    )
                })
            {
                if let Some((_, _, ack_tx)) = pending_steer.take() {
                    let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                }
                let silence = Instant::now().saturating_duration_since(last_activity_at);
                tracing::warn!("hard turn timeout exceeded (silence {silence:?})");
                return Err(AcpError::HardTimeout { silence });
            }

            // LinesCodec::new_with_max_length enforces MAX_LINE_SIZE at the
            // read level — the buffer never grows beyond the limit.
            let read_result = tokio::select! {
                biased;
                // Decision arm — must be FIRST in the biased select! (spec §9) so
                // owner decisions are not starved by a continuously-ready stdout.
                // Cancel-safe: `mpsc::Receiver::recv` does not lose messages on drop.
                Some(decision) = async {
                    match decision_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => None,
                    }
                } => {
                    // Find the pending entry by nonce match.
                    // A decision arriving during Publishing is buffered; it will
                    // be applied immediately when the relay ACK is received.
                    let entry_id = self.pending_permissions
                        .iter()
                        .find(|(_, e)| {
                            matches!(
                                e.state,
                                PermissionEntryState::Publishing | PermissionEntryState::Pending
                            ) && e.nonce == decision.request_nonce
                        })
                        .map(|(k, _)| k.clone());

                    if let Some(id_str) = entry_id {
                        // Accept the decision only when its optionId is exactly
                        // one of the two ruled card actions (allow_once /
                        // reject_once) — never a forbidden option (e.g.
                        // `allow_always`) that the adapter offered but the card
                        // never surfaced. Membership in the raw snapshot is not
                        // sufficient.
                        let opt_valid = self.pending_permissions
                            .get(&id_str)
                            .map(|e| e.card_actions.accepts(decision.option_id.as_str()))
                            .unwrap_or(false);

                        if !opt_valid {
                            tracing::warn!(
                                target: "acp::permission",
                                "permission_decision optionId {:?} is not a ruled card action for id={id_str} — ignoring",
                                decision.option_id
                            );
                        } else {
                            let entry_state = self
                                .pending_permissions
                                .get(&id_str)
                                .map(|e| e.state.clone());

                            match entry_state {
                                Some(PermissionEntryState::Publishing) => {
                                    // Buffer the first valid decision; apply on ACK.
                                    // Subsequent valid decisions for the same nonce
                                    // are ignored — first-wins prevents a later
                                    // opposing decision from overwriting the first
                                    // while the card publish is awaiting ACK.
                                    if let Some(entry) =
                                        self.pending_permissions.get_mut(&id_str)
                                    {
                                        if entry.early_decision.is_none() {
                                            entry.early_decision = Some(decision);
                                            tracing::debug!(
                                                target: "acp::permission",
                                                "permission_decision buffered during Publishing for id={id_str}"
                                            );
                                        } else {
                                            tracing::debug!(
                                                target: "acp::permission",
                                                "permission_decision ignored — first decision already buffered for id={id_str}"
                                            );
                                        }
                                    }
                                }
                                Some(PermissionEntryState::Pending) => {
                                    // Transition Pending → Writing.
                                    let (nonce, id_val) = {
                                        let entry =
                                            self.pending_permissions.get_mut(&id_str).unwrap();
                                        entry.state = PermissionEntryState::Writing;
                                        (
                                            entry.nonce.clone(),
                                            serde_json::from_str::<serde_json::Value>(&id_str)
                                                .unwrap_or_else(|_| {
                                                    serde_json::Value::String(id_str.clone())
                                                }),
                                        )
                                    };

                                    let response =
                                        permission_response_selected(&id_val, &decision.option_id);
                                    let write_deadline = (Instant::now()
                                        + std::time::Duration::from_secs(30))
                                    .min(hard_deadline);
                                    let ok = self
                                        .finish_permission(
                                            (&id_str, &id_val),
                                            (&nonce, "applied", response),
                                            Some(write_deadline),
                                            Some((&mut idle_deadline, idle_timeout)),
                                        )
                                        .await;
                                    if ok {
                                        tracing::info!(
                                            target: "acp::permission",
                                            "permission id={id_val} answered: optionId={:?}",
                                            decision.option_id
                                        );
                                    } else {
                                        // Write failed → process poisoned; break out immediately.
                                        if let Some((_, _, ack_tx)) = pending_steer.take() {
                                            let _ = ack_tx
                                                .send(crate::pool::SteerAck::PromptCompletedNeutral);
                                        }
                                        return Err(AcpError::PermissionPoisoned);
                                    }
                                }
                                _ => {}
                            }
                        }
                    } else {
                        tracing::warn!(
                            target: "acp::permission",
                            "permission_decision nonce {:?} has no matching pending entry — ignoring",
                            decision.request_nonce
                        );
                    }
                    None // loop back; don't set read_result
                }
                // Sentinel ACK arm: fires when the relay responds to the kind-9 publish.
                // Publishing → Pending on Accepted (apply any buffered early decision).
                // Any other outcome → deny synchronously and remove the entry.
                // Cancel-safe: mpsc::Receiver::recv does not lose messages on drop.
                Some((pub_id, ack_result)) = async {
                    match ack_result_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => None,
                    }
                } => {
                    // Received one ACK result; the channel is now drained (capacity=1).
                    ack_result_rx = None;
                    match ack_result {
                        crate::relay::AckOutcome::Accepted => {
                            // Transition Publishing → Pending and take any buffered
                            // early decision in one mutable access.
                            // sentinel_event_id is already stored at build time.
                            let early_decision =
                                if let Some(entry) = self
                                    .pending_permissions
                                    .get_mut(&pub_id)
                                    .filter(|e| matches!(e.state, PermissionEntryState::Publishing))
                                {
                                    entry.state = PermissionEntryState::Pending;
                                    tracing::debug!(
                                        target: "acp::permission",
                                        "sentinel ACK accepted for id={pub_id} — transitioning to Pending"
                                    );
                                    entry.early_decision.take()
                                } else {
                                    None
                                };
                            // Apply buffered early decision if present.
                            if let Some(decision) = early_decision {
                                let id_str = pub_id.clone();
                                let opt_valid = self
                                    .pending_permissions
                                    .get(&id_str)
                                    .map(|e| e.card_actions.accepts(decision.option_id.as_str()))
                                    .unwrap_or(false);
                                if opt_valid {
                                    let (nonce, id_val) = {
                                        let entry = self
                                            .pending_permissions
                                            .get_mut(&id_str)
                                            .unwrap();
                                        entry.state = PermissionEntryState::Writing;
                                        (
                                            entry.nonce.clone(),
                                            serde_json::from_str::<serde_json::Value>(&id_str)
                                                .unwrap_or_else(|_| {
                                                    serde_json::Value::String(id_str.clone())
                                                }),
                                        )
                                    };
                                    let response = permission_response_selected(
                                        &id_val,
                                        &decision.option_id,
                                    );
                                    let write_deadline = (Instant::now()
                                        + std::time::Duration::from_secs(30))
                                    .min(hard_deadline);
                                    let ok = self
                                        .finish_permission(
                                            (&id_str, &id_val),
                                            (&nonce, "applied", response),
                                            Some(write_deadline),
                                            Some((&mut idle_deadline, idle_timeout)),
                                        )
                                        .await;
                                    if ok {
                                        tracing::info!(
                                            target: "acp::permission",
                                            "permission id={id_val} answered (early decision applied): optionId={:?}",
                                            decision.option_id
                                        );
                                    } else {
                                        if let Some((_, _, ack_tx)) = pending_steer.take() {
                                            let _ = ack_tx.send(
                                                crate::pool::SteerAck::PromptCompletedNeutral,
                                            );
                                        }
                                        return Err(AcpError::PermissionPoisoned);
                                    }
                                }
                            }
                        }
                        outcome => {
                            // Rejected or Uncertain: deny and remove the entry.
                            let reason_str = match &outcome {
                                crate::relay::AckOutcome::Rejected { message } => {
                                    format!("rejected by relay: {message}")
                                }
                                _ => "relay delivery uncertain".to_string(),
                            };
                            tracing::warn!(
                                target: "acp::permission",
                                "sentinel publish not accepted for id={pub_id}: {reason_str} — failing closed"
                            );
                            if let Some(entry) = self
                                .pending_permissions
                                .get(&pub_id)
                                .filter(|e| matches!(e.state, PermissionEntryState::Publishing))
                            {
                                let id_val: serde_json::Value = serde_json::from_str(&pub_id)
                                    .unwrap_or_else(|_| serde_json::Value::String(pub_id.clone()));
                                let opts = entry.options_snapshot.clone();
                                let nonce = entry.nonce.clone();
                                if let Ok(response) = permission_denial_response(&id_val, &opts) {
                                    let ok = self
                                        .finish_permission(
                                            (&pub_id, &id_val),
                                            (&nonce, "timed_out", response),
                                            None,
                                            Some((&mut idle_deadline, idle_timeout)),
                                        )
                                        .await;
                                    if !ok {
                                        if let Some((_, _, ack_tx)) = pending_steer.take() {
                                            let _ = ack_tx.send(
                                                crate::pool::SteerAck::PromptCompletedNeutral,
                                            );
                                        }
                                        return Err(AcpError::PermissionPoisoned);
                                    }
                                }
                            }
                        }
                    }
                    None // loop back
                }
                read_result = self.reader.next() => Some(read_result),
                // Steer arm: gated off whenever a steer write is already in
                // flight so we don't stack two writes against the same
                // process. The `async { steer_rx.as_mut()?.recv().await }`
                // wrapper produces `None` when no receiver is installed,
                // which mismatches the `Some(req)` pattern and disables the
                // branch for that iteration (no busy loop). Cancel-safe:
                // `mpsc::Receiver::recv` does not lose messages on drop.
                Some(req) = async {
                    match steer_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => None,
                    }
                }, if pending_steer.is_none() => {
                    // Selected: choose the steer transport and build its
                    // params at write time using the lexical `session_id`
                    // and the freshest `active_run_id`.
                    //
                    // `active_run_id` is updated by `session/update`
                    // notifications inside this very loop; reading it here
                    // (rather than snapshotting at dispatch) guarantees the
                    // value matches what goose's run-id check will compare
                    // against.
                    //
                    // Transport precedence:
                    //   Some(run_id)              → GOOSE_STEER_METHOD. goose
                    //     wins whenever a run id exists: `expectedRunId` is
                    //     strictly more precise about *which* run is steered.
                    //   None + steering_supported → ACP_STEER_METHOD, the
                    //     cross-adapter extension (claude-agent-acp,
                    //     codex-acp), which takes no run id.
                    //   None + !steering_supported → write nothing and ack
                    //     `ExpectedRunIdMissing`; the main loop maps this to
                    //     the universal cancel+merge `Steer` fallback.
                    //
                    // The capability flag is the ONLY gate on writing
                    // ACP_STEER_METHOD. Probing an unknown method is unsafe:
                    // codex-acp answers unrecognized extension methods with
                    // `{}` — a JSON-RPC success — which would be read as a
                    // delivered steer and silently drop the user's message.
                    let prompt_block_refs: Vec<&str> =
                        req.prompt_blocks.iter().map(String::as_str).collect();
                    let selected = match (&self.active_run_id, self.steering_supported) {
                        (Some(run_id), _) => Some((
                            SteerTransport::Goose,
                            GOOSE_STEER_METHOD,
                            build_goose_steer_params(session_id, run_id, &prompt_block_refs),
                        )),
                        (None, true) => Some((
                            SteerTransport::AcpExtension,
                            ACP_STEER_METHOD,
                            build_acp_steer_params(session_id, &prompt_block_refs),
                        )),
                        (None, false) => None,
                    };
                    match selected {
                        None => {
                            tracing::warn!(
                                "steer: no active_run_id and agent did not advertise \
                                 {ACP_STEER_METHOD} — falling back to cancel+merge"
                            );
                            let _ = req.ack_tx.send(crate::pool::SteerAck::Err(
                                crate::pool::SteerError::ExpectedRunIdMissing,
                            ));
                        }
                        Some((transport, method, params)) => {
                            let id = self.next_id;
                            self.next_id += 1;
                            let msg = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "method": method,
                                "params": params,
                            });
                            tracing::debug!(
                                target: "acp::wire",
                                "→ {}",
                                serde_json::to_string(&msg).unwrap_or_default()
                            );
                            match self.write_ndjson(&msg).await {
                                Ok(()) => {
                                    pending_steer = Some((id, transport, req.ack_tx));
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "steer write failed ({method}): {e} — releasing withheld event"
                                    );
                                    let _ = req.ack_tx.send(crate::pool::SteerAck::Err(
                                        crate::pool::SteerError::Transport(e.to_string()),
                                    ));
                                }
                            }
                        }
                    }
                    // Loop back to the next iteration without consuming a
                    // reader line; we'll wait for either the prompt
                    // response or the steer response next.
                    None
                }
                _ = tokio::time::sleep_until(next_deadline) => {
                    // The pre-select check at the top of the next iteration
                    // would catch this anyway, but firing the deadline arm
                    // here makes the wakeup immediate (no extra reader poll
                    // round-trip when stdout is idle).
                    // When pending permissions exist (including equality with
                    // hard_deadline), loop back to let the expiry block process
                    // timed-out entries first.
                    if has_pending_permissions {
                        None // loop back; expiry block will fire (then we return HardTimeout if still past)
                    } else {
                        if let Some((_, _, ack_tx)) = pending_steer.take() {
                            let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                        }
                        if idle_fires_first {
                            tracing::warn!("idle timeout ({idle_timeout:?}) — no agent activity");
                            return Err(AcpError::IdleTimeout(idle_timeout));
                        } else {
                            let silence = Instant::now().saturating_duration_since(last_activity_at);
                            tracing::warn!("hard turn timeout exceeded (silence {silence:?})");
                            return Err(AcpError::HardTimeout { silence });
                        }
                    }
                }
            };

            // Steer arm fired (or the select selected nothing read-side this
            // iteration): no reader frame to process, loop to re-evaluate
            // deadlines and arm the next select.
            let read_result = match read_result {
                Some(r) => r,
                None => continue,
            };

            match read_result {
                None => {
                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    return Err(AcpError::AgentExited);
                }
                Some(Err(LinesCodecError::MaxLineLengthExceeded)) => {
                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    return Err(AcpError::Protocol(
                        "agent stdout line exceeded 10MB limit".into(),
                    ));
                }
                Some(Err(e)) => {
                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                        let _ = ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                    }
                    return Err(AcpError::Io(std::io::Error::other(e)));
                }
                Some(Ok(line)) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    tracing::debug!(target: "acp::wire", "← {trimmed}");

                    let msg: serde_json::Value = match serde_json::from_str(trimmed) {
                        Ok(v) => v,
                        Err(e) => {
                            self.observe(
                                "acp_parse_error",
                                serde_json::json!({
                                    "line": trimmed,
                                    "error": e.to_string(),
                                }),
                            );
                            tracing::warn!(
                                target: "acp::wire",
                                "failed to parse line as JSON: {e} — skipping"
                            );
                            continue;
                        }
                    };
                    // Suppress the generic `acp_read` for `session/request_permission`
                    // under the `ask` policy — `handle_permission_request` emits the
                    // single enveloped frame instead (spec §6 "one frame per request").
                    let is_ask_permission_request =
                        matches!(self.permission_config.policy, PermissionPolicy::Ask)
                            && msg.get("method").and_then(|v| v.as_str())
                                == Some("session/request_permission");
                    if !is_ask_permission_request {
                        self.observe("acp_read", msg.clone());
                    }

                    let activity_now = Instant::now();
                    idle_deadline = activity_now + idle_timeout;
                    last_activity_at = activity_now;

                    // Steer response routing must come BEFORE the prompt
                    // response check: a steer response is a regular
                    // JSON-RPC response (id + result/error, no method),
                    // so the matcher must disambiguate by id. Both checks
                    // share the `no method` guard.
                    if let Some(id) = msg.get("id") {
                        if msg.get("method").is_none() {
                            if let Some((steer_id, _, _)) = pending_steer.as_ref() {
                                if *id == serde_json::json!(*steer_id) {
                                    // Take the ack_tx out and route the
                                    // response. We do not return — keep
                                    // reading until the prompt response
                                    // arrives.
                                    let (_, transport, ack_tx) =
                                        pending_steer.take().expect("just checked");
                                    let ack = if let Some(error) = msg.get("error") {
                                        let code = error
                                            .get("code")
                                            .and_then(|c| c.as_i64())
                                            .unwrap_or(-1);
                                        let message = error.to_string();
                                        crate::pool::SteerAck::Err(
                                            crate::pool::SteerError::AgentError { code, message },
                                        )
                                    } else {
                                        // Success result. Whether it counts as
                                        // a delivered steer — and whether the
                                        // turn Buzz awaits is still running —
                                        // depends on the transport.
                                        let outcome = match transport {
                                            // goose returns no outcome field;
                                            // a success response means the
                                            // steer landed in the live run.
                                            SteerTransport::Goose => Some(STEER_OUTCOME_INJECTED),
                                            // The outcome must be positively
                                            // recognized. An unknown or absent
                                            // value (codex-acp answers
                                            // unrecognized ext methods with a
                                            // bare `{}`) is a rejection, never
                                            // a delivery — treating it as
                                            // success would drop the event.
                                            SteerTransport::AcpExtension => msg
                                                .pointer("/result/outcome")
                                                .and_then(|v| v.as_str())
                                                .filter(|o| {
                                                    *o == STEER_OUTCOME_INJECTED
                                                        || *o == STEER_OUTCOME_STARTED_NEW_TURN
                                                }),
                                        };
                                        match outcome {
                                            Some(STEER_OUTCOME_STARTED_NEW_TURN) => {
                                                // Delivered, but into a NEW
                                                // turn: the one this read loop
                                                // is awaiting had already
                                                // finished. Renewing the hard
                                                // deadline here would extend
                                                // the clock on a settled turn,
                                                // so leave it alone and let the
                                                // prompt response land on its
                                                // original budget.
                                                tracing::info!(
                                                    "steer accepted as {STEER_OUTCOME_STARTED_NEW_TURN}: \
                                                     awaited turn had ended — hard deadline not renewed"
                                                );
                                                crate::pool::SteerAck::Success {
                                                    session_id: session_id.to_owned(),
                                                }
                                            }
                                            Some(_) => {
                                                let renew_now = Instant::now();
                                                let new_deadline = renew_now + max_duration;
                                                if new_deadline > hard_deadline {
                                                    hard_deadline = new_deadline;
                                                    self.current_hard_deadline = Some(new_deadline);
                                                    tracing::info!(
                                                        "steer success: renewed hard deadline ({max_duration:?} from now)"
                                                    );
                                                }
                                                crate::pool::SteerAck::Success {
                                                    session_id: session_id.to_owned(),
                                                }
                                            }
                                            None => {
                                                // Report the raw string when
                                                // there is one, so logs read
                                                // `failed` not `"failed"`;
                                                // fall back to the JSON for a
                                                // non-string value.
                                                let reported = match msg.pointer("/result/outcome")
                                                {
                                                    None => "<absent>".to_string(),
                                                    Some(serde_json::Value::String(s)) => s.clone(),
                                                    Some(other) => other.to_string(),
                                                };
                                                tracing::warn!(
                                                    "steer rejected: {ACP_STEER_METHOD} returned \
                                                     unrecognized outcome {reported} — releasing \
                                                     withheld event for cancel+merge"
                                                );
                                                crate::pool::SteerAck::Err(
                                                    crate::pool::SteerError::OutcomeRejected {
                                                        outcome: reported,
                                                    },
                                                )
                                            }
                                        }
                                    };
                                    let _ = ack_tx.send(ack);
                                    continue;
                                }
                            }
                            if *id == serde_json::json!(expected_id) {
                                if let Some(error) = msg.get("error") {
                                    if let Some((_, _, ack_tx)) = pending_steer.take() {
                                        let _ = ack_tx
                                            .send(crate::pool::SteerAck::PromptCompletedNeutral);
                                    }
                                    return Err(agent_error_from_json(error));
                                }
                                if let Some((_, _, ack_tx)) = pending_steer.take() {
                                    let _ =
                                        ack_tx.send(crate::pool::SteerAck::PromptCompletedNeutral);
                                }
                                return Ok(msg["result"].clone());
                            }
                        }
                    }

                    // Dispatch notifications and agent-initiated requests.
                    if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
                        match method {
                            "session/update" => {
                                if self.handle_session_update(&msg) {
                                    let activity_now = Instant::now();
                                    idle_deadline = activity_now + idle_timeout;
                                    last_activity_at = activity_now;
                                    tracing::debug!("idle clock reset: tool call started");
                                }
                            }
                            "_goose/unstable/session/update" => {
                                self.handle_goose_usage_update(&msg);
                            }
                            "session/request_permission" => {
                                self.handle_permission_request(&msg, hard_deadline).await?;
                            }
                            other => {
                                // If the unknown message has an id, it's a request expecting a reply.
                                // Silence would cause the agent to hang waiting for a response.
                                // Send a JSON-RPC -32601 "Method not found" error.
                                if msg.get("id").is_some() {
                                    let err_resp = serde_json::json!({
                                        "jsonrpc": "2.0",
                                        "id": msg["id"],
                                        "error": {"code": -32601, "message": format!("Method not found: {other}")}
                                    });
                                    // Surface write failures — a broken pipe means the
                                    // agent process is dead and continuing would hang.
                                    self.write_ndjson(&err_resp).await?;
                                }
                                tracing::debug!(target: "acp::wire", "ignoring unknown method: {other}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// Log a `session/update` notification via tracing.
    ///
    /// The discriminator field is `sessionUpdate` (not `type`) per the ACP schema.
    /// Returns `true` if the update indicates a tool call started, signaling that
    /// the idle clock should be explicitly reset (the agent will be silent while
    /// the tool executes).
    ///
    /// Takes `&mut self` (not `&self`) because some updates carry agent state
    /// the client must observe — notably goose's `session_info_update` with
    /// `_meta.goose.activeRunId`, which seeds [`active_run_id`](Self::active_run_id)
    /// so the steer arm can target `_goose/unstable/session/steer` at the
    /// correct run. Agents that never emit it (claude-agent-acp, codex-acp)
    /// leave it `None` and are steered via `_session/steering` instead, which
    /// needs no run id.
    fn handle_session_update(&mut self, msg: &serde_json::Value) -> bool {
        let update = &msg["params"]["update"];
        let update_type = update
            .get("sessionUpdate")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        match update_type {
            "agent_message_chunk" => {
                if let Some(text) = update["content"]["text"].as_str() {
                    tracing::info!(target: "acp::stream", "{text}");
                }
                false
            }
            "tool_call" => {
                let title = update
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let kind = update
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                tracing::info!(target: "acp::tool", "tool_call: {title} ({kind})");
                true
            }
            "tool_call_update" => {
                let tool_id = update
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                tracing::info!(target: "acp::tool", "tool_call_update: {tool_id} → {status}");
                false
            }
            "plan" => {
                tracing::info!(target: "acp::plan", "plan update received");
                false
            }
            "agent_thought_chunk" => {
                if let Some(text) = update["content"]["text"].as_str() {
                    tracing::debug!(target: "acp::thought", "{text}");
                }
                false
            }
            "available_commands_update" => {
                // Advertised slash commands (ACP slash-commands extension).
                // Logged for observability; UI surfacing is a follow-up.
                let names: Vec<&str> = update["availableCommands"]
                    .as_array()
                    .map(|cmds| cmds.iter().filter_map(|c| c["name"].as_str()).collect())
                    .unwrap_or_default();
                tracing::info!(
                    target: "acp::update",
                    "available_commands_update: {} commands [{}]",
                    names.len(),
                    names.join(", ")
                );
                false
            }
            "session_info_update" => {
                // Both goose and buzz-agent emit `session_info_update` with
                // `_meta.goose.activeRunId`: the id of the currently-active
                // prompt run, or `null` when the run has cleared. Other agents
                // don't emit this field; for them `active_run_id` stays `None`
                // and steer callers will fall back to cancel+merge.
                //
                // Per the ACP `SessionInfoUpdate` schema, `_meta` is a field
                // on the update object itself — nested inside `update`, not
                // alongside it at the params level. Goose and buzz-agent both
                // emit it at `params.update._meta.goose.activeRunId`.
                let meta = msg["params"]["update"]
                    .get("_meta")
                    .and_then(|m| m.get("goose"));
                if let Some(goose_meta) = meta {
                    match goose_meta.get("activeRunId") {
                        Some(serde_json::Value::String(run_id)) => {
                            tracing::debug!(
                                target: "acp::update",
                                "session_info_update: activeRunId={run_id}"
                            );
                            self.active_run_id = Some(run_id.clone());
                        }
                        Some(serde_json::Value::Null) => {
                            tracing::debug!(
                                target: "acp::update",
                                "session_info_update: activeRunId cleared"
                            );
                            self.active_run_id = None;
                        }
                        // Missing or non-string/null — leave state untouched.
                        _ => {}
                    }
                }
                false
            }
            "usage_update" => {
                self.handle_standard_usage_update(msg);
                false
            }
            "keepalive" => false,
            other => {
                tracing::debug!(target: "acp::update", "session/update: {other}");
                false
            }
        }
    }

    /// Record the standard ACP cumulative cost notification when emitted by
    /// Claude. Unlike Goose's payload, `used`/`size` are context occupancy and
    /// are intentionally not mapped to token accounting.
    fn handle_standard_usage_update(&mut self, msg: &serde_json::Value) {
        if self.standard_adapter != Some(StandardAdapterKind::Claude) {
            return;
        }
        let session_id = match msg
            .pointer("/params/sessionId")
            .and_then(serde_json::Value::as_str)
        {
            Some(session_id) => session_id,
            None => return,
        };
        let cost = match msg
            .pointer("/params/update/cost/amount")
            .and_then(serde_json::Value::as_f64)
        {
            Some(cost) => cost,
            None => return,
        };
        self.standard_usage.record_cost(session_id, cost);
    }

    /// Parse a `_goose/unstable/session/update` notification and record the
    /// usage snapshot in the per-session tracker.
    ///
    /// Silently ignores malformed or non-`usage_update` variants — the
    /// notification is best-effort observability data, not a protocol
    /// requirement. Failures are logged at debug level.
    fn handle_goose_usage_update(&mut self, msg: &serde_json::Value) {
        use crate::usage::{GooseSessionUpdateNotification, GooseSessionUpdateVariant};
        let params = match msg.get("params") {
            Some(p) => p,
            None => {
                tracing::debug!(
                    target: "acp::usage",
                    "_goose/unstable/session/update: missing params"
                );
                return;
            }
        };
        match serde_json::from_value::<GooseSessionUpdateNotification>(params.clone()) {
            Ok(notif) => {
                if let GooseSessionUpdateVariant::UsageUpdate(payload) = &notif.update {
                    tracing::debug!(
                        target: "acp::usage",
                        session_id = %notif.session_id,
                        input = ?payload.accumulated_input_tokens,
                        output = ?payload.accumulated_output_tokens,
                        // A subset of `input`, logged so downstream accounting can
                        // price it at the provider's cached rate. Always emitted,
                        // including as 0, so a parser can tell "no cache hits"
                        // apart from "this build predates the field".
                        cached = payload.accumulated_cached_input_tokens,
                        "goose usage update"
                    );
                    self.goose_usage.record(&notif.session_id, payload);
                }
            }
            Err(e) => {
                tracing::debug!(
                    target: "acp::usage",
                    "_goose/unstable/session/update: deserialization error: {e}"
                );
            }
        }
    }

    /// Handle a `session/request_permission` request from the agent.
    ///
    /// Dispatches based on the resolved permission policy:
    /// - `reject` — deny via `reject_once`/`cancelled` (byte-for-byte old behaviour).
    /// - `allow`  — auto-select the unique validated `allow_once` option; fail closed.
    /// - `ask`    — register in the pending map, emit an actionable frame, and return.
    ///   The read loop's decision arm (added to `select!`) delivers the owner
    ///   decision. This call is intentionally **non-blocking** for `ask`;
    ///   the actual response is written asynchronously via the decision arm.
    ///
    /// **Admission preflight (always runs before any policy dispatch):**
    /// options nonempty, count ≤ PERMISSION_OPTIONS_MAX, every optionId unique +
    /// nonempty, required kind/name fields present, no duplicate live requestId,
    /// plaintext size ≤ OBSERVER_MAX_PLAINTEXT_LEN. Fail → immediate denial + emit
    /// with `actionable: false`.
    ///
    /// Under `ask`, the generic pre-dispatch `acp_read` (acp.rs:1697 seam) is
    /// **suppressed** for permission requests; this method emits the single
    /// post-preflight enveloped frame instead.
    ///
    /// Returns `Ok(true)` when the caller should suppress the normal `acp_read` emit
    /// (i.e. this method already emitted the enveloped frame), `Ok(false)` otherwise.
    pub(crate) async fn handle_permission_request(
        &mut self,
        msg: &serde_json::Value,
        // Hard deadline for the current turn. Used to bound per-request ask timeouts.
        hard_deadline: tokio::time::Instant,
    ) -> Result<bool, AcpError> {
        // Extract id as a Value — JSON-RPC 2.0 allows both numeric and string IDs.
        let id = msg
            .get("id")
            .cloned()
            .ok_or_else(|| AcpError::Protocol("permission request missing id".into()))?;

        let options = match msg["params"]["options"].as_array() {
            Some(o) => o.clone(),
            None => {
                // Missing options — emit non-actionable frame and deny.
                let reason = "missing or non-array options field";
                tracing::warn!(target: "acp::permission", "{reason}, id={id}");
                let nonce = new_permission_nonce();
                self.emit_permission_read_non_actionable(&id, msg, &nonce, reason);
                let response = permission_denial_response(&id, &[])?;
                self.finish_permission_sync(&id, &nonce, "rejected", response)
                    .await?;
                return Ok(true);
            }
        };

        // ── Admission preflight ────────────────────────────────────────────────
        let preflight_result = run_admission_preflight(
            &id,
            &options,
            msg,
            self.permission_config.policy,
            AskGates {
                // Check for duplicate live requestId under ask.
                is_duplicate_id: if matches!(self.permission_config.policy, PermissionPolicy::Ask) {
                    let id_str = id.to_string();
                    self.pending_permissions.contains_key(&id_str)
                } else {
                    false
                },
                is_map_at_cap: if matches!(self.permission_config.policy, PermissionPolicy::Ask) {
                    // Count every live entry — including `Publishing` — against the
                    // cap. A broken/malicious adapter that never triggers the ACK
                    // could otherwise accumulate unbounded Publishing entries/cards
                    // below the cap; counting them here bounds the total live set.
                    self.pending_permissions
                        .values()
                        .filter(|e| {
                            matches!(
                                e.state,
                                PermissionEntryState::Publishing
                                    | PermissionEntryState::Pending
                                    | PermissionEntryState::Writing
                            )
                        })
                        .count()
                        >= PERMISSION_MAP_CAP
                } else {
                    false
                },
                // A single sentinel ACK receiver slot is shared across publishes,
                // so at most one entry may be in `Publishing` at a time. A new Ask
                // request while a publish is still in flight is denied (fail closed).
                is_publish_in_flight: if matches!(
                    self.permission_config.policy,
                    PermissionPolicy::Ask
                ) {
                    self.pending_permissions
                        .values()
                        .any(|e| matches!(e.state, PermissionEntryState::Publishing))
                } else {
                    false
                },
            },
            (&self.observer_context, self.observer_agent_index),
        );

        if let Err(reason) = preflight_result {
            tracing::warn!(target: "acp::permission", "preflight failed: {reason}, id={id}");
            let nonce = new_permission_nonce();
            self.emit_permission_read_non_actionable(&id, msg, &nonce, &reason);
            let response = permission_denial_response(&id, &options)?;
            self.finish_permission_sync(&id, &nonce, "rejected", response)
                .await?;
            return Ok(true);
        }
        // ── Preflight passed ───────────────────────────────────────────────────

        tracing::debug!(
            target: "acp::permission",
            "session/request_permission id={id}, {} options, policy={}",
            options.len(),
            self.permission_config.policy
        );

        match self.permission_config.policy {
            PermissionPolicy::Reject => {
                // Byte-for-byte old behaviour: deny, track pending id for cancel.
                self.pending_permission_id = Some(id.clone());
                self.permission_responded = false;

                // For reject, the caller already emitted acp_read unconditionally;
                // emit a non-actionable authorization envelope alongside.
                let nonce = new_permission_nonce();
                self.emit_permission_read_with_nonce(
                    &id,
                    msg,
                    &nonce,
                    false,
                    Some("policy=reject"),
                );

                let response = permission_denial_response(&id, &options)?;
                self.finish_permission_sync(&id, &nonce, "rejected", response)
                    .await?;
                self.permission_responded = true;
                self.pending_permission_id = None;
                Ok(true)
            }
            PermissionPolicy::Allow => {
                // Auto-select the unique allow_once option; fail closed otherwise.
                self.pending_permission_id = Some(id.clone());
                self.permission_responded = false;

                match select_allow_once(&options) {
                    Ok(option_id) => {
                        tracing::info!(
                            target: "acp::permission",
                            "allow: selecting allow_once optionId={option_id:?} for id={id}"
                        );
                        let nonce = new_permission_nonce();
                        // Emit enveloped acp_read (non-actionable: auto-approved).
                        self.emit_permission_read_with_nonce(
                            &id,
                            msg,
                            &nonce,
                            false,
                            Some("policy=allow; auto-approved"),
                        );
                        let response = permission_response_selected(&id, &option_id);
                        self.finish_permission_sync(&id, &nonce, "allowed", response)
                            .await?;
                        self.permission_responded = true;
                        self.pending_permission_id = None;
                    }
                    Err(reason) => {
                        // Fail closed.
                        tracing::warn!(
                            target: "acp::permission",
                            "allow: fail closed — {reason}, id={id}"
                        );
                        let nonce = new_permission_nonce();
                        self.emit_permission_read_with_nonce(
                            &id,
                            msg,
                            &nonce,
                            false,
                            Some(&format!("policy=allow; fail closed: {reason}")),
                        );
                        let response = permission_denial_response(&id, &options)?;
                        self.finish_permission_sync(&id, &nonce, "allow_failed_closed", response)
                            .await?;
                        self.permission_responded = true;
                        self.pending_permission_id = None;
                    }
                }
                Ok(true)
            }
            PermissionPolicy::Ask => {
                // Availability gate (spec §10): `ask` requires both an active observer
                // and a known owner. Without either, downgrade to `reject` with a loud
                // warning — never sideways to `allow`.
                let observer_active = self.observer.is_some();
                if !observer_active || !self.owner_pubkey_known {
                    tracing::warn!(
                        target: "acp::permission",
                        "ask policy unavailable (observer={}, owner_known={}) — downgrading to reject for id={id}",
                        observer_active,
                        self.owner_pubkey_known
                    );
                    // Fall through to the Reject arm's logic.
                    self.pending_permission_id = Some(id.clone());
                    self.permission_responded = false;
                    let nonce = new_permission_nonce();
                    self.emit_permission_read_with_nonce(
                        &id,
                        msg,
                        &nonce,
                        false,
                        Some("policy=ask unavailable (no observer/owner); downgraded to reject"),
                    );
                    let response = permission_denial_response(&id, &options)?;
                    self.finish_permission_sync(&id, &nonce, "rejected", response)
                        .await?;
                    self.permission_responded = true;
                    self.pending_permission_id = None;
                    return Ok(true);
                }

                // Register in the pending map and emit the actionable frame.
                // The read loop's decision arm delivers the response asynchronously.
                let id_str = id.to_string();
                let nonce = new_permission_nonce();

                // D7-final admission check: `ask` only proceeds when a relay
                // publisher is available AND the turn was initiated by the agent
                // owner. Without either, deny synchronously with zero card events.
                // There is no bypass for sessions without relay context — a request
                // that cannot present a card to the owner is always denied.
                let owner_initiated = match (
                    &self.relay_publisher,
                    &self.turn_initiator_pubkey,
                    &self.agent_owner_pubkey_hex,
                ) {
                    (Some(_), Some(initiator), Some(owner_hex)) => initiator.to_hex() == *owner_hex,
                    // No publisher, or owner/initiator not set: deny.
                    _ => false,
                };
                if !owner_initiated {
                    tracing::warn!(
                        target: "acp::permission",
                        "ask D7-final: turn not owner-initiated (or no relay context) — downgrading to reject for id={id}"
                    );
                    self.pending_permission_id = Some(id.clone());
                    self.permission_responded = false;
                    let nonce = new_permission_nonce();
                    self.emit_permission_read_with_nonce(
                        &id,
                        msg,
                        &nonce,
                        false,
                        Some("policy=ask; D7-final: non-owner turn or no relay context; downgraded to reject"),
                    );
                    let response = permission_denial_response(&id, &options)?;
                    self.finish_permission_sync(&id, &nonce, "rejected", response)
                        .await?;
                    self.permission_responded = true;
                    self.pending_permission_id = None;
                    return Ok(true);
                }

                // Select exactly the two ruled card actions (allow_once /
                // reject_once). Fail closed — deny with zero card events — if the
                // adapter does not offer exactly one of each. This is the single
                // enforcement point that keeps a forbidden option (e.g.
                // `allow_always`) from ever reaching the owner as a button.
                let card_actions = match select_card_actions(&options) {
                    Ok(actions) => actions,
                    Err(reason) => {
                        tracing::warn!(
                            target: "acp::permission",
                            "ask: cannot build two-action card ({reason}) — downgrading to reject for id={id}"
                        );
                        self.pending_permission_id = Some(id.clone());
                        self.permission_responded = false;
                        let deny_nonce = new_permission_nonce();
                        self.emit_permission_read_with_nonce(
                            &id,
                            msg,
                            &deny_nonce,
                            false,
                            Some(&format!("policy=ask; fail closed: {reason}")),
                        );
                        let response = permission_denial_response(&id, &options)?;
                        self.finish_permission_sync(&id, &deny_nonce, "rejected", response)
                            .await?;
                        self.permission_responded = true;
                        self.pending_permission_id = None;
                        return Ok(true);
                    }
                };

                // Per-request deadline: min(now + 300s, turn hard deadline).
                let ask_deadline = tokio::time::Instant::now()
                    + std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS);
                let entry_deadline = ask_deadline.min(hard_deadline);
                // Compute and store expiry_unix_secs once — the envelope, the
                // pending payload, and the resolved payload all reuse this value
                // (no recompute drift). The desktop bounds its
                // retransmit-until-acked loop by the envelope's copy.
                let expiry_unix_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    + entry_deadline
                        .checked_duration_since(tokio::time::Instant::now())
                        .unwrap_or_default()
                        .as_secs();

                // Emit the single enveloped acp_read — suppresses the caller's
                // generic emit via the Ok(true) return.
                self.observe_authorized(
                    "acp_read",
                    AuthorizationEnvelope {
                        request_nonce: nonce.clone(),
                        actionable: true,
                        reason: None,
                        expires_at: Some(expiry_unix_secs),
                    },
                    msg.clone(),
                );

                // Build and sign the kind-9 sentinel event ONCE before inserting
                // the entry — the resolved edit retransmits the same signed event
                // on retry, matching the spec requirement.
                // Extract a human-readable description from the real producer
                // shapes — see `description_from_request_permission` for the
                // full precedence rationale.
                let description_owned: Option<String> = description_from_request_permission(msg);
                let sentinel_event = {
                    let keys_opt = self.agent_relay_keys.clone();
                    let channel_id_opt = self.sentinel_channel_id;
                    let owner_hex_opt = self.agent_owner_pubkey_hex.clone();
                    let turn_id = self.observer_context.turn_id.clone().unwrap_or_default();
                    let session_id_owned = self.observer_context.session_id.clone();
                    let reply_id = self.sentinel_thread_reply_id.clone();

                    keys_opt.zip(channel_id_opt).zip(owner_hex_opt).and_then(
                        |((keys, channel_id), owner_hex)| {
                            let content = build_sentinel_pending_payload(
                                &nonce,
                                &card_actions,
                                expiry_unix_secs,
                                session_id_owned.as_deref(),
                                &turn_id,
                                description_owned.as_deref(),
                            )?;
                            build_kind9_sentinel(
                                &keys,
                                channel_id,
                                &owner_hex,
                                reply_id.as_deref(),
                                &content,
                            )
                        },
                    )
                };

                // Insert entry as Publishing. The relay ACK transitions it to Pending.
                // If the sentinel event could not be built (keys/channel absent even
                // after the D7 check passes — shouldn't happen in production), skip
                // the ACK path and fall through to deny.
                let publisher_opt = self.relay_publisher.clone();
                match (sentinel_event, publisher_opt) {
                    (Some(event), Some(publisher)) => {
                        let sentinel_id = event.id.to_hex();
                        self.pending_permissions.insert(
                            id_str.clone(),
                            PermissionEntry {
                                nonce: nonce.clone(),
                                options_snapshot: options.clone(),
                                card_actions,
                                state: PermissionEntryState::Publishing,
                                deadline: entry_deadline,
                                expiry_unix_secs,
                                // Store the event ID at build time so the resolved edit
                                // can reference it even if the ACK arm hasn't fired yet.
                                sentinel_event_id: Some(sentinel_id),
                                early_decision: None,
                                description: description_owned.clone(),
                            },
                        );
                        // Publish deadline: min(fixed publish timeout, entry deadline).
                        let publish_deadline = (tokio::time::Instant::now()
                            + std::time::Duration::from_secs(SENTINEL_PUBLISH_TIMEOUT_SECS))
                        .min(entry_deadline);
                        match publisher
                            .register_publish_ack(event, publish_deadline)
                            .await
                        {
                            Ok(ack_rx) => {
                                // Spawn a task that awaits the relay ACK and forwards
                                // the result via mpsc to the read loop's select! arm.
                                //
                                // The background relay task owns the `publish_deadline`
                                // — it sweeps expired waiters with `Uncertain` so
                                // `ack_rx` always resolves before the deadline. No
                                // caller-side timeout is needed here.
                                let (ack_result_tx, ack_result_rx) = tokio::sync::mpsc::channel(1);
                                let entry_id_for_task = id_str.clone();
                                tokio::spawn(async move {
                                    let outcome =
                                        ack_rx.await.unwrap_or(crate::relay::AckOutcome::Uncertain);
                                    // Best-effort send: if the read loop already
                                    // cleaned up, the send fails harmlessly.
                                    let _ = ack_result_tx.send((entry_id_for_task, outcome)).await;
                                });
                                self.sentinel_ack_result_rx = Some(ack_result_rx);
                            }
                            Err(_) => {
                                // Command channel closed — relay unavailable.
                                // Remove the Publishing entry and deny synchronously.
                                self.pending_permissions.remove(&id_str);
                                tracing::warn!(
                                    target: "acp::permission",
                                    "sentinel publish channel closed for id={id} — downgrading to reject"
                                );
                                self.pending_permission_id = Some(id.clone());
                                self.permission_responded = false;
                                let deny_nonce = new_permission_nonce();
                                self.emit_permission_read_with_nonce(
                                    &id,
                                    msg,
                                    &deny_nonce,
                                    false,
                                    Some("policy=ask; relay channel closed; downgraded to reject"),
                                );
                                let response = permission_denial_response(&id, &options)?;
                                self.finish_permission_sync(&id, &deny_nonce, "rejected", response)
                                    .await?;
                                self.permission_responded = true;
                                self.pending_permission_id = None;
                            }
                        }
                    }
                    _ => {
                        // Keys or channel absent despite D7 passing — deny.
                        tracing::warn!(
                            target: "acp::permission",
                            "sentinel event could not be built for id={id} — downgrading to reject"
                        );
                        self.pending_permission_id = Some(id.clone());
                        self.permission_responded = false;
                        let deny_nonce = new_permission_nonce();
                        self.emit_permission_read_with_nonce(
                            &id,
                            msg,
                            &deny_nonce,
                            false,
                            Some("policy=ask; sentinel build failed; downgraded to reject"),
                        );
                        let response = permission_denial_response(&id, &options)?;
                        self.finish_permission_sync(&id, &deny_nonce, "rejected", response)
                            .await?;
                        self.permission_responded = true;
                        self.pending_permission_id = None;
                    }
                }

                // Do NOT set pending_permission_id for ask — the map is the
                // sole source of truth. The legacy single-id slot is only used
                // by reject/allow (synchronous paths).
                Ok(true)
            }
        }
    }

    /// Emit a non-actionable `acp_read` authorization frame for a permission request.
    ///
    /// The caller is responsible for generating the nonce and passing the same
    /// value to the corresponding `finish_permission_sync` call so that both the
    /// `acp_read` and `acp_write` telemetry frames share one nonce — required for
    /// Desktop's nonce-only correlation to retire the card.
    fn emit_permission_read_non_actionable(
        &self,
        id: &serde_json::Value,
        msg: &serde_json::Value,
        nonce: &str,
        reason: &str,
    ) {
        self.observe_authorized(
            "acp_read",
            AuthorizationEnvelope {
                request_nonce: nonce.to_string(),
                actionable: false,
                reason: Some(reason.to_string()),
                expires_at: None,
            },
            msg.clone(),
        );
        tracing::debug!(target: "acp::permission", "non-actionable permission read id={id}");
    }

    /// Emit an `acp_read` with an authorization envelope.
    ///
    /// Only ever called with `actionable: false` (fail-closed / auto-deny
    /// paths); the single actionable emit builds its envelope inline with the
    /// card expiry. `expires_at` is therefore always `None` here — no owner
    /// decision is awaited on these frames.
    fn emit_permission_read_with_nonce(
        &self,
        _id: &serde_json::Value,
        msg: &serde_json::Value,
        nonce: &str,
        actionable: bool,
        reason: Option<&str>,
    ) {
        self.observe_authorized(
            "acp_read",
            AuthorizationEnvelope {
                request_nonce: nonce.to_string(),
                actionable,
                reason: reason.map(str::to_string),
                expires_at: None,
            },
            msg.clone(),
        );
    }

    /// Parse a completed prompt response and retain its optional per-turn usage.
    fn parse_prompt_response(
        &mut self,
        session_id: &str,
        result: &serde_json::Value,
    ) -> Result<StopReason, AcpError> {
        let stop_reason = self.parse_stop_reason(result)?;
        if let Some(adapter) = self.standard_adapter {
            match serde_json::from_value::<PromptResponseUsage>(result["usage"].clone()) {
                Ok(usage) => self
                    .standard_usage
                    .record_prompt_usage(session_id, usage, adapter),
                Err(_) if result.get("usage").is_some() => tracing::debug!(
                    target: "acp::usage",
                    "session/prompt response contained malformed standard usage"
                ),
                Err(_) => {}
            }
        }
        Ok(stop_reason)
    }

    /// Parse `stopReason` from a `session/prompt` result value.
    fn parse_stop_reason(&self, result: &serde_json::Value) -> Result<StopReason, AcpError> {
        let raw = result["stopReason"].as_str().ok_or_else(|| {
            AcpError::Protocol("session/prompt response missing stopReason".into())
        })?;
        StopReason::from_str(raw)
            .ok_or_else(|| AcpError::Protocol(format!("unknown stopReason: {raw:?}")))
    }
}

/// Build `session/prompt` params from one or more text content blocks.
fn build_prompt_params(session_id: &str, prompt_blocks: &[&str]) -> serde_json::Value {
    let blocks: Vec<serde_json::Value> = prompt_blocks
        .iter()
        .map(|text| serde_json::json!({ "type": "text", "text": text }))
        .collect();
    serde_json::json!({
        "sessionId": session_id,
        "prompt": blocks,
    })
}

/// Build `_goose/unstable/session/steer` params from one or more text
/// content blocks plus the freshest `expectedRunId`.
///
/// Wire shape:
/// ```json
/// { "sessionId": "...", "expectedRunId": "...", "prompt": [{"type":"text","text":"..."}, ...] }
/// ```
///
/// Called from the read-loop steer arm at write time so `expectedRunId`
/// matches goose's *current* run (it advances on each `session/update`).
/// See [`crate::pool::SteerRequest`] for why this is the read loop's job
/// and not the main loop's.
fn build_goose_steer_params(
    session_id: &str,
    expected_run_id: &str,
    prompt_blocks: &[&str],
) -> serde_json::Value {
    serde_json::json!({
        "sessionId": session_id,
        "expectedRunId": expected_run_id,
        "prompt": steer_prompt_blocks(prompt_blocks),
    })
}

/// Build the params for an [`ACP_STEER_METHOD`] request.
///
/// Wire shape:
/// ```json
/// { "sessionId": "...", "prompt": [{"type":"text","text":"..."}, ...] }
/// ```
///
/// Deliberately carries **no** `expectedRunId`: the cross-adapter method
/// steers whatever turn is currently running and neither claude-agent-acp nor
/// codex-acp emits a run id to target.
fn build_acp_steer_params(session_id: &str, prompt_blocks: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "sessionId": session_id,
        "prompt": steer_prompt_blocks(prompt_blocks),
    })
}

/// Render steer body strings as ACP `text` content blocks. Shared by both
/// steer transports so the prompt shape cannot drift between them.
fn steer_prompt_blocks(prompt_blocks: &[&str]) -> Vec<serde_json::Value> {
    prompt_blocks
        .iter()
        .map(|text| serde_json::json!({ "type": "text", "text": text }))
        .collect()
}

/// Build a JSON-RPC permission response with `outcome: "selected"`.
fn permission_response_selected(id: &serde_json::Value, option_id: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "outcome": { "outcome": "selected", "optionId": option_id } }
    })
}

/// Build a JSON-RPC permission response with `outcome: "cancelled"`.
fn permission_response_cancelled(id: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "outcome": { "outcome": "cancelled" } }
    })
}

/// Choose the fail-closed response to a `session/request_permission` request.
///
/// Buzz has no human permission prompt in this harness, so selecting
/// `allow_once` would turn any admitted prompt into an implicit approval.
/// Prefer the adapter's `reject_once` option — matched by `kind`, never by a
/// hardcoded `optionId` — and fall back to the protocol's cancelled outcome for
/// adapters that do not offer one. Both answers deny.
///
/// Kept free of the client so the decision is testable without an agent
/// subprocess: `AcpClient` owns a real `Child` and its stdio pipes.
fn permission_denial_response(
    id: &serde_json::Value,
    options: &[serde_json::Value],
) -> Result<serde_json::Value, AcpError> {
    let reject_once = options
        .iter()
        .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("reject_once"));

    let Some(opt) = reject_once else {
        tracing::warn!(
            target: "acp::permission",
            "no reject_once option found in permission request id={id}, cancelling"
        );
        return Ok(permission_response_cancelled(id));
    };

    let Some(option_id) = opt["optionId"].as_str().filter(|s| !s.is_empty()) else {
        // reject_once found but optionId is missing or empty — malformed request;
        // fall back to `cancelled` rather than returning a Protocol error so the
        // adapter still receives a valid JSON-RPC response.
        tracing::warn!(
            target: "acp::permission",
            "reject_once option has missing or empty optionId for id={id}, cancelling"
        );
        return Ok(permission_response_cancelled(id));
    };
    tracing::info!(
        target: "acp::permission",
        "rejecting permission id={id} with reject_once optionId={option_id:?}"
    );
    Ok(permission_response_selected(id, option_id))
}

/// Generate a cryptographically random, URL-safe nonce string.
///
/// Used as the `requestNonce` in [`crate::observer::AuthorizationEnvelope`].
/// The nonce is single-use and bound to a specific permission request.
fn new_permission_nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Frozen sentinel byte bounds, shared verbatim with the Desktop parser
/// (`MAX_STRING_BYTES` / `MAX_CONTENT_BYTES` in `permissionRequest.ts`).
///
/// The producer and parser MUST agree on both the values AND the unit — UTF-8
/// bytes — so a card the harness emits always parses and a card the parser
/// accepts is always one the harness could emit. Measuring in Rust `char`
/// scalars vs JavaScript UTF-16 code units (the prior split) let a producer-
/// valid multibyte label be rejected by the parser, publishing a card the
/// desktop renders as raw JSON until timeout.
///
/// `SENTINEL_STRING_MAX_BYTES` bounds every untrusted string leaf: labels,
/// each `optionId`, `requestNonce`, `sessionId`, `turnId`, and `chosenOptionId`.
/// `SENTINEL_CONTENT_MAX_BYTES` bounds the total serialized sentinel content.
const SENTINEL_STRING_MAX_BYTES: usize = 200;
const SENTINEL_CONTENT_MAX_BYTES: usize = 4096;

/// Truncate `s` to at most `max_bytes` UTF-8 bytes on a char boundary.
///
/// Labels are lossy display strings, so an over-long one is truncated (not
/// rejected). Truncating on a char boundary guarantees valid UTF-8 and a byte
/// length the Desktop parser — which bounds the same field in bytes — accepts.
fn truncate_to_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Truncate `s` to at most `max_bytes` UTF-8 bytes using a head + "…" + tail
/// layout that preserves both the start and the end of the string.
///
/// This keeps distinguishing suffixes visible even when many strings share a
/// long common prefix, which head-only truncation collapses into identical
/// output. The ellipsis is the UTF-8 character U+2026 (3 bytes); head and tail
/// together fill the remaining budget. If `max_bytes < 5` (3-byte ellipsis +
/// at least one byte each side) the function falls back to head-only
/// truncation so it always fits within the budget.
fn truncate_to_bytes_head_tail(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    const ELLIPSIS: &str = "\u{2026}"; // 3 UTF-8 bytes
    const ELLIPSIS_BYTES: usize = 3;
    // Need room for at least 1 head byte + ellipsis + 1 tail byte.
    if max_bytes < 1 + ELLIPSIS_BYTES + 1 {
        return truncate_to_bytes(s, max_bytes);
    }
    let available = max_bytes - ELLIPSIS_BYTES;
    // Split evenly; tail gets the extra byte when available is odd.
    let head_budget = available / 2;
    let tail_budget = available - head_budget;

    // Snap head to a char boundary (walk backward from head_budget).
    let mut head_end = head_budget;
    while head_end > 0 && !s.is_char_boundary(head_end) {
        head_end -= 1;
    }
    // Snap tail to a char boundary (walk forward from s.len() - tail_budget).
    let tail_start_raw = s.len().saturating_sub(tail_budget);
    let mut tail_start = tail_start_raw;
    while tail_start < s.len() && !s.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    // Guard: if the boundaries crossed (very short string with multi-byte
    // chars), fall back to head-only.
    if head_end >= tail_start {
        return truncate_to_bytes(s, max_bytes);
    }
    format!("{}{}{}", &s[..head_end], ELLIPSIS, &s[tail_start..])
}

/// Enforce the frozen sentinel string bound on one field.
///
/// Returns `None` (fail closed) when `value` exceeds `SENTINEL_STRING_MAX_BYTES`
/// UTF-8 bytes. `sessionId` is the load-bearing case: it comes straight from the
/// adapter's unbounded `session/new` response, so an oversized adapter session
/// ID must abort sentinel construction rather than publish a card the Desktop
/// parser rejects (which would render as raw JSON until timeout). Labels are the
/// exception — they are truncated at the source, never passed here.
fn check_sentinel_field(field: &str, value: &str) -> Option<()> {
    if value.len() > SENTINEL_STRING_MAX_BYTES {
        tracing::warn!(
            target: "acp::permission",
            "sentinel field {field} exceeds {SENTINEL_STRING_MAX_BYTES} bytes ({}) — failing closed",
            value.len()
        );
        return None;
    }
    Some(())
}

/// Serialize a sentinel payload and enforce the total-content byte bound.
///
/// Returns `None` (fail closed) when serialization fails or the serialized
/// content exceeds `SENTINEL_CONTENT_MAX_BYTES`. This is the single total-size
/// gate the Desktop parser mirrors (`MAX_CONTENT_BYTES`), so producer and parser
/// can never disagree on whether a given card is admissible.
fn serialize_bounded_sentinel(payload: &serde_json::Value) -> Option<String> {
    let content = serde_json::to_string(payload).ok()?;
    if content.len() > SENTINEL_CONTENT_MAX_BYTES {
        tracing::warn!(
            target: "acp::permission",
            "sentinel content exceeds {SENTINEL_CONTENT_MAX_BYTES} bytes ({}) — failing closed",
            content.len()
        );
        return None;
    }
    Some(content)
}

/// The two card actions surfaced to the owner: the validated `allow_once` and
/// `reject_once` options, in that fixed order. Built by [`select_card_actions`]
/// so the sentinel can never advertise a third (e.g. `allow_always`) action.
#[derive(Debug, Clone)]
struct CardActions {
    allow: serde_json::Value,
    reject: serde_json::Value,
}

impl CardActions {
    /// The `optionId` of the validated `allow_once` action.
    fn allow_id(&self) -> &str {
        self.allow
            .get("optionId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
    }

    /// The `optionId` of the validated `reject_once` action.
    fn reject_id(&self) -> &str {
        self.reject
            .get("optionId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
    }

    /// True iff `option_id` is exactly one of the two ruled card actions.
    /// The read loop gates owner decisions on this so a decision can never
    /// select a forbidden option (e.g. `allow_always`) that an adapter offered
    /// but the card never surfaced.
    fn accepts(&self, option_id: &str) -> bool {
        option_id == self.allow_id() || option_id == self.reject_id()
    }
}

/// Build the ordered `[optionIds, labels]` pair for a sentinel from the two
/// validated card actions. Order is fixed: allow first, reject second.
fn sentinel_option_fields(actions: &CardActions) -> (Vec<serde_json::Value>, serde_json::Value) {
    let mut option_ids = Vec::with_capacity(2);
    let mut labels = serde_json::Map::with_capacity(2);
    for opt in [&actions.allow, &actions.reject] {
        // optionId presence/non-emptiness was validated by select_card_actions.
        let id = opt
            .get("optionId")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let name = opt.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let capped = truncate_to_bytes(name, SENTINEL_STRING_MAX_BYTES);
        option_ids.push(serde_json::Value::String(id.to_string()));
        labels.insert(id.to_string(), serde_json::Value::String(capped));
    }
    (option_ids, labels.into())
}

/// Combined byte budget for the entire description string (title + argument
/// context). Matches `SENTINEL_STRING_MAX_BYTES` so the value is already
/// within the per-field sentinel cap, and the final
/// `build_sentinel_pending_payload` cap is a no-op identity for well-formed
/// descriptions. Keeping it at 200 leaves the context fragment at most
/// `200 - len(title) - 3` bytes (for the `(…)` wrapper) when title is short.
const DESCRIPTION_COMBINED_MAX_BYTES: usize = 200;

/// Prefix characters that suggest a value is a secret or credential.
/// Checked against JSON object keys (lowercased) to decide whether a value
/// must be redacted before appending it to a card description.
///
/// Design: narrow allowlist of truly suspicious prefixes rather than a wide
/// blocklist so legitimate fields (e.g. `token_count`, `pathname`) are not
/// inadvertently suppressed. The check is recursive — any nested object whose
/// key matches is also redacted. Secret-shaped values are replaced with
/// `"<redacted>"` regardless of their actual type.
const SECRET_KEY_PREFIXES: &[&str] = &[
    "secret",
    "password",
    "passwd",
    "token",
    "apikey",
    "api_key",
    "auth",
    "credential",
    "private",
];

/// Return `true` when `key` (lowercased) suggests a secret/credential value
/// that should be redacted from card descriptions.
fn is_secret_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    SECRET_KEY_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

/// Produce a compact, human-readable argument context string from a
/// `rawInput` JSON object.
///
/// Precedence within the object:
///
/// 1. `command` — verbatim shell command string (already legible).
/// 2. File/path keys (`file`, `path`, `filename`, `filepath`, `target`,
///    `source`, `destination`, `url`) — most relevant for file-access tools.
/// 3. `cwd` — working-directory context.
/// 4. `reason` — rationale text provided by the caller.
/// 5. Compact JSON fallback: all non-secret scalar fields serialised as a
///    JSON object, e.g. `{"n":3,"mode":"fast"}`.
///
/// Secret-bearing keys (see `is_secret_key`) are replaced with `"<redacted>"`
/// at every level before the fallback serialisation; they are also excluded
/// from the named-key paths (a field named `password` is never surfaced).
///
/// Returns `None` when `raw_input` is not a JSON object, is null, or contains
/// no extractable non-secret fields.
fn summarize_raw_input(raw_input: &serde_json::Value) -> Option<String> {
    let obj = raw_input.as_object()?;

    // --- Priority 1: shell command ---
    if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
        if !cmd.is_empty() && !is_secret_key("command") {
            return Some(cmd.to_string());
        }
    }

    // --- Priority 2: file / path keys ---
    const FILE_KEYS: &[&str] = &[
        "file",
        "path",
        "filename",
        "filepath",
        "target",
        "source",
        "destination",
        "url",
    ];
    for key in FILE_KEYS {
        if is_secret_key(key) {
            continue;
        }
        if let Some(val) = obj.get(*key).and_then(|v| v.as_str()) {
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }

    // --- Priority 3: cwd ---
    if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
        if !cwd.is_empty() && !is_secret_key("cwd") {
            return Some(cwd.to_string());
        }
    }

    // --- Priority 4: reason ---
    if let Some(reason) = obj.get("reason").and_then(|v| v.as_str()) {
        if !reason.is_empty() && !is_secret_key("reason") {
            return Some(reason.to_string());
        }
    }

    // --- Priority 5: compact JSON fallback (scalars only, secrets redacted) ---
    let mut sanitised = serde_json::Map::new();
    for (k, v) in obj {
        if is_secret_key(k) {
            sanitised.insert(
                k.clone(),
                serde_json::Value::String("<redacted>".to_string()),
            );
        } else if v.is_string() || v.is_number() || v.is_boolean() {
            sanitised.insert(k.clone(), v.clone());
        }
        // Skip null, arrays, nested objects in the fallback.
    }
    if sanitised.is_empty() {
        return None;
    }
    serde_json::to_string(&sanitised).ok()
}

/// Extract a truthful, bounded operation description from a
/// `session/request_permission` JSON-RPC message, trying real producer shapes
/// in priority order:
///
/// 1. `params.title` — buzz-agent v2 top-level string (= `call.name`).
/// 2. `params.subject.toolCall.title` — v2 nested fallback (same value).
/// 3. `params.toolCall.title` — buzz-agent v1 / codex-acp permissions-request.
/// 4. `params.toolCall.rawInput.command` — codex-acp v1.1.7 command execution.
/// 5. `params._meta.codex.params.reason` — codex-acp v1.1.7 file-change.
///
/// For paths 1–3 (buzz-agent v1/v2), the function also inspects the `rawInput`
/// object and appends a bounded argument-context summary so that two calls of
/// the same tool with different arguments produce distinguishable descriptions.
/// The combined form is `"<title>(<context>)"` or `"<title>(<context>…)"` when
/// truncated. The combined output is capped at `DESCRIPTION_COMBINED_MAX_BYTES`
/// (200 UTF-8 bytes); if the context portion would be empty after extracting
/// all known fields, no parenthetical is appended.
///
/// Context extraction order within `rawInput` (see `summarize_raw_input`):
///   `command` → file/path keys → `cwd` → `reason` → compact JSON fallback.
/// Secret-bearing keys (`token*`, `password*`, etc.) are redacted at all
/// levels before any fallback serialisation and are never surfaced verbatim.
///
/// For paths 4–5 (codex-specific), the extracted string is concrete command or
/// reason text that already carries the distinguishing argument; no additional
/// summarisation is needed.
///
/// Returns `None` when no non-empty string is found in any path, or when `msg`
/// does not have a `params` object.
///
/// Extracted as a pure function so tests can exercise it with verbatim wire
/// shapes without going through the full permission-request lifecycle.
pub(crate) fn description_from_request_permission(msg: &serde_json::Value) -> Option<String> {
    // Paths 1-3: tool-name title (buzz-agent v1/v2).
    let title = [
        msg.pointer("/params/title").and_then(|v| v.as_str()),
        msg.pointer("/params/subject/toolCall/title")
            .and_then(|v| v.as_str()),
        msg.pointer("/params/toolCall/title")
            .and_then(|v| v.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|s| !s.is_empty());

    if let Some(t) = title {
        // v2 rawInput lives under `params.subject.toolCall.rawInput`;
        // v1 lives under `params.toolCall.rawInput`. Read v2 first.
        let raw_input = msg
            .pointer("/params/subject/toolCall/rawInput")
            .or_else(|| msg.pointer("/params/toolCall/rawInput"));

        return Some(match raw_input {
            Some(ri) if ri.is_object() => {
                // Budget allocation so the combined form always fits in
                // DESCRIPTION_COMBINED_MAX_BYTES:
                //
                //   wrapper overhead: 5 bytes — "(" + "…" (U+2026, 3 bytes) + ")"
                //   context reserve:  10 bytes — minimum useful argument context
                //
                // The title is capped to `DESCRIPTION_COMBINED_MAX_BYTES -
                // wrapper_overhead - context_reserve` so that even a maximum-length
                // title always leaves room for at least `context_reserve` bytes of
                // argument context. Context is truncated with head+tail layout so
                // that strings sharing a long common prefix remain distinguishable
                // (suffix differences survive even when the budget is small).
                const WRAPPER_OVERHEAD: usize = 5; // "(" + "…" (3 bytes) + ")"
                const CONTEXT_RESERVE: usize = 10; // minimum visible context
                let title_cap_limit = DESCRIPTION_COMBINED_MAX_BYTES
                    .saturating_sub(WRAPPER_OVERHEAD)
                    .saturating_sub(CONTEXT_RESERVE);
                let title_cap = truncate_to_bytes(t, title_cap_limit);
                let context_budget = DESCRIPTION_COMBINED_MAX_BYTES
                    .saturating_sub(title_cap.len())
                    .saturating_sub(WRAPPER_OVERHEAD);

                match summarize_raw_input(ri) {
                    Some(ctx) if !ctx.is_empty() => {
                        let ctx_cap = truncate_to_bytes_head_tail(&ctx, context_budget);
                        let truncated = ctx.len() > ctx_cap.len();
                        if truncated {
                            format!("{title_cap}({ctx_cap}…)")
                        } else {
                            format!("{title_cap}({ctx_cap})")
                        }
                    }
                    _ => title_cap,
                }
            }
            _ => truncate_to_bytes(t, DESCRIPTION_COMBINED_MAX_BYTES),
        });
    }

    // Paths 4-5: codex-specific fallbacks — concrete argument text.
    [
        // codex-acp v1.1.7 command execution: toolCall.rawInput.command.
        // Verbatim wire shape from codex-acp tag v1.1.7,
        // `buildCommandPermissionRequest` in `CodexApprovalHandler.ts`.
        msg.pointer("/params/toolCall/rawInput/command")
            .and_then(|v| v.as_str()),
        // codex-acp v1.1.7 file-change: reason in _meta.codex.params.
        // Verbatim wire shape from `buildFileChangePermissionRequest`.
        msg.pointer("/params/_meta/codex/params/reason")
            .and_then(|v| v.as_str()),
    ]
    .into_iter()
    .flatten()
    .find(|s| !s.is_empty())
    .map(|s| truncate_to_bytes(s, DESCRIPTION_COMBINED_MAX_BYTES))
}

/// Build the JSON payload for a kind-9 PENDING sentinel card.
///
/// Fails closed (`None`) when any bounded string field (`requestNonce`,
/// `turnId`, `sessionId`) exceeds `SENTINEL_STRING_MAX_BYTES`, when the total
/// serialized content exceeds `SENTINEL_CONTENT_MAX_BYTES`, or when
/// `serde_json::to_string` fails (the last is unreachable in practice). Labels
/// are truncated to the byte limit rather than rejected. A `None` return routes
/// to synchronous denial — no card is ever published. The `expiry_unix_secs` is
/// `min(registered_at + 300, hard_deadline)`.
///
/// The card advertises EXACTLY the two ruled actions (allow_once, reject_once);
/// no other adapter option (e.g. `allow_always`) is ever forwarded.
fn build_sentinel_pending_payload(
    nonce: &str,
    actions: &CardActions,
    expiry_unix_secs: u64,
    session_id: Option<&str>,
    turn_id: &str,
    description: Option<&str>,
) -> Option<String> {
    check_sentinel_field("requestNonce", nonce)?;
    check_sentinel_field("turnId", turn_id)?;
    if let Some(sid) = session_id {
        check_sentinel_field("sessionId", sid)?;
    }
    let (option_ids, labels) = sentinel_option_fields(actions);
    // Description is display-only — truncate rather than reject, matching the
    // labels precedent. A None or empty subject is omitted from the payload.
    let description_capped = description.map(|d| truncate_to_bytes(d, SENTINEL_STRING_MAX_BYTES));
    let payload = serde_json::json!({
        "v": 1,
        "state": "pending",
        "requestNonce": nonce,
        "sessionId": session_id,
        "turnId": turn_id,
        "expiresAt": expiry_unix_secs,
        "optionIds": option_ids,
        "labels": labels,
        "description": description_capped,
    });
    serialize_bounded_sentinel(&payload)
}

/// Build the JSON payload for a kind-40003 RESOLVED sentinel card edit.
#[allow(clippy::too_many_arguments)]
fn build_sentinel_resolved_payload(
    nonce: &str,
    original_event_id: &str,
    actions: &CardActions,
    expiry_unix_secs: u64,
    session_id: Option<&str>,
    turn_id: &str,
    outcome: &str,
    chosen_option_id: Option<&str>,
    description: Option<&str>,
) -> Option<String> {
    check_sentinel_field("requestNonce", nonce)?;
    check_sentinel_field("turnId", turn_id)?;
    if let Some(sid) = session_id {
        check_sentinel_field("sessionId", sid)?;
    }
    if let Some(chosen) = chosen_option_id {
        check_sentinel_field("chosenOptionId", chosen)?;
    }
    let (option_ids, labels) = sentinel_option_fields(actions);
    let description_capped = description.map(|d| truncate_to_bytes(d, SENTINEL_STRING_MAX_BYTES));
    let payload = serde_json::json!({
        "v": 1,
        "state": "resolved",
        "requestNonce": nonce,
        "originalEventId": original_event_id,
        "sessionId": session_id,
        "turnId": turn_id,
        "expiresAt": expiry_unix_secs,
        "optionIds": option_ids,
        "labels": labels,
        "outcome": outcome,
        "chosenOptionId": chosen_option_id,
        "description": description_capped,
    });
    serialize_bounded_sentinel(&payload)
}

/// Build and sign a kind-9 sentinel card event.
///
/// Returns `None` when required context is absent (relay keys, channel ID, or
/// payload serialization fails). The event is signed by the agent's relay keys.
fn build_kind9_sentinel(
    keys: &Keys,
    channel_id: Uuid,
    owner_pubkey_hex: &str,
    thread_reply_event_id: Option<&str>,
    content: &str,
) -> Option<nostr::Event> {
    let mut tags = vec![
        Tag::parse(["h", &channel_id.to_string()]).ok()?,
        Tag::parse(["p", owner_pubkey_hex]).ok()?,
    ];
    if let Some(reply_id) = thread_reply_event_id {
        // NIP-10 reply tag: ["e", <id>, "", "reply"]
        tags.push(Tag::parse(["e", reply_id, "", "reply"]).ok()?);
    }
    EventBuilder::new(Kind::Custom(9), content)
        .tags(tags)
        .sign_with_keys(keys)
        .ok()
}

/// Build and sign a kind-40003 edit event targeting a kind-9 sentinel.
fn build_kind40003_sentinel(
    keys: &Keys,
    channel_id: Uuid,
    target_event_id: &str,
    content: &str,
) -> Option<nostr::Event> {
    let tags = vec![
        Tag::parse(["h", &channel_id.to_string()]).ok()?,
        Tag::parse(["e", target_event_id]).ok()?,
    ];
    EventBuilder::new(Kind::Custom(40003), content)
        .tags(tags)
        .sign_with_keys(keys)
        .ok()
}

/// Retransmit an already-signed resolved kind-40003 edit until the relay
/// accepts it, bounded by `delivery_deadline`.
///
/// The permission decision is irreversible before this runs (`finish_permission`
/// has already written the ACP response and removed the entry). A plain
/// fire-and-forget publish loses the edit whenever the socket is down at that
/// instant — the relay background task drops non-observer publishes while
/// disconnected — leaving the authoritative thread card stuck as "Timed out"
/// even though execution continued. Reusing the pending path's acked lane, this
/// retransmits the *same signed event* (idempotent by event id) on every
/// `Uncertain` outcome, pausing [`RESOLVED_RETRANSMIT_BACKOFF`] between tries so
/// a reconnect can carry it through. `Accepted`/`Rejected` are terminal (the
/// relay saw it).
///
/// `delivery_deadline` is computed at resolution time as
/// `Instant::now() + RESOLVED_DELIVERY_WINDOW_SECS`, independent of the original
/// card/click deadline. The first publish attempt is **unconditional** — the
/// deadline is only consulted before each *retry* so that the relay always sees
/// at least one publication even when the caller supplies an already-expired
/// deadline (e.g. during ordinary timeouts where `entry_deadline` was already
/// past when `finish_permission` fired).
///
/// Spawned detached so it never blocks the read loop. `event` is consumed and
/// resent by clone each attempt so the signature and id are stable across retries.
async fn retransmit_resolved_edit(
    publisher: RelayEventPublisher,
    event: nostr::Event,
    delivery_deadline: tokio::time::Instant,
) {
    let mut first_attempt = true;
    loop {
        // The first attempt is unconditional — an already-expired deadline must
        // not prevent the single relay write that resolves the card. Subsequent
        // retries (Uncertain outcome) are gated by the deadline so the loop
        // terminates once the delivery window closes.
        if !first_attempt && tokio::time::Instant::now() >= delivery_deadline {
            tracing::warn!(
                target: "acp::permission",
                "resolved edit {} not accepted before delivery window — giving up",
                event.id.to_hex()
            );
            return;
        }
        first_attempt = false;
        // Per-attempt ACK deadline: min(fixed publish timeout, delivery_deadline).
        // Capping each attempt at SENTINEL_PUBLISH_TIMEOUT sweeps a stuck waiter
        // promptly so the same signed event is resent, while the deadline check
        // above keeps the overall delivery window as the outer bound.
        let attempt_deadline = (tokio::time::Instant::now()
            + std::time::Duration::from_secs(SENTINEL_PUBLISH_TIMEOUT_SECS))
        .min(delivery_deadline);
        match publisher
            .register_publish_ack(event.clone(), attempt_deadline)
            .await
        {
            Ok(ack_rx) => match ack_rx.await.unwrap_or(crate::relay::AckOutcome::Uncertain) {
                crate::relay::AckOutcome::Accepted => {
                    tracing::debug!(
                        target: "acp::permission",
                        "resolved edit {} accepted by relay",
                        event.id.to_hex()
                    );
                    return;
                }
                crate::relay::AckOutcome::Rejected { message } => {
                    tracing::warn!(
                        target: "acp::permission",
                        "resolved edit {} rejected by relay: {message} — not retrying",
                        event.id.to_hex()
                    );
                    return;
                }
                crate::relay::AckOutcome::Uncertain => {
                    // Socket down or ACK deadline swept: back off, then resend
                    // the identical signed event once a reconnect is possible.
                }
            },
            Err(_) => {
                // Command channel closed — the relay task is gone for good;
                // no reconnect will happen, so stop.
                tracing::warn!(
                    target: "acp::permission",
                    "resolved edit {} publish channel closed — giving up",
                    event.id.to_hex()
                );
                return;
            }
        }
        tokio::time::sleep(RESOLVED_RETRANSMIT_BACKOFF).await;
    }
}

/// Select the unique `allow_once` option from a permission request's option list.
///
/// Returns `Ok(option_id)` when there is exactly one option with `kind =
/// "allow_once"` and a non-empty `optionId`. Returns `Err(reason)` (fail
/// closed) when:
/// - zero `allow_once` options are present,
/// - multiple `allow_once` options are present (ambiguous),
/// - the matching option has a missing or empty `optionId`.
///
/// `allow_always` options are deliberately not selected — they would grant
/// indefinite access without a per-request human decision.
fn select_allow_once(options: &[serde_json::Value]) -> Result<String, String> {
    select_unique_option_id(options, "allow_once")
}

/// Select the unique `reject_once` option's `optionId` from a request's option
/// list. Same fail-closed semantics as [`select_allow_once`].
fn select_reject_once(options: &[serde_json::Value]) -> Result<String, String> {
    select_unique_option_id(options, "reject_once")
}

/// Return the `optionId` of the single option whose `kind` matches `kind`.
///
/// Returns `Err(reason)` (fail closed) when zero or multiple options match, or
/// when the matching option's `optionId` is missing/empty.
fn select_unique_option_id(options: &[serde_json::Value], kind: &str) -> Result<String, String> {
    let candidates: Vec<&serde_json::Value> = options
        .iter()
        .filter(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some(kind))
        .collect();

    match candidates.len() {
        0 => Err(format!("no {kind} option found")),
        2.. => Err(format!(
            "multiple {kind} options found ({}); ambiguous",
            candidates.len()
        )),
        1 => candidates[0]
            .get("optionId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("{kind} option has missing or empty optionId")),
    }
}

/// Select the exactly-two card actions from a permission request's options:
/// the unique `allow_once` and the unique `reject_once`. Returns their option
/// objects (with `kind`/`name`/`optionId`) so the caller can build a card that
/// offers ONLY those two, fail closed otherwise.
///
/// This is the single enforcement point for the ruled product contract:
/// Allow-once / Reject only. `allow_always` and any other adapter option are
/// never surfaced as an actionable button.
fn select_card_actions(options: &[serde_json::Value]) -> Result<CardActions, String> {
    let allow_id = select_allow_once(options)?;
    let reject_id = select_reject_once(options)?;
    for id in [&allow_id, &reject_id] {
        if id.len() > SENTINEL_STRING_MAX_BYTES {
            return Err(format!(
                "optionId exceeds {SENTINEL_STRING_MAX_BYTES} bytes: {} > {}",
                id.len(),
                SENTINEL_STRING_MAX_BYTES
            ));
        }
    }
    let find = |target: &str| -> serde_json::Value {
        options
            .iter()
            .find(|o| o.get("optionId").and_then(|v| v.as_str()) == Some(target))
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    };
    Ok(CardActions {
        allow: find(&allow_id),
        reject: find(&reject_id),
    })
}

/// Ask-only admission gates the caller precomputes from live map state. Grouped
/// so the preflight signature stays small; each field short-circuits a distinct
/// fail-closed reason (see checks 6, 7, 7b below).
struct AskGates {
    is_duplicate_id: bool,
    is_map_at_cap: bool,
    is_publish_in_flight: bool,
}

/// Validate a `session/request_permission` request before it touches the
/// pending map or policy dispatch.
///
/// Returns `Ok(())` on a clean request; `Err(reason)` on the first violation.
///
/// Checks (in order):
/// 1. `options` nonempty.
/// 2. `options` count ≤ `PERMISSION_OPTIONS_MAX`.
/// 3. Every `optionId` is present and non-empty.
/// 4. Every `optionId` is unique across the request.
/// 5. Every option has a non-empty `kind` and `name`.
/// 6. Duplicate live `requestId` (only relevant under `ask`, caller passes flag).
/// 7. Permission map at capacity (only relevant under `ask`, caller passes flag).
/// 8. Full serialised `ObserverEvent` (raw payload + all envelope fields + real
///    context) fits within `OBSERVER_MAX_PLAINTEXT_LEN` — no leaf surgery on frames.
fn run_admission_preflight(
    _id: &serde_json::Value,
    options: &[serde_json::Value],
    msg: &serde_json::Value,
    _policy: PermissionPolicy,
    ask_gates: AskGates,
    size_ctx: (&ObserverContext, Option<usize>),
) -> Result<(), String> {
    let AskGates {
        is_duplicate_id,
        is_map_at_cap,
        is_publish_in_flight,
    } = ask_gates;
    let (observer_context, agent_index) = size_ctx;
    // 1. options nonempty
    if options.is_empty() {
        return Err("options array is empty".to_string());
    }

    // 2. count ≤ PERMISSION_OPTIONS_MAX
    if options.len() > PERMISSION_OPTIONS_MAX {
        return Err(format!(
            "too many options: {} > {}",
            options.len(),
            PERMISSION_OPTIONS_MAX
        ));
    }

    // 3 & 4. optionId present, non-empty, unique
    let mut seen_ids = std::collections::HashSet::new();
    for opt in options {
        let option_id = opt
            .get("optionId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "option has missing or empty optionId".to_string())?;
        if !seen_ids.insert(option_id) {
            return Err(format!("duplicate optionId: {option_id:?}"));
        }
    }

    // 5. required kind and name fields
    for opt in options {
        if opt
            .get("kind")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .is_none()
        {
            return Err("option has missing or empty kind".to_string());
        }
        if opt
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .is_none()
        {
            return Err("option has missing or empty name".to_string());
        }
    }

    // 6. duplicate live requestId (ask only — caller computes flag)
    if is_duplicate_id {
        return Err("duplicate live requestId".to_string());
    }

    // 7. map at capacity (ask only — caller computes flag)
    if is_map_at_cap {
        return Err(format!(
            "pending permission map at capacity ({})",
            PERMISSION_MAP_CAP
        ));
    }

    // 7b. a sentinel publish is already in flight (ask only — one at a time)
    if is_publish_in_flight {
        return Err("a sentinel publish is already in flight".to_string());
    }

    // 8. Full annotated `ObserverEvent` fits within `OBSERVER_MAX_PLAINTEXT_LEN`.
    //
    // Construct the exact production `ObserverEvent` with the real observer context
    // and a representative nonce. Serialise it and reject if over cap. This is the
    // same construction path the observer uses at emit time, so any payload that
    // passes here is guaranteed to fit in the final frame — no leaf surgery needed.
    //
    // A UUID nonce is used for sizing; the actual nonce is generated after the
    // preflight passes, but all nonces are the same UUID length.
    let candidate_event = ObserverEvent {
        seq: u64::MAX, // worst-case seq (19 digits)
        timestamp: "2026-01-01T00:00:00.000000000+00:00".to_string(), // max RFC3339 len
        kind: "acp_read".to_string(),
        agent_index,
        channel_id: observer_context.channel_id.clone(),
        session_id: observer_context.session_id.clone(),
        turn_id: observer_context.turn_id.clone(),
        started_at: observer_context.started_at.clone(),
        authorization: Some(AuthorizationEnvelope {
            // UUID nonce — all production nonces are this length.
            request_nonce: "00000000-0000-0000-0000-000000000000".to_string(),
            actionable: true,
            reason: None,
            expires_at: None,
        }),
        payload: msg.clone(),
    };
    let annotated_len = serde_json::to_string(&candidate_event)
        .map(|s| s.len())
        .unwrap_or(usize::MAX);
    if annotated_len > OBSERVER_MAX_PLAINTEXT_LEN {
        return Err(format!(
            "permission request payload too large: annotated size {annotated_len} > {OBSERVER_MAX_PLAINTEXT_LEN}"
        ));
    }

    Ok(())
}

/// Full `session/new` response — session ID plus the raw JSON result.
///
/// Callers use the extractor helpers to pull model info from `raw`.
pub struct SessionNewResponse {
    pub session_id: String,
    /// The full `result` value from the JSON-RPC response.
    pub raw: serde_json::Value,
}

/// How to deliver a system prompt on `session/new`.
///
/// The two variants match the two mechanisms supported by current adapters:
///
/// - **`Field`** — bare `systemPrompt` field (ACP protocol v2, buzz-agent).
/// - **`ClaudeMeta`** — `_meta.systemPrompt: {"append": text}`, used by
///   `claude-agent-acp` to append to the adapter's own native system prompt
///   while keeping its tool-use preset intact.
#[derive(Debug, Clone, PartialEq)]
pub enum SystemPromptTransport<'a> {
    /// Deliver as a bare top-level `systemPrompt` field.
    Field(&'a str),
    /// Deliver as `_meta.systemPrompt: {"append": text}`.
    ClaudeMeta(&'a str),
}

/// How to switch to a particular model on a session.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type")]
pub enum ModelSwitchMethod {
    /// Stable: use `session/set_config_option` with these exact values.
    ConfigOption {
        config_id: String,
        option_value: String,
    },
    /// Unstable: use `session/set_model` with this model_id.
    SetModel { model_id: String },
}

/// Extract `configOptions` entries with `category == "model"` from a `session/new` result.
///
/// Returns the raw JSON array entries. Each entry has `configId` (spelled `id`
/// by some adapters, e.g. claude-agent-acp), `displayName`,
/// `options: [{ value, displayName }]`, etc.
pub fn extract_model_config_options(result: &serde_json::Value) -> Vec<serde_json::Value> {
    result["configOptions"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|opt| opt.get("category").and_then(|c| c.as_str()) == Some("model"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Extract `SessionModelState` (unstable path) from a `session/new` result.
///
/// Returns the `models` object if present: `{ currentModelId, availableModels: [...] }`.
pub fn extract_model_state(result: &serde_json::Value) -> Option<serde_json::Value> {
    result.get("models").cloned()
}

/// Extract the `configId` for the `thought_level` category option from a
/// `session/new` result, if the adapter advertised one.
///
/// Claude Code's adapter uses `category: "thought_level"` in its `configOptions`.
/// The configId is adapter-defined (e.g. `"effort"` on claude-agent-acp) and must
/// not be hardcoded in the harness — this function discovers it at session time so
/// the spawn-scoped effort application forwards the adapter's real id. Accepts both
/// `configId` (ACP spec) and `id` (claude-agent-acp), matching the model-switch path.
pub fn extract_thought_level_config_id(result: &serde_json::Value) -> Option<String> {
    let arr = result["configOptions"].as_array()?;
    for opt in arr {
        if opt.get("category").and_then(|c| c.as_str()) == Some("thought_level") {
            let config_id = opt
                .get("configId")
                .or_else(|| opt.get("id"))
                .and_then(|v| v.as_str())?;
            return Some(config_id.to_string());
        }
    }
    None
}

/// Match a desired model ID against a fresh `session/new` response.
///
/// Returns the correct ACP method to call, or `None` if no match.
///
/// **Precedence**: stable `configOptions` first (spec-blessed), then unstable
/// `availableModels`. The fresh `session/new` response is always authoritative.
pub fn resolve_model_switch_method(
    session_new_result: &serde_json::Value,
    desired_model: &str,
) -> Option<ModelSwitchMethod> {
    // 1. Search stable configOptions for a "model"-category entry whose
    //    options contain a value matching desired_model.
    for config_opt in extract_model_config_options(session_new_result) {
        // Adapters disagree on the key: the ACP spec says `configId`, but
        // claude-agent-acp emits `id`. Accept both; the set request always
        // uses `configId` on the wire.
        let config_id = match config_opt
            .get("configId")
            .or_else(|| config_opt.get("id"))
            .and_then(|v| v.as_str())
        {
            Some(id) => id,
            None => continue,
        };
        if let Some(options) = config_opt.get("options").and_then(|v| v.as_array()) {
            for opt in options {
                if opt.get("value").and_then(|v| v.as_str()) == Some(desired_model) {
                    return Some(ModelSwitchMethod::ConfigOption {
                        config_id: config_id.to_string(),
                        option_value: desired_model.to_string(),
                    });
                }
            }
        }
    }

    // 2. Search unstable availableModels for a matching modelId.
    if let Some(models) = extract_model_state(session_new_result) {
        if let Some(available) = models.get("availableModels").and_then(|v| v.as_array()) {
            for model in available {
                if model.get("modelId").and_then(|v| v.as_str()) == Some(desired_model) {
                    return Some(ModelSwitchMethod::SetModel {
                        model_id: desired_model.to_string(),
                    });
                }
            }
        }
    }

    // 3. No match.
    None
}

/// Whether `desired_model` appears in pre-extracted catalog halves.
///
/// Mirrors [`resolve_model_switch_method`]'s match, but operates on the
/// already-extracted `configOptions` (model category) and `models` state that
/// [`AgentModelCapabilities`](crate::pool::AgentModelCapabilities) caches — the
/// idle-path pre-cancel guard has those halves, not the full `session/new` JSON.
pub fn model_in_catalog(
    config_options: &[serde_json::Value],
    available_models: Option<&serde_json::Value>,
    desired_model: &str,
) -> bool {
    let in_config_options = config_options.iter().any(|config_opt| {
        config_opt
            .get("options")
            .and_then(|v| v.as_array())
            .is_some_and(|options| {
                options
                    .iter()
                    .any(|opt| opt.get("value").and_then(|v| v.as_str()) == Some(desired_model))
            })
    });
    if in_config_options {
        return true;
    }

    available_models
        .and_then(|models| models.get("availableModels"))
        .and_then(|v| v.as_array())
        .is_some_and(|available| {
            available
                .iter()
                .any(|model| model.get("modelId").and_then(|v| v.as_str()) == Some(desired_model))
        })
}

// ─── Drop: kill child process ─────────────────────────────────────────────────

impl Drop for AcpClient {
    fn drop(&mut self) {
        // Best-effort SIGKILL + reap. We cannot `await` in Drop (sync context).
        // Kill the process group when possible so subprocesses don't leak.
        // Callers SHOULD still call `shutdown().await` for guaranteed reaping.
        match self.child.id() {
            Some(pid) if kill_process_group(pid) => {}
            _ => {
                let _ = self.child.start_kill();
            }
        }
        // Non-blocking reap attempt — prevents zombie accumulation in the
        // common case where SIGKILL takes effect before Drop returns.
        let _ = self.child.try_wait();
    }
}

/// Send SIGKILL to an entire process group. Returns `true` if the signal was sent.
///
/// The child is spawned with `process_group(0)`, so its PID equals its PGID.
/// Killing the group ensures subprocesses (MCP servers, tool processes) are
/// cleaned up rather than orphaned to init on repeated crash-recovery cycles.
///
/// Uses `nix::sys::signal::killpg` — a safe wrapper around the POSIX `killpg`
/// syscall — so the crate's `#![deny(unsafe_code)]` policy is preserved.
#[cfg(unix)]
fn kill_process_group(pid: u32) -> bool {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;

    // pid == pgid because the child was spawned with process_group(0).
    killpg(Pid::from_raw(pid as i32), Signal::SIGKILL).is_ok()
}

/// Fallback for non-Unix: process-group kill not available.
/// Returns `false` so the caller falls back to `child.start_kill()`.
#[cfg(not(unix))]
fn kill_process_group(_pid: u32) -> bool {
    false
}

/// Suppress the console window that Windows otherwise allocates for every
/// console-subsystem child process spawned from a GUI (non-console) parent.
/// No-op on non-Windows platforms.
fn configure_no_window(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModeSource;

    #[test]
    fn stop_reason_parses_all_known_values() {
        assert_eq!(StopReason::from_str("end_turn"), Some(StopReason::EndTurn));
        assert_eq!(
            StopReason::from_str("cancelled"),
            Some(StopReason::Cancelled)
        );
        assert_eq!(
            StopReason::from_str("max_tokens"),
            Some(StopReason::MaxTokens)
        );
        assert_eq!(
            StopReason::from_str("max_turn_requests"),
            Some(StopReason::MaxTurnRequests)
        );
        assert_eq!(StopReason::from_str("refusal"), Some(StopReason::Refusal));
    }

    #[test]
    fn stop_reason_returns_none_for_unknown() {
        assert_eq!(StopReason::from_str("unknown_value"), None);
        assert_eq!(StopReason::from_str(""), None);
        assert_eq!(StopReason::from_str("endturn"), None); // no camelCase — still unknown
    }

    #[test]
    fn stop_reason_is_case_insensitive() {
        // Agents may send uppercase or mixed-case variants — all should parse correctly.
        assert_eq!(StopReason::from_str("END_TURN"), Some(StopReason::EndTurn));
        assert_eq!(
            StopReason::from_str("CANCELLED"),
            Some(StopReason::Cancelled)
        );
        assert_eq!(
            StopReason::from_str("Max_Tokens"),
            Some(StopReason::MaxTokens)
        );
        assert_eq!(
            StopReason::from_str("MAX_TURN_REQUESTS"),
            Some(StopReason::MaxTurnRequests)
        );
        assert_eq!(StopReason::from_str("Refusal"), Some(StopReason::Refusal));
    }

    fn options(json: &str) -> Vec<serde_json::Value> {
        serde_json::from_str(json).expect("option list")
    }

    fn outcome(response: &serde_json::Value) -> Option<&str> {
        response["result"]["outcome"]["outcome"].as_str()
    }

    /// The offered `allow_once` and `allow_always` options must be ignored:
    /// there is no human to click them, so choosing either would make every
    /// admitted prompt an implicit approval. `optionId`s are deliberately
    /// non-obvious to prove they are matched by `kind`, never hardcoded.
    #[test]
    fn permission_requests_select_reject_once_not_allow_once() {
        let options = options(
            r#"[
            {"optionId": "opt-reject-42",  "name": "Reject",       "kind": "reject_once"},
            {"optionId": "opt-allow-99",   "name": "Allow once",   "kind": "allow_once"},
            {"optionId": "opt-always-7",   "name": "Always allow", "kind": "allow_always"}
        ]"#,
        );

        let response =
            permission_denial_response(&serde_json::json!(7), &options).expect("denial response");

        assert_eq!(outcome(&response), Some("selected"));
        assert_eq!(
            response["result"]["outcome"]["optionId"].as_str(),
            Some("opt-reject-42"),
            "must select reject_once even when allow options are offered"
        );
    }

    /// Fail-closed backstop: an adapter that offers no `reject_once` must still
    /// be denied, via the protocol's cancelled outcome rather than an error or
    /// an approval.
    #[test]
    fn permission_request_without_reject_once_is_cancelled() {
        let options = options(
            r#"[
            {"optionId": "opt-allow-99", "name": "Allow once",   "kind": "allow_once"},
            {"optionId": "opt-always-7", "name": "Always allow", "kind": "allow_always"}
        ]"#,
        );

        let response = permission_denial_response(&serde_json::json!("req-1"), &options)
            .expect("cancelled response");

        assert_eq!(outcome(&response), Some("cancelled"));
        assert_eq!(
            response["id"].as_str(),
            Some("req-1"),
            "string ids must round-trip per JSON-RPC 2.0"
        );
    }

    /// An empty option list is the degenerate form of the same backstop.
    #[test]
    fn permission_request_with_no_options_is_cancelled() {
        let response =
            permission_denial_response(&serde_json::json!(1), &[]).expect("cancelled response");

        assert_eq!(outcome(&response), Some("cancelled"));
    }

    /// A `reject_once` option missing its `optionId` falls back to a `cancelled`
    /// response rather than propagating a Protocol error. This ensures the adapter
    /// always receives a valid JSON-RPC response, even for malformed requests.
    #[test]
    fn reject_once_without_option_id_falls_back_to_cancelled() {
        let options = options(r#"[{"name": "Reject", "kind": "reject_once"}]"#);

        let response = permission_denial_response(&serde_json::json!(1), &options)
            .expect("malformed reject_once must not error");

        assert_eq!(
            response["result"]["outcome"]["outcome"].as_str(),
            Some("cancelled"),
            "malformed reject_once must produce cancelled, got: {response}"
        );
    }

    #[test]
    fn find_reject_once_by_kind() {
        let options =
            options(r#"[{"optionId": "rej-x", "name": "Reject", "kind": "reject_once"}]"#);

        let response =
            permission_denial_response(&serde_json::json!(1), &options).expect("denial response");

        assert_eq!(
            response["result"]["outcome"]["optionId"].as_str(),
            Some("rej-x")
        );
    }

    #[test]
    fn request_has_id_field() {
        let id: u64 = 42;
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {}
        });
        assert!(msg.get("id").is_some(), "request must have id field");
        assert_eq!(msg["id"].as_u64(), Some(42));
        assert_eq!(msg["jsonrpc"].as_str(), Some("2.0"));
        assert_eq!(msg["method"].as_str(), Some("initialize"));
    }

    #[test]
    fn notification_has_no_id_field() {
        // session/cancel is a notification — must NOT have an id field.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": {
                "sessionId": "sess_abc123"
            }
        });
        assert!(
            msg.get("id").is_none(),
            "notification must NOT have id field"
        );
        assert_eq!(msg["jsonrpc"].as_str(), Some("2.0"));
        assert_eq!(msg["method"].as_str(), Some("session/cancel"));
    }

    #[test]
    fn initialize_request_format() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0u64,
            "method": "initialize",
            "params": {
                "protocolVersion": 2,
                "clientCapabilities": build_client_capabilities(),
                "clientInfo": {
                    "name": "buzz-acp",
                    "version": "0.1.0"
                }
            }
        });
        assert_eq!(msg["params"]["protocolVersion"].as_u64(), Some(2));
        assert_eq!(
            msg["params"]["clientInfo"]["name"].as_str(),
            Some("buzz-acp")
        );
        assert!(msg["params"]["clientCapabilities"].is_object());
        assert_eq!(
            msg["params"]["clientCapabilities"]["auth"]["terminal"].as_bool(),
            Some(true),
            "terminal auth capability must be advertised so adapters can expose terminal login methods"
        );
        assert_eq!(
            msg["params"]["clientCapabilities"]["_meta"]["goose"]["customNotifications"].as_bool(),
            Some(true),
            "goose customNotifications capability must be advertised"
        );
    }

    #[test]
    fn session_new_mcp_server_has_required_fields() {
        // Schema requires name, command, args, env — all present, args/env may be empty.
        let server = McpServer {
            name: "test-mcp".into(),
            command: "/usr/local/bin/test-mcp-server".into(),
            args: vec![],
            env: vec![
                EnvVar {
                    name: "BUZZ_RELAY_URL".into(),
                    value: "ws://localhost:3000".into(),
                },
                EnvVar {
                    name: "BUZZ_PRIVATE_KEY".into(),
                    value: "nsec1abc".into(),
                },
            ],
        };
        let serialized = serde_json::to_value(&server).unwrap();
        assert_eq!(serialized["name"].as_str(), Some("test-mcp"));
        assert_eq!(
            serialized["command"].as_str(),
            Some("/usr/local/bin/test-mcp-server")
        );
        assert!(serialized["args"].is_array());
        assert_eq!(serialized["args"].as_array().unwrap().len(), 0);
        assert!(serialized["env"].is_array());
        assert_eq!(serialized["env"].as_array().unwrap().len(), 2);
        assert_eq!(
            serialized["env"][0]["name"].as_str(),
            Some("BUZZ_RELAY_URL")
        );
    }

    #[test]
    fn session_prompt_request_format() {
        let prompt_text = "[Buzz @mention]\nChannel: test\nFrom: npub1...\nMessage: hello";
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2u64,
            "method": "session/prompt",
            "params": {
                "sessionId": "sess_abc123",
                "prompt": [
                    { "type": "text", "text": prompt_text }
                ]
            }
        });
        assert_eq!(msg["method"].as_str(), Some("session/prompt"));
        let prompt = msg["params"]["prompt"].as_array().unwrap();
        assert_eq!(prompt.len(), 1);
        assert_eq!(prompt[0]["type"].as_str(), Some("text"));
        assert_eq!(prompt[0]["text"].as_str(), Some(prompt_text));
    }

    #[test]
    fn session_prompt_slash_command_two_block_format() {
        // Slash-command pass-through: bare command first, wrapped context second.
        let params = build_prompt_params(
            "sess_abc123",
            &[
                "/goal ship it",
                "[Buzz event: @mention]\nContent: @Eva /goal ship it",
            ],
        );
        let prompt = params["prompt"].as_array().unwrap();
        assert_eq!(prompt.len(), 2);
        assert_eq!(prompt[0]["type"].as_str(), Some("text"));
        assert_eq!(prompt[0]["text"].as_str(), Some("/goal ship it"));
        assert!(prompt[0]["text"].as_str().unwrap().starts_with('/'));
        assert_eq!(prompt[1]["type"].as_str(), Some("text"));
    }

    #[test]
    fn permission_response_selected_format() {
        let id: u64 = 5;
        let option_id = "opt-allow-99";
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "outcome": {
                    "outcome": "selected",
                    "optionId": option_id
                }
            }
        });
        assert_eq!(response["id"].as_u64(), Some(5));
        assert_eq!(
            response["result"]["outcome"]["outcome"].as_str(),
            Some("selected")
        );
        assert_eq!(
            response["result"]["outcome"]["optionId"].as_str(),
            Some("opt-allow-99")
        );
    }

    #[test]
    fn permission_response_cancelled_format() {
        let id: u64 = 5;
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "outcome": {
                    "outcome": "cancelled"
                }
            }
        });
        assert_eq!(
            response["result"]["outcome"]["outcome"].as_str(),
            Some("cancelled")
        );
        // cancelled outcome has no optionId
        assert!(response["result"]["outcome"].get("optionId").is_none());
    }

    #[test]
    fn session_cancel_notification_has_session_id_in_params() {
        let session_id = "sess_xyz789";
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": {
                "sessionId": session_id
            }
        });
        // Must have no id (notification)
        assert!(msg.get("id").is_none());
        // Must have sessionId in params
        assert_eq!(msg["params"]["sessionId"].as_str(), Some("sess_xyz789"));
    }

    #[test]
    fn permission_request_with_string_id() {
        // Verify that permission response uses the same ID type as the request.
        // JSON-RPC 2.0 permits string IDs from the agent.
        let string_id = serde_json::json!("perm-req-001");
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": string_id,
            "result": {
                "outcome": { "outcome": "selected", "optionId": "allow-once" }
            }
        });
        assert_eq!(response["id"], "perm-req-001");
        assert!(response["id"].is_string());
    }

    #[test]
    fn id_comparison_works_for_numeric_and_string() {
        // Verify json!(expected_id) comparison logic used in read_until_response.
        let expected_id: u64 = 3;
        let numeric_response_id = serde_json::json!(3u64);
        let string_response_id = serde_json::json!("3");

        // Numeric matches
        assert_eq!(numeric_response_id, serde_json::json!(expected_id));
        // String does NOT match numeric (correct — different types)
        assert_ne!(string_response_id, serde_json::json!(expected_id));
    }

    #[test]
    fn permission_cancelled_response_preserves_id_type() {
        // String ID from agent should be echoed back as string in cancelled response.
        let string_id = serde_json::json!("req-abc");
        let cancelled = serde_json::json!({
            "jsonrpc": "2.0",
            "id": string_id.clone(),
            "result": { "outcome": { "outcome": "cancelled" } }
        });
        assert_eq!(cancelled["id"], string_id);
        assert!(cancelled["id"].is_string());

        // Numeric ID from agent should be echoed back as numeric.
        let numeric_id = serde_json::json!(42u64);
        let cancelled_numeric = serde_json::json!({
            "jsonrpc": "2.0",
            "id": numeric_id.clone(),
            "result": { "outcome": { "outcome": "cancelled" } }
        });
        assert_eq!(cancelled_numeric["id"], numeric_id);
        assert!(cancelled_numeric["id"].is_number());
    }

    #[test]
    fn extract_model_config_options_finds_model_category() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "configOptions": [
                {
                    "configId": "model",
                    "category": "model",
                    "displayName": "Model",
                    "options": [
                        { "value": "claude-sonnet-4-20250514", "displayName": "Claude Sonnet 4" },
                        { "value": "claude-opus-4-20250514", "displayName": "Claude Opus 4" }
                    ]
                },
                {
                    "configId": "theme",
                    "category": "appearance",
                    "displayName": "Theme",
                    "options": [{ "value": "dark", "displayName": "Dark" }]
                }
            ]
        });
        let opts = super::extract_model_config_options(&result);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0]["configId"].as_str(), Some("model"));
    }

    #[test]
    fn extract_model_config_options_empty_when_no_config_options() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::extract_model_config_options(&result).is_empty());
    }

    #[test]
    fn extract_model_config_options_empty_when_no_model_category() {
        let result = serde_json::json!({
            "configOptions": [
                { "configId": "theme", "category": "appearance" }
            ]
        });
        assert!(super::extract_model_config_options(&result).is_empty());
    }

    #[test]
    fn extract_model_state_returns_models_object() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "models": {
                "currentModelId": "gpt-5",
                "availableModels": [
                    { "modelId": "gpt-5", "name": "GPT-5" },
                    { "modelId": "o3-pro", "name": "o3 Pro" }
                ]
            }
        });
        let ms = super::extract_model_state(&result).expect("should have models");
        assert_eq!(ms["currentModelId"].as_str(), Some("gpt-5"));
        assert_eq!(ms["availableModels"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn extract_model_state_none_when_absent() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::extract_model_state(&result).is_none());
    }

    #[test]
    fn extract_thought_level_config_id_finds_config_id() {
        let result = serde_json::json!({
            "sessionId": "sess-1",
            "configOptions": [
                { "configId": "model", "category": "model" },
                {
                    "configId": "effort",
                    "category": "thought_level",
                    "options": [{ "value": "high" }, { "value": "low" }]
                }
            ]
        });
        assert_eq!(
            super::extract_thought_level_config_id(&result).as_deref(),
            Some("effort")
        );
    }

    #[test]
    fn extract_thought_level_config_id_falls_back_to_id_key() {
        let result = serde_json::json!({
            "configOptions": [
                { "id": "effort", "category": "thought_level" }
            ]
        });
        assert_eq!(
            super::extract_thought_level_config_id(&result).as_deref(),
            Some("effort")
        );
    }

    #[test]
    fn extract_thought_level_config_id_none_without_category() {
        let result = serde_json::json!({
            "configOptions": [
                { "configId": "model", "category": "model" }
            ]
        });
        assert!(super::extract_thought_level_config_id(&result).is_none());
    }

    #[test]
    fn extract_thought_level_config_id_none_without_config_options() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::extract_thought_level_config_id(&result).is_none());
    }

    #[test]
    fn resolve_prefers_stable_over_unstable() {
        let result = serde_json::json!({
            "configOptions": [{
                "configId": "model",
                "category": "model",
                "options": [
                    { "value": "claude-sonnet-4-20250514", "displayName": "Sonnet 4" }
                ]
            }],
            "models": {
                "currentModelId": "claude-sonnet-4-20250514",
                "availableModels": [
                    { "modelId": "claude-sonnet-4-20250514", "name": "Sonnet 4" }
                ]
            }
        });
        let method = super::resolve_model_switch_method(&result, "claude-sonnet-4-20250514");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::ConfigOption {
                config_id: "model".to_string(),
                option_value: "claude-sonnet-4-20250514".to_string(),
            })
        );
    }

    #[test]
    fn resolve_accepts_id_keyed_config_options() {
        // claude-agent-acp (observed on v0.61.0) keys config options with
        // `id` instead of the spec's `configId`. Payload mirrors its real
        // `session/new` response.
        let result = serde_json::json!({
            "configOptions": [{
                "id": "model",
                "name": "Model",
                "category": "model",
                "type": "select",
                "currentValue": "default",
                "options": [
                    { "value": "default", "name": "Default" },
                    { "value": "opus[1m]", "name": "Opus" },
                    { "value": "sonnet", "name": "Sonnet" }
                ]
            }],
            "models": null
        });
        let method = super::resolve_model_switch_method(&result, "opus[1m]");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::ConfigOption {
                config_id: "model".to_string(),
                option_value: "opus[1m]".to_string(),
            })
        );
    }

    #[test]
    fn resolve_falls_back_to_unstable() {
        let result = serde_json::json!({
            "models": {
                "currentModelId": "gpt-5",
                "availableModels": [
                    { "modelId": "gpt-5", "name": "GPT-5" },
                    { "modelId": "o3-pro", "name": "o3 Pro" }
                ]
            }
        });
        let method = super::resolve_model_switch_method(&result, "o3-pro");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::SetModel {
                model_id: "o3-pro".to_string(),
            })
        );
    }

    #[test]
    fn resolve_returns_none_when_no_match() {
        let result = serde_json::json!({
            "configOptions": [{
                "configId": "model",
                "category": "model",
                "options": [{ "value": "claude-sonnet-4-20250514" }]
            }],
            "models": {
                "availableModels": [{ "modelId": "gpt-5" }]
            }
        });
        assert!(super::resolve_model_switch_method(&result, "nonexistent-model").is_none());
    }

    #[test]
    fn resolve_returns_none_when_no_model_info() {
        let result = serde_json::json!({ "sessionId": "sess-1" });
        assert!(super::resolve_model_switch_method(&result, "anything").is_none());
    }

    #[test]
    fn resolve_handles_multiple_config_options() {
        // Agent could have multiple configOptions with category "model"
        // (unlikely but defensive).
        let result = serde_json::json!({
            "configOptions": [
                {
                    "configId": "primary-model",
                    "category": "model",
                    "options": [{ "value": "model-a" }]
                },
                {
                    "configId": "fallback-model",
                    "category": "model",
                    "options": [{ "value": "model-b" }]
                }
            ]
        });
        let method = super::resolve_model_switch_method(&result, "model-b");
        assert_eq!(
            method,
            Some(super::ModelSwitchMethod::ConfigOption {
                config_id: "fallback-model".to_string(),
                option_value: "model-b".to_string(),
            })
        );
    }

    // ── model_in_catalog tests ────────────────────────────────────────────

    #[test]
    fn model_in_catalog_true_when_in_config_options() {
        let config_options = vec![serde_json::json!({
            "configId": "model",
            "category": "model",
            "options": [
                { "value": "claude-sonnet-4-20250514" },
                { "value": "claude-opus-4-20250514" }
            ]
        })];
        assert!(super::model_in_catalog(
            &config_options,
            None,
            "claude-opus-4-20250514"
        ));
    }

    #[test]
    fn model_in_catalog_true_when_in_available_models() {
        let available = serde_json::json!({
            "currentModelId": "gpt-5",
            "availableModels": [
                { "modelId": "gpt-5" },
                { "modelId": "o3-pro" }
            ]
        });
        assert!(super::model_in_catalog(&[], Some(&available), "o3-pro"));
    }

    #[test]
    fn model_in_catalog_false_when_absent_from_both_halves() {
        let config_options = vec![serde_json::json!({
            "configId": "model",
            "options": [{ "value": "claude-sonnet-4-20250514" }]
        })];
        let available = serde_json::json!({
            "availableModels": [{ "modelId": "gpt-5" }]
        });
        assert!(!super::model_in_catalog(
            &config_options,
            Some(&available),
            "nonexistent-model"
        ));
    }

    #[test]
    fn model_in_catalog_false_when_both_halves_empty() {
        assert!(!super::model_in_catalog(&[], None, "anything"));
    }

    // ── Error variant display ─────────────────────────────────────────────

    #[test]
    fn idle_timeout_error_includes_duration() {
        let err = AcpError::IdleTimeout(std::time::Duration::from_secs(320));
        let msg = err.to_string();
        assert!(
            msg.contains("320"),
            "IdleTimeout display should include duration: {msg}"
        );
    }

    #[test]
    fn hard_timeout_error_display() {
        let err = AcpError::HardTimeout {
            silence: std::time::Duration::from_secs(120),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("Hard turn timeout"),
            "HardTimeout display: {msg}"
        );
    }

    async fn spawn_script(script: &str) -> AcpClient {
        AcpClient::spawn("bash", &["-c".into(), script.into()], &[], false)
            .await
            .expect("failed to spawn test script")
    }

    #[cfg(unix)]
    async fn spawn_named_script(name: &str, script: &str) -> (AcpClient, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "buzz-acp-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp adapter dir");
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/usr/bin/env bash\n{script}\n"))
            .expect("write fake adapter");
        let mut permissions = std::fs::metadata(&path)
            .expect("adapter metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("chmod fake adapter");
        let client = AcpClient::spawn(path.to_str().expect("utf8 path"), &[], &[], false)
            .await
            .expect("spawn named fake adapter");
        (client, dir)
    }

    /// Spawn a probe script whose file name carries a runtime identity (e.g.
    /// `hermes-acp`) and return the value of `var` as the child observed it.
    /// `<unset>` means the child did not receive the var.
    #[cfg(unix)]
    async fn spawn_named_and_read_child_env(
        file_name: &str,
        var: &str,
        extra_env: &[(String, String)],
    ) -> String {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("buzz-acp-env-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create env probe dir");
        let path = dir.join(file_name);
        std::fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' \"${{{var}:-<unset>}}\"\n"),
        )
        .expect("write env probe script");
        let mut permissions = std::fs::metadata(&path).expect("stat probe").permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&path, permissions).expect("chmod probe");

        let mut client = AcpClient::spawn(
            path.to_str().expect("probe path is UTF-8"),
            &[],
            extra_env,
            false,
        )
        .await
        .expect("spawn env probe script");
        let observed = client
            .reader
            .next()
            .await
            .unwrap_or_else(|| panic!("child produced no output for {var}"))
            .expect("child stdout was not readable");
        client.shutdown().await;
        std::fs::remove_dir_all(&dir).expect("remove env probe dir");
        observed
    }

    /// Buzz-owned Hermes processes get the configured-MCP isolation default,
    /// and an explicit persona entry still overrides it (defaults are applied
    /// before `extra_env`, so the later `Command::env` write wins).
    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_applies_runtime_env_defaults_with_extra_env_precedence() {
        const VAR: &str = "HERMES_ACP_SKIP_CONFIGURED_MCP";
        if std::env::var_os(VAR).is_some() {
            // Inherited parent values win over both layers; the default and
            // override behavior below is unobservable in such an environment.
            return;
        }

        assert_eq!(
            spawn_named_and_read_child_env("hermes-acp", VAR, &[]).await,
            "1",
            "Hermes spawns must default {VAR}=1"
        );
        assert_eq!(
            spawn_named_and_read_child_env("hermes-acp", VAR, &[(VAR.into(), "0".into())]).await,
            "0",
            "an explicit extra_env entry must override the runtime default"
        );
        assert_eq!(
            spawn_named_and_read_child_env("other-agent", VAR, &[]).await,
            "<unset>",
            "non-Hermes spawns must not receive Hermes defaults"
        );
    }

    #[tokio::test]
    async fn idle_timeout_fires_on_silent_process() {
        let mut client = spawn_script("sleep 10").await;
        let max_dur = std::time::Duration::from_secs(30);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(100),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "expected IdleTimeout, got {result:?}"
        );
    }

    #[tokio::test]
    async fn hard_timeout_fires_when_deadline_is_immediate() {
        let mut client = spawn_script("while true; do echo 'noise'; sleep 0.01; done").await;
        let max_dur = std::time::Duration::from_millis(1);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_secs(60),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(
            matches!(result, Err(AcpError::HardTimeout { .. })),
            "expected HardTimeout, got {result:?}"
        );
    }

    /// `cancel_with_cleanup_grace`'s bounded drain deadline must map to
    /// [`AcpError::CancelDrainTimeout`], never [`AcpError::HardTimeout`] —
    /// the two share an underlying deadline mechanism but must not share
    /// classification, since callers dead-letter a real `HardTimeout` and
    /// must not dead-letter a drain that simply ran past its grace window.
    #[tokio::test]
    async fn cancel_with_cleanup_grace_maps_expiry_to_cancel_drain_timeout() {
        // Agent ignores `session/cancel` on stdin and keeps producing noise
        // forever — never drains within the grace window.
        let mut client = spawn_script("while true; do echo 'noise'; sleep 0.01; done").await;
        client.last_prompt_id = Some(999);
        let grace = std::time::Duration::from_millis(200);
        let result = client
            .cancel_with_cleanup_grace("test-session", grace)
            .await;
        assert!(
            matches!(result, Err(AcpError::CancelDrainTimeout(g)) if g == grace),
            "expected CancelDrainTimeout({grace:?}), got {result:?}"
        );
    }

    #[tokio::test]
    async fn idle_resets_on_stdout_activity() {
        // Send valid JSON (session/update notifications) to reset the idle timer.
        // Non-JSON lines no longer reset idle — only valid JSON notifications do.
        let mut client = spawn_script(
            r#"for i in $(seq 1 10); do echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_thought_chunk","content":{"text":"thinking"}}}}'; sleep 0.05; done; sleep 10"#,
        )
        .await;
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(200),
                hard_deadline,
                max_dur,
            )
            .await;
        let elapsed = start.elapsed();
        // 10 messages × 50ms = ~500ms of activity, then idle timeout fires after 200ms more
        assert!(elapsed >= std::time::Duration::from_millis(400));
        assert!(elapsed < std::time::Duration::from_secs(3));
        assert!(matches!(result, Err(AcpError::IdleTimeout(_))));
    }

    #[tokio::test]
    async fn response_returned_when_matching_id_arrives() {
        let mut client =
            spawn_script(r#"echo '{"jsonrpc":"2.0","id":42,"result":{"stopReason":"end_turn"}}'"#)
                .await;
        let max_dur = std::time::Duration::from_secs(5);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                42,
                std::time::Duration::from_secs(2),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["stopReason"].as_str(), Some("end_turn"));
    }

    #[tokio::test]
    async fn agent_exit_detected_as_eof() {
        let mut client = spawn_script("exit 0").await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let max_dur = std::time::Duration::from_secs(5);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_secs(2),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(matches!(result, Err(AcpError::AgentExited)));
    }

    /// A message with both `id` and `method` is an agent-initiated request,
    /// not a response. The response matcher must not consume it even if the
    /// id happens to match the expected value.
    #[tokio::test]
    async fn agent_request_with_matching_id_not_consumed_as_response() {
        // The script sends an agent-initiated request (has both id and method)
        // whose id matches what we're waiting for (0), then sends the real
        // response. The request should be dispatched (triggering -32601 since
        // "test/method" is unknown), and the real response should be returned.
        let script = r#"
            echo '{"jsonrpc":"2.0","id":0,"method":"test/method","params":{}}'
            read -t 2 _reply
            echo '{"jsonrpc":"2.0","id":0,"result":{"ok":true}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        let max_dur = std::time::Duration::from_secs(5);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                0,
                std::time::Duration::from_secs(3),
                hard_deadline,
                max_dur,
            )
            .await;
        assert!(result.is_ok(), "expected Ok response, got {result:?}");
        assert_eq!(result.unwrap()["ok"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn idle_fires_before_hard_when_idle_is_shorter() {
        let mut client = spawn_script("sleep 10").await;
        let idle = std::time::Duration::from_millis(100);
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout("test", 999, idle, hard_deadline, max_dur)
            .await;
        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "idle should fire before hard when idle << hard, got {result:?}"
        );
    }

    /// Hard-deadline starvation regression (Max's review gate, Eva's required test).
    ///
    /// When the read-loop became a `tokio::select!` with `biased; reader →
    /// steer → sleep_until`, a continuously-ready reader arm could win every
    /// poll and starve the timer arm — silently defeating the hard-deadline
    /// guarantee. The fix is a pre-select deadline check at the top of every
    /// loop iteration; this test pins that behavior.
    ///
    /// Setup: agent emits a **gapless** stream of valid JSON `session/update`
    /// notifications (no `sleep` between lines) so the reader arm is
    /// continuously ready. Each line is valid JSON, so it resets the idle
    /// clock — and we set idle ≫ hard so idle cannot fire first. With
    /// `biased; reader → steer → sleep_until`, the reader arm would win
    /// every poll and `sleep_until` would never be reached. Only the
    /// pre-select deadline check at the top of the loop can stop us.
    ///
    /// Without the pre-select check, this test hangs against the infinite
    /// bash subprocess until the test harness's own outer timeout, and the
    /// returned error would never be `HardTimeout`.
    #[tokio::test]
    async fn hard_deadline_fires_under_continuous_valid_json_stream() {
        // Truly infinite, gapless stream of valid JSON. No `sleep` between
        // echoes — the reader arm is continuously ready, which is the
        // exact starvation scenario the pre-select check guards against.
        // `while :; do echo ...; done` (not a fixed-count `for`) so the
        // subprocess never naturally exits before the hard deadline,
        // regardless of how fast the host drains bash output. Without
        // this, fast hardware drains a bounded loop in < hard_deadline
        // and the reader hits EOF (`AgentExited`) before the timer fires,
        // masking whether the pre-select check actually works.
        let mut client = spawn_script(
            r#"while :; do echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk","content":{"text":"x"}}}}'; done"#,
        )
        .await;
        let hard = std::time::Duration::from_millis(300);
        let hard_deadline = tokio::time::Instant::now() + hard;
        let idle = std::time::Duration::from_secs(60); // idle ≫ hard
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout("test", 999, idle, hard_deadline, hard)
            .await;
        let elapsed = start.elapsed();
        assert!(
            matches!(result, Err(AcpError::HardTimeout { .. })),
            "expected HardTimeout under gapless valid-JSON stream, got {result:?} (elapsed {elapsed:?})"
        );
        // Must fire close to the hard deadline, not late. Without the
        // pre-select check the reader arm starves sleep_until and elapsed
        // tracks the bash subprocess lifetime instead.
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "HardTimeout fired late ({elapsed:?}); reader arm may be starving sleep_until"
        );
    }

    /// Same as `agent_request_with_matching_id_not_consumed_as_response` but
    /// exercises the non-idle `read_until_response` path (via `send_request`).
    #[tokio::test]
    async fn agent_request_not_consumed_via_send_request() {
        // Script: wait for the initialize request, reply, then send an
        // agent-initiated request with id=1 (matching the next send_request id),
        // wait for the -32601 error reply, then send the real response.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 _req
            echo '{"jsonrpc":"2.0","id":1,"method":"test/unknown","params":{}}'
            read -t 2 _err_reply
            echo '{"jsonrpc":"2.0","id":1,"result":{"worked":true}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        // initialize consumes id=0
        let _init = client
            .initialize()
            .await
            .expect("initialize should succeed");
        // send_request uses id=1 — the agent's request with id=1 and method
        // must not be consumed as the response.
        let result = client
            .send_request("test/echo", serde_json::json!({}))
            .await;
        assert!(result.is_ok(), "expected Ok, got {result:?}");
        assert_eq!(result.unwrap()["worked"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn keepalive_resets_idle_past_deadline() {
        // Keepalive session/update lines every 50ms against a 100ms idle deadline.
        // The turn should survive well past the 100ms deadline (proves the fix).
        let mut client = spawn_script(
            r#"for i in $(seq 1 20); do echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"keepalive"}}}'; sleep 0.05; done; sleep 10"#,
        )
        .await;
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(100),
                hard_deadline,
                max_dur,
            )
            .await;
        let elapsed = start.elapsed();
        // 20 keepalives × 50ms = ~1000ms of activity, then idle fires after 100ms more.
        // Must survive well past the 100ms deadline.
        assert!(
            elapsed >= std::time::Duration::from_millis(500),
            "keepalive should reset idle past the deadline; elapsed only {elapsed:?}"
        );
        assert!(elapsed < std::time::Duration::from_secs(5));
        assert!(matches!(result, Err(AcpError::IdleTimeout(_))));
    }

    #[tokio::test]
    async fn tool_call_resets_idle_then_silence_times_out() {
        // A tool_call session/update resets the idle timer (belt-and-suspenders path),
        // then silence causes idle timeout. This proves the reset works for tool_call
        // specifically — not just via the general valid-JSON reset at line 839.
        //
        // The script emits a tool_call, waits 80ms (under the 200ms idle), then goes
        // silent. If the tool_call reset didn't fire, idle would fire at 200ms from
        // start. With the reset, idle fires at 80ms + 200ms = ~280ms from start.
        let mut client = spawn_script(
            r#"echo '{"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"tool_call","title":"long_running","kind":"shell"}}}'; sleep 0.08; sleep 10"#,
        )
        .await;
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let start = std::time::Instant::now();
        let result = client
            .read_until_response_with_idle_timeout(
                "test",
                999,
                std::time::Duration::from_millis(200),
                hard_deadline,
                max_dur,
            )
            .await;
        let elapsed = start.elapsed();
        // The tool_call arrives near-instantly and resets idle.
        // Then 80ms of silence, then idle fires at ~280ms from start.
        // Must be > 200ms (proves the reset happened after the tool_call).
        assert!(
            elapsed >= std::time::Duration::from_millis(200),
            "tool_call should reset idle; elapsed only {elapsed:?}"
        );
        assert!(elapsed < std::time::Duration::from_secs(2));
        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "expected IdleTimeout after silence, got {result:?}"
        );
    }

    #[tokio::test]
    async fn session_new_full_includes_system_prompt_when_some() {
        // Script: respond to initialize, then echo back the session/new request.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full(
                "/tmp",
                vec![],
                Some(SystemPromptTransport::Field("Custom system prompt")),
                None,
            )
            .await
            .expect("session_new_full should succeed");

        assert_eq!(resp.session_id, "ses_test");
        let received = &resp.raw["_receivedRequest"];
        assert_eq!(
            received["params"]["systemPrompt"].as_str(),
            Some("Custom system prompt"),
            "systemPrompt should be included in params when Some"
        );
    }

    #[tokio::test]
    async fn goose_system_prompt_request_uses_set_contract() {
        let script = r#"
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":0,"result":{"_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        let result = client
            .session_set_goose_system_prompt("ses_goose", "Be terse")
            .await
            .expect("custom request succeeds");
        let received = &result["_receivedRequest"];
        assert_eq!(
            received["method"],
            "_goose/unstable/session/system-prompt/set"
        );
        assert_eq!(received["params"]["sessionId"], "ses_goose");
        assert_eq!(received["params"]["mode"], "set");
        assert_eq!(received["params"]["key"], "buzz");
        assert_eq!(received["params"]["text"], "Be terse");
    }

    #[tokio::test]
    async fn goose_system_prompt_preserves_method_not_found_for_fallback() {
        let script = r#"
            read -t 2 _REQ
            echo '{"jsonrpc":"2.0","id":0,"error":{"code":-32601,"message":"Method not found"}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        assert!(matches!(
            client
                .session_set_goose_system_prompt("ses_goose", "Be terse")
                .await,
            Err(AcpError::AgentError { code: -32601, .. })
        ));
    }

    #[tokio::test]
    async fn goose_system_prompt_preserves_invalid_params_as_error() {
        let script = r#"
            read -t 2 _REQ
            echo '{"jsonrpc":"2.0","id":0,"error":{"code":-32602,"message":"Invalid params"}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        assert!(matches!(
            client
                .session_set_goose_system_prompt("ses_goose", "Be terse")
                .await,
            Err(AcpError::AgentError { code: -32602, .. })
        ));
    }

    #[tokio::test]
    async fn session_new_full_omits_system_prompt_when_none() {
        // When system_prompt is None, the field should not appear in params.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full("/tmp", vec![], None, None)
            .await
            .expect("session_new_full should succeed");

        assert_eq!(resp.session_id, "ses_test");
        let received = &resp.raw["_receivedRequest"];
        assert!(
            received["params"]["systemPrompt"].is_null(),
            "systemPrompt should NOT be in params when value is None"
        );
    }

    #[tokio::test]
    async fn session_new_full_sends_session_title_in_meta_when_some() {
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full("/tmp", vec![], None, Some("Fizz · #buzz-dev"))
            .await
            .expect("session_new_full should succeed");

        let received = &resp.raw["_receivedRequest"];
        assert_eq!(
            received["params"]["_meta"]["sessionTitle"].as_str(),
            Some("Fizz · #buzz-dev"),
            "title should ride in _meta.sessionTitle, out of band from the prompt"
        );
    }

    #[tokio::test]
    async fn session_new_full_omits_meta_when_session_title_none() {
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_test","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full("/tmp", vec![], None, None)
            .await
            .expect("session_new_full should succeed");

        let received = &resp.raw["_receivedRequest"];
        assert!(
            received["params"].get("_meta").is_none(),
            "_meta should be absent entirely, not an empty object or null"
        );
    }

    // ── claude-agent-acp _meta.systemPrompt transport ─────────────────────

    #[tokio::test]
    async fn session_new_full_sends_claude_meta_system_prompt_when_claude_meta_transport() {
        // When ClaudeMeta transport is requested, the prompt must appear as
        // _meta.systemPrompt: {"append": text} — never as a bare systemPrompt field.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_claude","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full(
                "/tmp",
                vec![],
                Some(SystemPromptTransport::ClaudeMeta("Be concise")),
                None,
            )
            .await
            .expect("session_new_full should succeed");

        let received = &resp.raw["_receivedRequest"];
        assert!(
            received["params"].get("systemPrompt").is_none(),
            "bare systemPrompt must not be present for ClaudeMeta transport"
        );
        assert_eq!(
            received["params"]["_meta"]["systemPrompt"]["append"].as_str(),
            Some("Be concise"),
            "_meta.systemPrompt.append must carry the prompt text"
        );
    }

    #[tokio::test]
    async fn session_new_full_merges_claude_meta_and_session_title_into_single_meta_object() {
        // Both ClaudeMeta prompt and session_title must coexist under _meta —
        // the prompt must not clobber sessionTitle or vice versa.
        let script = r#"
            read -t 2 _init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
            read -t 2 REQ
            echo '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"ses_merged","_receivedRequest":'"$REQ"'}}'
            sleep 1
        "#;
        let mut client = spawn_script(script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");

        let resp = client
            .session_new_full(
                "/tmp",
                vec![],
                Some(SystemPromptTransport::ClaudeMeta("Be concise")),
                Some("Fizz · #buzz-dev"),
            )
            .await
            .expect("session_new_full should succeed");

        let received = &resp.raw["_receivedRequest"];
        assert_eq!(
            received["params"]["_meta"]["systemPrompt"]["append"].as_str(),
            Some("Be concise"),
            "_meta.systemPrompt.append must be present"
        );
        assert_eq!(
            received["params"]["_meta"]["sessionTitle"].as_str(),
            Some("Fizz · #buzz-dev"),
            "_meta.sessionTitle must be present alongside systemPrompt"
        );
    }

    // ── Goose-native steer scaffold (PR follow-up to #1160) ──────────────

    /// Helper: spawn an inert `cat` subprocess so we have a real AcpClient
    /// to drive `handle_session_update` against. `cat` never writes back,
    /// which is fine — these tests don't read from the agent, they just
    /// feed JSON into the parser.
    async fn spawn_inert_client() -> AcpClient {
        AcpClient::spawn("cat", &[], &[], false)
            .await
            .expect("spawn cat as inert client")
    }

    /// Build a `session/update` JSON-RPC notification carrying a
    /// `session_info_update` with the given `_meta.goose.activeRunId` value.
    /// Pass `None` to omit the `activeRunId` field entirely.
    ///
    /// `_meta` is nested inside the `update` object (per the ACP
    /// `SessionInfoUpdate` schema), matching what goose and buzz-agent
    /// emit on the wire.
    fn session_info_update_msg(active_run_id: Option<serde_json::Value>) -> serde_json::Value {
        let mut goose = serde_json::Map::new();
        if let Some(v) = active_run_id {
            goose.insert("activeRunId".to_string(), v);
        }
        let mut meta = serde_json::Map::new();
        meta.insert("goose".to_string(), serde_json::Value::Object(goose));
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "test-session",
                "update": {
                    "sessionUpdate": "session_info_update",
                    "_meta": serde_json::Value::Object(meta),
                },
            }
        })
    }

    #[tokio::test]
    async fn active_run_id_sets_on_string() {
        let mut client = spawn_inert_client().await;
        assert!(client.active_run_id().is_none(), "starts as None");

        let msg = session_info_update_msg(Some(serde_json::json!("run-abc-123")));
        let _ = client.handle_session_update(&msg);

        assert_eq!(client.active_run_id(), Some("run-abc-123"));
    }

    #[tokio::test]
    async fn active_run_id_clears_on_null() {
        let mut client = spawn_inert_client().await;
        // Set it first
        let set_msg = session_info_update_msg(Some(serde_json::json!("run-xyz")));
        let _ = client.handle_session_update(&set_msg);
        assert_eq!(client.active_run_id(), Some("run-xyz"));

        // Then clear with explicit null
        let clear_msg = session_info_update_msg(Some(serde_json::Value::Null));
        let _ = client.handle_session_update(&clear_msg);
        assert!(
            client.active_run_id().is_none(),
            "explicit null must clear active_run_id"
        );
    }

    #[tokio::test]
    async fn active_run_id_untouched_when_missing() {
        // Field absent entirely — must NOT clear existing state (only an
        // explicit null clears; missing means "no new info this update").
        let mut client = spawn_inert_client().await;
        let set_msg = session_info_update_msg(Some(serde_json::json!("run-stable")));
        let _ = client.handle_session_update(&set_msg);
        assert_eq!(client.active_run_id(), Some("run-stable"));

        // session_info_update with no activeRunId field — leave state alone.
        let missing_msg = session_info_update_msg(None);
        let _ = client.handle_session_update(&missing_msg);
        assert_eq!(
            client.active_run_id(),
            Some("run-stable"),
            "missing activeRunId must leave state untouched"
        );
    }

    #[tokio::test]
    async fn active_run_id_untouched_on_wrong_type() {
        // A number or object in activeRunId is malformed — neither set nor clear.
        let mut client = spawn_inert_client().await;
        let set_msg = session_info_update_msg(Some(serde_json::json!("run-stable")));
        let _ = client.handle_session_update(&set_msg);
        assert_eq!(client.active_run_id(), Some("run-stable"));

        let wrong_type_msg = session_info_update_msg(Some(serde_json::json!(42)));
        let _ = client.handle_session_update(&wrong_type_msg);
        assert_eq!(
            client.active_run_id(),
            Some("run-stable"),
            "non-string/non-null activeRunId must leave state untouched"
        );
    }

    // ── Goose-native steer arm tests ──────────────────────────────────────
    //
    // These exercise the seam between `install_steer_rx` and the read
    // loop's steer arm, isolated from `AgentPool` / `EventQueue` /
    // dispatch. They prove the locked Option-X contract at the read-loop
    // boundary:
    //   1. With `active_run_id == None`, the steer arm acks
    //      `Err(ExpectedRunIdMissing)` and writes nothing — the main
    //      loop's "Err-before-pending" fallback path is reachable.
    //   2. With `active_run_id` set, the steer arm writes the JSON-RPC
    //      request with the matching `expectedRunId` and routes the
    //      response to the ack oneshot as `Success`.
    //
    // We don't test the full mode-gate fork here — that lives in lib.rs
    // and is covered by goose e2e (Eva's lane).

    /// Steer with no `active_run_id` set acks `ExpectedRunIdMissing`
    /// without writing anything. The read loop continues normally and
    /// eventually hits the idle timeout (which is fine — we just need to
    /// observe the ack).
    #[tokio::test]
    async fn native_steer_with_no_active_run_id_acks_expected_run_id_missing() {
        // Quiet process: never emits anything, so the read loop has only
        // the steer arm and the idle timeout to consider.
        let mut client = spawn_script("sleep 10").await;
        assert!(
            client.active_run_id().is_none(),
            "precondition: active_run_id starts as None"
        );

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        // Fire-and-forget: send a SteerRequest from a separate task so
        // the read loop picks it up via the select! arm.
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["test steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        // Drive the read loop with short idle timeout so the test
        // doesn't hang. The expected_id is intentionally never going to
        // be matched (the script writes nothing); the read loop will
        // exit via IdleTimeout shortly after the steer arm fires.
        let idle = std::time::Duration::from_millis(500);
        let max_dur = std::time::Duration::from_secs(5);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let read_result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        // Read loop exit shape: IdleTimeout (no agent activity).
        assert!(
            matches!(read_result, Err(AcpError::IdleTimeout(_))),
            "expected IdleTimeout once steer was acked + script stayed silent, got {read_result:?}"
        );

        // Ack must be ExpectedRunIdMissing — the steer arm bailed out
        // without writing because active_run_id was None at write time.
        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        match ack {
            crate::pool::SteerAck::Err(crate::pool::SteerError::ExpectedRunIdMissing) => {}
            other => panic!("expected SteerAck::Err(ExpectedRunIdMissing), got {other:?}"),
        }
    }

    /// Steer with `active_run_id` set writes the JSON-RPC request and
    /// routes the matching response to the ack oneshot as `Success`.
    /// Verifies the wire shape (`sessionId` + `expectedRunId` + `prompt`)
    /// indirectly: the bash script emits a response keyed by the steer
    /// id (0), and `Success` only fires if the read loop matched that
    /// id to its `pending_steer` entry.
    #[tokio::test]
    async fn native_steer_with_active_run_id_routes_response_to_ack() {
        // Script: pause briefly so the test task can install the steer
        // and we can be sure the response doesn't race ahead of the
        // write — then emit the steer response (id=0 because next_id
        // starts at 0 and the steer is the first request the read loop
        // writes), then idle. This is a JSON-RPC success response with
        // a `stopReason` payload (matching the shape goose uses for
        // steer responses in fake_llm.rs).
        let script = "sleep 0.5; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"stopReason\":\"end_turn\"}}'; \
                      sleep 10";
        let mut client = spawn_script(script).await;

        // Set active_run_id via a synthesized session_info_update so the
        // steer arm has a non-None value to read at write time.
        let update = session_info_update_msg(Some(serde_json::json!("run-42")));
        let _ = client.handle_session_update(&update);
        assert_eq!(client.active_run_id(), Some("run-42"));

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["test steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        // Drive the read loop. Expected_id 999 will never be emitted by
        // the script so the read loop exits via idle timeout after the
        // steer response is routed to ack.
        let idle = std::time::Duration::from_secs(2);
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let read_result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        // Read loop exit: IdleTimeout (no further activity after the
        // routed steer response). AgentExited would also be a valid
        // exit if the bash script terminated early; either is fine —
        // what matters is the ack.
        assert!(
            matches!(
                read_result,
                Err(AcpError::IdleTimeout(_)) | Err(AcpError::AgentExited)
            ),
            "expected IdleTimeout or AgentExited after steer ack, got {read_result:?}"
        );

        // Ack must be Success: the steer response (id=0) was routed to
        // pending_steer.ack_tx.
        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        match ack {
            crate::pool::SteerAck::Success { .. } => {}
            other => panic!("expected SteerAck::Success, got {other:?}"),
        }
    }

    /// Steer-success renewal keeps the turn alive past the original hard
    /// deadline. This is the red-on-old/green-on-new test for the core bug
    /// fix (acp.rs:1440-1444): without renewal, the read loop returns
    /// `HardTimeout` before the prompt response arrives.
    ///
    /// Timeline:
    ///   t≈0:    read loop starts, `hard_deadline = now + 1s`
    ///   t≈0.5s: script emits steer response (id=0) → Success renewal
    ///           moves `hard_deadline` to `now + 3s` (≈3.5s from start)
    ///   t≈1.5s: script emits prompt response (id=999) → `Ok`
    ///
    /// Old code: `HardTimeout` at t≈1s (before prompt response).
    /// New code: deadline renewed at t≈0.5s → prompt response at t≈1.5s → `Ok`.
    #[tokio::test]
    async fn steer_success_renews_hard_deadline_and_survives_past_original() {
        let script = "sleep 0.5; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"stopReason\":\"end_turn\"}}'; \
                      sleep 1; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"done\":true}}'";
        let mut client = spawn_script(script).await;

        let update = session_info_update_msg(Some(serde_json::json!("run-99")));
        let _ = client.handle_session_update(&update);

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        let idle = std::time::Duration::from_secs(10);
        let max_dur = std::time::Duration::from_secs(3);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        assert!(
            result.is_ok(),
            "expected Ok (prompt response after renewed deadline), got {result:?}"
        );
        assert_eq!(result.unwrap()["done"], serde_json::json!(true));

        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        match ack {
            crate::pool::SteerAck::Success { .. } => {}
            other => panic!("expected SteerAck::Success, got {other:?}"),
        }
    }

    // ── Cross-harness steer transport tests ───────────────────────────────
    //
    // These cover the `_session/steering` transport added alongside the
    // goose-native method: capability capture at `initialize`, write-time
    // transport selection, and outcome decoding. Wire-shape assertions read
    // the actual serialized request bytes via `capture_steer_request` rather
    // than inferring the shape from response-id routing.

    /// Spawn a client whose script captures the first line written to its
    /// stdin into `capture_path`, then emits `response` (already-serialized
    /// JSON-RPC) and idles.
    ///
    /// The steer request is the first thing this read loop writes, so the
    /// captured line IS the steer request bytes.
    async fn spawn_steer_capture_script(
        capture_path: &std::path::Path,
        response: &str,
    ) -> AcpClient {
        let script = format!(
            "read -r line; printf '%s' \"$line\" > {capture}; \
             printf '%s\\n' '{response}'; sleep 10",
            capture = capture_path.display(),
            response = response,
        );
        spawn_script(&script).await
    }

    /// Drive one steer through the read loop and return
    /// `(captured_request_bytes, ack)`.
    ///
    /// `capture_path` may be absent afterwards when the arm wrote nothing —
    /// callers assert on that. The read loop is expected to exit via a
    /// timeout or EOF; the ack is what these tests care about.
    async fn run_one_steer(
        client: &mut AcpClient,
        capture_path: &std::path::Path,
    ) -> (Option<String>, crate::pool::SteerAck) {
        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);

        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        let idle = std::time::Duration::from_millis(800);
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let _ = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        let ack = ack_rx
            .await
            .expect("ack oneshot must have received a SteerAck");
        (std::fs::read_to_string(capture_path).ok(), ack)
    }

    /// Unique temp path for one test's captured request bytes.
    fn capture_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("buzz-acp-steer-capture");
        std::fs::create_dir_all(&dir).expect("create capture dir");
        let path = dir.join(format!("{name}.json"));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// Mark a client as having advertised `_meta.steering.supported` without
    /// running a real `initialize` handshake. The capability-parsing tests
    /// cover the handshake itself.
    fn set_steering_supported(client: &mut AcpClient) {
        client.steering_supported = true;
    }

    /// Run `initialize` against a script that replies with `init_result` as
    /// the JSON-RPC result, and return the resulting `steering_supported`.
    async fn steering_supported_after_initialize(init_result: &str) -> bool {
        let script = format!(
            "read -r _init; printf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{result}}}'; \
             sleep 5",
            result = init_result,
        );
        let mut client = spawn_script(&script).await;
        client
            .initialize()
            .await
            .expect("initialize should succeed");
        client.steering_supported()
    }

    /// Test 1a: an adapter advertising `_meta.steering.supported: true`
    /// (claude-agent-acp `src/acp-agent.ts:1444`, codex-acp
    /// `src/CodexAcpServer.ts:247`) is recorded as steering-capable.
    #[tokio::test]
    async fn initialize_records_steering_supported_when_advertised() {
        let supported = steering_supported_after_initialize(
            r#"{"protocolVersion":2,"agentCapabilities":{},"_meta":{"steering":{"supported":true}}}"#,
        )
        .await;
        assert!(
            supported,
            "_meta.steering.supported: true must set steering_supported"
        );
    }

    /// Test 1b: no `_meta` at all (goose, buzz-agent, any older adapter) must
    /// leave the capability off — this is what keeps a steer off the wire for
    /// agents that never implemented it.
    #[tokio::test]
    async fn initialize_leaves_steering_unsupported_when_meta_absent() {
        let supported =
            steering_supported_after_initialize(r#"{"protocolVersion":2,"agentCapabilities":{}}"#)
                .await;
        assert!(
            !supported,
            "absent _meta must leave steering_supported false"
        );
    }

    /// Test 1c: an explicit `supported: false` is respected, not treated as
    /// "the key exists so it must work".
    #[tokio::test]
    async fn initialize_leaves_steering_unsupported_when_explicitly_false() {
        let supported = steering_supported_after_initialize(
            r#"{"protocolVersion":2,"_meta":{"steering":{"supported":false}}}"#,
        )
        .await;
        assert!(
            !supported,
            "_meta.steering.supported: false must leave steering_supported false"
        );
    }

    /// Test 2: no `active_run_id` + capability advertised → the bytes on the
    /// wire are an `_session/steering` request carrying `sessionId` and
    /// `prompt`, and carrying **no** `expectedRunId` (the adapters reject
    /// unknown required fields, and there is no run id to report anyway).
    #[tokio::test]
    async fn acp_steer_request_omits_expected_run_id_and_carries_session_and_prompt() {
        let capture = capture_path("acp_shape");
        let mut client = spawn_steer_capture_script(
            &capture,
            r#"{"jsonrpc":"2.0","id":0,"result":{"outcome":"injected"}}"#,
        )
        .await;
        set_steering_supported(&mut client);
        assert!(
            client.active_run_id().is_none(),
            "precondition: no active_run_id"
        );

        let (written, ack) = run_one_steer(&mut client, &capture).await;

        let written = written.expect("steer request must have been written");
        let msg: serde_json::Value =
            serde_json::from_str(&written).expect("written line must be valid JSON");
        assert_eq!(
            msg["method"].as_str(),
            Some(ACP_STEER_METHOD),
            "must use the cross-adapter steer method; wrote: {written}"
        );
        assert_eq!(msg["params"]["sessionId"].as_str(), Some("sess-test"));
        assert_eq!(
            msg["params"]["prompt"][0]["text"].as_str(),
            Some("steer body"),
            "prompt must carry the steer body as a text block"
        );
        assert!(
            msg["params"].get("expectedRunId").is_none(),
            "_session/steering must not carry expectedRunId; wrote: {written}"
        );
        assert!(
            matches!(ack, crate::pool::SteerAck::Success { .. }),
            "injected outcome must ack Success, got {ack:?}"
        );
    }

    /// Test 3: goose keeps priority. With both an `active_run_id` and the
    /// advertised capability, the goose method wins — `expectedRunId` is
    /// strictly more precise about which run is being steered.
    #[tokio::test]
    async fn goose_transport_wins_when_both_run_id_and_capability_present() {
        let capture = capture_path("goose_priority");
        let mut client =
            spawn_steer_capture_script(&capture, r#"{"jsonrpc":"2.0","id":0,"result":{}}"#).await;
        set_steering_supported(&mut client);
        let update = session_info_update_msg(Some(serde_json::json!("run-77")));
        let _ = client.handle_session_update(&update);

        let (written, ack) = run_one_steer(&mut client, &capture).await;

        let written = written.expect("steer request must have been written");
        let msg: serde_json::Value =
            serde_json::from_str(&written).expect("written line must be valid JSON");
        assert_eq!(
            msg["method"].as_str(),
            Some(GOOSE_STEER_METHOD),
            "goose method must win when a run id exists; wrote: {written}"
        );
        assert_eq!(msg["params"]["expectedRunId"].as_str(), Some("run-77"));
        // A bare `{}` result is a success on the goose transport (goose sends
        // no `outcome`) — the OutcomeRejected guard applies only to
        // `_session/steering`.
        assert!(
            matches!(ack, crate::pool::SteerAck::Success { .. }),
            "goose success result must ack Success, got {ack:?}"
        );
    }

    /// Test 7: codex-acp's third outcome, `failed`
    /// (`src/AcpExtensions.ts:92`), is a delivery rejection despite being a
    /// JSON-RPC success — release the event and fall back.
    #[tokio::test]
    async fn acp_steer_failed_outcome_acks_outcome_rejected() {
        let capture = capture_path("outcome_failed");
        let mut client = spawn_steer_capture_script(
            &capture,
            r#"{"jsonrpc":"2.0","id":0,"result":{"outcome":"failed"}}"#,
        )
        .await;
        set_steering_supported(&mut client);

        let (_written, ack) = run_one_steer(&mut client, &capture).await;

        match ack {
            crate::pool::SteerAck::Err(crate::pool::SteerError::OutcomeRejected { outcome }) => {
                assert_eq!(
                    outcome, "failed",
                    "rejected outcome must report what the agent said, unquoted"
                );
            }
            other => panic!("expected Err(OutcomeRejected), got {other:?}"),
        }
    }

    /// Test 8: **codex `extMethod` silent-loss regression guard.** codex-acp's
    /// ext dispatcher answers unrecognized methods with a bare `{}` — a
    /// JSON-RPC *success*, not `-32601` (`src/CodexAcpServer.ts:255-258`).
    /// Buzz maps `SteerAck::Success` to `queue.remove_event`, so decoding
    /// `{}` as success would delete the user's message with no error, no
    /// fallback, and no log. An absent `outcome` must therefore be a
    /// rejection, which releases the event and fires cancel+merge.
    #[tokio::test]
    async fn acp_steer_missing_outcome_acks_outcome_rejected_and_never_drops_event() {
        let capture = capture_path("outcome_absent");
        let mut client =
            spawn_steer_capture_script(&capture, r#"{"jsonrpc":"2.0","id":0,"result":{}}"#).await;
        set_steering_supported(&mut client);

        let (_written, ack) = run_one_steer(&mut client, &capture).await;

        match ack {
            crate::pool::SteerAck::Err(crate::pool::SteerError::OutcomeRejected { outcome }) => {
                assert_eq!(
                    outcome, "<absent>",
                    "a result with no outcome field must be reported as absent"
                );
            }
            other => panic!(
                "expected Err(OutcomeRejected) for a bare {{}} success — \
                 anything else risks dropping the event, got {other:?}"
            ),
        }
    }

    /// Test 5: `injected` renews the hard deadline, so the turn survives past
    /// its original one. Mirrors
    /// `steer_success_renews_hard_deadline_and_survives_past_original` for
    /// the `_session/steering` transport.
    ///
    /// Timeline: original hard deadline at t≈1s; steer response at t≈0.5s
    /// renews it to t≈3.5s; prompt response at t≈1.5s lands inside it.
    #[tokio::test]
    async fn acp_steer_injected_renews_hard_deadline_and_survives_past_original() {
        let script = "sleep 0.5; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":\"injected\"}}'; \
                      sleep 1; \
                      echo '{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"done\":true}}'";
        let mut client = spawn_script(script).await;
        set_steering_supported(&mut client);

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        let idle = std::time::Duration::from_secs(10);
        let max_dur = std::time::Duration::from_secs(3);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        assert!(
            result.is_ok(),
            "injected must renew the deadline so the prompt response still lands, got {result:?}"
        );
        assert_eq!(result.unwrap()["done"], serde_json::json!(true));
        let ack = ack_rx.await.expect("ack must be received");
        assert!(
            matches!(ack, crate::pool::SteerAck::Success { .. }),
            "injected must ack Success, got {ack:?}"
        );
    }

    /// Test 6: **red/green for the no-renewal rule.** `startedNewTurn` means
    /// the turn Buzz was steering had already ended and the adapter began a
    /// fresh, detached one. It acks `Success` (the message WAS delivered, so
    /// the event must not be redelivered) but must NOT renew the hard
    /// deadline — that clock belongs to a turn which is already settled.
    ///
    /// Same timeline as the `injected` test, so the only difference is the
    /// outcome string: original hard deadline at t≈1s, steer response at
    /// t≈0.5s, prompt response at t≈1.5s. With renewal the prompt response
    /// would land and this returns `Ok`; without renewal the original
    /// deadline fires first and we get `HardTimeout`.
    #[tokio::test]
    async fn acp_steer_started_new_turn_acks_success_without_renewing_hard_deadline() {
        let script = "sleep 0.5; \
             echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":\"startedNewTurn\"}}'; \
             sleep 1; \
             echo '{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"done\":true}}'";
        let mut client = spawn_script(script).await;
        set_steering_supported(&mut client);

        let (steer_tx, steer_rx) = tokio::sync::mpsc::channel::<crate::pool::SteerRequest>(1);
        client.install_steer_rx(steer_rx);
        let (ack_tx, ack_rx) = tokio::sync::oneshot::channel::<crate::pool::SteerAck>();
        let send_task = tokio::spawn(async move {
            steer_tx
                .send(crate::pool::SteerRequest {
                    prompt_blocks: vec!["steer body".into()],
                    ack_tx,
                })
                .await
                .expect("steer_tx send should succeed");
        });

        let idle = std::time::Duration::from_secs(10);
        let max_dur = std::time::Duration::from_secs(3);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let result = client
            .read_until_response_with_idle_timeout("sess-test", 999, idle, hard_deadline, max_dur)
            .await;
        send_task.await.expect("send_task should complete");

        // The original deadline must still fire — renewal here would extend
        // the clock on a turn the adapter has already finished.
        assert!(
            matches!(result, Err(AcpError::HardTimeout { .. })),
            "startedNewTurn must NOT renew the hard deadline, so the original \
             one must still fire; got {result:?}"
        );
        // Delivery still succeeded, so the withheld event must be dropped
        // rather than released — hence Success, not an Err.
        let ack = ack_rx.await.expect("ack must be received");
        assert!(
            matches!(ack, crate::pool::SteerAck::Success { .. }),
            "startedNewTurn is a delivery success, got {ack:?}"
        );
    }

    /// Test 4 (companion to the existing
    /// `native_steer_with_no_active_run_id_acks_expected_run_id_missing`):
    /// no run id AND no advertised capability means nothing is written at
    /// all. This is the gate that keeps a steer off the wire for adapters
    /// that never implemented either method.
    #[tokio::test]
    async fn steer_writes_nothing_when_no_run_id_and_capability_absent() {
        let capture = capture_path("no_transport");
        let mut client =
            spawn_steer_capture_script(&capture, r#"{"jsonrpc":"2.0","id":0,"result":{}}"#).await;
        assert!(!client.steering_supported(), "precondition: not advertised");
        assert!(
            client.active_run_id().is_none(),
            "precondition: no active_run_id"
        );

        let (written, ack) = run_one_steer(&mut client, &capture).await;

        assert!(
            written.is_none(),
            "no transport available must write nothing; wrote: {written:?}"
        );
        match ack {
            crate::pool::SteerAck::Err(crate::pool::SteerError::ExpectedRunIdMissing) => {}
            other => panic!("expected Err(ExpectedRunIdMissing), got {other:?}"),
        }
    }

    // ── Standard ACP prompt-response usage ─────────────────────────────────

    fn prompt_response_usage(
        input: u64,
        output: u64,
        total: u64,
        cached_read: Option<u64>,
        cached_write: Option<u64>,
    ) -> serde_json::Value {
        let mut usage = serde_json::json!({
            "inputTokens": input,
            "outputTokens": output,
            "totalTokens": total,
        });
        if let Some(cached_read) = cached_read {
            usage["cachedReadTokens"] = serde_json::json!(cached_read);
        }
        if let Some(cached_write) = cached_write {
            usage["cachedWriteTokens"] = serde_json::json!(cached_write);
        }
        serde_json::json!({"stopReason": "end_turn", "usage": usage})
    }

    fn standard_cost_update(session_id: &str, cost: f64) -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "usage_update",
                    "cost": {"amount": cost, "currency": "USD"}
                }
            }
        })
    }

    #[tokio::test]
    async fn claude_prompt_response_usage_merges_with_cumulative_cost() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.notify_session_spawned("claude-session");
        client.standard_usage.begin_turn("claude-session");
        client.handle_session_update(&standard_cost_update("claude-session", 0.042));
        assert_eq!(
            client
                .parse_prompt_response(
                    "claude-session",
                    &prompt_response_usage(100, 20, 175, Some(30), Some(25)),
                )
                .unwrap(),
            StopReason::EndTurn
        );

        let usage = client.take_turn_usage().expect("prompt usage");
        assert!(usage.delta_reliable, "response tokens need no baseline");
        assert_eq!(usage.turn_input_tokens, Some(155));
        assert_eq!(usage.turn_output_tokens, Some(20));
        assert_eq!(
            usage.turn_total_tokens, None,
            "Claude total is adapter-derived"
        );
        assert_eq!(usage.turn_cache_read_tokens, Some(30));
        assert_eq!(usage.turn_cache_write_tokens, Some(25));
        assert_eq!(usage.turn_cost_usd, Some(0.042));
        assert_eq!(usage.cumulative_cost_usd, Some(0.042));
        assert_eq!(usage.cumulative_input_tokens, None);
        assert_eq!(usage.cumulative_output_tokens, None);
    }

    #[tokio::test]
    async fn codex_prompt_response_usage_preserves_provider_total_without_cost() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Codex);
        client.standard_usage.begin_turn("codex-session");
        client.handle_session_update(&standard_cost_update("codex-session", 0.042));
        client
            .parse_prompt_response(
                "codex-session",
                &prompt_response_usage(90, 10, 140, Some(40), None),
            )
            .unwrap();

        let usage = client.take_turn_usage().expect("prompt usage");
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, Some(130));
        assert_eq!(usage.turn_output_tokens, Some(10));
        assert_eq!(usage.turn_total_tokens, Some(140));
        assert_eq!(usage.turn_cache_read_tokens, Some(40));
        assert_eq!(usage.turn_cache_write_tokens, None);
        assert_eq!(
            usage.cumulative_cost_usd, None,
            "Codex cost update is ignored"
        );
        assert_eq!(usage.cumulative_input_tokens, None);
        assert_eq!(usage.cumulative_output_tokens, None);
    }

    #[tokio::test]
    async fn standard_prompt_input_overflow_fails_closed() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.standard_usage.begin_turn("overflow-session");
        client
            .parse_prompt_response(
                "overflow-session",
                &prompt_response_usage(u64::MAX, 10, u64::MAX, Some(1), None),
            )
            .unwrap();

        assert!(
            client.take_turn_usage().is_none(),
            "overflow without another valid signal must not emit all-null usage"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_named_adapter_wire_lifecycle_records_prompt_and_cost() {
        let script = r#"
            read -r REQ
            ID=$(printf '%s' "$REQ" | sed -E 's/.*"id":([0-9]+).*/\1/')
            echo '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"wire-session","update":{"sessionUpdate":"usage_update","cost":{"amount":0.5,"currency":"USD"}}}}'
            echo '{"jsonrpc":"2.0","id":'"$ID"',"result":{"stopReason":"end_turn","usage":{"inputTokens":7,"outputTokens":3,"totalTokens":10,"cachedReadTokens":2}}}'
            sleep 1
        "#;
        let (mut client, dir) = spawn_named_script("claude-code", script).await;
        assert_eq!(client.standard_adapter, Some(StandardAdapterKind::Claude));
        client.notify_session_spawned("wire-session");

        let stop = client
            .session_prompt_with_idle_timeout(
                "wire-session",
                "hello",
                std::time::Duration::from_secs(2),
                std::time::Duration::from_secs(5),
            )
            .await
            .expect("wire prompt");
        assert_eq!(stop, StopReason::EndTurn);

        let usage = client.take_turn_usage().expect("wire usage");
        assert_eq!(usage.turn_seq, 1);
        assert_eq!(usage.turn_input_tokens, Some(9));
        assert_eq!(usage.turn_output_tokens, Some(3));
        assert_eq!(usage.turn_cost_usd, Some(0.5));
        assert_eq!(usage.cumulative_cost_usd, Some(0.5));
        drop(client);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn claude_cost_only_record_survives_missing_prompt_usage() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.notify_session_spawned("cost-only-session");
        client.standard_usage.begin_turn("cost-only-session");
        client.handle_session_update(&standard_cost_update("cost-only-session", 0.125));

        let usage = client.take_turn_usage().expect("cost-only usage");
        assert_eq!(usage.turn_seq, 1);
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, None);
        assert_eq!(usage.turn_cost_usd, Some(0.125));
        assert_eq!(usage.cumulative_cost_usd, Some(0.125));
    }

    #[tokio::test]
    async fn attached_claude_session_does_not_invent_first_cost_delta() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.standard_usage.begin_turn("attached-session");
        client.handle_session_update(&standard_cost_update("attached-session", 1.25));
        client
            .parse_prompt_response(
                "attached-session",
                &prompt_response_usage(10, 2, 12, None, None),
            )
            .unwrap();

        let usage = client.take_turn_usage().expect("attached usage");
        assert_eq!(usage.turn_cost_usd, None);
        assert_eq!(usage.cumulative_cost_usd, Some(1.25));
    }

    #[tokio::test]
    async fn standard_usage_two_prompts_preserve_both_monotonic_sequences() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.notify_session_spawned("two-prompt-session");

        client.standard_usage.begin_turn("two-prompt-session");
        client.handle_session_update(&standard_cost_update("two-prompt-session", 0.1));
        client
            .parse_prompt_response(
                "two-prompt-session",
                &prompt_response_usage(10, 2, 12, None, None),
            )
            .unwrap();
        let initial = client.take_turn_usage().expect("initial prompt usage");

        client.standard_usage.begin_turn("two-prompt-session");
        client.handle_session_update(&standard_cost_update("two-prompt-session", 0.25));
        client
            .parse_prompt_response(
                "two-prompt-session",
                &prompt_response_usage(20, 3, 23, None, None),
            )
            .unwrap();
        let user = client.take_turn_usage().expect("user prompt usage");

        assert_eq!((initial.turn_seq, user.turn_seq), (1, 2));
        assert_eq!(
            (initial.turn_input_tokens, user.turn_input_tokens),
            (Some(10), Some(20))
        );
        assert_eq!(
            (initial.turn_cost_usd, user.turn_cost_usd),
            (Some(0.1), Some(0.15))
        );
    }

    #[tokio::test]
    async fn goose_usage_stays_exclusive_and_drains_standard_usage() {
        let mut client = spawn_inert_client().await;
        client.standard_adapter = Some(StandardAdapterKind::Claude);
        client.goose_usage.begin_turn("goose-session");
        client.standard_usage.begin_turn("goose-session");
        client.handle_goose_usage_update(&goose_usage_update_msg("goose-session", 1000, 200, None));
        client
            .parse_prompt_response(
                "goose-session",
                &prompt_response_usage(100, 20, 120, None, None),
            )
            .unwrap();

        let usage = client.take_turn_usage().expect("goose usage");
        assert_eq!(usage.cumulative_input_tokens, Some(1000));
        assert_eq!(
            usage.turn_input_tokens, None,
            "goose first delta remains exclusive"
        );
        assert!(
            client.take_turn_usage().is_none(),
            "standard usage was drained"
        );
    }

    // ── Goose usage notification integration ──────────────────────────────

    /// Build a `_goose/unstable/session/update` JSON-RPC notification.
    fn goose_usage_update_msg(
        session_id: &str,
        input: u64,
        output: u64,
        cost: Option<f64>,
    ) -> serde_json::Value {
        let mut update = serde_json::json!({
            "sessionUpdate": "usage_update",
            "used": input + output,
            "contextLimit": 200000u64,
            "accumulatedInputTokens": input,
            "accumulatedOutputTokens": output,
        });
        if let Some(c) = cost {
            update["accumulatedCost"] = serde_json::json!(c);
        }
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "_goose/unstable/session/update",
            "params": {
                "sessionId": session_id,
                "update": update
            }
        })
    }

    #[tokio::test]
    async fn goose_usage_notification_recorded_and_take_returns_usage() {
        let mut client = spawn_inert_client().await;
        assert!(client.take_turn_usage().is_none(), "starts empty");

        // begin_turn before sending the prompt — mirrors the real call flow.
        client.goose_usage.begin_turn("s1");
        let msg = goose_usage_update_msg("s1", 1000, 200, Some(0.01));
        client.handle_goose_usage_update(&msg);

        let usage = client
            .take_turn_usage()
            .expect("usage should be present after notification");
        assert_eq!(usage.session_id, "s1");
        assert_eq!(usage.turn_seq, 1);
        assert!(!usage.delta_reliable, "first turn must be unreliable");
        assert_eq!(usage.cumulative_input_tokens, Some(1000));
        assert_eq!(usage.cumulative_output_tokens, Some(200));
        assert_eq!(usage.cumulative_cost_usd, Some(0.01));

        // Second take must be None.
        assert!(
            client.take_turn_usage().is_none(),
            "take after drain is None"
        );
    }

    #[tokio::test]
    async fn goose_usage_second_turn_delta_reliable() {
        let mut client = spawn_inert_client().await;
        // Turn 1.
        client.goose_usage.begin_turn("s2");
        client.handle_goose_usage_update(&goose_usage_update_msg("s2", 1000, 200, None));
        let _ = client.take_turn_usage();
        // Turn 2.
        client.goose_usage.begin_turn("s2");
        client.handle_goose_usage_update(&goose_usage_update_msg("s2", 1800, 450, None));
        let usage = client.take_turn_usage().expect("turn 2 usage");
        assert!(usage.delta_reliable);
        assert_eq!(usage.turn_input_tokens, Some(800));
        assert_eq!(usage.turn_output_tokens, Some(250));
    }

    #[tokio::test]
    async fn goose_usage_malformed_notification_does_not_panic() {
        let mut client = spawn_inert_client().await;
        // Missing params entirely.
        let bad = serde_json::json!({"jsonrpc":"2.0","method":"_goose/unstable/session/update"});
        client.handle_goose_usage_update(&bad);
        assert!(client.take_turn_usage().is_none());

        // params present but wrong shape.
        let bad2 = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "_goose/unstable/session/update",
            "params": { "oops": true }
        });
        client.handle_goose_usage_update(&bad2);
        assert!(client.take_turn_usage().is_none());
    }

    #[test]
    fn agent_error_from_json_falls_back_to_full_json_when_message_missing() {
        // Errors without a string `message` field (e.g. only a `data` field) must
        // not be silently truncated to "unknown error" — the full JSON is preserved.
        let error = serde_json::json!({"code": -32000, "data": "quota exceeded"});
        match super::agent_error_from_json(&error) {
            AcpError::AgentError { code, message } => {
                assert_eq!(code, -32000);
                assert!(
                    message.contains("quota exceeded"),
                    "expected full JSON in message, got: {message}"
                );
            }
            other => panic!("expected AgentError, got {other:?}"),
        }
    }

    #[test]
    fn agent_error_from_json_uses_message_field_when_present() {
        let error = serde_json::json!({"code": -32001, "message": "auth denied"});
        match super::agent_error_from_json(&error) {
            AcpError::AgentError { code, message } => {
                assert_eq!(code, -32001);
                assert_eq!(message, "auth denied");
            }
            other => panic!("expected AgentError, got {other:?}"),
        }
    }

    // ── build_codex_config_env ────────────────────────────────────────────────

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const GENERATED: &str = r#"{"sandbox_workspace_write":{"network_access":true}}"#;

    #[test]
    fn build_codex_config_env_returns_none_when_no_codex_config_in_extra_env() {
        // Non-Codex agents: extra_env has no CODEX_CONFIG → None regardless of signal.
        let extra = env(&[("GOOSE_PROVIDER", "openai")]);
        let result = build_codex_config_env(&extra, None, false).unwrap();
        assert_eq!(
            result, None,
            "no CODEX_CONFIG in extra_env must return None"
        );
    }

    #[test]
    fn build_codex_config_env_generated_only_single_entry_with_signal_true_merges_with_parent() {
        // No persona: Buzz injects one CODEX_CONFIG; signal=true.
        // Parent may have its own CODEX_CONFIG — deep_merge applies, network_access forced.
        let extra = env(&[("CODEX_CONFIG", GENERATED)]);
        let parent =
            r#"{"some_operator_key":"val","sandbox_workspace_write":{"operator_key":"keep"}}"#;
        let merged = build_codex_config_env(&extra, Some(parent), true)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // network_access forced true even though only one entry in extra_env.
        assert_eq!(
            v["sandbox_workspace_write"]["network_access"], true,
            "network_access must be forced true with signal=true"
        );
        // Operator key preserved via deep_merge.
        assert_eq!(
            v["sandbox_workspace_write"]["operator_key"], "keep",
            "operator nested key must survive"
        );
        assert_eq!(
            v["some_operator_key"], "val",
            "operator top-level key must survive"
        );
    }

    #[test]
    fn build_codex_config_env_persona_only_signal_false_returns_none() {
        // Persona set CODEX_CONFIG; Buzz did not inject a generated overlay (signal=false).
        // Must return None — no merging, no sandbox widening.
        let persona = r#"{"some_feature":"on"}"#;
        let extra = env(&[("CODEX_CONFIG", persona)]);
        let result = build_codex_config_env(&extra, None, false).unwrap();
        assert_eq!(
            result, None,
            "persona-only CODEX_CONFIG with signal=false must return None"
        );
    }

    #[test]
    fn build_codex_config_env_returns_none_for_persona_only_no_generated_overlay() {
        // Alias: same scenario as above, confirms the old count-based path no longer exists.
        let persona = r#"{"some_feature":"on"}"#;
        let extra = env(&[("CODEX_CONFIG", persona)]);
        let result = build_codex_config_env(&extra, None, false).unwrap();
        assert_eq!(
            result, None,
            "persona-only CODEX_CONFIG with signal=false must return None"
        );
    }

    #[test]
    fn build_codex_config_env_sets_network_access_from_scratch() {
        // Persona + generated overlay, signal=true: network_access is forced true.
        let persona = r#"{}"#;
        let extra = env(&[("CODEX_CONFIG", persona), ("CODEX_CONFIG", GENERATED)]);
        let merged = build_codex_config_env(&extra, None, true).unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["sandbox_workspace_write"]["network_access"], true);
    }

    #[test]
    fn build_codex_config_env_persona_keys_survive_merge() {
        // Persona has CODEX_CONFIG with unrelated keys; generated overlay must
        // force network_access=true without erasing persona keys.
        let persona_cfg = r#"{"some_feature":{"enabled":true}}"#;
        // Config::from_args appends generated AFTER persona env vars.
        let extra = env(&[("CODEX_CONFIG", persona_cfg), ("CODEX_CONFIG", GENERATED)]);
        let merged = build_codex_config_env(&extra, None, true).unwrap().unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            v["some_feature"]["enabled"], true,
            "persona key must survive merge"
        );
        assert_eq!(
            v["sandbox_workspace_write"]["network_access"], true,
            "network_access must be forced true"
        );
    }

    #[test]
    fn build_codex_config_env_nested_persona_keys_survive_when_parent_has_same_top_level_key() {
        // Persona has sandbox_workspace_write.persona_only; parent has
        // sandbox_workspace_write.parent_only.  A flat top-level spread would drop
        // persona_only.  deep_merge must preserve both nested keys, and
        // network_access must be forced true last.
        let persona_cfg = r#"{"sandbox_workspace_write":{"persona_only":"keep_me"}}"#;
        let extra = env(&[("CODEX_CONFIG", persona_cfg), ("CODEX_CONFIG", GENERATED)]);
        let parent = r#"{"sandbox_workspace_write":{"parent_only":"also_here"}}"#;
        let merged = build_codex_config_env(&extra, Some(parent), true)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // Both nested keys survive — no flat-spread drop.
        assert_eq!(
            v["sandbox_workspace_write"]["persona_only"], "keep_me",
            "nested persona key must survive when parent has the same top-level key"
        );
        assert_eq!(
            v["sandbox_workspace_write"]["parent_only"], "also_here",
            "nested parent key must be present"
        );
        // Forced last.
        assert_eq!(
            v["sandbox_workspace_write"]["network_access"], true,
            "network_access must be forced true"
        );
    }

    #[test]
    fn build_codex_config_env_parent_env_wins_on_collisions_persona_keys_survive() {
        // Parent env has CODEX_CONFIG with some keys; persona has different keys.
        // Parent wins on collision; unrelated persona keys survive.
        // network_access is always forced true.
        let persona_cfg = r#"{"persona_key":"persona_val","shared_key":"persona_version"}"#;
        // Config::from_args appends generated AFTER persona env vars.
        let extra = env(&[("CODEX_CONFIG", persona_cfg), ("CODEX_CONFIG", GENERATED)]);
        let parent = r#"{"parent_key":"parent_val","shared_key":"parent_version"}"#;
        let merged = build_codex_config_env(&extra, Some(parent), true)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // Parent-only key present
        assert_eq!(
            v["parent_key"], "parent_val",
            "parent-only key must be present"
        );
        // Unrelated persona key survives (no collision with parent)
        assert_eq!(
            v["persona_key"], "persona_val",
            "unrelated persona key must survive"
        );
        // Collision: parent wins
        assert_eq!(
            v["shared_key"], "parent_version",
            "parent must win on colliding key"
        );
        // network_access always true (forced last)
        assert_eq!(v["sandbox_workspace_write"]["network_access"], true);
    }

    #[test]
    fn build_codex_config_env_parent_has_existing_sandbox_other_keys_survive() {
        // Parent env has sandbox_workspace_write with extra keys; after merge
        // those extra keys survive alongside network_access=true.
        let persona = r#"{}"#;
        let extra = env(&[("CODEX_CONFIG", persona), ("CODEX_CONFIG", GENERATED)]);
        let parent =
            r#"{"sandbox_workspace_write":{"network_access":false,"other_sandbox_key":"val"}}"#;
        let merged = build_codex_config_env(&extra, Some(parent), true)
            .unwrap()
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        // network_access forced true even though parent set false
        assert_eq!(v["sandbox_workspace_write"]["network_access"], true);
        // other_sandbox_key survives (parent's sws merged, then network_access forced)
        assert_eq!(v["sandbox_workspace_write"]["other_sandbox_key"], "val");
    }

    #[test]
    fn build_codex_config_env_errors_on_invalid_persona_json() {
        // Bad persona JSON + generated overlay, signal=true → parse error before merging.
        let extra = env(&[("CODEX_CONFIG", "not-json"), ("CODEX_CONFIG", GENERATED)]);
        let result = build_codex_config_env(&extra, None, true);
        assert!(result.is_err(), "invalid persona JSON must return Err");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("CODEX_CONFIG"),
            "error must mention CODEX_CONFIG"
        );
    }

    #[test]
    fn build_codex_config_env_errors_on_non_object_persona_json() {
        // Non-object persona JSON + generated overlay, signal=true → parse error.
        let extra = env(&[("CODEX_CONFIG", "[1,2,3]"), ("CODEX_CONFIG", GENERATED)]);
        let result = build_codex_config_env(&extra, None, true);
        assert!(result.is_err(), "non-object persona JSON must return Err");
    }

    #[test]
    fn build_codex_config_env_errors_on_invalid_parent_json() {
        let persona = r#"{}"#;
        let extra = env(&[("CODEX_CONFIG", persona), ("CODEX_CONFIG", GENERATED)]);
        let result = build_codex_config_env(&extra, Some("bad-json"), true);
        assert!(result.is_err(), "invalid parent env JSON must return Err");
    }

    #[test]
    fn build_codex_config_env_errors_on_non_object_sandbox_workspace_write() {
        // sandbox_workspace_write must be an object for network_access forcing.
        // If the parent env sets it to a non-object scalar, deep_merge replaces
        // our object with the scalar, and the force step must fail clearly.
        let persona = r#"{}"#;
        let extra = env(&[("CODEX_CONFIG", persona), ("CODEX_CONFIG", GENERATED)]);
        // Parent replaces the object with a scalar — deep_merge: scalar overlay wins.
        let parent = r#"{"sandbox_workspace_write": 42}"#;
        let result = build_codex_config_env(&extra, Some(parent), true);
        assert!(
            result.is_err(),
            "non-object sandbox_workspace_write must return Err"
        );
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("sandbox_workspace_write"),
            "error must mention sandbox_workspace_write"
        );
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ── Permission policy: pinned tests (#4938) ───────────────────────────────
    // ══════════════════════════════════════════════════════════════════════════
    //
    // Tests are grouped by the pinned requirement they cover, labelled as
    // "Pinned §N" matching the spec's numbered list.
    //
    // These tests use:
    //   • `spawn_inert_client()` (cat) for pure unit coverage of `handle_permission_request`.
    //   • `spawn_script(s)` for end-to-end coverage of `read_until_response_with_idle_timeout`.
    //   • `AcpClient::set_permission_config` / `set_owner_pubkey_known` helpers.
    //
    // "observer" is left None for tests that only care about deny/allow path;
    // an in-process observer is installed for tests that verify acp_write events.

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a minimal `session/request_permission` JSON-RPC message.
    fn perm_request(id: u64, options: &[(&str, &str, &str)]) -> serde_json::Value {
        let opts: Vec<serde_json::Value> = options
            .iter()
            .map(|(opt_id, kind, name)| {
                serde_json::json!({"optionId": opt_id, "kind": kind, "name": name})
            })
            .collect();
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "session/request_permission",
            "params": {
                "sessionId": "sess-test",
                "options": opts,
            }
        })
    }

    /// Canonical 3-option set used in most tests.
    fn default_opts() -> &'static [(&'static str, &'static str, &'static str)] {
        &[
            ("opt-allow", "allow_once", "Allow once"),
            ("opt-reject", "reject_once", "Reject once"),
            ("opt-always", "allow_always", "Always allow"),
        ]
    }

    /// A canonical `CardActions` pair for tests that construct a
    /// `PermissionEntry` directly (allow-once `opt-allow`, reject-once
    /// `opt-reject`).
    fn test_card_actions() -> CardActions {
        CardActions {
            allow: serde_json::json!({"optionId":"opt-allow","kind":"allow_once","name":"Allow once"}),
            reject: serde_json::json!({"optionId":"opt-reject","kind":"reject_once","name":"Reject once"}),
        }
    }

    /// Set policy=allow on a client and mark owner known.
    fn set_policy(client: &mut AcpClient, policy: PermissionPolicy) {
        let config = ResolvedPermissionConfig::resolve(policy, None).expect("valid policy");
        client.set_permission_config(config);
        client.set_owner_pubkey_known(true);
    }

    /// Install a matching owner/initiator relay context on `client` so that the
    /// D7-final admission check passes and `handle_permission_request` inserts an
    /// entry as `Publishing` instead of denying synchronously.
    ///
    /// The test_pair publisher auto-ACKs every `PublishEventAcked` command with
    /// `AckOutcome::Accepted`. A background task drains the event receiver so the
    /// channel never fills and blocks the background task inside the publisher.
    ///
    /// Returns the matching owner `Keys` so callers that need a non-owner pubkey
    /// can derive a different key for negative tests.
    fn install_test_relay_context(client: &mut AcpClient) -> Keys {
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        let (publisher, event_rx) = crate::relay::RelayEventPublisher::test_pair();
        // Drain published events so the channel never fills.
        tokio::spawn(async move {
            let mut rx = event_rx;
            while rx.recv().await.is_some() {}
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
            None,
        );
        keys
    }

    // ── Pinned §2: allow selector — unique/zero/multiple/malformed ────────────

    #[test]
    fn allow_selector_picks_unique_allow_once() {
        // Unique allow_once → Ok with that optionId.
        let opts = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"optionId":"opt-a","kind":"allow_once","name":"Allow"},
               {"optionId":"opt-r","kind":"reject_once","name":"Reject"}]"#,
        )
        .unwrap();
        assert_eq!(select_allow_once(&opts), Ok("opt-a".to_string()));
    }

    #[test]
    fn allow_selector_fails_closed_on_zero_allow_once() {
        // No allow_once options → fail closed.
        let opts = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"optionId":"opt-r","kind":"reject_once","name":"Reject"}]"#,
        )
        .unwrap();
        assert!(select_allow_once(&opts).is_err());
    }

    #[test]
    fn allow_selector_fails_closed_on_multiple_allow_once() {
        // Two allow_once candidates → ambiguous, fail closed.
        let opts = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"optionId":"opt-a1","kind":"allow_once","name":"A1"},
               {"optionId":"opt-a2","kind":"allow_once","name":"A2"}]"#,
        )
        .unwrap();
        assert!(select_allow_once(&opts).is_err());
    }

    #[test]
    fn allow_selector_fails_closed_on_missing_option_id() {
        // allow_once present but optionId absent → malformed, fail closed.
        let opts = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"kind":"allow_once","name":"Allow"}]"#,
        )
        .unwrap();
        assert!(select_allow_once(&opts).is_err());
    }

    #[test]
    fn allow_selector_never_selects_allow_always() {
        // allow_always must NOT be selected even when it is the only option
        // with an "allow" kind — indefinite access without per-request approval.
        let opts = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"optionId":"opt-aa","kind":"allow_always","name":"Always"}]"#,
        )
        .unwrap();
        assert!(
            select_allow_once(&opts).is_err(),
            "allow_always must never be auto-selected"
        );
    }

    // ── F1: two-action card contract — select_card_actions + accepts ─────────

    #[test]
    fn select_card_actions_picks_exactly_allow_and_reject_dropping_allow_always() {
        // A request offering allow_once, reject_once, AND allow_always must
        // yield a card carrying only the two ruled actions.
        let opts = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"optionId":"a","kind":"allow_once","name":"Allow"},
               {"optionId":"r","kind":"reject_once","name":"Reject"},
               {"optionId":"aa","kind":"allow_always","name":"Always"}]"#,
        )
        .unwrap();
        let actions = select_card_actions(&opts).expect("must select the two ruled actions");
        assert_eq!(actions.allow_id(), "a");
        assert_eq!(actions.reject_id(), "r");
        // The forbidden allow_always option is neither surfaced nor acceptable.
        assert!(actions.accepts("a"), "allow_once must be accepted");
        assert!(actions.accepts("r"), "reject_once must be accepted");
        assert!(
            !actions.accepts("aa"),
            "allow_always must never be an acceptable decision"
        );
    }

    #[test]
    fn select_card_actions_fails_closed_without_both_actions() {
        // Missing reject_once → fail closed (no card).
        let allow_only = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"optionId":"a","kind":"allow_once","name":"Allow"}]"#,
        )
        .unwrap();
        assert!(select_card_actions(&allow_only).is_err());

        // Missing allow_once → fail closed.
        let reject_only = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"optionId":"r","kind":"reject_once","name":"Reject"}]"#,
        )
        .unwrap();
        assert!(select_card_actions(&reject_only).is_err());

        // Ambiguous (two allow_once) → fail closed.
        let ambiguous = serde_json::from_str::<Vec<serde_json::Value>>(
            r#"[{"optionId":"a1","kind":"allow_once","name":"A1"},
               {"optionId":"a2","kind":"allow_once","name":"A2"},
               {"optionId":"r","kind":"reject_once","name":"R"}]"#,
        )
        .unwrap();
        assert!(select_card_actions(&ambiguous).is_err());
    }

    #[test]
    fn select_card_actions_fails_closed_on_oversized_option_id() {
        // An adversarial adapter embedding an oversized optionId must be
        // rejected before it can inflate the sentinel/DOM.
        let big = "x".repeat(SENTINEL_STRING_MAX_BYTES + 1);
        let opts = serde_json::json!([
            {"optionId": big, "kind": "allow_once", "name": "Allow"},
            {"optionId": "r", "kind": "reject_once", "name": "Reject"},
        ]);
        let opts = opts.as_array().unwrap().clone();
        assert!(
            select_card_actions(&opts).is_err(),
            "an optionId over SENTINEL_STRING_MAX_BYTES must fail closed"
        );
    }

    // ── F3: frozen sentinel byte bounds (producer side) ──────────────────────

    #[test]
    fn build_sentinel_pending_fails_closed_on_oversized_session_id() {
        // The adapter-supplied sessionId is unbounded upstream. An oversized one
        // must abort sentinel construction — never publish a card the Desktop
        // parser rejects (which renders as raw JSON until timeout).
        let actions = test_card_actions();
        let big_session = "s".repeat(SENTINEL_STRING_MAX_BYTES + 1);
        let out = build_sentinel_pending_payload(
            "nonce-abc",
            &actions,
            1_700_000_300,
            Some(&big_session),
            "turn-xyz",
            None,
        );
        assert!(
            out.is_none(),
            "an over-limit sessionId must fail closed (no sentinel)"
        );
    }

    #[test]
    fn build_sentinel_pending_fails_closed_on_oversized_nonce() {
        let actions = test_card_actions();
        let big_nonce = "n".repeat(SENTINEL_STRING_MAX_BYTES + 1);
        let out = build_sentinel_pending_payload(
            &big_nonce,
            &actions,
            1_700_000_300,
            Some("sess"),
            "turn-xyz",
            None,
        );
        assert!(out.is_none(), "an over-limit nonce must fail closed");
    }

    #[test]
    fn build_sentinel_pending_at_session_id_limit_succeeds() {
        // Exactly at the limit must succeed — the gate is not over-tight.
        let actions = test_card_actions();
        let session = "s".repeat(SENTINEL_STRING_MAX_BYTES);
        let out = build_sentinel_pending_payload(
            "nonce-abc",
            &actions,
            1_700_000_300,
            Some(&session),
            "turn-xyz",
            None,
        );
        assert!(
            out.is_some(),
            "a sessionId exactly at SENTINEL_STRING_MAX_BYTES must be accepted"
        );
    }

    #[test]
    fn sentinel_label_truncated_to_byte_limit_on_char_boundary() {
        // A multibyte label over the byte limit is truncated on a char boundary,
        // yielding valid UTF-8 within SENTINEL_STRING_MAX_BYTES that the Desktop
        // byte-bounded parser accepts.
        let big_label = "😀".repeat(60); // 240 UTF-8 bytes
        let opts = serde_json::json!([
            {"optionId":"a","kind":"allow_once","name": big_label},
            {"optionId":"r","kind":"reject_once","name":"Reject"},
        ]);
        let opts = opts.as_array().unwrap().clone();
        let actions = select_card_actions(&opts).expect("two actions");
        let (_, labels) = sentinel_option_fields(&actions);
        let label = labels["a"].as_str().unwrap();
        assert!(
            label.len() <= SENTINEL_STRING_MAX_BYTES,
            "label must be truncated to <= {SENTINEL_STRING_MAX_BYTES} bytes, got {}",
            label.len()
        );
        // Every 😀 is 4 bytes, so a byte-boundary truncation at 200 keeps 50 of
        // them (200 bytes) — never a split scalar.
        assert!(
            label.chars().all(|c| c == '😀'),
            "truncation must land on a char boundary (no mojibake)"
        );
    }

    #[test]
    fn build_sentinel_fails_closed_on_oversized_total_content() {
        // Drive serialize_bounded_sentinel directly to prove the total-content
        // gate rejects an oversized payload. (Per-field-valid input can reach
        // this gate through select_card_actions too — JSON escaping expands
        // control characters, so distinct 200-byte option IDs built from
        // U+0000/U+0001, repeated as optionIds and label keys, inflate the
        // serialized total past SENTINEL_CONTENT_MAX_BYTES — but a synthetic
        // oversized payload exercises the gate in isolation.)
        let mut labels = serde_json::Map::new();
        labels.insert("a".into(), serde_json::json!("x".repeat(200)));
        let payload = serde_json::json!({
            "v": 1,
            "state": "pending",
            "pad": "y".repeat(SENTINEL_CONTENT_MAX_BYTES),
            "labels": labels,
        });
        assert!(
            serialize_bounded_sentinel(&payload).is_none(),
            "total content over SENTINEL_CONTENT_MAX_BYTES must fail closed"
        );
    }

    #[test]
    fn build_sentinel_resolved_fails_closed_on_oversized_chosen_option_id() {
        let actions = test_card_actions();
        let big_chosen = "c".repeat(SENTINEL_STRING_MAX_BYTES + 1);
        let out = build_sentinel_resolved_payload(
            "nonce-abc",
            "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead",
            &actions,
            1_700_000_300,
            Some("sess"),
            "turn-xyz",
            "applied",
            Some(&big_chosen),
            None,
        );
        assert!(
            out.is_none(),
            "an over-limit chosenOptionId must fail closed"
        );
    }

    // ── F2: description field — truncation and omission ───────────────────────

    #[test]
    fn build_sentinel_pending_description_present_and_within_limit() {
        let actions = test_card_actions();
        let out = build_sentinel_pending_payload(
            "nonce-abc",
            &actions,
            1_700_000_300,
            Some("sess"),
            "turn-xyz",
            Some("read a file"),
        )
        .expect("must succeed with a short description");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["description"], "read a file",
            "description must round-trip"
        );
    }

    #[test]
    fn build_sentinel_pending_description_none_omits_field() {
        let actions = test_card_actions();
        let out = build_sentinel_pending_payload(
            "nonce-abc",
            &actions,
            1_700_000_300,
            Some("sess"),
            "turn-xyz",
            None,
        )
        .expect("must succeed without description");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        // `None` produces `"description": null` in the JSON, not a missing key.
        assert!(
            v["description"].is_null(),
            "description must be null when not provided"
        );
    }

    #[test]
    fn build_sentinel_pending_description_truncated_on_producer_side() {
        // An over-limit description is truncated at a char boundary and accepted
        // (display-only — truncate, not reject, matching the labels precedent).
        let actions = test_card_actions();
        let over_limit = "a".repeat(SENTINEL_STRING_MAX_BYTES + 50);
        let out = build_sentinel_pending_payload(
            "nonce-abc",
            &actions,
            1_700_000_300,
            Some("sess"),
            "turn-xyz",
            Some(&over_limit),
        )
        .expect("over-limit description must be truncated and accepted");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let description = v["description"]
            .as_str()
            .expect("description must be a string");
        assert!(
            description.len() <= SENTINEL_STRING_MAX_BYTES,
            "truncated description must be within limit: got {} bytes",
            description.len()
        );
    }

    #[test]
    fn build_sentinel_pending_description_multibyte_truncated_on_char_boundary() {
        // A multibyte description over the byte limit is truncated on a char
        // boundary, yielding valid UTF-8 within SENTINEL_STRING_MAX_BYTES.
        let actions = test_card_actions();
        let big_desc = "😀".repeat(60); // 240 UTF-8 bytes > 200
        let out = build_sentinel_pending_payload(
            "nonce-abc",
            &actions,
            1_700_000_300,
            Some("sess"),
            "turn-xyz",
            Some(&big_desc),
        )
        .expect("multibyte over-limit description must be truncated and accepted");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let desc = v["description"]
            .as_str()
            .expect("description must be a string");
        assert!(
            desc.len() <= SENTINEL_STRING_MAX_BYTES,
            "truncated multibyte description must be within limit"
        );
        // Must be valid UTF-8 — verify by checking no decoding errors.
        assert!(
            std::str::from_utf8(desc.as_bytes()).is_ok(),
            "truncated description must be valid UTF-8"
        );
    }

    // ── F2: description extraction from real producer wire shapes ─────────────
    //
    // These tests call `description_from_request_permission` with verbatim wire
    // shapes produced by real adapters. If the extraction pointer changes, at
    // least one test goes red — preventing the "green tests, dead feature" trap.

    #[test]
    fn description_from_v2_params_title() {
        // buzz-agent v2: `request_permission_params(2, ...)` from wire.rs.
        // `params.title` = call.name; `params.subject.toolCall.rawInput` = call.arguments.
        // The description must include BOTH the tool name AND a summary of rawInput
        // so two calls of the same tool with different commands are distinguishable.
        // Verbatim shape from `request_permission_params` in
        // `crates/buzz-agent/src/wire.rs` (version >= 2 branch).
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-1",
                "title": "fake__shell",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-abc",
                        "title": "fake__shell",
                        "rawInput": {"command": "ls -la /tmp"},
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg);
        // Must include the tool name AND the rawInput summary.
        let desc_str = desc.as_deref().expect("v2 must yield a description");
        assert!(
            desc_str.starts_with("fake__shell("),
            "v2 description must include tool name; got {desc_str:?}"
        );
        assert!(
            desc_str.contains("ls -la /tmp"),
            "v2 description must include rawInput command; got {desc_str:?}"
        );
    }

    #[test]
    fn description_from_v2_two_distinct_commands_are_distinguishable() {
        // Two v2 calls of the SAME tool (fake__shell) with different commands must
        // produce different descriptions — the informed-consent requirement.
        let make_msg = |cmd: &str| {
            serde_json::json!({
                "jsonrpc": "2.0", "id": 1,
                "method": "session/request_permission",
                "params": {
                    "sessionId": "ses-x",
                    "title": "fake__shell",
                    "subject": {
                        "type": "tool_call",
                        "toolCall": {
                            "toolCallId": "tc-x",
                            "title": "fake__shell",
                            "rawInput": {"command": cmd},
                        },
                    },
                    "options": [],
                }
            })
        };
        let desc_ls = description_from_request_permission(&make_msg("ls /tmp"))
            .expect("must yield a description");
        let desc_rm = description_from_request_permission(&make_msg("rm -rf /home"))
            .expect("must yield a description");
        assert_ne!(
            desc_ls, desc_rm,
            "different shell commands must produce distinguishable descriptions"
        );
        assert!(
            desc_ls.contains("ls /tmp"),
            "description must carry the actual command: {desc_ls:?}"
        );
        assert!(
            desc_rm.contains("rm -rf /home"),
            "description must carry the actual command: {desc_rm:?}"
        );
    }

    #[test]
    fn description_from_v2_file_path_argument_is_visible() {
        // A file-read tool call with a path argument must surface the path in the
        // description so the owner can identify which file is being accessed.
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-y",
                "title": "read_file",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-y",
                        "title": "read_file",
                        "rawInput": {"path": "/etc/secrets/api.key"},
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg).expect("must yield a description");
        assert!(
            desc.contains("/etc/secrets/api.key"),
            "file path must appear in description: {desc:?}"
        );
    }

    #[test]
    fn description_from_v2_hostile_markup_in_rawinput_is_safe() {
        // rawInput containing HTML/script tags is stored as a plain JSON string —
        // not interpreted as markup by the extractor. The description is used in
        // a React text node, not dangerouslySetInnerHTML, so the content is safe
        // at render time. This test confirms the extractor does not strip or
        // reject hostile content (stripping would defeat truthfulness).
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-z",
                "title": "eval_code",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-z",
                        "title": "eval_code",
                        "rawInput": {"code": "<script>alert(1)</script>"},
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg).expect("must yield a description");
        // The raw tag text is preserved verbatim (safe in a text node), not stripped.
        assert!(
            desc.contains("script"),
            "hostile markup must be preserved as plain text: {desc:?}"
        );
        // The description is a regular Rust String — valid UTF-8, no panic.
        assert!(!desc.is_empty());
    }

    #[test]
    fn description_from_v2_control_characters_in_rawinput_are_preserved() {
        // Control characters in rawInput are encoded as \uXXXX in JSON, so the
        // compact JSON form is safe ASCII. The extractor does not reject them.
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-ctrl",
                "title": "exec",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-ctrl",
                        "title": "exec",
                        "rawInput": {"cmd": "echo\x00\x01\x1b"},
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg)
            .expect("must yield a description for control-char input");
        assert!(
            desc.starts_with("exec("),
            "description must start with tool name: {desc:?}"
        );
    }

    #[test]
    fn description_from_v2_rawinput_truncated_to_byte_limit() {
        // A very large rawInput command is truncated so the total description
        // fits within DESCRIPTION_COMBINED_MAX_BYTES. The truncated form includes
        // the "…" marker.
        let big_cmd = "x".repeat(500);
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-big",
                "title": "fake__shell",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-big",
                        "title": "fake__shell",
                        "rawInput": {"command": big_cmd},
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg)
            .expect("must yield a description even for oversized rawInput");
        assert!(
            desc.len() <= DESCRIPTION_COMBINED_MAX_BYTES,
            "combined description must be within DESCRIPTION_COMBINED_MAX_BYTES ({DESCRIPTION_COMBINED_MAX_BYTES}): {} bytes, got {desc:?}",
            desc.len()
        );
        // Must include the truncation marker.
        assert!(
            desc.contains('…'),
            "truncated description must contain the ellipsis marker: {desc:?}"
        );
    }

    #[test]
    fn description_from_v2_utf8_truncation_on_char_boundary() {
        // rawInput with multibyte characters (e.g. emoji) is truncated on a
        // character boundary so the result is always valid UTF-8.
        let emoji_cmd = "🚀".repeat(40); // 4 bytes each → 160 bytes > 120 limit
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-utf8",
                "title": "run",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-utf8",
                        "title": "run",
                        "rawInput": {"cmd": emoji_cmd},
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg)
            .expect("must yield a description for multibyte rawInput");
        assert!(
            std::str::from_utf8(desc.as_bytes()).is_ok(),
            "description must be valid UTF-8 after truncation: {desc:?}"
        );
    }

    #[test]
    fn description_from_v1_toolcall_title() {
        // buzz-agent v1: `request_permission_params(1, ...)` from wire.rs.
        // No top-level `title`; `params.toolCall.title` carries the name and
        // `params.toolCall.rawInput` carries the arguments.
        // Also matches codex-acp's permissions-request variant (kind: "other").
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-2",
                "toolCall": {
                    "toolCallId": "tc-def",
                    "title": "read_file",
                    "kind": "other",
                    "rawInput": {"path": "/etc/hosts"},
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg);
        let desc_str = desc.as_deref().expect("v1 must yield a description");
        assert!(
            desc_str.starts_with("read_file("),
            "v1 description must include tool name; got {desc_str:?}"
        );
        assert!(
            desc_str.contains("/etc/hosts"),
            "v1 description must include path argument; got {desc_str:?}"
        );
    }

    #[test]
    fn description_from_v1_two_distinct_commands_are_distinguishable() {
        // Two v1 calls of the SAME tool with different rawInputs must be
        // distinguishable — matching the requirement for v2 above.
        let make_msg = |path: &str| {
            serde_json::json!({
                "jsonrpc": "2.0", "id": 1,
                "method": "session/request_permission",
                "params": {
                    "sessionId": "ses-v1",
                    "toolCall": {
                        "toolCallId": "tc-v1",
                        "title": "read_file",
                        "kind": "other",
                        "rawInput": {"path": path},
                    },
                    "options": [],
                }
            })
        };
        let desc_a = description_from_request_permission(&make_msg("/etc/hosts"))
            .expect("must yield description");
        let desc_b = description_from_request_permission(&make_msg("/etc/shadow"))
            .expect("must yield description");
        assert_ne!(
            desc_a, desc_b,
            "different paths must yield different descriptions"
        );
    }

    #[test]
    fn description_title_only_when_rawinput_is_null() {
        // rawInput = null means no argument context is available; description
        // is the tool name alone.
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-null",
                "title": "noop_tool",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-null",
                        "title": "noop_tool",
                        "rawInput": null,
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg)
            .expect("must yield a description even with null rawInput");
        assert_eq!(desc, "noop_tool", "null rawInput must yield title only");
    }

    // ── Pinned §3: duplicate option IDs ──────────────────────────────────────

    // ── F2 codex-acp v1.1.7 wire shapes ─────────────────────────────────────
    //
    // Verbatim wire shapes from `buildCommandPermissionRequest` and
    // `buildFileChangePermissionRequest` in codex-acp tag v1.1.7
    // (`src/CodexApprovalHandler.ts`). These tests go red if the extraction
    // pointers are changed back to anything that misses the v1.1.7 shapes.

    #[test]
    fn description_from_codex_v1_1_7_command_request() {
        // codex-acp v1.1.7 `buildCommandPermissionRequest`:
        //   { sessionId, toolCall: { toolCallId, kind:"execute", status:"pending",
        //     rawInput: { command, cwd } | null }, options, _meta: { codex: { params } } }
        // No `title` at params or toolCall level — command is in rawInput.command.
        // Verbatim from codex-acp tag v1.1.7 `CodexApprovalHandler.ts:119-135`.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-codex-1",
                "toolCall": {
                    "toolCallId": "tc-cmd-001",
                    "kind": "execute",
                    "status": "pending",
                    "rawInput": {
                        "command": "ls -la /tmp",
                        "cwd": "/home/user"
                    }
                },
                "options": [],
                "_meta": { "codex": { "params": { "command": "ls -la /tmp", "itemId": "tc-cmd-001" } } }
            }
        });
        let desc = description_from_request_permission(&msg);
        assert_eq!(
            desc.as_deref(),
            Some("ls -la /tmp"),
            "codex v1.1.7 command request must extract rawInput.command as description"
        );
    }

    #[test]
    fn description_from_codex_v1_1_7_file_change_request() {
        // codex-acp v1.1.7 `buildFileChangePermissionRequest`:
        //   { sessionId, toolCall: { toolCallId, kind:"edit", status:"pending" },
        //     options, _meta: { codex: { params: { ..., reason? } } } }
        // No title or rawInput — useful context is in _meta.codex.params.reason.
        // Verbatim from codex-acp tag v1.1.7 `CodexApprovalHandler.ts:137-151`.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-codex-2",
                "toolCall": {
                    "toolCallId": "tc-edit-002",
                    "kind": "edit",
                    "status": "pending"
                },
                "options": [],
                "_meta": {
                    "codex": {
                        "params": {
                            "itemId": "tc-edit-002",
                            "reason": "Write updated config to /etc/app.conf"
                        }
                    }
                }
            }
        });
        let desc = description_from_request_permission(&msg);
        assert_eq!(
            desc.as_deref(),
            Some("Write updated config to /etc/app.conf"),
            "codex v1.1.7 file-change request must extract _meta.codex.params.reason as description"
        );
    }

    #[test]
    fn description_returns_none_for_file_change_without_reason() {
        // codex-acp v1.1.7 file-change with no reason field — no fallback available.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-codex-3",
                "toolCall": {
                    "toolCallId": "tc-edit-003",
                    "kind": "edit",
                    "status": "pending"
                },
                "options": [],
                "_meta": { "codex": { "params": { "itemId": "tc-edit-003" } } }
            }
        });
        let desc = description_from_request_permission(&msg);
        assert_eq!(
            desc, None,
            "file-change request with no reason must yield None"
        );
    }

    // ── F1 v2: malformed / null / scalar rawInput / redaction ────────────────
    //
    // These tests cover the new `summarize_raw_input` extraction logic:
    // malformed structures, null values, scalar rawInput (not an object),
    // secret-bearing key redaction, and the combined byte-bound invariant.

    #[test]
    fn description_from_v2_rawinput_scalar_string_yields_title_only() {
        // rawInput is a scalar string (not an object) — summarize_raw_input
        // returns None for non-objects, so the description is the title only.
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "title": "some_tool",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-scalar",
                        "title": "some_tool",
                        "rawInput": "not-an-object",
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg)
            .expect("scalar rawInput must yield title-only description");
        assert_eq!(
            desc, "some_tool",
            "scalar rawInput must yield title only, not produce a panic: {desc:?}"
        );
    }

    #[test]
    fn description_from_v2_rawinput_empty_object_yields_title_only() {
        // rawInput is an empty object {} — summarize_raw_input yields None
        // (no known keys, no fallback scalars), so description is title only.
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "title": "empty_tool",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-empty",
                        "title": "empty_tool",
                        "rawInput": {},
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg)
            .expect("empty rawInput must yield title-only description");
        assert_eq!(
            desc, "empty_tool",
            "empty rawInput must yield title only: {desc:?}"
        );
    }

    #[test]
    fn description_from_v2_rawinput_secret_key_redacted() {
        // rawInput contains a `token` key — a secret-bearing key that must be
        // redacted. The description must NOT expose the token value verbatim.
        // The fallback serialisation includes the key but with "<redacted>" value.
        //
        // Mutation proof: removing `is_secret_key` check from the fallback loop
        // makes the raw token value appear in the description — this assertion
        // would then go red.
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "title": "api_call",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-secret",
                        "title": "api_call",
                        "rawInput": {
                            "token": "super-secret-bearer-12345",
                            "mode": "fast",
                        },
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg)
            .expect("must yield a description with redacted token");
        assert!(
            !desc.contains("super-secret-bearer-12345"),
            "secret token value must not appear verbatim in description: {desc:?}"
        );
        // The non-secret key `mode` may appear.
        assert!(
            desc.contains("fast") || desc.contains("api_call"),
            "description must contain either the non-secret field or the tool name: {desc:?}"
        );
    }

    #[test]
    fn description_from_v2_password_key_redacted() {
        // rawInput contains a `password` key — must be redacted in the fallback.
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "title": "login",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-pwd",
                        "title": "login",
                        "rawInput": {
                            "username": "alice",
                            "password": "hunter2",
                        },
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg)
            .expect("must yield a description with redacted password");
        assert!(
            !desc.contains("hunter2"),
            "password value must not appear verbatim in description: {desc:?}"
        );
    }

    #[test]
    fn description_from_v2_command_key_takes_priority_over_path() {
        // When rawInput has both `command` and `path`, `command` wins (priority 1 > 2).
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "title": "do_thing",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-priority",
                        "title": "do_thing",
                        "rawInput": {
                            "command": "rm -rf /tmp",
                            "path": "/home/user",
                        },
                    },
                },
                "options": [],
            }
        });
        let desc = description_from_request_permission(&msg)
            .expect("must yield description with command priority");
        // command wins over path.
        assert!(
            desc.contains("rm -rf /tmp"),
            "command key must take priority over path key: {desc:?}"
        );
    }

    #[test]
    fn description_combined_byte_bound_invariant() {
        // Regardless of rawInput content, the combined description must never
        // exceed DESCRIPTION_COMBINED_MAX_BYTES (200).
        //
        // Regression proof: the old implementation could produce a combined
        // string up to SENTINEL_STRING_MAX_BYTES + DESCRIPTION_RAW_INPUT_BYTES + 2
        // (322 bytes) because title and summary were budgeted independently.
        // The new implementation computes the total budget from a single cap.
        let long_title = "t".repeat(50);
        let long_cmd = "c".repeat(300);
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "session/request_permission",
            "params": {
                "sessionId": "ses-bound",
                "title": long_title,
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-bound",
                        "title": long_title,
                        "rawInput": {"command": long_cmd},
                    },
                },
                "options": [],
            }
        });
        // Count the bytes of the entire title field (simulating a long title
        // + long command edge case).
        let title_200 = "a".repeat(DESCRIPTION_COMBINED_MAX_BYTES);
        let msg2 = serde_json::json!({
            "jsonrpc": "2.0", "id": 2,
            "method": "session/request_permission",
            "params": {
                "title": title_200,
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-bound2",
                        "title": title_200,
                        "rawInput": {"command": long_cmd},
                    },
                },
                "options": [],
            }
        });
        for (label, m) in [("long_title+long_cmd", &msg), ("max_title+long_cmd", &msg2)] {
            let desc = description_from_request_permission(m).expect("must yield a description");
            assert!(
                desc.len() <= DESCRIPTION_COMBINED_MAX_BYTES,
                "{label}: combined description must fit in {DESCRIPTION_COMBINED_MAX_BYTES} bytes; got {} bytes: {desc:?}",
                desc.len()
            );
        }
    }

    /// Production-seam test: the combined description in the sentinel carries
    /// both the title and a bounded argument context — two different commands
    /// under the same long title produce distinguishable descriptions, and
    /// the distinguishing content survives the `build_sentinel_pending_payload`
    /// serialization path.
    ///
    /// This test asserts:
    ///   1. Both `description_from_request_permission` outputs fit within
    ///      `DESCRIPTION_COMBINED_MAX_BYTES`.
    ///   2. Each extractor output contains the (truncated) title AND part of
    ///      the command — proving neither erases the other.
    ///   3. The two extractor descriptions differ (`assert_ne!`) — proving
    ///      distinguishability even for same-prefix contexts.
    ///   4. The descriptions survive `build_sentinel_pending_payload` unchanged
    ///      (the sentinel serialises the extractor output verbatim; confirming
    ///      that the downstream cap is a no-op for well-formed descriptions).
    ///   5. The two sentinel `description` fields also differ — proving that
    ///      the distinguishability is not lost in the sentinel seam.
    ///
    /// Mutation proofs:
    ///   - Restoring `title_cap_limit = DESCRIPTION_COMBINED_MAX_BYTES` saturates
    ///     `context_budget` to zero for a 200-byte title → no context appended →
    ///     assertions 2/3 fire.
    ///   - Replacing `truncate_to_bytes_head_tail` with `truncate_to_bytes`
    ///     (prefix-only) with same-prefix contexts `sameprefix-a` / `sameprefix-b`
    ///     produces identical truncated prefixes → assertion 3/5 fire.
    #[tokio::test]
    async fn production_seam_description_combined_bound_in_sentinel() {
        // 200-byte title triggers the old saturation: old code → context_budget = 0.
        // New code: title_cap = truncate(200, 185) = 185 bytes,
        //   context_budget = 200 - 185 - 5 = 10 → context preserved.
        let long_title = "t".repeat(DESCRIPTION_COMBINED_MAX_BYTES); // 200 bytes

        // Same-prefix commands: prefix-only truncation to 10 bytes collapses both
        // to "sameprefix" — identical. Head/tail truncation preserves the suffix
        // ("-a" vs "-b") making them distinct.
        let cmd_a = "sameprefix-a".repeat(20); // 240 bytes — requires truncation
        let cmd_b = "sameprefix-b".repeat(20); // same length, different suffix

        let make_msg = |title: &str, cmd: &str| {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 77,
                "method": "session/request_permission",
                "params": {
                    "sessionId": "sess-bound-seam",
                    "title": title,
                    "subject": {
                        "type": "tool_call",
                        "toolCall": {
                            "toolCallId": "tc-seam",
                            "title": title,
                            "rawInput": {"command": cmd},
                        },
                    },
                    "options": [
                        {"optionId": "opt-allow", "kind": "allow_once", "name": "Allow"},
                        {"optionId": "opt-deny",  "kind": "reject_once", "name": "Deny"},
                    ],
                }
            })
        };

        // ── Step 1: extractor outputs ──────────────────────────────────────────
        let desc_a = description_from_request_permission(&make_msg(&long_title, &cmd_a))
            .expect("must yield description for cmd_a");
        let desc_b = description_from_request_permission(&make_msg(&long_title, &cmd_b))
            .expect("must yield description for cmd_b");

        // 1. Both fit the combined budget.
        assert!(
            desc_a.len() <= DESCRIPTION_COMBINED_MAX_BYTES,
            "desc_a must fit in {DESCRIPTION_COMBINED_MAX_BYTES} bytes; got {} bytes: {desc_a:?}",
            desc_a.len()
        );
        assert!(
            desc_b.len() <= DESCRIPTION_COMBINED_MAX_BYTES,
            "desc_b must fit in {DESCRIPTION_COMBINED_MAX_BYTES} bytes; got {} bytes: {desc_b:?}",
            desc_b.len()
        );

        // 2. Each description contains both title context AND argument context.
        //    The title is truncated to 185 bytes (200 - 5 overhead - 10 reserve).
        let title_prefix = &long_title[..185];
        assert!(
            desc_a.starts_with(title_prefix),
            "desc_a must start with the capped title; got: {desc_a:?}"
        );
        // Contains '…' from head/tail context — proves truncation included both ends.
        assert!(
            desc_a.contains('\u{2026}'),
            "desc_a must contain the ellipsis from truncation; got: {desc_a:?}"
        );

        // 3. Same-prefix distinguishability: same title, same prefix in command,
        //    different suffix → descriptions must differ.
        //    Mutation: prefix-only truncation → both collapse to "sameprefix" → assert_ne! fails.
        assert_ne!(
            desc_a, desc_b,
            "descriptions for same-prefix commands must be distinguishable via head/tail truncation; \
             mutation: prefix-only truncation → both collapse to the same prefix → assert_ne! fails"
        );

        // ── Step 2: sentinel seam — descriptions survive build_sentinel_pending_payload ──
        let card_actions = CardActions {
            allow: serde_json::json!({"optionId": "opt-allow", "kind": "allow_once", "name": "Allow"}),
            reject: serde_json::json!({"optionId": "opt-deny",  "kind": "reject_once", "name": "Deny"}),
        };
        let nonce_a = "nonce-seam-a";
        let nonce_b = "nonce-seam-b";
        let expiry = 9_999_999_999u64;
        let turn_id = "turn-seam";

        let payload_a = build_sentinel_pending_payload(
            nonce_a,
            &card_actions,
            expiry,
            Some("sess-seam"),
            turn_id,
            Some(&desc_a),
        )
        .expect("sentinel payload must build for cmd_a");
        let payload_b = build_sentinel_pending_payload(
            nonce_b,
            &card_actions,
            expiry,
            Some("sess-seam"),
            turn_id,
            Some(&desc_b),
        )
        .expect("sentinel payload must build for cmd_b");

        let sentinel_a: serde_json::Value =
            serde_json::from_str(&payload_a).expect("sentinel_a must be valid JSON");
        let sentinel_b: serde_json::Value =
            serde_json::from_str(&payload_b).expect("sentinel_b must be valid JSON");

        let sdesc_a = sentinel_a["description"]
            .as_str()
            .expect("sentinel_a must have a description field");
        let sdesc_b = sentinel_b["description"]
            .as_str()
            .expect("sentinel_b must have a description field");

        // 4. Sentinel descriptions match extractor output (downstream cap is a no-op).
        assert_eq!(
            sdesc_a, desc_a,
            "sentinel description_a must equal the extractor output verbatim"
        );
        assert_eq!(
            sdesc_b, desc_b,
            "sentinel description_b must equal the extractor output verbatim"
        );

        // 5. Sentinel descriptions also differ.
        assert_ne!(
            sdesc_a, sdesc_b,
            "sentinel descriptions for different commands must be distinguishable; \
             mutation: prefix-only context truncation → both sentinel descriptions collapse \
             to the same prefix → assert_ne! fails"
        );
    }

    // ── Pinned §3: duplicate option IDs ──────────────────────────────────────

    #[test]
    fn admission_preflight_rejects_duplicate_option_ids() {
        let id = serde_json::json!(1);
        let msg = perm_request(
            1,
            &[("dup", "allow_once", "A"), ("dup", "reject_once", "R")],
        );
        let opts = msg["params"]["options"].as_array().unwrap().clone();
        let result = run_admission_preflight(
            &id,
            &opts,
            &msg,
            PermissionPolicy::Ask,
            AskGates {
                is_duplicate_id: false,
                is_map_at_cap: false,
                is_publish_in_flight: false,
            },
            (&ObserverContext::default(), None),
        );
        assert!(result.is_err(), "duplicate optionId must fail preflight");
        let reason = result.unwrap_err();
        assert!(
            reason.contains("duplicate optionId"),
            "reason must name the check, got: {reason}"
        );
    }

    // ── Publish-in-flight admission guard ────────────────────────────────────

    #[test]
    fn admission_preflight_rejects_publish_in_flight() {
        // A single sentinel ACK slot is shared across publishes, so at most one
        // entry may be in `Publishing` at a time. When a publish is already in
        // flight the preflight must fail closed. Mutation proof: flipping the
        // flag to `false` makes the same input pass.
        let id = serde_json::json!(2);
        let msg = perm_request(2, default_opts());
        let opts = msg["params"]["options"].as_array().unwrap().clone();
        let result = run_admission_preflight(
            &id,
            &opts,
            &msg,
            PermissionPolicy::Ask,
            AskGates {
                is_duplicate_id: false,
                is_map_at_cap: false,
                is_publish_in_flight: true, // publish already in flight
            },
            (&ObserverContext::default(), None),
        );
        assert!(
            result.is_err(),
            "a request while a publish is in flight must fail preflight"
        );
        assert!(
            result.unwrap_err().contains("publish is already in flight"),
            "reason must name the check"
        );
        // Same input with no publish in flight passes the guard.
        assert!(
            run_admission_preflight(
                &id,
                &opts,
                &msg,
                PermissionPolicy::Ask,
                AskGates {
                    is_duplicate_id: false,
                    is_map_at_cap: false,
                    is_publish_in_flight: false,
                },
                (&ObserverContext::default(), None),
            )
            .is_ok(),
            "identical input must pass when no publish is in flight"
        );
    }

    // ── Publish-in-flight: second Ask denied without disturbing the slot ─────

    #[tokio::test]
    async fn handle_permission_request_denies_second_while_publishing() {
        // A distinct-id Ask request arriving while an earlier one is still in
        // `Publishing` must be denied synchronously by the publish-in-flight
        // guard — never inserting a second Publishing entry that would overwrite
        // the single ACK receiver slot or create an unroutable card.
        //
        // Production-shaped: a full owner/initiator/channel/publisher context is
        // installed and the FIRST entry is created through
        // `handle_permission_request` with a SILENT publisher (never ACKs), so
        // it genuinely reaches and stays in `Publishing`. This exercises the real
        // insertion path after preflight — the acceptance bar is that mutating
        // ONLY the production `is_publish_in_flight` argument (~acp.rs:3102) to
        // `false` turns THIS test red (the second request would then be admitted
        // and overwrite the live ACK slot).
        let mut client = spawn_script("sleep 600").await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        // Silent publisher: never sends an ACK, so the first entry stays Publishing.
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        let (publisher, event_rx) = crate::relay::RelayEventPublisher::test_pair_silent();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while rx.recv().await.is_some() {}
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000007").unwrap()),
            None,
        );
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // First request: reaches Publishing (silent publisher never ACKs).
        let first = perm_request(1, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        client
            .handle_permission_request(&first, hard)
            .await
            .expect("first request must register as Publishing");
        assert!(
            matches!(
                client.pending_permissions.get("1").map(|e| e.state.clone()),
                Some(PermissionEntryState::Publishing)
            ),
            "first entry must be in Publishing state"
        );
        assert!(
            client.sentinel_ack_result_rx.is_some(),
            "first request must install its ACK receiver"
        );

        // Second request with a DIFFERENT id while the first is Publishing.
        let second = perm_request(2, default_opts());
        let result = client.handle_permission_request(&second, hard).await;
        assert!(
            result.is_ok(),
            "publish-in-flight denial must not propagate as Err, got {result:?}"
        );
        // No second entry added — only the original Publishing entry remains.
        assert_eq!(
            client.pending_permissions.len(),
            1,
            "second request must be denied, not registered"
        );
        assert!(
            client.pending_permissions.contains_key("1"),
            "the in-flight Publishing entry must survive untouched"
        );
        // The single ACK slot must still hold the ORIGINAL receiver (not overwritten).
        assert!(
            client.sentinel_ack_result_rx.is_some(),
            "the in-flight ACK receiver must not be overwritten or dropped"
        );
        // The denial reason must explicitly name the publish-in-flight guard —
        // this is what distinguishes it from any unrelated fail-closed gate and
        // makes the production mutation (is_publish_in_flight → false) turn the
        // test red rather than passing via a different denial path.
        let events = obs.snapshot();
        let publish_in_flight_denials: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_read"
                    && e.authorization
                        .as_ref()
                        .and_then(|a| a.reason.as_deref())
                        .map(|r| r.contains("publish is already in flight"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            publish_in_flight_denials.len(),
            1,
            "second request must be denied by the publish-in-flight guard; events: {events:?}"
        );
    }

    // ── Pinned §2: duplicate request ID ──────────────────────────────────────

    #[tokio::test]
    async fn handle_permission_request_denies_duplicate_live_request_id() {
        // Under ask policy, a second request with the same id while the first
        // is still pending must be denied immediately without disturbing the original.
        let mut client = spawn_inert_client().await;
        set_policy(&mut client, PermissionPolicy::Ask);
        // Simulate an already-registered pending entry with the same id.
        client.pending_permissions.insert(
            "1".to_string(),
            PermissionEntry {
                nonce: "nonce-abc".to_string(),
                options_snapshot: vec![],
                card_actions: test_card_actions(),
                state: PermissionEntryState::Pending,
                deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(300),
                expiry_unix_secs: 0,
                sentinel_event_id: None,
                early_decision: None,
                description: None,
            },
        );
        let msg = perm_request(1, default_opts());
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard_deadline).await;
        // Must succeed (Ok) — denial was written and the call itself doesn't error.
        assert!(
            result.is_ok(),
            "duplicate-id must not propagate as Err, got {result:?}"
        );
        // The original entry must still be in the map, untouched.
        assert!(
            client.pending_permissions.contains_key("1"),
            "original pending entry must survive the duplicate-id rejection"
        );
        // Only one entry should exist (the duplicate was denied, not registered).
        assert_eq!(
            client.pending_permissions.len(),
            1,
            "no new entry should be added for the duplicate id"
        );
    }

    // ── Pinned §4: oversize subject → plaintext cap exceeded ─────────────────

    #[test]
    fn admission_preflight_rejects_oversize_msg_exceeding_plaintext_cap() {
        // Construct a message large enough to exceed OBSERVER_MAX_PLAINTEXT_LEN.
        // We embed the large payload directly in the msg so that
        // `serde_json::to_string(msg).len() > OBSERVER_MAX_PLAINTEXT_LEN`.
        let id = serde_json::json!(42);
        let oversize_subject = "x".repeat(OBSERVER_MAX_PLAINTEXT_LEN + 1);
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "session/request_permission",
            "params": {
                "sessionId": "sess",
                "subject": oversize_subject,
                "options": [{"optionId":"opt","kind":"allow_once","name":"A"}]
            }
        });
        let opts = vec![serde_json::json!({"optionId":"opt","kind":"allow_once","name":"A"})];
        let result = run_admission_preflight(
            &id,
            &opts,
            &msg,
            PermissionPolicy::Ask,
            AskGates {
                is_duplicate_id: false,
                is_map_at_cap: false,
                is_publish_in_flight: false,
            },
            (&ObserverContext::default(), None),
        );
        assert!(result.is_err(), "oversize msg must fail preflight");
        let reason = result.unwrap_err();
        assert!(
            reason.contains("too large") || reason.contains("payload"),
            "reason should mention payload size, got: {reason}"
        );
    }

    #[test]
    fn admission_preflight_rejects_payload_overflowing_after_full_event_construction() {
        // Construct a context matching production (UUID-sized IDs) and compute the
        // maximum msg payload that fits within OBSERVER_MAX_PLAINTEXT_LEN when
        // serialised as the actual ObserverEvent. Then submit a payload one byte
        // larger and verify the preflight rejects it.
        //
        // This exercises the production code path: the check constructs the
        // exact ObserverEvent with real context fields, not an estimate.
        use crate::observer::ObserverContext;

        let ctx = ObserverContext {
            channel_id: Some("00000000-0000-0000-0000-000000000000".to_string()),
            session_id: Some("sess-00000000-0000-0000-0000-000000000000".to_string()),
            turn_id: Some("00000000-0000-0000-0000-000000000000".to_string()),
            started_at: Some("2026-01-01T00:00:00.000000000+00:00".to_string()),
        };

        // Binary-search for the exact max subject length that still fits.
        // We wrap it in a minimal msg structure to simulate a real request.
        let template = |subject: &str| {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 42,
                "method": "session/request_permission",
                "params": {
                    "sessionId": "sess",
                    "subject": subject,
                    "options": [{"optionId":"opt","kind":"allow_once","name":"A"}]
                }
            })
        };
        let opts = vec![serde_json::json!({"optionId":"opt","kind":"allow_once","name":"A"})];
        let id = serde_json::json!(42);

        // Build the ObserverEvent exactly as the preflight does to find where the
        // boundary is — then make a msg one byte over that boundary.
        let make_candidate = |msg: &serde_json::Value| ObserverEvent {
            seq: u64::MAX,
            timestamp: "2026-01-01T00:00:00.000000000+00:00".to_string(),
            kind: "acp_read".to_string(),
            agent_index: None,
            channel_id: ctx.channel_id.clone(),
            session_id: ctx.session_id.clone(),
            turn_id: ctx.turn_id.clone(),
            started_at: ctx.started_at.clone(),
            authorization: Some(AuthorizationEnvelope {
                request_nonce: "00000000-0000-0000-0000-000000000000".to_string(),
                actionable: true,
                reason: None,
                expires_at: None,
            }),
            payload: msg.clone(),
        };

        // Find a subject length that overflows after event wrapping.
        // Start with a large subject known to overflow (cap worth of padding).
        let overflow_subject = "z".repeat(OBSERVER_MAX_PLAINTEXT_LEN);
        let overflow_msg = template(&overflow_subject);
        let overflow_event_len = serde_json::to_string(&make_candidate(&overflow_msg))
            .unwrap()
            .len();
        assert!(
            overflow_event_len > OBSERVER_MAX_PLAINTEXT_LEN,
            "test setup: overflow_event_len ({overflow_event_len}) must exceed cap"
        );

        // The preflight must reject this payload.
        let result = run_admission_preflight(
            &id,
            &opts,
            &overflow_msg,
            PermissionPolicy::Ask,
            AskGates {
                is_duplicate_id: false,
                is_map_at_cap: false,
                is_publish_in_flight: false,
            },
            (&ctx, None),
        );
        assert!(
            result.is_err(),
            "payload overflowing after event construction must fail preflight (event_len={overflow_event_len})"
        );
        let reason = result.unwrap_err();
        assert!(
            reason.contains("too large") || reason.contains("payload"),
            "reason should mention payload size, got: {reason}"
        );

        // Sanity-check: an empty subject (tiny msg) must pass the preflight.
        let tiny_msg = template("");
        let tiny_event_len = serde_json::to_string(&make_candidate(&tiny_msg))
            .unwrap()
            .len();
        assert!(
            tiny_event_len <= OBSERVER_MAX_PLAINTEXT_LEN,
            "test setup: tiny_event_len ({tiny_event_len}) must be within cap"
        );
        let ok_result = run_admission_preflight(
            &id,
            &opts,
            &tiny_msg,
            PermissionPolicy::Ask,
            AskGates {
                is_duplicate_id: false,
                is_map_at_cap: false,
                is_publish_in_flight: false,
            },
            (&ctx, None),
        );
        assert!(
            ok_result.is_ok(),
            "small payload must pass preflight, got: {ok_result:?}"
        );
    }

    #[test]
    fn denial_response_with_malformed_reject_once_falls_back_to_cancelled() {
        // A reject_once option with a missing optionId must produce a `cancelled`
        // response, not a Protocol error — the adapter must always receive a valid
        // JSON-RPC response.
        let id = serde_json::json!(7);
        let opts = vec![
            serde_json::json!({"kind": "reject_once", "name": "Reject"}), // no optionId
        ];
        let response = permission_denial_response(&id, &opts)
            .expect("malformed reject_once must not return Err");
        // The response must be a cancelled frame (no optionId in result.outcome).
        let outcome = &response["result"]["outcome"];
        assert_eq!(
            outcome["outcome"].as_str(),
            Some("cancelled"),
            "malformed reject_once must produce cancelled response, got: {response}"
        );
    }

    // ── Pinned §5: map overflow ───────────────────────────────────────────────

    #[tokio::test]
    async fn handle_permission_request_denies_when_map_at_capacity() {
        let mut client = spawn_inert_client().await;
        set_policy(&mut client, PermissionPolicy::Ask);

        // Fill the map to PERMISSION_MAP_CAP.
        for i in 0..PERMISSION_MAP_CAP {
            client.pending_permissions.insert(
                format!("{i}"),
                PermissionEntry {
                    nonce: format!("nonce-{i}"),
                    options_snapshot: vec![],
                    card_actions: test_card_actions(),
                    state: PermissionEntryState::Pending,
                    deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(300),
                    expiry_unix_secs: 0,
                    sentinel_event_id: None,
                    early_decision: None,
                    description: None,
                },
            );
        }
        assert_eq!(client.pending_permissions.len(), PERMISSION_MAP_CAP);

        // One more request with a new id → must be denied.
        let msg = perm_request(99, default_opts());
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard_deadline).await;
        assert!(
            result.is_ok(),
            "map-at-cap must not propagate Err, got {result:?}"
        );
        // Map must not have grown.
        assert_eq!(
            client.pending_permissions.len(),
            PERMISSION_MAP_CAP,
            "map must not grow beyond capacity after denial"
        );
    }

    #[tokio::test]
    async fn handle_permission_request_counts_publishing_toward_capacity() {
        // The cap must count `Publishing` entries too: 7 Pending + 1 Publishing
        // == PERMISSION_MAP_CAP, so a 9th request is denied at the CAPACITY
        // check (which precedes the publish-in-flight check). Mutation proof:
        // excluding `Publishing` from the count drops the total to 7 < cap, so
        // the request instead reaches — and is denied by — the publish-in-flight
        // guard, changing the reason string. Asserting the "at capacity" reason
        // pins that Publishing is counted.
        let mut client = spawn_inert_client().await;
        set_policy(&mut client, PermissionPolicy::Ask);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);

        for i in 0..(PERMISSION_MAP_CAP - 1) {
            client.pending_permissions.insert(
                format!("{i}"),
                PermissionEntry {
                    nonce: format!("nonce-{i}"),
                    options_snapshot: vec![],
                    card_actions: test_card_actions(),
                    state: PermissionEntryState::Pending,
                    deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(300),
                    expiry_unix_secs: 0,
                    sentinel_event_id: None,
                    early_decision: None,
                    description: None,
                },
            );
        }
        // The 8th entry is Publishing (with a live ACK slot).
        client.pending_permissions.insert(
            "pub".to_string(),
            PermissionEntry {
                nonce: "nonce-pub".to_string(),
                options_snapshot: vec![],
                card_actions: test_card_actions(),
                state: PermissionEntryState::Publishing,
                deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(300),
                expiry_unix_secs: 0,
                sentinel_event_id: Some("sentinel-pub".to_string()),
                early_decision: None,
                description: None,
            },
        );
        let (_ack_tx, ack_rx) = tokio::sync::mpsc::channel(1);
        client.sentinel_ack_result_rx = Some(ack_rx);
        assert_eq!(client.pending_permissions.len(), PERMISSION_MAP_CAP);

        let msg = perm_request(99, default_opts());
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard_deadline).await;
        assert!(result.is_ok(), "cap denial must not propagate Err");
        assert_eq!(
            client.pending_permissions.len(),
            PERMISSION_MAP_CAP,
            "map must not grow past the cap"
        );
        // The denial reason must name the capacity check (proving Publishing counts).
        let events = obs.snapshot();
        let cap_reads: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_read"
                    && e.authorization
                        .as_ref()
                        .and_then(|a| a.reason.as_deref())
                        .map(|r| r.contains("at capacity"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            cap_reads.len(),
            1,
            "denial reason must name the capacity check; events: {events:?}"
        );
    }

    // ── Pinned §7: mode matrix — unset + every explicit mode × 3 policies ────

    #[test]
    fn resolved_permission_config_reject_unset_derives_dont_ask() {
        let cfg = ResolvedPermissionConfig::resolve(PermissionPolicy::Reject, None).unwrap();
        assert_eq!(cfg.effective_mode, PermissionMode::DontAsk);
        assert_eq!(cfg.mode_source, ModeSource::Derived);
        assert!(cfg.transmit_mode, "transmit_mode must always be true");
    }

    #[test]
    fn resolved_permission_config_ask_unset_derives_default() {
        let cfg = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        assert_eq!(cfg.effective_mode, PermissionMode::Default);
        assert_eq!(cfg.mode_source, ModeSource::Derived);
    }

    #[test]
    fn resolved_permission_config_allow_unset_derives_default_not_dont_ask() {
        // allow + unset → default (NOT dontAsk — dontAsk self-denies before Buzz can answer)
        let cfg = ResolvedPermissionConfig::resolve(PermissionPolicy::Allow, None).unwrap();
        assert_eq!(cfg.effective_mode, PermissionMode::Default);
        assert!(
            cfg.effective_mode != PermissionMode::DontAsk,
            "allow policy must NOT derive dontAsk"
        );
    }

    #[test]
    fn resolved_permission_config_reject_plus_explicit_dont_ask_is_ok() {
        // reject + dontAsk explicit is valid: both say "deny".
        let cfg = ResolvedPermissionConfig::resolve(
            PermissionPolicy::Reject,
            Some(PermissionMode::DontAsk),
        )
        .unwrap();
        assert_eq!(cfg.effective_mode, PermissionMode::DontAsk);
        assert_eq!(cfg.mode_source, ModeSource::Explicit);
    }

    #[test]
    fn resolved_permission_config_ask_plus_explicit_dont_ask_is_startup_error() {
        let result =
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, Some(PermissionMode::DontAsk));
        assert!(result.is_err(), "ask + dontAsk must be a startup error");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("dontAsk"),
            "error must mention dontAsk, got: {msg}"
        );
    }

    #[test]
    fn resolved_permission_config_allow_plus_explicit_dont_ask_is_startup_error() {
        let result = ResolvedPermissionConfig::resolve(
            PermissionPolicy::Allow,
            Some(PermissionMode::DontAsk),
        );
        assert!(result.is_err(), "allow + dontAsk must be a startup error");
    }

    #[test]
    fn resolved_permission_config_ask_plus_explicit_accept_edits_is_ok() {
        let cfg = ResolvedPermissionConfig::resolve(
            PermissionPolicy::Ask,
            Some(PermissionMode::AcceptEdits),
        )
        .unwrap();
        assert_eq!(cfg.effective_mode, PermissionMode::AcceptEdits);
        assert_eq!(cfg.mode_source, ModeSource::Explicit);
    }

    #[test]
    fn resolved_permission_config_allow_plus_explicit_plan_is_ok() {
        let cfg =
            ResolvedPermissionConfig::resolve(PermissionPolicy::Allow, Some(PermissionMode::Plan))
                .unwrap();
        assert_eq!(cfg.effective_mode, PermissionMode::Plan);
        assert_eq!(cfg.mode_source, ModeSource::Explicit);
    }

    #[test]
    fn resolved_permission_config_transmit_mode_always_true() {
        // transmit_mode is always true regardless of policy/mode combination.
        for policy in [
            PermissionPolicy::Reject,
            PermissionPolicy::Ask,
            PermissionPolicy::Allow,
        ] {
            let cfg = ResolvedPermissionConfig::resolve(policy, None).unwrap();
            assert!(cfg.transmit_mode, "transmit_mode must be true for {policy}");
        }
    }

    // ── Pinned §10: ask availability gate — no observer → downgrade to reject ─

    #[tokio::test]
    async fn ask_without_observer_downgrades_to_reject() {
        // ask policy but no observer installed → must downgrade to reject,
        // never sideways to allow.
        let mut client = spawn_inert_client().await;
        let config = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        client.set_permission_config(config);
        client.set_owner_pubkey_known(true);
        // No observer installed (default).

        let msg = perm_request(1, default_opts());
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard_deadline).await;
        // Denial was written — Ok(true) means caller should suppress generic emit.
        assert!(
            result.is_ok(),
            "ask downgrade to reject must not propagate Err"
        );
        // Confirm nothing was left pending in the map — it was denied synchronously.
        assert!(
            client.pending_permissions.is_empty(),
            "downgraded-to-reject must not leave a pending entry"
        );
    }

    #[tokio::test]
    async fn ask_without_owner_known_downgrades_to_reject() {
        // ask policy with observer but unknown owner → downgrade to reject.
        let mut client = spawn_inert_client().await;
        let config = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        client.set_permission_config(config);
        client.set_owner_pubkey_known(false); // explicitly unknown

        let msg = perm_request(2, default_opts());
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard_deadline).await;
        assert!(result.is_ok());
        assert!(client.pending_permissions.is_empty());
    }

    // ── Production-path tests: real loop emits request, captures nonce ──────

    /// Full end-to-end production path test for the `ask` decision flow:
    ///
    /// 1. Script emits a real `session/request_permission` on stdout.
    /// 2. The read loop processes it via `handle_permission_request()` —
    ///    no state is pre-planted.
    /// 3. The nonce is captured from the observer.
    /// 4. A valid decision is sent through the decision channel.
    /// 5. The loop writes the permission response to the script's stdin.
    /// 6. The script captures the response line into a temp file — the test
    ///    reads the file and asserts the exact JSON-RPC id and option_id at
    ///    the wire level.
    /// 7. The script emits the terminal id=999 reply; the loop returns `Ok`.
    #[tokio::test]
    async fn ask_production_path_emits_request_captures_nonce_and_delivers_decision() {
        // Script: emit permission request, read the harness response into a file
        // so the test can verify what was actually written on the wire, then emit
        // the terminal response.
        let capture_file =
            std::env::temp_dir().join(format!("buzz-acp-wire-{}.json", uuid::Uuid::new_v4()));
        let perm_req = r#"{"jsonrpc":"2.0","id":42,"method":"session/request_permission","params":{"sessionId":"sess","requestId":"req-prod","subject":"read a file","options":[{"optionId":"opt-allow","kind":"allow_once","name":"Allow"},{"optionId":"opt-deny","kind":"reject_once","name":"Deny"}]}}"#;
        let terminal = r#"{"jsonrpc":"2.0","id":999,"result":{"stopReason":"end_turn"}}"#;
        // Read the permission response from harness stdin, save to capture_file,
        // then emit the terminal session/prompt response.
        let script = format!(
            r#"printf '{perm_req}\n'; read -r resp; printf '%s' "$resp" > {capture}; printf '{terminal}\n'"#,
            perm_req = perm_req,
            capture = capture_file.display(),
            terminal = terminal,
        );

        let mut client = spawn_script(&script).await;
        let config = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        client.set_permission_config(config);
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client);

        // Subscribe to the observer BEFORE starting the loop so we capture all events.
        let obs = crate::observer::ObserverHandle::in_process();
        let mut obs_rx = obs.subscribe();
        client.set_observer(Some(obs.clone()), 0);

        let (perm_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Spawn a task that waits for the observer to emit the actionable acp_read
        // (the permission request), then delivers a matching decision.
        let decision_task = tokio::spawn(async move {
            // Wait for the actionable acp_read from the observer.
            let mut found_nonce: Option<String> = None;
            while let Ok(Ok(event)) =
                tokio::time::timeout(std::time::Duration::from_secs(5), obs_rx.recv()).await
            {
                if event.kind == "acp_read" {
                    if let Some(auth) = &event.authorization {
                        if auth.actionable {
                            found_nonce = Some(auth.request_nonce.clone());
                            break;
                        }
                    }
                }
            }
            let nonce = found_nonce.expect("actionable acp_read must be emitted");
            // Deliver a valid decision by the captured nonce.
            perm_tx
                .send(PermissionDecision {
                    request_nonce: nonce,
                    option_id: "opt-allow".to_string(),
                })
                .await
                .expect("decision channel must accept");
        });

        let idle = std::time::Duration::from_secs(5);
        let max_dur = std::time::Duration::from_secs(15);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout("sess", 999, idle, hard_deadline, max_dur)
            .await;

        assert!(
            result.is_ok(),
            "production-path ask loop must succeed after decision is delivered, got: {result:?}"
        );
        assert_eq!(
            result.unwrap().get("stopReason").and_then(|v| v.as_str()),
            Some("end_turn"),
        );

        // Verify the observer emitted an authorized acp_write (the decision response).
        let _ = decision_task.await;
        let events = obs.snapshot();
        let write_events: Vec<_> = events
            .iter()
            .filter(|e| e.kind == "acp_write" && e.authorization.is_some())
            .collect();
        assert!(
            !write_events.is_empty(),
            "observer must emit at least one authorized acp_write after decision applied"
        );

        // Wire-level assertion: read what the harness actually wrote on the pipe.
        // The capture file contains the raw NDJSON line the agent's stdin received.
        let wire_line = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::task::spawn_blocking({
                let capture_file = capture_file.clone();
                move || {
                    // Poll briefly for the file to be populated.
                    for _ in 0..20 {
                        if let Ok(s) = std::fs::read_to_string(&capture_file) {
                            if !s.is_empty() {
                                return s;
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    String::new()
                }
            }),
        )
        .await
        .expect("timeout reading wire capture")
        .expect("spawn_blocking failed");

        let _ = std::fs::remove_file(&capture_file);

        assert!(
            !wire_line.is_empty(),
            "harness must write a permission response on the wire (capture file was empty)"
        );
        let wire_json: serde_json::Value =
            serde_json::from_str(&wire_line).expect("wire response must be valid JSON");
        assert_eq!(
            wire_json["id"],
            serde_json::json!(42),
            "wire response id must match the permission request id=42"
        );
        let outcome = &wire_json["result"]["outcome"];
        assert_eq!(
            outcome["outcome"].as_str(),
            Some("selected"),
            "wire response must carry selected outcome for an approved decision"
        );
        assert_eq!(
            outcome["optionId"].as_str(),
            Some("opt-allow"),
            "wire response optionId must match the delivered decision"
        );
    }

    /// Cancel test: asserts exactly one JSON-RPC response per pending id, no
    /// replay on subsequent cancel. Proves behavior at the wire level by
    /// capturing the raw NDJSON lines written to the agent's stdin.
    #[tokio::test]
    async fn cancel_writes_exactly_one_response_per_pending_id_no_replay() {
        // Script: read all stdin lines (cancel responses) into a capture file,
        // then stay alive briefly.
        let capture_file =
            std::env::temp_dir().join(format!("buzz-acp-cancel-{}.ndjson", uuid::Uuid::new_v4()));
        // Loop reading stdin, appending each line to capture file, exit on EOF.
        let script = format!(
            r#"while IFS= read -r line; do printf '%s\n' "$line" >> {capture}; done; sleep 2"#,
            capture = capture_file.display(),
        );
        let mut client = spawn_script(&script).await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client);

        // Subscribe to observer to capture writes.
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);

        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Register two distinct Pending entries via the production path.
        let mut expected_ids: Vec<u64> = Vec::new();
        let mut expected_nonces: Vec<String> = Vec::new();
        for i in 0..2u64 {
            let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
            let msg = perm_request(i, default_opts());
            client
                .handle_permission_request(&msg, hard_deadline)
                .await
                .expect("ask registration must succeed");
            // In production the relay ACK transitions the entry Publishing →
            // Pending in the read loop before the next request arrives, so two
            // Pending entries legitimately coexist. This test does not drive the
            // loop between registrations, so apply that transition explicitly —
            // otherwise the publish-in-flight guard denies the second request.
            if let Some(entry) = client.pending_permissions.get_mut(&i.to_string()) {
                entry.state = PermissionEntryState::Pending;
            }
            // Capture the nonce that was bound to this entry.
            let nonce = client
                .pending_permissions
                .get(&i.to_string())
                .expect("entry must be registered")
                .nonce
                .clone();
            expected_ids.push(i);
            expected_nonces.push(nonce);
        }
        assert_eq!(
            client.pending_permissions.len(),
            2,
            "two pending entries must be registered before cancel"
        );
        client.last_prompt_id = Some(999);

        // First cancel: must drain both entries and write exactly two responses.
        let _ = client
            .cancel_with_cleanup_grace("sess-exact-once", std::time::Duration::from_millis(200))
            .await;
        assert!(
            client.pending_permissions.is_empty(),
            "all pending entries must be drained after cancel"
        );

        // Give the script a moment to flush appended lines.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Wire-level assertion: read capture file and parse each line.
        let wire_lines = tokio::task::spawn_blocking({
            let capture_file = capture_file.clone();
            move || {
                for _ in 0..20 {
                    if let Ok(s) = std::fs::read_to_string(&capture_file) {
                        let lines: Vec<String> = s
                            .lines()
                            .filter(|l| !l.is_empty())
                            .map(|l| l.to_string())
                            .collect();
                        if lines.len() >= 2 {
                            return lines;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                vec![]
            }
        })
        .await
        .expect("spawn_blocking failed");
        let _ = std::fs::remove_file(&capture_file);

        // Two wire responses must have been written (one per pending entry).
        // Note: session/cancel also writes to stdin; filter to permission responses only.
        let perm_responses: Vec<serde_json::Value> = wire_lines
            .iter()
            .filter_map(|l| serde_json::from_str(l).ok())
            .filter(|v: &serde_json::Value| {
                // Permission responses have {"id": <num>, "result": {"outcome": {...}}}
                // (no "method" key).
                v.get("result").and_then(|r| r.get("outcome")).is_some()
            })
            .collect();

        assert_eq!(
            perm_responses.len(),
            2,
            "cancel must write exactly two permission responses on the wire (one per pending id), got: {perm_responses:?}"
        );

        // Each response must carry one of the registered ids and have a rejection outcome.
        let written_ids: Vec<u64> = perm_responses
            .iter()
            .filter_map(|v| v["id"].as_u64())
            .collect();
        for expected_id in &expected_ids {
            assert!(
                written_ids.contains(expected_id),
                "wire responses must cover id={expected_id}, got: {written_ids:?}"
            );
        }

        // Observer-level: nonces must match registered entries.
        let events_after_first = obs.snapshot();
        let cancel_nonces: Vec<String> = events_after_first
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("cancelled"))
                        .unwrap_or(false)
            })
            .filter_map(|e| e.authorization.as_ref().map(|a| a.request_nonce.clone()))
            .collect();
        assert_eq!(
            cancel_nonces.len(),
            2,
            "cancel must emit exactly one authorized acp_write per pending id, got: {cancel_nonces:?}"
        );
        for nonce in &cancel_nonces {
            assert!(
                expected_nonces.contains(nonce),
                "emitted cancel nonce {nonce:?} does not match any registered entry nonce"
            );
        }

        // Second cancel on the same client: no pending entries remain, must not
        // re-emit any additional acp_write (no replay).
        let _ = client
            .cancel_with_cleanup_grace("sess-exact-once", std::time::Duration::from_millis(200))
            .await;
        let events_after_second = obs.snapshot();
        let write_count_after_second = events_after_second
            .iter()
            .filter(|e| e.kind == "acp_write" && e.authorization.is_some())
            .count();
        assert_eq!(
            write_count_after_second, 2,
            "second cancel must not emit additional acp_writes (no replay)"
        );
    }

    /// Paused-time test — Part 1: at exactly 299s, the pending entry still exists
    /// and the loop has NOT timed out.
    ///
    /// Uses a single continuously running loop advanced to 299s then hard-stopped.
    /// Asserts the loop returned an external (outer) timeout, not an internal deadline,
    /// AND the entry is still Pending in the map — proving idle suspension works.
    #[tokio::test(start_paused = true)]
    async fn ask_permission_pending_at_299_seconds() {
        let mut client = spawn_script("sleep 600").await;
        let config = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        client.set_permission_config(config);
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Register one pending entry — deadline is now + 300s.
        let msg = perm_request(1, default_opts());
        let hard_deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS + 10);
        client
            .handle_permission_request(&msg, hard_deadline)
            .await
            .expect("ask registration must succeed");
        assert_eq!(client.pending_permissions.len(), 1, "entry registered");

        // Idle is 5s — would fire immediately if not suspended.
        let idle = std::time::Duration::from_secs(5);
        let max_dur = std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS + 10);
        let hard_deadline2 = tokio::time::Instant::now() + max_dur;

        // Advance virtual time to 299s concurrently with the running loop.
        // The loop must be running to process the advance; the outer real-time
        // timeout (50ms wall clock) is the expected exit path.
        let loop_fut = client.read_until_response_with_idle_timeout(
            "sess-299s",
            999,
            idle,
            hard_deadline2,
            max_dur,
        );
        let result = tokio::select! {
            r = loop_fut => Some(r),
            _ = async {
                tokio::time::advance(std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS - 1)).await;
            } => None,
        };

        // Loop must still be pending (returned None from the select advance branch).
        // If result is Some, the loop exited — which means it timed out internally.
        assert!(
            result.is_none(),
            "loop must still be running at 299s (idle suspended); \
             it exited with: {result:?}"
        );
        // Entry must still be Pending in the map at 299s.
        assert!(
            client.pending_permissions.contains_key("1"),
            "entry must still be Pending at 299s"
        );
        // No timed_out acp_write must have been emitted yet.
        let events = obs.snapshot();
        let timeout_writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("timed_out"))
                        .unwrap_or(false)
            })
            .collect();
        assert!(
            timeout_writes.is_empty(),
            "no timed_out write must be emitted at 299s; got: {timeout_writes:?}"
        );
    }

    /// Paused-time test — Part 2: the permission deadline fires at exactly 300s.
    ///
    /// Runs the loop continuously and advances virtual time to 300s. Asserts:
    /// - The entry is removed from the map (deadline processed).
    /// - Exactly one `timed_out` authorized `acp_write` is emitted in the observer.
    /// - The loop exits via `HardTimeout` (not `PermissionPoisoned`).
    #[tokio::test(start_paused = true)]
    async fn ask_permission_deadline_fires_at_exactly_300_seconds() {
        let mut client = spawn_script("sleep 600").await;
        let config = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        client.set_permission_config(config);
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // hard_deadline is equal to the permission deadline — exercises the
        // equality case fixed in this round.
        let now = tokio::time::Instant::now();
        let perm_deadline = now + std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS);
        // Use the same deadline for both the entry and the hard deadline.
        let msg = perm_request(1, default_opts());
        client
            .handle_permission_request(&msg, perm_deadline)
            .await
            .expect("ask registration must succeed");
        assert_eq!(client.pending_permissions.len(), 1, "entry registered");

        let idle = std::time::Duration::from_secs(5);
        let max_dur = std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS + 10);
        // Loop hard deadline is generous — permission deadline (== hard_deadline passed
        // to handle_permission_request) is the one that must fire.
        let loop_hard = tokio::time::Instant::now() + max_dur;

        // Run the loop and advance virtual time to 300s concurrently.
        let loop_result = tokio::select! {
            r = client.read_until_response_with_idle_timeout("sess-300s", 999, idle, loop_hard, max_dur) => Some(r),
            _ = async {
                // Advance 1ms past the 300s permission deadline.
                tokio::time::advance(std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS) + std::time::Duration::from_millis(1)).await;
            } => None,
        };

        // The loop MUST complete (not be cancelled by the select branch):
        // the advance fires and triggers the expiry block, which should
        // process the entry and return HardTimeout (since entry.deadline == hard_deadline).
        // If it comes back None, advance happened before the loop could react — tolerate
        // this only if the entry is removed.
        let entry_removed = !client.pending_permissions.contains_key("1");

        // Verify the observer emitted exactly one timed_out write.
        let events = obs.snapshot();
        let timeout_writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("timed_out"))
                        .unwrap_or(false)
            })
            .collect();

        // Either the loop completed with HardTimeout after writing timed_out,
        // or the advance preempted it — in the latter case we at minimum need
        // to confirm the entry WAS processed (removed) on the next loop iteration.
        // Allow for either pattern since tokio::select non-determinism can fire
        // the advance arm first; what must hold is: once we drive the loop once more,
        // the entry is gone and one timed_out was written.
        if loop_result.is_none() {
            // Advance won the select — drive the loop one more iteration to process expiry.
            let drive_result = tokio::select! {
                r = client.read_until_response_with_idle_timeout("sess-300s", 999, idle, loop_hard, max_dur) => Some(r),
                _ = async {
                    tokio::time::advance(std::time::Duration::from_millis(100)).await;
                } => None,
            };
            let _ = drive_result;
        }

        // Now assert invariants.
        assert!(
            !client.pending_permissions.contains_key("1"),
            "entry must be removed after 300s permission deadline"
        );
        let events2 = obs.snapshot();
        let timeout_writes2: Vec<_> = events2
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("timed_out"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            timeout_writes2.len(),
            1,
            "exactly one timed_out acp_write must be emitted at 300s; got: {timeout_writes2:?}"
        );
        let _ = entry_removed;
        let _ = timeout_writes;
    }

    /// Deadline-equality test: `entry.deadline == loop_hard_deadline`.
    ///
    /// When a request is registered within 300s of the turn hard cap,
    /// `entry.deadline = min(now + 300s, hard_deadline) = hard_deadline`.
    ///
    /// The pre-select check must NOT return `HardTimeout` before processing the
    /// expired entry — it must write the fail-closed denial first, THEN return
    /// `HardTimeout`.  This test proves the fix: equal deadlines → denial written.
    ///
    /// Wire-level proof: the denial line is captured from child stdin NDJSON and
    /// parsed to confirm it contains exactly one `timed_out` response for id=1
    /// before `HardTimeout` is returned.
    #[tokio::test(start_paused = true)]
    async fn ask_permission_entry_deadline_equal_to_loop_hard_deadline_writes_denial_before_exit() {
        // Proves: when entry.deadline == loop_hard_deadline, the fail-closed denial
        // is written to the pipe exactly once BEFORE HardTimeout is returned.
        //
        // Proof strategy:
        //   1. tokio::spawn keeps the loop future alive continuously (no drops/restarts).
        //   2. Virtual time advances past the shared deadline; loop returns HardTimeout.
        //   3. Attempt counter (incremented before I/O in write_ndjson_inner) asserts
        //      exactly one write attempt — distinguishes "stopped after first" from
        //      "tried all and all failed".
        //   4. Observer payload asserts the exact fail-closed JSON written to the pipe:
        //      the observer records the same serde_json::Value that is serialised and
        //      written; with emit_observe=true in write_ndjson_inner this is identical
        //      to what the adapter receives.
        //
        // File-capture is not used because start_paused = true makes real-time I/O
        // between the harness and the shell subprocess unreliable for test assertions
        // (virtual-time advance does not advance wall-clock for OS file flushing).
        let mut client = spawn_script("sleep 600").await;
        let config = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        client.set_permission_config(config);
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Install the attempt counter — proves exactly one write attempt.
        let attempt_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        client.set_write_attempt_count(attempt_counter.clone());

        // Set entry.deadline == loop_hard_deadline.
        // With PERMISSION_ASK_TIMEOUT_SECS = 300:
        //   entry.deadline = min(now + 300s, hard_deadline) = now + 300s = hard_deadline.
        let now = tokio::time::Instant::now();
        let shared_deadline = now + std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS);

        let msg = perm_request(1, default_opts());
        client
            .handle_permission_request(&msg, shared_deadline)
            .await
            .expect("ask registration must succeed");
        assert_eq!(client.pending_permissions.len(), 1, "entry registered");

        // Move the client into a spawned task so it stays alive across the
        // virtual-time advance — mirrors the idle-rearm test pattern.  The task
        // owns the loop future continuously from start to finish (no drops, no
        // restarts) while the test body drives time from the outside.
        let idle = std::time::Duration::from_secs(5);
        let max_dur = std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS);
        let loop_task = tokio::spawn(async move {
            client
                .read_until_response_with_idle_timeout(
                    "sess-eq",
                    999,
                    idle,
                    shared_deadline,
                    max_dur,
                )
                .await
        });

        // Advance virtual time past the shared deadline.  The loop task wakes,
        // processes the expired entry (writes the fail-closed denial), and then
        // returns HardTimeout because entry.deadline == hard_deadline.
        tokio::time::advance(
            std::time::Duration::from_secs(PERMISSION_ASK_TIMEOUT_SECS)
                + std::time::Duration::from_millis(1),
        )
        .await;

        // Await the continuously running loop and assert HardTimeout — not any
        // other error and not Ok (Ok would mean a terminal session/prompt response
        // was read instead of the hard deadline firing).
        let loop_result = loop_task.await.expect("loop task must not panic");
        assert!(
            matches!(loop_result, Err(AcpError::HardTimeout { .. })),
            "loop must exit with HardTimeout after equality deadline fires; got: {loop_result:?}"
        );

        // Assert exactly ONE write attempt — the fail-closed denial for id=1.
        // Counter increments at the top of write_ndjson_inner before I/O;
        // a value > 1 would mean a duplicate write escaped the expiry block.
        let attempts = attempt_counter.load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            attempts, 1,
            "exactly one write attempt must be made (the timed-out denial for id=1); \
             got {attempts} attempts"
        );

        // Exact payload proof via observer telemetry.
        // write_ndjson_inner calls observe("acp_write", value) with emit_observe=true
        // using the same serde_json::Value that was serialised to the pipe — the
        // observer record IS the wire content for virtual-time tests.
        // Assert: exactly one timed_out acp_write, id=1, outcome=selected, optionId=opt-reject.
        // (permission_denial_response selects the reject_once option from default_opts.)
        let events = obs.snapshot();
        let timed_out_writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("timed_out"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            timed_out_writes.len(),
            1,
            "exactly one timed_out acp_write must be observed; got: {timed_out_writes:?}"
        );
        let payload = &timed_out_writes[0].payload;
        assert_eq!(
            payload["id"],
            serde_json::json!(1),
            "denial payload id must be 1; got {payload}"
        );
        assert_eq!(
            payload["result"]["outcome"]["outcome"].as_str(),
            Some("selected"),
            "denial payload must carry outcome=selected; got {payload}"
        );
        assert_eq!(
            payload["result"]["outcome"]["optionId"].as_str(),
            Some("opt-reject"),
            "denial optionId must be opt-reject (reject_once from default_opts); got {payload}"
        );
    }

    /// Real-time test — Part 3: idle is re-armed after the last pending entry resolves.
    ///
    /// A single continuously running loop:
    /// 1. Processes a permission request (idle suspended while pending).
    /// 2. Receives a decision (applied) — entry removed, idle re-armed.
    /// 3. After one full idle interval of silence, the loop exits with IdleTimeout.
    ///
    /// This proves that a slow human decision grants the agent a fresh idle window,
    /// not an insta-cancel. Uses real time with short (100ms) idle window.
    #[tokio::test]
    async fn ask_permission_idle_rearmed_after_last_entry_resolves() {
        // Script: emit a permission request, read one line (the response), then sleep forever.
        // After the permission is answered, the agent stays silent — idle must fire.
        let perm_req = r#"{"jsonrpc":"2.0","id":1,"method":"session/request_permission","params":{"sessionId":"sess","requestId":"req-rearm","subject":"test","options":[{"optionId":"opt-allow","kind":"allow_once","name":"Allow"},{"optionId":"opt-deny","kind":"reject_once","name":"Deny"}]}}"#;
        let script = format!(
            r#"printf '{perm_req}\n'; read -r _resp; sleep 600"#,
            perm_req = perm_req
        );

        let mut client = spawn_script(&script).await;
        let config = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        client.set_permission_config(config);
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client);
        let obs = crate::observer::ObserverHandle::in_process();
        let mut obs_rx = obs.subscribe();
        client.set_observer(Some(obs.clone()), 0);
        let (perm_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Use real-time with short (100ms) idle window so the test completes fast.
        // Hard deadline is generous (10s) — only idle fires in this scenario.
        let idle = std::time::Duration::from_millis(100);
        let max_dur = std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + max_dur;

        // Run the full loop in a spawned task (continuously, no restarts).
        let loop_task = tokio::spawn(async move {
            client
                .read_until_response_with_idle_timeout(
                    "sess-rearm",
                    999,
                    idle,
                    hard_deadline,
                    max_dur,
                )
                .await
        });

        // Wait for the actionable acp_read from the observer (real-time wait, 5s budget).
        let mut found_nonce: Option<String> = None;
        let wait_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while tokio::time::Instant::now() < wait_deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(200), obs_rx.recv()).await {
                Ok(Ok(event)) => {
                    if event.kind == "acp_read" {
                        if let Some(auth) = &event.authorization {
                            if auth.actionable {
                                found_nonce = Some(auth.request_nonce.clone());
                                break;
                            }
                        }
                    }
                }
                // Timeout or channel closed — give up.
                _ => break,
            }
        }
        let nonce = found_nonce.expect("actionable acp_read must be emitted within 5s");

        // Send the decision — causes finish_permission to write the response and
        // re-arm the idle deadline to now + 100ms.
        perm_tx
            .send(PermissionDecision {
                request_nonce: nonce,
                option_id: "opt-allow".to_string(),
            })
            .await
            .expect("decision channel must accept");

        // The loop now has a fresh 100ms idle window. It must exit via IdleTimeout
        // (agent stays silent after the response). Wait up to 5s (generous real-time
        // budget), then assert the loop exited with IdleTimeout — not PermissionPoisoned
        // or any other error — proving idle was re-armed after the decision was applied.
        let result = loop_task.await.expect("loop task must not panic");

        assert!(
            matches!(result, Err(AcpError::IdleTimeout(_))),
            "after permission resolved, idle must fire and exit the loop; got: {result:?}"
        );

        // Confirm the applied decision emitted an authorized acp_write in the observer.
        let events = obs.snapshot();
        let applied_writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("applied"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            applied_writes.len(),
            1,
            "exactly one applied acp_write must be emitted after decision; got: {applied_writes:?}"
        );
    }

    /// Capacity recovery: 9 sequential requests all succeed when each prior
    /// request is decided before the next is queued. Entries are removed on
    /// terminal transition so the 9th slot is available.
    ///
    /// Proves behavior at the wire level: a capture script collects all stdin
    /// NDJSON lines so we can assert 9 distinct permission responses were written.
    #[tokio::test]
    async fn ask_nine_sequential_requests_all_succeed_after_capacity_recovery() {
        // Script: read all stdin lines into a capture file, then stay alive.
        // This captures every wire write the harness makes to the agent.
        let capture_file =
            std::env::temp_dir().join(format!("buzz-acp-cap9-{}.ndjson", uuid::Uuid::new_v4()));
        let script = format!(
            r#"while IFS= read -r line; do printf '%s\n' "$line" >> {capture}; done; sleep 2"#,
            capture = capture_file.display(),
        );
        let mut client = spawn_script(&script).await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);

        // Register each request and immediately deliver a decision, one at a time.
        // After each decision is applied, the entry is removed from the map,
        // freeing a slot for the next request. This proves capacity recovery.
        //
        // A fresh permission decision channel is installed for each iteration so
        // the receiver is live when the loop runs. `read_until_response_with_idle_timeout`
        // takes the rx for its duration; creating a new one per iteration avoids
        // the "rx dropped between calls" problem that would occur with a single receiver.
        let mut response_nonces: Vec<String> = Vec::new();
        for i in 0..9u64 {
            // Fresh channel per iteration — the rx is live for exactly one loop call.
            let (iter_tx, iter_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(4);
            client.install_permission_decision_rx(iter_rx);

            let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
            let msg = perm_request(i + 100, default_opts());
            let result = client.handle_permission_request(&msg, hard).await;
            assert!(
                result.as_ref().is_ok_and(|v| *v),
                "request {i} must register successfully (capacity not exhausted), got: {result:?}"
            );

            // Capture the nonce and deliver a decision immediately.
            let id_str = (i + 100).to_string();
            let nonce = client
                .pending_permissions
                .get(&id_str)
                .expect("entry must be Pending after registration")
                .nonce
                .clone();
            response_nonces.push(nonce.clone());
            iter_tx
                .send(PermissionDecision {
                    request_nonce: nonce,
                    option_id: "opt-allow".to_string(),
                })
                .await
                .ok();

            // Drive the loop briefly to process the queued decision.
            let hard_loop = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(300),
                client.read_until_response_with_idle_timeout(
                    "sess-cap9",
                    9999,
                    std::time::Duration::from_millis(150),
                    hard_loop,
                    std::time::Duration::from_secs(5),
                ),
            )
            .await;

            // After the decision is applied the entry must be removed (no tombstone).
            assert!(
                !client.pending_permissions.contains_key(&id_str),
                "entry {i} must be removed after decision applied"
            );
        }

        // All 9 requests succeeded. Map must be empty.
        assert!(
            client.pending_permissions.is_empty(),
            "map must be empty after 9 sequential requests all resolved"
        );

        // Give the script a moment to flush all lines.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Wire-level assertion: 9 distinct permission responses were written on the pipe.
        let wire_lines = tokio::task::spawn_blocking({
            let capture_file = capture_file.clone();
            move || {
                for _ in 0..30 {
                    if let Ok(s) = std::fs::read_to_string(&capture_file) {
                        let lines: Vec<String> = s
                            .lines()
                            .filter(|l| !l.is_empty())
                            .map(|l| l.to_string())
                            .collect();
                        if lines.len() >= 9 {
                            return lines;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                // Return whatever we have.
                std::fs::read_to_string(&capture_file)
                    .unwrap_or_default()
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| l.to_string())
                    .collect()
            }
        })
        .await
        .expect("spawn_blocking failed");
        let _ = std::fs::remove_file(&capture_file);

        // Filter to permission responses: {"id": <num>, "result": {"outcome": {...}}}
        let perm_responses: Vec<serde_json::Value> = wire_lines
            .iter()
            .filter_map(|l| serde_json::from_str(l).ok())
            .filter(|v: &serde_json::Value| {
                v.get("result").and_then(|r| r.get("outcome")).is_some()
            })
            .collect();

        // The 9 distinct IDs (100..108) each got one wire response.
        let written_ids: std::collections::HashSet<u64> = perm_responses
            .iter()
            .filter_map(|v| v["id"].as_u64())
            .collect();
        assert_eq!(
            written_ids.len(),
            9,
            "must have 9 distinct permission wire responses (one per request id), \
             got ids: {written_ids:?}, total responses: {perm_responses:?}"
        );
        // Verify ids span 100..108 inclusive.
        for expected_id in 100..109u64 {
            assert!(
                written_ids.contains(&expected_id),
                "missing wire response for id={expected_id}"
            );
        }

        // Observer-level: 9 distinct authorized acp_write nonces.
        let events = obs.snapshot();
        let write_nonces: std::collections::HashSet<String> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("applied"))
                        .unwrap_or(false)
            })
            .filter_map(|e| e.authorization.as_ref().map(|a| a.request_nonce.clone()))
            .collect();
        assert_eq!(
            write_nonces.len(),
            9,
            "must have 9 distinct authorized acp_write events (one per request), got: {write_nonces:?}"
        );
    }

    // ── Pinned §1 (simpler): ask entry registered synchronously ──────────────

    #[tokio::test]
    async fn ask_registers_entry_in_pending_map() {
        // Verify that handle_permission_request under ask policy inserts
        // a Pending entry into the map (without needing a live decision loop).
        let mut client = spawn_inert_client().await;
        let config = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        client.set_permission_config(config);
        client.set_owner_pubkey_known(true);
        // Install an observer so the ask arm doesn't downgrade.
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs), 0);
        // Install a permission decision channel (must be installed or take() panics).
        let (_perm_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);
        // Install relay context so D7 passes and the entry is inserted.
        install_test_relay_context(&mut client);

        let msg = perm_request(42, default_opts());
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard_deadline).await;
        assert!(
            result.is_ok(),
            "ask must return Ok to suppress generic emit"
        );
        assert!(
            result.unwrap(),
            "ask must return Ok(true) to suppress generic emit"
        );
        assert_eq!(
            client.pending_permissions.len(),
            1,
            "exactly one entry must be registered after ask"
        );
        let entry = client
            .pending_permissions
            .get("42")
            .expect("entry under id=42");
        assert!(
            matches!(
                entry.state,
                PermissionEntryState::Publishing | PermissionEntryState::Pending
            ),
            "entry must start in Publishing or Pending state (relay ACK may arrive before assertion)"
        );
    }

    // ── Pinned §1 (cancel during write path): poison process test ────────────

    #[test]
    fn cancel_during_writing_poisons_process() {
        // Simulate a process that has an entry in Writing state at cancel time.
        // cancel_with_cleanup_until must return PermissionPoisoned and set the flag.
        //
        // We test this synchronously because cancel_with_cleanup_until is async
        // and we need to manipulate state directly. We use a tokio runtime.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Use a "sleep" script so the process is alive but won't emit responses.
            let mut client = spawn_script("sleep 10").await;
            client.set_permission_config(
                ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
            );
            client.set_owner_pubkey_known(true);

            // Manually plant an entry in Writing state — this simulates cancel
            // arriving while the harness was in the middle of writing.
            client.pending_permissions.insert(
                "99".to_string(),
                PermissionEntry {
                    nonce: "n99".to_string(),
                    options_snapshot: vec![],
                    card_actions: test_card_actions(),
                    state: PermissionEntryState::Writing,
                    deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(300),
                    expiry_unix_secs: 0,
                    sentinel_event_id: None,
                    early_decision: None,
                    description: None,
                },
            );
            // cancel_with_cleanup needs last_prompt_id to be Some.
            client.last_prompt_id = Some(999);

            let err = client
                .cancel_with_cleanup_grace("sess-poison", std::time::Duration::from_millis(500))
                .await
                .expect_err("cancel during write must return Err");

            assert!(
                matches!(err, AcpError::PermissionPoisoned),
                "expected PermissionPoisoned, got {err:?}"
            );
            assert!(
                client.permission_poisoned,
                "poisoned flag must be set after cancel-during-write"
            );
        });
    }

    #[test]
    fn poisoned_process_surfaces_immediately_on_next_cancel() {
        // Once poisoned, every subsequent cancel must immediately return PermissionPoisoned
        // without writing anything — the process is unsafe to use.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut client = spawn_script("sleep 10").await;
            client.permission_poisoned = true;
            client.last_prompt_id = Some(1);

            let err = client
                .cancel_with_cleanup_grace("sess", std::time::Duration::from_millis(200))
                .await
                .expect_err("poisoned process must error immediately");
            assert!(matches!(err, AcpError::PermissionPoisoned));
        });
    }

    /// Two-entry cancel: first write fails → stop immediately, no second write.
    ///
    /// Registers two Pending entries, then cancels against a process whose stdin
    /// pipe is already closed (script exits immediately). The first
    /// `finish_permission()` call returns `false` (write failed, process poisoned),
    /// and the cancel loop must return `Err(PermissionPoisoned)` immediately — zero
    /// bytes are written for the second entry.
    ///
    /// Uses an instrumented write-attempt counter to assert exactly ONE attempt was
    /// made (the first, which failed), not just that no successful writes occurred.
    /// The counter distinguishes "stopped after first attempt" from "tried all and
    /// all failed" — the latter would allow the loop to continue past the poison.
    #[tokio::test]
    async fn cancel_first_write_fails_stops_immediately_no_second_write() {
        // Script: exit immediately without reading stdin.
        // After exit, the read-end of stdin is closed; writes fail with BrokenPipe.
        let mut client = spawn_script("exit 0").await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);
        install_test_relay_context(&mut client);

        // Install the write-attempt counter BEFORE registration so all writes
        // (including the registration acks and the cancel responses) are counted.
        let attempt_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        client.set_write_attempt_count(attempt_counter.clone());

        // Register two Pending entries.
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        for i in 0..2u64 {
            let msg = perm_request(i, default_opts());
            client
                .handle_permission_request(&msg, hard)
                .await
                .expect("ask registration must succeed");
            // In production the relay ACK transitions the entry Publishing →
            // Pending in the read loop before the next request arrives, so two
            // Pending entries legitimately coexist. This test does not drive the
            // loop between registrations, so apply that transition explicitly —
            // otherwise the publish-in-flight guard denies the second request.
            if let Some(entry) = client.pending_permissions.get_mut(&i.to_string()) {
                entry.state = PermissionEntryState::Pending;
            }
        }
        assert_eq!(
            client.pending_permissions.len(),
            2,
            "two entries must be registered"
        );
        client.last_prompt_id = Some(999);

        // Wait briefly for the script to exit and close its stdin read-end.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Snapshot the attempt count before cancel so we can count only cancel writes.
        let attempts_before_cancel = attempt_counter.load(std::sync::atomic::Ordering::Relaxed);

        // Cancel: the first finish_permission() write must fail (BrokenPipe),
        // poison the process, and return Err(PermissionPoisoned) immediately.
        let err = client
            .cancel_with_cleanup_grace("sess-fail2", std::time::Duration::from_millis(500))
            .await
            .expect_err("cancel on closed-stdin process must return Err");
        assert!(
            matches!(err, AcpError::PermissionPoisoned),
            "expected PermissionPoisoned, got {err:?}"
        );
        assert!(
            client.permission_poisoned,
            "poisoned flag must be set after cancel write failure"
        );

        // Exactly ONE write attempt during the cancel phase.
        // If the loop stopped after the first failed attempt, count = 1.
        // If it continued and tried the second entry, count = 2.
        let attempts_during_cancel =
            attempt_counter.load(std::sync::atomic::Ordering::Relaxed) - attempts_before_cancel;
        assert_eq!(
            attempts_during_cancel, 1,
            "cancel must attempt exactly one write (for the first entry) then stop; \
             attempted {attempts_during_cancel} times"
        );

        // No successful cancel writes.
        let events = obs.snapshot();
        let cancel_writes = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("cancelled"))
                        .unwrap_or(false)
            })
            .count();
        assert_eq!(
            cancel_writes, 0,
            "no successful cancel writes must be emitted when first write fails; got {cancel_writes}"
        );

        // At least one `permission_terminal` uncertain event must be emitted.
        let uncertain_events = events
            .iter()
            .filter(|e| {
                e.kind == "permission_terminal"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("uncertain"))
                        .unwrap_or(false)
            })
            .count();
        assert!(
            uncertain_events >= 1,
            "at least one permission_terminal(uncertain) must be emitted on write failure; got {uncertain_events}"
        );
    }

    #[test]
    fn poisoned_process_check_in_read_loop_returns_poison_error() {
        // Once permission_poisoned is set, read_until_response_with_idle_timeout
        // must return PermissionPoisoned on the next loop iteration.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut client = spawn_script("sleep 10").await;
            client.permission_poisoned = true;
            client.last_prompt_id = Some(42);

            let idle = std::time::Duration::from_secs(5);
            let max_dur = std::time::Duration::from_secs(10);
            let hard_deadline = tokio::time::Instant::now() + max_dur;
            let result = client
                .read_until_response_with_idle_timeout("sess", 42, idle, hard_deadline, max_dur)
                .await;
            assert!(
                matches!(result, Err(AcpError::PermissionPoisoned)),
                "expected PermissionPoisoned from poisoned-flag check, got {result:?}"
            );
        });
    }

    // ── Pinned §5: cancel drains pending entries with cancelled ───────────────

    #[test]
    fn cancel_drains_pending_entries_with_cancelled_response() {
        // Under ask policy: cancel must drain all Pending entries and write
        // "cancelled" responses for each, then proceed to session/cancel.
        // Verifies:
        //   - Map is empty after cancel (entries were drained).
        //   - Cancel result is NOT PermissionPoisoned (no Writing entries present).
        //   - Cancel exits normally (Ok or CancelDrainTimeout — sleep script never
        //     emits a response, so this exits via timeout, which is expected).
        //
        // We can verify that Pending entries are removed by checking the map post-cancel.
        // We don't verify the wire bytes here (that requires a live script) — we verify
        // the state machine: Pending entries disappear after cancel.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            // Use a "sleep" script — stays alive but ignores stdin.
            let mut client = spawn_script("sleep 5").await;
            client.set_permission_config(
                ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
            );

            // Plant two Pending entries.
            for i in 0..2u64 {
                client.pending_permissions.insert(
                    format!("{i}"),
                    PermissionEntry {
                        nonce: format!("n{i}"),
                        options_snapshot: vec![
                            serde_json::json!({"optionId":"opt","kind":"reject_once","name":"R"}),
                        ],
                        card_actions: test_card_actions(),
                        state: PermissionEntryState::Pending,
                        deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(300),
                        expiry_unix_secs: 0,
                        sentinel_event_id: None,
                        early_decision: None,
                        description: None,
                    },
                );
            }
            client.last_prompt_id = Some(999);

            // cancel_with_cleanup_grace with short grace — the sleep script will
            // never emit a response, so this exits via CancelDrainTimeout.
            let result = client
                .cancel_with_cleanup_grace("sess-drain", std::time::Duration::from_millis(200))
                .await;

            // Should NOT be PermissionPoisoned (no Writing entries).
            assert!(
                !matches!(result, Err(AcpError::PermissionPoisoned)),
                "no Writing entries — must not be PermissionPoisoned"
            );
            // Map must be empty — Pending entries were drained.
            assert!(
                client.pending_permissions.is_empty(),
                "all Pending entries must be removed from the map after cancel"
            );
        });
    }

    // ── D7-final admission: named tests (owner / non-owner / no-publisher / unresolved) ─

    /// D7: owner-initiated turn + matching owner hex → entry inserted as Publishing.
    #[tokio::test]
    async fn d7_owner_initiated_turn_inserts_publishing_entry() {
        let mut client = spawn_inert_client().await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);
        // Owner-initiated: initiator == owner.
        install_test_relay_context(&mut client);

        let msg = perm_request(1, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard).await;
        assert!(result.is_ok_and(|v| v), "owner-initiated ask must succeed");
        assert_eq!(
            client.pending_permissions.len(),
            1,
            "entry must be inserted for owner-initiated turn"
        );
    }

    /// D7: non-owner-initiated turn → request denied synchronously, no entry inserted.
    #[tokio::test]
    async fn d7_non_owner_initiated_turn_denied_no_entry() {
        let mut client = spawn_inert_client().await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Install relay context but set a DIFFERENT initiator (non-owner).
        let owner_keys = install_test_relay_context(&mut client);
        let non_owner_keys = Keys::generate();
        assert_ne!(
            owner_keys.public_key(),
            non_owner_keys.public_key(),
            "keys must be different"
        );
        // Override the initiator with a different pubkey.
        client.set_turn_initiator_pubkey(Some(non_owner_keys.public_key()));

        let msg = perm_request(1, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard).await;
        assert!(
            result.is_ok(),
            "non-owner ask must return Ok (not propagate error)"
        );
        assert!(
            client.pending_permissions.is_empty(),
            "non-owner ask must not insert a pending entry"
        );
    }

    /// D7: no relay publisher → request denied synchronously, no entry inserted.
    #[tokio::test]
    async fn d7_no_relay_publisher_denied_no_entry() {
        let mut client = spawn_inert_client().await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);
        // Intentionally set owner/initiator WITHOUT installing a relay publisher.
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        // No relay publisher → D7 denies.

        let msg = perm_request(1, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard).await;
        assert!(
            result.is_ok(),
            "no-publisher ask must return Ok (not propagate error)"
        );
        assert!(
            client.pending_permissions.is_empty(),
            "no-publisher ask must not insert a pending entry"
        );
    }

    /// D7: unresolved owner (relay present but owner hex absent) → denied, no entry.
    #[tokio::test]
    async fn d7_unresolved_owner_denied_no_entry() {
        let mut client = spawn_inert_client().await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Install publisher and initiator but NO owner hex.
        let keys = Keys::generate();
        let (publisher, event_rx) = crate::relay::RelayEventPublisher::test_pair();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while rx.recv().await.is_some() {}
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap()),
            None,
        );
        // owner_hex deliberately NOT set → D7 denies.

        let msg = perm_request(1, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard).await;
        assert!(result.is_ok(), "unresolved-owner ask must return Ok");
        assert!(
            client.pending_permissions.is_empty(),
            "unresolved-owner ask must not insert a pending entry"
        );
    }

    // ── ACK lifecycle tests (frozen named list) ───────────────────────────────

    /// Positive OK: relay accepts → entry transitions Publishing → Pending,
    /// then a decision drives it to Writing/terminal. Map empty after resolution.
    #[tokio::test]
    async fn sentinel_ack_accepted_transitions_to_pending_and_decision_applies() {
        // Script: read one line (the permission response), then exit.
        let capture_file =
            std::env::temp_dir().join(format!("buzz-acp-ack-ok-{}.json", uuid::Uuid::new_v4()));
        let script = format!(
            r#"read -r resp; printf '%s' "$resp" > {capture}"#,
            capture = capture_file.display(),
        );
        let mut client = spawn_script(&script).await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client); // auto-accepts
        let obs = crate::observer::ObserverHandle::in_process();
        let mut obs_rx = obs.subscribe();
        client.set_observer(Some(obs.clone()), 0);
        let (perm_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Background task: wait for actionable acp_read, then deliver decision.
        let decision_task = tokio::spawn(async move {
            let mut found_nonce: Option<String> = None;
            while let Ok(Ok(event)) =
                tokio::time::timeout(std::time::Duration::from_secs(5), obs_rx.recv()).await
            {
                if event.kind == "acp_read" {
                    if let Some(auth) = &event.authorization {
                        if auth.actionable {
                            found_nonce = Some(auth.request_nonce.clone());
                            break;
                        }
                    }
                }
            }
            let nonce = found_nonce.expect("actionable acp_read must be emitted after ACK");
            perm_tx
                .send(PermissionDecision {
                    request_nonce: nonce,
                    option_id: "opt-allow".to_string(),
                })
                .await
                .expect("decision send must succeed");
        });

        let _ = decision_task.await;

        // Run the loop briefly — it should process the ACK (Accepted), transition to Pending,
        // then apply the decision via the observer-based task above.
        // We use a short-lived inert script since we only care about the permission write.
        let idle = std::time::Duration::from_secs(5);
        let max_dur = std::time::Duration::from_secs(10);
        let hard = tokio::time::Instant::now() + max_dur;

        // Drive the loop; it will exit via IdleTimeout after the decision is applied.
        let result = tokio::time::timeout(
            max_dur,
            client.read_until_response_with_idle_timeout("sess-ack-ok", 999, idle, hard, max_dur),
        )
        .await;

        // Map must be empty after the decision is applied.
        assert!(
            client.pending_permissions.is_empty(),
            "map must be empty after ACK+decision cycle; result: {result:?}"
        );
        let _ = std::fs::remove_file(&capture_file);
    }

    /// Rejected OK: relay rejects sentinel → entry denied immediately, map empty, no card shown.
    #[tokio::test]
    async fn sentinel_ack_rejected_denies_immediately_map_empty() {
        let mut client = spawn_script("sleep 600").await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        // Install a rejecting publisher.
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        let (publisher, event_rx) = crate::relay::RelayEventPublisher::test_pair_rejecting();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while rx.recv().await.is_some() {}
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap()),
            None,
        );
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        let msg = perm_request(1, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("registration must succeed");
        assert_eq!(
            client.pending_permissions.len(),
            1,
            "entry must be inserted as Publishing before ACK"
        );

        // Drive the loop: relay task runs, sends Rejected, ACK arm fires, entry denied.
        // Use a real-time timeout — the rejecting publisher fires immediately.
        let max_dur = std::time::Duration::from_secs(5);
        let hard2 = tokio::time::Instant::now() + max_dur;
        let loop_result = tokio::time::timeout(
            max_dur,
            client.read_until_response_with_idle_timeout(
                "sess-ack-reject",
                999,
                std::time::Duration::from_secs(5),
                hard2,
                max_dur,
            ),
        )
        .await;

        assert!(
            client.pending_permissions.is_empty(),
            "map must be empty after relay rejection; loop_result={loop_result:?}"
        );

        // A timed_out write must have been emitted by the reject path.
        let events = obs.snapshot();
        let timeout_or_denied_writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| {
                            a.reason.as_deref() == Some("timed_out")
                                || a.reason.as_deref() == Some("rejected")
                        })
                        .unwrap_or(false)
            })
            .collect();
        assert!(
            !timeout_or_denied_writes.is_empty(),
            "a denial write must be emitted after relay rejection; events: {events:?}"
        );
    }

    /// Timeout with map empty: relay never ACKs within SENTINEL_PUBLISH_TIMEOUT_SECS →
    /// entry denied, map provably empty before the 300s turn deadline.
    #[tokio::test(start_paused = true)]
    async fn sentinel_ack_timeout_denies_and_map_empty() {
        let mut client = spawn_script("sleep 600").await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        // Install a silent publisher — never sends an ACK.
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        let (publisher, event_rx) = crate::relay::RelayEventPublisher::test_pair_silent();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while rx.recv().await.is_some() {}
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap()),
            None,
        );
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        let msg = perm_request(1, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("registration must succeed");
        assert_eq!(
            client.pending_permissions.len(),
            1,
            "entry must be inserted as Publishing"
        );

        // Advance past SENTINEL_PUBLISH_TIMEOUT_SECS (10s) so the background task's
        // timeout fires and sends Uncertain to ack_result_rx.
        tokio::time::advance(std::time::Duration::from_secs(
            SENTINEL_PUBLISH_TIMEOUT_SECS + 1,
        ))
        .await;

        // Drive the loop to process the timeout outcome.
        let hard2 = tokio::time::Instant::now() + std::time::Duration::from_secs(290);
        let _ = tokio::select! {
            r = client.read_until_response_with_idle_timeout(
                "sess-ack-timeout", 999,
                std::time::Duration::from_secs(5),
                hard2,
                std::time::Duration::from_secs(290),
            ) => r,
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => Err(AcpError::IdleTimeout(std::time::Duration::from_millis(100))),
        };

        assert!(
            client.pending_permissions.is_empty(),
            "map must be empty after publish timeout"
        );

        // A denial write must have been emitted.
        let events = obs.snapshot();
        let denial_writes: Vec<_> = events
            .iter()
            .filter(|e| e.kind == "acp_write" && e.authorization.is_some())
            .collect();
        assert!(
            !denial_writes.is_empty(),
            "a denial write must be emitted after publish timeout; events: {events:?}"
        );
    }

    /// Socket failure (channel closed): relay command channel closes → `register_publish_ack`
    /// returns Err → entry denied synchronously, map empty immediately.
    #[tokio::test]
    async fn sentinel_ack_socket_failure_denies_synchronously_map_empty() {
        let mut client = spawn_inert_client().await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Build a publisher whose cmd_tx is immediately dropped so any send returns Err.
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        // Create a publisher with a dead (closed) command channel.
        let publisher = crate::relay::RelayEventPublisher::test_pair_dead();
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap()),
            None,
        );

        let msg = perm_request(1, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        // register_publish_ack will fail → deny path runs synchronously.
        let result = client.handle_permission_request(&msg, hard).await;
        assert!(
            result.is_ok(),
            "socket-failure ask must return Ok (deny path)"
        );
        assert!(
            client.pending_permissions.is_empty(),
            "map must be empty after socket failure — entry was removed before returning"
        );
    }

    /// Early decision buffered then applied: a decision arrives while the entry is
    /// still in Publishing state; it is buffered and applied immediately on ACK.
    #[tokio::test]
    async fn sentinel_ack_early_decision_buffered_then_applied_on_accepted() {
        // Script: read one permission response line (from the early-decision path), exit.
        let capture_file =
            std::env::temp_dir().join(format!("buzz-acp-early-{}.json", uuid::Uuid::new_v4()));
        let script = format!(
            r#"read -r resp; printf '%s' "$resp" > {capture}; sleep 2"#,
            capture = capture_file.display(),
        );
        let mut client = spawn_script(&script).await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        // Use the auto-accepting test_pair.
        install_test_relay_context(&mut client);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (perm_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        let msg = perm_request(77, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("registration must succeed");

        // Read the nonce from the Publishing entry (before ACK arrives).
        let nonce = client
            .pending_permissions
            .get("77")
            .expect("entry must be in map")
            .nonce
            .clone();

        // Send a decision NOW — the entry is still in Publishing state.
        // This decision should be buffered in early_decision and applied on ACK.
        perm_tx
            .send(PermissionDecision {
                request_nonce: nonce,
                option_id: "opt-allow".to_string(),
            })
            .await
            .expect("decision send must succeed");

        // Drive the loop — ACK fires (Accepted), buffered decision applied, map empties.
        let idle = std::time::Duration::from_millis(200);
        let max_dur = std::time::Duration::from_secs(5);
        let hard2 = tokio::time::Instant::now() + max_dur;
        let _ = tokio::time::timeout(
            max_dur,
            client.read_until_response_with_idle_timeout(
                "sess-early-decision",
                999,
                idle,
                hard2,
                max_dur,
            ),
        )
        .await;

        assert!(
            client.pending_permissions.is_empty(),
            "map must be empty after early-decision + ACK cycle"
        );

        // Observer must show an applied write.
        let events = obs.snapshot();
        let applied_writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("applied"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            applied_writes.len(),
            1,
            "exactly one applied write after early decision + ACK; got: {applied_writes:?}"
        );
        let _ = std::fs::remove_file(&capture_file);
    }

    /// Deadline-during-publish: an entry whose publish deadline has passed while
    /// still in `Publishing` state is denied and never transitions to `Pending`.
    ///
    /// Uses `test_pair_silent` (drops ack_tx immediately) to simulate a relay
    /// that never sends OK. With `start_paused = true` we advance time past
    /// `SENTINEL_PUBLISH_TIMEOUT_SECS` so the relay background task's deadline
    /// arm fires, sweeping the waiter as `Uncertain`, which the ACP loop processes
    /// as a denial — the entry must not enter `Pending` and the map must be empty.
    #[tokio::test(start_paused = true)]
    async fn sentinel_ack_deadline_during_publishing_never_admitted() {
        let mut client = spawn_script("sleep 600").await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        let (publisher, event_rx) = crate::relay::RelayEventPublisher::test_pair_silent();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while rx.recv().await.is_some() {}
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000006").unwrap()),
            None,
        );
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        let msg = perm_request(99, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("registration must succeed");

        // Entry is in Publishing state. Advance past the publish deadline.
        tokio::time::advance(std::time::Duration::from_secs(
            SENTINEL_PUBLISH_TIMEOUT_SECS + 1,
        ))
        .await;

        // Drive the loop — ack_result_rx receives Uncertain (from the dropped
        // sender), the ACK arm fires, the entry is denied, and the map empties.
        let hard2 = tokio::time::Instant::now() + std::time::Duration::from_secs(290);
        let _ = tokio::select! {
            r = client.read_until_response_with_idle_timeout(
                "sess-deadline-during-publishing", 999,
                std::time::Duration::from_secs(5),
                hard2,
                std::time::Duration::from_secs(290),
            ) => r,
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                Err(AcpError::IdleTimeout(std::time::Duration::from_millis(100)))
            }
        };

        assert!(
            client.pending_permissions.is_empty(),
            "map must be empty — deadline-during-publish must deny, never admit to Pending"
        );

        // A denial write must have been emitted (publish timeout → fail closed).
        // No Pending transition occurred — the entry went Publishing → denied.
        let events = obs.snapshot();
        let denial_writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| {
                            a.reason.as_deref() == Some("timed_out")
                                || a.reason.as_deref() == Some("rejected")
                        })
                        .unwrap_or(false)
            })
            .collect();
        assert!(
            !denial_writes.is_empty(),
            "a denial write must be emitted after deadline fires during Publishing; events: {events:?}"
        );
    }

    /// Resolved-edit delivery survives a relay disconnect at decision time.
    ///
    /// The permission decision is irreversible once `finish_permission` writes
    /// the ACP response and removes the entry, so the resolved kind-40003 edit
    /// that retires the UI card MUST reach the relay even if the socket is down
    /// at that instant. This drives the full production lifecycle (Publishing →
    /// Pending → Writing via an early-buffered decision → `finish_permission`),
    /// with a publisher that reports the FIRST resolved-edit publish as
    /// `Uncertain` (disconnected) and every later one as `Accepted`
    /// (reconnected). The fix retransmits the *same signed event* on Uncertain,
    /// so the card is repaired on reconnect.
    ///
    /// Acceptance bar (mutation proof): reverting the production path to a
    /// fire-and-forget `publisher.publish_event(event)` publishes the resolved
    /// edit exactly once with no ACK awaited, so only ONE kind-40003 event is
    /// ever emitted and this test goes red on the retransmission assertion.
    #[tokio::test]
    async fn resolved_edit_retransmitted_until_accepted_across_disconnect() {
        // Script reads the one permission response line (early-decision path), then idles.
        let capture_file = std::env::temp_dir().join(format!(
            "buzz-acp-resolved-retx-{}.json",
            uuid::Uuid::new_v4()
        ));
        let script = format!(
            r#"read -r resp; printf '%s' "$resp" > {capture}; sleep 5"#,
            capture = capture_file.display(),
        );
        let mut client = spawn_script(&script).await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);

        // Publisher: kind-9 sentinel Accepted (lifecycle proceeds); the first
        // resolved kind-40003 publish is Uncertain (socket down), then Accepted.
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        let (publisher, event_rx) =
            crate::relay::RelayEventPublisher::test_pair_resolved_reconnect(1);
        // Collect every published event (including each retransmission attempt).
        let published: std::sync::Arc<std::sync::Mutex<Vec<(u16, String)>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let published_drain = published.clone();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(ev) = rx.recv().await {
                published_drain
                    .lock()
                    .unwrap()
                    .push((ev.kind.as_u16(), ev.id.to_hex()));
            }
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000008").unwrap()),
            None,
        );
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (perm_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        let msg = perm_request(88, default_opts());
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("registration must succeed");

        // Buffer an allow decision while still Publishing; applied on ACK.
        let nonce = client
            .pending_permissions
            .get("88")
            .expect("entry must be in map")
            .nonce
            .clone();
        perm_tx
            .send(PermissionDecision {
                request_nonce: nonce,
                option_id: "opt-allow".to_string(),
            })
            .await
            .expect("decision send must succeed");

        // Drive the loop: ACK Accepted → Pending → buffered decision applied →
        // finish_permission writes the ACP response and spawns the resolved-edit
        // retransmit task. Loop exits on the short idle timeout.
        let idle = std::time::Duration::from_millis(200);
        let max_dur = std::time::Duration::from_secs(5);
        let hard2 = tokio::time::Instant::now() + max_dur;
        let _ = tokio::time::timeout(
            max_dur,
            client.read_until_response_with_idle_timeout(
                "sess-resolved-retx",
                999,
                idle,
                hard2,
                max_dur,
            ),
        )
        .await;

        assert!(
            client.pending_permissions.is_empty(),
            "map must be empty after the decision is applied"
        );

        // Wait for the detached retransmit task: first attempt (Uncertain),
        // RESOLVED_RETRANSMIT_BACKOFF, second attempt (Accepted). Poll until two
        // resolved-edit publishes are observed or a generous bound elapses.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(6);
        loop {
            let resolved_count = published
                .lock()
                .unwrap()
                .iter()
                .filter(|(kind, _)| *kind == 40003)
                .count();
            if resolved_count >= 2 || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        let resolved_ids: Vec<String> = published
            .lock()
            .unwrap()
            .iter()
            .filter(|(kind, _)| *kind == 40003)
            .map(|(_, id)| id.clone())
            .collect();

        // The resolved edit was retransmitted across the disconnect — this is
        // the assertion the fire-and-forget mutation turns red (it publishes
        // the edit exactly once with no ACK, so resolved_ids.len() == 1).
        assert!(
            resolved_ids.len() >= 2,
            "resolved kind-40003 edit must be retransmitted after an Uncertain outcome; \
             saw {} publish(es): {resolved_ids:?}",
            resolved_ids.len()
        );
        // Every retransmission is the SAME signed event (idempotent by id) —
        // the spec requirement that a retry resends the identical event.
        assert!(
            resolved_ids.windows(2).all(|w| w[0] == w[1]),
            "every retransmission must be the same signed event id; saw {resolved_ids:?}"
        );

        let _ = std::fs::remove_file(&capture_file);
    }

    /// Resolved-edit delivery survives a *lost `OK` on a connected socket*.
    ///
    /// Distinct from the disconnect case: here the relay receives the EVENT but
    /// its `OK` never comes back, so the acked waiter is resolved only when its
    /// per-waiter deadline sweeps. With the raw card expiry (≤300s) as that
    /// deadline the single attempt would park the whole window and exit with
    /// zero retransmissions; the fix caps each attempt at
    /// `SENTINEL_PUBLISH_TIMEOUT_SECS` so the stuck waiter sweeps promptly and
    /// the identical signed event is resent and accepted.
    ///
    /// Acceptance bar (mutation proof): reverting the per-attempt deadline back
    /// to the raw `expiry_deadline` (`register_publish_ack(event.clone(),
    /// expiry_deadline)`) makes the first attempt park until card expiry, so
    /// only ONE kind-40003 publish is ever emitted and this test goes red.
    ///
    /// Runs under paused tokio time so the 10s per-attempt deadline and the 2s
    /// backoff advance deterministically without real waiting.
    #[tokio::test(start_paused = true)]
    async fn resolved_edit_retransmitted_after_lost_ok_on_connected_socket() {
        let keys = Keys::generate();
        let (publisher, mut event_rx) =
            crate::relay::RelayEventPublisher::test_pair_resolved_lost_ok(1);

        // Collect every resolved-edit publish (each retransmission attempt).
        let published: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let published_drain = published.clone();
        tokio::spawn(async move {
            while let Some(ev) = event_rx.recv().await {
                if ev.kind.as_u16() == 40003 {
                    published_drain.lock().unwrap().push(ev.id.to_hex());
                }
            }
        });

        // Sign the resolved edit once; the retransmit loop resends this exact event.
        let event = build_kind40003_sentinel(
            &keys,
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000009").unwrap(),
            "target-event-id",
            "resolved-edit-content",
        )
        .expect("sentinel must build");

        // Card expiry generously beyond one per-attempt deadline (10s) so the
        // first attempt sweeps and a second attempt is still within the window.
        let expiry_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);

        let task = tokio::spawn(retransmit_resolved_edit(publisher, event, expiry_deadline));

        // Under paused time the runtime auto-advances the clock while every task
        // is parked on a timer, fast-forwarding the 10s per-attempt deadline sweep
        // and the 2s backoff. Joining the task drives it to the Accepted second
        // attempt; a wall-clock guard keeps a regression from hanging.
        let joined = tokio::time::timeout(std::time::Duration::from_secs(120), task).await;
        assert!(joined.is_ok(), "retransmit task must terminate, not hang");
        // Let the collector task drain the forwarded publishes.
        tokio::task::yield_now().await;

        let resolved_ids = published.lock().unwrap().clone();
        // Retransmitted despite the connected-socket lost OK — the assertion the
        // raw-expiry-deadline mutation turns red (it parks 60s, publishes once).
        assert!(
            resolved_ids.len() >= 2,
            "resolved kind-40003 edit must be retransmitted after a lost OK on a \
             connected socket; saw {} publish(es): {resolved_ids:?}",
            resolved_ids.len()
        );
        assert!(
            resolved_ids.windows(2).all(|w| w[0] == w[1]),
            "every retransmission must be the same signed event id; saw {resolved_ids:?}"
        );
    }

    // ── Item 5: exact kind-9 content string from build_sentinel_pending_payload ─

    /// Emit the exact JSON string that `build_sentinel_pending_payload` produces
    /// for a canonical request with a fixed nonce, session, turn, and expiry.
    /// This string is the cross-boundary fixture the Desktop parser is verified
    /// against (no fence wrapper).
    ///
    /// The card advertises EXACTLY the two ruled actions (allow_once,
    /// reject_once). A third adapter option (`allow_always`) is present in the
    /// request but must never appear in the sentinel — this test is the
    /// mutation proof for that contract (F1): if `select_card_actions` stopped
    /// filtering, `optionIds` would carry the forbidden ID and the length
    /// assertion below would fail.
    #[test]
    fn kind9_content_fixture_structural_invariants() {
        let nonce = "test-nonce-fixture-abc123";
        let options: Vec<serde_json::Value> = vec![
            serde_json::json!({"optionId":"opt-allow","kind":"allow_once","name":"Allow once"}),
            serde_json::json!({"optionId":"opt-reject","kind":"reject_once","name":"Reject"}),
            serde_json::json!({"optionId":"opt-always","kind":"allow_always","name":"Always allow"}),
        ];
        let expiry_unix_secs: u64 = 1_700_000_300; // fixed for reproducibility
        let session_id = Some("sess-fixture-001");
        let turn_id = "turn-fixture-xyz";

        let actions = select_card_actions(&options)
            .expect("select_card_actions must succeed with one allow_once + one reject_once");
        let content = build_sentinel_pending_payload(
            nonce,
            &actions,
            expiry_unix_secs,
            session_id,
            turn_id,
            Some("read a file"),
        )
        .expect("build_sentinel_pending_payload must succeed");

        // Fixture coupling: the producer output MUST be byte-identical to the
        // checked-in fixture the Desktop boundary test parses. A producer-side
        // change to the wire shape breaks THIS assertion, forcing the fixture
        // (and the desktop test that consumes it) to be updated in lockstep.
        const FIXTURE: &str = include_str!("../tests/fixtures/sentinel_pending.json");
        assert_eq!(
            content, FIXTURE,
            "producer output must be byte-equal to the shared cross-language fixture \
             (crates/buzz-acp/tests/fixtures/sentinel_pending.json); if this diff is \
             intentional, regenerate the fixture and the desktop boundary test"
        );

        // Print the canonical fixture string for the Desktop fixture.
        println!("kind-9 content fixture:\n{content}");

        let v: serde_json::Value =
            serde_json::from_str(&content).expect("content must be valid JSON");

        // Structural invariants required by the Desktop parser.
        assert_eq!(v["v"], serde_json::json!(1), "v must be 1");
        assert_eq!(v["state"], "pending", "state must be 'pending'");
        assert_eq!(v["requestNonce"], nonce, "requestNonce must match");
        assert_eq!(
            v["expiresAt"], expiry_unix_secs,
            "expiresAt must be the supplied unix seconds"
        );
        assert_eq!(
            v["sessionId"],
            serde_json::json!("sess-fixture-001"),
            "sessionId must match"
        );
        assert_eq!(v["turnId"], turn_id, "turnId must match");

        // optionIds must contain EXACTLY the two ruled actions, allow first,
        // reject second — never the forbidden allow_always option.
        let option_ids = v["optionIds"]
            .as_array()
            .expect("optionIds must be an array");
        assert_eq!(option_ids.len(), 2, "optionIds must have exactly 2 entries");
        assert_eq!(option_ids[0], "opt-allow");
        assert_eq!(option_ids[1], "opt-reject");
        assert!(
            !option_ids.iter().any(|id| id == "opt-always"),
            "the forbidden allow_always option must never reach the sentinel"
        );

        // labels must be an object with one key per surfaced optionId.
        let labels = v["labels"].as_object().expect("labels must be an object");
        assert_eq!(labels.len(), 2, "labels must have exactly 2 entries");
        assert_eq!(labels["opt-allow"], "Allow once");
        assert_eq!(labels["opt-reject"], "Reject");
        assert!(
            !labels.contains_key("opt-always"),
            "the forbidden allow_always label must never reach the sentinel"
        );

        // The durable-rule disclosure fields are gone: the card can no longer
        // carry an allow_always action, so there is nothing to disclose.
        assert!(
            v.get("hasDurableRule").is_none(),
            "hasDurableRule must not be present — the card is two-action only"
        );
        assert!(
            v.get("durableRuleNote").is_none(),
            "durableRuleNote must not be present — the card is two-action only"
        );

        // originalEventId must NOT be present in a pending payload.
        assert!(
            v.get("originalEventId").is_none() || v["originalEventId"].is_null(),
            "pending payload must not contain a non-null originalEventId"
        );
    }

    // ── Pinned §2: reject policy is byte-for-byte unchanged ───────────────────

    #[tokio::test]
    async fn reject_policy_denies_synchronously_and_returns_ok_true() {
        let mut client = spawn_inert_client().await;
        set_policy(&mut client, PermissionPolicy::Reject);

        let msg = perm_request(7, default_opts());
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard_deadline).await;
        // Reject is synchronous — no pending entry, Ok(true) to suppress generic emit.
        assert!(result.is_ok(), "reject must return Ok");
        assert!(result.unwrap(), "reject must return Ok(true)");
        assert!(
            client.pending_permissions.is_empty(),
            "reject must not leave pending entries"
        );
        // Legacy single-id slot must also be cleared after the synchronous response.
        assert!(
            client.pending_permission_id.is_none(),
            "pending_permission_id must be None after reject completes"
        );
        assert!(
            client.permission_responded,
            "permission_responded must be true after reject completes"
        );
    }

    // ── Pinned §2: allow policy auto-selects allow_once ───────────────────────

    #[tokio::test]
    async fn allow_policy_auto_selects_allow_once_and_returns_ok_true() {
        let mut client = spawn_inert_client().await;
        set_policy(&mut client, PermissionPolicy::Allow);

        let msg = perm_request(8, default_opts());
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard_deadline).await;
        assert!(result.is_ok(), "allow auto-select must return Ok");
        assert!(result.unwrap(), "allow auto-select must return Ok(true)");
        // No pending entries — handled synchronously.
        assert!(client.pending_permissions.is_empty());
    }

    #[tokio::test]
    async fn allow_policy_fails_closed_with_no_allow_once_option() {
        let mut client = spawn_inert_client().await;
        set_policy(&mut client, PermissionPolicy::Allow);

        // Only reject_once offered — allow policy must fail closed.
        let msg = perm_request(9, &[("opt-r", "reject_once", "Reject")]);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        let result = client.handle_permission_request(&msg, hard_deadline).await;
        // Fail closed: denial written, Ok(true) returned.
        assert!(result.is_ok(), "fail-closed allow must return Ok");
        assert!(result.unwrap(), "fail-closed allow must return Ok(true)");
        assert!(client.pending_permissions.is_empty());
    }

    // ── Pinned §6: decision arm — validated option_id must be in snapshot ─────

    #[tokio::test]
    async fn decision_with_unknown_option_id_is_ignored() {
        // A decision carrying an optionId not in the snapshot must be ignored
        // (no response written, entry stays Pending) — the loop continues.
        // After the bad decision is processed, the loop times out on idle (since the
        // script produces no output after the initial response) and the entry is
        // still Pending at that point.
        //
        // The script produces the terminal id=999 response only AFTER a short delay,
        // giving the loop time to process the bad decision and leave the entry Pending.
        // We verify the entry is still Pending by running the loop until idle timeout.
        let script = "sleep 2; echo '{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"done\":true}}'";
        let mut client = spawn_script(script).await;
        client.set_owner_pubkey_known(true);
        set_policy(&mut client, PermissionPolicy::Ask);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs), 0);

        let nonce = "test-nonce-bad-opt".to_string();
        let req_id_str = "5".to_string();
        client.pending_permissions.insert(
            req_id_str.clone(),
            PermissionEntry {
                nonce: nonce.clone(),
                options_snapshot: vec![
                    serde_json::json!({"optionId":"valid-opt","kind":"allow_once","name":"A"}),
                ],
                card_actions: CardActions {
                    allow: serde_json::json!({"optionId":"valid-opt","kind":"allow_once","name":"A"}),
                    reject: serde_json::json!({"optionId":"valid-reject","kind":"reject_once","name":"R"}),
                },
                state: PermissionEntryState::Pending,
                deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(300),
                expiry_unix_secs: 0,
                sentinel_event_id: None,
                early_decision: None,
                description: None,
            },
        );

        // Deliver a decision with a nonce that matches but an invalid optionId.
        let bad_decision = PermissionDecision {
            request_nonce: nonce,
            option_id: "nonexistent-option".to_string(),
        };

        let (tx, rx) = tokio::sync::mpsc::channel::<PermissionDecision>(1);
        client.install_permission_decision_rx(rx);
        // Send the bad decision; then close the sender so the channel is exhausted.
        tx.send(bad_decision).await.unwrap();
        drop(tx);

        // Drive the loop with a short idle timeout — the bad decision is processed
        // on the first iteration (entry stays Pending), then the loop idles.
        let idle = std::time::Duration::from_millis(300);
        let max_dur = std::time::Duration::from_secs(5);
        let hard_deadline = tokio::time::Instant::now() + max_dur;
        let result = client
            .read_until_response_with_idle_timeout("sess-bad-opt", 5, idle, hard_deadline, max_dur)
            .await;

        // The loop exits via idle timeout (script sleeps; bad decision was ignored,
        // so no terminal response for id=5 was written, and idle fires).
        // We accept either idle timeout OR id=999 match (if the script's sleep was short).
        // The critical assertion is on the entry state.
        let _ = result; // exit reason is not the focus

        // Entry must still be Pending — the bad decision did not mutate it.
        let entry = client.pending_permissions.get(&req_id_str);
        // The loop drains on non-recoverable errors; on idle timeout (recoverable) it
        // does NOT drain — entry must still be there and Pending.
        match entry {
            Some(e) => assert!(
                matches!(e.state, PermissionEntryState::Pending),
                "entry must still be Pending after bad decision, got: {:?}",
                e.state
            ),
            None => panic!("entry was removed — idle timeout should not drain the map"),
        }
    }

    // ── Pinned §7 (wire transmission): transmit_mode drives set_config_option ─

    #[test]
    fn resolved_permission_config_effective_mode_wire_string_is_correct() {
        // Verify that effective_mode.as_wire_str() returns the correct ACP wire value.
        let cfg = ResolvedPermissionConfig::resolve(PermissionPolicy::Reject, None).unwrap();
        assert_eq!(cfg.effective_mode.as_wire_str(), "dontAsk");

        let cfg = ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap();
        assert_eq!(cfg.effective_mode.as_wire_str(), "default");

        let cfg = ResolvedPermissionConfig::resolve(PermissionPolicy::Allow, None).unwrap();
        assert_eq!(cfg.effective_mode.as_wire_str(), "default");
    }

    // ── Pinned amendment: PermissionMode::Auto matrix row ────────────────────
    //
    // `auto` = model-gated classifier — the adapter may self-approve most tool
    // calls internally but can still forward residual permission requests to ACP.
    // - allow + auto → compatible (transmit as-is; both want unattended approval)
    // - ask   + auto → compatible with warning (residual escalations surface cards;
    //                  internally-approved calls bypass ask silently)
    // - reject + auto → startup error (inverted security: policy says deny, adapter
    //                   auto-approves everything)

    #[test]
    fn resolved_permission_config_allow_plus_explicit_auto_is_ok() {
        // allow + auto is compatible: both want unattended approval.
        let cfg =
            ResolvedPermissionConfig::resolve(PermissionPolicy::Allow, Some(PermissionMode::Auto))
                .unwrap();
        assert_eq!(cfg.effective_mode, PermissionMode::Auto);
        assert_eq!(cfg.effective_mode.as_wire_str(), "auto");
        assert_eq!(cfg.mode_source, ModeSource::Explicit);
    }

    #[test]
    fn resolved_permission_config_ask_plus_explicit_auto_is_ok_with_warning() {
        // ask + auto is compatible-with-warning: residual escalations still surface
        // cards; internally-approved calls bypass the ask flow silently.
        // `auto` is a model classifier, not a bypass — some requests still escalate.
        let result =
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, Some(PermissionMode::Auto));
        assert!(
            result.is_ok(),
            "ask + auto must succeed (warn only), got: {result:?}"
        );
        let cfg = result.unwrap();
        assert_eq!(cfg.effective_mode, PermissionMode::Auto);
        assert_eq!(cfg.mode_source, ModeSource::Explicit);
    }

    #[test]
    fn resolved_permission_config_reject_plus_explicit_auto_is_startup_error() {
        // reject + auto: inverted-security worst case — policy says deny but
        // adapter auto-approves everything internally.
        let result =
            ResolvedPermissionConfig::resolve(PermissionPolicy::Reject, Some(PermissionMode::Auto));
        assert!(result.is_err(), "reject + auto must be a startup error");
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("auto"), "error must mention auto, got: {msg}");
    }

    #[test]
    fn permission_mode_auto_wire_string_is_correct() {
        assert_eq!(PermissionMode::Auto.as_wire_str(), "auto");
        assert!(!PermissionMode::Auto.is_default());
    }

    /// Synchronous denial (missing options): `acp_read` and `acp_write` must share one nonce.
    ///
    /// Before the nonce-threading fix, `emit_permission_read_non_actionable` generated
    /// its own nonce independently of the nonce passed to `finish_permission_sync`, so
    /// the two telemetry frames carried different nonces. Desktop's nonce-only rule then
    /// left the read card live because the write could never find it.
    #[tokio::test]
    async fn sync_denial_malformed_options_read_and_write_carry_same_nonce() {
        let mut client = spawn_inert_client().await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);

        // Request with no options field — triggers the malformed path.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 77,
            "method": "session/request_permission",
            "params": {
                "sessionId": "sess",
                "subject": "read a file"
                // "options" deliberately omitted
            }
        });
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("malformed denial must not error");

        let events = obs.snapshot();

        let read_nonce = events
            .iter()
            .find(|e| e.kind == "acp_read" && e.authorization.is_some())
            .and_then(|e| e.authorization.as_ref())
            .map(|a| a.request_nonce.clone())
            .expect("acp_read with authorization must be emitted");

        let write_nonce = events
            .iter()
            .find(|e| e.kind == "acp_write" && e.authorization.is_some())
            .and_then(|e| e.authorization.as_ref())
            .map(|a| a.request_nonce.clone())
            .expect("acp_write with authorization must be emitted");

        assert_eq!(
            read_nonce, write_nonce,
            "acp_read and acp_write must carry the same nonce so Desktop can retire the card; \
             read={read_nonce}, write={write_nonce}"
        );
    }

    /// Synchronous denial (preflight failure): `acp_read` and `acp_write` must share one nonce.
    #[tokio::test]
    async fn sync_denial_preflight_failure_read_and_write_carry_same_nonce() {
        let mut client = spawn_inert_client().await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);

        // Oversize subject triggers admission preflight failure.
        let oversize_subject = "x".repeat(OBSERVER_MAX_PLAINTEXT_LEN + 1);
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 88,
            "method": "session/request_permission",
            "params": {
                "sessionId": "sess",
                "subject": oversize_subject,
                "options": [
                    {"optionId": "opt-allow", "kind": "allow_once", "name": "Allow"},
                    {"optionId": "opt-deny",  "kind": "reject_once", "name": "Deny"}
                ]
            }
        });
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("preflight denial must not error");

        let events = obs.snapshot();

        let read_nonce = events
            .iter()
            .find(|e| e.kind == "acp_read" && e.authorization.is_some())
            .and_then(|e| e.authorization.as_ref())
            .map(|a| a.request_nonce.clone())
            .expect("acp_read with authorization must be emitted");

        let write_nonce = events
            .iter()
            .find(|e| e.kind == "acp_write" && e.authorization.is_some())
            .and_then(|e| e.authorization.as_ref())
            .map(|a| a.request_nonce.clone())
            .expect("acp_write with authorization must be emitted");

        assert_eq!(
            read_nonce, write_nonce,
            "acp_read and acp_write must carry the same nonce so Desktop can retire the card; \
             read={read_nonce}, write={write_nonce}"
        );
    }

    // ── F1: production-seam — description reaches the pending card ────────────
    //
    // Carl's bar: a regression that goes through `handle_permission_request` and
    // asserts the extracted description makes it into the published sentinel
    // content — not just the pure extractor function.
    //
    // Shape: buzz-agent v2 (`params.title` + `params.subject.toolCall.rawInput`).
    // The published kind-9 content is captured from the relay test publisher's
    // event channel and parsed to confirm the `description` field carries the
    // expected `"<tool_name>(<rawInput_json>)"` form.

    #[tokio::test]
    async fn production_seam_description_reaches_pending_card_sentinel() {
        // Script: read one line (the permission response when decided), then idle.
        let capture_file =
            std::env::temp_dir().join(format!("buzz-acp-desc-seam-{}.json", uuid::Uuid::new_v4()));
        let script = format!(
            r#"read -r resp; printf '%s' "$resp" > {capture}; sleep 5"#,
            capture = capture_file.display(),
        );
        let mut client = spawn_script(&script).await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);

        // Capture every published event, including the kind-9 pending sentinel.
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        let (publisher, event_rx) = crate::relay::RelayEventPublisher::test_pair();
        let published: std::sync::Arc<std::sync::Mutex<Vec<nostr::Event>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let published_drain = published.clone();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(ev) = rx.recv().await {
                published_drain.lock().unwrap().push(ev);
            }
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000010").unwrap()),
            None,
        );
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // v2 buzz-agent wire shape: params.title = tool name,
        // params.subject.toolCall.rawInput = call.arguments object.
        // Two distinct commands (ls vs rm) must produce distinguishable descriptions.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 55,
            "method": "session/request_permission",
            "params": {
                "sessionId": "sess-seam",
                "title": "fake__shell",
                "subject": {
                    "type": "tool_call",
                    "toolCall": {
                        "toolCallId": "tc-seam",
                        "title": "fake__shell",
                        "rawInput": {"command": "ls -la /tmp"},
                    },
                },
                "options": [
                    {"optionId": "opt-allow", "kind": "allow_once", "name": "Allow"},
                    {"optionId": "opt-deny",  "kind": "reject_once", "name": "Deny"},
                ],
            }
        });
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("registration must succeed");

        // Wait for the kind-9 sentinel to be published (auto-ACKed by test_pair).
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let found = published
                .lock()
                .unwrap()
                .iter()
                .any(|ev| ev.kind.as_u16() == 9);
            if found || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // Extract the kind-9 event content and parse the sentinel payload.
        let kind9_content = {
            let guard = published.lock().unwrap();
            guard
                .iter()
                .find(|ev| ev.kind.as_u16() == 9)
                .map(|ev| ev.content.clone())
                .expect("kind-9 sentinel must have been published")
        };
        let payload: serde_json::Value =
            serde_json::from_str(&kind9_content).expect("kind-9 content must be valid JSON");

        // The description field must carry the tool name and the rawInput summary —
        // confirming that `description_from_request_permission` is wired to the real
        // sentinel-building path, not just tested as a pure function.
        let description = payload["description"]
            .as_str()
            .expect("description must be a string in the published kind-9 content");
        assert!(
            description.starts_with("fake__shell("),
            "sentinel description must include the tool name; got: {description:?}"
        );
        assert!(
            description.contains("ls -la /tmp"),
            "sentinel description must carry the rawInput command; got: {description:?}"
        );

        // Regression binding: removing `description_from_request_permission` from the
        // `handle_permission_request` path (passing None always) turns this test red
        // because `payload["description"]` becomes null and `as_str()` fails.

        let _ = std::fs::remove_file(&capture_file);
    }

    // ── F2: ordinary-timeout path publishes the resolved edit ─────────────────
    //
    // Bug reproduced: `retransmit_resolved_edit` was spawned with `entry_deadline`,
    // which is ALREADY PAST when an ordinary timeout fires (the entry expired →
    // deadline = then, now > then → loop exits immediately with zero publish
    // attempts). Fix: compute `delivery_deadline = Instant::now() +
    // RESOLVED_DELIVERY_WINDOW_SECS` at resolution time.
    //
    // Mutation proof: reverting the production path back to `entry_deadline`
    // (already past at resolution time) makes the retransmit task exit without
    // publishing any kind-40003 event — this test goes red.

    #[tokio::test(start_paused = true)]
    async fn ordinary_timeout_publishes_resolved_edit() {
        // Script: capture the permission response (timed_out denial), then idle.
        let capture_file = std::env::temp_dir().join(format!(
            "buzz-acp-timeout-retransmit-{}.json",
            uuid::Uuid::new_v4()
        ));
        let script = format!(
            r#"read -r resp; printf '%s' "$resp" > {capture}; sleep 600"#,
            capture = capture_file.display(),
        );
        let mut client = spawn_script(&script).await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);

        // Collect every published event so we can count kind-40003 resolved edits.
        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        // test_pair auto-ACKs every sentinel → entry transitions Publishing→Pending.
        let (publisher, event_rx) = crate::relay::RelayEventPublisher::test_pair();
        let published_40003: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let drain_40003 = published_40003.clone();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(ev) = rx.recv().await {
                if ev.kind.as_u16() == 40003 {
                    drain_40003.lock().unwrap().push(ev.id.to_hex());
                }
            }
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000011").unwrap()),
            None,
        );
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (_perm_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Register the permission request with a short deadline (10s from now,
        // under paused time so it won't actually elapse without explicit advance).
        let perm_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        let msg = perm_request(1, default_opts());
        client
            .handle_permission_request(&msg, perm_deadline)
            .await
            .expect("registration must succeed");

        let idle = std::time::Duration::from_millis(200);
        let max_dur = std::time::Duration::from_secs(60);

        // Pass 1: let the loop run briefly to pick up the auto-ACK from test_pair
        // (the background ACK task runs immediately since test_pair resolves Accepted).
        // This transitions the entry from Publishing → Pending.
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            client.read_until_response_with_idle_timeout(
                "sess-timeout-retransmit",
                999,
                idle,
                hard_deadline,
                max_dur,
            ),
        )
        .await;

        // Advance virtual time past the permission deadline (10s) so the expired
        // Pending entry is visible to the next loop iteration.
        tokio::time::advance(std::time::Duration::from_secs(
            SENTINEL_PUBLISH_TIMEOUT_SECS + 11,
        ))
        .await;

        // Pass 2: drive the loop to detect the expired entry → finish_permission
        // writes the timed_out denial and spawns the resolved-edit retransmit task.
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            client.read_until_response_with_idle_timeout(
                "sess-timeout-retransmit",
                999,
                std::time::Duration::from_millis(50),
                hard_deadline,
                max_dur,
            ),
        )
        .await;

        // Entry must be removed (timed_out).
        assert!(
            client.pending_permissions.is_empty(),
            "entry must be gone after ordinary timeout"
        );

        // Wait for the detached retransmit task to publish the resolved kind-40003 edit.
        // Under paused time, advance a generous window for the first attempt.
        let retransmit_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(65);
        loop {
            let count = published_40003.lock().unwrap().len();
            if count >= 1 || tokio::time::Instant::now() >= retransmit_deadline {
                break;
            }
            // Advance time in small steps to let the spawned task run its first attempt.
            tokio::time::advance(std::time::Duration::from_millis(100)).await;
        }

        let resolved_count = published_40003.lock().unwrap().len();
        assert!(
            resolved_count >= 1,
            "ordinary timeout must publish at least one kind-40003 resolved edit \
             (would be 0 with the old entry_deadline which is already past at timeout); \
             got {resolved_count} publish(es)"
        );

        let _ = std::fs::remove_file(&capture_file);
    }

    // ── F4: first-wins — early_decision guards subsequent valid decisions ──────
    //
    // Regression: before F4, `entry.early_decision = Some(decision)` was
    // unconditional, so a later conflicting decision could overwrite the first.
    // After F4, the guard `if entry.early_decision.is_none()` ensures only the
    // FIRST valid decision is buffered; subsequent ones are ignored.
    //
    // Mutation proof: removing the `is_none()` guard (changing it back to an
    // unconditional assignment) lets the second Allow overwrite the first Reject,
    // so the applied write carries "opt-allow" and the assertion on "opt-reject"
    // goes red.

    #[tokio::test]
    async fn early_decision_first_wins_reject_then_allow_reject_applied() {
        // Script: capture the decision response (one JSON-RPC result line).
        let capture_file =
            std::env::temp_dir().join(format!("buzz-acp-first-wins-{}.json", uuid::Uuid::new_v4()));
        let script = format!(
            r#"read -r resp; printf '%s' "$resp" > {capture}; sleep 5"#,
            capture = capture_file.display(),
        );
        let mut client = spawn_script(&script).await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (perm_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Manually insert a Publishing entry — the sentinel is already "published"
        // from the agent's perspective (we own the ack_tx). The test controls when
        // the ACK fires so both decisions arrive before the transition to Pending.
        let nonce = "nonce-first-wins".to_string();
        let entry_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
        client.pending_permissions.insert(
            "99".to_string(),
            PermissionEntry {
                nonce: nonce.clone(),
                options_snapshot: default_opts()
                    .iter()
                    .map(|(id, kind, name)| {
                        serde_json::json!({"optionId": id, "kind": kind, "name": name})
                    })
                    .collect(),
                card_actions: test_card_actions(),
                state: PermissionEntryState::Publishing,
                deadline: entry_deadline,
                expiry_unix_secs: 0,
                sentinel_event_id: Some("sentinel-fw".to_string()),
                early_decision: None,
                description: None,
            },
        );
        // Install a manual ACK channel so we control when the ACK fires.
        let (ack_tx, ack_rx) = tokio::sync::mpsc::channel::<(String, crate::relay::AckOutcome)>(1);
        client.sentinel_ack_result_rx = Some(ack_rx);

        // Pre-send the full decision sequence before the loop runs:
        //   Reject (first) → buffered as early_decision
        //   Reject (dup)   → ignored because early_decision is already set
        //   Allow          → ignored because early_decision is already set
        //   Allow (dup)    → ignored because early_decision is already set
        // Channel capacity (8) holds all four without blocking.
        for option_id in &["opt-reject", "opt-reject", "opt-allow", "opt-allow"] {
            perm_tx
                .send(PermissionDecision {
                    request_nonce: nonce.clone(),
                    option_id: (*option_id).to_string(),
                })
                .await
                .expect("send must succeed");
        }

        // Fire the ACK from a background task with a slight delay so all four
        // decisions are processed first (buffered) before the ACK transitions
        // Publishing → Pending → applies the early decision.
        let ack_tx_clone = ack_tx;
        tokio::spawn(async move {
            // Let all four decision messages be processed by the decision arm first.
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let _ = ack_tx_clone
                .send(("99".to_string(), crate::relay::AckOutcome::Accepted))
                .await;
        });

        // Drive the loop until the entry is resolved (map empties).
        // The loop processes: (1) Reject → buffered as early_decision,
        // (2) Reject dup → ignored (early_decision already set),
        // (3) Allow → ignored (early_decision already set),
        // (4) Allow dup → ignored (early_decision already set),
        // (5) ACK Accepted → Publishing→Pending → apply buffered Reject → map empties.
        let idle = std::time::Duration::from_millis(200);
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_until_response_with_idle_timeout(
                "sess-first-wins",
                999,
                idle,
                hard,
                std::time::Duration::from_secs(10),
            ),
        )
        .await;

        // Entry must be gone — decision was applied.
        assert!(
            client.pending_permissions.is_empty(),
            "entry must be removed after ACK + early decision applied"
        );

        // Observer must show exactly one applied write with the REJECT option id.
        let events = obs.snapshot();
        let applied_writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("applied"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            applied_writes.len(),
            1,
            "exactly one applied write must be emitted; got: {applied_writes:?}"
        );
        // The applied write's payload carries the decision optionId in the ACP
        // result. Use the same path as the existing denial-optionId tests.
        let payload = &applied_writes[0].payload;
        assert_eq!(
            payload["result"]["outcome"]["optionId"].as_str(),
            Some("opt-reject"),
            "applied decision must be the first (Reject) — not the second (Allow); \
             mutation: remove is_none() guard → Allow overwrites → this assertion goes red; \
             got: {payload}"
        );

        let _ = std::fs::remove_file(&capture_file);
    }

    // ── Thread routing: nonce-mismatch drop in two concurrent read loops ──────
    //
    // When two read loops own distinct nonce snapshots in the same channel,
    // a decision fan-outed to both must be applied by the owner (Thread A)
    // and silently dropped by the non-owner (Thread B, nonce mismatch).
    // Thread B must remain pending and still be resolvable by its own decision.
    //
    // This is the read-loop counterpart to the lib.rs fan-out routing tests:
    // those prove delivery to both mpsc receivers; this proves the read loop
    // correctly handles a mismatched nonce without resolving the wrong entry.
    //
    // Mutation proof: removing the `card_actions.accepts(decision.option_id)`
    // check in the read loop (accepting any nonce unconditionally) causes Thread B
    // to consume Thread A's decision — its entry resolves on the wrong nonce —
    // and the assertion `!client_b.pending_permissions.is_empty()` goes red.
    // (Actually the nonce check is in the entry lookup, not card_actions; the
    // test proves the correct entry-by-nonce lookup path.)
    //
    // Mutation: removing the `nonce == entry.nonce` guard (accepting any nonce)
    // makes client_b apply Thread A's decision → map empties → assert fires.

    #[tokio::test]
    async fn two_read_loops_same_channel_nonce_mismatch_dropped_by_sibling() {
        let capture_a =
            std::env::temp_dir().join(format!("buzz-acp-routing-a-{}.json", uuid::Uuid::new_v4()));
        let capture_b =
            std::env::temp_dir().join(format!("buzz-acp-routing-b-{}.json", uuid::Uuid::new_v4()));
        let script_a = format!(
            r#"read -r resp; printf '%s' "$resp" > {capture}; sleep 600"#,
            capture = capture_a.display()
        );
        let script_b = format!(
            r#"read -r resp; printf '%s' "$resp" > {capture}; sleep 600"#,
            capture = capture_b.display()
        );

        // Client A and Client B share the same channel_id so their decisions
        // would be fan-outed to each other's read loop.
        let channel_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000041").unwrap();

        let make_client = |script: &str| {
            let s = script.to_string();
            async move {
                let mut c = spawn_script(&s).await;
                c.set_permission_config(
                    ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
                );
                c.set_owner_pubkey_known(true);
                let keys = Keys::generate();
                let owner_hex = keys.public_key().to_hex();
                let (publisher, event_rx) = crate::relay::RelayEventPublisher::test_pair();
                tokio::spawn(async move {
                    let mut rx = event_rx;
                    while rx.recv().await.is_some() {}
                });
                c.set_relay_publisher(publisher, keys.clone());
                c.set_agent_owner_pubkey_hex(Some(owner_hex));
                c.set_turn_initiator_pubkey(Some(keys.public_key()));
                c.set_turn_channel_context(Some(channel_id), None);
                let obs = crate::observer::ObserverHandle::in_process();
                c.set_observer(Some(obs), 0);
                c
            }
        };
        let mut client_a = make_client(&script_a).await;
        let mut client_b = make_client(&script_b).await;

        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);

        // Register permission requests for both clients.
        let msg_a = perm_request(61, default_opts());
        let msg_b = perm_request(62, default_opts());
        client_a
            .handle_permission_request(&msg_a, hard)
            .await
            .expect("A registration must succeed");
        client_b
            .handle_permission_request(&msg_b, hard)
            .await
            .expect("B registration must succeed");

        // Extract the auto-generated nonces.
        let nonce_a = client_a
            .pending_permissions
            .values()
            .next()
            .map(|e| e.nonce.clone())
            .expect("client_a must have one entry");
        let nonce_b = client_b
            .pending_permissions
            .values()
            .next()
            .map(|e| e.nonce.clone())
            .expect("client_b must have one entry");
        assert_ne!(nonce_a, nonce_b, "two distinct nonces must be generated");

        let idle = std::time::Duration::from_millis(100);
        let ten_s = std::time::Duration::from_secs(10);

        // ── Phase 1: fan-out Thread A's decision to BOTH read loops ───────────
        // Send nonce_a's decision to client_a's loop (it should apply) and
        // ALSO to client_b's loop (it should drop — nonce mismatch).
        {
            let (tx_a, rx_a) = tokio::sync::mpsc::channel::<PermissionDecision>(4);
            tx_a.send(PermissionDecision {
                request_nonce: nonce_a.clone(),
                option_id: "opt-allow".to_string(),
            })
            .await
            .expect("send must succeed");
            client_a.install_permission_decision_rx(rx_a);
        }
        // Same decision to client_b (simulates fan-out; nonce_a ≠ nonce_b → dropped).
        {
            let (tx_b_cross, rx_b_cross) = tokio::sync::mpsc::channel::<PermissionDecision>(4);
            tx_b_cross
                .send(PermissionDecision {
                    request_nonce: nonce_a.clone(), // Thread A's nonce, wrong for B
                    option_id: "opt-allow".to_string(),
                })
                .await
                .expect("send must succeed");
            client_b.install_permission_decision_rx(rx_b_cross);
        }

        // Drive both loops simultaneously; Thread A resolves, Thread B idles out.
        let (res_a, res_b) = tokio::join!(
            tokio::time::timeout(
                std::time::Duration::from_secs(3),
                client_a.read_until_response_with_idle_timeout(
                    "sess-routing-a",
                    61,
                    idle,
                    hard,
                    ten_s,
                )
            ),
            tokio::time::timeout(
                std::time::Duration::from_secs(3),
                client_b.read_until_response_with_idle_timeout(
                    "sess-routing-b",
                    62,
                    idle,
                    hard,
                    ten_s,
                )
            ),
        );
        // client_a resolves (response written → loop returns Ok).
        assert!(
            res_a.is_ok(),
            "Thread A's loop must complete (decision applied) within the timeout"
        );
        // client_b may timeout (no response for B yet) — that's expected.
        drop(res_b);

        assert!(
            client_a.pending_permissions.is_empty(),
            "Thread A's entry must be resolved after its own decision; \
             mutation: nonce mismatch not checked → Thread B consumes A's decision \
             → client_a's entry is never resolved → this fires instead"
        );
        assert!(
            !client_b.pending_permissions.is_empty(),
            "Thread B's entry must remain pending after Thread A's decision fan-out; \
             mutation: nonce not checked → B wrongly applies A's decision → map empty → this fires"
        );

        // ── Phase 2: Thread B's own decision arrives and is applied ───────────
        {
            let (tx_b_own, rx_b_own) = tokio::sync::mpsc::channel::<PermissionDecision>(4);
            tx_b_own
                .send(PermissionDecision {
                    request_nonce: nonce_b.clone(),
                    option_id: "opt-allow".to_string(),
                })
                .await
                .expect("send must succeed");
            client_b.install_permission_decision_rx(rx_b_own);
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            client_b.read_until_response_with_idle_timeout(
                "sess-routing-b2",
                62,
                idle,
                hard,
                ten_s,
            ),
        )
        .await;
        assert!(
            client_b.pending_permissions.is_empty(),
            "Thread B's entry must be resolved after its own decision arrives"
        );

        let _ = std::fs::remove_file(&capture_a);
        let _ = std::fs::remove_file(&capture_b);
    }

    // ── F2: unconditional first attempt with an already-expired deadline ──────
    //
    // The retransmit loop must publish the resolved edit at least once even when
    // the delivery_deadline has already elapsed at call time. This covers the
    // ordinary-timeout path where the card's entry_deadline expired before
    // `finish_permission` called `retransmit_resolved_edit`.
    //
    // Mutation proof: reverting the `!first_attempt &&` guard (i.e. making the
    // deadline check unconditional at loop-top) causes the loop to return
    // immediately on an already-expired deadline without publishing — the
    // assertion that a kind-40003 event was emitted goes red.

    #[tokio::test(start_paused = true)]
    async fn retransmit_resolved_edit_unconditional_first_attempt_on_expired_deadline() {
        let keys = Keys::generate();

        let event = nostr::EventBuilder::new(nostr::Kind::from(40003), "resolved-expired")
            .sign(&keys)
            .await
            .unwrap();
        let event_id = event.id.to_hex();

        // Use an accepting publisher — we only need to confirm the event is
        // attempted once despite the expired deadline.
        let (publisher, mut event_rx) = crate::relay::RelayEventPublisher::test_pair();

        let collected: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let collected_drain = collected.clone();
        tokio::spawn(async move {
            while let Some(ev) = event_rx.recv().await {
                if ev.kind.as_u16() == 40003 {
                    collected_drain.lock().unwrap().push(ev.id.to_hex());
                }
            }
        });

        // Supply an already-expired deadline.
        // Under start_paused, Instant::now() is fixed at epoch; subtract 1ns.
        let already_expired = tokio::time::Instant::now() - std::time::Duration::from_nanos(1);
        let handle = tokio::spawn(retransmit_resolved_edit(publisher, event, already_expired));

        // Advance time past the per-attempt timeout so the spawned task drains.
        tokio::time::advance(std::time::Duration::from_secs(
            SENTINEL_PUBLISH_TIMEOUT_SECS + 1,
        ))
        .await;
        tokio::task::yield_now().await;
        let _ = tokio::time::timeout(std::time::Duration::from_millis(200), handle).await;
        tokio::task::yield_now().await;

        let seen = collected.lock().unwrap().clone();
        assert!(
            !seen.is_empty(),
            "retransmit must publish even when delivery_deadline is already expired at call time; \
             mutation: unconditional loop-top deadline check → zero publishes → this goes red"
        );
        assert_eq!(
            seen[0], event_id,
            "published event must carry the same stable signed id"
        );
    }

    // ── F2: always-Uncertain bounded-exit ─────────────────────────────────────
    //
    // A retransmit loop that perpetually receives `Uncertain` (via test_pair_silent
    // which drops ack_tx so every await resolves as RecvError → Uncertain) must
    // still terminate once the delivery window elapses. Under paused tokio time
    // we advance past the window and confirm the loop exits.
    //
    // Mutation proof: removing the `delivery_deadline` gate from the retry loop
    // (i.e. looping forever on Uncertain) makes `handle.is_finished()` never true
    // within the test budget — the polling loop exhausts its window and the
    // subsequent `timeout(1s, handle)` fires → assertion fails.

    #[tokio::test(start_paused = true)]
    async fn retransmit_resolved_edit_always_uncertain_bounded_exit() {
        let keys = Keys::generate();

        let event = nostr::EventBuilder::new(nostr::Kind::from(40003), "resolved-uncertain")
            .sign(&keys)
            .await
            .unwrap();

        // test_pair_silent drops ack_tx on each PublishEventAcked → every
        // ack_rx.await yields Err(RecvError) → unwrapped as Uncertain.
        let (publisher, _event_rx) = crate::relay::RelayEventPublisher::test_pair_silent();

        // Short delivery window to advance past quickly.
        let delivery_window = std::time::Duration::from_secs(4);
        let delivery_deadline = tokio::time::Instant::now() + delivery_window;
        let handle = tokio::spawn(retransmit_resolved_edit(
            publisher,
            event,
            delivery_deadline,
        ));

        // Each Uncertain attempt is followed by RESOLVED_RETRANSMIT_BACKOFF sleep,
        // and resolving `ack_rx` requires the test_pair_silent task to run (to drop
        // ack_tx). Under paused time, advance in small steps so channel-driven
        // interleaving between the retransmit task and the silent publisher task
        // can proceed; tokio auto-advances through parked timers on each step.
        let poll_deadline =
            tokio::time::Instant::now() + delivery_window + std::time::Duration::from_secs(30);
        loop {
            if handle.is_finished() || tokio::time::Instant::now() >= poll_deadline {
                break;
            }
            tokio::time::advance(std::time::Duration::from_millis(500)).await;
            tokio::task::yield_now().await;
        }

        let join_result = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;
        assert!(
            join_result.is_ok(),
            "retransmit loop must exit once the delivery window elapses even when every \
             attempt returns Uncertain; mutation: increase `delivery_window` (the injected \
             4s deadline passed directly to this test) → loop never completes within test \
             budget → timeout fires"
        );
    }

    // ── F2: old-expiry-crossing retransmit — decision applied after card expires ─
    //
    // Both tests below verify that the delivery window for the kind-40003
    // resolved edit is anchored at **resolution time** (`now() + 300s`), NOT
    // at the original card deadline.  The card deadline has already elapsed
    // when `finish_permission` runs; if the old `entry.deadline` were used as
    // `delivery_deadline`, the retransmit loop would have an already-expired
    // window after its first attempt — only one publish, never a retry.
    //
    // Mutation proof (both tests): restoring `entry.deadline` as the spawn
    // argument to `retransmit_resolved_edit` sets `delivery_deadline` to a
    // value that is already < `now()` at retry time.  The second assertion
    // (`resolved_ids.len() >= 2`) goes red — only one publish ever occurs.

    // Case A: disconnect/Uncertain on the first resolved-edit attempt, resolved
    //         after the original card deadline — second attempt must still land.
    #[tokio::test(start_paused = true)]
    async fn resolved_edit_retransmitted_across_old_card_expiry_disconnect() {
        let capture_file = std::env::temp_dir().join(format!(
            "buzz-acp-expiry-disconnect-{}.json",
            uuid::Uuid::new_v4()
        ));
        let script = format!(
            r#"read -r resp; printf '%s' "$resp" > {capture}; sleep 600"#,
            capture = capture_file.display(),
        );
        let mut client = spawn_script(&script).await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);

        let keys = Keys::generate();
        let owner_hex = keys.public_key().to_hex();
        // One Uncertain on the first kind-40003, then Accepted — simulates
        // a disconnect that clears between the first and second attempt.
        let (publisher, event_rx) =
            crate::relay::RelayEventPublisher::test_pair_resolved_reconnect(1);
        let published_40003: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let drain = published_40003.clone();
        tokio::spawn(async move {
            let mut rx = event_rx;
            while let Some(ev) = rx.recv().await {
                if ev.kind.as_u16() == 40003 {
                    drain.lock().unwrap().push(ev.id.to_hex());
                }
            }
        });
        client.set_relay_publisher(publisher, keys.clone());
        client.set_agent_owner_pubkey_hex(Some(owner_hex));
        client.set_turn_initiator_pubkey(Some(keys.public_key()));
        client.set_turn_channel_context(
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000031").unwrap()),
            None,
        );
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);
        let (perm_tx, perm_rx) = tokio::sync::mpsc::channel::<PermissionDecision>(8);
        client.install_permission_decision_rx(perm_rx);

        // Short card deadline — 1 s. The entry will expire before the decision
        // is applied, proving the delivery window is not tied to entry.deadline.
        let short_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        let msg = perm_request(51, default_opts());
        client
            .handle_permission_request(&msg, short_deadline)
            .await
            .expect("registration must succeed");

        let nonce = client
            .pending_permissions
            .get("51")
            .expect("entry must be in map")
            .nonce
            .clone();

        // Pre-send decision before the loop; the entry is still in Publishing
        // state at this point — decision is buffered as early_decision.
        perm_tx
            .send(PermissionDecision {
                request_nonce: nonce,
                option_id: "opt-allow".to_string(),
            })
            .await
            .expect("send must succeed");

        // Advance past the short card deadline BEFORE running the loop.
        // This ensures the original entry.deadline has already elapsed when
        // finish_permission fires — simulating a long-delayed decision.
        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        tokio::task::yield_now().await;

        // Drive the loop; finish_permission will compute delivery_deadline =
        // now() + 300s, well beyond the expired entry.deadline.
        let idle = std::time::Duration::from_millis(200);
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_until_response_with_idle_timeout(
                "sess-expiry-dc",
                51,
                idle,
                hard,
                std::time::Duration::from_secs(10),
            ),
        )
        .await;

        // Wait for both retransmit attempts to complete.
        let poll_end = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            if published_40003.lock().unwrap().len() >= 2 || tokio::time::Instant::now() >= poll_end
            {
                break;
            }
            tokio::time::advance(std::time::Duration::from_millis(500)).await;
            tokio::task::yield_now().await;
        }

        let ids = published_40003.lock().unwrap().clone();
        assert!(
            ids.len() >= 2,
            "resolved edit must be retransmitted after an Uncertain outcome even when \
             the original card deadline has already elapsed; \
             mutation: restore entry.deadline as delivery_deadline → second attempt \
             never fires (already-expired window) → len()==1 → this assertion goes red; \
             saw {ids:?}"
        );
        assert!(
            ids.windows(2).all(|w| w[0] == w[1]),
            "every retransmission must carry the same signed event id; got {ids:?}"
        );
        let _ = std::fs::remove_file(&capture_file);
    }

    // Case B: lost-OK (connected socket) resolved after the original card
    //         deadline — the per-attempt timeout sweeps and a retry still lands.
    #[tokio::test(start_paused = true)]
    async fn resolved_edit_retransmitted_across_old_card_expiry_lost_ok() {
        let keys = Keys::generate();
        // One lost-OK on the first kind-40003 attempt, then Accepted.
        let (publisher, mut event_rx) =
            crate::relay::RelayEventPublisher::test_pair_resolved_lost_ok(1);

        let published_40003: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let drain = published_40003.clone();
        tokio::spawn(async move {
            while let Some(ev) = event_rx.recv().await {
                if ev.kind.as_u16() == 40003 {
                    drain.lock().unwrap().push(ev.id.to_hex());
                }
            }
        });

        // Sign the resolved edit once; the retransmit loop resends this exact event.
        let event = build_kind40003_sentinel(
            &keys,
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000032").unwrap(),
            "original-event-id-lost-ok",
            "resolved-lost-ok-expiry",
        )
        .expect("sentinel must build");

        // delivery_deadline = 60s from now (well within the 300s production window).
        // The old entry.deadline would have been, say, 1s — already expired.
        let delivery_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);

        // Advance past the "old card expiry" (1s) before starting, so any code
        // using entry.deadline as the window would already be expired.
        tokio::time::advance(std::time::Duration::from_secs(2)).await;
        tokio::task::yield_now().await;

        let task = tokio::spawn(retransmit_resolved_edit(
            publisher,
            event,
            delivery_deadline,
        ));

        // Under paused time the per-attempt deadline (SENTINEL_PUBLISH_TIMEOUT_SECS)
        // auto-advances and the stuck waiter sweeps. Then the backoff elapses and
        // the second attempt (Accepted) lands.
        let joined = tokio::time::timeout(std::time::Duration::from_secs(120), task).await;
        assert!(joined.is_ok(), "retransmit task must terminate");
        tokio::task::yield_now().await;

        let ids = published_40003.lock().unwrap().clone();
        assert!(
            ids.len() >= 2,
            "resolved edit must be retransmitted via a lost-OK sweep even when started \
             after the old card deadline elapsed; \
             mutation: restore entry.deadline as delivery_deadline → after the lost-OK \
             sweep, delivery_deadline is already past → retry loop exits → len()==1 → \
             this assertion goes red; saw {ids:?}"
        );
        assert!(
            ids.windows(2).all(|w| w[0] == w[1]),
            "every retransmission must carry the same signed event id; got {ids:?}"
        );
    }

    // ── F3: Rust read-loop coverage — allow_always / reject_always rejected ───
    //
    // The read loop (via the `ask` policy path in `handle_permission_request`)
    // snapshots only the `allow_once` and `reject_once` option IDs into
    // `card_actions`. A decision carrying `allow_always` or `reject_always`
    // as its `option_id` does NOT match either snapshotted ID and is silently
    // ignored (logged as "not a ruled card action"). This test drives the loop
    // with all four option kinds offered by the adapter and confirms:
    //   - `allow_always` and `reject_always` decisions are dropped.
    //   - `allow_once` and `reject_once` decisions ARE accepted.
    //
    // The test delivers each persistent kind in isolation and inspects the
    // observer AFTER each partial loop run:
    //   step 1: allow_always alone → entry still Pending, zero applied writes.
    //   step 2: reject_always alone → entry still Pending, zero applied writes.
    //   step 3: allow_once → entry resolved, exactly one applied write.
    //
    // Mutation proof: changing `CardActions::accepts()` to always return `true`
    // allows `allow_always` through on step 1. The entry is resolved early
    // (map empty) and the observer shows an applied write before allow_once
    // ever arrives — the intermediate step-1 pending assertion goes red.

    #[tokio::test]
    async fn allow_always_and_reject_always_decisions_are_ignored_by_read_loop() {
        let mut client = spawn_script("sleep 600").await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);

        // Offer all four option kinds. The read loop must only snapshot
        // allow_once + reject_once into card_actions.
        let four_opts: &[(&str, &str, &str)] = &[
            ("opt-allow-once", "allow_once", "Allow once"),
            ("opt-reject-once", "reject_once", "Deny once"),
            ("opt-allow-always", "allow_always", "Always allow"),
            ("opt-reject-always", "reject_always", "Always deny"),
        ];
        let msg = perm_request(42, four_opts);
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);

        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("registration must succeed");

        // Extract the nonce from the pending map (auto-generated by handle_permission_request).
        let nonce = client
            .pending_permissions
            .values()
            .next()
            .map(|e| e.nonce.clone())
            .expect("one entry must be in the pending map after registration");

        let short_idle = std::time::Duration::from_millis(100);
        let ten_s = std::time::Duration::from_secs(10);

        // Helper: count applied acp_write events in the observer snapshot.
        let applied_count = |o: &crate::observer::ObserverHandle| {
            o.snapshot()
                .into_iter()
                .filter(|e| {
                    e.kind == "acp_write"
                        && e.authorization
                            .as_ref()
                            .map(|a| a.reason.as_deref() == Some("applied"))
                            .unwrap_or(false)
                })
                .count()
        };

        // `read_until_response_with_idle_timeout` takes `permission_decision_rx`
        // via `.take()` and drops it on return. A fresh channel pair is installed
        // before each step so the next call sees a live receiver with its message
        // already buffered. No outer channel is needed — each step is self-contained.

        // ── Step 1: allow_always alone ────────────────────────────────────────
        // The loop ACKs the sentinel on its first iteration (Publishing → Pending);
        // allow_always is delivered but must be dropped by `card_actions.accepts()`.
        {
            let (tx1, rx1) = tokio::sync::mpsc::channel::<PermissionDecision>(4);
            tx1.send(PermissionDecision {
                request_nonce: nonce.clone(),
                option_id: "opt-allow-always".to_string(),
            })
            .await
            .expect("send must succeed");
            client.install_permission_decision_rx(rx1);
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.read_until_response_with_idle_timeout("sess-f3-aa", 42, short_idle, hard, ten_s),
        )
        .await;

        // Entry must still be pending — allow_always was ignored.
        // Mutation: `accepts()` → true → allow_always resolves the entry →
        // map is empty here → this assert fires.
        assert!(
            !client.pending_permissions.is_empty(),
            "entry must still be pending after allow_always — it is not a ruled card action; \
             mutation: CardActions::accepts() → true → entry resolved early → this fires"
        );
        assert_eq!(
            applied_count(&obs),
            0,
            "no applied ACP write must occur after allow_always; \
             mutation: accepts()→true → applied write emitted → count>0 → this fires"
        );

        // ── Step 2: reject_always alone ───────────────────────────────────────
        {
            let (tx2, rx2) = tokio::sync::mpsc::channel::<PermissionDecision>(4);
            tx2.send(PermissionDecision {
                request_nonce: nonce.clone(),
                option_id: "opt-reject-always".to_string(),
            })
            .await
            .expect("send must succeed");
            client.install_permission_decision_rx(rx2);
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.read_until_response_with_idle_timeout("sess-f3-ra", 42, short_idle, hard, ten_s),
        )
        .await;

        assert!(
            !client.pending_permissions.is_empty(),
            "entry must still be pending after reject_always — it is not a ruled card action"
        );
        assert_eq!(
            applied_count(&obs),
            0,
            "no applied ACP write must occur after reject_always"
        );

        // ── Step 3: allow_once resolves the entry ─────────────────────────────
        {
            let (tx3, rx3) = tokio::sync::mpsc::channel::<PermissionDecision>(4);
            tx3.send(PermissionDecision {
                request_nonce: nonce.clone(),
                option_id: "opt-allow-once".to_string(),
            })
            .await
            .expect("send must succeed");
            client.install_permission_decision_rx(rx3);
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_until_response_with_idle_timeout("sess-f3-ao", 42, short_idle, hard, ten_s),
        )
        .await;

        assert!(
            client.pending_permissions.is_empty(),
            "entry must be resolved after allow_once decision"
        );
        assert_eq!(
            applied_count(&obs),
            1,
            "exactly one applied ACP write must be emitted for allow_once"
        );
    }

    /// F3 reject_once proof: the counterpart to the allow_once step above.
    ///
    /// A fresh client with the same four options registers one permission request.
    /// `reject_once` must resolve the entry and emit exactly one applied ACP write
    /// carrying the reject option ID.
    ///
    /// Mutation proof: changing `CardActions::accepts()` to return only
    /// `option_id == self.allow_id()` (dropping reject acceptance) leaves the
    /// entry pending and emits zero writes — this assertion fires.
    #[tokio::test]
    async fn reject_once_resolves_entry_with_applied_write() {
        tokio::time::pause();
        let mut client = spawn_script("sleep 600").await;
        client.set_permission_config(
            ResolvedPermissionConfig::resolve(PermissionPolicy::Ask, None).unwrap(),
        );
        client.set_owner_pubkey_known(true);
        install_test_relay_context(&mut client);
        let obs = crate::observer::ObserverHandle::in_process();
        client.set_observer(Some(obs.clone()), 0);

        let four_opts: &[(&str, &str, &str)] = &[
            ("opt-allow-once", "allow_once", "Allow once"),
            ("opt-reject-once", "reject_once", "Deny once"),
            ("opt-allow-always", "allow_always", "Always allow"),
            ("opt-reject-always", "reject_always", "Always deny"),
        ];
        let msg = perm_request(99, four_opts);
        let hard = tokio::time::Instant::now() + std::time::Duration::from_secs(300);

        client
            .handle_permission_request(&msg, hard)
            .await
            .expect("registration must succeed");

        let nonce = client
            .pending_permissions
            .values()
            .next()
            .map(|e| e.nonce.clone())
            .expect("one entry must be in the pending map after registration");

        let short_idle = std::time::Duration::from_millis(100);
        let ten_s = std::time::Duration::from_secs(10);

        // ── reject_once resolves the entry ────────────────────────────────────
        {
            let (tx, rx) = tokio::sync::mpsc::channel::<PermissionDecision>(4);
            tx.send(PermissionDecision {
                request_nonce: nonce.clone(),
                option_id: "opt-reject-once".to_string(),
            })
            .await
            .expect("send must succeed");
            client.install_permission_decision_rx(rx);
        }
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_until_response_with_idle_timeout("sess-f3-ro", 99, short_idle, hard, ten_s),
        )
        .await;

        assert!(
            client.pending_permissions.is_empty(),
            "entry must be resolved after reject_once decision; \
             mutation: accepts() drops reject → entry stays pending → this fires"
        );

        // Collect the applied writes and assert exactly one, carrying the reject option ID.
        let events = obs.snapshot();
        let applied_writes: Vec<_> = events
            .iter()
            .filter(|e| {
                e.kind == "acp_write"
                    && e.authorization
                        .as_ref()
                        .map(|a| a.reason.as_deref() == Some("applied"))
                        .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            applied_writes.len(),
            1,
            "exactly one applied ACP write must be emitted for reject_once; \
             mutation: accepts() drops reject → zero writes → this fires; got: {applied_writes:?}"
        );
        let payload = &applied_writes[0].payload;
        assert_eq!(
            payload["result"]["outcome"]["optionId"].as_str(),
            Some("opt-reject-once"),
            "applied write must carry the reject option ID; \
             mutation: response helper emits wrong optionId → this fires; got: {payload}"
        );
    }
}
