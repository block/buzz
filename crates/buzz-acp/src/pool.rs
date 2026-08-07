//! Agent pool — owns N AcpClient instances and dispatches prompt tasks.
//!
//! # Mental model
//!
//! ```text
//!   AgentPool
//!   ├── agents: Vec<Option<OwnedAgent>>   ← idle agents sit here
//!   ├── join_set: JoinSet<()>             ← in-flight tasks
//!   ├── task_map: HashMap<Id, TaskMeta>   ← panic recovery metadata
//!   └── result_tx/rx: mpsc channel        ← tasks return agents here
//!
//!   Dispatch:
//!     try_claim() → OwnedAgent (removed from slot)
//!     spawn run_prompt_task(agent, ...) into join_set
//!     task sends PromptResult { agent, outcome } via result_tx
//!     rx_and_join_set() → poll result_rx for PromptResult
//!     return_agent(agent) → puts agent back in slot
//! ```
//!
//! `AcpClient` is NOT Clone — ownership moves out on claim and back on return.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;
use uuid::Uuid;

use crate::acp::{
    extract_model_config_options, extract_model_state, model_in_catalog,
    resolve_model_switch_method, AcpClient, AcpError, EnvVar, McpServer, ModelSwitchMethod,
    StopReason, SystemPromptTransport,
};
use crate::config::{compose_session_title, DedupMode, PermissionMode};
use crate::observer;
use crate::queue::{
    CancelReason, ContextMessage, ConversationContext, FlushBatch, PromptChannelInfo,
    PromptProfile, PromptProfileLookup, ThreadTags,
};
use crate::relay::{ChannelInfo, RestClient};

/// Window within which agent activity before a hard-cap death qualifies
/// the turn as "recently active" (eligible for requeue instead of dead-letter).
const RECENT_ACTIVITY_WINDOW: Duration = Duration::from_secs(60);

// FlushBatch and BatchEvent derive Clone (added in queue.rs) so we can store
// a recoverable copy in TaskMeta for panic recovery in Queue mode.

/// Metadata stored per in-flight task for panic recovery.
pub struct TaskMeta {
    pub agent_index: usize,
    pub channel_id: Option<Uuid>,
    /// Identifies terminal events when the task panics before returning a result.
    pub turn_id: String,
    /// Clone of batch for Queue mode panic recovery.
    pub recoverable_batch: Option<FlushBatch>,
    /// Control signal for the in-flight prompt task.
    /// `None` for heartbeat tasks (not controllable) and after signal is consumed.
    pub control_tx: Option<tokio::sync::oneshot::Sender<ControlSignal>>,
    /// Steer request channel for non-cancelling mid-turn delivery.
    /// Capacity-1; `try_send` from the main loop fails on `Full`/`Closed`,
    /// in which case the caller must fall back to the universal
    /// `ControlSignal::Steer` cancel+merge path. `None` for heartbeat
    /// tasks only — all prompt tasks install a steer channel regardless
    /// of the agent's name.
    pub steer_tx: Option<tokio::sync::mpsc::Sender<SteerRequest>>,
}

/// Agent-level model capabilities. Populated on first session creation.
/// The catalog is the same across all sessions for a given agent process.
/// Fields are read by the desktop's `get_agent_models` Tauri command (Phase 3).
#[allow(dead_code)] // Scaffolding for desktop integration — fields read via serde.
pub struct AgentModelCapabilities {
    /// Stable: configOptions with category "model" from session/new.
    pub config_options_raw: Vec<serde_json::Value>,
    /// Unstable: SessionModelState from session/new.
    pub available_models_raw: Option<serde_json::Value>,
}

/// Per-channel session IDs and turn counters.
///
/// Separated from `OwnedAgent` so the state machine is testable without
/// spawning a real agent subprocess.
#[derive(Default)]
pub struct SessionState {
    /// channel_id → session_id
    pub sessions: HashMap<Uuid, String>,
    pub heartbeat_session: Option<String>,
    /// Per-channel turn counters for proactive session rotation.
    /// Incremented on each successful prompt; reset when the session is rotated.
    pub turn_counts: HashMap<Uuid, u32>,
    /// Turn counter for the heartbeat session.
    pub heartbeat_turn_count: u32,
    /// channel_id → rendered NIP-AE core prompt section, populated once at
    /// session creation per Tyler's spec (no mid-session refresh).
    pub core_sections: HashMap<Uuid, String>,
}

impl SessionState {
    /// Invalidate the session (and turn counter) for a specific prompt source.
    pub fn invalidate(&mut self, source: &PromptSource) {
        match source {
            PromptSource::Channel(cid) => {
                self.invalidate_channel(cid);
            }
            PromptSource::Heartbeat => {
                self.heartbeat_session = None;
                self.heartbeat_turn_count = 0;
            }
        }
    }

    /// Invalidate a single channel's session and turn counter.
    /// Returns `true` if the channel had an active session.
    pub fn invalidate_channel(&mut self, channel_id: &Uuid) -> bool {
        self.turn_counts.remove(channel_id);
        self.core_sections.remove(channel_id);
        self.sessions.remove(channel_id).is_some()
    }

    /// Invalidate all sessions and turn counters (e.g. after agent exit).
    pub fn invalidate_all(&mut self) {
        self.sessions.clear();
        self.turn_counts.clear();
        self.heartbeat_session = None;
        self.heartbeat_turn_count = 0;
        self.core_sections.clear();
    }

    #[cfg(test)]
    fn has_channel_state(&self, channel_id: &Uuid) -> bool {
        self.sessions.contains_key(channel_id)
            || self.turn_counts.contains_key(channel_id)
            || self.core_sections.contains_key(channel_id)
    }
}

/// Tri-state result of a per-turn canvas fetch.
///
/// - `Present` — a canvas event was found; carry the pointer into `[Context]`.
/// - `Absent`  — relay confirms no canvas (or a blank/deleted canvas). Clears
///   any previously cached pointer for the channel.
/// - `Failed`  — transport timeout, REST error, or parse failure. The caller
///   must serve the last-known cached value (stale-on-failure).
#[derive(Debug)]
pub enum CanvasFetchResult {
    Present(crate::queue::CanvasPointer),
    Absent,
    Failed,
}

/// Process-lifetime per-channel canvas revision cache.
///
/// Keyed by channel `Uuid`. Values:
/// - `Some(pointer)` — last known present canvas revision.
/// - `None`          — confirmed absent (blank/deleted); cleared on `Absent` fetch.
/// - No entry        — never fetched for this channel.
///
/// Independent of `SessionState`: cache survives ACP session rotation/invalidation
/// so the canvas pointer is never lost when a session is recycled.
#[derive(Clone, Default)]
pub struct CanvasRevisionCache {
    inner: Arc<std::sync::RwLock<HashMap<Uuid, Option<crate::queue::CanvasPointer>>>>,
}

impl CanvasRevisionCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve the canvas pointer for the current turn, updating the cache.
    ///
    /// - `Present(p)` → cache `Some(p)`, return `Some(p)`.
    /// - `Absent`     → cache `None`, return `None`.
    /// - `Failed`     → cache unchanged; return last-known value or `None` on
    ///   first-fetch failure.
    pub fn resolve_for_turn(
        &self,
        channel_id: &Uuid,
        result: CanvasFetchResult,
    ) -> Option<crate::queue::CanvasPointer> {
        let mut map = self.inner.write().unwrap_or_else(|p| p.into_inner());
        match result {
            CanvasFetchResult::Present(p) => {
                map.insert(*channel_id, Some(p.clone()));
                Some(p)
            }
            CanvasFetchResult::Absent => {
                map.insert(*channel_id, None);
                None
            }
            CanvasFetchResult::Failed => {
                // Serve stale; do not update the entry.
                map.get(channel_id).and_then(|v| v.clone())
            }
        }
    }
}

/// An agent with its session state, owned by the pool or a running task.
pub struct OwnedAgent {
    pub index: usize,
    pub acp: AcpClient,
    pub state: SessionState,
    /// Model catalog from first session/new. None until first session created.
    pub model_capabilities: Option<AgentModelCapabilities>,
    /// Desired model ID (from `Config.model`). Applied after every `session_new_full()`.
    pub desired_model: Option<String>,
    /// Whether `desired_model` was set by a live `SwitchModel` control signal
    /// (as opposed to being derived from config/persona at spawn). Used by the
    /// desktop reader to distinguish a genuine runtime override from a stale
    /// session whose persona model was edited. Reset on spawn/restart.
    pub model_overridden: bool,
    /// Normalized agent name from initialize (`agentInfo.name`/`serverInfo.name`).
    pub agent_name: String,
    /// Whether Goose accepted its custom system-prompt method. `None` probes on
    /// the first session; method-not-found is cached as `Some(false)` so legacy
    /// user-message framing is used for this process thereafter.
    pub goose_system_prompt_supported: Option<bool>,
    /// Protocol version reported by the agent in its initialize response.
    pub protocol_version: u32,
}

/// Package name reported by `claude-agent-acp` in its `initialize` response.
/// Any adapter reporting this name supports `_meta.systemPrompt: {append: ...}`
/// on `session/new` — the feature landed in v0.6.0 (Oct 2025), before the
/// `@zed-industries/claude-code-acp` → `@agentclientprotocol/claude-agent-acp`
/// rename, so the new name is a reliable capability gate.
const CLAUDE_AGENT_ACP_NAME: &str = "@agentclientprotocol/claude-agent-acp";

fn has_system_prompt_support(
    protocol_version: u32,
    agent_name: &str,
    goose_system_prompt_supported: Option<bool>,
) -> bool {
    if agent_name == "goose" {
        goose_system_prompt_supported == Some(true)
    } else if agent_name == CLAUDE_AGENT_ACP_NAME {
        true
    } else {
        protocol_version >= 2
    }
}

fn session_new_system_prompt<'a>(
    is_goose: bool,
    protocol_version: u32,
    agent_name: &str,
    prompt: Option<&'a str>,
) -> Option<SystemPromptTransport<'a>> {
    if is_goose || (protocol_version < 2 && agent_name != CLAUDE_AGENT_ACP_NAME) {
        None
    } else if agent_name == CLAUDE_AGENT_ACP_NAME {
        prompt.map(SystemPromptTransport::ClaudeMeta)
    } else {
        prompt.map(SystemPromptTransport::Field)
    }
}

impl OwnedAgent {
    pub(crate) fn has_system_prompt_support(&self) -> bool {
        has_system_prompt_support(
            self.protocol_version,
            &self.agent_name,
            self.goose_system_prompt_supported,
        )
    }
}

/// Pool of agents with take-and-return ownership semantics.
///
/// Agents are either idle (sitting in `agents[i]`) or checked out
/// (running inside a spawned task). The `task_map` tracks in-flight
/// tasks for panic recovery.
pub struct AgentPool {
    agents: Vec<Option<OwnedAgent>>,
    result_tx: mpsc::UnboundedSender<PromptResult>,
    result_rx: mpsc::UnboundedReceiver<PromptResult>,
    pub join_set: JoinSet<()>,
    task_map: HashMap<tokio::task::Id, TaskMeta>,
}

/// Result returned by a completed prompt task.
pub struct PromptResult {
    pub agent: OwnedAgent,
    pub source: PromptSource,
    /// Identifies the completed turn for observer terminal events.
    pub turn_id: String,
    pub outcome: PromptOutcome,
    /// Present on failure in Queue mode, for requeue.
    pub batch: Option<FlushBatch>,
}

/// Whether the prompt came from a channel event or a heartbeat.
#[derive(Debug)]
pub enum PromptSource {
    Channel(Uuid),
    Heartbeat,
}

/// Apply state effects for Race 1, where a control signal arrives just after the
/// prompt completed naturally. The prompt result has already been consumed by
/// `select!`, so the harness must synthesize a successful result while still
/// honoring any load-bearing control signal semantics.
fn apply_completed_before_control_signal(
    state: &mut SessionState,
    source: &PromptSource,
    control_signal: &ControlSignal,
) {
    // Rotate and SwitchModel both invalidate so the next turn creates a fresh
    // session. For SwitchModel the caller has already set `desired_model`, so
    // the fresh session applies the new model on its next creation.
    if matches!(
        control_signal,
        ControlSignal::Rotate | ControlSignal::SwitchModel(_)
    ) {
        state.invalidate(source);
    }
}

/// Control signal for an in-flight channel turn.
///
/// Not `Copy`: `SwitchModel` carries an owned `String`. Callers must clone when
/// a value is needed after a move, or match by reference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlSignal {
    /// Stop the current turn and drop its triggering batch.
    Cancel,
    /// Stop the current turn and requeue its triggering batch for a merged
    /// re-prompt framed as a **supersede**: the new request replaces the old.
    Interrupt,
    /// Stop the current turn and requeue its triggering batch for a merged
    /// re-prompt framed as a **steer**: a message arrived while the agent was
    /// working; it should continue its work and incorporate the message if
    /// relevant, not treat it as a replacement task. This is the default
    /// mid-turn delivery path (see [`MultipleEventHandling::Steer`]).
    Steer,
    /// Stop the current turn and drop its triggering batch. The session is
    /// invalidated just like cancel; the next turn creates a fresh session.
    Rotate,
    /// Switch the agent's model, then requeue the triggering batch so it
    /// re-runs on a fresh session under the new model. The model lands by
    /// setting `OwnedAgent::desired_model` before invalidation; the requeued
    /// turn re-creates the session and re-applies `desired_model`. Runtime-only
    /// — never persisted, gone on restart/respawn.
    SwitchModel(String),
}

/// Goose-native non-cancelling steer request, sent from the main loop to an
/// in-flight prompt task's read loop via a capacity-1 mpsc channel.
///
/// The read loop owns the `AcpClient`'s reader/writer for the duration of the
/// turn, so we cannot drive a steer write from the main thread directly. The
/// main loop carries the steer prompt body (already framed by
/// `queue::native_steer_framing()` + `queue::format_event_block`); the read
/// loop completes `sessionId` (lexical) and `expectedRunId`
/// (`AcpClient::active_run_id` at write time) when it actually emits the
/// JSON-RPC request. The main loop awaits a `SteerAck` on the `ack_tx`
/// oneshot.
///
/// ## Why the read loop fills params, not the main loop
///
/// `expectedRunId` is a *moving target*: the read loop updates
/// `self.active_run_id` as goose emits `session/update` notifications, and
/// the steer is rejected if the supplied id doesn't match the *current* run.
/// A snapshot taken at dispatch (or at mode-gate time) can be stale by the
/// time the read loop actually writes the steer line. Filling params at
/// write time uses the freshest possible run id and is correct-by-
/// construction on the one field whose freshness the protocol checks.
/// `sessionId` is in lexical scope inside the read loop's caller
/// (`session_prompt_blocks_with_idle_timeout`), so no plumbing is required
/// for that — only a function parameter pass-through.
///
/// If `active_run_id` is `None` at write time (no `session/update` seen yet
/// — e.g. agents that never emit run-id metadata), the goose-native method
/// cannot form a valid `expectedRunId`, and the read loop falls back to the
/// cross-adapter `_session/steering` method when the agent advertised
/// `_meta.steering.supported` at `initialize`. That method takes no run id, so
/// no freshness concern applies to it. When neither transport is available the
/// read loop acks [`SteerError::ExpectedRunIdMissing`]. The main loop maps that
/// to the "Err-before-pending" bucket: no withhold/mark was established at
/// `pool::send_steer` time because the request was rejected before any
/// write, so the watcher only needs to release nothing and fall back to the
/// universal `ControlSignal::Steer` cancel+merge path.
pub struct SteerRequest {
    /// Prompt body text blocks. Each entry becomes one `text` content
    /// block in `params.prompt`. Built by the main loop via
    /// `queue::native_steer_framing()` + `queue::format_event_block` so
    /// the wording cannot drift from the cancel+merge fallback path.
    pub prompt_blocks: Vec<String>,
    /// Oneshot for the read loop to report the outcome.
    pub ack_tx: tokio::sync::oneshot::Sender<SteerAck>,
}

/// Why a mid-turn steer failed, on either transport
/// (`_goose/unstable/session/steer` or `_session/steering`).
///
/// String and integer fields are intentionally `Debug`-only — read by
/// `tracing` macros in the main loop's `PoolEvent::SteerAck` arm via
/// `?ack`. The dead-code lint can't see that path because it doesn't
/// trace through `Debug` derives, hence the `#[allow]`.
#[allow(dead_code)]
#[derive(Debug)]
pub enum SteerError {
    /// The agent returned a JSON-RPC error response to the steer request.
    ///
    /// `code` is the JSON-RPC error code:
    /// - `-32601` (`method_not_found`): the agent does not implement the
    ///   steer extension. The main loop should fire the cancel+merge
    ///   fallback so the message still reaches the agent.
    /// - Any other code: the write landed and the agent rejected it at the
    ///   application level (e.g. wrong run id). Release the withheld event
    ///   for normal dispatch; do NOT fire the fallback — the turn is still
    ///   running or just ended.
    AgentError { code: i64, message: String },
    /// Transport-level failure: write error, read EOF, JSON-RPC framing
    /// violation, etc. The string carries the underlying `AcpError`'s display.
    Transport(String),
    /// At steer-write time neither steer transport was available: no
    /// `expectedRunId` (`AcpClient::active_run_id` was `None`, so the
    /// goose-native method could not be formed) and the agent did not
    /// advertise the cross-adapter `_session/steering` extension. The read
    /// loop drops the request without writing anything; the main loop should
    /// release any withheld event and fall back to the universal cancel+merge
    /// `ControlSignal::Steer` path. This is in the same "Err-before-pending"
    /// bucket as `Transport` write failures: no in-process state was
    /// established, so no in-process cleanup is needed.
    ExpectedRunIdMissing,
    /// A `_session/steering` request returned a JSON-RPC *success* whose
    /// `outcome` was not one of the two recognized delivery outcomes
    /// (`injected`, `startedNewTurn`) — including `failed` (codex-acp) and a
    /// missing `outcome` entirely. `outcome` carries what the agent actually
    /// reported, for logs.
    ///
    /// The steer did NOT land, so the main loop must release the withheld
    /// event and fire the cancel+merge fallback — exactly like a write that
    /// never happened. Treating an unrecognized success as delivery would
    /// drop the user's message: codex-acp answers unrecognized extension
    /// methods with a bare `{}` success rather than `-32601`.
    OutcomeRejected { outcome: String },
    /// The read loop never got to dispatch the steer because the prompt
    /// completed first. Delivery state for the underlying message is
    /// unknown after prompt completion — the main loop must treat this as
    /// "release the withheld event so normal dispatch handles it" with no
    /// claims that the agent did or did not incorporate it.
    ///
    /// Returned synchronously by `send_steer` when no task is in flight
    /// for the channel. Never sent through the ack channel — the ack
    /// watcher is only spawned on `send_steer` success.
    PromptCompleted,
}

/// Outcome of a mid-turn steer, sent from the read loop back to the
/// main loop's ack watcher.
#[derive(Debug)]
pub enum SteerAck {
    /// The agent returned a successful response to the steer request.
    /// The main loop must drop the withheld event (`remove_event`) — it
    /// has been delivered via the non-cancelling path.
    Success,
    /// The steer was attempted but failed. Delivery state for the
    /// underlying message is unknown after prompt completion; the main
    /// loop must release the withheld event and fall back to the
    /// universal `Steer` cancel+merge path so the message still reaches
    /// the agent.
    Err(SteerError),
    /// The prompt completed before the read loop selected the steer arm.
    /// Treated as a benign no-op: release the withheld event for normal
    /// dispatch. Do not fire the fallback `Steer` signal — there is no
    /// in-flight turn to signal, and normal dispatch handles delivery.
    PromptCompletedNeutral,
}

/// Whether a turn was cut by the idle clock or the hard wall-clock cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    /// No ACP wire activity for `idle_timeout` seconds.
    Idle,
    /// Turn ran for `max_turn_duration` seconds of wall-clock time.
    /// `recently_active` is true when the agent produced output within
    /// `RECENT_ACTIVITY_WINDOW` of the hard-cap firing.
    Hard { recently_active: bool },
}

/// Outcome of a prompt task.
#[allow(dead_code)]
pub enum PromptOutcome {
    Ok(StopReason),
    Error(AcpError),
    AgentExited,
    Timeout(TimeoutKind),
    /// Intentional cancel via `!cancel` command or interrupt mode.
    /// Agent is healthy — no respawn, no retry penalty.
    Cancelled,
    /// The agent did not stop within `grace` after `session/cancel` was sent
    /// for a control-signal cancellation (steer fallback, interrupt, or
    /// explicit stop). Distinct from [`TimeoutKind::Hard`]: this is a bounded
    /// cleanup deadline, not the turn's configured max-turn wall clock, so it
    /// must never be reported or dead-lettered as a hard-cap breach. The
    /// agent process is uncertain — treated as poisoned and respawned, same
    /// as a hard timeout, but the triggering batch's fate follows the
    /// `CancelReason` on the batch (steer/interrupt requeue, explicit cancel
    /// drops) rather than the hard-cap's unconditional dead-letter.
    CancelDrainTimeout(Duration),
}

/// Per-entry TTL for channel metadata cache entries (~5 minutes).
const CHANNEL_INFO_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Backoff applied to `next_refresh_at` after a failed TTL refresh.
///
/// On failure the entry is served stale; the next consumer will retry after
/// this interval rather than immediately re-driving the full retry sequence.
const CHANNEL_INFO_REFRESH_BACKOFF: std::time::Duration = std::time::Duration::from_secs(60);

/// A cached channel metadata entry with a TTL-based refresh deadline.
#[derive(Clone, Debug)]
struct CachedChannelInfo {
    info: PromptChannelInfo,
    /// Absolute instant after which a background refresh should be triggered.
    /// Consumers continue serving the cached `info` while the refresh runs.
    refresh_after: tokio::time::Instant,
    /// Absolute instant before which background refreshes should be suppressed
    /// (set after a failed refresh to prevent retry storms on degraded relays).
    next_refresh_at: tokio::time::Instant,
}

/// Shared channel-metadata resolver for startup-known and dynamically joined channels.
///
/// Successful lazy lookups are cached for every consumer (author gate, prompt
/// context, canvas, and setup mode). Unknown metadata is never cached as a
/// non-DM: callers can fail closed and a later event retries resolution.
///
/// Each entry carries a ~5-minute TTL (`CHANNEL_INFO_TTL`). On expiry the
/// current value is served immediately (stale-while-revalidate) and a
/// background refresh is spawned. On refresh failure the `next_refresh_at`
/// timestamp is advanced by `CHANNEL_INFO_REFRESH_BACKOFF` to throttle retries
/// on degraded relays — preventing repeated ~6.5-second retry sequences on the
/// DM author gate path.
#[derive(Debug, Clone)]
pub struct ChannelInfoResolver {
    cache: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<Uuid, CachedChannelInfo>>>,
    /// Per-channel in-flight marker: a channel UUID is inserted before spawning a
    /// background refresh and removed when it completes (success or failure).
    /// Prevents an expired entry from spawning duplicate refresh tasks when multiple
    /// callers see the same expired timestamps before the first task finishes.
    in_flight: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<Uuid>>>,
    rest_client: RestClient,
}

impl ChannelInfoResolver {
    pub fn new(
        startup: std::collections::HashMap<Uuid, ChannelInfo>,
        rest_client: RestClient,
    ) -> Self {
        let now = tokio::time::Instant::now();
        let cache = startup
            .into_iter()
            .filter_map(|(id, info)| {
                (info.channel_type != "unknown").then_some((
                    id,
                    CachedChannelInfo {
                        info: PromptChannelInfo {
                            name: info.name,
                            channel_type: info.channel_type,
                            description: info.description,
                        },
                        refresh_after: now + CHANNEL_INFO_TTL,
                        next_refresh_at: now,
                    },
                ))
            })
            .collect();
        Self {
            cache: std::sync::Arc::new(std::sync::RwLock::new(cache)),
            in_flight: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            rest_client,
        }
    }

    pub async fn resolve(&self, channel_id: Uuid) -> Option<PromptChannelInfo> {
        let now = tokio::time::Instant::now();

        // Fast path: return cached entry; spawn a background refresh if expired.
        if let Some(cached) = self
            .cache
            .read()
            .ok()
            .and_then(|c| c.get(&channel_id).cloned())
        {
            if now >= cached.refresh_after && now >= cached.next_refresh_at {
                // Stale — claim the in-flight slot; if already claimed, skip spawning.
                let claimed = self
                    .in_flight
                    .lock()
                    .map(|mut s| s.insert(channel_id))
                    .unwrap_or(false);
                if claimed {
                    let resolver = self.clone();
                    tokio::spawn(async move {
                        resolver.background_refresh(channel_id).await;
                    });
                }
            }
            return Some(cached.info);
        }

        // Slow path: uncached — fetch synchronously (one logical sequence = two HTTP
        // attempts per fetch_with_retry, exactly as before the TTL addition).
        let info = fetch_channel_info(channel_id, &self.rest_client).await?;
        let now = tokio::time::Instant::now();
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(
                channel_id,
                CachedChannelInfo {
                    info: info.clone(),
                    refresh_after: now + CHANNEL_INFO_TTL,
                    next_refresh_at: now,
                },
            );
        }
        Some(info)
    }

    /// Background TTL refresh: fetch fresh metadata and update the cache.
    ///
    /// On failure: advance `next_refresh_at` by `CHANNEL_INFO_REFRESH_BACKOFF`
    /// so the next consumer does not immediately re-trigger the retry sequence.
    /// On success: reset the TTL window.
    ///
    /// Clears the per-channel `in_flight` marker on return, allowing the next
    /// expiry cycle to spawn a fresh refresh.
    async fn background_refresh(&self, channel_id: Uuid) {
        match fetch_channel_info(channel_id, &self.rest_client).await {
            Some(info) => {
                let now = tokio::time::Instant::now();
                if let Ok(mut cache) = self.cache.write() {
                    cache.insert(
                        channel_id,
                        CachedChannelInfo {
                            info,
                            refresh_after: now + CHANNEL_INFO_TTL,
                            next_refresh_at: now,
                        },
                    );
                }
            }
            None => {
                // Fetch failed — serve stale; suppress retries for backoff duration.
                let now = tokio::time::Instant::now();
                if let Ok(mut cache) = self.cache.write() {
                    if let Some(entry) = cache.get_mut(&channel_id) {
                        entry.next_refresh_at = now + CHANNEL_INFO_REFRESH_BACKOFF;
                    }
                }
            }
        }
        // Release the in-flight slot so the next expiry cycle can refresh again.
        if let Ok(mut s) = self.in_flight.lock() {
            s.remove(&channel_id);
        }
    }
}

/// Immutable config subset shared (via `Arc`) by all spawned prompt tasks.
///
/// Built once from `Config` at startup. Avoids cloning the full config
/// into every task.
pub struct PromptContext {
    pub mcp_servers: Vec<McpServer>,
    pub initial_message: Option<String>,
    pub idle_timeout: Duration,
    pub max_turn_duration: Duration,
    /// Interval between per-turn `turn_liveness` observer pings. `Duration::ZERO`
    /// disables emission. This is the desktop crash-backstop signal — distinct
    /// from `heartbeat_prompt` (agent self-prompting).
    pub turn_liveness_interval: Duration,
    pub dedup_mode: DedupMode,
    pub system_prompt: Option<String>,
    /// Sanitized title for each new ACP session, sent as `_meta.sessionTitle`
    /// on `session/new`. Never part of the prompt.
    pub session_title: Option<String>,
    pub team_instructions: Option<String>,
    pub heartbeat_prompt: Option<String>,
    /// Base prompt content, or `None` if `--no-base-prompt` was passed.
    ///
    /// `'static` because `PromptContext` is `Arc`-shared across async tasks.
    /// Content from `--base-prompt-file` is promoted via `Box::leak` in `main.rs`
    /// after validated file read in `Config::from_cli()`. The compiled-in default
    /// (`include_str!`) is inherently `'static`.
    pub base_prompt: Option<&'static str>,
    pub cwd: String,
    /// REST client for pre-prompt context fetches (thread/DM history).
    pub rest_client: RestClient,
    /// Shared channel metadata for startup-known and dynamically joined channels.
    pub channel_info: ChannelInfoResolver,
    /// Max messages to include in thread/DM context. 0 = disabled.
    pub context_message_limit: u32,
    /// Max turns per session before proactive rotation. 0 = disabled.
    pub max_turns_per_session: u32,
    /// Permission mode to apply after session creation. `Default` = skip.
    pub permission_mode: PermissionMode,
    /// Agent identity — used to derive the NIP-AE conversation key at
    /// session creation for core injection.
    pub agent_keys: nostr::Keys,
    /// Owner pubkey (hex), if resolved at startup. When unset, NIP-AE core
    /// injection is skipped entirely (no owner = no `(agent, owner)` pair).
    pub agent_owner_pubkey: Option<nostr::PublicKey>,
    /// Whether NIP-AE agent core memory injection is enabled. When false,
    /// the per-session core engram fetch is skipped and `core_sections`
    /// remains empty for every channel, so `format_prompt` renders no
    /// `[Agent Memory — core]` section. On by default; disabled via
    /// `--no-memory` / `BUZZ_ACP_NO_MEMORY`.
    pub memory_enabled: bool,
    /// Harness identity string for NIP-AM `harness` field. Derived from the
    /// configured `agent_command` at startup (e.g. `"goose"`, `"buzz-agent"`).
    pub harness_name: String,
    /// Relay URL this harness is connected to. Rides in observer payloads that
    /// the desktop keys per (agent, relay) pair, e.g. `session_config_captured`,
    /// mirroring the `managed_agent_runtime_lifecycle` frames.
    pub relay_url: String,
    /// Process-lifetime per-channel canvas revision cache.
    ///
    /// Keyed by channel UUID; stores the last-known canvas pointer or confirmed
    /// absence. Independent of `SessionState` — survives ACP session rotation.
    pub canvas_cache: CanvasRevisionCache,
}

impl AgentPool {
    /// Create a pool from pre-indexed slots (may contain None for failed startups).
    ///
    /// Slot positions are preserved so that `agent.index` always matches the
    /// index into `self.agents`. Use this instead of `new()` when the startup
    /// loop skips failed agents — `new()` would pack agents densely and break
    /// the index invariant.
    pub fn from_slots(slots: Vec<Option<OwnedAgent>>) -> Self {
        let (result_tx, result_rx) = mpsc::unbounded_channel();
        Self {
            agents: slots,
            result_tx,
            result_rx,
            join_set: JoinSet::new(),
            task_map: HashMap::new(),
        }
    }

    /// Try to claim an idle agent for the given channel (or heartbeat if `None`).
    ///
    /// Pass 1: prefer an agent that already has a session for `channel_id`.
    /// Pass 2: any idle agent.
    ///
    /// Returns `None` if all agents are checked out.
    pub fn try_claim(&mut self, channel_id: Option<Uuid>) -> Option<OwnedAgent> {
        // Pass 1: prefer agent with existing session for this channel.
        if let Some(cid) = channel_id {
            let idx = self.agents.iter().position(|slot| {
                slot.as_ref()
                    .map(|a| a.state.sessions.contains_key(&cid))
                    .unwrap_or(false)
            });
            if let Some(i) = idx {
                return self.agents[i].take();
            }
        }

        // Pass 2: first idle agent.
        let idx = self.agents.iter().position(|slot| slot.is_some());
        idx.map(|i| self.agents[i].take().unwrap())
    }

    /// Return an agent to its slot after a task completes.
    pub fn return_agent(&mut self, agent: OwnedAgent) {
        let idx = agent.index;
        if self.agents[idx].is_some() {
            // This is a bug: two tasks returned the same agent index. Log it
            // loudly so it shows up in production logs, then overwrite — the
            // alternative (dropping the incoming agent) would permanently leak
            // the slot.
            tracing::error!(
                idx,
                "BUG: return_agent called for slot {idx} which is already occupied — overwriting"
            );
        }
        self.agents[idx] = Some(agent);
    }

    /// Whether any agent is currently idle (sitting in its slot).
    pub fn any_idle(&self) -> bool {
        self.agents.iter().any(|slot| slot.is_some())
    }

    /// Whether any idle agent already has a session for `channel_id`.
    /// Used to compute `affinity_hit` before calling `try_claim`.
    pub fn has_session_for(&self, channel_id: Uuid) -> bool {
        self.agents.iter().any(|slot| {
            slot.as_ref()
                .map(|a| a.state.sessions.contains_key(&channel_id))
                .unwrap_or(false)
        })
    }

    /// Count of agents that are alive: idle OR checked out (have a task_map entry).
    ///
    /// Used to detect when all agents have exited so the caller can respawn.
    pub fn live_count(&self) -> usize {
        let idle = self.agents.iter().filter(|s| s.is_some()).count();
        let checked_out = self.task_map.len();
        idle + checked_out
    }

    pub fn task_map(&self) -> &HashMap<tokio::task::Id, TaskMeta> {
        &self.task_map
    }

    pub fn task_map_mut(&mut self) -> &mut HashMap<tokio::task::Id, TaskMeta> {
        &mut self.task_map
    }

    /// Try to send a goose-native steer request to the in-flight task for
    /// `channel_id`.
    ///
    /// Returns `Ok(())` if the request was accepted by the read loop's
    /// receiver (capacity-1 mpsc; one slot is the single in-flight steer
    /// write). Returns `Err(SteerError::Transport(_))` on `Full`/`Closed`
    /// (already-in-flight write, or read loop torn down). Callers must
    /// fall back to the universal `ControlSignal::Steer` cancel+merge path
    /// on `Err`.
    ///
    /// This does **not** spawn the ack watcher — the caller owns the
    /// oneshot `ack_tx` inside `SteerRequest` and is responsible for
    /// awaiting it and applying the locked Success / Err / PromptCompletedNeutral
    /// semantics. Caller is also responsible for the synchronous
    /// `queue.mark_native_steer_pending(...)` *before* spawning the
    /// watcher, to close the result-vs-ack race.
    ///
    /// Returns `Err(SteerError::PromptCompleted)` if no task is in flight
    /// for `channel_id` (the prompt completed between the mode-gate check
    /// and this call, or the channel was never in flight). This is
    /// semantically a soft no-op — the caller should release any withheld
    /// event and let normal dispatch handle delivery.
    pub fn send_steer(
        &mut self,
        channel_id: Uuid,
        request: SteerRequest,
    ) -> Result<(), SteerError> {
        let meta = self
            .task_map
            .values_mut()
            .find(|m| m.channel_id == Some(channel_id))
            .ok_or(SteerError::PromptCompleted)?;
        let tx = meta
            .steer_tx
            .as_ref()
            .ok_or_else(|| SteerError::Transport("steer_tx not installed".into()))?;
        tx.try_send(request)
            .map_err(|e| SteerError::Transport(e.to_string()))
    }

    pub fn result_tx(&self) -> mpsc::UnboundedSender<PromptResult> {
        self.result_tx.clone()
    }

    /// Split-borrow: returns mutable refs to `result_rx` and `join_set`
    /// simultaneously. This lets callers poll both in a single `select!`
    /// without a double-borrow error on `&mut AgentPool`.
    pub fn rx_and_join_set(
        &mut self,
    ) -> (&mut mpsc::UnboundedReceiver<PromptResult>, &mut JoinSet<()>) {
        (&mut self.result_rx, &mut self.join_set)
    }

    /// Non-blocking drain of the result channel. Used during shutdown to
    /// collect agents that completed while join_set was being drained.
    pub fn result_rx_try_recv(&mut self) -> Result<PromptResult, mpsc::error::TryRecvError> {
        self.result_rx.try_recv()
    }

    /// Check whether a slot is alive: either idle in the pool or checked out
    /// for an in-flight task. Returns `false` only when the slot is truly
    /// empty and available for refill.
    pub fn slot_alive(&self, index: usize) -> bool {
        let idle = self.agents.get(index).is_some_and(|s| s.is_some());
        if idle {
            return true;
        }
        // Check if the agent is checked out (in-flight on a task).
        self.task_map.values().any(|m| m.agent_index == index)
    }

    pub fn agents_mut(&mut self) -> &mut Vec<Option<OwnedAgent>> {
        &mut self.agents
    }

    /// Remove the session for `channel_id` from all idle agents.
    ///
    /// Called when the agent is removed from a channel — stale sessions
    /// should not be reused. Checked-out agents (in-flight) are not
    /// modified; their sessions will fail naturally on the next prompt
    /// if the relay rejects the request.
    ///
    /// Returns the number of sessions invalidated.
    pub fn invalidate_channel_sessions(&mut self, channel_id: Uuid) -> usize {
        let mut count = 0;
        for slot in &mut self.agents {
            if let Some(agent) = slot.as_mut() {
                if agent.state.invalidate_channel(&channel_id) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Idle-path model switch: set `desired_model` on the idle agent for
    /// `channel_id` and invalidate its session so the next turn re-creates the
    /// session under the new model.
    ///
    /// Pre-cancel guard: the desired model is validated against the agent's
    /// cached catalog *before* the session is invalidated, so an unsupported
    /// pick is rejected without disturbing the existing session.
    ///
    /// Returns [`IdleSwitchResult`] describing what happened. The model does not
    /// take effect — and the panel does not reflect it — until the agent next
    /// runs a turn (no live session exists to re-emit `session_config_captured`
    /// from an idle agent). This lag is intentional: faking the emit would
    /// surface an override the session has not actually applied.
    pub fn switch_idle_agent_model(
        &mut self,
        channel_id: Uuid,
        model_id: &str,
    ) -> IdleSwitchResult {
        let Some(agent) = self
            .agents
            .iter_mut()
            .flatten()
            .find(|a| a.state.sessions.contains_key(&channel_id))
        else {
            return IdleSwitchResult::NoIdleAgent;
        };

        // Pre-cancel guard against the cached catalog. None = catalog not yet
        // populated (no session ever created); defer validation to apply time.
        if let Some(caps) = agent.model_capabilities.as_ref() {
            if !model_in_catalog(
                &caps.config_options_raw,
                caps.available_models_raw.as_ref(),
                model_id,
            ) {
                return IdleSwitchResult::UnsupportedModel;
            }
        }

        agent.desired_model = Some(model_id.to_string());
        agent.model_overridden = true;
        agent.state.invalidate_channel(&channel_id);
        IdleSwitchResult::Switched
    }
}

/// Outcome of [`AgentPool::switch_idle_agent_model`].
#[derive(Debug, PartialEq, Eq)]
pub enum IdleSwitchResult {
    /// `desired_model` set and the channel session invalidated.
    Switched,
    /// Desired model is not in the agent's cached catalog — pick rejected,
    /// session untouched.
    UnsupportedModel,
    /// No idle agent available (all checked out / none spawned).
    NoIdleAgent,
}

/// Timeout for a single pre-prompt context fetch attempt (thread/DM history).
/// Each call gets this budget; with one retry the total worst-case is
/// 2 × CONTEXT_FETCH_TIMEOUT + CONTEXT_FETCH_RETRY_DELAY ≈ 6.5 s.
const CONTEXT_FETCH_TIMEOUT: Duration = Duration::from_millis(3_000);

/// Short, single-attempt timeout for best-effort exact truncated-thread counts.
const CONTEXT_COUNT_TIMEOUT: Duration = Duration::from_millis(500);

/// Delay between the first failed context fetch and the single retry.
const CONTEXT_FETCH_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Timeout for model-switch requests (`session/set_config_option`, `session/set_model`).
const MODEL_SWITCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Bounded grace window for the post-cancel drain after a control-signal
/// cancellation (steer fallback, interrupt, or explicit stop). This is a
/// cleanup deadline, not the turn's configured max-turn wall clock — see
/// [`AcpClient::cancel_with_cleanup_grace`] and
/// [`classify_control_cancel_failure`].
const CONTROL_CANCEL_GRACE: Duration = Duration::from_secs(5);

/// Timeout for permission-mode requests (`session/set_config_option` with `configId: "mode"`).
const PERMISSION_MODE_TIMEOUT: Duration = Duration::from_secs(5);

/// Placeholder [`fetch_channel_info`] substitutes when a channel's metadata
/// event carries no `name` tag. Not a real channel name — consumers that need
/// an identifying name must treat it as absent.
const UNKNOWN_CHANNEL_NAME: &str = "unknown";

/// Create a new ACP session via `session_new_full()`, populate model capabilities
/// on the agent (first session only), and apply `desired_model` if set.
///
/// On error from `session_new_full()`, returns the `AcpError` — caller handles
/// error reporting. Model-switch failures are logged and gracefully ignored
/// (the agent proceeds with its default model).
async fn create_session_and_apply_model(
    agent: &mut OwnedAgent,
    ctx: &PromptContext,
    agent_core: Option<&str>,
    channel_name: Option<&str>,
    channel_id: Option<Uuid>,
    channel_type: Option<&str>,
) -> Result<String, AcpError> {
    // Build base_prompt + system_prompt + agent core into a single prompt.
    // Standard protocol-v2 agents receive it in `session/new`;
    // Goose receives it through the custom request below. Legacy agents receive
    // the same content as user-message sections via `format_prompt`. Core carries
    // its own `[Agent Memory — core]` header and is appended with a blank-line
    // separator.
    let is_goose = agent.agent_name == "goose";
    let combined_system_prompt = with_core(
        with_team(
            framed_system_prompt(&ctx.cwd, ctx.base_prompt, ctx.system_prompt.as_deref()),
            ctx.team_instructions.as_deref(),
        ),
        agent_core,
    );

    let session_title = ctx
        .session_title
        .as_deref()
        .map(|agent_name| compose_session_title(agent_name, channel_name));
    let mcp_servers = mcp_servers_with_git_origin(
        &ctx.mcp_servers,
        channel_id,
        channel_type,
        ctx.session_title.as_deref(),
    );

    let resp = agent
        .acp
        .session_new_full(
            &ctx.cwd,
            mcp_servers,
            session_new_system_prompt(
                is_goose,
                agent.protocol_version,
                &agent.agent_name,
                combined_system_prompt.as_deref(),
            ),
            session_title.as_deref(),
        )
        .await?;

    if is_goose && agent.goose_system_prompt_supported != Some(false) {
        if let Some(prompt) = combined_system_prompt.as_deref() {
            match agent
                .acp
                .session_set_goose_system_prompt(&resp.session_id, prompt)
                .await
            {
                Ok(_) => agent.goose_system_prompt_supported = Some(true),
                Err(AcpError::AgentError { code: -32601, .. }) => {
                    agent.goose_system_prompt_supported = Some(false);
                    tracing::warn!(
                        target: "pool::session",
                        "Goose does not support its system-prompt extension; using user-message framing"
                    );
                }
                Err(error) => return Err(error),
            }
        }
    }

    // Populate model capabilities on first session creation.
    if agent.model_capabilities.is_none() {
        agent.model_capabilities = Some(AgentModelCapabilities {
            config_options_raw: extract_model_config_options(&resp.raw),
            available_models_raw: extract_model_state(&resp.raw),
        });
    }

    // Apply desired_model if set, matching against the fresh session/new response.
    // Track whether the switch succeeded so session_config_captured reflects
    // the post-switch state (not the pre-switch desired state).
    let switch_succeeded = if let Some(ref desired) = agent.desired_model {
        match resolve_model_switch_method(&resp.raw, desired) {
            Some(method) => {
                apply_model_switch(&mut agent.acp, &resp.session_id, desired, &method).await?;
                true
            }
            None => {
                tracing::warn!(
                    target: "pool::model",
                    "desired model {desired} not found in agent's available models — proceeding with agent default"
                );
                // Surface the miss so the desktop ModelPicker can reject a live
                // pick rather than silently no-op. On the busy path the turn has
                // already been cancelled+requeued by the time we get here, so the
                // turn restarts on the unchanged model and the user is told no.
                agent.acp.observe(
                    "control_result",
                    serde_json::json!({
                        "type": "switch_model",
                        "status": "unsupported_model",
                        "modelId": desired,
                    }),
                );
                false
            }
        }
    } else {
        false
    };

    // Emit session config for desktop consumption (config bridge tier 1b).
    // Emitted AFTER desired_model resolution so the desktop caches the
    // post-switch state. modelOverridden reflects whether the switch actually
    // applied — false on the unsupported arm so the panel doesn't show a
    // stale override badge.
    agent.acp.observe(
        "session_config_captured",
        serde_json::json!({
            "configOptions": resp.raw.get("configOptions").cloned().unwrap_or(serde_json::Value::Null),
            "modes": resp.raw.get("modes").cloned().unwrap_or(serde_json::Value::Null),
            "models": resp.raw.get("models").cloned().unwrap_or(serde_json::Value::Null),
            "modelOverridden": agent.model_overridden && switch_succeeded,
            // Pair identity for the desktop session-config cache, which is
            // keyed by (agent, relay) like the lifecycle frames.
            "relayUrl": ctx.relay_url,
        }),
    );

    // Apply permission mode if not the agent's built-in default AND the agent
    // advertises the requested mode in session/new. Agents that don't support
    // the mode (e.g., goose crashes on unrecognized set_config_option values)
    // are safely skipped — the harness rejects interactive permission requests.
    if !ctx.permission_mode.is_default()
        && agent_supports_mode(&resp.raw, ctx.permission_mode.as_wire_str())
    {
        apply_permission_mode(&mut agent.acp, &resp.session_id, &ctx.permission_mode).await?;
    }

    Ok(resp.session_id)
}

fn mcp_servers_with_git_origin(
    servers: &[McpServer],
    channel_id: Option<Uuid>,
    channel_type: Option<&str>,
    agent_name: Option<&str>,
) -> Vec<McpServer> {
    let mut servers = servers.to_vec();
    let origin = match (channel_id, channel_type) {
        (Some(channel_id), Some("stream")) => Some(EnvVar {
            name: "BUZZ_GIT_ORIGIN_CHANNEL_ID".into(),
            value: channel_id.to_string(),
        }),
        (Some(_), _) => agent_name
            .filter(|name| !name.trim().is_empty())
            .map(|name| EnvVar {
                name: "BUZZ_GIT_ORIGIN_AGENT_NAME".into(),
                value: name.trim().to_string(),
            }),
        (None, _) => None,
    };
    if let Some(origin) = origin {
        for server in &mut servers {
            server.env.push(origin.clone());
        }
    }
    servers
}

/// Send the appropriate ACP model-switch request with a timeout.
///
/// On timeout or error, logs a warning and returns — the caller proceeds
/// with the agent's default model. This is intentionally non-fatal: a stale
/// response from a timed-out request is safely ignored by `read_until_response`
/// (non-matching JSON-RPC IDs are skipped).
async fn apply_model_switch(
    acp: &mut AcpClient,
    session_id: &str,
    desired: &str,
    method: &ModelSwitchMethod,
) -> Result<(), AcpError> {
    let method_label = match method {
        ModelSwitchMethod::ConfigOption { config_id, .. } => {
            format!("configOption (configId={config_id})")
        }
        ModelSwitchMethod::SetModel { .. } => "set_model".to_string(),
    };

    let result = tokio::time::timeout(MODEL_SWITCH_TIMEOUT, async {
        match method {
            ModelSwitchMethod::ConfigOption {
                config_id,
                option_value,
            } => {
                acp.session_set_config_option(session_id, config_id, option_value)
                    .await
            }
            ModelSwitchMethod::SetModel { model_id } => {
                acp.session_set_model(session_id, model_id).await
            }
        }
    })
    .await;

    match result {
        Ok(Ok(_)) => {
            tracing::info!(
                target: "pool::model",
                "applied model {desired} via {method_label} on session {session_id}"
            );
        }
        // Transport-class errors may have corrupted the stdio stream — propagate
        // so the caller can respawn the agent instead of reusing a poisoned one.
        Ok(Err(e @ AcpError::Io(_)))
        | Ok(Err(e @ AcpError::WriteTimeout(_)))
        | Ok(Err(e @ AcpError::Timeout(_)))
        | Ok(Err(e @ AcpError::Protocol(_)))
        | Ok(Err(e @ AcpError::AgentExited)) => {
            tracing::error!(
                target: "pool::model",
                "fatal error setting model {desired} via {method_label}: {e}"
            );
            return Err(e);
        }
        // Application-level errors (Json, etc.) — agent is fine, just uses default model.
        Ok(Err(e)) => {
            tracing::warn!(
                target: "pool::model",
                "failed to set model {desired} via {method_label}: {e} — proceeding with agent default"
            );
        }
        Err(_) => {
            // Outer timeout fired — the inner send_request may have left the
            // stream in an unknown state. Treat as transport error.
            tracing::error!(
                target: "pool::model",
                "model set via {method_label} timed out ({MODEL_SWITCH_TIMEOUT:?}) — treating as fatal"
            );
            return Err(AcpError::Timeout(MODEL_SWITCH_TIMEOUT));
        }
    }
    Ok(())
}

/// Check whether the agent's `session/new` response advertises a given mode ID
/// in `result.modes.availableModes[].id`. Returns `false` if the modes
/// field is absent or the mode isn't listed.
fn agent_supports_mode(session_new_result: &serde_json::Value, mode_wire: &str) -> bool {
    session_new_result
        .get("modes")
        .and_then(|m| m.get("availableModes"))
        .and_then(|a| a.as_array())
        .map(|modes| {
            modes
                .iter()
                .any(|m| m.get("id").and_then(|v| v.as_str()) == Some(mode_wire))
        })
        .unwrap_or(false)
}

/// Set the session permission mode via `session/set_config_option`.
///
/// Non-fatal for most errors: logs and proceeds. The agent falls back to its
/// default mode, and any interactive permission request is rejected by
/// `handle_permission_request`.
///
/// **Fatal exception:** if the agent process exits (e.g., goose crashes on
/// unrecognized methods), returns `Err(AgentExited)` so the caller can respawn.
async fn apply_permission_mode(
    acp: &mut AcpClient,
    session_id: &str,
    mode: &PermissionMode,
) -> Result<(), AcpError> {
    let wire = mode.as_wire_str();
    let result = tokio::time::timeout(PERMISSION_MODE_TIMEOUT, async {
        acp.session_set_config_option(session_id, "mode", wire)
            .await
    })
    .await;

    match result {
        Ok(Ok(_)) => {
            tracing::info!(
                target: "pool::permission",
                "applied permission mode {wire:?} on session {session_id}"
            );
        }
        // Transport-class errors may have corrupted the stdio stream — propagate
        // so the caller can respawn the agent.
        Ok(Err(e @ AcpError::Io(_)))
        | Ok(Err(e @ AcpError::WriteTimeout(_)))
        | Ok(Err(e @ AcpError::Timeout(_)))
        | Ok(Err(e @ AcpError::Protocol(_)))
        | Ok(Err(e @ AcpError::AgentExited)) => {
            tracing::error!(
                target: "pool::permission",
                "fatal error setting permission mode {wire:?}: {e}"
            );
            return Err(e);
        }
        // Application-level errors — agent is fine, just uses default permission mode.
        Ok(Err(e)) => {
            tracing::warn!(
                target: "pool::permission",
                "failed to set permission mode {wire:?}: {e} — falling back to per-tool rejection"
            );
        }
        Err(_) => {
            // Outer timeout fired — stream may be in unknown state.
            tracing::error!(
                target: "pool::permission",
                "permission mode set timed out ({PERMISSION_MODE_TIMEOUT:?}) — treating as fatal"
            );
            return Err(AcpError::Timeout(PERMISSION_MODE_TIMEOUT));
        }
    }
    Ok(())
}

/// Prepend the `[Base]` section to a user-message body for legacy agents.
///
/// Legacy agents (`protocol_version < 2`) don't receive `base_prompt` via the
/// system role in `session/new`, so it must ride along in the user message.
/// Agents with `protocol_version >= 2`, or any agent without a `base_prompt`,
/// get `body` unchanged. The gate lives here so the heartbeat and
/// initial-message dispatch paths can't drift apart again.
pub(crate) fn prepend_base_for_legacy(
    protocol_version: u32,
    base_prompt: Option<&str>,
    body: &str,
) -> String {
    match base_prompt {
        Some(bp) if protocol_version < 2 => {
            format!("{}\n\n{body}", crate::queue::base_section(bp))
        }
        _ => body.to_string(),
    }
}

/// Frame the `session/new` `systemPrompt` so each present prompt carries its own
/// header, keeping the base/persona boundary recoverable downstream.
///
/// The header framing matches the legacy per-turn path (`queue::base_section`
/// for `[Base]`, `[System]\n{...}` for the persona) so the desktop observer can
/// split the combined value into labeled sub-sections. Each prompt is wrapped
/// only when present, so a persona-only agent yields `[System]\n{persona}`
/// rather than an unlabeled blob that would be mislabeled as `[Base]`.
///
/// Prepends a `[Workspace]` section naming the agent's absolute working
/// directory. The base prompt describes the workspace layout but never its
/// absolute root, so without this anchor a model fills the gap by searching
/// `$HOME` (triggering macOS TCC prompts) or by inventing its own workspace
/// directory. The line is emitted only when a real base prompt is present and
/// `cwd` is an absolute path other than the `/` fallback — naming `/` as the
/// workspace would itself invite a `$HOME`-wide scan.
fn framed_system_prompt(
    cwd: &str,
    base_prompt: Option<&str>,
    system_prompt: Option<&str>,
) -> Option<String> {
    let body = match (base_prompt, system_prompt) {
        (Some(bp), Some(sp)) => Some(format!(
            "{}\n\n[System]\n{sp}",
            crate::queue::base_section(bp)
        )),
        (Some(bp), None) => Some(crate::queue::base_section(bp)),
        (None, Some(sp)) => Some(format!("[System]\n{sp}")),
        (None, None) => None,
    }?;
    // Anchor the workspace only when a base prompt is present — the workspace
    // section grounds the base prompt's layout description, so it is meaningless
    // for a persona-only (`[System]`-only) agent that never received that layout.
    match (base_prompt, workspace_section(cwd)) {
        (Some(_), Some(workspace)) => Some(format!("{workspace}\n\n{body}")),
        _ => Some(body),
    }
}

/// Render the `[Workspace]` grounding section, or `None` when `cwd` is unusable.
///
/// Skips relative paths and the `/` fallback (`std::env::current_dir()` resolves
/// to `/` on failure): a `/`-rooted workspace line would actively encourage the
/// `$HOME`-wide scan this section exists to prevent.
fn workspace_section(cwd: &str) -> Option<String> {
    if cwd != "/" && cwd.starts_with('/') {
        Some(format!(
            "[Workspace]\nYour absolute working directory is `{cwd}`. All workspace \
             files — `AGENTS.md`, `RESEARCH/`, `PLANS/`, `GUIDES/`, `WORK_LOGS/`, \
             `OUTBOX/` — and any repositories you clone (under `{cwd}/REPOS/`) live \
             here. This is where you already are; do not search `$HOME` or other \
             directories for them."
        ))
    } else {
        None
    }
}

/// Append the team-owned instruction section after `[System]` and before core memory.
fn with_team(prompt: Option<String>, instructions: Option<&str>) -> Option<String> {
    let instructions = instructions
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (prompt, instructions) {
        (Some(prompt), Some(instructions)) => {
            Some(format!("{prompt}\n\n[Team Instructions]\n{instructions}"))
        }
        (None, Some(instructions)) => Some(format!("[Team Instructions]\n{instructions}")),
        (Some(prompt), None) => Some(prompt),
        (None, None) => None,
    }
}

/// Append the agent's core memory section onto the framed system prompt.
///
/// Core already carries its own `[Agent Memory — core]` header from
/// `engram_fetch::build_core_section`, so it is joined with a blank-line
/// separator and never re-labeled. Either side may be absent.
fn with_core(framed: Option<String>, core: Option<&str>) -> Option<String> {
    match (framed, core) {
        (Some(framed), Some(core)) => Some(format!("{framed}\n\n{core}")),
        (Some(framed), None) => Some(framed),
        (None, Some(core)) => Some(core.to_string()),
        (None, None) => None,
    }
}

/// Return `agent` to the pool via `result_tx`, clearing any steer receiver first.
///
/// Every path that returns an `OwnedAgent` to the pool via `PromptResult` goes
/// through this function. Panic/abort paths do not — and don't need to, since a
/// panicked task's agent is never sent back via `PromptResult`.
///
/// Clearing `steer_rx` here — rather than per-arm — makes the `install_steer_rx`
/// invariant (`steer_rx.is_none()` at dispatch) structurally unviolatable: a receiver
/// installed for a turn that ends before the read loop's `take()` (e.g. session-create
/// error) is always dropped before the agent re-enters the pool, so the next dispatch
/// can never trigger the assert.
///
/// On the happy path the read loop has already called `take()`, so this is a no-op.
fn send_prompt_result(
    result_tx: &mpsc::UnboundedSender<PromptResult>,
    turn_id: &str,
    mut agent: OwnedAgent,
    source: PromptSource,
    outcome: PromptOutcome,
    batch: Option<FlushBatch>,
) {
    agent.acp.clear_steer_rx();
    let _ = result_tx.send(PromptResult {
        agent,
        source,
        turn_id: turn_id.to_owned(),
        outcome,
        batch,
    });
}

/// Core async function spawned for each prompt.
///
/// Lifecycle:
/// 1. Resolve or create a session (channel or heartbeat).
/// 2. Send `initial_message` on new channel sessions (if configured).
/// 3. Fetch conversation context if needed (thread reply or DM).
/// 4. Build the prompt text from batch + context.
/// 5. Send the actual prompt with turn timeout.
/// 6. Handle all error paths, always returning the agent via `result_tx`.
///
/// The agent is ALWAYS returned — even on panic the `JoinSet` detects the
/// abort and the caller uses `task_map` to recover the agent index.
pub async fn run_prompt_task(
    mut agent: OwnedAgent,
    batch: Option<FlushBatch>,
    prompt_text: Option<String>,
    ctx: Arc<PromptContext>,
    result_tx: mpsc::UnboundedSender<PromptResult>,
    control_rx: Option<tokio::sync::oneshot::Receiver<ControlSignal>>,
    turn_id: String,
) {
    // Is this a channel prompt or a heartbeat?
    let source = match &batch {
        Some(b) => PromptSource::Channel(b.channel_id),
        None => PromptSource::Heartbeat,
    };
    let observer_channel_id = match &source {
        PromptSource::Channel(channel_id) => Some(*channel_id),
        PromptSource::Heartbeat => None,
    };
    let turn_started_at = chrono::Utc::now().to_rfc3339();
    agent.acp.set_observer_context(observer::context_for_turn(
        observer_channel_id,
        None,
        turn_id.clone(),
        turn_started_at.clone(),
    ));
    let triggering_event_ids: Vec<String> = batch
        .as_ref()
        .map(|b| b.events.iter().map(|be| be.event.id.to_hex()).collect())
        .unwrap_or_default();
    agent.acp.observe(
        "turn_started",
        serde_json::json!({
            "source": match &source {
                PromptSource::Channel(_) => "channel",
                PromptSource::Heartbeat => "heartbeat",
            },
            "triggeringEventIds": triggering_event_ids,
        }),
    );

    // Emits `turn_completed` on any exit path. Captures observer handle and
    // metadata now, before the agent is moved into PromptResult. It must be
    // declared before `liveness_guard`: Rust drops locals in reverse order, so
    // liveness is aborted before completion makes the turn terminal.
    let _turn_guard = TurnCompletionGuard::new(
        agent.acp.observer_handle(),
        agent.acp.observer_agent_index(),
        observer_channel_id,
        turn_id.clone(),
    );

    // Start liveness with `turn_started`, not the final session/prompt call:
    // session creation, context fetches, and an initial message can themselves
    // take longer than the desktop's bounded prune pause. This future is pinned
    // for the whole task and dropped with the turn on every exit path.
    //
    // `liveness_state` is shared with `LivenessGuard`: see its docs for why a
    // bare `abort()` alone cannot prevent a `turn_liveness` frame emitted after
    // `turn_completed`. Once the session resolves below, `set_session_id`
    // updates the same shared state so later ticks stop carrying `None`.
    let liveness_state = Arc::new(Mutex::new(LivenessState {
        closed: false,
        session_id: None,
    }));
    let liveness = run_turn_liveness(
        agent.acp.observer_handle(),
        agent.acp.observer_agent_index(),
        observer::context_for_turn(
            observer_channel_id,
            None,
            turn_id.clone(),
            turn_started_at.clone(),
        ),
        ctx.turn_liveness_interval,
        Arc::clone(&liveness_state),
    );
    let liveness_handle = tokio::spawn(liveness);
    let liveness_guard = LivenessGuard::new(liveness_handle, liveness_state);

    // Collects event IDs up front. On drop (any exit path — normal, early
    // return, or panic), spawns best-effort cleanup of both 👀 and 💬.
    // See `ReactionGuard` docs for ordering guarantees and known edge cases.
    let reaction_ids: Vec<String> = batch
        .as_ref()
        .map(|b| b.events.iter().map(|be| be.event.id.to_hex()).collect())
        .unwrap_or_default();
    let _reaction_guard = ReactionGuard::new(ctx.rest_client.clone(), reaction_ids.clone());

    //
    // Core memory is delivered inside the system prompt the harness already
    // builds (system role for protocol >= 2, the `[System]` user-message
    // section for legacy agents). To put it on the wire at `session/new` for
    // modern agents, the fetch must run *before* the session is created — so
    // we do it here and cache the rendered section in `state.core_sections`.
    //
    // Core is keyed by (agent_keys, owner) — both fixed for the process — so
    // it is identical across channels; the per-channel cache just avoids a
    // re-fetch on each new session and is cleared on session invalidation.
    //
    // Failure modes (all fail open — no crash, no block):
    //   * no owner configured → skip (no NIP-AE namespace exists)
    //   * confirmed absence → cache the onboarding nudge so the agent
    //     learns how to bootstrap itself.
    //   * transport / decrypt / parse error → inject nothing. We never
    //     mistake "relay slow or broken" for "no core" — that would invite
    //     the agent to overwrite real, just-unreachable memory.
    //   * fetch exceeds CORE_FETCH_TIMEOUT → inject nothing, same reason.
    //
    // Per Tyler's locked spec: NO mid-session refreshes. Re-fetch only
    // happens when a session is invalidated and recreated (see
    // `SessionState::invalidate_channel`).
    //
    // Operator opt-out: `--no-memory` / `BUZZ_ACP_NO_MEMORY` skips the fetch.
    if ctx.memory_enabled {
        if let (PromptSource::Channel(cid), Some(owner_pk)) =
            (&source, ctx.agent_owner_pubkey.as_ref())
        {
            let is_new_channel_session = !agent.state.sessions.contains_key(cid);
            if is_new_channel_session && !agent.state.core_sections.contains_key(cid) {
                // Bounded — we'd rather start the session with no core hint
                // than block session creation on a stalled relay.
                const CORE_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
                let fetch = crate::engram_fetch::build_core_section(
                    &ctx.rest_client,
                    &ctx.agent_keys,
                    owner_pk,
                );
                let section = match tokio::time::timeout(CORE_FETCH_TIMEOUT, fetch).await {
                    Ok(s) => s,
                    Err(_) => {
                        tracing::warn!(
                            target: "engram::core",
                            channel = %cid,
                            timeout_ms = CORE_FETCH_TIMEOUT.as_millis() as u64,
                            "core fetch timed out — emitting no section"
                        );
                        None
                    }
                };
                if let Some(rendered) = section {
                    tracing::info!(
                        target: "engram::core",
                        channel = %cid,
                        section_len = rendered.len(),
                        "injected NIP-AE core section into system prompt"
                    );
                    agent.state.core_sections.insert(*cid, rendered);
                }
            }
        }
    }

    // Channel metadata — resolved ONCE per turn. All consumers use this snapshot:
    // session title, canvas DM gate, initial_message context, and batch prompt context.
    // DM check fails closed: unknown channel type → treat as DM (no canvas, no description).
    let (turn_channel_info, is_dm_turn, title_channel) = match &source {
        PromptSource::Channel(cid) => {
            let info = ctx.channel_info.resolve(*cid).await;
            let is_dm = info
                .as_ref()
                .map(|i| i.channel_type == "dm")
                .unwrap_or(true); // fail closed on unknown
            let is_new_channel_session = !agent.state.sessions.contains_key(cid);
            let needs_title = is_new_channel_session && ctx.session_title.is_some();
            let title = if needs_title && !is_dm {
                info.as_ref()
                    .filter(|i| i.name != UNKNOWN_CHANNEL_NAME)
                    .map(|i| i.name.clone())
            } else {
                None
            };
            (info, is_dm, title)
        }
        PromptSource::Heartbeat => (None, false, None),
    };

    // Per-turn canvas fetch: resolve for the current turn and update the
    // process-lifetime cache. Canvas is only fetched for non-DM channel turns.
    //
    // A4: single-attempt, 3 s timeout, stale-on-failure. Runs before session
    // creation so the pointer is available for initial_message.
    let canvas_pointer: Option<crate::queue::CanvasPointer> = match &source {
        PromptSource::Channel(cid) if !is_dm_turn => {
            let result = fetch_canvas_pointer(*cid, &ctx.rest_client).await;
            ctx.canvas_cache.resolve_for_turn(cid, result)
        }
        _ => None,
    };

    // The core section to fold into the system prompt for this turn's session.
    // Channel-scoped; heartbeats carry no owner core.
    let agent_core: Option<String> = match &source {
        PromptSource::Channel(cid) => agent.state.core_sections.get(cid).cloned(),
        PromptSource::Heartbeat => None,
    };

    let (session_id, is_new_session) = match &source {
        PromptSource::Channel(cid) => {
            if let Some(sid) = agent.state.sessions.get(cid) {
                (sid.clone(), false)
            } else {
                // The title is channel-qualified (`Agent · #channel`) so one
                // agent in several channels doesn't produce identical session
                // rows; `title_channel` comes from the single resolve above and
                // is `None` for DM, unresolved, and unnamed channels.
                match create_session_and_apply_model(
                    &mut agent,
                    &ctx,
                    agent_core.as_deref(),
                    title_channel.as_deref(),
                    Some(*cid),
                    turn_channel_info.as_ref().map(|i| i.channel_type.as_str()),
                )
                .await
                {
                    Ok(sid) => {
                        tracing::info!(
                            target: "pool::session",
                            "created session {sid} for channel {cid}"
                        );
                        agent.state.sessions.insert(*cid, sid.clone());
                        (sid, true)
                    }
                    Err(AcpError::AgentExited) => {
                        agent.state.invalidate_all();
                        send_prompt_result(
                            &result_tx,
                            &turn_id,
                            agent,
                            source,
                            PromptOutcome::AgentExited,
                            requeue_batch_if_queue(&ctx, batch),
                        );
                        return;
                    }
                    Err(e) => {
                        // Session creation failed; pending canvas was never committed,
                        // so the next retry will re-fetch a fresh revision.
                        send_prompt_result(
                            &result_tx,
                            &turn_id,
                            agent,
                            source,
                            PromptOutcome::Error(e),
                            requeue_batch_if_queue(&ctx, batch),
                        );
                        return;
                    }
                }
            }
        }
        PromptSource::Heartbeat => {
            if let Some(sid) = &agent.state.heartbeat_session {
                (sid.clone(), false)
            } else {
                match create_session_and_apply_model(&mut agent, &ctx, None, None, None, None).await
                {
                    Ok(sid) => {
                        tracing::info!(
                            target: "pool::session",
                            "created heartbeat session {sid} for agent {}",
                            agent.index
                        );
                        agent.state.heartbeat_session = Some(sid.clone());
                        (sid, true)
                    }
                    Err(AcpError::AgentExited) => {
                        agent.state.invalidate_all();
                        send_prompt_result(
                            &result_tx,
                            &turn_id,
                            agent,
                            source,
                            PromptOutcome::AgentExited,
                            None,
                        );
                        return;
                    }
                    Err(e) => {
                        send_prompt_result(
                            &result_tx,
                            &turn_id,
                            agent,
                            source,
                            PromptOutcome::Error(e),
                            None,
                        );
                        return;
                    }
                }
            }
        }
    };
    agent.acp.set_observer_context(observer::context_for_turn(
        observer_channel_id,
        Some(session_id.clone()),
        turn_id.clone(),
        turn_started_at,
    ));
    // Backfill liveness's shared session ID so ticks after this point carry
    // it too, matching every other observer frame for this turn.
    liveness_guard.set_session_id(session_id.clone());
    agent.acp.observe(
        "session_resolved",
        serde_json::json!({
            "sessionId": session_id,
            "isNewSession": is_new_session,
        }),
    );

    if is_new_session {
        if let (PromptSource::Channel(cid), Some(ref initial_msg)) = (&source, &ctx.initial_message)
        {
            tracing::info!(
                target: "pool::session",
                "sending initial_message to session {session_id} for channel {cid}"
            );
            // Build the initial_message content. Base prompt is prepended for
            // legacy agents (protocol < 2) in the user message; protocol-v2
            // agents already have it in systemPrompt.
            //
            // Both modern and legacy agents receive the channel [Context] block
            // (A2 — canvas awareness before the first prompt).
            let init_msg = prepend_base_for_legacy(
                if agent.has_system_prompt_support() {
                    2
                } else {
                    1
                },
                ctx.base_prompt,
                initial_msg,
            );
            // Prepend [Context] block (description + canvas pointer) to initial_message.
            // Uses the same turn snapshot (turn_channel_info, is_dm_turn, canvas_pointer)
            // as the subsequent batch prompt — one consistent view per inbound turn (A2).
            let init_msg = {
                let ctx_block = crate::queue::format_context_hints(
                    *cid,
                    turn_channel_info.as_ref(),
                    &crate::queue::ThreadTags::default(),
                    is_dm_turn,
                    false,
                    None,
                    canvas_pointer.as_ref(),
                );
                format!("{ctx_block}\n\n{init_msg}")
            };
            let init_result = agent
                .acp
                .session_prompt_with_idle_timeout(
                    &session_id,
                    &init_msg,
                    ctx.idle_timeout,
                    ctx.max_turn_duration,
                )
                .await;

            match init_result {
                Ok(stop_reason) => {
                    tracing::info!(
                        target: "pool::session",
                        "initial_message complete for channel {cid}: {stop_reason:?}"
                    );
                }
                Err(AcpError::AgentExited) => {
                    agent.state.invalidate_all();
                    send_prompt_result(
                        &result_tx,
                        &turn_id,
                        agent,
                        source,
                        PromptOutcome::AgentExited,
                        requeue_batch_if_queue(&ctx, batch),
                    );
                    return;
                }
                Err(AcpError::IdleTimeout(_)) => {
                    tracing::warn!(
                        target: "pool::session",
                        "initial_message idle timeout ({}s) for channel {cid} — cancelling",
                        ctx.idle_timeout.as_secs()
                    );
                    match agent
                        .acp
                        .cancel_with_cleanup(&session_id, ctx.idle_timeout)
                        .await
                    {
                        Ok(_) => {
                            agent.state.invalidate(&source);
                        }
                        Err(AcpError::AgentExited) => {
                            agent.state.invalidate_all();
                            send_prompt_result(
                                &result_tx,
                                &turn_id,
                                agent,
                                source,
                                PromptOutcome::AgentExited,
                                requeue_batch_if_queue(&ctx, batch),
                            );
                            return;
                        }
                        Err(e) => {
                            tracing::error!(
                                target: "pool::session",
                                "cancel_with_cleanup failed during initial_message timeout: {e}"
                            );
                            agent.state.invalidate(&source);
                        }
                    }
                    send_prompt_result(
                        &result_tx,
                        &turn_id,
                        agent,
                        source,
                        PromptOutcome::Timeout(TimeoutKind::Idle),
                        requeue_batch_if_queue(&ctx, batch),
                    );
                    return;
                }
                Err(AcpError::HardTimeout { silence }) => {
                    let recently_active = silence < RECENT_ACTIVITY_WINDOW;
                    tracing::error!(
                        target: "pool::session",
                        "hard timeout ({}s cap, silence {silence:?}, recently_active={recently_active}) during initial_message for channel {cid} — agent process is unrecoverable",
                        ctx.max_turn_duration.as_secs()
                    );
                    agent.state.invalidate_all();
                    send_prompt_result(
                        &result_tx,
                        &turn_id,
                        agent,
                        source,
                        PromptOutcome::Timeout(TimeoutKind::Hard { recently_active }),
                        requeue_batch_if_queue(&ctx, batch),
                    );
                    return;
                }
                Err(e) => {
                    tracing::error!(
                        target: "pool::session",
                        "initial_message failed for channel {cid}: {e} — invalidating session"
                    );
                    agent.state.invalidate(&source);
                    send_prompt_result(
                        &result_tx,
                        &turn_id,
                        agent,
                        source,
                        PromptOutcome::Error(e),
                        requeue_batch_if_queue(&ctx, batch),
                    );
                    return;
                }
            }
        }
    }

    // When the batch is a single slash-command message (e.g. "@Eva /goal …"),
    // `slash_command` holds the bare command. It is sent as the FIRST prompt
    // content block so ACP connectors' slash-command detection
    // (`prompt[0].text.startsWith("/")`) fires; the wrapped Buzz context
    // follows as a second block.
    let mut slash_command: Option<String> = None;
    let prompt_sections: Vec<String> = if let Some(text) = prompt_text {
        // Heartbeats create their session before this point, so a Goose method-not-found
        // probe has already selected the correct framing for this process.
        let text = prepend_base_for_legacy(
            if agent.has_system_prompt_support() {
                2
            } else {
                1
            },
            ctx.base_prompt,
            &text,
        );
        vec![text]
    } else if let Some(ref b) = batch {
        // Build prompt from batch with context enrichment.
        // Use the per-turn metadata snapshot (resolved once at turn start) rather
        // than resolving again — ensures batch and initial_message see the same info.
        let channel_info = turn_channel_info.clone();

        let conversation_context = if ctx.context_message_limit > 0 {
            fetch_conversation_context(b, &ctx, is_dm_turn).await
        } else {
            None
        };

        let profile_lookup =
            fetch_prompt_profile_lookup(b, conversation_context.as_ref(), &ctx.rest_client).await;

        let known_names: Vec<&str> = profile_lookup
            .iter()
            .flat_map(|lookup| lookup.values())
            .flat_map(|p| [p.display_name.as_deref(), p.nip05_handle.as_deref()])
            .flatten()
            .collect();
        slash_command = crate::queue::slash_command_for_batch(b, &known_names);
        if let Some(ref cmd) = slash_command {
            tracing::info!(
                target: "pool::prompt",
                channel = %b.channel_id,
                command = %cmd,
                "slash-command pass-through"
            );
        }

        crate::queue::format_prompt(
            b,
            &crate::queue::FormatPromptArgs {
                agent_core: agent_core.as_deref(),
                channel_info: channel_info.as_ref(),
                conversation_context: conversation_context.as_ref(),
                profile_lookup: profile_lookup.as_ref(),
                has_system_prompt_support: agent.has_system_prompt_support(),
                base_prompt: ctx.base_prompt,
                system_prompt: ctx.system_prompt.as_deref(),
                team_instructions: ctx.team_instructions.as_deref(),
                canvas_pointer: canvas_pointer.as_ref(),
                is_dm: is_dm_turn,
            },
        )
    } else {
        // Should not happen — batch is None only for heartbeats which have prompt_text.
        // Return the agent to the pool to prevent a permanent slot leak.
        tracing::error!("run_prompt_task: no batch and no prompt_text — returning agent");
        send_prompt_result(
            &result_tx,
            &turn_id,
            agent,
            source,
            PromptOutcome::Error(AcpError::Protocol("no batch and no prompt_text".into())),
            None,
        );
        return;
    };

    // 💬 — fire-and-forget so the prompt fires immediately.
    // The guard's cleanup (spawned on drop) removes 💬 after the turn completes.
    // A brief race where 💬 appears slightly after the agent starts is acceptable.
    if !reaction_ids.is_empty() {
        let rest = ctx.rest_client.clone();
        let ids = reaction_ids.clone();
        tokio::spawn(async move {
            react_working(&rest, &ids).await;
        });
    }

    // Slash-command pass-through sends the bare command as the first text
    // block (so connector detection fires), then each prompt section as its
    // own block. Per-section blocks let the observer size trimmer elide a
    // section body in place while every `[Header]` line survives at the head
    // of its own leaf — so the "Prompt context" panel counts every section.
    let prompt_blocks: Vec<&str> = match slash_command {
        Some(ref cmd) => std::iter::once(cmd.as_str())
            .chain(prompt_sections.iter().map(String::as_str))
            .collect(),
        None => prompt_sections.iter().map(String::as_str).collect(),
    };

    // Turn start, labelled exactly as `log_stop_reason` labels the end, so a
    // log reads as start/stop pairs. Purely observational: an unpaired start is
    // the only durable evidence that a turn was entered and never returned, and
    // without it a stalled agent and an agent nobody woke leave identical logs —
    // zero completions either way, so anything reading them afterwards has to
    // guess which happened.
    tracing::info!(
        target: "pool::prompt",
        "turn starting for {}",
        prompt_label(&source)
    );

    // When control_rx is Some (channel tasks), wrap the prompt in select! so
    // the main loop can cancel, interrupt, or rotate it. Heartbeats
    // (control_rx=None) take the simple await path — they are not controllable.
    //
    let prompt_result = match control_rx {
        None => {
            // Heartbeat / non-cancellable path.
            tokio::select! {
                biased;
                result = agent.acp.session_prompt_blocks_with_idle_timeout(
                    &session_id,
                    &prompt_blocks,
                    ctx.idle_timeout,
                    ctx.max_turn_duration,
                ) => result,
            }
        }
        Some(rx) => {
            tokio::select! {
                biased;
                result = agent.acp.session_prompt_blocks_with_idle_timeout(
                    &session_id,
                    &prompt_blocks,
                    ctx.idle_timeout,
                    ctx.max_turn_duration,
                ) => result,
                mode = rx => {
                    let control_signal = mode.unwrap_or(ControlSignal::Cancel);
                    // Land the model switch before any cancel/requeue work: setting
                    // `desired_model` here means the fresh session created by the
                    // requeued turn (busy) or the next turn (already-completed)
                    // applies the new model. Runtime-only — never persisted.
                    if let ControlSignal::SwitchModel(ref model_id) = control_signal {
                        agent.desired_model = Some(model_id.clone());
                        agent.model_overridden = true;
                    }
                    // Control signal received. Guard against Race 1: the turn may
                    // have completed naturally just as cancel fired.
                    if agent.acp.has_in_flight_prompt() {
                        // Prompt is genuinely in-flight — cancel it.
                        match agent
                            .acp
                            .cancel_with_cleanup_grace(&session_id, CONTROL_CANCEL_GRACE)
                            .await
                        {
                            Ok(stop_reason) => {
                                log_stop_reason(&source, &stop_reason);
                                agent.state.invalidate(&source);
                                let retry_batch =
                                    requeue_cancelled_batch(&ctx, control_signal, batch);

                                let usage = agent.acp.take_turn_usage();
                                publish_agent_turn_metric(
                                    &ctx,
                                    usage,
                                    observer_channel_id,
                                    &session_id,
                                    &turn_id,
                                    Some(buzz_core::agent_turn_metric::StopReason::Cancelled),
                                )
                                .await;
                                send_prompt_result(
                                    &result_tx,
                                    &turn_id,
                                    agent,
                                    source,
                                    PromptOutcome::Cancelled,
                                    retry_batch,
                                );
                                return;
                            }
                            Err(error) => {
                                // Single production arm: classify the error→outcome
                                // and outcome→batch-fate boundary once via the seam
                                // shared with tests, then invalidate/publish/send once.
                                let failure = classify_control_cancel_failure(
                                    &ctx,
                                    error,
                                    control_signal,
                                    batch,
                                );
                                if failure.invalidate_all {
                                    agent.state.invalidate_all();
                                } else {
                                    agent.state.invalidate(&source);
                                }

                                let usage = agent.acp.take_turn_usage();
                                publish_agent_turn_metric(
                                    &ctx,
                                    usage,
                                    observer_channel_id,
                                    &session_id,
                                    &turn_id,
                                    Some(buzz_core::agent_turn_metric::StopReason::Error),
                                )
                                .await;
                                send_prompt_result(
                                    &result_tx,
                                    &turn_id,
                                    agent,
                                    source,
                                    failure.outcome,
                                    failure.retry_batch,
                                );
                                return;
                            }
                        }
                    } else {
                        // Race 1 resolution: turn completed naturally before cancel
                        // could fire. last_prompt_id is None — cleared by
                        // session_prompt_with_idle_timeout() on success. The prompt
                        // future was dropped by select! — its Ok result is gone.
                        //
                        // Note: this `else` branch (last_prompt_id is None) cannot
                        // fire during the pre-prompt phase because `biased` select!
                        // polls the prompt arm first. That arm sets last_prompt_id
                        // synchronously before its first yield point, so by the time
                        // the cancel arm can win, last_prompt_id is already Some.
                        // This branch only fires when the turn genuinely completed
                        // and last_prompt_id was cleared by the success path.
                        //
                        // MUST send a PromptResult or the main loop deadlocks.
                        if matches!(
                            control_signal,
                            ControlSignal::Rotate | ControlSignal::SwitchModel(_)
                        ) {
                            tracing::debug!(
                                target: "pool::prompt",
                                "rotate/switch signal arrived but turn already completed — invalidating session"
                            );
                        } else {
                            tracing::debug!(
                                target: "pool::prompt",
                                "control signal arrived but turn already completed — treating as success"
                            );
                        }
                        apply_completed_before_control_signal(
                            &mut agent.state,
                            &source,
                            &control_signal,
                        );
                        let usage = agent.acp.take_turn_usage();
                        publish_agent_turn_metric(
                            &ctx,
                            usage,
                            observer_channel_id,
                            &session_id,
                            &turn_id,
                            Some(buzz_core::agent_turn_metric::StopReason::EndTurn),
                        )
                        .await;
                        send_prompt_result(
                            &result_tx,
                            &turn_id,
                            agent,
                            source,
                            PromptOutcome::Ok(StopReason::EndTurn),
                            None, // turn succeeded — batch was processed, no requeue
                        );
                        return;
                    }
                }
            }
        }
    };

    match prompt_result {
        Ok(stop_reason) => {
            log_stop_reason(&source, &stop_reason);

            let should_rotate = matches!(
                stop_reason,
                StopReason::MaxTokens | StopReason::MaxTurnRequests
            );

            let should_rotate = should_rotate || {
                let limit = ctx.max_turns_per_session;
                if limit > 0 {
                    match &source {
                        PromptSource::Channel(cid) => {
                            let count = agent.state.turn_counts.entry(*cid).or_insert(0);
                            *count += 1;
                            *count >= limit
                        }
                        PromptSource::Heartbeat => {
                            agent.state.heartbeat_turn_count += 1;
                            agent.state.heartbeat_turn_count >= limit
                        }
                    }
                } else {
                    false
                }
            };

            if should_rotate {
                tracing::info!(
                    target: "pool::session",
                    "rotating session for {source:?} after {stop_reason:?}",
                );
                agent.state.invalidate(&source);
            }

            let core_stop = acp_stop_to_core(&stop_reason);
            let usage = agent.acp.take_turn_usage();
            publish_agent_turn_metric(
                &ctx,
                usage,
                observer_channel_id,
                &session_id,
                &turn_id,
                Some(core_stop),
            )
            .await;

            send_prompt_result(
                &result_tx,
                &turn_id,
                agent,
                source,
                PromptOutcome::Ok(stop_reason),
                None,
            );
        }
        Err(AcpError::AgentExited) => {
            tracing::error!(target: "pool::prompt", "agent {} exited during prompt", agent.index);
            agent.state.invalidate_all();
            let usage = agent.acp.take_turn_usage();
            publish_agent_turn_metric(
                &ctx,
                usage,
                observer_channel_id,
                &session_id,
                &turn_id,
                Some(buzz_core::agent_turn_metric::StopReason::Error),
            )
            .await;
            send_prompt_result(
                &result_tx,
                &turn_id,
                agent,
                source,
                PromptOutcome::AgentExited,
                requeue_batch_if_queue(&ctx, batch),
            );
        }
        Err(AcpError::IdleTimeout(_)) => {
            tracing::warn!(
                target: "pool::prompt",
                "idle timeout ({}s) — cancelling session {session_id}",
                ctx.idle_timeout.as_secs()
            );
            match agent
                .acp
                .cancel_with_cleanup(&session_id, ctx.idle_timeout)
                .await
            {
                Ok(stop_reason) => {
                    log_stop_reason(&source, &stop_reason);
                    let usage = agent.acp.take_turn_usage();
                    publish_agent_turn_metric(
                        &ctx,
                        usage,
                        observer_channel_id,
                        &session_id,
                        &turn_id,
                        Some(buzz_core::agent_turn_metric::StopReason::Cancelled),
                    )
                    .await;
                    // Timeout triggers respawn in handle_prompt_result —
                    // session state will be discarded with the old agent.
                    send_prompt_result(
                        &result_tx,
                        &turn_id,
                        agent,
                        source,
                        PromptOutcome::Timeout(TimeoutKind::Idle),
                        requeue_batch_if_queue(&ctx, batch),
                    );
                }
                Err(AcpError::AgentExited) => {
                    tracing::error!(
                        target: "pool::prompt",
                        "agent {} exited during cancel_with_cleanup",
                        agent.index
                    );
                    agent.state.invalidate_all();
                    let usage = agent.acp.take_turn_usage();
                    publish_agent_turn_metric(
                        &ctx,
                        usage,
                        observer_channel_id,
                        &session_id,
                        &turn_id,
                        Some(buzz_core::agent_turn_metric::StopReason::Error),
                    )
                    .await;
                    send_prompt_result(
                        &result_tx,
                        &turn_id,
                        agent,
                        source,
                        PromptOutcome::AgentExited,
                        requeue_batch_if_queue(&ctx, batch),
                    );
                }
                Err(e) => {
                    tracing::error!(
                        target: "pool::prompt",
                        "cancel_with_cleanup error: {e} — invalidating session"
                    );
                    agent.state.invalidate(&source);
                    let usage = agent.acp.take_turn_usage();
                    publish_agent_turn_metric(
                        &ctx,
                        usage,
                        observer_channel_id,
                        &session_id,
                        &turn_id,
                        Some(buzz_core::agent_turn_metric::StopReason::Error),
                    )
                    .await;
                    send_prompt_result(
                        &result_tx,
                        &turn_id,
                        agent,
                        source,
                        PromptOutcome::Timeout(TimeoutKind::Idle),
                        requeue_batch_if_queue(&ctx, batch),
                    );
                }
            }
        }
        Err(AcpError::HardTimeout { silence }) => {
            let recently_active = silence < RECENT_ACTIVITY_WINDOW;
            tracing::error!(
                target: "pool::prompt",
                "hard timeout ({}s cap, silence {silence:?}, recently_active={recently_active}) — agent process is unrecoverable, invalidating all sessions",
                ctx.max_turn_duration.as_secs()
            );
            agent.state.invalidate_all();
            let usage = agent.acp.take_turn_usage();
            publish_agent_turn_metric(
                &ctx,
                usage,
                observer_channel_id,
                &session_id,
                &turn_id,
                Some(buzz_core::agent_turn_metric::StopReason::Error),
            )
            .await;
            send_prompt_result(
                &result_tx,
                &turn_id,
                agent,
                source,
                PromptOutcome::Timeout(TimeoutKind::Hard { recently_active }),
                requeue_batch_if_queue(&ctx, batch),
            );
        }
        Err(e) => {
            tracing::error!(target: "pool::prompt", "session_prompt error: {e}");
            // AgentError means the agent caught a problem before mutating
            // session state (e.g. bad LLM response). The session is healthy —
            // don't invalidate it. Other errors may have corrupted state.
            if !matches!(e, AcpError::AgentError { .. }) {
                agent.state.invalidate(&source);
            }
            let usage = agent.acp.take_turn_usage();
            publish_agent_turn_metric(
                &ctx,
                usage,
                observer_channel_id,
                &session_id,
                &turn_id,
                Some(buzz_core::agent_turn_metric::StopReason::Error),
            )
            .await;
            send_prompt_result(
                &result_tx,
                &turn_id,
                agent,
                source,
                PromptOutcome::Error(e),
                requeue_batch_if_queue(&ctx, batch),
            );
        }
    }
    // _reaction_guard drops here → spawns clear_reactions for all exit paths.
}

/// Retry wrapper for context fetches: one retry with `CONTEXT_FETCH_RETRY_DELAY`
/// on any `None` result. The closure is called twice at most.
///
/// Using a closure (not a `Future`) so the retry can construct a fresh `Future`
/// each attempt without requiring `Clone` or re-boxing.
async fn fetch_with_retry<F, Fut, T>(f: F) -> Option<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    if let Some(result) = f().await {
        return Some(result);
    }
    tokio::time::sleep(CONTEXT_FETCH_RETRY_DELAY).await;
    f().await
}

/// Lazy-fetch channel metadata for a channel not in the startup discovery cache.
///
/// Handles channels added dynamically via membership notifications after startup.
/// Uses `CONTEXT_FETCH_TIMEOUT` with one retry on failure. Returns `None` on
/// persistent failure (graceful degradation — prompt will lack channel name and
/// DM detection).
pub(crate) async fn fetch_channel_info(
    channel_id: Uuid,
    rest: &RestClient,
) -> Option<PromptChannelInfo> {
    use nostr::{Alphabet, SingleLetterTag};

    let d_tag = SingleLetterTag::lowercase(Alphabet::D);
    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(
            buzz_core::kind::KIND_NIP29_GROUP_METADATA as u16,
        ))
        .custom_tags(d_tag, [channel_id.to_string()]);

    fetch_with_retry(|| async {
        match timeout(
            CONTEXT_FETCH_TIMEOUT,
            rest.query(std::slice::from_ref(&filter)),
        )
        .await
        {
            Ok(Ok(json)) => {
                let events = json.as_array()?;
                let ev = events.first()?;
                let tags = ev.get("tags")?.as_array()?;
                let mut name = None;
                let mut description = None;
                for tag in tags {
                    if let Some(arr) = tag.as_array() {
                        match arr.first().and_then(|v| v.as_str()) {
                            Some("name") => name = arr.get(1).and_then(|v| v.as_str()),
                            Some("about") => description = arr.get(1).and_then(|v| v.as_str()),
                            _ => {}
                        }
                    }
                }
                let channel_type = crate::relay::channel_type_from_tags(tags);
                let description = description
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                Some(PromptChannelInfo {
                    name: name.unwrap_or(UNKNOWN_CHANNEL_NAME).to_string(),
                    channel_type,
                    description,
                })
            }
            Ok(Err(e)) => {
                tracing::debug!(
                    channel_id = %channel_id,
                    "channel info fetch failed: {e} — will retry"
                );
                None
            }
            Err(_) => {
                tracing::debug!(
                    channel_id = %channel_id,
                    "channel info fetch timed out — will retry"
                );
                None
            }
        }
    })
    .await
}

/// Fetch the latest canvas event for `channel_id` and return a `CanvasFetchResult`.
///
/// - `Present(pointer)` — a valid, non-blank canvas event was found.
/// - `Absent`           — relay confirmed no event, or the latest event has blank content
///   (cleared canvas); older revisions are NOT resurrected.
/// - `Failed`           — transport timeout, REST error, or parse/verification error.
///   Callers must serve stale on `Failed`.
///
/// Single-attempt with a 3-second timeout — never wrapped in `fetch_with_retry`.
/// Accepted latency bound: at most +3 s per turn on a degraded relay.
async fn fetch_canvas_pointer(channel_id: Uuid, rest: &RestClient) -> CanvasFetchResult {
    use nostr::{Alphabet, SingleLetterTag};

    let h_tag = SingleLetterTag::lowercase(Alphabet::H);
    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(buzz_core::kind::KIND_CANVAS as u16))
        .custom_tags(h_tag, [channel_id.to_string()])
        .limit(1);

    const CANVAS_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

    let json = match tokio::time::timeout(
        CANVAS_FETCH_TIMEOUT,
        rest.query_once(std::slice::from_ref(&filter)),
    )
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_id,
                "canvas query failed: {e} — serving stale"
            );
            return CanvasFetchResult::Failed;
        }
        Err(_) => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_id,
                timeout_ms = CANVAS_FETCH_TIMEOUT.as_millis() as u64,
                "canvas fetch timed out — serving stale"
            );
            return CanvasFetchResult::Failed;
        }
    };

    let events = match json.as_array() {
        Some(arr) => arr,
        None => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_id,
                "canvas query response is not a JSON array — serving stale"
            );
            return CanvasFetchResult::Failed;
        }
    };

    canvas_pointer_from_query_response(events, &channel_id.to_string())
}

/// Parse a canvas query response array and return a `CanvasFetchResult`.
///
/// Extracted as a pure function so tests can exercise the parsing/validation
/// logic without async machinery or relay connectivity.
///
/// Returns:
/// - `Present(pointer)` — a valid, non-blank canvas event was found.
/// - `Absent` — empty array (confirmed no canvas) or blank/deleted content.
/// - `Failed` — malformed/partial event JSON, signature verification failure,
///   wrong kind, wrong channel tag, or out-of-range timestamp. The caller must
///   preserve any cached stale pointer rather than clearing it.
pub(crate) fn canvas_pointer_from_query_response(
    events: &[serde_json::Value],
    channel_uuid: &str,
) -> CanvasFetchResult {
    // Empty array: relay confirmed no canvas event exists.
    let raw = match events.first() {
        Some(v) => v,
        None => return CanvasFetchResult::Absent,
    };

    // Deserialise as a complete Nostr Event. Partial objects (missing pubkey,
    // sig, kind, or tags) are rejected here rather than trusted implicitly.
    // These are parse/structural failures → Failed (preserve stale).
    let event = match serde_json::from_value::<nostr::Event>(raw.clone()) {
        Ok(ev) => ev,
        Err(err) => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_uuid,
                %err,
                "canvas query returned a malformed event — serving stale",
            );
            return CanvasFetchResult::Failed;
        }
    };

    // Verify the event's id and signature agree with its content.
    // A structurally complete but tampered event must not supply trusted metadata.
    // Verification failure → Failed (preserve stale).
    if let Err(err) = event.verify() {
        tracing::warn!(
            target: "canvas::fetch",
            channel = %channel_uuid,
            %err,
            "canvas event failed signature verification — serving stale",
        );
        return CanvasFetchResult::Failed;
    }

    // Validate kind: must be KIND_CANVAS (40100).
    // Wrong kind → Failed (preserve stale).
    if event.kind != nostr::Kind::Custom(buzz_core::kind::KIND_CANVAS as u16) {
        tracing::warn!(
            target: "canvas::fetch",
            channel = %channel_uuid,
            kind = %event.kind.as_u16(),
            "canvas event has unexpected kind — serving stale",
        );
        return CanvasFetchResult::Failed;
    }

    // Validate h-tag: must carry the channel UUID we queried.
    // The REST boundary filters by #h, but we verify here to prevent a
    // misbehaving relay from injecting a different channel's canvas.
    // Wrong h-tag → Failed (preserve stale).
    let h_tag_matches = event.tags.iter().any(|tag| {
        let v = tag.as_slice();
        v.len() >= 2 && v[0] == "h" && v[1] == channel_uuid
    });
    if !h_tag_matches {
        tracing::warn!(
            target: "canvas::fetch",
            channel = %channel_uuid,
            "canvas event is missing expected h-tag — serving stale",
        );
        return CanvasFetchResult::Failed;
    }

    // Blank content means the canvas was cleared; do not fall back to older events.
    // Confirmed cleared canvas → Absent (clears any cached pointer).
    if event.content.trim().is_empty() {
        tracing::debug!(
            target: "canvas::fetch",
            channel = %channel_uuid,
            "latest canvas event has blank content — canvas absent"
        );
        return CanvasFetchResult::Absent;
    }

    let id = event.id.to_hex();

    // Convert the Nostr timestamp to a UTC RFC3339 string with Z suffix.
    // Use checked conversion: a u64 that exceeds i64::MAX (e.g. Timestamp::max())
    // wraps silently with `as i64`, producing a negative value that chrono would
    // accept as a date in 1969. Reject out-of-range values explicitly instead.
    // Out-of-range timestamp → Failed (preserve stale).
    let ts_secs = match i64::try_from(event.created_at.as_secs()) {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_uuid,
                "canvas event created_at overflows i64 — serving stale",
            );
            return CanvasFetchResult::Failed;
        }
    };
    let timestamp = match chrono::DateTime::from_timestamp(ts_secs, 0) {
        Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        None => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_uuid,
                ts_secs,
                "canvas event has out-of-range created_at — serving stale",
            );
            return CanvasFetchResult::Failed;
        }
    };

    tracing::info!(
        target: "canvas::fetch",
        channel = %channel_uuid,
        event_id = %id,
        "resolved canvas revision pointer for [Context]"
    );
    CanvasFetchResult::Present(crate::queue::CanvasPointer {
        event_id: id,
        timestamp,
    })
}

/// Fetch conversation context (thread or DM) for a batch before prompting.
///
/// Returns `None` if:
/// - The event is a plain channel message (not a thread reply, not a DM)
/// - The REST fetch fails or times out (graceful degradation)
/// - `context_message_limit` is 0
///
/// For batches with multiple events, thread context is fetched for the **last**
/// reply event only (most recent = most likely to need a response).
async fn fetch_conversation_context(
    batch: &FlushBatch,
    ctx: &PromptContext,
    is_dm_turn: bool,
) -> Option<ConversationContext> {
    let limit = ctx.context_message_limit;

    // Check thread tags on the last event first — this applies to both
    // channels and DMs. A DM reply needs thread context (not channel history)
    // because /api/channels/{id}/messages excludes thread replies.
    let last_event = batch.events.last()?;
    let tags = crate::queue::parse_thread_tags(&last_event.event);
    if let Some(root_id) = tags.root_event_id {
        return fetch_thread_context(
            batch.channel_id,
            &root_id,
            limit,
            ctx.agent_keys.public_key(),
            &ctx.rest_client,
        )
        .await;
    }

    // DM non-reply: fetch recent conversation history.
    if is_dm_turn {
        return fetch_dm_context(batch.channel_id, limit, &ctx.rest_client).await;
    }

    None
}

/// Normalize AND validate a pubkey for the batch profile API request.
/// Returns `None` for malformed input — only valid 64-char hex passes.
/// See also: `normalize_lookup_key` in queue.rs (normalize-only, no validation).
fn normalize_prompt_pubkey(pubkey: &str) -> Option<String> {
    let normalized = pubkey.trim().to_ascii_lowercase();
    if normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(normalized)
    } else {
        None
    }
}

fn collect_prompt_pubkeys(
    batch: &FlushBatch,
    conversation_context: Option<&ConversationContext>,
) -> Vec<String> {
    let mut pubkeys = HashSet::new();

    for event in &batch.events {
        pubkeys.insert(event.event.pubkey.to_hex().to_ascii_lowercase());

        for mentioned in crate::queue::parse_thread_tags(&event.event).mentioned_pubkeys {
            if let Some(normalized) = normalize_prompt_pubkey(&mentioned) {
                pubkeys.insert(normalized);
            }
        }
    }

    let context_messages = match conversation_context {
        Some(ConversationContext::Thread { messages, .. })
        | Some(ConversationContext::Dm { messages, .. }) => Some(messages),
        None => None,
    };

    if let Some(messages) = context_messages {
        for message in messages {
            if let Some(normalized) = normalize_prompt_pubkey(&message.pubkey) {
                pubkeys.insert(normalized);
            }
        }
    }

    let mut pubkeys: Vec<String> = pubkeys.into_iter().collect();
    pubkeys.sort();
    pubkeys
}

/// Detect whether a kind:0 profile event belongs to an owned agent.
///
/// Agents carry a NIP-OA `["auth", owner_pk, conditions, sig]` tag in their
/// profile; humans do not. This checks for the tag's presence/shape only — a
/// cheap routing heuristic for reply anchoring, not a verified security gate
/// (the signing path in `lib.rs::check_sibling_via_profile` does full
/// verification where it matters).
fn profile_event_is_agent(ev: &serde_json::Value) -> bool {
    ev.get("tags")
        .and_then(|t| t.as_array())
        .is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_array()
                    .is_some_and(|parts| parts.len() == 4 && parts[0].as_str() == Some("auth"))
            })
        })
}

/// Parse kind:0 profile events into a `PromptProfileLookup`.
///
/// Each kind:0 event has `pubkey` and JSON `content` with optional fields:
/// `display_name` (or `name`), `nip05`.
fn parse_kind0_profile_lookup(json: serde_json::Value) -> Option<PromptProfileLookup> {
    let events = json.as_array()?;
    let mut lookup = PromptProfileLookup::new();

    for ev in events {
        let pubkey = ev.get("pubkey").and_then(|v| v.as_str());
        let content_str = ev.get("content").and_then(|v| v.as_str());
        if let (Some(pk), Some(content)) = (pubkey, content_str) {
            if let Ok(profile) = serde_json::from_str::<serde_json::Value>(content) {
                let display_name = profile
                    .get("display_name")
                    .or_else(|| profile.get("name"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let nip05_handle = profile
                    .get("nip05")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let is_agent = profile_event_is_agent(ev);
                lookup.insert(
                    pk.to_ascii_lowercase(),
                    PromptProfile {
                        display_name,
                        nip05_handle,
                        is_agent,
                    },
                );
            }
        }
    }

    if lookup.is_empty() {
        None
    } else {
        Some(lookup)
    }
}

async fn fetch_prompt_profile_lookup(
    batch: &FlushBatch,
    conversation_context: Option<&ConversationContext>,
    rest: &RestClient,
) -> Option<PromptProfileLookup> {
    let pubkeys = collect_prompt_pubkeys(batch, conversation_context);
    if pubkeys.is_empty() {
        return None;
    }

    // Query kind:0 (NIP-01 profile metadata) for all pubkeys.
    let authors: Vec<nostr::PublicKey> = pubkeys
        .iter()
        .filter_map(|s| nostr::PublicKey::from_hex(s).ok())
        .collect();
    if authors.is_empty() {
        return None;
    }
    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Metadata)
        .authors(authors);

    fetch_with_retry(|| async {
        match timeout(
            CONTEXT_FETCH_TIMEOUT,
            rest.query(std::slice::from_ref(&filter)),
        )
        .await
        {
            Ok(Ok(json)) => parse_kind0_profile_lookup(json),
            Ok(Err(e)) => {
                tracing::debug!("prompt profile lookup failed: {e} — will retry");
                None
            }
            Err(_) => {
                tracing::debug!("prompt profile lookup timed out — will retry");
                None
            }
        }
    })
    .await
}

/// Fetch thread context via Nostr query: root event by ID + replies by `#e` tag.
///
/// The reply query intentionally requests one more reply than the configured
/// display window. That sentinel event lets the prompt say `N of M, truncated`
/// when the relay has more thread history, instead of reporting the capped page
/// as the total. When the window is full, a best-effort `/count` attempts to
/// improve that lower-bound total; because it is a separate racy request, the
/// result is clamped to the sentinel-proven minimum. The query also asks for the
/// agent's newest reply separately so the next prompt can include the agent's
/// own prior turn even in busy threads where the recent-message window would
/// otherwise push it out.
async fn fetch_thread_context(
    channel_id: Uuid,
    root_event_id: &str,
    limit: u32,
    agent_pubkey: nostr::PublicKey,
    rest: &RestClient,
) -> Option<ConversationContext> {
    fetch_thread_context_with(
        channel_id,
        root_event_id,
        limit,
        agent_pubkey,
        |filters| async move { rest.query(&filters).await },
        |filters| async move { rest.count(&filters).await },
    )
    .await
}

async fn fetch_thread_context_with<Query, QueryFut, Count, CountFut>(
    channel_id: Uuid,
    root_event_id: &str,
    limit: u32,
    agent_pubkey: nostr::PublicKey,
    query: Query,
    count: Count,
) -> Option<ConversationContext>
where
    Query: Fn(Vec<nostr::Filter>) -> QueryFut,
    QueryFut: std::future::Future<Output = Result<serde_json::Value, crate::relay::RelayError>>,
    Count: Fn(Vec<nostr::Filter>) -> CountFut,
    CountFut: std::future::Future<Output = Result<serde_json::Value, crate::relay::RelayError>>,
{
    use nostr::{Alphabet, SingleLetterTag};

    // Defense-in-depth: validate hex event ID.
    if root_event_id.is_empty()
        || root_event_id.len() != 64
        || !root_event_id.chars().all(|c| c.is_ascii_hexdigit())
    {
        tracing::warn!(
            channel_id = %channel_id,
            "invalid root_event_id (expected 64 hex chars) — skipping thread context fetch"
        );
        return None;
    }

    let e_tag = SingleLetterTag::lowercase(Alphabet::E);
    let h_tag = SingleLetterTag::lowercase(Alphabet::H);
    let ch_str = channel_id.to_string();

    // Three filters: (1) root event by ID, (2) recent replies with #e=root +
    // #h=channel plus a sentinel, and (3) the agent's newest reply for pinning.
    let root_filter = nostr::Filter::new().id(nostr::EventId::from_hex(root_event_id).ok()?);
    let replies_filter = nostr::Filter::new()
        .kinds([
            nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE as u16),
            nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE_V2 as u16),
        ])
        .custom_tags(e_tag, [root_event_id])
        .custom_tags(h_tag, [ch_str.as_str()])
        .limit(limit.saturating_add(1) as usize);
    let agent_reply_filter = replies_filter.clone().author(agent_pubkey).limit(1);

    let context = fetch_with_retry(|| async {
        match timeout(
            CONTEXT_FETCH_TIMEOUT,
            query(vec![
                root_filter.clone(),
                replies_filter.clone(),
                agent_reply_filter.clone(),
            ]),
        )
        .await
        {
            Ok(Ok(json)) => {
                parse_nostr_thread_response_with_meta(json, root_event_id, limit, &agent_pubkey)
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    channel_id = %channel_id,
                    root = root_event_id,
                    "thread context fetch failed: {e} — will retry"
                );
                None
            }
            Err(_) => {
                tracing::warn!(
                    channel_id = %channel_id,
                    root = root_event_id,
                    "thread context fetch timed out — will retry"
                );
                None
            }
        }
    })
    .await;

    let mut parsed = context?;

    if matches!(
        parsed.context,
        ConversationContext::Thread {
            truncated: true,
            ..
        }
    ) {
        let replies_count_filter = replies_filter.clone().limit(0);
        if let Some(total) = fetch_thread_total(
            channel_id,
            &replies_count_filter,
            parsed.root_present,
            &count,
        )
        .await
        {
            if let ConversationContext::Thread {
                total: context_total,
                ..
            } = &mut parsed.context
            {
                let sentinel_minimum = *context_total;
                // `/count` is a separate best-effort request after the message
                // query. If replies are deleted between the two, the exact count
                // can fall below the already-proven sentinel minimum; never
                // render impossible labels like `13 of 12 messages, truncated`.
                *context_total = total.max(sentinel_minimum);
            }
        }
    }

    Some(parsed.context)
}

/// Best-effort exact thread size for truncated context labels.
async fn fetch_thread_total<Count, CountFut>(
    channel_id: Uuid,
    replies_filter: &nostr::Filter,
    root_present: bool,
    count: &Count,
) -> Option<usize>
where
    Count: Fn(Vec<nostr::Filter>) -> CountFut,
    CountFut: std::future::Future<Output = Result<serde_json::Value, crate::relay::RelayError>>,
{
    let replies_count =
        match timeout(CONTEXT_COUNT_TIMEOUT, count(vec![replies_filter.clone()])).await {
            Ok(Ok(json)) => json.get("count").and_then(|v| v.as_u64())?,
            Ok(Err(e)) => {
                tracing::debug!(
                    channel_id = %channel_id,
                    "thread context count failed; using sentinel minimum: {e}"
                );
                return None;
            }
            Err(_) => {
                tracing::debug!(
                    channel_id = %channel_id,
                    "thread context count timed out; using sentinel minimum"
                );
                return None;
            }
        };

    Some(replies_count as usize + usize::from(root_present))
}

/// Fetch DM context via Nostr query: recent messages in channel by `#h` tag.
async fn fetch_dm_context(
    channel_id: Uuid,
    limit: u32,
    rest: &RestClient,
) -> Option<ConversationContext> {
    use nostr::{Alphabet, SingleLetterTag};

    let h_tag = SingleLetterTag::lowercase(Alphabet::H);
    let ch_str = channel_id.to_string();
    let filter = nostr::Filter::new()
        .kinds([
            nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE as u16),
            nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE_V2 as u16),
        ])
        .custom_tags(h_tag, [ch_str.as_str()])
        .limit(limit as usize);

    fetch_with_retry(|| async {
        match timeout(
            CONTEXT_FETCH_TIMEOUT,
            rest.query(std::slice::from_ref(&filter)),
        )
        .await
        {
            Ok(Ok(json)) => parse_nostr_dm_response(json, limit),
            Ok(Err(e)) => {
                tracing::warn!(
                    channel_id = %channel_id,
                    "DM context fetch failed: {e} — will retry"
                );
                None
            }
            Err(_) => {
                tracing::warn!(
                    channel_id = %channel_id,
                    "DM context fetch timed out — will retry"
                );
                None
            }
        }
    })
    .await
}

/// Parse the legacy REST thread response (used in tests only).
#[cfg(test)]
fn parse_thread_response(json: serde_json::Value) -> Option<ConversationContext> {
    let mut messages = Vec::new();

    // Root message.
    if let Some(root) = json.get("root") {
        if let Some(msg) = json_to_context_message(root) {
            messages.push(msg);
        }
    }

    // Replies.
    if let Some(replies) = json.get("replies").and_then(|v| v.as_array()) {
        for reply in replies {
            if let Some(msg) = json_to_context_message(reply) {
                messages.push(msg);
            }
        }
    }

    let total_replies = json
        .get("total_replies")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let total = total_replies + 1; // +1 for root
    let truncated = total > messages.len();

    if messages.is_empty() {
        return None;
    }

    Some(ConversationContext::Thread {
        messages,
        total,
        truncated,
    })
}

/// Parse the DM messages REST response into a `ConversationContext::Dm`.
///
/// Parse the legacy REST DM response (used in tests only).
#[cfg(test)]
fn parse_dm_response(json: serde_json::Value, limit: u32) -> Option<ConversationContext> {
    let arr = json.get("messages").and_then(|v| v.as_array())?;

    let mut messages: Vec<ContextMessage> =
        arr.iter().filter_map(json_to_context_message).collect();

    // API returns newest-first; reverse to chronological for the prompt.
    messages.reverse();

    // The relay's next_cursor is always set when the page is non-empty (not
    // just when more pages exist), so we can't use it for truncation detection.
    // Instead, compare returned count against the requested limit.
    let truncated = messages.len() >= limit as usize;
    let total = if truncated {
        messages.len() + 1 // indicate there are more
    } else {
        messages.len()
    };

    if messages.is_empty() {
        return None;
    }

    Some(ConversationContext::Dm {
        messages,
        total,
        truncated,
    })
}

/// Extract a `ContextMessage` from a JSON message object.
///
/// Works with both thread reply objects and channel message objects.
fn json_to_context_message(obj: &serde_json::Value) -> Option<ContextMessage> {
    let content = obj.get("content").and_then(|v| v.as_str())?;
    let pubkey = obj
        .get("pubkey")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let timestamp = obj
        .get("created_at")
        .and_then(|v| {
            // Handle both string timestamps and integer timestamps.
            v.as_str().map(|s| s.to_string()).or_else(|| {
                v.as_i64().map(|ts| {
                    chrono::DateTime::from_timestamp(ts, 0)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_else(|| ts.to_string())
                })
            })
        })
        .unwrap_or_else(|| "unknown".to_string());

    Some(ContextMessage {
        pubkey: pubkey.to_string(),
        timestamp,
        content: content.to_string(),
    })
}

/// Parse a Nostr query response (array of events) into thread context.
///
/// Separates the root event (matching `root_event_id`) from replies, keeps the
/// newest `limit` replies returned by the sentinel query, then sorts the
/// displayed window chronologically for the prompt. If the agent's newest reply
/// is outside that window, keep it instead of the oldest displayed reply so the
/// next prompt always includes the agent's most recent prior turn.
#[cfg(test)]
fn parse_nostr_thread_response(
    json: serde_json::Value,
    root_event_id: &str,
    limit: u32,
    agent_pubkey: &nostr::PublicKey,
) -> Option<ConversationContext> {
    parse_nostr_thread_response_with_meta(json, root_event_id, limit, agent_pubkey)
        .map(|parsed| parsed.context)
}

struct ParsedThreadContext {
    context: ConversationContext,
    root_present: bool,
}

fn parse_nostr_thread_response_with_meta(
    json: serde_json::Value,
    root_event_id: &str,
    limit: u32,
    agent_pubkey: &nostr::PublicKey,
) -> Option<ParsedThreadContext> {
    let events = json.as_array()?;
    let agent_pubkey_hex = agent_pubkey.to_hex();
    let mut root_msg = None;
    let mut reply_msgs = Vec::new();
    let mut seen_reply_ids = HashSet::new();

    for ev in events {
        let ev_id = ev.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(msg) = json_to_context_message(ev) {
            if ev_id == root_event_id {
                root_msg = Some(msg);
            } else if seen_reply_ids.insert(ev_id.to_string()) {
                let is_agent = msg.pubkey.eq_ignore_ascii_case(&agent_pubkey_hex);
                reply_msgs.push((
                    ev_id.to_string(),
                    ev.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
                    is_agent,
                    msg,
                ));
            }
        }
    }

    let root_present = root_msg.is_some();
    let fetched_total = reply_msgs.len() + usize::from(root_present);
    let newest_agent_reply = reply_msgs
        .iter()
        .filter(|(_, _, is_agent, _)| *is_agent)
        .max_by_key(|(_, ts, _, _)| *ts)
        .cloned();

    let truncated = reply_msgs.len() > limit as usize;
    if truncated {
        // The relay returns limited REQ results newest-first. Sort explicitly so
        // the sentinel we drop is the oldest reply in the fetched window, not an
        // arbitrary last element if the HTTP bridge ever changes iteration order.
        reply_msgs.sort_by_key(|(_, ts, _, _)| Reverse(*ts));
        reply_msgs.truncate(limit as usize);
    }

    if let Some(agent_reply) = newest_agent_reply {
        let agent_reply_already_displayed =
            reply_msgs.iter().any(|(id, _, _, _)| *id == agent_reply.0);
        if !agent_reply_already_displayed {
            reply_msgs.sort_by_key(|(_, ts, _, _)| *ts);
            if let Some(oldest) = reply_msgs.first_mut() {
                *oldest = agent_reply;
            }
        }
    }

    // Sort displayed replies chronologically.
    reply_msgs.sort_by_key(|(_, ts, _, _)| *ts);

    let mut messages = Vec::new();
    if let Some(root) = root_msg {
        messages.push(root);
    }
    messages.extend(reply_msgs.into_iter().map(|(_, _, _, msg)| msg));

    if messages.is_empty() {
        return None;
    }

    let total = if truncated {
        fetched_total // all distinct fetched replies plus the root are proven visible history
    } else {
        messages.len()
    };

    Some(ParsedThreadContext {
        context: ConversationContext::Thread {
            messages,
            total,
            truncated,
        },
        root_present,
    })
}

/// Parse a Nostr query response (array of events) into DM context.
///
/// Events arrive in relay order (newest first); reversed to chronological.
fn parse_nostr_dm_response(json: serde_json::Value, limit: u32) -> Option<ConversationContext> {
    let events = json.as_array()?;

    let mut messages: Vec<(u64, ContextMessage)> = events
        .iter()
        .filter_map(|ev| {
            let ts = ev.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
            json_to_context_message(ev).map(|msg| (ts, msg))
        })
        .collect();

    // Sort chronologically (oldest first).
    messages.sort_by_key(|(ts, _)| *ts);

    let messages: Vec<ContextMessage> = messages.into_iter().map(|(_, msg)| msg).collect();
    let truncated = messages.len() >= limit as usize;
    let total = if truncated {
        messages.len() + 1
    } else {
        messages.len()
    };

    if messages.is_empty() {
        return None;
    }

    Some(ConversationContext::Dm {
        messages,
        total,
        truncated,
    })
}

/// Return the batch for requeue only in Queue mode; drop it in Drop mode.
#[inline]
fn requeue_batch_if_queue(ctx: &PromptContext, batch: Option<FlushBatch>) -> Option<FlushBatch> {
    match ctx.dedup_mode {
        DedupMode::Queue => batch,
        DedupMode::Drop => None,
    }
}

/// Map a cancelling [`ControlSignal`] to the [`CancelReason`] that should frame
/// the merged re-prompt, then requeue the batch (in `Queue` dedup mode) with
/// that reason stamped onto [`FlushBatch::cancel_reason`]. `Cancel`/`Rotate`
/// drop the batch entirely. The reason is consumed by the main loop at requeue
/// time (`requeue_as_cancelled`) and ultimately by `format_prompt`.
#[inline]
fn requeue_cancelled_batch(
    ctx: &PromptContext,
    signal: ControlSignal,
    batch: Option<FlushBatch>,
) -> Option<FlushBatch> {
    let reason = match signal {
        ControlSignal::Steer => CancelReason::Steer,
        ControlSignal::Interrupt | ControlSignal::SwitchModel(_) => CancelReason::Interrupt,
        // Cancel/Rotate discard the batch — no merged re-prompt.
        ControlSignal::Cancel | ControlSignal::Rotate => return None,
    };
    requeue_batch_if_queue(ctx, batch).map(|mut b| {
        b.cancel_reason = Some(reason);
        b
    })
}

/// Result of classifying a failed [`AcpClient::cancel_with_cleanup_grace`]
/// call: the [`PromptOutcome`] to report and the triggering batch's fate,
/// decided together so tests cross the exact error→outcome→batch-fate
/// boundary the production `Err(error)` arm uses.
struct ControlCancelFailure {
    outcome: PromptOutcome,
    retry_batch: Option<FlushBatch>,
    /// `AgentExited` invalidates every session on the agent; every other
    /// failure invalidates only the source that triggered this turn.
    invalidate_all: bool,
}

/// Classify a failed control-signal cancellation (steer fallback, interrupt,
/// or explicit stop) into the [`PromptOutcome`] to report and the triggering
/// batch's fate. This is the single production seam used by the `Err(error)`
/// arm of the control-cancel branch in [`run_prompt_task`] — the boundary
/// this exists to keep singular, so regressions there are regression-tested.
///
/// [`AcpError::CancelDrainTimeout`] is the expected, common case: the agent
/// didn't stop within its bounded grace window. [`AcpError::HardTimeout`] is
/// not expected here — [`AcpClient::cancel_with_cleanup_grace`] translates its
/// own drain-deadline `HardTimeout` into `CancelDrainTimeout` before
/// returning — but for defense in depth an unexpected `HardTimeout` at this
/// bounded cancellation boundary must never regain real hard-cap/dead-letter
/// classification, so it maps to `CancelDrainTimeout(CONTROL_CANCEL_GRACE)`
/// rather than `Timeout(Hard)`.
fn classify_control_cancel_failure(
    ctx: &PromptContext,
    error: AcpError,
    signal: ControlSignal,
    batch: Option<FlushBatch>,
) -> ControlCancelFailure {
    let (outcome, invalidate_all) = match error {
        AcpError::AgentExited => (PromptOutcome::AgentExited, true),
        AcpError::IdleTimeout(_) => (PromptOutcome::Timeout(TimeoutKind::Idle), false),
        AcpError::CancelDrainTimeout(grace) => (PromptOutcome::CancelDrainTimeout(grace), false),
        // Defense in depth: this bounded cancellation API is documented to
        // translate its own HardTimeout into CancelDrainTimeout, so this arm
        // should be unreachable in practice. If it ever fires anyway, still
        // report the truthful non-hard outcome rather than the real hard-cap
        // (which would dead-letter the batch and claim the configured cap).
        AcpError::HardTimeout { .. } => (
            PromptOutcome::CancelDrainTimeout(CONTROL_CANCEL_GRACE),
            false,
        ),
        other => (PromptOutcome::Error(other), false),
    };
    ControlCancelFailure {
        outcome,
        retry_batch: requeue_cancelled_batch(ctx, signal, batch),
        invalidate_all,
    }
}

/// How a turn's source is named in the `pool::prompt` log lines.
///
/// Shared by the turn-start and turn-stop lines so a log can be read as pairs.
fn prompt_label(source: &PromptSource) -> String {
    match source {
        PromptSource::Channel(cid) => format!("channel {cid}"),
        PromptSource::Heartbeat => "heartbeat".to_string(),
    }
}

/// Log a stop reason at the appropriate tracing level.
fn log_stop_reason(source: &PromptSource, stop_reason: &StopReason) {
    let label = prompt_label(source);
    match stop_reason {
        StopReason::EndTurn => {
            tracing::info!(target: "pool::prompt", "turn complete for {label}: end_turn");
        }
        StopReason::Cancelled => {
            tracing::warn!(target: "pool::prompt", "turn cancelled for {label}");
        }
        StopReason::MaxTokens => {
            tracing::warn!(target: "pool::prompt", "turn hit max_tokens for {label} — session will be rotated");
        }
        StopReason::MaxTurnRequests => {
            tracing::warn!(target: "pool::prompt", "turn hit max_turn_requests for {label} — session will be rotated");
        }
        StopReason::Refusal => {
            tracing::warn!(target: "pool::prompt", "turn refused for {label}");
        }
    }
}

//
// Two-phase lifecycle visible to users:
//   👀  "seen"    — event was queued and an agent will handle it
//   💬  "working" — agent is actively prompting
//
// 💬 is awaited inline in `run_prompt_task` before the prompt fires, so
// add-before-remove ordering is structural. 👀 is fire-and-forget from
// `main.rs` at queue-push time for immediate responsiveness; on rare
// fast-failure paths the guard's cleanup may race with the 👀 add,
// leaving a cosmetic stale 👀 (see `ReactionGuard` docs).
//
// Cleanup is fire-and-forget via `ReactionGuard` (spawned on drop).
// Failures are debug-logged and ignored — reactions are cosmetic.

/// Drop guard that spawns reaction cleanup on any exit path.
///
/// Created at the top of `run_prompt_task`. On drop — normal return, early
/// return, or panic — spawns fire-and-forget removal of both 👀 and 💬.
///
/// ## Ordering
///
/// 💬 (`react_working`) is fire-and-forget (spawned before the prompt fires).
/// A brief race where 💬 appears slightly after the agent starts is acceptable.
///
/// 👀 (`react_seen`) is fire-and-forget from `main.rs` at queue-push time.
/// On rare fast-failure paths (e.g., `session_new` error on an idle agent),
/// the cleanup spawn may race with the 👀 add, leaving a stale 👀. This is
/// accepted as a cosmetic edge case — the message will be retried and the
/// stale 👀 is harmless.
struct ReactionGuard {
    rest: Option<crate::relay::RestClient>,
    ids: Vec<String>,
}

impl ReactionGuard {
    fn new(rest: crate::relay::RestClient, ids: Vec<String>) -> Self {
        Self {
            rest: if ids.is_empty() { None } else { Some(rest) },
            ids,
        }
    }
}

impl Drop for ReactionGuard {
    fn drop(&mut self) {
        // Guard against drop outside a tokio runtime (e.g., in unit tests or
        // during process teardown before the runtime is fully initialized).
        // `run_prompt_task` is always spawned via `JoinSet::spawn`, so a
        // runtime handle is normally available; `try_current` is the safe
        // fallback for the rare cases it isn't.
        if let Some(rest) = self.rest.take() {
            let ids = std::mem::take(&mut self.ids);
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(clear_reactions(rest, ids));
            }
            // If no runtime is available, reactions are left as-is — they are
            // cosmetic indicators and the stale state is harmless.
        }
    }
}

// Periodically emits a `turn_liveness` observer event while a turn is in-flight,
// so the desktop can prune turns whose host died without unwinding (kill -9 /
// crash) far sooner than the no-activity backstop. `run_prompt_task` runs it in
// a background task from `turn_started` until `LivenessGuard` drops, covering
// session setup as well as the final prompt call. When `interval` is zero,
// liveness is disabled and the future parks forever without emitting.
//
// `state` is the other half of `LivenessGuard`'s shutdown mutex (see its
// docs): held here across the check-then-emit, so a `LivenessGuard::drop`
// racing an in-flight tick either observes `state.closed == true` and skips
// the emit, or is blocked on the same lock until this tick's emit has
// already landed. Either way `turn_completed` cannot pass a live
// `turn_liveness` frame on the wire — the race is closed, not narrowed.
//
// `context`'s `session_id` starts `None` (liveness begins before session
// creation) and is filled in from `state.session_id` on each tick — set once
// by `run_prompt_task` after session resolution — so pings emitted for the
// remainder of the turn carry the real session, matching every other
// observer frame for this turn instead of a permanent `None`.
async fn run_turn_liveness(
    observer: Option<observer::ObserverHandle>,
    agent_index: Option<usize>,
    mut context: observer::ObserverContext,
    interval: Duration,
    state: Arc<Mutex<LivenessState>>,
) {
    let Some(observer) = observer else {
        return std::future::pending::<()>().await;
    };
    if interval.is_zero() {
        return std::future::pending::<()>().await;
    }
    let mut ticker = tokio::time::interval(interval);
    // The first tick completes immediately; skip it so the first liveness ping
    // fires one interval after the turn starts, not at t=0 (turn_started already
    // marks t=0).
    ticker.tick().await;
    loop {
        ticker.tick().await;
        // Nothing awaitable between the lock and the emit: `LivenessGuard::drop`
        // takes this same lock before its `abort()`, so the guard can only ever
        // observe this tick fully emitted or not yet started — never mid-emit.
        let guard = match state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.closed {
            return;
        }
        context.session_id = guard.session_id.clone();
        observer.emit(
            "turn_liveness",
            agent_index,
            &context,
            serde_json::json!({}),
        );
        drop(guard);
    }
}

/// Shared shutdown/session state between `run_turn_liveness` and its
/// `LivenessGuard`. A single lock covers both fields so a tick's
/// check-session/emit and a guard's set-closed/abort can never interleave.
struct LivenessState {
    closed: bool,
    session_id: Option<String>,
}

/// Owns the background liveness task for one `run_prompt_task` invocation.
///
/// Dropping the guard aborts the non-resolving task, so liveness covers all
/// pre-prompt setup yet cannot survive a completed, cancelled, or panicked turn.
///
/// `abort()` alone leaves a race: tokio's cooperative cancellation only takes
/// effect at the next `.await` point inside the aborted task, so a tick that
/// has already passed its await and is mid-`observer.emit` when `drop` runs
/// can still complete that emit — a `turn_liveness` frame lands on the wire
/// after `turn_completed`, reviving a finished turn's badge for up to the
/// desktop's bounded prune-pause window. `state` shares a lock with
/// `run_turn_liveness`'s check-then-emit (see its docs): setting `closed`
/// and aborting under the same lock the emitter holds during its tick means
/// `drop` either sees the flag land before that tick's lock is taken (emit
/// skipped) or blocks until the in-flight emit under the lock has finished
/// (then aborts, so there is no next tick) — no interleaving emits a frame
/// after this guard has dropped.
struct LivenessGuard {
    handle: JoinHandle<()>,
    state: Arc<Mutex<LivenessState>>,
}

impl LivenessGuard {
    fn new(handle: JoinHandle<()>, state: Arc<Mutex<LivenessState>>) -> Self {
        Self { handle, state }
    }

    /// Record the turn's session ID once known, so subsequent liveness ticks
    /// stamp it on the emitted `turn_liveness` frame instead of `None`.
    fn set_session_id(&self, session_id: String) {
        let mut guard = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.session_id = Some(session_id);
    }
}

impl Drop for LivenessGuard {
    fn drop(&mut self) {
        {
            let mut guard = match self.state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            guard.closed = true;
        }
        self.handle.abort();
    }
}

// Emits a `turn_completed` observer event on drop, covering ALL exit paths
// (success, error, timeout, cancel, panic) from `run_prompt_task`. Captures
// observer handle and metadata at creation time so it remains valid even after
// the agent is moved into `PromptResult`.

struct TurnCompletionGuard {
    observer: Option<observer::ObserverHandle>,
    agent_index: Option<usize>,
    channel_id: Option<uuid::Uuid>,
    turn_id: String,
}

impl TurnCompletionGuard {
    fn new(
        observer: Option<observer::ObserverHandle>,
        agent_index: Option<usize>,
        channel_id: Option<uuid::Uuid>,
        turn_id: String,
    ) -> Self {
        Self {
            observer,
            agent_index,
            channel_id,
            turn_id,
        }
    }
}

impl Drop for TurnCompletionGuard {
    fn drop(&mut self) {
        if let Some(observer) = self.observer.take() {
            let context = observer::context_for(self.channel_id, None, Some(self.turn_id.clone()));
            observer.emit(
                "turn_completed",
                self.agent_index,
                &context,
                serde_json::json!({}),
            );
        }
    }
}

/// Map an ACP `StopReason` to the NIP-AM `StopReason` used in kind 44200 payloads.
fn acp_stop_to_core(r: &StopReason) -> buzz_core::agent_turn_metric::StopReason {
    use buzz_core::agent_turn_metric::StopReason as CoreStop;
    match r {
        StopReason::EndTurn => CoreStop::EndTurn,
        StopReason::Cancelled => CoreStop::Cancelled,
        StopReason::MaxTokens => CoreStop::MaxTokens,
        StopReason::MaxTurnRequests => CoreStop::Unknown,
        StopReason::Refusal => CoreStop::Unknown,
    }
}

/// Build the `(turn, cumulative)` `TokenCounts` pair for a NIP-AM kind-44200
/// payload from a completed `TurnUsage`.
///
/// Extracted as a pure function so the mapping logic can be tested independently
/// of relay/crypto infrastructure. `publish_agent_turn_metric` is the only
/// production caller.
///
/// - `turn` is `None` when `delta_reliable` is false; otherwise it carries the
///   per-turn i/o/total/cost deltas for this turn.
/// - `cumulative` always carries the session-aggregate i/o/cost totals.
///   `total_tokens` is `Some` only when the session accumulated a genuine
///   provider-reported total on every turn — never derived from i/o sums
///   (NIP-AM MUST NOT).
pub(crate) fn build_turn_metric_counts(
    usage: &crate::usage::TurnUsage,
) -> (
    Option<buzz_core::agent_turn_metric::TokenCounts>,
    Option<buzz_core::agent_turn_metric::TokenCounts>,
) {
    use buzz_core::agent_turn_metric::TokenCounts;

    let turn_counts = if usage.delta_reliable {
        Some(TokenCounts {
            input_tokens: usage.turn_input_tokens,
            output_tokens: usage.turn_output_tokens,
            // Field-local: present only when both the previous and current
            // cumulative totals were available and monotonic. Never derived
            // from input+output.
            total_tokens: usage.turn_total_tokens,
            cost_usd: usage.turn_cost_usd,
            // Field-local: present when the cumulative counter was monotonic
            // across this turn. Zero means no cache hits this turn (not absent).
            cache_read_tokens: usage.turn_cache_read_tokens,
            // buzz-agent does not emit a cache-write count on the wire today;
            // leave None rather than deriving it from other fields.
            cache_write_tokens: None,
        })
    } else {
        // Defense-in-depth: UsageTracker already sets all turn_* fields to None
        // when delta_reliable is false, so the None arm here is technically
        // redundant. The explicit guard prevents a future refactor from
        // accidentally publishing unreliable per-turn counts.
        None
    };
    let cumulative_counts = Some(TokenCounts {
        input_tokens: Some(usage.cumulative_input_tokens),
        output_tokens: Some(usage.cumulative_output_tokens),
        // Present when every turn in the session reported a genuine provider
        // total. None when the session has never emitted one or any turn lacked
        // one. Never derived from input+output (NIP-AM MUST NOT).
        total_tokens: usage.cumulative_total_tokens,
        cost_usd: usage.cumulative_cost_usd,
        // Session-cumulative cache-read tokens; None when the harness never
        // reported this field (e.g. goose or older buzz-agent sessions).
        // Passes through directly — do not wrap in Some() as the field already
        // carries provenance (None vs Some(0) are distinct meanings).
        cache_read_tokens: usage.cumulative_cache_read_tokens,
        // buzz-agent does not emit a cache-write count on the wire today;
        // leave None rather than deriving it from other fields.
        cache_write_tokens: None,
    });
    (turn_counts, cumulative_counts)
}

/// Best-effort: build and publish a `kind:44200` NIP-AM agent turn metric event.
///
/// Does nothing when `usage` is `None` (goose emitted no usage notification
/// for this turn) or when `owner_pubkey` is unconfigured (no NIP-AO identity).
/// Errors are logged at WARN and never surface to the caller — metric
/// publishing must never fail a turn.
async fn publish_agent_turn_metric(
    ctx: &PromptContext,
    usage: Option<crate::usage::TurnUsage>,
    channel_id: Option<uuid::Uuid>,
    session_id: &str,
    turn_id: &str,
    stop_reason: Option<buzz_core::agent_turn_metric::StopReason>,
) {
    use buzz_core::agent_turn_metric::AgentTurnMetricPayload;
    use nostr::{EventBuilder, Kind, Tag};

    let (usage, owner_pk) = match (usage, ctx.agent_owner_pubkey.as_ref()) {
        (Some(u), Some(pk)) => (u, pk),
        _ => return,
    };

    let (turn_counts, cumulative_counts) = build_turn_metric_counts(&usage);
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let payload = AgentTurnMetricPayload {
        harness: ctx.harness_name.clone(),
        model: usage.model.clone(),
        channel_id: channel_id.map(|id| id.to_string()),
        session_id: Some(usage.session_id.clone()),
        turn_id: Some(turn_id.to_string()),
        turn_seq: Some(usage.turn_seq),
        timestamp,
        turn: turn_counts,
        cumulative: cumulative_counts,
        delta_reliable: usage.delta_reliable,
        stop_reason,
    };
    let ciphertext = match buzz_core::agent_turn_metric::encrypt_agent_turn_metric(
        &ctx.agent_keys,
        owner_pk,
        &payload,
    ) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                target: "pool::metrics",
                session_id,
                turn_id,
                "NIP-AM: encrypt failed: {e}"
            );
            return;
        }
    };
    let agent_hex = ctx.agent_keys.public_key().to_hex();
    let owner_hex = owner_pk.to_hex();
    let event = match EventBuilder::new(
        Kind::Custom(buzz_core::kind::KIND_AGENT_TURN_METRIC as u16),
        ciphertext,
    )
    .tags([
        Tag::parse(["p", &owner_hex]).expect("p tag"),
        Tag::parse(["agent", &agent_hex]).expect("agent tag"),
    ])
    .sign_with_keys(&ctx.agent_keys)
    {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                target: "pool::metrics",
                session_id,
                turn_id,
                "NIP-AM: sign failed: {e}"
            );
            return;
        }
    };
    const METRIC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
    match tokio::time::timeout(METRIC_TIMEOUT, ctx.rest_client.submit_event(&event)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => tracing::warn!(
            target: "pool::metrics",
            session_id,
            turn_id,
            "NIP-AM: publish failed: {e}"
        ),
        Err(_) => tracing::warn!(
            target: "pool::metrics",
            session_id,
            turn_id,
            "NIP-AM: publish timed out"
        ),
    }
}

const REACTION_SEEN: &str = "👀";
const REACTION_WORKING: &str = "💬";

/// Best-effort timeout for a single reaction REST call.
const REACTION_TIMEOUT: Duration = Duration::from_millis(500);

/// Percent-encode a string for use in a URL path segment (used in tests only).
#[cfg(test)]
fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// Best-effort: add a reaction via a signed Nostr kind-7 event (NIP-25).
///
/// Builds a reaction event with `buzz_sdk::build_reaction`, signs it with
/// the keys already stored in `RestClient`, and submits via `POST /events`.
/// Returns immediately on timeout or any error — reactions are cosmetic.
pub(crate) async fn reaction_add(rest: &crate::relay::RestClient, event_id: &str, emoji: &str) {
    let target_id = match nostr::EventId::from_hex(event_id) {
        Ok(id) => id,
        Err(e) => {
            tracing::debug!(event_id, emoji, "reaction add: invalid event ID: {e}");
            return;
        }
    };
    let builder = match buzz_sdk::build_reaction(target_id, emoji) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(event_id, emoji, "reaction add: build failed: {e}");
            return;
        }
    };
    let event = match builder.sign_with_keys(&rest.keys) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(event_id, emoji, "reaction add: sign failed: {e}");
            return;
        }
    };
    match tokio::time::timeout(REACTION_TIMEOUT, rest.submit_event(&event)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => tracing::debug!(event_id, emoji, "reaction add failed: {e}"),
        Err(_) => tracing::debug!(event_id, emoji, "reaction add timed out"),
    }
}

/// Best-effort: post a visible failure notice (kind:9) to a channel after a
/// batch is dead-lettered. Replies into the thread of `thread_tags` when the
/// triggering event was threaded. Errors are logged and swallowed — the
/// notice must never take down the main loop.
pub(crate) async fn post_failure_notice(
    rest: &crate::relay::RestClient,
    channel_id: Uuid,
    thread_tags: &ThreadTags,
    content: &str,
) {
    let thread_ref = thread_tags.root_event_id.as_deref().and_then(|root| {
        let root_id = nostr::EventId::from_hex(root).ok()?;
        let parent_id = thread_tags
            .parent_event_id
            .as_deref()
            .and_then(|p| nostr::EventId::from_hex(p).ok())
            .unwrap_or(root_id);
        Some(buzz_sdk::ThreadRef {
            root_event_id: root_id,
            parent_event_id: parent_id,
        })
    });
    let builder =
        match buzz_sdk::build_message(channel_id, content, thread_ref.as_ref(), &[], false, &[]) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(channel = %channel_id, "failure notice: build failed: {e}");
                return;
            }
        };
    let event = match builder.sign_with_keys(&rest.keys) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(channel = %channel_id, "failure notice: sign failed: {e}");
            return;
        }
    };
    match tokio::time::timeout(Duration::from_secs(5), rest.submit_event(&event)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => tracing::warn!(channel = %channel_id, "failure notice failed: {e}"),
        Err(_) => tracing::warn!(channel = %channel_id, "failure notice timed out"),
    }
}

/// Best-effort: remove a reaction via a signed kind:5 (NIP-09) deletion event.
///
/// Queries kind:7 reactions by our pubkey targeting the event, finds the matching
/// emoji, then submits a signed kind:5 deletion via `POST /events`.
/// Returns immediately on timeout or any error — reactions are cosmetic.
pub(crate) async fn reaction_remove(rest: &crate::relay::RestClient, event_id: &str, emoji: &str) {
    use nostr::{Alphabet, SingleLetterTag};

    // Step 1: query our kind:7 reactions targeting this event.
    let my_pubkey = rest.keys.public_key();
    let e_tag = SingleLetterTag::lowercase(Alphabet::E);
    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Reaction)
        .author(my_pubkey)
        .custom_tags(e_tag, [event_id]);

    let resp = match tokio::time::timeout(Duration::from_millis(1_000), rest.query(&[filter])).await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::debug!(event_id, emoji, "reaction remove: query failed: {e}");
            return;
        }
        Err(_) => {
            tracing::debug!(event_id, emoji, "reaction remove: query timed out");
            return;
        }
    };

    // Find our reaction event with matching emoji content.
    let reid = resp.as_array().and_then(|events| {
        events.iter().find_map(|ev| {
            let content = ev.get("content")?.as_str()?;
            if content != emoji {
                return None;
            }
            ev.get("id")?.as_str().map(|s| s.to_string())
        })
    });

    let reid = match reid {
        Some(id) => id,
        None => {
            tracing::debug!(event_id, emoji, "reaction remove: no reaction event found");
            return;
        }
    };

    // Step 2: build and submit a signed kind:5 deletion for the reaction event.
    let target_id = match nostr::EventId::from_hex(&reid) {
        Ok(id) => id,
        Err(e) => {
            tracing::debug!(
                event_id,
                emoji,
                "reaction remove: invalid reaction event ID: {e}"
            );
            return;
        }
    };
    let builder = match buzz_sdk::build_remove_reaction(target_id) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(event_id, emoji, "reaction remove: build failed: {e}");
            return;
        }
    };
    let event = match builder.sign_with_keys(&rest.keys) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(event_id, emoji, "reaction remove: sign failed: {e}");
            return;
        }
    };
    match tokio::time::timeout(Duration::from_millis(1_000), rest.submit_event(&event)).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => tracing::debug!(event_id, emoji, "reaction remove failed: {e}"),
        Err(_) => tracing::debug!(event_id, emoji, "reaction remove timed out"),
    }
}

/// Maximum concurrent reaction HTTP requests per fan-out call.
/// Prevents unbounded parallelism when a large batch of events arrives.
const REACTION_CONCURRENCY: usize = 10;

/// Add 💬 to all events, capped at `REACTION_CONCURRENCY` concurrent requests.
/// Awaited inline before the prompt fires.
async fn react_working(rest: &crate::relay::RestClient, event_ids: &[String]) {
    for chunk in event_ids.chunks(REACTION_CONCURRENCY) {
        futures_util::future::join_all(
            chunk
                .iter()
                .map(|eid| reaction_add(rest, eid, REACTION_WORKING)),
        )
        .await;
    }
}

/// Fire-and-forget: remove both 👀 and 💬 from all events. Spawned on turn complete.
/// Capped at `REACTION_CONCURRENCY` concurrent requests per chunk to avoid
/// unbounded HTTP fan-out on large batches.
async fn clear_reactions(rest: crate::relay::RestClient, event_ids: Vec<String>) {
    // Each event needs two removals (👀 and 💬); pair them and chunk by
    // REACTION_CONCURRENCY pairs so the total concurrent requests stay bounded.
    for chunk in event_ids.chunks(REACTION_CONCURRENCY) {
        futures_util::future::join_all(chunk.iter().flat_map(|eid| {
            [
                reaction_remove(&rest, eid, REACTION_SEEN),
                reaction_remove(&rest, eid, REACTION_WORKING),
            ]
        }))
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use serde_json::json;

    // These pin the initial_message dispatch path (run_prompt_task, ~line 855):
    // a legacy agent WITH a base_prompt must get [Base] prepended to the user
    // message. This is the exact regression that shipped in the round-2 bug.

    fn test_mcp_server() -> McpServer {
        McpServer {
            name: "dev".into(),
            command: "buzz-dev-mcp".into(),
            args: vec![],
            env: vec![],
        }
    }

    #[test]
    fn public_session_forwards_channel_origin_to_mcp() {
        let channel_id = Uuid::new_v4();
        let servers = mcp_servers_with_git_origin(
            &[test_mcp_server()],
            Some(channel_id),
            Some("stream"),
            None,
        );
        assert!(servers[0].env.iter().any(|entry| {
            entry.name == "BUZZ_GIT_ORIGIN_CHANNEL_ID" && entry.value == channel_id.to_string()
        }));
        assert!(!servers[0]
            .env
            .iter()
            .any(|entry| entry.name == "BUZZ_GIT_ORIGIN_AGENT_NAME"));
    }

    #[test]
    fn private_session_forwards_agent_name_without_channel_id() {
        let servers = mcp_servers_with_git_origin(
            &[test_mcp_server()],
            Some(Uuid::new_v4()),
            Some("dm"),
            Some("Builder"),
        );
        assert!(servers[0].env.iter().any(|entry| {
            entry.name == "BUZZ_GIT_ORIGIN_AGENT_NAME" && entry.value == "Builder"
        }));
        assert!(!servers[0]
            .env
            .iter()
            .any(|entry| entry.name == "BUZZ_GIT_ORIGIN_CHANNEL_ID"));
    }

    #[test]
    fn test_initial_message_legacy_agent_gets_base_prepended() {
        // protocol_version 1 + Some(base_prompt): [Base] rides along in the
        // user message, composed as `[Base]\n{bp}\n\n{initial_msg}`.
        let composed = prepend_base_for_legacy(1, Some("you are a helpful agent"), "hello channel");
        assert_eq!(composed, "[Base]\nyou are a helpful agent\n\nhello channel");
        assert!(composed.starts_with("[Base]\nyou are a helpful agent\n\n"));
    }

    #[test]
    fn test_initial_message_modern_agent_omits_base() {
        // protocol_version 2 receives base_prompt via session/new, so the user
        // message is left untouched even when a base_prompt is present.
        let composed = prepend_base_for_legacy(2, Some("you are a helpful agent"), "hello channel");
        assert_eq!(composed, "hello channel");
    }

    #[test]
    fn goose_uses_system_prompt_only_after_custom_method_succeeds() {
        assert!(!has_system_prompt_support(2, "goose", None));
        assert!(!has_system_prompt_support(2, "goose", Some(false)));
        assert!(has_system_prompt_support(2, "goose", Some(true)));
        assert!(has_system_prompt_support(1, "goose", Some(true)));
        assert!(has_system_prompt_support(2, "buzz-agent", None));
        // Goose never receives system prompt via session/new (uses post-hoc method).
        assert_eq!(
            session_new_system_prompt(true, 2, "goose", Some("instructions")),
            None
        );
        // Protocol-v2 non-goose gets Field transport.
        assert_eq!(
            session_new_system_prompt(false, 2, "buzz-agent", Some("instructions")),
            Some(SystemPromptTransport::Field("instructions"))
        );
        // Protocol-v1 non-goose, non-claude gets None (legacy user-message framing).
        assert_eq!(
            session_new_system_prompt(false, 1, "codex", Some("instructions")),
            None
        );
        // claude-agent-acp gets ClaudeMeta transport regardless of protocol version.
        assert_eq!(
            session_new_system_prompt(false, 1, CLAUDE_AGENT_ACP_NAME, Some("instructions")),
            Some(SystemPromptTransport::ClaudeMeta("instructions"))
        );
        assert_eq!(
            session_new_system_prompt(true, 1, CLAUDE_AGENT_ACP_NAME, Some("instructions")),
            None,
            "goose path must never produce a transport even when agent_name matches"
        );
    }

    #[test]
    fn claude_agent_acp_has_system_prompt_support_regardless_of_protocol_version() {
        // claude-agent-acp declares protocolVersion:1 but supports _meta.systemPrompt;
        // has_system_prompt_support must return true so user-message framing is suppressed.
        assert!(has_system_prompt_support(1, CLAUDE_AGENT_ACP_NAME, None));
        assert!(has_system_prompt_support(2, CLAUDE_AGENT_ACP_NAME, None));
    }

    #[test]
    fn old_zed_adapter_name_falls_through_to_protocol_version_gate() {
        // The renamed @zed-industries package predates the _meta.systemPrompt support,
        // so it must not be treated as capable and stays on legacy user-message framing.
        let old_name = "@zed-industries/claude-code-acp";
        assert!(!has_system_prompt_support(1, old_name, None));
        assert!(has_system_prompt_support(2, old_name, None));
    }

    #[test]
    fn test_initial_message_legacy_agent_without_base_is_unchanged() {
        // No base_prompt configured: nothing to prepend regardless of version.
        let composed = prepend_base_for_legacy(1, None, "hello channel");
        assert_eq!(composed, "hello channel");
    }

    // Pin the session/new systemPrompt framing: each present prompt carries its
    // own header so the desktop observer can split into labeled sub-sections.

    #[test]
    fn test_framed_system_prompt_both_present_carries_both_headers() {
        // Also the regression guard against #2372: the session title travels
        // out of band in `_meta.sessionTitle`, so this exact-bytes assertion is
        // what pins the framing against a `[Session]` section reappearing here.
        let framed = framed_system_prompt("/", Some("base text"), Some("persona text"))
            .expect("both present yields Some");
        assert_eq!(framed, "[Base]\nbase text\n\n[System]\npersona text");
    }

    #[test]
    fn test_framed_system_prompt_base_only_labels_base() {
        let framed = framed_system_prompt("/", Some("base text"), None).expect("base yields Some");
        assert_eq!(framed, "[Base]\nbase text");
    }

    #[test]
    fn test_framed_system_prompt_persona_only_labels_system() {
        // A bare persona would be mislabeled "Base" downstream — it must carry
        // its own [System] header even when no base prompt exists.
        let framed =
            framed_system_prompt("/", None, Some("persona text")).expect("persona yields Some");
        assert_eq!(framed, "[System]\npersona text");
    }

    #[test]
    fn test_framed_system_prompt_neither_is_none() {
        assert!(framed_system_prompt("/", None, None).is_none());
    }

    #[test]
    fn test_framed_system_prompt_absolute_cwd_prepends_workspace_before_base() {
        let framed = framed_system_prompt("/Users/me/.buzz", Some("base text"), None)
            .expect("base yields Some");
        assert!(
            framed.starts_with("[Workspace]\n"),
            "workspace section must lead: {framed}"
        );
        assert!(framed.contains("`/Users/me/.buzz`"));
        assert!(
            framed.contains("\n\n[Base]\nbase text"),
            "base must follow the workspace section: {framed}"
        );
    }

    #[test]
    fn test_framed_system_prompt_persona_only_omits_workspace() {
        // The workspace section grounds the base prompt's layout; a persona-only
        // agent never received that layout, so no [Workspace] anchor is emitted.
        let framed = framed_system_prompt("/Users/me/.buzz", None, Some("persona text"))
            .expect("persona yields Some");
        assert_eq!(framed, "[System]\npersona text");
    }

    #[test]
    fn test_framed_system_prompt_root_cwd_omits_workspace() {
        // The "/" fallback must never be named — it would invite a $HOME scan.
        let framed = framed_system_prompt("/", Some("base text"), None).expect("base yields Some");
        assert_eq!(framed, "[Base]\nbase text");
    }

    #[test]
    fn test_workspace_section_relative_cwd_is_none() {
        assert!(workspace_section("relative/path").is_none());
        assert!(workspace_section("").is_none());
    }

    #[test]
    fn test_with_core_appends_below_framed() {
        let framed = with_core(
            Some("[System]\npersona".to_string()),
            Some("[Agent Memory — core]\nbe helpful"),
        )
        .expect("both present yields Some");
        assert_eq!(
            framed,
            "[System]\npersona\n\n[Agent Memory — core]\nbe helpful"
        );
    }

    #[test]
    fn test_with_core_framed_only_passes_through() {
        let framed = with_core(Some("[System]\npersona".to_string()), None)
            .expect("framed-only yields Some");
        assert_eq!(framed, "[System]\npersona");
    }

    #[test]
    fn test_with_core_core_only_is_just_core() {
        let framed = with_core(None, Some("[Agent Memory — core]\nbe helpful"))
            .expect("core-only yields Some");
        assert_eq!(framed, "[Agent Memory — core]\nbe helpful");
    }

    #[test]
    fn test_with_core_neither_is_none() {
        assert!(with_core(None, None).is_none());
    }

    #[test]
    fn test_parse_thread_response_basic() {
        let json = json!({
            "root": {
                "event_id": "abc123",
                "pubkey": "pub1",
                "content": "root message",
                "created_at": 1710518400
            },
            "replies": [
                {
                    "event_id": "def456",
                    "pubkey": "pub2",
                    "content": "first reply",
                    "created_at": 1710518460
                }
            ],
            "total_replies": 1
        });

        let ctx = parse_thread_response(json).expect("should parse");
        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert_eq!(messages.len(), 2); // root + 1 reply
                assert_eq!(total, 2); // 1 reply + 1 root
                assert!(!truncated);
                assert_eq!(messages[0].content, "root message");
                assert_eq!(messages[1].content, "first reply");
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[test]
    fn test_parse_thread_response_truncated() {
        let json = json!({
            "root": {
                "event_id": "abc",
                "pubkey": "pub1",
                "content": "root",
                "created_at": 1710518400
            },
            "replies": [
                {
                    "event_id": "def",
                    "pubkey": "pub2",
                    "content": "reply1",
                    "created_at": 1710518460
                }
            ],
            "total_replies": 10
        });

        let ctx = parse_thread_response(json).expect("should parse");
        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert_eq!(messages.len(), 2);
                assert_eq!(total, 11); // 10 replies + 1 root
                assert!(truncated);
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[test]
    fn test_parse_thread_response_empty() {
        let json = json!({
            "root": null,
            "replies": [],
            "total_replies": 0
        });
        assert!(parse_thread_response(json).is_none());
    }

    #[test]
    fn test_parse_thread_response_missing_fields() {
        // Malformed JSON — no root, no replies key.
        let json = json!({ "something": "else" });
        assert!(parse_thread_response(json).is_none());
    }

    #[test]
    fn test_parse_dm_response_basic() {
        let json = json!({
            "messages": [
                {
                    "event_id": "msg2",
                    "pubkey": "pub2",
                    "content": "newer message",
                    "created_at": 1710518500
                },
                {
                    "event_id": "msg1",
                    "pubkey": "pub1",
                    "content": "older message",
                    "created_at": 1710518400
                }
            ],
            "next_cursor": null
        });

        // limit=12 > 2 messages → not truncated.
        let ctx = parse_dm_response(json, 12).expect("should parse");
        match ctx {
            ConversationContext::Dm {
                messages,
                total,
                truncated,
            } => {
                // Should be reversed to chronological order.
                assert_eq!(messages.len(), 2);
                assert_eq!(messages[0].content, "older message");
                assert_eq!(messages[1].content, "newer message");
                assert!(!truncated);
                assert_eq!(total, 2);
            }
            _ => panic!("expected Dm context"),
        }
    }

    #[test]
    fn test_parse_dm_response_truncated() {
        let json = json!({
            "messages": [
                {
                    "event_id": "msg1",
                    "pubkey": "pub1",
                    "content": "message",
                    "created_at": 1710518400
                }
            ],
            "next_cursor": "00000000660f5a80"
        });

        // limit=1 == 1 message → truncated.
        let ctx = parse_dm_response(json, 1).expect("should parse");
        match ctx {
            ConversationContext::Dm {
                truncated, total, ..
            } => {
                assert!(truncated);
                assert_eq!(total, 2); // 1 message + indicator
            }
            _ => panic!("expected Dm context"),
        }
    }

    #[test]
    fn test_parse_dm_response_not_truncated_despite_cursor() {
        // Relay always sets next_cursor when page is non-empty, but if
        // returned count < limit, the page is complete.
        let json = json!({
            "messages": [
                {
                    "event_id": "msg1",
                    "pubkey": "pub1",
                    "content": "only message",
                    "created_at": 1710518400
                }
            ],
            "next_cursor": "00000000660f5a80"
        });

        // limit=12 > 1 message → NOT truncated despite next_cursor being set.
        let ctx = parse_dm_response(json, 12).expect("should parse");
        match ctx {
            ConversationContext::Dm {
                truncated, total, ..
            } => {
                assert!(!truncated, "should not be truncated when count < limit");
                assert_eq!(total, 1);
            }
            _ => panic!("expected Dm context"),
        }
    }

    #[test]
    fn test_parse_dm_response_empty() {
        let json = json!({
            "messages": [],
            "next_cursor": null
        });
        assert!(parse_dm_response(json, 12).is_none());
    }

    #[test]
    fn test_parse_dm_response_missing_messages_key() {
        let json = json!({ "data": [] });
        assert!(parse_dm_response(json, 12).is_none());
    }

    #[test]
    fn test_parse_nostr_thread_response_marks_query_window_truncated() {
        let agent = Keys::generate();
        let root_id = "1111111111111111111111111111111111111111111111111111111111111111";
        let agent_hex = agent.public_key().to_hex();
        let json = json!([
            {
                "id": root_id,
                "pubkey": "rootpub",
                "content": "root",
                "created_at": 1000
            },
            {
                "id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "pubkey": agent_hex,
                "content": "newest agent reply",
                "created_at": 4000
            },
            {
                "id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "pubkey": "humanpub",
                "content": "middle reply",
                "created_at": 3000
            },
            {
                "id": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "pubkey": "oldpub",
                "content": "sentinel omitted reply",
                "created_at": 2000
            }
        ]);

        let ctx = parse_nostr_thread_response(json, root_id, 2, &agent.public_key())
            .expect("should parse");
        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert_eq!(messages.len(), 3); // root + 2 displayed replies
                assert_eq!(total, 4); // root + displayed replies + sentinel
                assert!(truncated);
                assert_eq!(messages[0].content, "root");
                assert_eq!(messages[1].content, "middle reply");
                assert_eq!(messages[2].content, "newest agent reply");
                assert!(messages
                    .iter()
                    .all(|msg| msg.content != "sentinel omitted reply"));
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[test]
    fn test_parse_nostr_thread_response_not_truncated_below_limit() {
        let agent = Keys::generate();
        let root_id = "1111111111111111111111111111111111111111111111111111111111111111";
        let json = json!([
            {
                "id": root_id,
                "pubkey": "rootpub",
                "content": "root",
                "created_at": 1000
            },
            {
                "id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "pubkey": "replypub",
                "content": "reply",
                "created_at": 2000
            }
        ]);

        let ctx = parse_nostr_thread_response(json, root_id, 2, &agent.public_key())
            .expect("should parse");
        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert_eq!(messages.len(), 2);
                assert_eq!(total, 2);
                assert!(!truncated);
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[test]
    fn test_parse_nostr_thread_response_keeps_agent_reply_outside_recent_window() {
        let agent = Keys::generate();
        let root_id = "1111111111111111111111111111111111111111111111111111111111111111";
        let agent_hex = agent.public_key().to_hex();
        let json = json!([
            {
                "id": root_id,
                "pubkey": "rootpub",
                "content": "root",
                "created_at": 1000
            },
            {
                "id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "pubkey": "humanpub",
                "content": "newer human reply",
                "created_at": 5000
            },
            {
                "id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "pubkey": "humanpub",
                "content": "middle human reply",
                "created_at": 4000
            },
            {
                "id": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "pubkey": "humanpub",
                "content": "oldest displayed reply without agent pin",
                "created_at": 3000
            },
            {
                "id": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                "pubkey": agent_hex,
                "content": "agent reply outside recent window",
                "created_at": 2000
            }
        ]);

        let ctx = parse_nostr_thread_response(json, root_id, 2, &agent.public_key())
            .expect("should parse");
        match ctx {
            ConversationContext::Thread { messages, .. } => {
                assert_eq!(messages.len(), 3); // root + 2 displayed replies
                assert_eq!(messages[0].content, "root");
                assert!(messages
                    .iter()
                    .any(|msg| msg.content == "agent reply outside recent window"));
                assert!(messages
                    .iter()
                    .any(|msg| msg.content == "newer human reply"));
                assert!(messages
                    .iter()
                    .all(|msg| msg.content != "middle human reply"));
                assert!(messages
                    .iter()
                    .all(|msg| msg.content != "oldest displayed reply without agent pin"));
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[tokio::test]
    async fn test_fetch_thread_context_uses_exact_count_when_above_sentinel_minimum() {
        let agent = Keys::generate();
        let root_id = "1111111111111111111111111111111111111111111111111111111111111111";
        let channel_id = Uuid::new_v4();
        let agent_pubkey = agent.public_key();
        let json = json!([
            thread_event(root_id, "rootpub", "root", 1000),
            thread_event(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "humanpub",
                "newest reply",
                4000
            ),
            thread_event(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "humanpub",
                "middle reply",
                3000
            ),
            thread_event(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "humanpub",
                "sentinel reply",
                2000
            )
        ]);

        let ctx = fetch_thread_context_with(
            channel_id,
            root_id,
            2,
            agent_pubkey,
            move |filters| {
                assert_thread_query_filters(&filters, channel_id, root_id, agent_pubkey, 3);
                std::future::ready(Ok(json.clone()))
            },
            move |filters| {
                assert_thread_count_filter(&filters, channel_id, root_id);
                std::future::ready(Ok(json!({ "count": 6 })))
            },
        )
        .await
        .expect("thread context");

        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert!(truncated);
                assert_eq!(messages.len(), 3);
                assert_eq!(total, 7); // 6 replies + root
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[tokio::test]
    async fn test_fetch_thread_context_does_not_add_missing_root_to_exact_count() {
        let agent = Keys::generate();
        let root_id = "1111111111111111111111111111111111111111111111111111111111111111";
        let channel_id = Uuid::new_v4();
        let json = json!([
            thread_event(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "humanpub",
                "newest reply",
                4000
            ),
            thread_event(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "humanpub",
                "middle reply",
                3000
            ),
            thread_event(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "humanpub",
                "sentinel reply",
                2000
            )
        ]);

        let ctx = fetch_thread_context_with(
            channel_id,
            root_id,
            2,
            agent.public_key(),
            move |_filters| std::future::ready(Ok(json.clone())),
            |_filters| std::future::ready(Ok(json!({ "count": 6 }))),
        )
        .await
        .expect("thread context");

        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert!(truncated);
                assert_eq!(messages.len(), 2);
                assert_eq!(total, 6);
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[tokio::test]
    async fn test_fetch_thread_context_clamps_count_below_sentinel_minimum() {
        let agent = Keys::generate();
        let root_id = "1111111111111111111111111111111111111111111111111111111111111111";
        let channel_id = Uuid::new_v4();
        let json = json!([
            thread_event(root_id, "rootpub", "root", 1000),
            thread_event(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "humanpub",
                "newest reply",
                4000
            ),
            thread_event(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "humanpub",
                "middle reply",
                3000
            ),
            thread_event(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "humanpub",
                "sentinel reply",
                2000
            )
        ]);

        let ctx = fetch_thread_context_with(
            channel_id,
            root_id,
            2,
            agent.public_key(),
            move |_filters| std::future::ready(Ok(json.clone())),
            |_filters| std::future::ready(Ok(json!({ "count": 1 }))),
        )
        .await
        .expect("thread context");

        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert!(truncated);
                assert_eq!(messages.len(), 3);
                assert_eq!(total, 4); // root + displayed replies + sentinel minimum
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[tokio::test]
    async fn test_fetch_thread_context_preserves_sentinel_minimum_when_count_fails() {
        let agent = Keys::generate();
        let root_id = "1111111111111111111111111111111111111111111111111111111111111111";
        let channel_id = Uuid::new_v4();
        let json = json!([
            thread_event(root_id, "rootpub", "root", 1000),
            thread_event(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "humanpub",
                "newest reply",
                4000
            ),
            thread_event(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "humanpub",
                "middle reply",
                3000
            ),
            thread_event(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "humanpub",
                "sentinel reply",
                2000
            )
        ]);

        let ctx = fetch_thread_context_with(
            channel_id,
            root_id,
            2,
            agent.public_key(),
            move |_filters| std::future::ready(Ok(json.clone())),
            |_filters| std::future::ready(Err(crate::relay::RelayError::Http("boom".into()))),
        )
        .await
        .expect("thread context");

        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert!(truncated);
                assert_eq!(messages.len(), 3);
                assert_eq!(total, 4); // count failure leaves parser's sentinel minimum intact
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[tokio::test]
    async fn test_fetch_thread_context_deduplicates_and_pins_agent_reply() {
        let agent = Keys::generate();
        let agent_hex = agent.public_key().to_hex();
        let root_id = "1111111111111111111111111111111111111111111111111111111111111111";
        let channel_id = Uuid::new_v4();
        let json = json!([
            thread_event(root_id, "rootpub", "root", 1000),
            thread_event(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "humanpub",
                "newer human reply",
                5000
            ),
            thread_event(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "humanpub",
                "middle human reply",
                4000
            ),
            thread_event(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                &agent_hex,
                "agent reply outside recent window",
                2000
            ),
            // Same event as the separately fetched author-filtered result; the
            // parser should deduplicate it before pinning.
            thread_event(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                &agent_hex,
                "agent reply outside recent window",
                2000
            )
        ]);

        let ctx = fetch_thread_context_with(
            channel_id,
            root_id,
            2,
            agent.public_key(),
            move |_filters| std::future::ready(Ok(json.clone())),
            |_filters| std::future::ready(Ok(json!({ "count": 3 }))),
        )
        .await
        .expect("thread context");

        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert!(truncated);
                assert_eq!(total, 4);
                assert_eq!(messages.len(), 3);
                assert_eq!(
                    messages
                        .iter()
                        .filter(|msg| msg.content == "agent reply outside recent window")
                        .count(),
                    1,
                    "separate agent-reply query must not duplicate the same event"
                );
                assert!(messages
                    .iter()
                    .any(|msg| msg.content == "newer human reply"));
                assert!(messages
                    .iter()
                    .all(|msg| msg.content != "middle human reply"));
            }
            _ => panic!("expected Thread context"),
        }
    }

    #[tokio::test]
    async fn test_fetch_thread_context_uses_distinct_fetched_replies_as_minimum() {
        let agent = Keys::generate();
        let agent_hex = agent.public_key().to_hex();
        let root_id = "1111111111111111111111111111111111111111111111111111111111111111";
        let channel_id = Uuid::new_v4();
        let json = json!([
            thread_event(root_id, "rootpub", "root", 1000),
            thread_event(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "humanpub",
                "newest human reply",
                5000
            ),
            thread_event(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "humanpub",
                "middle human reply",
                4000
            ),
            thread_event(
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "humanpub",
                "sentinel human reply",
                3000
            ),
            thread_event(
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                &agent_hex,
                "older distinct agent reply",
                2000
            )
        ]);

        let ctx = fetch_thread_context_with(
            channel_id,
            root_id,
            2,
            agent.public_key(),
            move |_filters| std::future::ready(Ok(json.clone())),
            |_filters| std::future::ready(Err(crate::relay::RelayError::Http("boom".into()))),
        )
        .await
        .expect("thread context");

        match ctx {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert!(truncated);
                assert_eq!(messages.len(), 3);
                assert_eq!(
                    total, 5,
                    "root plus all four distinct fetched replies prove the lower bound"
                );
                assert!(messages
                    .iter()
                    .any(|msg| msg.content == "older distinct agent reply"));
                assert!(messages
                    .iter()
                    .any(|msg| msg.content == "newest human reply"));
                assert!(messages
                    .iter()
                    .all(|msg| msg.content != "middle human reply"));
                assert!(messages
                    .iter()
                    .all(|msg| msg.content != "sentinel human reply"));
            }
            _ => panic!("expected Thread context"),
        }
    }

    fn assert_thread_query_filters(
        filters: &[nostr::Filter],
        channel_id: Uuid,
        root_id: &str,
        agent_pubkey: nostr::PublicKey,
        reply_limit: u64,
    ) {
        assert_eq!(
            filters.len(),
            3,
            "root, recent replies, and agent reply filters"
        );

        let root = serde_json::to_value(&filters[0]).expect("serialize root filter");
        assert_eq!(root.get("ids"), Some(&json!([root_id])));
        assert!(root.get("limit").is_none());

        let replies = serde_json::to_value(&filters[1]).expect("serialize replies filter");
        assert_eq!(replies.get("kinds"), Some(&json!([9, 40002])));
        assert_eq!(replies.get("#e"), Some(&json!([root_id])));
        assert_eq!(replies.get("#h"), Some(&json!([channel_id.to_string()])));
        assert_eq!(replies.get("limit"), Some(&json!(reply_limit)));
        assert!(replies.get("authors").is_none());

        let agent = serde_json::to_value(&filters[2]).expect("serialize agent filter");
        assert_eq!(agent.get("kinds"), Some(&json!([9, 40002])));
        assert_eq!(agent.get("#e"), Some(&json!([root_id])));
        assert_eq!(agent.get("#h"), Some(&json!([channel_id.to_string()])));
        assert_eq!(agent.get("authors"), Some(&json!([agent_pubkey.to_hex()])));
        assert_eq!(agent.get("limit"), Some(&json!(1)));
    }

    fn assert_thread_count_filter(filters: &[nostr::Filter], channel_id: Uuid, root_id: &str) {
        assert_eq!(filters.len(), 1, "count should query only matching replies");

        let count = serde_json::to_value(&filters[0]).expect("serialize count filter");
        assert_eq!(count.get("kinds"), Some(&json!([9, 40002])));
        assert_eq!(count.get("#e"), Some(&json!([root_id])));
        assert_eq!(count.get("#h"), Some(&json!([channel_id.to_string()])));
        assert_eq!(count.get("limit"), Some(&json!(0)));
        assert!(count.get("ids").is_none());
        assert!(count.get("authors").is_none());
    }

    fn thread_event(id: &str, pubkey: &str, content: &str, created_at: u64) -> serde_json::Value {
        json!({
            "id": id,
            "pubkey": pubkey,
            "content": content,
            "created_at": created_at
        })
    }

    #[test]
    fn test_json_to_context_message_integer_timestamp() {
        let obj = json!({
            "pubkey": "abc",
            "content": "hello",
            "created_at": 1710518400
        });
        let msg = json_to_context_message(&obj).expect("should parse");
        assert_eq!(msg.pubkey, "abc");
        assert_eq!(msg.content, "hello");
        assert!(msg.timestamp.contains("2024")); // 1710518400 = 2024-03-15
    }

    #[test]
    fn test_json_to_context_message_string_timestamp() {
        let obj = json!({
            "pubkey": "abc",
            "content": "hello",
            "created_at": "2026-03-15T16:30:00+00:00"
        });
        let msg = json_to_context_message(&obj).expect("should parse");
        assert_eq!(msg.timestamp, "2026-03-15T16:30:00+00:00");
    }

    #[test]
    fn test_json_to_context_message_missing_content() {
        let obj = json!({ "pubkey": "abc" });
        assert!(json_to_context_message(&obj).is_none());
    }

    #[test]
    fn test_collect_prompt_pubkeys_includes_authors_mentions_and_context() {
        let keys = Keys::generate();
        let p_tag = Tag::parse([
            "p",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
        .unwrap();
        let event = EventBuilder::new(Kind::Custom(9), "hello")
            .tags([p_tag])
            .sign_with_keys(&keys)
            .unwrap();
        let author_hex = event.pubkey.to_hex();
        let batch = FlushBatch {
            channel_id: Uuid::new_v4(),
            events: vec![crate::queue::BatchEvent {
                event,
                prompt_tag: "@mention".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };
        let context = ConversationContext::Thread {
            messages: vec![ContextMessage {
                pubkey: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                timestamp: "2026-03-25T05:51:25Z".into(),
                content: "follow up".into(),
            }],
            total: 1,
            truncated: false,
        };

        let pubkeys = collect_prompt_pubkeys(&batch, Some(&context));

        let mut expected = vec![
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            author_hex,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        ];
        expected.sort();

        assert_eq!(pubkeys, expected);
    }

    #[test]
    fn test_parse_kind0_profile_lookup_extracts_display_name_and_nip05() {
        let lookup = parse_kind0_profile_lookup(json!([
            {
                "id": "0000000000000000000000000000000000000000000000000000000000000001",
                "pubkey": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "kind": 0,
                "content": "{\"display_name\":\"Wes\",\"nip05\":\"wes@example.com\"}",
                "created_at": 1000,
                "tags": [],
                "sig": "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            }
        ]))
        .expect("lookup should parse");

        assert_eq!(
            lookup.get("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            Some(&PromptProfile {
                display_name: Some("Wes".into()),
                nip05_handle: Some("wes@example.com".into()),
                is_agent: false,
            })
        );
    }

    #[test]
    fn test_profile_event_is_agent_detects_nip_oa_auth_tag() {
        // Agent profile carries a 4-element NIP-OA ["auth", owner, cond, sig] tag.
        let agent_ev = json!({
            "pubkey": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "tags": [["auth", "owner_pk", "conditions", "sig"]],
        });
        assert!(profile_event_is_agent(&agent_ev));

        // Human profile: no auth tag.
        let human_ev = json!({ "pubkey": "bbbb", "tags": [["t", "topic"]] });
        assert!(!profile_event_is_agent(&human_ev));

        // Empty / missing tags → not an agent.
        assert!(!profile_event_is_agent(&json!({ "tags": [] })));
        assert!(!profile_event_is_agent(&json!({})));

        // Malformed auth tag (wrong arity) → not treated as an agent.
        let malformed = json!({ "tags": [["auth", "owner_pk"]] });
        assert!(!profile_event_is_agent(&malformed));
    }

    #[test]
    fn test_parse_kind0_profile_lookup_returns_none_for_empty() {
        assert!(parse_kind0_profile_lookup(json!([])).is_none());
        assert!(parse_kind0_profile_lookup(json!({})).is_none());
    }

    #[test]
    fn test_json_to_context_message_missing_pubkey_uses_default() {
        let obj = json!({ "content": "hello" });
        let msg = json_to_context_message(&obj).expect("should parse");
        assert_eq!(msg.pubkey, "unknown");
    }

    #[test]
    fn test_pct_encode_hex_passthrough() {
        let hex = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        assert_eq!(pct_encode(hex), hex);
    }

    #[test]
    fn test_pct_encode_emoji() {
        // 👀 = U+1F440 = F0 9F 91 80 in UTF-8
        assert_eq!(pct_encode("👀"), "%F0%9F%91%80");
    }

    #[test]
    fn test_pct_encode_emoji_speech_balloon() {
        // 💬 = U+1F4AC = F0 9F 92 AC in UTF-8
        assert_eq!(pct_encode("💬"), "%F0%9F%92%AC");
    }

    #[test]
    fn test_pct_encode_empty() {
        assert_eq!(pct_encode(""), "");
    }

    #[test]
    fn test_pct_encode_unreserved_passthrough() {
        assert_eq!(pct_encode("AZaz09-_.~"), "AZaz09-_.~");
    }

    #[test]
    fn test_pct_encode_reserved_chars() {
        assert_eq!(pct_encode("/"), "%2F");
        assert_eq!(pct_encode("+"), "%2B");
        assert_eq!(pct_encode(" "), "%20");
    }

    fn make_state() -> (SessionState, Uuid, Uuid) {
        let ch_a = Uuid::new_v4();
        let ch_b = Uuid::new_v4();
        let mut s = SessionState::default();
        s.sessions.insert(ch_a, "sess-a".into());
        s.sessions.insert(ch_b, "sess-b".into());
        s.turn_counts.insert(ch_a, 5);
        s.turn_counts.insert(ch_b, 3);
        s.core_sections.insert(ch_a, "core-a".into());
        s.core_sections.insert(ch_b, "core-b".into());
        s.heartbeat_session = Some("sess-hb".into());
        s.heartbeat_turn_count = 7;
        (s, ch_a, ch_b)
    }

    #[test]
    fn test_rotate_after_natural_completion_invalidates_channel_state() {
        let (mut s, ch_a, ch_b) = make_state();

        apply_completed_before_control_signal(
            &mut s,
            &PromptSource::Channel(ch_a),
            &ControlSignal::Rotate,
        );

        assert!(!s.sessions.contains_key(&ch_a));
        assert!(!s.turn_counts.contains_key(&ch_a));
        assert!(!s.core_sections.contains_key(&ch_a));
        assert!(!s.has_channel_state(&ch_a));
        assert_eq!(s.sessions.get(&ch_b).unwrap(), "sess-b");
        assert_eq!(*s.turn_counts.get(&ch_b).unwrap(), 3);
        assert_eq!(s.core_sections.get(&ch_b).unwrap(), "core-b");
        assert_eq!(s.heartbeat_session.as_deref(), Some("sess-hb"));
        assert_eq!(s.heartbeat_turn_count, 7);
    }

    #[test]
    fn test_cancel_after_natural_completion_preserves_channel_state() {
        let (mut s, ch_a, ch_b) = make_state();

        apply_completed_before_control_signal(
            &mut s,
            &PromptSource::Channel(ch_a),
            &ControlSignal::Cancel,
        );

        assert_eq!(s.sessions.get(&ch_a).unwrap(), "sess-a");
        assert_eq!(*s.turn_counts.get(&ch_a).unwrap(), 5);
        assert_eq!(s.core_sections.get(&ch_a).unwrap(), "core-a");
        assert_eq!(s.sessions.get(&ch_b).unwrap(), "sess-b");
    }

    #[test]
    fn test_invalidate_channel_clears_session_and_turn_count() {
        let (mut s, ch_a, ch_b) = make_state();
        s.invalidate(&PromptSource::Channel(ch_a));

        assert!(!s.sessions.contains_key(&ch_a));
        assert!(!s.turn_counts.contains_key(&ch_a));
        assert!(!s.core_sections.contains_key(&ch_a));
        assert!(!s.has_channel_state(&ch_a));
        // ch_b untouched
        assert_eq!(s.sessions.get(&ch_b).unwrap(), "sess-b");
        assert_eq!(*s.turn_counts.get(&ch_b).unwrap(), 3);
        assert_eq!(s.core_sections.get(&ch_b).unwrap(), "core-b");
        // heartbeat untouched
        assert_eq!(s.heartbeat_session.as_deref(), Some("sess-hb"));
        assert_eq!(s.heartbeat_turn_count, 7);
    }

    #[test]
    fn test_invalidate_heartbeat_clears_session_and_turn_count() {
        let (mut s, ch_a, ch_b) = make_state();
        s.invalidate(&PromptSource::Heartbeat);

        assert!(s.heartbeat_session.is_none());
        assert_eq!(s.heartbeat_turn_count, 0);
        // channels untouched
        assert_eq!(s.sessions.len(), 2);
        assert_eq!(*s.turn_counts.get(&ch_a).unwrap(), 5);
        assert_eq!(*s.turn_counts.get(&ch_b).unwrap(), 3);
        assert_eq!(s.core_sections.get(&ch_a).unwrap(), "core-a");
        assert_eq!(s.core_sections.get(&ch_b).unwrap(), "core-b");
    }

    #[test]
    fn test_invalidate_all_clears_everything() {
        let (mut s, _ch_a, _ch_b) = make_state();
        s.invalidate_all();

        assert!(s.sessions.is_empty());
        assert!(s.turn_counts.is_empty());
        assert!(s.core_sections.is_empty());
        assert!(s.heartbeat_session.is_none());
        assert_eq!(s.heartbeat_turn_count, 0);
    }

    #[test]
    fn test_invalidate_nonexistent_channel_is_noop() {
        let (mut s, ch_a, ch_b) = make_state();
        let ghost = Uuid::new_v4();
        s.invalidate(&PromptSource::Channel(ghost));

        // Everything still intact.
        assert_eq!(s.sessions.len(), 2);
        assert_eq!(s.turn_counts.len(), 2);
        assert_eq!(*s.turn_counts.get(&ch_a).unwrap(), 5);
        assert_eq!(*s.turn_counts.get(&ch_b).unwrap(), 3);
        assert_eq!(s.core_sections.get(&ch_a).unwrap(), "core-a");
        assert_eq!(s.core_sections.get(&ch_b).unwrap(), "core-b");
    }

    #[test]
    fn test_invalidate_all_on_empty_state_is_noop() {
        let mut s = SessionState::default();
        s.invalidate_all(); // should not panic
        assert!(s.sessions.is_empty());
        assert!(s.turn_counts.is_empty());
        assert!(s.core_sections.is_empty());
    }

    #[test]
    fn test_invalidate_channel_returns_true_when_session_existed() {
        let (mut s, ch_a, ch_b) = make_state();
        assert!(s.invalidate_channel(&ch_a));
        assert!(!s.sessions.contains_key(&ch_a));
        assert!(!s.turn_counts.contains_key(&ch_a));
        assert!(!s.core_sections.contains_key(&ch_a));
        assert!(!s.has_channel_state(&ch_a));
        // ch_b untouched
        assert_eq!(s.sessions.get(&ch_b).unwrap(), "sess-b");
        assert_eq!(*s.turn_counts.get(&ch_b).unwrap(), 3);
        assert_eq!(s.core_sections.get(&ch_b).unwrap(), "core-b");
        // heartbeat untouched
        assert_eq!(s.heartbeat_session.as_deref(), Some("sess-hb"));
        assert_eq!(s.heartbeat_turn_count, 7);
    }

    #[test]
    fn test_invalidate_channel_returns_false_when_no_session() {
        let (mut s, _ch_a, _ch_b) = make_state();
        let ghost = Uuid::new_v4();
        assert!(!s.invalidate_channel(&ghost));
        // Nothing changed.
        assert_eq!(s.sessions.len(), 2);
        assert_eq!(s.turn_counts.len(), 2);
    }

    #[test]
    fn test_removed_channels_cleaned_via_invalidate_channel() {
        // Simulates handle_prompt_result: channels removed while agent
        // was checked out should have both sessions and turn_counts stripped.
        let (mut s, ch_a, ch_b) = make_state();
        let removed = vec![ch_a];
        for ch in &removed {
            s.invalidate_channel(ch);
        }
        assert!(!s.sessions.contains_key(&ch_a));
        assert!(!s.turn_counts.contains_key(&ch_a));
        assert!(!s.core_sections.contains_key(&ch_a));
        assert!(!s.has_channel_state(&ch_a));
        assert_eq!(s.sessions.get(&ch_b).unwrap(), "sess-b");
        assert_eq!(*s.turn_counts.get(&ch_b).unwrap(), 3);
        assert_eq!(s.core_sections.get(&ch_b).unwrap(), "core-b");
    }

    // ── ControlSignal::SwitchModel (Phase 3a, Option ii) ─────────────────────

    #[test]
    fn test_switch_model_after_natural_completion_invalidates_channel_state() {
        let (mut s, ch_a, ch_b) = make_state();

        // SwitchModel must invalidate just like Rotate so the requeued turn
        // re-creates a fresh session that re-applies the new desired_model.
        apply_completed_before_control_signal(
            &mut s,
            &PromptSource::Channel(ch_a),
            &ControlSignal::SwitchModel("gpt-5".into()),
        );

        assert!(!s.has_channel_state(&ch_a));
        // ch_b untouched — the switch is channel-scoped.
        assert_eq!(s.sessions.get(&ch_b).unwrap(), "sess-b");
        assert_eq!(*s.turn_counts.get(&ch_b).unwrap(), 3);
    }

    // ── requeue_cancelled_batch ────────────────────────────────────────────
    // Table-driven pin of the `ControlSignal` → `CancelReason` ownership that
    // decides whether a cancel-drain-expiry batch is merged into the next
    // flush or dropped outright. `Cancel`/`Rotate` must return `None` — a
    // regression here would silently fall through to
    // `unwrap_or(CancelReason::Steer)` at the requeue site and preserve a
    // batch that should have been discarded.

    fn one_event_batch(channel_id: Uuid) -> FlushBatch {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "test")
            .sign_with_keys(&keys)
            .unwrap();
        FlushBatch {
            channel_id,
            events: vec![crate::queue::BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        }
    }

    #[test]
    fn test_requeue_cancelled_batch_maps_control_signal_to_cancel_reason() {
        let cases = [
            (ControlSignal::Steer, Some(CancelReason::Steer)),
            (ControlSignal::Interrupt, Some(CancelReason::Interrupt)),
            (
                ControlSignal::SwitchModel("gpt-5".into()),
                Some(CancelReason::Interrupt),
            ),
            (ControlSignal::Cancel, None),
            (ControlSignal::Rotate, None),
        ];
        let mut ctx = make_prompt_context_no_owner();
        ctx.dedup_mode = DedupMode::Queue;

        for (signal, expected_reason) in cases {
            let channel_id = Uuid::new_v4();
            let batch = one_event_batch(channel_id);
            let result = requeue_cancelled_batch(&ctx, signal.clone(), Some(batch));
            match expected_reason {
                Some(reason) => {
                    let batch = result
                        .unwrap_or_else(|| panic!("{signal:?} must preserve the batch, got None"));
                    assert_eq!(
                        batch.cancel_reason,
                        Some(reason),
                        "{signal:?} must stamp {reason:?}"
                    );
                }
                None => assert!(
                    result.is_none(),
                    "{signal:?} must drop the batch, got {result:?}"
                ),
            }
        }
    }

    // ── classify_control_cancel_failure ─────────────────────────────────────
    // Table-driven pin of the single production seam used by the
    // `Err(error)` arm in `run_prompt_task`'s control-cancel branch. Crosses
    // the exact error→outcome AND outcome→batch-fate boundary in one call,
    // so a regression to the old per-arm duplication (or to routing an
    // unexpected HardTimeout back through the real hard-cap path) fails
    // here rather than only in independently-manufactured unit tests.

    /// Assert `outcome` is the expected `PromptOutcome` variant. `PromptOutcome`
    /// has no `PartialEq` (it wraps `AcpError`, which isn't `PartialEq`), so
    /// this matches by shape instead of deriving equality onto the whole enum.
    fn assert_outcome_matches(outcome: &PromptOutcome, expected: &str) {
        let label = match outcome {
            PromptOutcome::AgentExited => "AgentExited",
            PromptOutcome::Timeout(TimeoutKind::Idle) => "Timeout(Idle)",
            PromptOutcome::Timeout(TimeoutKind::Hard { .. }) => "Timeout(Hard)",
            PromptOutcome::CancelDrainTimeout(_) => "CancelDrainTimeout",
            PromptOutcome::Error(_) => "Error",
            PromptOutcome::Cancelled => "Cancelled",
            PromptOutcome::Ok(_) => "Ok",
        };
        assert_eq!(
            label, expected,
            "got outcome shape {label}, want {expected}"
        );
    }

    #[test]
    fn test_classify_control_cancel_failure_crosses_error_outcome_and_batch_fate() {
        let ctx = {
            let mut ctx = make_prompt_context_no_owner();
            ctx.dedup_mode = DedupMode::Queue;
            ctx
        };

        struct Case {
            name: &'static str,
            error: fn() -> AcpError,
            signal: ControlSignal,
            expected_outcome: &'static str,
            batch_preserved: bool,
            expected_reason: Option<CancelReason>,
            invalidate_all: bool,
        }

        let cases = [
            Case {
                name: "CancelDrainTimeout + Steer preserves batch with Steer reason",
                error: || AcpError::CancelDrainTimeout(CONTROL_CANCEL_GRACE),
                signal: ControlSignal::Steer,
                expected_outcome: "CancelDrainTimeout",
                batch_preserved: true,
                expected_reason: Some(CancelReason::Steer),
                invalidate_all: false,
            },
            Case {
                name: "CancelDrainTimeout + Cancel drops the batch",
                error: || AcpError::CancelDrainTimeout(CONTROL_CANCEL_GRACE),
                signal: ControlSignal::Cancel,
                expected_outcome: "CancelDrainTimeout",
                batch_preserved: false,
                expected_reason: None,
                invalidate_all: false,
            },
            Case {
                name: "CancelDrainTimeout + Interrupt preserves batch with Interrupt reason",
                error: || AcpError::CancelDrainTimeout(CONTROL_CANCEL_GRACE),
                signal: ControlSignal::Interrupt,
                expected_outcome: "CancelDrainTimeout",
                batch_preserved: true,
                expected_reason: Some(CancelReason::Interrupt),
                invalidate_all: false,
            },
            Case {
                name: "CancelDrainTimeout + Rotate drops the batch",
                error: || AcpError::CancelDrainTimeout(CONTROL_CANCEL_GRACE),
                signal: ControlSignal::Rotate,
                expected_outcome: "CancelDrainTimeout",
                batch_preserved: false,
                expected_reason: None,
                invalidate_all: false,
            },
            Case {
                name: "CancelDrainTimeout + SwitchModel preserves batch with Interrupt reason",
                error: || AcpError::CancelDrainTimeout(CONTROL_CANCEL_GRACE),
                signal: ControlSignal::SwitchModel("gpt-5".to_string()),
                expected_outcome: "CancelDrainTimeout",
                batch_preserved: true,
                expected_reason: Some(CancelReason::Interrupt),
                invalidate_all: false,
            },
            Case {
                name: "unexpected HardTimeout cannot become Timeout(Hard)",
                error: || AcpError::HardTimeout {
                    silence: Duration::from_secs(300),
                },
                signal: ControlSignal::Steer,
                expected_outcome: "CancelDrainTimeout",
                batch_preserved: true,
                expected_reason: Some(CancelReason::Steer),
                invalidate_all: false,
            },
            Case {
                name: "AgentExited requests all-session invalidation and preserves via Steer",
                error: || AcpError::AgentExited,
                signal: ControlSignal::Steer,
                expected_outcome: "AgentExited",
                batch_preserved: true,
                expected_reason: Some(CancelReason::Steer),
                invalidate_all: true,
            },
            Case {
                name: "AgentExited + Cancel still drops the batch",
                error: || AcpError::AgentExited,
                signal: ControlSignal::Cancel,
                expected_outcome: "AgentExited",
                batch_preserved: false,
                expected_reason: None,
                invalidate_all: true,
            },
            Case {
                name: "IdleTimeout maps to Timeout(Idle)",
                error: || AcpError::IdleTimeout(Duration::from_secs(30)),
                signal: ControlSignal::Steer,
                expected_outcome: "Timeout(Idle)",
                batch_preserved: true,
                expected_reason: Some(CancelReason::Steer),
                invalidate_all: false,
            },
        ];

        for case in cases {
            let channel_id = Uuid::new_v4();
            let batch = one_event_batch(channel_id);
            let failure = classify_control_cancel_failure(
                &ctx,
                (case.error)(),
                case.signal.clone(),
                Some(batch),
            );
            assert_outcome_matches(&failure.outcome, case.expected_outcome);
            assert_eq!(
                failure.invalidate_all, case.invalidate_all,
                "{}: invalidate_all mismatch",
                case.name
            );
            match case.expected_reason {
                Some(reason) => {
                    let batch = failure
                        .retry_batch
                        .unwrap_or_else(|| panic!("{}: batch must be preserved", case.name));
                    assert_eq!(
                        batch.cancel_reason,
                        Some(reason),
                        "{}: cancel_reason mismatch",
                        case.name
                    );
                }
                None => assert!(
                    failure.retry_batch.is_none(),
                    "{}: batch must be dropped, got {:?}",
                    case.name,
                    failure.retry_batch
                ),
            }
            assert_eq!(
                case.batch_preserved,
                case.expected_reason.is_some(),
                "{}: test table internally inconsistent",
                case.name
            );
        }
    }

    // ── turn liveness emission ───────────────────────────────────────────────

    fn liveness_count(handle: &observer::ObserverHandle) -> usize {
        handle
            .snapshot()
            .iter()
            .filter(|e| e.kind == "turn_liveness")
            .count()
    }

    fn open_liveness_state() -> Arc<Mutex<LivenessState>> {
        Arc::new(Mutex::new(LivenessState {
            closed: false,
            session_id: None,
        }))
    }

    #[tokio::test(start_paused = true)]
    async fn test_liveness_stops_before_completion_frame() {
        let observer = observer::ObserverHandle::in_process();
        let context =
            observer::context_for_turn(None, None, "t-1".into(), "2026-07-14T21:00:00Z".into());
        let completion_context = observer::context_for(None, None, Some("t-1".into()));
        let completion_observer = observer.clone();
        let completion_handle = tokio::spawn(async move {
            let state = open_liveness_state();
            let _liveness_guard = LivenessGuard::new(
                tokio::spawn(run_turn_liveness(
                    Some(observer.clone()),
                    Some(0),
                    context,
                    Duration::from_secs(10),
                    Arc::clone(&state),
                )),
                state,
            );
            tokio::time::sleep(Duration::from_secs(25)).await;
            observer.emit(
                "turn_completed",
                Some(0),
                &completion_context,
                serde_json::json!({}),
            );
        });
        tokio::task::yield_now().await;

        tokio::time::advance(Duration::from_secs(25)).await;
        completion_handle.await.unwrap();
        tokio::task::yield_now().await;

        let events = completion_observer.snapshot();
        let completion_index = events
            .iter()
            .position(|event| event.kind == "turn_completed")
            .expect("turn must complete");
        assert!(
            events[..completion_index]
                .iter()
                .all(|event| event.kind != "turn_liveness"
                    || event.turn_id.as_deref() == Some("t-1")),
            "pre-completion liveness must belong to the active turn"
        );
        assert!(
            events[completion_index + 1..]
                .iter()
                .all(|event| event.kind != "turn_liveness"),
            "liveness must be aborted before a completion frame is emitted"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_liveness_fires_until_guard_drops() {
        let observer = observer::ObserverHandle::in_process();
        let started_at = "2026-07-14T21:00:00Z".to_string();
        let context = observer::context_for_turn(None, None, "t-1".into(), started_at.clone());
        let state = open_liveness_state();
        let guard = LivenessGuard::new(
            tokio::spawn(run_turn_liveness(
                Some(observer.clone()),
                Some(0),
                context,
                Duration::from_secs(10),
                Arc::clone(&state),
            )),
            state,
        );
        tokio::task::yield_now().await;

        // First liveness tick at 10s and the second at 20s.
        tokio::time::advance(Duration::from_secs(25)).await;
        tokio::task::yield_now().await;
        assert_eq!(liveness_count(&observer), 2);

        let pings: Vec<_> = observer
            .snapshot()
            .into_iter()
            .filter(|e| e.kind == "turn_liveness")
            .collect();
        assert!(pings
            .iter()
            .all(|event| event.turn_id.as_deref() == Some("t-1")));
        assert!(pings
            .iter()
            .all(|event| event.started_at.as_deref() == Some(&started_at)));
        assert!(pings
            .iter()
            .all(|event| event.payload == serde_json::json!({})));
        assert_eq!(
            serde_json::to_value(&pings[0]).unwrap()["startedAt"],
            started_at,
            "turn start must serialize in the observer envelope"
        );

        // The guard is owned by `run_prompt_task`; dropping it aborts liveness
        // so completed, cancelled, and errored turns cannot emit late pings.
        drop(guard);
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(liveness_count(&observer), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn test_liveness_backfills_session_id_after_resolution() {
        let observer = observer::ObserverHandle::in_process();
        let context =
            observer::context_for_turn(None, None, "t-1".into(), "2026-07-14T21:00:00Z".into());
        let state = open_liveness_state();
        let guard = LivenessGuard::new(
            tokio::spawn(run_turn_liveness(
                Some(observer.clone()),
                Some(0),
                context,
                Duration::from_secs(10),
                Arc::clone(&state),
            )),
            state,
        );
        tokio::task::yield_now().await;

        // First tick at 10s fires before the session resolves — must carry
        // no session ID, matching every other pre-resolution observer frame.
        tokio::time::advance(Duration::from_secs(10)).await;
        tokio::task::yield_now().await;
        guard.set_session_id("sess-1".to_string());

        // Second tick at 20s fires after resolution — must carry it.
        tokio::time::advance(Duration::from_secs(10)).await;
        tokio::task::yield_now().await;

        let pings: Vec<_> = observer
            .snapshot()
            .into_iter()
            .filter(|e| e.kind == "turn_liveness")
            .collect();
        assert_eq!(pings.len(), 2);
        assert_eq!(
            pings[0].session_id, None,
            "pre-resolution ping must not carry a session ID"
        );
        assert_eq!(
            pings[1].session_id.as_deref(),
            Some("sess-1"),
            "post-resolution ping must carry the resolved session ID"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn test_liveness_disabled_when_interval_zero_emits_nothing() {
        let observer = observer::ObserverHandle::in_process();
        let context = observer::context_for(None, None, Some("t-1".into()));
        let liveness = run_turn_liveness(
            Some(observer.clone()),
            Some(0),
            context,
            Duration::ZERO,
            open_liveness_state(),
        );
        tokio::pin!(liveness);

        tokio::select! {
            biased;
            () = tokio::time::sleep(Duration::from_secs(120)) => {}
            _ = &mut liveness => unreachable!("disabled liveness future never resolves"),
        }

        assert_eq!(liveness_count(&observer), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn test_liveness_without_observer_emits_nothing() {
        // A turn that never started has no observer handle — the future must
        // park without emitting or panicking.
        let context = observer::context_for(None, None, Some("t-1".into()));
        let liveness = run_turn_liveness(
            None,
            None,
            context,
            Duration::from_secs(10),
            open_liveness_state(),
        );
        tokio::pin!(liveness);

        tokio::select! {
            biased;
            () = tokio::time::sleep(Duration::from_secs(120)) => {}
            _ = &mut liveness => unreachable!("handle-less liveness future never resolves"),
        }
        // No observer to assert against — reaching here without panic is the test.
    }

    // These two tests pin the shutdown mechanism itself (F1), not timing.
    // The existing paused-clock tests above only prove liveness stops
    // *eventually* after a guard drop — under `tokio::time::pause`, the
    // scheduler never actually interleaves a drop with an in-flight emit, so
    // they cannot catch a real cross-thread race between `LivenessGuard::drop`
    // and `run_turn_liveness`'s tick. These assert the two halves of the
    // contract directly: the check gates the emit with the flag pre-set (no
    // `LivenessGuard` involved), and `drop` cannot return while the shared
    // lock is held by an in-flight tick (real OS threads, no cooperative
    // scheduling to serialize the race away).

    #[tokio::test(start_paused = true)]
    async fn test_liveness_emits_nothing_once_closed_flag_is_set() {
        let observer = observer::ObserverHandle::in_process();
        let context =
            observer::context_for_turn(None, None, "t-1".into(), "2026-07-14T21:00:00Z".into());
        // Set directly, bypassing `LivenessGuard` — isolates the read side of
        // the contract: the check under the lock must gate the emit on its own.
        let state = Arc::new(Mutex::new(LivenessState {
            closed: true,
            session_id: None,
        }));
        let liveness = run_turn_liveness(
            Some(observer.clone()),
            Some(0),
            context,
            Duration::from_secs(10),
            state,
        );
        tokio::time::timeout(Duration::from_secs(60), liveness)
            .await
            .expect("run_turn_liveness must return once closed, not park forever");

        assert_eq!(
            liveness_count(&observer),
            0,
            "the pre-set closed flag must suppress every tick's emit"
        );
    }

    #[test]
    fn test_liveness_guard_drop_blocks_while_emit_lock_is_held() {
        // Standing in for a tick that has already entered its critical
        // section: hold the shared lock before the guard drops.
        let state = Arc::new(Mutex::new(LivenessState {
            closed: false,
            session_id: None,
        }));
        let held = state.lock().unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.spawn(std::future::pending::<()>());
        let guard = LivenessGuard::new(handle, Arc::clone(&state));

        let (tx, rx) = std::sync::mpsc::channel();
        let drop_thread = std::thread::spawn(move || {
            drop(guard);
            tx.send(()).unwrap();
        });

        // While the emit lock is held, `drop` cannot have completed: it takes
        // the same lock before it sets the flag and aborts. A bounded timeout
        // proves non-completion by construction of the lock, not the clock —
        // `recv_timeout` returning `Timeout` here only holds because the
        // mutex is genuinely contended; it cannot pass by scheduling luck.
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout),
            "drop must block while the tick's emit lock is held"
        );

        // Release the lock — drop can now acquire it, set the flag, and abort.
        drop(held);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("drop must complete once the emit lock is released");
        drop_thread.join().unwrap();
        assert!(
            state.lock().unwrap().closed,
            "closed flag must be set by the time drop has returned"
        );
    }

    // ── steer_rx invariant tests ──────────────────────────────────────────
    //
    // These pin the `send_prompt_result` invariant: `steer_rx` is always
    // `None` on any agent returned to the pool, regardless of which exit
    // path fired.
    //
    // Test 1 (session-create-error path): installs a receiver, then calls
    // `send_prompt_result` without the read loop running `take()` — simulating
    // any early-return arm (e.g. session-create failure). The receiver must be
    // cleared and the next `install_steer_rx` must not panic.
    //
    // Test 2 (post-read-loop path): receiver is already `None` (the read loop
    // already consumed it via `take()`). `send_prompt_result` is idempotent —
    // `steer_rx` stays `None` and the next `install_steer_rx` still does not
    // panic.

    /// After an early-return path (receiver installed but read loop never ran),
    /// the returned agent's `steer_rx` is `None` and a subsequent
    /// `install_steer_rx` does not panic.
    #[tokio::test]
    async fn test_send_prompt_result_clears_steer_rx_on_early_return() {
        let acp = AcpClient::spawn(
            "bash",
            &["-c".to_string(), "sleep 10".to_string()],
            &[],
            false,
        )
        .await
        .expect("failed to spawn test agent");
        let mut agent = OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            agent_name: "unknown".into(),
            goose_system_prompt_supported: None,
            protocol_version: 2,
        };

        // Simulate dispatch: install a steer receiver (normally done by
        // `dispatch_pending` before `run_prompt_task` is spawned).
        let (_steer_tx, steer_rx) = tokio::sync::mpsc::channel::<SteerRequest>(1);
        agent.acp.install_steer_rx(steer_rx);

        // Simulate session-create error: early-return path calls
        // `send_prompt_result` without the read loop ever running `take()`.
        let (result_tx, mut result_rx) = tokio::sync::mpsc::unbounded_channel::<PromptResult>();
        let source = PromptSource::Heartbeat;
        send_prompt_result(
            &result_tx,
            "test-turn-id",
            agent,
            source,
            PromptOutcome::Error(AcpError::Protocol("simulated session-create error".into())),
            None,
        );

        // Receive the PromptResult back from the channel.
        let mut result = result_rx.recv().await.expect("PromptResult must be sent");

        // steer_rx must be cleared even though the read loop never ran take().
        assert!(
            result.agent.acp.steer_rx_is_none(),
            "steer_rx must be None after send_prompt_result on error path"
        );

        // The next dispatch can now install a fresh receiver without panicking.
        let (_steer_tx2, steer_rx2) = tokio::sync::mpsc::channel::<SteerRequest>(1);
        result.agent.acp.install_steer_rx(steer_rx2);
        // Reaching here without a panic is the test.
    }

    /// After a successful prompt (read loop already consumed `steer_rx` via
    /// `take()`), `send_prompt_result` is a no-op — `steer_rx` stays `None`
    /// and the next `install_steer_rx` does not panic.
    #[tokio::test]
    async fn test_send_prompt_result_is_noop_when_steer_rx_already_consumed() {
        let acp = AcpClient::spawn(
            "bash",
            &["-c".to_string(), "sleep 10".to_string()],
            &[],
            false,
        )
        .await
        .expect("failed to spawn test agent");
        let agent = OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            agent_name: "unknown".into(),
            goose_system_prompt_supported: None,
            protocol_version: 2,
        };

        // Simulate a completed turn: `steer_rx` was consumed by the read loop
        // (`take()` was called), so it is already `None` when the turn ends.
        assert!(
            agent.acp.steer_rx_is_none(),
            "precondition: steer_rx starts as None"
        );

        let (result_tx, mut result_rx) = tokio::sync::mpsc::unbounded_channel::<PromptResult>();
        let source = PromptSource::Heartbeat;
        send_prompt_result(
            &result_tx,
            "test-turn-id",
            agent,
            source,
            PromptOutcome::Ok(StopReason::EndTurn),
            None,
        );

        let mut result = result_rx.recv().await.expect("PromptResult must be sent");

        // Still None — clear_steer_rx on an already-None field is idempotent.
        assert!(
            result.agent.acp.steer_rx_is_none(),
            "steer_rx must remain None after send_prompt_result on happy path"
        );

        // The next dispatch can install a fresh receiver without panicking.
        let (_steer_tx, steer_rx) = tokio::sync::mpsc::channel::<SteerRequest>(1);
        result.agent.acp.install_steer_rx(steer_rx);
        // Reaching here without a panic is the test.
    }

    // ── NIP-AM emit-hook unit tests ────────────────────────────────────────

    /// `acp_stop_to_core` maps all ACP stop reasons to the correct NIP-AM
    /// variants without panicking on any input.
    #[test]
    fn test_acp_stop_to_core_maps_all_variants() {
        use buzz_core::agent_turn_metric::StopReason as CoreStop;
        assert_eq!(acp_stop_to_core(&StopReason::EndTurn), CoreStop::EndTurn);
        assert_eq!(
            acp_stop_to_core(&StopReason::Cancelled),
            CoreStop::Cancelled
        );
        assert_eq!(
            acp_stop_to_core(&StopReason::MaxTokens),
            CoreStop::MaxTokens
        );
        assert_eq!(
            acp_stop_to_core(&StopReason::MaxTurnRequests),
            CoreStop::Unknown
        );
        assert_eq!(acp_stop_to_core(&StopReason::Refusal), CoreStop::Unknown);
    }

    /// `publish_agent_turn_metric` is a no-op when `usage` is `None`.
    #[tokio::test]
    async fn test_publish_agent_turn_metric_noop_on_no_usage() {
        let ctx = make_prompt_context_no_owner();
        // usage = None → early return, no panic.
        publish_agent_turn_metric(
            &ctx,
            None,
            None,
            "sess-1",
            "turn-1",
            Some(buzz_core::agent_turn_metric::StopReason::EndTurn),
        )
        .await;
    }

    /// `publish_agent_turn_metric` is a no-op when `owner_pubkey` is absent.
    #[tokio::test]
    async fn test_publish_agent_turn_metric_noop_on_no_owner() {
        let ctx = make_prompt_context_no_owner();
        let usage = crate::usage::TurnUsage {
            session_id: "sess-1".to_string(),
            turn_seq: 1,
            delta_reliable: true,
            turn_input_tokens: Some(100),
            turn_output_tokens: Some(50),
            turn_total_tokens: None,
            turn_cost_usd: None,
            turn_cache_read_tokens: None,
            cumulative_input_tokens: 100,
            cumulative_output_tokens: 50,
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            model: None,
        };
        // owner_pubkey = None → early return, no panic.
        publish_agent_turn_metric(
            &ctx,
            Some(usage),
            None,
            "sess-1",
            "turn-1",
            Some(buzz_core::agent_turn_metric::StopReason::EndTurn),
        )
        .await;
    }

    /// `publish_agent_turn_metric` encrypts the payload when owner is present
    /// (the HTTP submit will fail in tests, but we verify no panic and the
    /// encrypt/sign path executes).
    #[tokio::test]
    async fn test_publish_agent_turn_metric_encrypts_with_owner() {
        let agent_keys = nostr::Keys::generate();
        let owner_keys = nostr::Keys::generate();
        let ctx = make_prompt_context_with_owner(&agent_keys, owner_keys.public_key());
        let usage = crate::usage::TurnUsage {
            session_id: "sess-1".to_string(),
            turn_seq: 1,
            delta_reliable: true,
            turn_input_tokens: Some(200),
            turn_output_tokens: Some(80),
            turn_total_tokens: None,
            turn_cost_usd: Some(0.001),
            turn_cache_read_tokens: None,
            cumulative_input_tokens: 200,
            cumulative_output_tokens: 80,
            cumulative_total_tokens: None,
            cumulative_cost_usd: Some(0.001),
            cumulative_cache_read_tokens: None,
            model: None,
        };
        // Will try to publish and fail (no real relay) but must not panic.
        publish_agent_turn_metric(
            &ctx,
            Some(usage),
            Some(uuid::Uuid::new_v4()),
            "sess-1",
            "turn-1",
            Some(buzz_core::agent_turn_metric::StopReason::EndTurn),
        )
        .await;
    }

    /// Regression for the control-cancel drain: `publish_agent_turn_metric`
    /// with a `Cancelled` stop reason and pending usage executes without panic
    /// (encrypt+sign path). This mirrors the control-signal arm that previously
    /// returned early without draining usage.
    #[tokio::test]
    async fn test_publish_agent_turn_metric_cancelled_stop_reason() {
        let agent_keys = nostr::Keys::generate();
        let owner_keys = nostr::Keys::generate();
        let ctx = make_prompt_context_with_owner(&agent_keys, owner_keys.public_key());
        let usage = crate::usage::TurnUsage {
            session_id: "sess-cancel".to_string(),
            turn_seq: 2,
            delta_reliable: true,
            turn_input_tokens: Some(50),
            turn_output_tokens: Some(20),
            turn_total_tokens: None,
            turn_cost_usd: None,
            turn_cache_read_tokens: None,
            cumulative_input_tokens: 150,
            cumulative_output_tokens: 70,
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            model: None,
        };
        // Must not panic; HTTP submit will fail (no real relay) — that's fine.
        publish_agent_turn_metric(
            &ctx,
            Some(usage),
            Some(uuid::Uuid::new_v4()),
            "sess-cancel",
            "turn-cancel",
            Some(buzz_core::agent_turn_metric::StopReason::Cancelled),
        )
        .await;
    }

    /// `publish_agent_turn_metric` uses `ctx.harness_name` in the payload.
    /// A buzz-agent-commanded context must not panic — verifies the harness
    /// field flows through encrypt/sign without error.
    #[tokio::test]
    async fn test_publish_agent_turn_metric_buzz_agent_harness_name() {
        let agent_keys = nostr::Keys::generate();
        let owner_keys = nostr::Keys::generate();
        let mut ctx = make_prompt_context_with_owner(&agent_keys, owner_keys.public_key());
        ctx.harness_name = "buzz-agent".to_string();
        let usage = crate::usage::TurnUsage {
            session_id: "sess-ba".to_string(),
            turn_seq: 1,
            delta_reliable: false, // first turn from buzz-agent
            turn_input_tokens: None,
            turn_output_tokens: None,
            turn_total_tokens: None,
            turn_cost_usd: None,
            turn_cache_read_tokens: None,
            cumulative_input_tokens: 400,
            cumulative_output_tokens: 100,
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            model: None,
        };
        // Will try to publish (encrypt succeeds) and fail HTTP (no relay) — must not panic.
        publish_agent_turn_metric(
            &ctx,
            Some(usage),
            Some(uuid::Uuid::new_v4()),
            "sess-ba",
            "turn-ba",
            Some(buzz_core::agent_turn_metric::StopReason::EndTurn),
        )
        .await;
    }

    /// `build_turn_metric_counts` maps exact turn and cumulative totals from
    /// `TurnUsage` to the corresponding `TokenCounts.total_tokens` fields.
    /// Reverting the production fields at the call site to `None` would break
    /// this test; the test constrains the real code path.
    #[test]
    fn test_build_turn_metric_counts_exact_totals_map_through() {
        let usage = crate::usage::TurnUsage {
            session_id: "sess-total".to_string(),
            turn_seq: 2,
            delta_reliable: true,
            turn_input_tokens: Some(100),
            turn_output_tokens: Some(30),
            turn_total_tokens: Some(130), // genuine per-turn total
            turn_cost_usd: None,
            turn_cache_read_tokens: None,
            cumulative_input_tokens: 500,
            cumulative_output_tokens: 120,
            cumulative_total_tokens: Some(620), // genuine cumulative total
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            model: None,
        };

        let (turn, cumulative) = crate::pool::build_turn_metric_counts(&usage);

        // Serialise to JSON — this is what ultimately goes on the wire.
        let turn_json = serde_json::to_value(turn.as_ref().expect("turn counts present")).unwrap();
        let cum_json =
            serde_json::to_value(cumulative.as_ref().expect("cumulative counts present")).unwrap();

        // Per-turn total must be the genuine provider-reported value.
        assert_eq!(
            turn_json["totalTokens"],
            serde_json::json!(130),
            "per-turn total must map to TokenCounts.totalTokens in wire JSON"
        );
        assert_eq!(turn_json["inputTokens"], serde_json::json!(100));
        assert_eq!(turn_json["outputTokens"], serde_json::json!(30));

        // Cumulative total must be the genuine session total.
        assert_eq!(
            cum_json["totalTokens"],
            serde_json::json!(620),
            "cumulative total must map to TokenCounts.totalTokens in wire JSON"
        );
        assert_eq!(cum_json["inputTokens"], serde_json::json!(500));
        assert_eq!(cum_json["outputTokens"], serde_json::json!(120));
    }

    /// When totals are absent, `build_turn_metric_counts` must produce null
    /// `total_tokens` — never a derived input+output sum (NIP-AM MUST NOT).
    /// Reverting the production fields to hardcoded `None` would leave this test
    /// passing but input/output would disagree, making the null-path detectable.
    #[test]
    fn test_build_turn_metric_counts_null_totals_never_derived() {
        let usage = crate::usage::TurnUsage {
            session_id: "sess-nototal".to_string(),
            turn_seq: 1,
            delta_reliable: true,
            turn_input_tokens: Some(200),
            turn_output_tokens: Some(60),
            turn_total_tokens: None, // provider did not supply a total
            turn_cost_usd: None,
            turn_cache_read_tokens: None,
            cumulative_input_tokens: 200,
            cumulative_output_tokens: 60,
            cumulative_total_tokens: None, // session has no total
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            model: None,
        };

        let (turn, cumulative) = crate::pool::build_turn_metric_counts(&usage);

        let turn_json = serde_json::to_value(turn.as_ref().expect("turn counts present")).unwrap();
        let cum_json =
            serde_json::to_value(cumulative.as_ref().expect("cumulative counts present")).unwrap();

        // total_tokens must be null in the wire JSON.
        assert!(
            turn_json["totalTokens"].is_null(),
            "absent turn total must serialize as null — not derived from in+out"
        );
        assert!(
            cum_json["totalTokens"].is_null(),
            "absent cumulative total must serialize as null — not derived from in+out"
        );

        // Input/output must still carry their real values.
        assert_eq!(
            turn_json["inputTokens"],
            serde_json::json!(200),
            "inputTokens must be present even when total is absent"
        );
        assert_eq!(
            turn_json["outputTokens"],
            serde_json::json!(60),
            "outputTokens must be present even when total is absent"
        );

        // The null total must not equal the input+output sum — it must be genuinely null.
        let derived_sum = serde_json::json!(200u64 + 60u64);
        assert_ne!(
            turn_json["totalTokens"], derived_sum,
            "total_tokens must never equal input+output when provider omitted it"
        );
    }

    /// A payload with nonzero `accumulatedCachedInputTokens` on the second turn
    /// must produce a kind:44200 payload where `cumulative.cacheReadTokens` is
    /// nonzero and `turn.cacheReadTokens` reflects the per-turn delta.
    /// This is the acceptance-criterion test: it proves the threading is live,
    /// not hardcoded to None.
    #[test]
    fn test_build_turn_metric_counts_cache_read_tokens_thread_through() {
        // Wire-parse a buzz-agent payload with cache, run it through the tracker,
        // and verify the published TokenCounts carry the cache field.
        let raw1 = serde_json::json!({
            "sessionId": "cache-sess",
            "update": {
                "sessionUpdate": "usage_update",
                "accumulatedInputTokens": 15_091,
                "accumulatedOutputTokens": 156,
                "accumulatedCachedInputTokens": 5_033,
            }
        });
        let raw2 = serde_json::json!({
            "sessionId": "cache-sess",
            "update": {
                "sessionUpdate": "usage_update",
                "accumulatedInputTokens": 28_500,
                "accumulatedOutputTokens": 310,
                "accumulatedCachedInputTokens": 11_000,
            }
        });

        let mut tracker = crate::usage::UsageTracker::default();

        // Turn 1 — establish baseline (delta unreliable, but cumulative still present).
        tracker.begin_turn("cache-sess");
        if let crate::usage::GooseSessionUpdateVariant::UsageUpdate(p) =
            serde_json::from_value::<crate::usage::GooseSessionUpdateNotification>(raw1)
                .unwrap()
                .update
        {
            tracker.record("cache-sess", &p);
        }
        let t1 = tracker.take().expect("turn 1");

        // Turn 1: cumulative must carry the cache count; turn delta is None (no baseline).
        let (turn1, cum1) = crate::pool::build_turn_metric_counts(&t1);
        // delta_reliable = false on first turn → no turn counts.
        assert!(turn1.is_none(), "first turn: no reliable turn counts");
        let cum1 = cum1.expect("cumulative always present");
        assert_eq!(
            cum1.cache_read_tokens,
            Some(5_033),
            "cumulative.cacheReadTokens must be 5033 after turn 1"
        );

        // Turn 2 — delta reliable.
        tracker.begin_turn("cache-sess");
        if let crate::usage::GooseSessionUpdateVariant::UsageUpdate(p) =
            serde_json::from_value::<crate::usage::GooseSessionUpdateNotification>(raw2)
                .unwrap()
                .update
        {
            tracker.record("cache-sess", &p);
        }
        let t2 = tracker.take().expect("turn 2");

        let (turn2, cum2) = crate::pool::build_turn_metric_counts(&t2);

        let turn2 = turn2.expect("reliable turn counts on turn 2");
        // Per-turn cache delta: 11_000 - 5_033 = 5_967.
        assert_eq!(
            turn2.cache_read_tokens,
            Some(5_967),
            "turn.cacheReadTokens must be the per-turn delta"
        );
        // cache_write_tokens is always None — buzz-agent doesn't emit it.
        assert!(
            turn2.cache_write_tokens.is_none(),
            "cache_write_tokens must be None — not emitted by buzz-agent"
        );

        let cum2 = cum2.expect("cumulative always present");
        assert_eq!(
            cum2.cache_read_tokens,
            Some(11_000),
            "cumulative.cacheReadTokens must be 11_000 after turn 2"
        );
        assert!(
            cum2.cache_write_tokens.is_none(),
            "cache_write_tokens must be None on cumulative too"
        );
    }

    fn make_prompt_context_no_owner() -> PromptContext {
        let agent_keys = nostr::Keys::generate();
        make_prompt_context_impl(&agent_keys, None)
    }

    fn make_prompt_context_with_owner(
        agent_keys: &nostr::Keys,
        owner_pubkey: nostr::PublicKey,
    ) -> PromptContext {
        make_prompt_context_impl(agent_keys, Some(owner_pubkey))
    }

    fn make_prompt_context_impl(
        agent_keys: &nostr::Keys,
        owner_pubkey: Option<nostr::PublicKey>,
    ) -> PromptContext {
        use crate::relay::RestClient;
        PromptContext {
            mcp_servers: vec![],
            initial_message: None,
            idle_timeout: Duration::from_secs(60),
            max_turn_duration: Duration::from_secs(120),
            turn_liveness_interval: Duration::ZERO,
            dedup_mode: DedupMode::Drop,
            system_prompt: None,
            session_title: None,
            team_instructions: None,
            heartbeat_prompt: None,
            base_prompt: None,
            cwd: ".".to_string(),
            rest_client: RestClient {
                http: reqwest::Client::new(),
                base_url: "http://127.0.0.1:0".to_string(),
                keys: agent_keys.clone(),
                auth_tag_json: None,
            },
            channel_info: ChannelInfoResolver::new(
                std::collections::HashMap::new(),
                RestClient {
                    http: reqwest::Client::new(),
                    base_url: "http://127.0.0.1:0".to_string(),
                    keys: agent_keys.clone(),
                    auth_tag_json: None,
                },
            ),
            context_message_limit: 0,
            max_turns_per_session: 0,
            permission_mode: PermissionMode::Default,
            agent_keys: agent_keys.clone(),
            agent_owner_pubkey: owner_pubkey,
            memory_enabled: false,
            harness_name: "goose".to_string(),
            relay_url: "ws://127.0.0.1:3000".to_string(),
            canvas_cache: CanvasRevisionCache::new(),
        }
    }

    // ── canvas_pointer_from_query_response ──────────────────────────────────

    const CHANNEL_UUID: &str = "00f1ccaf-1506-4dd7-9a0e-fa67e9e486ae";

    /// Build a real, cryptographically signed Nostr canvas event for tests.
    ///
    /// Includes the correct kind (40100) and an `h` tag carrying `CHANNEL_UUID`
    /// so all structural and content validations pass.
    fn make_canvas_event_value(content: &str) -> serde_json::Value {
        let keys = Keys::generate();
        let h_tag = Tag::parse(["h", CHANNEL_UUID]).expect("h tag");
        let event = EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), content)
            .tags([h_tag])
            .sign_with_keys(&keys)
            .expect("sign");
        serde_json::to_value(&event).expect("serialise")
    }

    /// Present outcome: correct event → pointer with the event's id and a Z-suffix timestamp.
    #[test]
    fn test_canvas_pointer_from_query_response_present() {
        let ev = make_canvas_event_value("# Team instructions\nBe helpful.");
        let id = ev["id"].as_str().unwrap().to_string();
        let result = canvas_pointer_from_query_response(&[ev], CHANNEL_UUID);
        let pointer = match result {
            CanvasFetchResult::Present(p) => p,
            other => panic!("expected Present, got {other:?}"),
        };
        assert_eq!(pointer.event_id, id, "pointer must carry the event id");
        assert!(
            pointer.timestamp.ends_with('Z'),
            "timestamp must use Z suffix, not +00:00"
        );
        assert!(
            !pointer.timestamp.contains("+00:00"),
            "timestamp must not use +00:00 offset"
        );
    }

    /// Absent outcome: empty array (no canvas event) or blank/empty content (cleared canvas).
    #[test]
    fn test_canvas_pointer_from_query_response_absent_cases() {
        let cases: &[(&str, &dyn Fn() -> Vec<serde_json::Value>)] = &[
            ("empty array", &|| vec![]),
            ("blank content", &|| vec![make_canvas_event_value("   ")]),
            ("empty content", &|| vec![make_canvas_event_value("")]),
        ];
        for (name, make_input) in cases {
            let result = canvas_pointer_from_query_response(&make_input(), CHANNEL_UUID);
            assert!(
                matches!(result, CanvasFetchResult::Absent),
                "{name}: expected Absent, got {result:?}"
            );
        }
    }

    /// Failed outcome: structurally invalid or security-failing events must be
    /// Failed (not Absent) so stale cache entries are preserved rather than cleared.
    #[test]
    fn test_canvas_pointer_from_query_response_failed_cases() {
        // Partial object (missing pubkey/sig/kind/tags)
        let partial = serde_json::json!({
            "id": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
            "created_at": 1705312200_i64,
            "content": "some instructions"
        });

        // String created_at (nostr::Event deserialiser rejects non-integer)
        let string_ts = {
            let keys = Keys::generate();
            let h_tag = Tag::parse(["h", CHANNEL_UUID]).expect("h tag");
            let mut ev = serde_json::to_value(
                EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), "content")
                    .tags([h_tag])
                    .sign_with_keys(&keys)
                    .expect("sign"),
            )
            .expect("serialise");
            ev["created_at"] = serde_json::Value::String("2026-03-15T16:30:00+00:00".into());
            ev
        };

        // Missing created_at field
        let missing_ts = {
            let keys = Keys::generate();
            let h_tag = Tag::parse(["h", CHANNEL_UUID]).expect("h tag");
            let mut ev = serde_json::to_value(
                EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), "content")
                    .tags([h_tag])
                    .sign_with_keys(&keys)
                    .expect("sign"),
            )
            .expect("serialise");
            ev.as_object_mut().unwrap().remove("created_at");
            ev
        };

        // Timestamp::max() — u64::MAX wraps to -1 via i64 cast; checked conversion must reject it
        let ts_max = {
            let keys = Keys::generate();
            let h_tag = Tag::parse(["h", CHANNEL_UUID]).expect("h tag");
            serde_json::to_value(
                EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), "content")
                    .tags([h_tag])
                    .custom_created_at(Timestamp::max())
                    .sign_with_keys(&keys)
                    .expect("sign"),
            )
            .expect("serialise")
        };

        // Tampered event (content altered after signing — id/sig no longer agree)
        let tampered = {
            let keys = Keys::generate();
            let h_tag = Tag::parse(["h", CHANNEL_UUID]).expect("h tag");
            let mut ev = serde_json::to_value(
                EventBuilder::new(
                    Kind::Custom(buzz_core::kind::KIND_CANVAS as u16),
                    "original",
                )
                .tags([h_tag])
                .sign_with_keys(&keys)
                .expect("sign"),
            )
            .expect("serialise");
            ev["content"] = serde_json::Value::String("injected instructions".into());
            ev
        };

        // Wrong kind (not 40100)
        let wrong_kind = {
            let keys = Keys::generate();
            let h_tag = Tag::parse(["h", CHANNEL_UUID]).expect("h tag");
            serde_json::to_value(
                EventBuilder::new(Kind::Custom(9), "content")
                    .tags([h_tag])
                    .sign_with_keys(&keys)
                    .expect("sign"),
            )
            .expect("serialise")
        };

        // Wrong h-tag (different channel UUID)
        let wrong_h = {
            let keys = Keys::generate();
            let h_tag = Tag::parse(["h", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"]).expect("h tag");
            serde_json::to_value(
                EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), "content")
                    .tags([h_tag])
                    .sign_with_keys(&keys)
                    .expect("sign"),
            )
            .expect("serialise")
        };

        let cases: &[(&str, serde_json::Value)] = &[
            ("partial object (missing pubkey/sig/kind/tags)", partial),
            ("string created_at", string_ts),
            ("missing created_at", missing_ts),
            ("Timestamp::max() (u64::MAX)", ts_max),
            ("tampered event (sig mismatch)", tampered),
            ("wrong kind", wrong_kind),
            ("wrong h-tag", wrong_h),
        ];
        for (name, ev) in cases {
            let result = canvas_pointer_from_query_response(std::slice::from_ref(ev), CHANNEL_UUID);
            assert!(
                matches!(result, CanvasFetchResult::Failed),
                "{name}: expected Failed (not Absent — stale cache must be preserved), got {result:?}"
            );
        }
    }

    // ── CanvasRevisionCache tri-state (A1) ────────────────────────────────────

    /// Exercises all eight tri-state transitions of `CanvasRevisionCache` in sequence:
    /// Failed-on-empty → Present(p1) → stale-on-Failed → Present(p2) replaces p1 →
    /// stale-on-Failed serves p2 → Absent clears → no-stale-after-Absent → new Present(p3).
    #[test]
    fn test_canvas_revision_cache_tri_state_transitions() {
        let cache = CanvasRevisionCache::new();
        let ch = Uuid::new_v4();

        let p1 = crate::queue::CanvasPointer {
            event_id: "rev1".to_string(),
            timestamp: "2024-01-15T10:00:00Z".to_string(),
        };
        let p2 = crate::queue::CanvasPointer {
            event_id: "rev2".to_string(),
            timestamp: "2024-01-16T10:00:00Z".to_string(),
        };
        let p3 = crate::queue::CanvasPointer {
            event_id: "rev3".to_string(),
            timestamp: "2024-01-17T10:00:00Z".to_string(),
        };

        // 1. Failed with no prior entry → None (nothing to serve stale)
        assert!(
            cache
                .resolve_for_turn(&ch, CanvasFetchResult::Failed)
                .is_none(),
            "Failed on empty cache must return None"
        );

        // 2. Present → updates cache and returns pointer
        assert_eq!(
            cache.resolve_for_turn(&ch, CanvasFetchResult::Present(p1.clone())),
            Some(p1.clone()),
            "Present must return the new pointer"
        );

        // 3. Failed after Present → stale pointer served; cache entry preserved
        assert_eq!(
            cache.resolve_for_turn(&ch, CanvasFetchResult::Failed),
            Some(p1.clone()),
            "Failed after Present must serve stale value"
        );

        // 4. Present(p2) while p1 is cached → p2 replaces p1 and is returned
        assert_eq!(
            cache.resolve_for_turn(&ch, CanvasFetchResult::Present(p2.clone())),
            Some(p2.clone()),
            "Present(p2) must overwrite Present(p1) and return p2"
        );

        // 5. Failed after replacement → stale p2 served (replacement persisted)
        assert_eq!(
            cache.resolve_for_turn(&ch, CanvasFetchResult::Failed),
            Some(p2.clone()),
            "Failed after replacement must serve the replaced (p2) value"
        );

        // 6. Absent after Present → confirmed deletion; cache cleared; returns None
        assert!(
            cache
                .resolve_for_turn(&ch, CanvasFetchResult::Absent)
                .is_none(),
            "Absent must return None and clear cache"
        );

        // 7. Failed after Absent → no stale entry; returns None
        assert!(
            cache
                .resolve_for_turn(&ch, CanvasFetchResult::Failed)
                .is_none(),
            "Failed after Absent must return None (no cached entry)"
        );

        // 8. New Present after Absent → pointer updated
        assert_eq!(
            cache.resolve_for_turn(&ch, CanvasFetchResult::Present(p3.clone())),
            Some(p3),
            "new revision after Absent must return the new pointer"
        );
    }

    // ── CanvasRevisionCache stale-preservation: Failed from parser preserves cache (A1) ─

    /// Proves that a `Failed` parse result (malformed event, tampered sig, or wrong h-tag)
    /// leaves the cache entry intact so the stale pointer is served rather than cleared.
    #[test]
    #[allow(clippy::type_complexity)]
    fn test_cache_stale_preserved_on_parser_failure() {
        // Build a parse-Failed input for each failure mode.
        let make_malformed = |_ch_str: &str| -> serde_json::Value {
            serde_json::json!({
                "id": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
                "created_at": 1705312200_i64,
                "content": "some instructions"
            })
        };
        let make_tampered = |ch_str: &str| -> serde_json::Value {
            let keys = Keys::generate();
            let h_tag = Tag::parse(["h", ch_str]).expect("h tag");
            let mut ev = serde_json::to_value(
                EventBuilder::new(
                    Kind::Custom(buzz_core::kind::KIND_CANVAS as u16),
                    "original",
                )
                .tags([h_tag])
                .sign_with_keys(&keys)
                .expect("sign"),
            )
            .expect("serialise");
            ev["content"] = serde_json::Value::String("injected".into());
            ev
        };
        let make_wrong_h = |_ch_str: &str| -> serde_json::Value {
            let keys = Keys::generate();
            let wrong_h = Tag::parse(["h", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"]).expect("h tag");
            serde_json::to_value(
                EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), "content")
                    .tags([wrong_h])
                    .sign_with_keys(&keys)
                    .expect("sign"),
            )
            .expect("serialise")
        };

        let cases: &[(&str, &dyn Fn(&str) -> serde_json::Value)] = &[
            ("malformed event (partial object)", &make_malformed),
            ("tampered event (sig mismatch)", &make_tampered),
            ("wrong h-tag", &make_wrong_h),
        ];

        for (name, make_ev) in cases {
            let cache = CanvasRevisionCache::new();
            let ch = Uuid::new_v4();
            let prior = crate::queue::CanvasPointer {
                event_id: format!("stale-{name}"),
                timestamp: "2024-01-15T10:30:00Z".to_string(),
            };
            cache.resolve_for_turn(&ch, CanvasFetchResult::Present(prior.clone()));

            let ch_str = ch.to_string();
            let ev = make_ev(&ch_str);
            let parse_result = canvas_pointer_from_query_response(&[ev], &ch_str);
            assert!(
                matches!(parse_result, CanvasFetchResult::Failed),
                "{name}: input must produce Failed from parser"
            );
            let served = cache.resolve_for_turn(&ch, parse_result);
            assert_eq!(
                served,
                Some(prior),
                "{name}: Failed must serve stale pointer, not clear the cache"
            );
        }
    }

    // ── ChannelInfoResolver TTL (A3) ──────────────────────────────────────────

    // ── new-session channel context (one resolve, two consumers) ─────────────

    /// A [`ChannelInfoResolver`] whose lazy REST fallback is served by a local
    /// HTTP server, plus a counter of the requests that actually reached it.
    /// Counting real requests is the point: the composition tests are pure and
    /// cannot see duplicated I/O.
    async fn counting_resolver(
        response: serde_json::Value,
    ) -> (
        ChannelInfoResolver,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test HTTP server");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        let body = response.to_string();
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0; 8192];
                let _ = socket.read(&mut buf).await;
                server_requests.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        let rest = crate::relay::RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: nostr::Keys::generate(),
            auth_tag_json: None,
        };
        (
            ChannelInfoResolver::new(std::collections::HashMap::new(), rest),
            requests,
            server,
        )
    }

    fn channel_metadata_response(id: Uuid, tags: &[[&str; 2]]) -> serde_json::Value {
        let mut event_tags = vec![json!(["d", id.to_string()])];
        event_tags.extend(tags.iter().map(|[k, v]| json!([k, v])));
        json!([{ "tags": event_tags }])
    }

    /// A normal channel resolves to its name and type; cached second call hits no wire.
    #[tokio::test]
    async fn test_channel_resolver_qualifies_a_normal_channel() {
        use std::sync::atomic::Ordering;

        let id = Uuid::new_v4();
        let response = channel_metadata_response(id, &[["name", "buzz-dev"], ["t", "stream"]]);
        let (resolver, requests, server) = counting_resolver(response).await;

        let info = resolver.resolve(id).await.expect("should resolve");
        assert_eq!(info.name, "buzz-dev");
        assert_eq!(info.channel_type, "stream");
        assert!(!info.name.is_empty());
        assert_eq!(requests.load(Ordering::SeqCst), 1);

        // Second resolve hits the cache, not the wire.
        let again = resolver.resolve(id).await.expect("cached resolve");
        assert_eq!(again.name, "buzz-dev");
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "a resolved channel is cached — no second lookup"
        );
        server.abort();
    }

    /// A DM channel resolves with the correct type.
    #[tokio::test]
    async fn test_channel_resolver_resolves_dm_type() {
        let id = Uuid::new_v4();
        let response = channel_metadata_response(id, &[["name", "DM"], ["t", "dm"]]);
        let (resolver, _requests, server) = counting_resolver(response).await;

        let info = resolver.resolve(id).await.expect("should resolve");
        assert_eq!(info.channel_type, "dm");
        server.abort();
    }

    /// The `"unknown"` placeholder is returned as-is — callers exclude it from titles.
    #[tokio::test]
    async fn test_channel_resolver_returns_unknown_name_for_unnamed_channel() {
        let id = Uuid::new_v4();
        let response = channel_metadata_response(id, &[["t", "stream"]]);
        let (resolver, _requests, server) = counting_resolver(response).await;

        let info = resolver.resolve(id).await.expect("should resolve");
        assert_eq!(
            info.name, "unknown",
            "nameless channel gets unknown placeholder"
        );
        server.abort();
    }

    /// An unresolvable channel costs exactly ONE `fetch_channel_info` sequence —
    /// two HTTP attempts (initial + one retry from fetch_with_retry). The resolver
    /// does not cache failures, so a second call will retry.
    #[tokio::test]
    async fn test_channel_resolver_attempts_unresolved_channel_once() {
        use std::sync::atomic::Ordering;

        let (resolver, requests, server) = counting_resolver(json!([])).await;

        let result = resolver.resolve(Uuid::new_v4()).await;
        assert!(result.is_none(), "unresolvable channel returns None");
        assert_eq!(
            requests.load(Ordering::SeqCst),
            2,
            "one fetch_channel_info sequence (initial attempt + single retry)"
        );
        server.abort();
    }

    /// Channel description is delivered through resolver when present in metadata.
    #[tokio::test]
    async fn test_channel_resolver_delivers_description() {
        let id = Uuid::new_v4();
        let response = channel_metadata_response(
            id,
            &[
                ["name", "team-chat"],
                ["t", "stream"],
                ["about", "Engineering discussions"],
            ],
        );
        let (resolver, _requests, server) = counting_resolver(response).await;

        let info = resolver.resolve(id).await.expect("should resolve");
        assert_eq!(info.description.as_deref(), Some("Engineering discussions"));
        server.abort();
    }

    /// TTL expiry triggers a background refresh; stale value is served immediately
    /// while the refresh runs (stale-while-revalidate).
    #[tokio::test]
    async fn test_channel_resolver_ttl_serves_stale_on_expiry() {
        use std::sync::atomic::Ordering;

        let id = Uuid::new_v4();
        let response = channel_metadata_response(id, &[["name", "buzz-dev"], ["t", "stream"]]);
        let (resolver, requests, server) = counting_resolver(response).await;

        // Warm the cache.
        let info = resolver.resolve(id).await.expect("should resolve");
        assert_eq!(info.name, "buzz-dev");
        let initial_requests = requests.load(Ordering::SeqCst);

        // Manually expire the entry by setting refresh_after to the past.
        {
            let mut cache = resolver.cache.write().unwrap();
            if let Some(entry) = cache.get_mut(&id) {
                entry.refresh_after =
                    tokio::time::Instant::now() - std::time::Duration::from_secs(1);
            }
        }

        // resolve() returns stale immediately and spawns background refresh.
        let stale = resolver.resolve(id).await.expect("stale serve");
        assert_eq!(
            stale.name, "buzz-dev",
            "stale value served while refresh runs"
        );

        // Give background refresh time to complete.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let after_refresh = requests.load(Ordering::SeqCst);
        assert!(
            after_refresh > initial_requests,
            "background refresh must have fired"
        );
        server.abort();
    }

    /// On a failed TTL refresh the entry's next_refresh_at is bumped by backoff,
    /// preventing immediate re-retry on every subsequent consumer.
    #[tokio::test]
    async fn test_channel_resolver_ttl_backoff_on_failed_refresh() {
        use std::sync::atomic::Ordering;

        let id = Uuid::new_v4();
        // Use a new resolver seeded with the channel at an already-expired TTL.
        let (resolver, requests, server) = counting_resolver(json!([])).await;

        // Manually seed the cache with an expired entry pointing at the non-responding server.
        {
            let mut cache = resolver.cache.write().unwrap();
            cache.insert(
                id,
                CachedChannelInfo {
                    info: PromptChannelInfo {
                        name: "buzz-dev".to_string(),
                        channel_type: "stream".to_string(),
                        description: None,
                    },
                    refresh_after: tokio::time::Instant::now() - std::time::Duration::from_secs(1),
                    next_refresh_at: tokio::time::Instant::now()
                        - std::time::Duration::from_secs(1),
                },
            );
        }

        // First resolve: serves stale, spawns background refresh (which fails).
        let info = resolver.resolve(id).await.expect("stale serve");
        assert_eq!(info.name, "buzz-dev");

        // Give background refresh time to fail and set backoff.
        // fetch_with_retry sleeps CONTEXT_FETCH_RETRY_DELAY (500 ms) between its two
        // attempts, so we must wait longer than that before inspecting the cache.
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

        // Verify: next_refresh_at should have been bumped by backoff.
        {
            let cache = resolver.cache.read().unwrap();
            let entry = cache.get(&id).expect("entry still present");
            // next_refresh_at should be in the future after backoff.
            assert!(
                entry.next_refresh_at > tokio::time::Instant::now(),
                "next_refresh_at must be in the future after failed refresh backoff"
            );
        }

        let after = requests.load(Ordering::SeqCst);
        assert!(
            after >= 2,
            "failed refresh should have attempted (fetch_with_retry = 2 attempts)"
        );
        server.abort();
    }

    // ── Per-key in-flight refresh guard (A3) ─────────────────────────────────

    /// Two concurrent callers on an expired cache entry must spawn exactly one
    /// background refresh — the `in_flight` guard prevents a stampede.
    ///
    /// Uses a slow-responding server (200ms delay) so both callers race to
    /// claim the in-flight slot before the first refresh completes.
    #[tokio::test]
    async fn test_channel_resolver_concurrent_expiry_spawns_one_refresh() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let id = Uuid::new_v4();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();

        let channel_id_str = id.to_string();
        // A well-formed channel-metadata response that includes the channel id.
        let body = serde_json::json!([{"tags":[["d", channel_id_str],["name","team-chat"],["t","stream"]]}]).to_string();

        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = socket.read(&mut buf).await;
                server_requests.fetch_add(1, Ordering::SeqCst);
                // Slow response (100 ms) so a second caller races the first.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let rest = crate::relay::RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: nostr::Keys::generate(),
            auth_tag_json: None,
        };
        let resolver = ChannelInfoResolver::new(std::collections::HashMap::new(), rest);

        // Warm the cache with an already-expired entry.
        {
            let mut cache = resolver.cache.write().unwrap();
            cache.insert(
                id,
                CachedChannelInfo {
                    info: crate::queue::PromptChannelInfo {
                        name: "team-chat".to_string(),
                        channel_type: "stream".to_string(),
                        description: None,
                    },
                    refresh_after: tokio::time::Instant::now() - std::time::Duration::from_secs(1),
                    next_refresh_at: tokio::time::Instant::now()
                        - std::time::Duration::from_secs(1),
                },
            );
        }

        let baseline = requests.load(Ordering::SeqCst);

        // Two concurrent resolve() calls on the same expired entry.
        let r1 = resolver.resolve(id);
        let r2 = resolver.resolve(id);
        let (s1, s2) = tokio::join!(r1, r2);
        assert_eq!(s1.as_ref().map(|i| i.name.as_str()), Some("team-chat"));
        assert_eq!(s2.as_ref().map(|i| i.name.as_str()), Some("team-chat"));

        // Wait long enough for the background refresh to complete (100 ms server
        // delay + fetch overhead).
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let fired = requests.load(Ordering::SeqCst) - baseline;
        assert_eq!(
            fired, 1,
            "exactly one background refresh must be spawned for concurrent expiry callers"
        );

        server.abort();
    }

    // ── query_once is single-attempt (A4) ─────────────────────────────────────

    /// `RestClient::query_once` issues exactly one HTTP request even when the
    /// server returns a retriable status (503), verifying it does not inherit
    /// the retry loop of `request_with_retry`.
    #[tokio::test]
    async fn test_query_once_makes_exactly_one_request_on_retriable_response() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();

        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = socket.read(&mut buf).await;
                server_requests.fetch_add(1, Ordering::SeqCst);
                // Always return 503 Service Unavailable — a retriable status.
                let response =
                    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let rest = crate::relay::RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: nostr::Keys::generate(),
            auth_tag_json: None,
        };

        let filter = nostr::Filter::new().kind(nostr::Kind::Custom(39000));
        let result = rest.query_once(&[filter]).await;

        assert!(
            result.is_err(),
            "query_once must return Err on a non-2xx response"
        );
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "query_once must issue exactly one HTTP request — no retry loop"
        );

        server.abort();
    }

    // ── initial_message context composition (A2) ──────────────────────────────

    /// `initial_message` for a channel turn: the `[Context]` block prepended
    /// must show `Scope: channel`, include the description, and include the
    /// canvas revision — same renderer as the batch prompt.
    #[test]
    fn test_initial_message_context_channel_has_scope_description_canvas() {
        let channel_id = Uuid::from_u128(0x0001);
        let channel_info = crate::queue::PromptChannelInfo {
            name: "team-chat".to_string(),
            channel_type: "stream".to_string(),
            description: Some("Engineering discussions".to_string()),
        };
        let canvas = crate::queue::CanvasPointer {
            event_id: "abc123".to_string(),
            timestamp: "2024-06-01T12:00:00Z".to_string(),
        };

        // This is the exact call made by initial_message in run_prompt_task (A2).
        let ctx_block = crate::queue::format_context_hints(
            channel_id,
            Some(&channel_info),
            &crate::queue::ThreadTags::default(),
            false, // is_dm_turn = false
            false,
            None,
            Some(&canvas),
        );

        assert!(
            ctx_block.contains("Scope: channel"),
            "channel turn must have Scope: channel; got:\n{ctx_block}"
        );
        assert!(
            ctx_block.contains("Description: Engineering discussions"),
            "channel turn must include description; got:\n{ctx_block}"
        );
        assert!(
            ctx_block.contains("Canvas revision (event ID): abc123"),
            "channel turn must include canvas revision; got:\n{ctx_block}"
        );
    }

    /// `initial_message` for a DM turn: `[Context]` must show `Scope: dm`,
    /// no description, and no canvas revision — DM fail-closed.
    #[test]
    fn test_initial_message_context_dm_has_scope_dm_no_description_no_canvas() {
        let channel_id = Uuid::from_u128(0x0002);
        let dm_info = crate::queue::PromptChannelInfo {
            name: "Direct Message".to_string(),
            channel_type: "dm".to_string(),
            description: Some("should be suppressed".to_string()),
        };
        let canvas = crate::queue::CanvasPointer {
            event_id: "def456".to_string(),
            timestamp: "2024-06-01T12:00:00Z".to_string(),
        };

        // is_dm_turn = true; canvas should be suppressed regardless of pointer presence.
        let ctx_block = crate::queue::format_context_hints(
            channel_id,
            Some(&dm_info),
            &crate::queue::ThreadTags::default(),
            true, // is_dm_turn = true
            false,
            None,
            Some(&canvas),
        );

        assert!(
            ctx_block.contains("Scope: dm"),
            "DM turn must have Scope: dm; got:\n{ctx_block}"
        );
        assert!(
            !ctx_block.contains("Description:"),
            "DM turn must not include description; got:\n{ctx_block}"
        );
        assert!(
            !ctx_block.contains("Canvas revision (event ID):"),
            "DM turn must not include canvas revision; got:\n{ctx_block}"
        );
    }

    // ── Unresolved-metadata fail-closed classification (A1/A2) ───────────────

    /// When channel metadata cannot be resolved (None), the turn's authoritative
    /// `is_dm_turn = true` must flow through to BOTH prompt paths:
    ///
    /// 1. `initial_message` (via `format_context_hints` with `is_dm = true`)
    /// 2. Batch prompt (via `FormatPromptArgs::is_dm = true`)
    ///
    /// Both must render `Scope: dm`, omit description, omit canvas pointer, and
    /// select DM conversation behavior — never silently fall back to channel scope.
    #[test]
    fn test_unresolved_metadata_both_prompts_render_scope_dm() {
        use crate::queue::{
            format_context_hints, format_prompt, CanvasPointer, FlushBatch, FormatPromptArgs,
            ThreadTags,
        };
        use nostr::Timestamp;

        let channel_id = Uuid::from_u128(0x0003);
        // Simulate a canvas pointer that must be suppressed for DM turns.
        let canvas = CanvasPointer {
            event_id: "cafebabe".to_string(),
            timestamp: "2024-06-01T12:00:00Z".to_string(),
        };

        // ── initial_message path ─────────────────────────────────────────────
        // run_prompt_task passes: channel_info = None (unresolved), is_dm_turn = true.
        let init_ctx = format_context_hints(
            channel_id,
            None, // channel_info = None (unresolved metadata)
            &ThreadTags::default(),
            true, // is_dm_turn = true (fail-closed)
            false,
            None,
            Some(&canvas), // canvas pointer present — must be suppressed
        );

        assert!(
            init_ctx.contains("Scope: dm"),
            "initial_message: unresolved metadata must render Scope: dm; got:\n{init_ctx}"
        );
        assert!(
            !init_ctx.contains("Description:"),
            "initial_message: unresolved metadata must not include description; got:\n{init_ctx}"
        );
        assert!(
            !init_ctx.contains("Canvas revision (event ID):"),
            "initial_message: unresolved metadata must not include canvas; got:\n{init_ctx}"
        );

        // ── batch prompt path ────────────────────────────────────────────────
        // FormatPromptArgs::is_dm = true must govern rendering, not channel_info.
        let event = {
            let keys = nostr::Keys::generate();
            nostr::EventBuilder::new(nostr::Kind::Custom(1), "hello")
                .custom_created_at(Timestamp::from(1_700_000_000u64))
                .sign_with_keys(&keys)
                .expect("sign")
        };
        let batch = FlushBatch {
            channel_id,
            events: vec![crate::queue::BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };

        let prompt = format_prompt(
            &batch,
            &FormatPromptArgs {
                channel_info: None,            // unresolved metadata
                is_dm: true,                   // fail-closed classification
                canvas_pointer: Some(&canvas), // must be suppressed
                ..Default::default()
            },
        )
        .join("\n\n");

        assert!(
            prompt.contains("Scope: dm"),
            "batch prompt: unresolved metadata must render Scope: dm; got:\n{prompt}"
        );
        assert!(
            !prompt.contains("Description:"),
            "batch prompt: unresolved metadata must not include description; got:\n{prompt}"
        );
        assert!(
            !prompt.contains("Canvas revision (event ID):"),
            "batch prompt: unresolved metadata must not include canvas; got:\n{prompt}"
        );
    }

    // ── Unresolved-metadata: fetch_conversation_context DM selection (A1/A2) ──

    /// Proves the two-sided classification behavior of `fetch_conversation_context`:
    ///
    /// * `is_dm_turn = false` → skips the DM history fetch entirely (None, zero requests).
    /// * `is_dm_turn = true`  → issues the DM history fetch and returns
    ///   `Some(ConversationContext::Dm)` when the response is non-empty.
    ///
    /// The end-to-end path through `run_prompt_task` (fail-closed resolver +
    /// forwarding) is covered by
    /// `test_run_prompt_task_dm_classification_forwarding_is_end_to_end_authoritative`.
    #[tokio::test]
    async fn test_fetch_conversation_context_dm_classification_is_authoritative() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // A single valid DM event body the server returns for true-arm calls.
        let dm_response = serde_json::json!([{
            "content": "hello from dm",
            "created_at": 1_700_000_000u64,
            "pubkey": "aabbccdd"
        }])
        .to_string();
        let server_dm_response = dm_response.clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let requests = std::sync::Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();

        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let _ = socket.read(&mut buf).await;
                server_requests.fetch_add(1, Ordering::SeqCst);
                let _ = socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            server_dm_response.len(),
                            server_dm_response
                        )
                        .as_bytes(),
                    )
                    .await;
            }
        });

        let agent_keys = nostr::Keys::generate();
        let rest = crate::relay::RestClient {
            http: reqwest::Client::new(),
            base_url,
            keys: agent_keys.clone(),
            auth_tag_json: None,
        };

        let make_batch = |channel_id: Uuid| {
            let keys = nostr::Keys::generate();
            let event = nostr::EventBuilder::new(nostr::Kind::Custom(1), "msg")
                .custom_created_at(nostr::Timestamp::from(1_700_000_000u64))
                .sign_with_keys(&keys)
                .expect("sign");
            FlushBatch {
                channel_id,
                events: vec![crate::queue::BatchEvent {
                    event,
                    prompt_tag: "test".into(),
                    received_at: std::time::Instant::now(),
                }],
                cancelled_events: vec![],
                cancel_reason: None,
            }
        };

        let ctx = PromptContext {
            rest_client: rest,
            context_message_limit: 10,
            agent_keys,
            ..make_prompt_context_no_owner()
        };

        // ── false arm ─────────────────────────────────────────────────────────
        // is_dm_turn = false on a non-thread batch → must return None and issue zero requests.
        let requests_before = requests.load(Ordering::SeqCst);
        let result =
            fetch_conversation_context(&make_batch(Uuid::from_u128(0x1002)), &ctx, false).await;

        assert!(
            result.is_none(),
            "is_dm_turn=false on a non-thread turn must return None; got: {result:?}"
        );
        assert_eq!(
            requests.load(Ordering::SeqCst),
            requests_before,
            "is_dm_turn=false must not issue any HTTP request to the DM history endpoint"
        );

        // ── true arm ──────────────────────────────────────────────────────────
        // is_dm_turn = true → must issue the DM history fetch and return Some(Dm).
        let result =
            fetch_conversation_context(&make_batch(Uuid::from_u128(0x1003)), &ctx, true).await;

        assert!(
            matches!(result, Some(ConversationContext::Dm { .. })),
            "is_dm_turn=true with a non-empty DM response must return Some(ConversationContext::Dm); got: {result:?}"
        );

        server.abort();
    }

    // ── A2: run_prompt_task DM forwarding end-to-end ────────────────────────

    /// Proves the fail-closed `is_dm_turn` classification at `pool.rs:1616-1632`
    /// and its forwarding at `pool.rs:1942` as ONE tested behavior by entering
    /// through `run_prompt_task` with **unresolved** channel metadata and a
    /// scripted bash agent.
    ///
    /// ## How fail-close is exercised
    ///
    /// The channel is NOT pre-seeded in `ChannelInfoResolver`. On the slow
    /// path, `resolve()` calls `fetch_channel_info`, which issues a kind-39000
    /// metadata query. The test server returns `[]` for kind-39000 (a valid
    /// empty array — no metadata event), causing `fetch_channel_info` to return
    /// `None` after its two attempts (initial + `fetch_with_retry` once). With
    /// `resolve()` returning `None`, the classification at `pool.rs:1622`
    /// reaches `.unwrap_or(true)` — fail-closed — setting `is_dm_turn = true`.
    /// That value is forwarded to `fetch_conversation_context` at `pool.rs:1942`,
    /// which then issues a kind-40002 DM history query. The kind-40002 counter
    /// proves the entire chain is one tested behavior.
    ///
    /// ## Mutation checks
    ///
    /// 1. `.unwrap_or(true)` → `.unwrap_or(false)` at `pool.rs:1622`:
    ///    `is_dm_turn` becomes `false`; `fetch_conversation_context` skips the
    ///    DM fetch (no thread root, not DM → `None`, zero kind-40002 requests).
    ///    The `dm_requests >= 1` assertion fails.
    ///
    /// 2. `is_dm_turn` → hardcoded `false` at `pool.rs:1942`:
    ///    Same outcome — DM fetch skipped, assertion fails.
    ///
    /// Both mutations were verified before push; see the hand-back message for
    /// explicit confirmation that both tripped.
    #[tokio::test]
    async fn test_run_prompt_task_dm_classification_forwarding_is_end_to_end_authoritative() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // ── HTTP server: dispatches on request kind ───────────────────────────
        // kind-39000 (NIP-29 group metadata): returns `[]` — no metadata event.
        //   → fetch_channel_info returns None → resolve() returns None
        //   → .unwrap_or(true) fires → is_dm_turn = true (fail-closed)
        // kind-40002 (KIND_STREAM_MESSAGE_V2, DM history): returns a minimal
        //   DM event array; the request is counted in dm_requests.
        // kind-0 (profile lookup): returns `[]` (not counted).
        //
        // fetch_with_retry issues the kind-39000 call twice (initial attempt +
        // one retry after CONTEXT_FETCH_RETRY_DELAY=500 ms); both return `[]`.
        let dm_body = serde_json::json!([{
            "content": "hello from dm",
            "created_at": 1_700_000_000u64,
            "pubkey": "aabb"
        }])
        .to_string();
        let empty_body = "[]".to_string();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let base_url = format!("http://{}", listener.local_addr().unwrap());

        // `dm_requests` counts only requests whose body contains kind 40002
        // (KIND_STREAM_MESSAGE_V2). kind-39000 metadata queries and kind-0
        // profile lookups are never counted here.
        let dm_requests = std::sync::Arc::new(AtomicUsize::new(0));
        let server_dm_requests = dm_requests.clone();
        let server_dm_body = dm_body.clone();
        let server_empty_body = empty_body.clone();

        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let raw = String::from_utf8_lossy(&buf[..n]);

                let body = if raw.contains("40002") {
                    // DM history query — count it and return the DM event.
                    server_dm_requests.fetch_add(1, Ordering::SeqCst);
                    server_dm_body.clone()
                } else {
                    // kind-39000 metadata query or kind-0 profile lookup —
                    // return an empty array so fetch_channel_info returns None.
                    server_empty_body.clone()
                };

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        // ── Scripted bash agent: responds to session/new (id=0) then ─────────
        // session/prompt (id=1). Two kind-39000 fetches (fetch_with_retry)
        // plus the 500 ms retry sleep occur before the DM history fetch, so the
        // script must tolerate a ~1–2 s pause; read -t 10 absorbs that.
        let script = r#"
            read -t 10 _req1
            printf '{"jsonrpc":"2.0","id":0,"result":{"sessionId":"ses-dm-test"}}\n'
            read -t 10 _req2
            printf '{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}\n'
            sleep 1
        "#;

        let acp = crate::acp::AcpClient::spawn(
            "bash",
            &["-c".to_string(), script.to_string()],
            &[],
            false,
        )
        .await
        .expect("spawn bash agent");

        // ── OwnedAgent: protocol_version=2, agent_name not "goose" ───────────
        let agent_keys = nostr::Keys::generate();
        let agent = OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            agent_name: "test-bash".into(),
            goose_system_prompt_supported: None,
            protocol_version: 2,
        };

        // ── PromptContext: channel NOT pre-seeded — resolver takes the slow ───
        // path and issues kind-39000 queries to the test server. Both
        // rest_client and channel_info_rest point to the same server so all
        // HTTP traffic is observable.
        let channel_id = Uuid::from_u128(0x2001);
        let rest_client = crate::relay::RestClient {
            http: reqwest::Client::new(),
            base_url: base_url.clone(),
            keys: agent_keys.clone(),
            auth_tag_json: None,
        };
        let channel_info_rest = crate::relay::RestClient {
            http: reqwest::Client::new(),
            base_url: base_url.clone(),
            keys: agent_keys.clone(),
            auth_tag_json: None,
        };
        let ctx = std::sync::Arc::new(PromptContext {
            rest_client,
            context_message_limit: 10,
            agent_keys: agent_keys.clone(),
            channel_info: ChannelInfoResolver::new(
                std::collections::HashMap::new(), // no startup entries
                channel_info_rest,
            ),
            ..make_prompt_context_no_owner()
        });

        // ── FlushBatch: no thread tags → DM non-reply path in ────────────────
        // fetch_conversation_context. A fresh event pubkey triggers a kind-0
        // profile lookup (always issued, never counted in dm_requests).
        let event = {
            let event_keys = nostr::Keys::generate();
            nostr::EventBuilder::new(nostr::Kind::Custom(1), "hello dm")
                .custom_created_at(nostr::Timestamp::from(1_700_000_000u64))
                .sign_with_keys(&event_keys)
                .expect("sign")
        };
        let batch = FlushBatch {
            channel_id,
            events: vec![crate::queue::BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };

        // ── run_prompt_task: fire and receive the PromptResult ────────────────
        let (result_tx, mut result_rx) = tokio::sync::mpsc::unbounded_channel::<PromptResult>();
        run_prompt_task(
            agent,
            Some(batch),
            None,
            ctx,
            result_tx,
            None, // control_rx: None = non-cancellable path
            "turn-dm-test".to_string(),
        )
        .await;

        // Drain the result channel (run_prompt_task always sends one PromptResult).
        let _result = result_rx.try_recv().expect("PromptResult must be sent");

        // ── Assertion: DM fetch (kind-40002 query) must have reached the server ─
        // Driven by the fail-closed path: resolve() returns None →
        // .unwrap_or(true) → is_dm_turn = true → forwarded to
        // fetch_conversation_context at pool.rs:1942 → kind-40002 query issued.
        //
        // Mutation 1 — pool.rs:1622 .unwrap_or(false): is_dm_turn = false →
        //   DM fetch skipped → dm_requests stays 0 → assertion FAILS.
        // Mutation 2 — pool.rs:1942 hardcoded false: same outcome → FAILS.
        let total_dm_requests = dm_requests.load(Ordering::SeqCst);
        assert!(
            total_dm_requests >= 1,
            "run_prompt_task with unresolved channel metadata must issue at least \
             one DM history query (kind 40002) via the fail-closed is_dm_turn path; \
             got {total_dm_requests} — if this is 0, either the fail-closed \
             .unwrap_or(true) at pool.rs:1622 or the is_dm_turn forwarding at \
             pool.rs:1942 has regressed"
        );

        server.abort();
    }
}
