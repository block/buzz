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
    extract_model_config_options, extract_model_state, extract_thought_level_config_id,
    model_in_catalog, resolve_model_switch_method, AcpClient, AcpError, EnvVar, McpServer,
    ModelSwitchMethod, StopReason, SystemPromptTransport,
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
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SuccessfulSteerDelivery {
    pub event_id: String,
    pub session_id: String,
}

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
    /// Successful non-cancelling steers acknowledged while this task owned the
    /// live session. The session ID prevents a late ack from contaminating a
    /// replacement session after task return.
    pub successful_steer_deliveries: HashSet<SuccessfulSteerDelivery>,
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
    /// B5: configId for the `thought_level` category option, if the adapter
    /// advertised one in session/new. Resolved at session time so the
    /// spawn-scoped effort application forwards the adapter's real configId
    /// instead of hardcoding it. `None` when the adapter advertises no
    /// `thought_level` option.
    pub thought_level_config_id: Option<String>,
}

/// Successful deliveries associated with one live channel session.
#[derive(Default)]
pub struct ChannelDeliveryState {
    /// Whether a legacy user message has successfully carried standing context.
    pub standing_context_sent: bool,
    /// Buzz event IDs already delivered to this ACP session, either as trigger
    /// events or conversation context.
    pub delivered_event_ids: HashSet<String>,
}

/// Per-channel session IDs, turn counters, and delivery state.
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
    /// Whether the live heartbeat session has successfully received `[Base]`.
    pub heartbeat_standing_context_sent: bool,
    /// channel_id → rendered NIP-AE core prompt section, populated once at
    /// session creation per Tyler's spec (no mid-session refresh).
    pub core_sections: HashMap<Uuid, String>,
    /// channel_id → rendered `[Channel Canvas]` metadata section.
    ///
    /// Populated once before session creation (same lifecycle as `core_sections`).
    /// Absent when the channel has no canvas, the canvas content is blank, or the
    /// fetch fails — all fail open. Cleared on session invalidation alongside
    /// `core_sections` so the next session picks up any canvas change.
    pub canvas_sections: HashMap<Uuid, String>,
    /// Per-channel successful-delivery state. Created with the ACP session and
    /// cleared atomically with every invalidation path.
    pub deliveries: HashMap<Uuid, ChannelDeliveryState>,
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
                self.heartbeat_standing_context_sent = false;
            }
        }
    }

    /// Invalidate a single channel's session and turn counter.
    /// Returns `true` if the channel had an active session.
    pub fn invalidate_channel(&mut self, channel_id: &Uuid) -> bool {
        self.turn_counts.remove(channel_id);
        self.core_sections.remove(channel_id);
        self.canvas_sections.remove(channel_id);
        self.deliveries.remove(channel_id);
        self.sessions.remove(channel_id).is_some()
    }

    /// Invalidate all sessions and turn counters (e.g. after agent exit).
    pub fn invalidate_all(&mut self) {
        self.sessions.clear();
        self.turn_counts.clear();
        self.heartbeat_session = None;
        self.heartbeat_turn_count = 0;
        self.heartbeat_standing_context_sent = false;
        self.core_sections.clear();
        self.canvas_sections.clear();
        self.deliveries.clear();
    }

    pub(crate) fn mark_channel_delivery_success(
        &mut self,
        channel_id: Uuid,
        standing_context_sent: bool,
        event_ids: impl IntoIterator<Item = String>,
    ) {
        let delivery = self.deliveries.entry(channel_id).or_default();
        delivery.standing_context_sent |= standing_context_sent;
        delivery.delivered_event_ids.extend(event_ids);
    }

    #[cfg(test)]
    fn has_channel_state(&self, channel_id: &Uuid) -> bool {
        self.sessions.contains_key(channel_id)
            || self.turn_counts.contains_key(channel_id)
            || self.core_sections.contains_key(channel_id)
            || self.canvas_sections.contains_key(channel_id)
            || self.deliveries.contains_key(channel_id)
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
    /// Opaque per-pick `request_id` from the live `SwitchModel` that set
    /// `desired_model`, echoed on the late `control_result` frame so the
    /// Desktop ModelPicker can correlate it to the pick that fired the switch.
    /// `None` for config/persona-derived models (no live pick to correlate).
    pub desired_model_request_id: Option<String>,
    /// True when a busy-path live switch is awaiting its deferred apply: the
    /// switch was delivered to an in-flight turn (`sent` ack), the turn was
    /// cancelled+requeued, and the real apply runs at the next session. On that
    /// apply, `create_session_and_apply_model` emits a positive terminal
    /// `control_result` (success) so the Desktop learns the outcome instead of
    /// inferring it from timeout silence. The idle path never sets this — it
    /// already emits its terminal immediately — so this gate prevents a
    /// double-emit there. Consumed (reset) at apply time.
    pub desired_model_pending_ack: bool,
    /// Persisted startup effort value from `BUZZ_ACP_EFFORT_LEVEL` (carried from
    /// the Desktop record via `Config.effort_level`). Held per-worker and applied
    /// once, at the first session creation, by pairing with the adapter's
    /// advertised `thought_level` configId. This is spawn-scoped only — there is
    /// no pool-level effort state and no live mid-conversation effort switching.
    /// Non-fatal when absent or when the adapter does not advertise
    /// `thought_level`.
    pub startup_effort: Option<String>,
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
        ControlSignal::Rotate | ControlSignal::SwitchModel { .. }
    ) {
        state.invalidate(source);
    }
}

/// Control signal for an in-flight channel turn.
///
/// Not `Copy`: `SwitchModel` carries owned `String`s. Callers must clone when
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
    ///
    /// Carries `(model_id, request_id)`: the opaque per-pick `request_id`
    /// originates in the Desktop ModelPicker and is echoed on every
    /// `control_result` frame so a replayed result cannot settle a later pick.
    SwitchModel {
        model_id: String,
        request_id: Option<String>,
    },
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
    Success { session_id: String },
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

/// Immutable config subset shared (via `Arc`) by all spawned prompt tasks.
///
/// Built once from `Config` at startup. Avoids cloning the full config
/// into every task.
/// Shared channel-metadata resolver for startup-known and dynamically joined channels.
///
/// Successful lazy lookups are cached for every consumer (author gate, prompt
/// context, canvas, and setup mode). Unknown metadata is never cached as a
/// non-DM: callers can fail closed and a later event retries resolution.
#[derive(Debug, Clone)]
pub struct ChannelInfoResolver {
    cache: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<Uuid, PromptChannelInfo>>>,
    rest_client: RestClient,
}

impl ChannelInfoResolver {
    pub fn new(
        startup: std::collections::HashMap<Uuid, ChannelInfo>,
        rest_client: RestClient,
    ) -> Self {
        let cache = startup
            .into_iter()
            .filter_map(|(id, info)| {
                (info.channel_type != "unknown").then_some((
                    id,
                    PromptChannelInfo {
                        name: info.name,
                        channel_type: info.channel_type,
                        description: info.description,
                    },
                ))
            })
            .collect();
        Self {
            cache: std::sync::Arc::new(std::sync::RwLock::new(cache)),
            rest_client,
        }
    }

    pub async fn resolve(&self, channel_id: Uuid) -> Option<PromptChannelInfo> {
        if let Some(info) = self
            .cache
            .read()
            .ok()
            .and_then(|cache| cache.get(&channel_id).cloned())
        {
            return Some(info);
        }

        let info = fetch_channel_info(channel_id, &self.rest_client).await?;
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(channel_id, info.clone());
        }
        Some(info)
    }
}

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

    /// Durably associate a successful steer with the exact ACP session that
    /// accepted it. Acks may arrive before or after the prompt result: while
    /// the task is in flight we stage the delivery in `TaskMeta`; after return
    /// we write directly to the idle agent's matching live-session ledger.
    pub fn record_successful_steer(
        &mut self,
        channel_id: Uuid,
        event_id: String,
        session_id: String,
    ) -> bool {
        if let Some(meta) = self
            .task_map
            .values_mut()
            .find(|meta| meta.channel_id == Some(channel_id))
        {
            meta.successful_steer_deliveries
                .insert(SuccessfulSteerDelivery {
                    event_id,
                    session_id,
                });
            return true;
        }

        let Some(agent) = self.agents.iter_mut().flatten().find(|agent| {
            agent.state.sessions.get(&channel_id).map(String::as_str) == Some(session_id.as_str())
        }) else {
            return false;
        };
        agent
            .state
            .mark_channel_delivery_success(channel_id, false, [event_id]);
        true
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
        request_id: Option<String>,
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
        // Carry the pick's correlator so a deferred-validation miss on the next
        // turn's session creation emits a late frame the Desktop can match.
        agent.desired_model_request_id = request_id;
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

/// Channel-derived inputs for a new session — `(is_dm, title_channel)` — from
/// **one** metadata resolve.
///
/// Both new-session consumers need the same lookup: the canvas block skips DMs
/// (and fails closed when the channel type can't be determined), and the
/// session title is qualified with the channel name. Resolving once is
/// load-bearing rather than tidy: [`ChannelInfoResolver`] caches only `Some`,
/// so two calls against an unresolvable channel pay the whole
/// [`fetch_channel_info`] retry sequence twice — two `CONTEXT_FETCH_TIMEOUT`
/// attempts plus `CONTEXT_FETCH_RETRY_DELAY` each, in front of `session/new`,
/// precisely when the relay is already degraded.
///
/// `title_channel` is `None` whenever the channel can't usefully identify the
/// session: an unresolved channel, a DM (no meaningful name), or the literal
/// `"unknown"` that [`fetch_channel_info`] substitutes for a metadata event
/// with no `name` tag. Composing that sentinel would title every unnamed
/// channel identically (`Agent · #unknown`) — reintroducing the collision the
/// suffix exists to remove, while naming a channel something it isn't. The
/// startup cache already refuses `channel_type == "unknown"` for the same
/// reason.
///
/// Renames do not retitle live sessions, and a **channel** rename is stickier
/// than an agent rename: `invalidate_channel` drops the session but not the
/// resolver's cached entry, so a renamed channel keeps its old suffix until the
/// process restarts. An agent rename lands on the next spawn (the desktop
/// restart badge covers it — see `spawn_config_hash`).
async fn resolve_new_session_channel_context(
    channel_info: &ChannelInfoResolver,
    channel_id: Uuid,
) -> (bool, Option<String>, Option<String>) {
    let Some(info) = channel_info.resolve(channel_id).await else {
        return (true, None, None);
    };
    let is_dm = info.channel_type == "dm";
    let title_channel = (!is_dm && info.name != UNKNOWN_CHANNEL_NAME).then_some(info.name);
    (is_dm, title_channel, Some(info.channel_type))
}

/// Create a new ACP session via `session_new_full()`, populate model capabilities
/// on the agent (first session only), and apply `desired_model` if set.
///
/// On error from `session_new_full()`, returns the `AcpError` — caller handles
/// error reporting. Model-switch failures are logged and gracefully ignored
/// (the agent proceeds with its default model).
struct NewSessionChannelContext<'a> {
    huddle_instructions: Option<&'a str>,
    canvas: Option<&'a str>,
    name: Option<&'a str>,
    id: Option<Uuid>,
    channel_type: Option<&'a str>,
}

async fn create_session_and_apply_model(
    agent: &mut OwnedAgent,
    ctx: &PromptContext,
    agent_core: Option<&str>,
    channel: NewSessionChannelContext<'_>,
) -> Result<String, AcpError> {
    // Build base_prompt + system_prompt + agent core + canvas metadata into a
    // single prompt. Standard protocol-v2 agents receive it in `session/new`;
    // Goose receives it through the custom request below. Legacy agents receive
    // the same content as user-message sections via `format_prompt`. Core carries
    // its own `[Agent Memory — core]` header, and canvas carries its own
    // `[Channel Canvas]` header; both are appended with a blank-line separator.
    let is_goose = agent.agent_name == "goose";
    let combined_system_prompt = with_canvas(
        with_huddle_instructions(
            with_core(
                with_team(
                    framed_system_prompt(&ctx.cwd, ctx.base_prompt, ctx.system_prompt.as_deref()),
                    ctx.team_instructions.as_deref(),
                ),
                agent_core,
            ),
            channel.huddle_instructions,
        ),
        channel.canvas,
    );

    let session_title = ctx
        .session_title
        .as_deref()
        .map(|agent_name| compose_session_title(agent_name, channel.name));
    let mcp_servers = mcp_servers_with_git_origin(
        &ctx.mcp_servers,
        channel.id,
        channel.channel_type,
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
            thought_level_config_id: extract_thought_level_config_id(&resp.raw),
        });
    }

    // Apply desired_model if set, matching against the fresh session/new
    // response. `post_switch_snapshot` drives everything downstream:
    //   `Some(value)` → a switch applied; `value` is the adapter's post-switch
    //                   RPC response, whose `configOptions` describe the target
    //                   model. Effort resolution and the Desktop capture both
    //                   read it so they converge on the model the session is
    //                   actually running, not the pre-switch default.
    //   `None`        → no switch, or the adapter rejected/does-not-know the
    //                   model; the session/new snapshot is cached as-is and
    //                   `switch_succeeded` stays false.
    let post_switch_snapshot: Option<serde_json::Value> = if let Some(ref desired) =
        agent.desired_model
    {
        // Consume the busy-path pending-ack once for this apply: only the
        // `Applied` arm turns it into a positive terminal; the rejection and
        // unsupported arms already emit their own correlated failure frame, so
        // taking it here keeps a leftover flag from firing a spurious success
        // on some later unrelated session.
        let pending_ack = std::mem::take(&mut agent.desired_model_pending_ack);
        match resolve_model_switch_method(&resp.raw, desired) {
            Some(method) => {
                match apply_model_switch(&mut agent.acp, &resp.session_id, desired, &method).await?
                {
                    ModelSwitchOutcome::Applied(switch_result) => {
                        // The adapter rebuilds `session.configOptions` for the
                        // target model and echoes them here. Refresh capabilities
                        // from that authoritative snapshot when present so the
                        // idle-switch guard and the panel reflect the target
                        // model; drop to `None` (re-derive next session) when the
                        // adapter returned no options so a pre-switch snapshot is
                        // never mistaken for the target model's.
                        if switch_result
                            .get("configOptions")
                            .is_some_and(|v| !v.is_null())
                        {
                            agent.model_capabilities = Some(AgentModelCapabilities {
                                config_options_raw: extract_model_config_options(&switch_result),
                                available_models_raw: extract_model_state(&switch_result),
                                thought_level_config_id: extract_thought_level_config_id(
                                    &switch_result,
                                ),
                            });
                        } else {
                            agent.model_capabilities = None;
                        }
                        // Busy-path deferred switch: emit a positive terminal so
                        // the Desktop confirms success from a real frame instead
                        // of inferring it from timeout silence. Gated on the
                        // pending-ack flag so the idle path (which already acked
                        // `switched` immediately) does not double-emit.
                        if pending_ack {
                            agent.acp.observe(
                                "control_result",
                                serde_json::json!({
                                    "type": "switch_model",
                                    "status": "switched",
                                    "modelId": desired,
                                    "requestId": agent.desired_model_request_id,
                                }),
                            );
                        }
                        Some(switch_result)
                    }
                    ModelSwitchOutcome::Rejected => {
                        // The adapter explicitly rejected the switch: the session
                        // is still on its default model. Surface a terminal
                        // failure so the Desktop ModelPicker rejects the live pick
                        // instead of falsely reporting success, and preserve the
                        // pre-switch capabilities the session is really running.
                        agent.acp.observe(
                            "control_result",
                            serde_json::json!({
                                "type": "switch_model",
                                "status": "failure",
                                "modelId": desired,
                                // Echo the pick's request_id so the Desktop can
                                // correlate this late frame to the operation
                                // that fired it, and ignore replayed results.
                                "requestId": agent.desired_model_request_id,
                            }),
                        );
                        None
                    }
                }
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
                        // Echo the pick's request_id (see the failure arm).
                        "requestId": agent.desired_model_request_id,
                    }),
                );
                None
            }
        }
    } else {
        None
    };
    let switch_succeeded = post_switch_snapshot.is_some();

    // Apply the worker's spawn-scoped startup effort, if configured and the
    // running model advertises a `thought_level` option. Runs on every session
    // creation (config options are per-session), mirroring the model-switch
    // application above. The held value comes from `BUZZ_ACP_EFFORT_LEVEL` and
    // never mutates — there is no pool-level effort state and no live switching.
    // Reads the post-switch snapshot so the configId is discovered on the model
    // the session is actually running; computed BEFORE the capture emission so
    // the cached configOptions tell the truth about the running session.
    let effort_snapshot = post_switch_snapshot.as_ref().unwrap_or(&resp.raw);
    let effort_outcome = apply_startup_effort(agent, effort_snapshot, &resp.session_id).await?;

    // Emit session config for desktop consumption (config bridge tier 1b).
    // Emitted AFTER desired_model resolution so the desktop caches the
    // post-switch state. modelOverridden reflects whether the switch actually
    // applied — false on the rejected/unsupported arms so the panel doesn't show
    // a stale override badge.
    //
    // configOptions come from the post-switch snapshot on a successful switch
    // (the target model's option set) and the session/new snapshot otherwise.
    // Truthful capture: after a successful effort application the snapshot still
    // carries the pre-set `currentValue`, so patch the applied option to the
    // value the session is actually running. A rejected effort or a model with
    // no `thought_level` option leaves the snapshot untouched.
    let config_options_for_cache = {
        let mut opts = effort_snapshot
            .get("configOptions")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        if let Some(StartupEffortOutcome::Applied { config_id, value }) = &effort_outcome {
            patch_config_option_current_value(&mut opts, config_id, value);
        }
        opts
    };
    agent.acp.observe(
        "session_config_captured",
        serde_json::json!({
            "configOptions": config_options_for_cache,
            "modes": resp.raw.get("modes").cloned().unwrap_or(serde_json::Value::Null),
            // `models` must come from the SAME snapshot as configOptions — the
            // post-switch snapshot on a successful switch, session/new otherwise.
            // Taking it from `resp.raw` here would emit the target model's option
            // set alongside the pre-switch model identity, so the desktop panel
            // would report the old model as live after an applied switch. When a
            // successful target response omits `models`, this emits Null rather
            // than falling back to the pre-switch `resp.raw.models`.
            "models": effort_snapshot.get("models").cloned().unwrap_or(serde_json::Value::Null),
            "modelOverridden": agent.model_overridden && switch_succeeded,
            // Pair identity for the desktop session-config cache, which is
            // keyed by (agent, relay) like the lifecycle frames.
            "relayUrl": ctx.relay_url,
        }),
    );

    // Apply permission mode if not the agent's built-in default AND the agent
    // advertises the requested mode in session/new. Agents that don't support
    // the mode (e.g., goose crashes on unrecognized set_config_option values)
    // are safely skipped — the harness auto-approves via handle_permission_request.
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

/// Outcome of a live model-switch RPC returned by [`apply_model_switch`].
///
/// `Applied` and `Rejected` are distinct outcomes and must not be collapsed:
/// the caller needs to know whether the session is now on the target model
/// before deciding what capabilities to cache and whether to surface a failure.
#[derive(Debug)]
enum ModelSwitchOutcome {
    /// The adapter accepted the switch. Carries the RPC response value, which
    /// may include refreshed `configOptions` for the target model.
    Applied(serde_json::Value),
    /// The adapter returned an application-level error (e.g. JSON error,
    /// unrecognised model). The session is still on its default model;
    /// pre-switch capabilities must be preserved.
    Rejected,
}

/// Send the appropriate ACP model-switch request with a timeout.
///
/// Transport-class errors propagate as `Err` so the caller respawns the agent
/// rather than reuse a poisoned stdio stream. An application-level rejection is
/// non-fatal but distinct from success: it returns [`ModelSwitchOutcome::Rejected`]
/// so the caller preserves pre-switch capabilities and tells Desktop the pick
/// failed instead of silently claiming the switch landed.
async fn apply_model_switch(
    acp: &mut AcpClient,
    session_id: &str,
    desired: &str,
    method: &ModelSwitchMethod,
) -> Result<ModelSwitchOutcome, AcpError> {
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
        // Return the RPC result so the caller can consume the post-switch
        // capability snapshot the adapter echoes (claude-agent-acp rebuilds
        // `session.configOptions` on a model change and returns them here).
        Ok(Ok(value)) => {
            tracing::info!(
                target: "pool::model",
                "applied model {desired} via {method_label} on session {session_id}"
            );
            Ok(ModelSwitchOutcome::Applied(value))
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
            Err(e)
        }
        // Application-level errors (Json, etc.) — the adapter explicitly
        // rejected the switch; the session is still on its default model.
        // Distinct from a successful switch that returned no configOptions:
        // the caller must preserve pre-switch capabilities here.
        Ok(Err(e)) => {
            tracing::warn!(
                target: "pool::model",
                "failed to set model {desired} via {method_label}: {e} — proceeding with agent default"
            );
            Ok(ModelSwitchOutcome::Rejected)
        }
        Err(_) => {
            // Outer timeout fired — the inner send_request may have left the
            // stream in an unknown state. Treat as transport error.
            tracing::error!(
                target: "pool::model",
                "model set via {method_label} timed out ({MODEL_SWITCH_TIMEOUT:?}) — treating as fatal"
            );
            Err(AcpError::Timeout(MODEL_SWITCH_TIMEOUT))
        }
    }
}

/// Outcome of applying a worker's spawn-scoped startup effort at session creation.
///
/// Drives truthful capture: only `Applied` patches the cached `currentValue`.
/// `Rejected` (adapter refused) and the `None` return (model advertises no
/// `thought_level` option, or no effort was configured) leave the session/new
/// snapshot untouched so the panel reflects the session's real state.
enum StartupEffortOutcome {
    Applied { config_id: String, value: String },
    Rejected,
}

/// Apply the worker's held `startup_effort` via `session/set_config_option`, if
/// set and the current model advertises a `thought_level` option.
///
/// Returns `Ok(None)` when there is nothing to apply (no configured effort, or
/// the model has no `thought_level` option) or `Ok(Some(_))` describing whether
/// the adapter accepted the value. Transport-class errors propagate as `Err` so
/// the caller respawns the worker rather than reuse a poisoned stream — mirroring
/// [`apply_model_switch`]'s classification. Application-level rejection is
/// non-fatal: the session proceeds on the model's default effort.
async fn apply_startup_effort(
    agent: &mut OwnedAgent,
    session_new_result: &serde_json::Value,
    session_id: &str,
) -> Result<Option<StartupEffortOutcome>, AcpError> {
    let Some(value) = agent.startup_effort.clone() else {
        return Ok(None);
    };
    let Some(config_id) = extract_thought_level_config_id(session_new_result) else {
        tracing::info!(
            target: "pool::effort",
            "startup effort {value} configured but model advertises no thought_level option — leaving agent default"
        );
        return Ok(None);
    };

    let result = tokio::time::timeout(MODEL_SWITCH_TIMEOUT, async {
        agent
            .acp
            .session_set_config_option(session_id, &config_id, &value)
            .await
    })
    .await;

    match result {
        Ok(Ok(_)) => {
            tracing::info!(
                target: "pool::effort",
                "applied startup effort {value} via configId={config_id} on session {session_id}"
            );
            Ok(Some(StartupEffortOutcome::Applied { config_id, value }))
        }
        // Transport-class errors may have corrupted the stdio stream — propagate
        // so the caller can respawn the agent instead of reusing a poisoned one.
        Ok(Err(e @ AcpError::Io(_)))
        | Ok(Err(e @ AcpError::WriteTimeout(_)))
        | Ok(Err(e @ AcpError::Timeout(_)))
        | Ok(Err(e @ AcpError::Protocol(_)))
        | Ok(Err(e @ AcpError::AgentExited)) => {
            tracing::error!(
                target: "pool::effort",
                "fatal error applying startup effort {value} via configId={config_id}: {e}"
            );
            Err(e)
        }
        // Application-level rejection (e.g. Json) — agent is fine, uses default effort.
        Ok(Err(e)) => {
            tracing::warn!(
                target: "pool::effort",
                "adapter rejected startup effort {value} via configId={config_id}: {e} — proceeding with agent default"
            );
            Ok(Some(StartupEffortOutcome::Rejected))
        }
        Err(_) => {
            // Outer timeout fired — the inner send_request may have left the
            // stream in an unknown state. Treat as transport error.
            tracing::error!(
                target: "pool::effort",
                "startup effort {value} via configId={config_id} timed out ({MODEL_SWITCH_TIMEOUT:?}) — treating as fatal"
            );
            Err(AcpError::Timeout(MODEL_SWITCH_TIMEOUT))
        }
    }
}

/// Patch the `currentValue` of the configOption whose `configId`/`id` matches
/// `config_id` in a session/new `configOptions` array, in place.
///
/// Used by truthful capture: a successful `session/set_config_option` is not
/// reflected in the original session/new snapshot, so the accepted value is
/// written back before the snapshot is cached. A no-op when `options` is not an
/// array or no entry matches (the id came from the same array, so a match is
/// expected in practice).
fn patch_config_option_current_value(
    options: &mut serde_json::Value,
    config_id: &str,
    value: &str,
) {
    let Some(arr) = options.as_array_mut() else {
        return;
    };
    for opt in arr {
        let matches = opt
            .get("configId")
            .or_else(|| opt.get("id"))
            .and_then(|v| v.as_str())
            == Some(config_id);
        if matches {
            opt["currentValue"] = serde_json::Value::String(value.to_string());
            return;
        }
    }
}

/// Set the session permission mode via `session/set_config_option`.
///
/// Non-fatal for most errors: logs and proceeds. The agent falls back
/// to its default permission mode (`"default"`), which still works via
/// Check if the agent's `session/new` response advertises a given mode ID
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

/// per-tool auto-approval in `handle_permission_request`.
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
                "failed to set permission mode {wire:?}: {e} — falling back to per-tool auto-approval"
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

/// Prepend a legacy agent's standing context to a user-message body.
///
/// Legacy agents (`protocol_version < 2`) don't receive standing context via
/// the system role in `session/new`, so it must ride along in the user message
/// — in the session's *first* one, and never again. Agents with
/// `protocol_version >= 2`, or an empty [`StandingContext`], get `body`
/// unchanged. Both legacy dispatch paths (initial message, heartbeat) go
/// through this one gate so they can't drift apart again.
///
/// A heartbeat passes base only: it has no channel, so there is no core or
/// canvas to carry, and it has never been given the persona.
pub(crate) fn prepend_standing_for_legacy(
    protocol_version: u32,
    standing: &crate::queue::StandingContext<'_>,
    body: &str,
) -> String {
    if protocol_version >= 2 {
        return body.to_string();
    }
    let sections = standing.sections();
    if sections.is_empty() {
        return body.to_string();
    }
    format!("{}\n\n{body}", sections.join("\n\n"))
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
             here. This is where you already are, so start here rather than scanning \
             `$HOME`. Any specific path the user names is fine to read."
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

/// Append owner-signed huddle instructions to this channel session's system prompt.
fn with_huddle_instructions(prompt: Option<String>, instructions: Option<&str>) -> Option<String> {
    let instructions = instructions
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (prompt, instructions) {
        (Some(prompt), Some(instructions)) => {
            Some(format!("{prompt}\n\n[Huddle Instructions]\n{instructions}"))
        }
        (None, Some(instructions)) => Some(format!("[Huddle Instructions]\n{instructions}")),
        (Some(prompt), None) => Some(prompt),
        (None, None) => None,
    }
}

/// Append the `[Channel Canvas]` metadata section onto the accumulated system prompt.
///
/// The canvas section already carries its `[Channel Canvas]` header (from
/// `render_canvas_section`), so it is joined with a blank-line separator.
/// Either side may be absent.
fn with_canvas(prompt: Option<String>, canvas: Option<&str>) -> Option<String> {
    match (prompt, canvas) {
        (Some(prompt), Some(canvas)) => Some(format!("{prompt}\n\n{canvas}")),
        (Some(prompt), None) => Some(prompt),
        (None, Some(canvas)) => Some(canvas.to_string()),
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

    // Canvas metadata fetch — same lifecycle as core: once per new channel session,
    // never for heartbeats, cached until session invalidation.
    //
    // DM check: use startup channel_info first; lazy-fetch only when missing.
    // A confirmed DM never receives a canvas section. If the channel type cannot
    // be determined (metadata absent and lazy fetch fails/unknown), skip the canvas
    // rather than assuming non-DM — failing closed on DM ambiguity is safer.
    //
    // I3 lifecycle: hold the fetched section in a local `pending_canvas` and
    // commit it to `canvas_sections` only after session creation succeeds. This
    // prevents a stale revision A surviving a failed create and being re-used by
    // the next attempt after the canvas was cleared.
    let mut pending_canvas: Option<(Uuid, String)> = None;
    let mut huddle_instructions: Option<String> = None;
    // Channel name for the session title, from the same single resolve the
    // canvas DM check uses — see `resolve_new_session_channel_context`.
    let mut title_channel: Option<String> = None;
    let mut origin_channel_type: Option<String> = None;
    if let PromptSource::Channel(cid) = &source {
        let is_new_channel_session = !agent.state.sessions.contains_key(cid);
        let needs_canvas = is_new_channel_session && !agent.state.canvas_sections.contains_key(cid);
        if is_new_channel_session {
            let (is_dm, resolved_channel, resolved_channel_type) =
                resolve_new_session_channel_context(&ctx.channel_info, *cid).await;
            title_channel = resolved_channel;
            origin_channel_type = resolved_channel_type;
            if let Some(owner) = ctx.agent_owner_pubkey.as_ref() {
                huddle_instructions =
                    fetch_huddle_instructions(*cid, owner, &ctx.rest_client).await;
            }
            // A confirmed DM never receives a canvas section; an undeterminable
            // channel type fails closed as a DM for the same reason.
            if needs_canvas && !is_dm {
                if let Some(section) = fetch_canvas_section(*cid, &ctx.rest_client).await {
                    pending_canvas = Some((*cid, section));
                }
            }
        }
    }

    // The core section to fold into the system prompt for this turn's session.
    // Channel-scoped; heartbeats carry no owner core.
    let agent_core: Option<String> = match &source {
        PromptSource::Channel(cid) => agent.state.core_sections.get(cid).cloned(),
        PromptSource::Heartbeat => None,
    };

    // The canvas metadata section — channel-scoped, absent for heartbeats/DMs.
    // Prefer the committed cache; fall back to pending (for new sessions being created now).
    let agent_canvas: Option<String> = match &source {
        PromptSource::Channel(cid) => agent
            .state
            .canvas_sections
            .get(cid)
            .cloned()
            .or_else(|| pending_canvas.as_ref().map(|(_, s)| s.clone())),
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
                    NewSessionChannelContext {
                        huddle_instructions: huddle_instructions.as_deref(),
                        canvas: agent_canvas.as_deref(),
                        name: title_channel.as_deref(),
                        id: Some(*cid),
                        channel_type: origin_channel_type.as_deref(),
                    },
                )
                .await
                {
                    Ok(sid) => {
                        tracing::info!(
                            target: "pool::session",
                            "created session {sid} for channel {cid}"
                        );
                        agent.state.sessions.insert(*cid, sid.clone());
                        agent
                            .state
                            .deliveries
                            .insert(*cid, ChannelDeliveryState::default());
                        // Seed a zero usage baseline: buzz-acp spawned this session
                        // so prior usage is zero by definition — first turn is reliable.
                        agent.acp.notify_session_spawned(&sid);
                        // Commit canvas only after session creation succeeds (I3).
                        if let Some((pending_cid, section)) = pending_canvas.take() {
                            agent.state.canvas_sections.insert(pending_cid, section);
                        }
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
                match create_session_and_apply_model(
                    &mut agent,
                    &ctx,
                    None,
                    NewSessionChannelContext {
                        huddle_instructions: None,
                        canvas: None,
                        name: None,
                        id: None,
                        channel_type: None,
                    },
                )
                .await
                {
                    Ok(sid) => {
                        tracing::info!(
                            target: "pool::session",
                            "created heartbeat session {sid} for agent {}",
                            agent.index
                        );
                        agent.state.heartbeat_session = Some(sid.clone());
                        // Seed a zero usage baseline: buzz-acp spawned this session.
                        agent.acp.notify_session_spawned(&sid);
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

    // Standing context is fixed for the life of a session. Agents with
    // systemPrompt support already hold it from session/new; legacy agents
    // receive it in the session's first user message and never again.
    //
    // `is_new_session` comes from the session registry, which is cleared
    // whenever a session is invalidated — so the replacement session re-delivers
    // rather than leaving the agent unbriefed.
    let standing = crate::queue::StandingContext {
        base_prompt: ctx.base_prompt,
        system_prompt: ctx.system_prompt.as_deref(),
        team_instructions: ctx.team_instructions.as_deref(),
        agent_core: agent_core.as_deref(),
        huddle_instructions: huddle_instructions.as_deref(),
        agent_canvas: agent_canvas.as_deref(),
    };
    // Delivery state is committed only after ACP confirms success. Existing
    // sessions created before this field existed fail safe by behaving as
    // undelivered once, rather than silently omitting standing context.
    let mut standing_context_sent = match &source {
        PromptSource::Channel(cid) => agent
            .state
            .deliveries
            .get(cid)
            .is_some_and(|delivery| delivery.standing_context_sent),
        PromptSource::Heartbeat => agent.state.heartbeat_standing_context_sent,
    };

    if is_new_session {
        if let (PromptSource::Channel(cid), Some(ref initial_msg)) = (&source, &ctx.initial_message)
        {
            tracing::info!(
                target: "pool::session",
                "sending initial_message to session {session_id} for channel {cid}"
            );
            let init_msg = prepend_standing_for_legacy(
                if agent.has_system_prompt_support() {
                    2
                } else {
                    1
                },
                &standing,
                initial_msg,
            );
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
                    // The legacy agent has its standing context now; the turn
                    // prompt below must not repeat it. Every other arm returns.
                    standing_context_sent = true;
                    if !agent.has_system_prompt_support() {
                        agent.state.mark_channel_delivery_success(*cid, true, []);
                    }
                    let usage = agent.acp.take_turn_usage();
                    publish_agent_turn_metric(
                        &ctx,
                        usage,
                        Some(*cid),
                        &session_id,
                        &format!("{turn_id}:initial"),
                        Some(acp_stop_to_core(&stop_reason)),
                    )
                    .await;
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
                        Ok(stop_reason) => {
                            let usage = agent.acp.take_turn_usage();
                            publish_agent_turn_metric(
                                &ctx,
                                usage,
                                Some(*cid),
                                &session_id,
                                &format!("{turn_id}:initial"),
                                Some(acp_stop_to_core(&stop_reason)),
                            )
                            .await;
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
    // Event IDs represented by this prompt. Commit only after ACP reports a
    // successful turn; failed/cancelled prompts must be retryable without loss.
    let mut pending_delivered_event_ids = HashSet::new();
    let prompt_sections: Vec<String> = if let Some(text) = prompt_text {
        // Heartbeats create their session before this point, so a Goose method-not-found
        // probe has already selected the correct framing for this process.
        //
        // Only the first heartbeat of a session carries `[Base]`; later ticks
        // reuse the same session, so the agent already has it.
        let text = if standing_context_sent {
            text
        } else {
            prepend_standing_for_legacy(
                if agent.has_system_prompt_support() {
                    2
                } else {
                    1
                },
                &crate::queue::StandingContext {
                    base_prompt: ctx.base_prompt,
                    ..Default::default()
                },
                &text,
            )
        };
        vec![text]
    } else if let Some(ref b) = batch {
        // Build prompt from batch with context enrichment.
        // Try startup cache first; lazy-fetch via REST for dynamic channels.
        let channel_info = ctx.channel_info.resolve(b.channel_id).await;

        let conversation_context = if ctx.context_message_limit > 0 {
            fetch_conversation_context(b, &channel_info, &ctx).await
        } else {
            None
        };
        let rendered_batch_ids: HashSet<String> = b
            .events
            .iter()
            .chain(b.cancelled_events.iter())
            .map(|event| event.event.id.to_hex())
            .collect();
        let delivered_ids = agent
            .state
            .deliveries
            .get(&b.channel_id)
            .map(|delivery| &delivery.delivered_event_ids)
            .cloned()
            .unwrap_or_default();
        let conversation_context_had_delivered_events =
            conversation_context.as_ref().is_some_and(|context| {
                conversation_context_event_ids(Some(context))
                    .iter()
                    .any(|event_id| delivered_ids.contains(event_id))
            });
        let conversation_context =
            conversation_context_delta(conversation_context, &delivered_ids, &rendered_batch_ids);
        pending_delivered_event_ids.extend(rendered_batch_ids);
        pending_delivered_event_ids.extend(conversation_context_event_ids(
            conversation_context.as_ref(),
        ));

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
                agent_core: standing.agent_core,
                huddle_instructions: standing.huddle_instructions,
                channel_info: channel_info.as_ref(),
                conversation_context: conversation_context.as_ref(),
                conversation_context_had_delivered_events,
                profile_lookup: profile_lookup.as_ref(),
                has_system_prompt_support: agent.has_system_prompt_support(),
                base_prompt: standing.base_prompt,
                system_prompt: standing.system_prompt,
                team_instructions: standing.team_instructions,
                agent_canvas: standing.agent_canvas,
                standing_context_sent,
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
    let prompt_bytes: usize = prompt_blocks.iter().map(|block| block.len()).sum();
    let has_standing_context = match &source {
        PromptSource::Channel(_) => !standing.sections().is_empty(),
        PromptSource::Heartbeat => ctx.base_prompt.is_some(),
    };
    let standing_context_included =
        !agent.has_system_prompt_support() && !standing_context_sent && has_standing_context;
    tracing::info!(
        target: "pool::prompt",
        prompt_bytes,
        standing_context_included,
        delivered_event_delta = pending_delivered_event_ids.len(),
        "prompt context delivery"
    );
    agent.acp.observe(
        "prompt_context_delivery",
        serde_json::json!({
            "promptBytes": prompt_bytes,
            "standingContextIncluded": standing_context_included,
            "eventDeltaCount": pending_delivered_event_ids.len(),
        }),
    );

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
                    if let ControlSignal::SwitchModel { model_id, request_id } = &control_signal {
                        agent.desired_model = Some(model_id.clone());
                        agent.model_overridden = true;
                        agent.desired_model_request_id = request_id.clone();
                        // Busy path: the real apply is deferred to the requeued
                        // session. Arm the positive-terminal emit so that apply
                        // reports success explicitly rather than the Desktop
                        // inferring it from timeout silence.
                        agent.desired_model_pending_ack = true;
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
                            ControlSignal::Rotate | ControlSignal::SwitchModel { .. }
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
                        if let PromptSource::Channel(cid) = &source {
                            let standing_sent = !agent.has_system_prompt_support();
                            agent.state.mark_channel_delivery_success(
                                *cid,
                                standing_sent,
                                pending_delivered_event_ids.iter().cloned(),
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

            if let PromptSource::Channel(cid) = &source {
                let standing_sent = !agent.has_system_prompt_support();
                agent.state.mark_channel_delivery_success(
                    *cid,
                    standing_sent,
                    pending_delivered_event_ids.iter().cloned(),
                );
            } else if !agent.has_system_prompt_support() {
                agent.state.heartbeat_standing_context_sent = true;
            }

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

/// Fetch owner-signed huddle instructions for a new channel session.
///
/// The event is promoted into the system role, so accepting any channel member's
/// event would be a privilege escalation. Only the configured agent owner's
/// valid signature is accepted; absence or failure simply yields no section.
async fn fetch_huddle_instructions(
    channel_id: Uuid,
    owner: &nostr::PublicKey,
    rest: &RestClient,
) -> Option<String> {
    use nostr::{Alphabet, SingleLetterTag};

    let h_tag = SingleLetterTag::lowercase(Alphabet::H);
    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(
            buzz_core::kind::KIND_HUDDLE_GUIDELINES as u16,
        ))
        .author(*owner)
        .custom_tags(h_tag, [channel_id.to_string()])
        .limit(1);
    let json = match timeout(
        CONTEXT_FETCH_TIMEOUT,
        rest.query(std::slice::from_ref(&filter)),
    )
    .await
    {
        Ok(Ok(json)) => json,
        Ok(Err(error)) => {
            tracing::warn!(channel = %channel_id, "huddle instructions query failed: {error}");
            return None;
        }
        Err(_) => {
            tracing::warn!(channel = %channel_id, "huddle instructions query timed out");
            return None;
        }
    };
    huddle_instructions_from_query_response(json.as_array()?, channel_id, owner)
}

fn huddle_instructions_from_query_response(
    events: &[serde_json::Value],
    channel_id: Uuid,
    owner: &nostr::PublicKey,
) -> Option<String> {
    let raw = events.first()?;
    let event = serde_json::from_value::<nostr::Event>(raw.clone()).ok()?;
    event.verify().ok()?;
    let channel_id = channel_id.to_string();
    if event.pubkey != *owner
        || event.kind.as_u16() as u32 != buzz_core::kind::KIND_HUDDLE_GUIDELINES
        || !event
            .tags
            .iter()
            .any(|tag| tag.kind().to_string() == "h" && tag.content() == Some(channel_id.as_str()))
    {
        return None;
    }
    let content = event.content.trim();
    (!content.is_empty()).then(|| content.to_owned())
}

/// Fetch the latest canvas event for `channel_id` and return a rendered
/// `[Channel Canvas]` metadata section, or `None` if absent/blank/error.
///
/// Failure modes (all fail open — no crash, no block):
/// * relay returns no event → `None`
/// * latest event's content is blank → `None` (cleared canvas; older revisions
///   are NOT resurrected)
/// * malformed JSON array, missing fields, bad event ID, bad timestamp →
///   logged at `warn`; returns `None`
/// * REST error or timeout → returns `None`
///
/// Called at most once per new channel session; the result is cached in
/// `SessionState::canvas_sections` and cleared on session invalidation.
async fn fetch_canvas_section(channel_id: Uuid, rest: &RestClient) -> Option<String> {
    use nostr::{Alphabet, SingleLetterTag};

    let h_tag = SingleLetterTag::lowercase(Alphabet::H);
    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(buzz_core::kind::KIND_CANVAS as u16))
        .custom_tags(h_tag, [channel_id.to_string()])
        .limit(1);

    const CANVAS_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
    let json = match tokio::time::timeout(
        CANVAS_FETCH_TIMEOUT,
        rest.query(std::slice::from_ref(&filter)),
    )
    .await
    {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_id,
                "canvas query failed: {e} — emitting no section"
            );
            return None;
        }
        Err(_) => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_id,
                timeout_ms = CANVAS_FETCH_TIMEOUT.as_millis() as u64,
                "canvas fetch timed out — emitting no section"
            );
            return None;
        }
    };

    let events = match json.as_array() {
        Some(arr) => arr,
        None => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_id,
                "canvas query response is not a JSON array — emitting no section"
            );
            return None;
        }
    };

    canvas_section_from_query_response(events, &channel_id.to_string())
}

/// Parse a canvas query response array and render a `[Channel Canvas]` section.
///
/// Extracted as a pure function so tests can exercise the parsing/validation
/// logic without async machinery or relay connectivity.
///
/// Returns `None` on: empty array, blank content, malformed/partial event JSON
/// (requires a complete, structurally valid Nostr event), or an out-of-range
/// `created_at` timestamp. Never falls back to epoch or raw integers.
pub(crate) fn canvas_section_from_query_response(
    events: &[serde_json::Value],
    channel_uuid: &str,
) -> Option<String> {
    let raw = events.first()?;

    // Deserialise as a complete Nostr Event. Partial objects (missing pubkey,
    // sig, kind, or tags) are rejected here rather than trusted implicitly.
    let event = match serde_json::from_value::<nostr::Event>(raw.clone()) {
        Ok(ev) => ev,
        Err(err) => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_uuid,
                %err,
                "canvas query returned a malformed event — emitting no section",
            );
            return None;
        }
    };

    // Verify the event's id and signature agree with its content.
    // A structurally complete but tampered event must not supply trusted metadata.
    if let Err(err) = event.verify() {
        tracing::warn!(
            target: "canvas::fetch",
            channel = %channel_uuid,
            %err,
            "canvas event failed signature verification — emitting no section",
        );
        return None;
    }

    // Validate kind: must be KIND_CANVAS (40100).
    if event.kind != nostr::Kind::Custom(buzz_core::kind::KIND_CANVAS as u16) {
        tracing::warn!(
            target: "canvas::fetch",
            channel = %channel_uuid,
            kind = %event.kind.as_u16(),
            "canvas event has unexpected kind — emitting no section",
        );
        return None;
    }

    // Validate h-tag: must carry the channel UUID we queried.
    // The REST boundary filters by #h, but we verify here to prevent a
    // misbehaving relay from injecting a different channel's canvas.
    let h_tag_matches = event.tags.iter().any(|tag| {
        let v = tag.as_slice();
        v.len() >= 2 && v[0] == "h" && v[1] == channel_uuid
    });
    if !h_tag_matches {
        tracing::warn!(
            target: "canvas::fetch",
            channel = %channel_uuid,
            "canvas event is missing expected h-tag — emitting no section",
        );
        return None;
    }

    // Blank content means the canvas was cleared; do not fall back to older events.
    if event.content.trim().is_empty() {
        tracing::debug!(
            target: "canvas::fetch",
            channel = %channel_uuid,
            "latest canvas event has blank content — emitting no section"
        );
        return None;
    }

    let id = event.id.to_hex();

    // Convert the Nostr timestamp to a UTC RFC3339 string with Z suffix.
    // Use checked conversion: a u64 that exceeds i64::MAX (e.g. Timestamp::max())
    // wraps silently with `as i64`, producing a negative value that chrono would
    // accept as a date in 1969. Reject out-of-range values explicitly instead.
    let ts_secs = match i64::try_from(event.created_at.as_secs()) {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_uuid,
                "canvas event created_at overflows i64 — emitting no section",
            );
            return None;
        }
    };
    let timestamp = match chrono::DateTime::from_timestamp(ts_secs, 0) {
        Some(dt) => dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        None => {
            tracing::warn!(
                target: "canvas::fetch",
                channel = %channel_uuid,
                ts_secs,
                "canvas event has out-of-range created_at — emitting no section",
            );
            return None;
        }
    };

    tracing::info!(
        target: "canvas::fetch",
        channel = %channel_uuid,
        event_id = %id,
        "injected channel canvas metadata section into system prompt"
    );
    Some(render_canvas_section(&id, &timestamp, channel_uuid))
}

/// Render the `[Channel Canvas]` metadata section string.
///
/// Pure function — kept separate so unit tests can exercise rendering
/// without async machinery or relay connectivity.
pub(crate) fn render_canvas_section(event_id: &str, timestamp: &str, channel_uuid: &str) -> String {
    format!(
        "[Channel Canvas]\n\
         Canvas revision (event ID): {event_id}\n\
         Last modified: {timestamp}\n\
         Fetch current content with: buzz canvas get --channel {channel_uuid}"
    )
}

fn conversation_context_event_ids(context: Option<&ConversationContext>) -> HashSet<String> {
    match context {
        Some(ConversationContext::Thread { messages, .. })
        | Some(ConversationContext::Dm { messages, .. }) => messages
            .iter()
            .filter(|message| !message.event_id.is_empty())
            .map(|message| message.event_id.clone())
            .collect(),
        None => HashSet::new(),
    }
}

/// Remove events already delivered to this live ACP session. Triggering events
/// are also excluded because they are rendered separately in `[Event]`.
/// IDs are compared in Buzz's canonical 64-character lowercase hex form: relay
/// context JSON supplies the same form emitted by `EventId::to_hex()`. A
/// non-canonical or missing ID deliberately fails open and may be re-sent.
fn conversation_context_delta(
    context: Option<ConversationContext>,
    delivered: &HashSet<String>,
    triggering: &HashSet<String>,
) -> Option<ConversationContext> {
    let filter = |messages: Vec<ContextMessage>| {
        messages
            .into_iter()
            .filter(|message| {
                message.event_id.is_empty()
                    || (!delivered.contains(&message.event_id)
                        && !triggering.contains(&message.event_id))
            })
            .collect::<Vec<_>>()
    };

    match context? {
        ConversationContext::Thread {
            messages,
            total,
            truncated,
        } => {
            let messages = filter(messages);
            (!messages.is_empty()).then_some(ConversationContext::Thread {
                messages,
                total,
                truncated,
            })
        }
        ConversationContext::Dm {
            messages,
            total,
            truncated,
        } => {
            let messages = filter(messages);
            (!messages.is_empty()).then_some(ConversationContext::Dm {
                messages,
                total,
                truncated,
            })
        }
    }
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
    channel_info: &Option<PromptChannelInfo>,
    ctx: &PromptContext,
) -> Option<ConversationContext> {
    let limit = ctx.context_message_limit;
    let is_dm = channel_info
        .as_ref()
        .map(|ci| ci.channel_type == "dm")
        .unwrap_or(false);

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
    if is_dm {
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

    let event_id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    Some(ContextMessage {
        event_id,
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
        ControlSignal::Interrupt | ControlSignal::SwitchModel { .. } => CancelReason::Interrupt,
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
            // Field-local: same contract as cache_read_tokens.
            cache_write_tokens: usage.turn_cache_write_tokens,
        })
    } else {
        // Defense-in-depth: UsageTracker already sets all turn_* fields to None
        // when delta_reliable is false, so the None arm here is technically
        // redundant. The explicit guard prevents a future refactor from
        // accidentally publishing unreliable per-turn counts.
        None
    };
    let cumulative_counts = Some(TokenCounts {
        input_tokens: usage.cumulative_input_tokens,
        output_tokens: usage.cumulative_output_tokens,
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
        // Session-cumulative cache-write tokens; same provenance contract as
        // cache_read_tokens.
        cache_write_tokens: usage.cumulative_cache_write_tokens,
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
        pricing_identity: usage.pricing_identity.clone(),
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

    fn test_mcp_server() -> McpServer {
        McpServer {
            name: "dev".into(),
            command: "buzz-dev-mcp".into(),
            args: vec![],
            env: vec![],
        }
    }

    // MINOR (#2884): the permission-mode RPC is gated on agent_supports_mode.
    // An advertised mode issues set_config_option; an absent one is skipped so
    // the harness falls back to per-tool auto-approval. Pin both edges directly.
    #[test]
    fn agent_supports_mode_advertised_auto_is_true() {
        let session_new = json!({
            "modes": { "availableModes": [{ "id": "default" }, { "id": "auto" }] }
        });
        assert!(agent_supports_mode(
            &session_new,
            PermissionMode::Auto.as_wire_str()
        ));
    }

    #[test]
    fn agent_supports_mode_absent_auto_is_false() {
        let session_new = json!({
            "modes": { "availableModes": [{ "id": "default" }] }
        });
        assert!(!agent_supports_mode(
            &session_new,
            PermissionMode::Auto.as_wire_str()
        ));
    }

    #[test]
    fn agent_supports_mode_missing_modes_field_is_false() {
        let session_new = json!({ "sessionId": "sess-1" });
        assert!(!agent_supports_mode(
            &session_new,
            PermissionMode::Auto.as_wire_str()
        ));
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

    // These pin the initial_message dispatch path (run_prompt_task, ~line 855):
    // a legacy agent WITH a base_prompt must get [Base] prepended to the user
    // message. This is the exact regression that shipped in the round-2 bug.

    fn base_only(base_prompt: Option<&str>) -> crate::queue::StandingContext<'_> {
        crate::queue::StandingContext {
            base_prompt,
            ..Default::default()
        }
    }

    #[test]
    fn test_initial_message_legacy_agent_gets_base_prepended() {
        // protocol_version 1 + Some(base_prompt): [Base] rides along in the
        // user message, composed as `[Base]\n{bp}\n\n{initial_msg}`.
        let composed = prepend_standing_for_legacy(
            1,
            &base_only(Some("you are a helpful agent")),
            "hello channel",
        );
        assert_eq!(composed, "[Base]\nyou are a helpful agent\n\nhello channel");
    }

    #[test]
    fn test_initial_message_modern_agent_omits_base() {
        // protocol_version 2 receives base_prompt via session/new, so the user
        // message is left untouched even when a base_prompt is present.
        let composed = prepend_standing_for_legacy(
            2,
            &base_only(Some("you are a helpful agent")),
            "hello channel",
        );
        assert_eq!(composed, "hello channel");
    }

    #[test]
    fn test_heartbeat_standing_block_is_base_only() {
        // A heartbeat has no channel, so core and canvas are absent by
        // construction — and it has never carried the persona. Pin that the
        // shared helper does not start handing heartbeats [System].
        let composed = prepend_standing_for_legacy(1, &base_only(Some("be helpful")), "tick");
        assert_eq!(composed, "[Base]\nbe helpful\n\ntick");
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
        let composed = prepend_standing_for_legacy(1, &base_only(None), "hello channel");
        assert_eq!(composed, "hello channel");
    }

    // ── prepend_standing_for_legacy ───────────────────────────────────────────

    fn full_standing() -> crate::queue::StandingContext<'static> {
        crate::queue::StandingContext {
            base_prompt: Some("be helpful"),
            system_prompt: Some("you are Eva"),
            team_instructions: Some("ship small"),
            agent_core: Some("[Agent Memory — core]\nremember this"),
            huddle_instructions: Some("reply immediately"),
            agent_canvas: Some("[Channel Canvas]\ncanvas content"),
        }
    }

    #[test]
    fn test_initial_message_legacy_agent_gets_whole_standing_block() {
        // The initial message is the legacy agent's first contact, so it must
        // carry every standing section — not just [Base] and the canvas, which
        // left the agent acting on its first turn with no persona and no memory.
        let composed = prepend_standing_for_legacy(1, &full_standing(), "do the thing");
        let positions: Vec<usize> = [
            "[Base]",
            "[System]",
            "[Team Instructions]",
            "[Agent Memory — core]",
            "[Huddle Instructions]",
            "[Channel Canvas]",
            "do the thing",
        ]
        .iter()
        .map(|needle| {
            composed
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle} in: {composed}"))
        })
        .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "sections must match the per-turn order, body last; got: {composed}"
        );
    }

    #[test]
    fn test_initial_message_standing_order_matches_per_turn_order() {
        // Both legacy paths render through StandingContext, so the initial
        // message and a first-turn prompt agree section-for-section.
        let standing = full_standing();
        let composed = prepend_standing_for_legacy(1, &standing, "do the thing");
        assert_eq!(
            composed,
            format!("{}\n\ndo the thing", standing.sections().join("\n\n"))
        );
    }

    #[test]
    fn test_initial_message_modern_agent_omits_standing_block() {
        // Protocol-v2 agents hold all of this from session/new; repeating it in
        // the initial-message user turn would double-render every section.
        let composed = prepend_standing_for_legacy(2, &full_standing(), "do the thing");
        assert_eq!(composed, "do the thing");
    }

    #[test]
    fn test_initial_message_legacy_agent_without_standing_is_unchanged() {
        // Nothing configured: body passes through with no stray blank lines.
        let composed =
            prepend_standing_for_legacy(1, &crate::queue::StandingContext::default(), "do it");
        assert_eq!(composed, "do it");
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
                event_id: String::new(),
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

    fn context_message(event_id: &str, content: &str) -> ContextMessage {
        ContextMessage {
            event_id: event_id.to_string(),
            pubkey: "author".into(),
            timestamp: "2026-08-09T00:00:00Z".into(),
            content: content.into(),
        }
    }

    #[tokio::test]
    async fn run_prompt_task_commits_standing_context_only_after_acp_success() {
        let capture = std::env::temp_dir().join(format!(
            "buzz-acp-standing-lifecycle-{}.ndjson",
            Uuid::new_v4()
        ));
        let quoted_capture = capture.to_string_lossy().replace('\'', "'\\''");
        let script = format!(
            r#"count=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> '{quoted_capture}'
  count=$((count + 1))
  if [ "$count" -eq 1 ]; then
    printf '%s\n' '{{"jsonrpc":"2.0","id":0,"error":{{"code":-32000,"message":"retry me"}}}}'
  else
    printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"id\":$((count - 1)),\"result\":{{\"stopReason\":\"end_turn\"}}}}"
  fi
done"#
        );
        let acp = AcpClient::spawn("bash", &["-c".to_string(), script], &[], false)
            .await
            .expect("spawn lifecycle ACP script");
        let mut agent = OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: None,
            agent_name: "legacy-test-agent".into(),
            goose_system_prompt_supported: None,
            protocol_version: 1,
        };
        agent.state.heartbeat_session = Some("live-session".into());

        let mut ctx = make_prompt_context_no_owner();
        ctx.base_prompt = Some("standing-once");
        let ctx = Arc::new(ctx);
        let (result_tx, mut result_rx) = mpsc::unbounded_channel();

        for turn in 1..=3 {
            run_prompt_task(
                agent,
                None,
                Some(format!("heartbeat-{turn}")),
                Arc::clone(&ctx),
                result_tx.clone(),
                None,
                format!("turn-{turn}"),
            )
            .await;
            let result = result_rx.recv().await.expect("prompt result");
            match turn {
                1 => assert!(matches!(result.outcome, PromptOutcome::Error(_))),
                _ => assert!(matches!(
                    result.outcome,
                    PromptOutcome::Ok(StopReason::EndTurn)
                )),
            }
            assert_eq!(
                result.agent.state.heartbeat_standing_context_sent,
                turn >= 2,
                "failed first delivery must not commit; first success must commit"
            );
            agent = result.agent;
        }
        agent.acp.shutdown().await;

        let requests: Vec<serde_json::Value> = std::fs::read_to_string(&capture)
            .expect("read captured ACP requests")
            .lines()
            .map(|line| serde_json::from_str(line).expect("captured request is JSON"))
            .collect();
        std::fs::remove_file(&capture).expect("remove ACP capture");
        assert_eq!(requests.len(), 3);
        let prompt_text = |index: usize| {
            requests[index]["params"]["prompt"][0]["text"]
                .as_str()
                .expect("text prompt")
        };
        assert_eq!(prompt_text(0), "[Base]\nstanding-once\n\nheartbeat-1");
        assert_eq!(
            prompt_text(1),
            "[Base]\nstanding-once\n\nheartbeat-2",
            "retry after ACP failure must resend standing context"
        );
        assert_eq!(
            prompt_text(2),
            "heartbeat-3",
            "turn after ACP success must omit standing context"
        );
    }

    #[tokio::test]
    async fn channel_prompt_commits_delivery_state_only_after_acp_success() {
        let capture = std::env::temp_dir().join(format!(
            "buzz-acp-channel-delivery-lifecycle-{}.ndjson",
            Uuid::new_v4()
        ));
        let quoted_capture = capture.to_string_lossy().replace('\'', "'\\''");
        let script = format!(
            r#"count=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> '{quoted_capture}'
  count=$((count + 1))
  if [ "$count" -eq 1 ]; then
    printf '%s\n' '{{"jsonrpc":"2.0","id":0,"error":{{"code":-32000,"message":"retry me"}}}}'
  else
    printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"id\":$((count - 1)),\"result\":{{\"stopReason\":\"end_turn\"}}}}"
  fi
done"#
        );
        let acp = AcpClient::spawn("bash", &["-c".to_string(), script], &[], false)
            .await
            .expect("spawn channel lifecycle ACP script");
        let channel_id = Uuid::new_v4();
        let mut agent = OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: None,
            agent_name: "legacy-test-agent".into(),
            goose_system_prompt_supported: None,
            protocol_version: 1,
        };
        agent
            .state
            .sessions
            .insert(channel_id, "live-session".into());
        agent
            .state
            .deliveries
            .insert(channel_id, ChannelDeliveryState::default());

        let mut ctx = make_prompt_context_no_owner();
        ctx.base_prompt = Some("standing-once");
        let ctx = Arc::new(ctx);
        let (result_tx, mut result_rx) = mpsc::unbounded_channel();

        for turn in 1..=3 {
            let event = EventBuilder::new(Kind::Custom(9), format!("channel-{turn}"))
                .sign_with_keys(&Keys::generate())
                .unwrap();
            let event_id = event.id.to_hex();
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
            run_prompt_task(
                agent,
                Some(batch),
                None,
                Arc::clone(&ctx),
                result_tx.clone(),
                None,
                format!("turn-{turn}"),
            )
            .await;
            let result = result_rx.recv().await.expect("prompt result");
            match turn {
                1 => assert!(matches!(result.outcome, PromptOutcome::Error(_))),
                _ => assert!(matches!(
                    result.outcome,
                    PromptOutcome::Ok(StopReason::EndTurn)
                )),
            }
            let delivery = &result.agent.state.deliveries[&channel_id];
            assert_eq!(
                delivery.standing_context_sent,
                turn >= 2,
                "failed channel delivery must not commit; first success must commit"
            );
            assert_eq!(
                delivery.delivered_event_ids.contains(&event_id),
                turn >= 2,
                "channel event IDs must commit only after ACP success"
            );
            agent = result.agent;
        }
        agent.acp.shutdown().await;

        let requests: Vec<serde_json::Value> = std::fs::read_to_string(&capture)
            .expect("read captured ACP requests")
            .lines()
            .map(|line| serde_json::from_str(line).expect("captured request is JSON"))
            .collect();
        std::fs::remove_file(&capture).expect("remove ACP capture");
        let prompt_text = |index: usize| {
            requests[index]["params"]["prompt"][0]["text"]
                .as_str()
                .expect("text prompt")
        };
        assert!(prompt_text(0).contains("[Base]\nstanding-once"));
        assert!(
            prompt_text(1).contains("[Base]\nstanding-once"),
            "retry after channel ACP failure must resend standing context"
        );
        assert!(
            !prompt_text(2).contains("[Base]\nstanding-once"),
            "turn after channel ACP success must omit standing context"
        );
    }

    #[tokio::test]
    async fn merged_cancel_prompt_commits_and_deduplicates_all_rendered_event_ids() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let channel_id = Uuid::new_v4();
        let keys = Keys::generate();
        let carry_over = EventBuilder::new(Kind::Custom(9), "merged carry-over sentinel")
            .sign_with_keys(&keys)
            .unwrap();
        let carry_over_id = carry_over.id.to_hex();
        let new_event = EventBuilder::new(Kind::Custom(9), "merged new-event sentinel")
            .sign_with_keys(&keys)
            .unwrap();
        let new_event_id = new_event.id.to_hex();
        let next_event = EventBuilder::new(Kind::Custom(9), "ordinary next-turn sentinel")
            .sign_with_keys(&keys)
            .unwrap();
        let merged_batch = FlushBatch {
            channel_id,
            events: vec![crate::queue::BatchEvent {
                event: new_event.clone(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![crate::queue::BatchEvent {
                event: carry_over.clone(),
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancel_reason: Some(crate::queue::CancelReason::Steer),
        };
        let next_batch = FlushBatch {
            channel_id,
            events: vec![crate::queue::BatchEvent {
                event: next_event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };

        // Return both merged events as DM history. They must be excluded from
        // the merged prompt's context and, after success, from the next turn.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind context server");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let response_body = serde_json::to_string(&vec![carry_over, new_event]).unwrap();
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = vec![0; 16 * 1024];
                let _ = socket.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(), response_body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let capture = std::env::temp_dir().join(format!(
            "buzz-acp-merged-delivery-wire-{}.ndjson",
            Uuid::new_v4()
        ));
        let quoted_capture = capture.to_string_lossy().replace('\'', "'\\''");
        let script = format!(
            r#"count=0
while IFS= read -r line; do
  printf '%s\n' "$line" >> '{quoted_capture}'
  printf '%s\n' "{{\"jsonrpc\":\"2.0\",\"id\":$count,\"result\":{{\"stopReason\":\"end_turn\"}}}}"
  count=$((count + 1))
done"#
        );
        let acp = AcpClient::spawn("bash", &["-c".into(), script], &[], false)
            .await
            .expect("spawn wire-capture ACP");
        let mut agent = OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: None,
            agent_name: "legacy-test-agent".into(),
            goose_system_prompt_supported: None,
            protocol_version: 1,
        };
        agent
            .state
            .sessions
            .insert(channel_id, "live-session".into());
        agent
            .state
            .deliveries
            .insert(channel_id, ChannelDeliveryState::default());

        let mut ctx = make_prompt_context_no_owner();
        ctx.context_message_limit = 10;
        ctx.rest_client.base_url = base_url.clone();
        ctx.channel_info = ChannelInfoResolver::new(
            HashMap::from([(
                channel_id,
                crate::relay::ChannelInfo {
                    name: "test-dm".into(),
                    channel_type: "dm".into(),
                    description: None,
                },
            )]),
            RestClient {
                http: reqwest::Client::new(),
                base_url,
                keys: ctx.agent_keys.clone(),
                auth_tag_json: None,
            },
        );
        let ctx = Arc::new(ctx);
        let (result_tx, mut result_rx) = mpsc::unbounded_channel();

        for (turn_id, batch) in [("merged-turn", merged_batch), ("next-turn", next_batch)] {
            run_prompt_task(
                agent,
                Some(batch),
                None,
                Arc::clone(&ctx),
                result_tx.clone(),
                None,
                turn_id.into(),
            )
            .await;
            let result = result_rx.recv().await.expect("prompt result");
            assert!(matches!(
                result.outcome,
                PromptOutcome::Ok(StopReason::EndTurn)
            ));
            agent = result.agent;
        }
        let delivery = &agent.state.deliveries[&channel_id];
        assert!(delivery.delivered_event_ids.contains(&carry_over_id));
        assert!(delivery.delivered_event_ids.contains(&new_event_id));
        agent.acp.shutdown().await;
        server.abort();

        let requests: Vec<serde_json::Value> = std::fs::read_to_string(&capture)
            .expect("read captured prompts")
            .lines()
            .map(|line| serde_json::from_str(line).expect("captured prompt JSON"))
            .collect();
        std::fs::remove_file(&capture).expect("remove prompt capture");
        assert_eq!(requests.len(), 2);
        let wire = |index: usize| {
            requests[index]["params"]["prompt"]
                .as_array()
                .expect("prompt blocks")
                .iter()
                .filter_map(|block| block["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let merged_wire = wire(0);
        assert_eq!(merged_wire.matches("merged carry-over sentinel").count(), 1);
        assert_eq!(merged_wire.matches("merged new-event sentinel").count(), 1);
        let next_wire = wire(1);
        assert!(next_wire.contains("ordinary next-turn sentinel"));
        assert!(!next_wire.contains("merged carry-over sentinel"));
        assert!(!next_wire.contains("merged new-event sentinel"));
        assert!(!next_wire.contains(&carry_over_id));
        assert!(!next_wire.contains(&new_event_id));
    }

    #[tokio::test]
    async fn late_successful_steer_ack_excludes_event_from_next_channel_wire_prompt() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let channel_id = Uuid::new_v4();
        let keys = Keys::generate();
        let steered_event = EventBuilder::new(Kind::Custom(9), "steered context must not replay")
            .sign_with_keys(&keys)
            .unwrap();
        let steered_event_id = steered_event.id.to_hex();
        let trigger = EventBuilder::new(Kind::Custom(9), "ordinary next turn")
            .sign_with_keys(&keys)
            .unwrap();
        let batch = FlushBatch {
            channel_id,
            events: vec![crate::queue::BatchEvent {
                event: trigger,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        };

        // The local REST bridge returns the already-delivered steer as DM
        // history. Profile/reaction requests may also arrive; the same valid
        // event array is harmless for those best-effort paths.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind context server");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let response_body = serde_json::to_string(&vec![steered_event]).unwrap();
        let server = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = vec![0; 16 * 1024];
                let _ = socket.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response_body.len(), response_body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });

        let capture = std::env::temp_dir().join(format!(
            "buzz-acp-late-steer-wire-{}.ndjson",
            Uuid::new_v4()
        ));
        let quoted_capture = capture.to_string_lossy().replace('\'', "'\\''");
        let script = format!(
            r#"IFS= read -r line
printf '%s\n' "$line" > '{quoted_capture}'
printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"stopReason":"end_turn"}}}}'"#
        );
        let acp = AcpClient::spawn("bash", &["-c".into(), script], &[], false)
            .await
            .expect("spawn wire-capture ACP");
        let mut agent = OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: None,
            agent_name: "legacy-test-agent".into(),
            goose_system_prompt_supported: None,
            protocol_version: 1,
        };
        agent
            .state
            .sessions
            .insert(channel_id, "live-session".into());
        agent
            .state
            .deliveries
            .insert(channel_id, ChannelDeliveryState::default());

        // Model the adversarial ordering: the task result has already retired
        // its TaskMeta and returned the agent before the successful ack arrives.
        let mut pool = AgentPool::from_slots(vec![Some(agent)]);
        assert!(pool.record_successful_steer(
            channel_id,
            steered_event_id.clone(),
            "live-session".into(),
        ));
        let agent = pool
            .try_claim(Some(channel_id))
            .expect("claim returned agent");

        let mut ctx = make_prompt_context_no_owner();
        ctx.context_message_limit = 10;
        ctx.rest_client.base_url = base_url.clone();
        ctx.channel_info = ChannelInfoResolver::new(
            HashMap::from([(
                channel_id,
                crate::relay::ChannelInfo {
                    name: "test-dm".into(),
                    channel_type: "dm".into(),
                    description: None,
                },
            )]),
            RestClient {
                http: reqwest::Client::new(),
                base_url,
                keys: ctx.agent_keys.clone(),
                auth_tag_json: None,
            },
        );
        let (result_tx, mut result_rx) = mpsc::unbounded_channel();
        run_prompt_task(
            agent,
            Some(batch),
            None,
            Arc::new(ctx),
            result_tx,
            None,
            "next-turn".into(),
        )
        .await;
        let mut result = result_rx.recv().await.expect("next prompt result");
        assert!(matches!(
            result.outcome,
            PromptOutcome::Ok(StopReason::EndTurn)
        ));
        result.agent.acp.shutdown().await;
        server.abort();

        let request: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&capture).expect("read captured prompt"))
                .expect("captured prompt JSON");
        std::fs::remove_file(&capture).expect("remove prompt capture");
        let wire = request["params"]["prompt"]
            .as_array()
            .expect("prompt blocks")
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(wire.contains("ordinary next turn"));
        assert!(!wire.contains("steered context must not replay"));
        assert!(!wire.contains(&steered_event_id));
    }

    #[test]
    fn delivery_state_commits_only_when_explicitly_marked_successful() {
        let channel = Uuid::new_v4();
        let mut state = SessionState::default();
        state
            .deliveries
            .insert(channel, ChannelDeliveryState::default());

        // Building or attempting a prompt does not mutate delivery state.
        let delivery = state.deliveries.get(&channel).unwrap();
        assert!(!delivery.standing_context_sent);
        assert!(delivery.delivered_event_ids.is_empty());

        state.mark_channel_delivery_success(
            channel,
            true,
            ["trigger".to_string(), "context".to_string()],
        );
        let delivery = state.deliveries.get(&channel).unwrap();
        assert!(delivery.standing_context_sent);
        assert_eq!(delivery.delivered_event_ids.len(), 2);
    }

    #[test]
    fn delivery_state_is_cleared_on_rotation_and_restarts_empty() {
        let channel = Uuid::new_v4();
        let mut state = SessionState::default();
        state.sessions.insert(channel, "old-session".into());
        state.mark_channel_delivery_success(channel, true, ["old-event".to_string()]);

        assert!(state.invalidate_channel(&channel));
        assert!(!state.deliveries.contains_key(&channel));

        state.sessions.insert(channel, "new-session".into());
        state
            .deliveries
            .insert(channel, ChannelDeliveryState::default());
        let delivery = state.deliveries.get(&channel).unwrap();
        assert!(!delivery.standing_context_sent);
        assert!(delivery.delivered_event_ids.is_empty());
    }

    #[test]
    fn conversation_context_delta_omits_delivered_and_triggering_events() {
        let delivered = HashSet::from(["old".to_string()]);
        let triggering = HashSet::from(["trigger".to_string()]);
        let context = ConversationContext::Thread {
            messages: vec![
                context_message("old", "already sent"),
                context_message("trigger", "rendered as trigger"),
                context_message("new", "new context"),
            ],
            total: 3,
            truncated: false,
        };

        let delta = conversation_context_delta(Some(context), &delivered, &triggering)
            .expect("new context remains");
        match delta {
            ConversationContext::Thread {
                messages,
                total,
                truncated,
            } => {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].event_id, "new");
                assert_eq!(total, 3);
                assert!(!truncated);
            }
            _ => panic!("expected thread context"),
        }
    }

    #[test]
    fn conversation_context_delta_returns_none_when_no_new_events_remain() {
        let delivered = HashSet::from(["old".to_string()]);
        let context = ConversationContext::Dm {
            messages: vec![context_message("old", "already sent")],
            total: 1,
            truncated: false,
        };

        assert!(conversation_context_delta(Some(context), &delivered, &HashSet::new()).is_none());
    }

    #[test]
    fn conversation_context_delta_preserves_unidentified_legacy_messages() {
        let context = ConversationContext::Dm {
            messages: vec![context_message("", "cannot safely deduplicate")],
            total: 1,
            truncated: false,
        };

        assert!(
            conversation_context_delta(Some(context), &HashSet::new(), &HashSet::new()).is_some()
        );
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
        s.deliveries.insert(
            ch_a,
            ChannelDeliveryState {
                standing_context_sent: true,
                delivered_event_ids: HashSet::from(["event-a".into()]),
            },
        );
        s.deliveries.insert(
            ch_b,
            ChannelDeliveryState {
                standing_context_sent: true,
                delivered_event_ids: HashSet::from(["event-b".into()]),
            },
        );
        s.heartbeat_session = Some("sess-hb".into());
        s.heartbeat_turn_count = 7;
        s.heartbeat_standing_context_sent = true;
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
        assert!(!s.heartbeat_standing_context_sent);
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
        assert!(!s.heartbeat_standing_context_sent);
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
            &ControlSignal::SwitchModel {
                model_id: "gpt-5".into(),
                request_id: None,
            },
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
                ControlSignal::SwitchModel {
                    model_id: "gpt-5".into(),
                    request_id: None,
                },
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
                signal: ControlSignal::SwitchModel {
                    model_id: "gpt-5".to_string(),
                    request_id: None,
                },
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
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: None,
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
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: None,
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
            turn_cache_write_tokens: None,
            cumulative_input_tokens: Some(100),
            cumulative_output_tokens: Some(50),
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            cumulative_cache_write_tokens: None,
            model: None,
            pricing_identity: None,
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
            turn_cache_write_tokens: None,
            cumulative_input_tokens: Some(200),
            cumulative_output_tokens: Some(80),
            cumulative_total_tokens: None,
            cumulative_cost_usd: Some(0.001),
            cumulative_cache_read_tokens: None,
            cumulative_cache_write_tokens: None,
            model: None,
            pricing_identity: None,
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
            turn_cache_write_tokens: None,
            cumulative_input_tokens: Some(150),
            cumulative_output_tokens: Some(70),
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            cumulative_cache_write_tokens: None,
            model: None,
            pricing_identity: None,
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
            turn_cache_write_tokens: None,
            cumulative_input_tokens: Some(400),
            cumulative_output_tokens: Some(100),
            cumulative_total_tokens: None,
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            cumulative_cache_write_tokens: None,
            model: None,
            pricing_identity: None,
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
            turn_cache_write_tokens: None,
            cumulative_input_tokens: Some(500),
            cumulative_output_tokens: Some(120),
            cumulative_total_tokens: Some(620), // genuine cumulative total
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            cumulative_cache_write_tokens: None,
            model: None,
            pricing_identity: None,
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
            turn_cache_write_tokens: None,
            cumulative_input_tokens: Some(200),
            cumulative_output_tokens: Some(60),
            cumulative_total_tokens: None, // session has no total
            cumulative_cost_usd: None,
            cumulative_cache_read_tokens: None,
            cumulative_cache_write_tokens: None,
            model: None,
            pricing_identity: None,
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
        // cache_write_tokens: None in this test because the payloads don't
        // include accumulatedCacheWriteTokens (Anthropic cache-read only test).
        assert!(
            turn2.cache_write_tokens.is_none(),
            "cache_write_tokens must be None when harness omits the field"
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

    pub(super) fn make_prompt_context_no_owner() -> PromptContext {
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
        }
    }

    // ── huddle instructions ─────────────────────────────────────────────────

    #[test]
    fn huddle_instructions_append_as_system_section() {
        assert_eq!(
            with_huddle_instructions(Some("base".into()), Some("  reply now  ")).as_deref(),
            Some("base\n\n[Huddle Instructions]\nreply now")
        );
    }

    #[test]
    fn huddle_instructions_require_owner_signature_and_channel() {
        let owner = Keys::generate();
        let stranger = Keys::generate();
        let channel = Uuid::parse_str("00f1ccaf-1506-4dd7-9a0e-fa67e9e486ae").unwrap();
        let event = |keys: &Keys, channel_id: Uuid| {
            let channel_id = channel_id.to_string();
            let h_tag = Tag::parse(["h", channel_id.as_str()]).unwrap();
            serde_json::to_value(
                EventBuilder::new(
                    Kind::Custom(buzz_core::kind::KIND_HUDDLE_GUIDELINES as u16),
                    "reply immediately",
                )
                .tags([h_tag])
                .sign_with_keys(keys)
                .unwrap(),
            )
            .unwrap()
        };

        assert_eq!(
            huddle_instructions_from_query_response(
                &[event(&owner, channel)],
                channel,
                &owner.public_key(),
            )
            .as_deref(),
            Some("reply immediately")
        );
        assert!(huddle_instructions_from_query_response(
            &[event(&stranger, channel)],
            channel,
            &owner.public_key(),
        )
        .is_none());
        assert!(huddle_instructions_from_query_response(
            &[event(&owner, Uuid::new_v4())],
            channel,
            &owner.public_key(),
        )
        .is_none());
    }

    // ── render_canvas_section ────────────────────────────────────────────────

    #[test]
    fn test_render_canvas_section_produces_exact_shape() {
        let id = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let ts = "2024-01-15T10:30:00+00:00";
        let uuid = "00f1ccaf-1506-4dd7-9a0e-fa67e9e486ae";
        let section = render_canvas_section(id, ts, uuid);
        assert_eq!(
            section,
            "[Channel Canvas]\n\
             Canvas revision (event ID): a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2\n\
             Last modified: 2024-01-15T10:30:00+00:00\n\
             Fetch current content with: buzz canvas get --channel 00f1ccaf-1506-4dd7-9a0e-fa67e9e486ae"
        );
    }

    // ── with_canvas ──────────────────────────────────────────────────────────

    #[test]
    fn test_with_canvas_appends_to_existing_prompt() {
        let result = with_canvas(Some("base content".into()), Some("[Channel Canvas]\nstuff"));
        assert_eq!(result.unwrap(), "base content\n\n[Channel Canvas]\nstuff");
    }

    #[test]
    fn test_with_canvas_returns_canvas_alone_when_no_prompt() {
        let result = with_canvas(None, Some("[Channel Canvas]\nstuff"));
        assert_eq!(result.unwrap(), "[Channel Canvas]\nstuff");
    }

    #[test]
    fn test_with_canvas_returns_prompt_alone_when_no_canvas() {
        let result = with_canvas(Some("base content".into()), None);
        assert_eq!(result.unwrap(), "base content");
    }

    #[test]
    fn test_with_canvas_returns_none_when_both_absent() {
        let result = with_canvas(None, None);
        assert!(result.is_none());
    }

    // ── canvas_sections cache invalidation ───────────────────────────────────

    #[test]
    fn test_invalidate_channel_clears_canvas_section() {
        let ch = Uuid::new_v4();
        let mut s = SessionState::default();
        s.sessions.insert(ch, "sess".into());
        s.canvas_sections
            .insert(ch, "[Channel Canvas]\nrev abc".into());

        s.invalidate_channel(&ch);

        assert!(!s.canvas_sections.contains_key(&ch));
        assert!(!s.sessions.contains_key(&ch));
    }

    #[test]
    fn test_invalidate_all_clears_canvas_sections() {
        let ch_a = Uuid::new_v4();
        let ch_b = Uuid::new_v4();
        let mut s = SessionState::default();
        s.canvas_sections.insert(ch_a, "canvas-a".into());
        s.canvas_sections.insert(ch_b, "canvas-b".into());
        s.sessions.insert(ch_a, "sess-a".into());

        s.invalidate_all();

        assert!(s.canvas_sections.is_empty());
        assert!(s.sessions.is_empty());
    }

    #[test]
    fn test_invalidate_channel_leaves_other_channels_canvas_intact() {
        let ch_a = Uuid::new_v4();
        let ch_b = Uuid::new_v4();
        let mut s = SessionState::default();
        s.sessions.insert(ch_a, "sess-a".into());
        s.sessions.insert(ch_b, "sess-b".into());
        s.canvas_sections.insert(ch_a, "canvas-a".into());
        s.canvas_sections.insert(ch_b, "canvas-b".into());

        s.invalidate_channel(&ch_a);

        assert!(!s.canvas_sections.contains_key(&ch_a));
        assert_eq!(s.canvas_sections.get(&ch_b).unwrap(), "canvas-b");
    }

    #[test]
    fn test_has_channel_state_true_when_only_canvas_section_present() {
        let ch = Uuid::new_v4();
        let mut s = SessionState::default();
        s.canvas_sections.insert(ch, "canvas".into());
        assert!(s.has_channel_state(&ch));
    }

    // ── canvas_section_from_query_response ───────────────────────────────────

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

    #[test]
    fn test_canvas_section_from_query_response_happy_path() {
        let ev = make_canvas_event_value("# Team instructions\nBe helpful.");
        let id = ev["id"].as_str().unwrap().to_string();
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        let section = result.expect("expected Some");
        assert!(section.contains(&id), "section must contain the event id");
        assert!(section.contains("buzz canvas get --channel"));
        assert!(section.contains(CHANNEL_UUID));
        assert!(section.starts_with("[Channel Canvas]"));
        // Timestamp must use Z suffix, not +00:00
        assert!(section.contains('Z'), "timestamp must use Z suffix");
    }

    #[test]
    fn test_canvas_section_from_query_response_empty_array_returns_none() {
        let result = canvas_section_from_query_response(&[], CHANNEL_UUID);
        assert!(result.is_none());
    }

    #[test]
    fn test_canvas_section_from_query_response_blank_content_returns_none() {
        let ev = make_canvas_event_value("   ");
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        assert!(
            result.is_none(),
            "blank content must return None (cleared canvas)"
        );
    }

    #[test]
    fn test_canvas_section_from_query_response_empty_content_returns_none() {
        let ev = make_canvas_event_value("");
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        assert!(result.is_none());
    }

    /// A bare JSON object with a plausible-looking id but missing pubkey/sig/kind/tags
    /// must be rejected — not silently accepted with partial metadata.
    #[test]
    fn test_canvas_section_from_query_response_partial_object_returns_none() {
        let partial = serde_json::json!({
            "id": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
            "created_at": 1705312200_i64,
            "content": "some instructions"
        });
        let result = canvas_section_from_query_response(&[partial], CHANNEL_UUID);
        assert!(
            result.is_none(),
            "partial event object (missing pubkey/sig/kind/tags) must return None"
        );
    }

    /// A JSON object that looks like an event but has `created_at` as a string
    /// must be rejected — the nostr::Event parser enforces integer type.
    #[test]
    fn test_canvas_section_from_query_response_string_timestamp_returns_none() {
        let keys = Keys::generate();
        let h_tag = Tag::parse(["h", CHANNEL_UUID]).expect("h tag");
        let mut ev = serde_json::to_value(
            EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), "content")
                .tags([h_tag])
                .sign_with_keys(&keys)
                .expect("sign"),
        )
        .expect("serialise");
        // Corrupt created_at to a string value.
        ev["created_at"] = serde_json::Value::String("2026-03-15T16:30:00+00:00".into());
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        assert!(
            result.is_none(),
            "string created_at must be rejected by nostr::Event deserialiser"
        );
    }

    /// A JSON object that looks like an event but is missing `created_at`
    /// must be rejected — nostr::Event requires the field.
    #[test]
    fn test_canvas_section_from_query_response_missing_timestamp_returns_none() {
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
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        assert!(
            result.is_none(),
            "missing created_at must be rejected by nostr::Event deserialiser"
        );
    }

    /// An event with a timestamp at Timestamp::max() (u64::MAX) must return None.
    ///
    /// `u64::MAX as i64` wraps to -1, which chrono silently accepts as
    /// 1969-12-31T23:59:59Z. The checked i64::try_from must reject it first.
    #[test]
    fn test_canvas_section_from_query_response_timestamp_max_returns_none() {
        let keys = Keys::generate();
        let h_tag = Tag::parse(["h", CHANNEL_UUID]).expect("h tag");
        let ev = serde_json::to_value(
            EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), "content")
                .tags([h_tag])
                .custom_created_at(Timestamp::max())
                .sign_with_keys(&keys)
                .expect("sign"),
        )
        .expect("serialise");
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        assert!(
            result.is_none(),
            "Timestamp::max() (u64::MAX) must return None — not wrap to 1969"
        );
    }

    /// A structurally complete but tampered event (content altered after signing)
    /// must be rejected by event.verify().
    #[test]
    fn test_canvas_section_from_query_response_tampered_event_returns_none() {
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
        // Tamper the content after signing — id and sig no longer agree.
        ev["content"] = serde_json::Value::String("injected instructions".into());
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        assert!(
            result.is_none(),
            "tampered event must fail verify() and return None"
        );
    }

    /// An event with the wrong kind (not 40100) must be rejected.
    #[test]
    fn test_canvas_section_from_query_response_wrong_kind_returns_none() {
        let keys = Keys::generate();
        let h_tag = Tag::parse(["h", CHANNEL_UUID]).expect("h tag");
        let ev = serde_json::to_value(
            EventBuilder::new(Kind::Custom(9), "content")
                .tags([h_tag])
                .sign_with_keys(&keys)
                .expect("sign"),
        )
        .expect("serialise");
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        assert!(result.is_none(), "wrong kind must return None");
    }

    /// An event missing the expected h-tag (or carrying a different channel UUID)
    /// must be rejected.
    #[test]
    fn test_canvas_section_from_query_response_wrong_h_tag_returns_none() {
        let keys = Keys::generate();
        let wrong_h = Tag::parse(["h", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"]).expect("h tag");
        let ev = serde_json::to_value(
            EventBuilder::new(Kind::Custom(buzz_core::kind::KIND_CANVAS as u16), "content")
                .tags([wrong_h])
                .sign_with_keys(&keys)
                .expect("sign"),
        )
        .expect("serialise");
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        assert!(result.is_none(), "mismatched h-tag must return None");
    }

    #[test]
    fn test_canvas_section_from_query_response_timestamp_uses_z_suffix() {
        let ev = make_canvas_event_value("instructions");
        let result = canvas_section_from_query_response(&[ev], CHANNEL_UUID);
        let section = result.expect("valid event must produce a section");
        assert!(
            section.contains('Z'),
            "RFC3339 timestamp must use Z suffix, not +00:00"
        );
        assert!(
            !section.contains("+00:00"),
            "timestamp must not use +00:00 offset"
        );
    }

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

    /// A normal channel yields a non-DM (canvas allowed) and its name for the
    /// title suffix — and the second consumer reads it from cache, not the wire.
    #[tokio::test]
    async fn test_new_session_channel_context_qualifies_a_normal_channel() {
        use std::sync::atomic::Ordering;

        let id = Uuid::new_v4();
        let response = channel_metadata_response(id, &[["name", "buzz-dev"], ["t", "stream"]]);
        let (resolver, requests, server) = counting_resolver(response).await;

        let (is_dm, title_channel, channel_type) =
            resolve_new_session_channel_context(&resolver, id).await;
        assert!(!is_dm, "a stream channel is not a DM");
        assert_eq!(title_channel.as_deref(), Some("buzz-dev"));
        assert_eq!(channel_type.as_deref(), Some("stream"));
        assert_eq!(requests.load(Ordering::SeqCst), 1);

        let (_, again, _) = resolve_new_session_channel_context(&resolver, id).await;
        assert_eq!(again.as_deref(), Some("buzz-dev"));
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "a resolved channel is cached — no second lookup"
        );
        server.abort();
    }

    /// A channel's `about` tag is parsed through the lazy-fetch path and
    /// delivered as the resolved description.
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

    /// A metadata event with no `about` tag yields no description.
    #[tokio::test]
    async fn test_channel_resolver_absent_description_when_no_about_tag() {
        let id = Uuid::new_v4();
        let response = channel_metadata_response(id, &[["name", "buzz-dev"], ["t", "stream"]]);
        let (resolver, _requests, server) = counting_resolver(response).await;

        let info = resolver.resolve(id).await.expect("should resolve");
        assert_eq!(info.description, None);
        server.abort();
    }

    /// A DM carries no useful name, so it gets the bare agent title (and no
    /// canvas section).
    #[tokio::test]
    async fn test_new_session_channel_context_leaves_a_dm_unqualified() {
        let id = Uuid::new_v4();
        let response = channel_metadata_response(id, &[["name", "DM"], ["t", "dm"]]);
        let (resolver, _requests, server) = counting_resolver(response).await;

        let (is_dm, title_channel, channel_type) =
            resolve_new_session_channel_context(&resolver, id).await;
        assert!(is_dm);
        assert_eq!(channel_type.as_deref(), Some("dm"));
        assert_eq!(
            title_channel, None,
            "a DM name must never reach the session title"
        );
        server.abort();
    }

    /// The `"unknown"` placeholder `fetch_channel_info` substitutes for a
    /// metadata event with no `name` tag is not a channel name: qualifying with
    /// it would title every unnamed channel `Agent · #unknown`.
    #[tokio::test]
    async fn test_new_session_channel_context_treats_the_unknown_name_as_absent() {
        let id = Uuid::new_v4();
        let response = channel_metadata_response(id, &[["t", "stream"]]);
        let (resolver, _requests, server) = counting_resolver(response).await;

        let (is_dm, title_channel, _) = resolve_new_session_channel_context(&resolver, id).await;
        assert!(!is_dm, "a nameless stream channel is still not a DM");
        assert_eq!(
            title_channel, None,
            "the `unknown` placeholder must yield a bare title"
        );
        server.abort();
    }

    /// An unresolvable channel yields the bare title, fails closed as a DM, and
    /// costs exactly ONE `fetch_channel_info` sequence — two attempts, because
    /// `fetch_with_retry` retries once. `resolve()` caches only `Some`, so a
    /// second resolve for the title would double this in front of `session/new`,
    /// exactly when the relay is already degraded.
    #[tokio::test]
    async fn test_new_session_channel_context_attempts_an_unresolved_channel_once() {
        use std::sync::atomic::Ordering;

        let (resolver, requests, server) = counting_resolver(json!([])).await;

        let (is_dm, title_channel, channel_type) =
            resolve_new_session_channel_context(&resolver, Uuid::new_v4()).await;
        assert!(is_dm, "an undeterminable channel type must fail closed");
        assert_eq!(title_channel, None, "unresolved channels get a bare title");
        assert_eq!(channel_type, None);
        assert_eq!(
            requests.load(Ordering::SeqCst),
            2,
            "one fetch_channel_info sequence (initial attempt + single retry)"
        );
        server.abort();
    }
}

#[cfg(test)]
mod startup_effort_tests {
    use super::*;
    use crate::acp::AcpClient;
    use tests::make_prompt_context_no_owner;

    /// Build a protocol-v2, non-goose agent whose only ACP requests will be
    /// `session/new` (id 0) then the startup-effort `session/set_config_option`
    /// (id 1). `startup_effort` is the held spawn-scoped value under test.
    fn effort_agent(acp: AcpClient, startup_effort: Option<&str>) -> OwnedAgent {
        OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: startup_effort.map(str::to_string),
            agent_name: "effort-test-agent".into(),
            goose_system_prompt_supported: None,
            protocol_version: 2,
        }
    }

    /// Spawn a scripted ACP that answers `session/new` (request #1) with the
    /// given configOptions, then replies to the effort `set_config_option`
    /// (request #2) with `effort_reply` (a JSON-RPC `result`/`error` body, minus
    /// the id which is filled in). Any later request gets `{"ok":true}`.
    async fn spawn_effort_acp(session_new_config_options: &str, effort_reply: &str) -> AcpClient {
        let script = format!(
            r#"count=0
while IFS= read -r line; do
  count=$((count + 1))
  id=$((count - 1))
  if [ "$count" -eq 1 ]; then
    printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"sessionId":"sess-1","configOptions":{session_new_config_options}}}}}'
  elif [ "$count" -eq 2 ]; then
    printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',{effort_reply}}}'
  else
    printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"ok":true}}}}'
  fi
done"#
        );
        AcpClient::spawn("bash", &["-c".to_string(), script], &[], false)
            .await
            .expect("spawn effort ACP script")
    }

    fn captured_config_options(obs: &observer::ObserverHandle) -> serde_json::Value {
        obs.snapshot()
            .into_iter()
            .find(|e| e.kind == "session_config_captured")
            .expect("session_config_captured emitted")
            .payload["configOptions"]
            .clone()
    }

    fn effort_current_value(options: &serde_json::Value) -> Option<String> {
        options
            .as_array()?
            .iter()
            .find(|o| o["category"] == "thought_level")
            .and_then(|o| o["currentValue"].as_str())
            .map(str::to_string)
    }

    const OPTS_WITH_EFFORT_DEFAULT_LOW: &str = r#"[{"configId":"effort","category":"thought_level","currentValue":"low","options":[{"value":"low"},{"value":"high"}]}]"#;

    #[tokio::test]
    async fn test_applied_effort_patches_captured_current_value_to_high() {
        let acp = spawn_effort_acp(OPTS_WITH_EFFORT_DEFAULT_LOW, r#""result":{"ok":true}"#).await;
        let mut agent = effort_agent(acp, Some("high"));
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("session creation must succeed");

        let opts = captured_config_options(&obs);
        assert_eq!(
            effort_current_value(&opts).as_deref(),
            Some("high"),
            "applied effort must overwrite the pre-set currentValue in the capture"
        );
    }

    #[tokio::test]
    async fn test_rejected_effort_retains_captured_current_value() {
        // Adapter answers the effort set with a JSON-RPC error → AgentError →
        // application-level rejection: non-fatal, capture keeps the default.
        let acp = spawn_effort_acp(
            OPTS_WITH_EFFORT_DEFAULT_LOW,
            r#""error":{"code":-32602,"message":"unsupported effort value"}"#,
        )
        .await;
        let mut agent = effort_agent(acp, Some("high"));
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("rejection is non-fatal; session creation still succeeds");

        let opts = captured_config_options(&obs);
        assert_eq!(
            effort_current_value(&opts).as_deref(),
            Some("low"),
            "a rejected effort must not falsify the capture — keep the running value"
        );
    }

    #[tokio::test]
    async fn test_no_thought_level_model_leaves_capture_unpatched() {
        // Model advertises only a `model` option — no thought_level. The held
        // effort is silently ignored and no set_config_option is sent.
        let opts_no_effort = r#"[{"configId":"model","category":"model","currentValue":"m-a","options":[{"value":"m-a"}]}]"#;
        let acp = spawn_effort_acp(opts_no_effort, r#""result":{"ok":true}"#).await;
        let mut agent = effort_agent(acp, Some("high"));
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("session creation must succeed");

        let opts = captured_config_options(&obs);
        assert_eq!(
            opts,
            serde_json::from_str::<serde_json::Value>(opts_no_effort).unwrap(),
            "no thought_level option → capture is the untouched session/new snapshot"
        );
    }

    #[tokio::test]
    async fn test_no_startup_effort_leaves_capture_unpatched() {
        // No held effort at all: the set_config_option is never sent and the
        // default currentValue survives into the capture.
        let acp = spawn_effort_acp(OPTS_WITH_EFFORT_DEFAULT_LOW, r#""result":{"ok":true}"#).await;
        let mut agent = effort_agent(acp, None);
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("session creation must succeed");

        let opts = captured_config_options(&obs);
        assert_eq!(
            effort_current_value(&opts).as_deref(),
            Some("low"),
            "with no configured effort the capture reflects the model default"
        );
    }

    #[tokio::test]
    async fn test_transport_error_on_effort_propagates_for_respawn() {
        // Adapter exits after answering session/new but before the effort set →
        // AgentExited (transport class) → Err so the caller respawns the worker
        // instead of reusing a possibly-poisoned stream.
        let script = format!(
            r#"IFS= read -r _new
printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"sessionId":"sess-1","configOptions":{OPTS_WITH_EFFORT_DEFAULT_LOW}}}}}'
IFS= read -r _effort
exit 0"#
        );
        let acp = AcpClient::spawn("bash", &["-c".to_string(), script], &[], false)
            .await
            .expect("spawn transport-exit ACP script");
        let mut agent = effort_agent(acp, Some("high"));

        let ctx = make_prompt_context_no_owner();
        let err = create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect_err("transport-class effort failure must propagate as Err");
        assert!(
            matches!(err, AcpError::AgentExited | AcpError::Io(_)),
            "process exit mid-effort is a transport error, got {err:?}"
        );
    }

    #[test]
    fn test_patch_config_option_current_value_matches_by_id_key() {
        // The `id` key (claude-agent-acp) must also match, not just `configId`.
        let mut opts = serde_json::json!([
            { "id": "effort", "category": "thought_level", "currentValue": "low" }
        ]);
        patch_config_option_current_value(&mut opts, "effort", "high");
        assert_eq!(opts[0]["currentValue"], "high");
    }

    #[test]
    fn test_patch_config_option_current_value_noop_on_non_array() {
        let mut opts = serde_json::Value::Null;
        patch_config_option_current_value(&mut opts, "effort", "high");
        assert!(opts.is_null(), "a null snapshot must stay null");
    }
}

#[cfg(test)]
mod model_switch_tests {
    use super::*;
    use crate::acp::AcpClient;
    use tests::make_prompt_context_no_owner;

    /// A protocol-v2 agent with a live `desired_model` override and no startup
    /// effort. `model_overridden` is set so the capture's `modelOverridden`
    /// reflects only whether the switch actually landed.
    fn switching_agent(acp: AcpClient, desired_model: &str) -> OwnedAgent {
        OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: Some(desired_model.to_string()),
            model_overridden: true,
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: None,
            agent_name: "switch-test-agent".into(),
            goose_system_prompt_supported: None,
            protocol_version: 2,
        }
    }

    /// Scripted ACP: `session/new` (request #1) returns `session_new_options`,
    /// then the model-switch `set_config_option` (request #2) replies with
    /// `switch_reply` (a JSON-RPC `result`/`error` body minus the id). Any later
    /// request gets `{"ok":true}`.
    async fn spawn_switch_acp(session_new_options: &str, switch_reply: &str) -> AcpClient {
        let script = format!(
            r#"count=0
while IFS= read -r line; do
  count=$((count + 1))
  id=$((count - 1))
  if [ "$count" -eq 1 ]; then
    printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{{"sessionId":"sess-1","configOptions":{session_new_options}}}}}'
  elif [ "$count" -eq 2 ]; then
    printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',{switch_reply}}}'
  else
    printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"ok":true}}}}'
  fi
done"#
        );
        AcpClient::spawn("bash", &["-c".to_string(), script], &[], false)
            .await
            .expect("spawn switch ACP script")
    }

    fn capture(obs: &observer::ObserverHandle) -> serde_json::Value {
        obs.snapshot()
            .into_iter()
            .find(|e| e.kind == "session_config_captured")
            .expect("session_config_captured emitted")
            .payload
    }

    fn control_results(obs: &observer::ObserverHandle) -> Vec<serde_json::Value> {
        obs.snapshot()
            .into_iter()
            .filter(|e| e.kind == "control_result")
            .map(|e| e.payload)
            .collect()
    }

    // A `model`-category option offering the default model plus the target the
    // agent wants to switch to.
    const OPTS_MODEL_A_AND_B: &str = r#"[{"configId":"model","category":"model","currentValue":"model-a","options":[{"value":"model-a"},{"value":"model-b"}]}]"#;

    #[tokio::test]
    async fn test_applied_switch_refreshes_capabilities_from_post_switch_snapshot() {
        // The adapter accepts the switch and echoes the target model's rebuilt
        // configOptions — including a thought_level option the default model
        // never advertised. Capabilities and the capture must reflect the target
        // model, not the pre-switch default.
        let switch_reply = r#""result":{"configOptions":[{"configId":"model","category":"model","currentValue":"model-b","options":[{"value":"model-a"},{"value":"model-b"}]},{"configId":"effort","category":"thought_level","currentValue":"medium","options":[{"value":"low"},{"value":"medium"}]}]}"#;
        let acp = spawn_switch_acp(OPTS_MODEL_A_AND_B, switch_reply).await;
        let mut agent = switching_agent(acp, "model-b");
        // Busy path: this switch was delivered to an in-flight turn and its apply
        // is deferred to this requeued session. Arm the pending-ack and carry the
        // pick's correlator so the Applied arm emits a correlated positive
        // terminal instead of leaving the Desktop to infer success from silence.
        agent.desired_model_pending_ack = true;
        agent.desired_model_request_id = Some("req-busy-1".into());
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("session creation must succeed");

        let caps = agent
            .model_capabilities
            .as_ref()
            .expect("capabilities refreshed from the post-switch snapshot");
        assert_eq!(
            caps.thought_level_config_id.as_deref(),
            Some("effort"),
            "the target model's thought_level option must be discovered post-switch"
        );
        let cap = capture(&obs);
        assert_eq!(
            cap["modelOverridden"], true,
            "an applied switch must report modelOverridden true"
        );
        assert!(
            cap["configOptions"]
                .as_array()
                .is_some_and(|a| a.iter().any(|o| o["category"] == "thought_level")),
            "the cached configOptions must be the target model's post-switch set"
        );
        // The deferred apply must emit exactly one correlated positive terminal
        // so the Desktop learns success from a real frame, not timeout silence.
        let results = control_results(&obs);
        assert_eq!(
            results.len(),
            1,
            "a busy-path applied switch emits exactly one positive terminal"
        );
        assert_eq!(results[0]["status"], "switched");
        assert_eq!(results[0]["modelId"], "model-b");
        assert_eq!(
            results[0]["requestId"], "req-busy-1",
            "the positive terminal must carry the pick's correlator"
        );
        assert!(
            !agent.desired_model_pending_ack,
            "the pending-ack is consumed once so it cannot re-fire on a later session"
        );
    }

    #[tokio::test]
    async fn test_rejected_switch_preserves_capabilities_and_emits_failure() {
        // The adapter refuses the switch with a JSON-RPC error. The session is
        // still on its default model: pre-switch capabilities survive, the
        // capture reports modelOverridden false, and a terminal `failure`
        // control_result tells Desktop the pick did not land.
        let acp = spawn_switch_acp(
            OPTS_MODEL_A_AND_B,
            r#""error":{"code":-32602,"message":"model not accepted"}"#,
        )
        .await;
        let mut agent = switching_agent(acp, "model-b");
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("an application-level rejection is non-fatal");

        let caps = agent
            .model_capabilities
            .as_ref()
            .expect("pre-switch capabilities must be preserved on rejection");
        assert!(
            caps.config_options_raw
                .iter()
                .any(|o| o["currentValue"] == "model-a"),
            "capabilities must still describe the default model the session runs"
        );
        let cap = capture(&obs);
        assert_eq!(
            cap["modelOverridden"], false,
            "a rejected switch must not claim an override"
        );
        let results = control_results(&obs);
        assert_eq!(results.len(), 1, "exactly one control_result on rejection");
        assert_eq!(results[0]["status"], "failure");
        assert_eq!(results[0]["modelId"], "model-b");
    }

    #[tokio::test]
    async fn test_busy_path_rejection_emits_only_failure_and_consumes_pending_ack() {
        // K1 delayed-rejection at the Rust seam: a busy-path switch is armed
        // (pending_ack), its apply is deferred to this requeued session, and the
        // adapter then refuses it. The rejection arm must emit exactly one
        // `failure` (no spurious positive `switched`) and consume the pending-ack
        // so no later session can fire a phantom success.
        let acp = spawn_switch_acp(
            OPTS_MODEL_A_AND_B,
            r#""error":{"code":-32602,"message":"model not accepted"}"#,
        )
        .await;
        let mut agent = switching_agent(acp, "model-b");
        agent.desired_model_pending_ack = true;
        agent.desired_model_request_id = Some("req-busy-reject".into());
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("an application-level rejection is non-fatal");

        let results = control_results(&obs);
        assert_eq!(
            results.len(),
            1,
            "a busy-path rejection emits exactly one terminal — no phantom success"
        );
        assert_eq!(results[0]["status"], "failure");
        assert_eq!(results[0]["requestId"], "req-busy-reject");
        assert!(
            !agent.desired_model_pending_ack,
            "the pending-ack is consumed even on rejection so it cannot re-fire"
        );
    }

    #[tokio::test]
    async fn test_applied_switch_without_options_drops_capabilities() {
        // A successful switch whose response carries no configOptions (older
        // adapter, or a model with no options): the pre-switch snapshot cannot
        // be trusted for the target model, so capabilities drop to None to be
        // re-derived on the next session — but the switch still counts as an
        // override with no failure surfaced.
        let acp = spawn_switch_acp(OPTS_MODEL_A_AND_B, r#""result":{"ok":true}"#).await;
        let mut agent = switching_agent(acp, "model-b");
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("session creation must succeed");

        assert!(
            agent.model_capabilities.is_none(),
            "an optionless successful switch must drop stale capabilities"
        );
        let cap = capture(&obs);
        assert_eq!(
            cap["modelOverridden"], true,
            "the switch still applied even with no echoed options"
        );
        assert!(
            control_results(&obs).is_empty(),
            "a successful switch emits no failure control_result"
        );
    }

    #[tokio::test]
    async fn test_unsupported_model_emits_unsupported_without_switch_rpc() {
        // The desired model is absent from the session/new catalog: no switch
        // RPC is sent, the capture reports no override, and an
        // `unsupported_model` control_result rejects the live pick.
        let acp = spawn_switch_acp(OPTS_MODEL_A_AND_B, r#""result":{"ok":true}"#).await;
        let mut agent = switching_agent(acp, "model-z");
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("an unresolvable model is non-fatal");

        let cap = capture(&obs);
        assert_eq!(cap["modelOverridden"], false);
        let results = control_results(&obs);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["status"], "unsupported_model");
        assert_eq!(results[0]["modelId"], "model-z");
    }

    /// Scripted ACP whose `session/new` (request #1) returns a full result body
    /// `session_new_result` (a JSON object minus the outer envelope), and whose
    /// model-switch `set_config_option` (request #2) replies with `switch_reply`
    /// (a JSON-RPC `result`/`error` body minus the id). Lets a test control the
    /// `models` block in both the pre-switch and post-switch snapshots.
    async fn spawn_switch_acp_full(session_new_result: &str, switch_reply: &str) -> AcpClient {
        let script = format!(
            r#"count=0
while IFS= read -r line; do
  count=$((count + 1))
  id=$((count - 1))
  if [ "$count" -eq 1 ]; then
    printf '%s\n' '{{"jsonrpc":"2.0","id":0,"result":{session_new_result}}}'
  elif [ "$count" -eq 2 ]; then
    printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',{switch_reply}}}'
  else
    printf '%s\n' '{{"jsonrpc":"2.0","id":'"$id"',"result":{{"ok":true}}}}'
  fi
done"#
        );
        AcpClient::spawn("bash", &["-c".to_string(), script], &[], false)
            .await
            .expect("spawn switch ACP script")
    }

    /// F3: an applied switch must cache `models` from the POST-switch snapshot,
    /// not the pre-switch `session/new` response. The pre-switch snapshot reports
    /// the default model as current; the target response reports the target as
    /// current. The emitted capture must carry the target's models block. The
    /// Desktop-parsing half of this contract lives in `agent_config_tests.rs`
    /// (`live_switch_models_from_post_switch_snapshot_parses_target_current`).
    #[tokio::test]
    async fn test_applied_switch_caches_target_model_not_pre_switch() {
        // session/new: model-a is current. switch reply: model-b is current,
        // and it echoes rebuilt configOptions so capabilities refresh cleanly.
        let session_new = r#"{"sessionId":"sess-1","configOptions":[{"configId":"model","category":"model","currentValue":"model-a","options":[{"value":"model-a"},{"value":"model-b"}]}],"models":{"currentModelId":"model-a","availableModels":[{"modelId":"model-a"},{"modelId":"model-b"}]}}"#;
        let switch_reply = r#""result":{"configOptions":[{"configId":"model","category":"model","currentValue":"model-b","options":[{"value":"model-a"},{"value":"model-b"}]}],"models":{"currentModelId":"model-b","availableModels":[{"modelId":"model-a"},{"modelId":"model-b"}]}}"#;
        let acp = spawn_switch_acp_full(session_new, switch_reply).await;
        let mut agent = switching_agent(acp, "model-b");
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("session creation must succeed");

        let cap = capture(&obs);
        assert_eq!(
            cap["models"]["currentModelId"], "model-b",
            "an applied switch must cache the target model, not the pre-switch model-a"
        );
    }

    /// F3: an applied switch whose target response omits `models` must cache
    /// Null — never fall back to the pre-switch `resp.raw.models`. Otherwise the
    /// panel would report the pre-switch model as live after a successful switch.
    #[tokio::test]
    async fn test_applied_switch_without_models_does_not_leak_pre_switch_model() {
        // session/new advertises model-a as current; the successful switch reply
        // echoes configOptions (so the switch is Applied) but NO models block.
        let session_new = r#"{"sessionId":"sess-1","configOptions":[{"configId":"model","category":"model","currentValue":"model-a","options":[{"value":"model-a"},{"value":"model-b"}]}],"models":{"currentModelId":"model-a","availableModels":[{"modelId":"model-a"},{"modelId":"model-b"}]}}"#;
        let switch_reply = r#""result":{"configOptions":[{"configId":"model","category":"model","currentValue":"model-b","options":[{"value":"model-a"},{"value":"model-b"}]}]}"#;
        let acp = spawn_switch_acp_full(session_new, switch_reply).await;
        let mut agent = switching_agent(acp, "model-b");
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("session creation must succeed");

        let cap = capture(&obs);
        assert!(
            cap["models"].is_null(),
            "an optionless-models successful switch must emit Null, not the pre-switch models"
        );
    }

    /// Like `switching_agent` but also holds a spawn-scoped startup effort, so a
    /// single session creation both switches the model AND applies startup
    /// effort — the interaction F5.6 pins.
    fn switching_agent_with_effort(
        acp: AcpClient,
        desired_model: &str,
        startup_effort: &str,
    ) -> OwnedAgent {
        OwnedAgent {
            index: 0,
            acp,
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: Some(desired_model.to_string()),
            model_overridden: true,
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: Some(startup_effort.to_string()),
            agent_name: "switch-effort-test-agent".into(),
            goose_system_prompt_supported: None,
            protocol_version: 2,
        }
    }

    fn effort_option_current_value(cap: &serde_json::Value) -> Option<String> {
        cap["configOptions"]
            .as_array()?
            .iter()
            .find(|o| o["category"] == "thought_level")
            .and_then(|o| o["currentValue"].as_str())
            .map(str::to_string)
    }

    /// F5.6: startup effort resolves against the TARGET model's option set. The
    /// pre-switch model-a advertises no `thought_level`; only the post-switch
    /// model-b does. `apply_startup_effort` reads the post-switch snapshot, so
    /// the held `high` applies against model-b's option and the cached
    /// configOptions show it at `high`. Had it read the pre-switch snapshot the
    /// effort would find no option and silently no-op.
    #[tokio::test]
    async fn test_startup_effort_resolves_against_post_switch_target_options() {
        // session/new: model-a, model option only — NO thought_level.
        let session_new = r#"[{"configId":"model","category":"model","currentValue":"model-a","options":[{"value":"model-a"},{"value":"model-b"}]}]"#;
        // switch reply: model-b current AND a target-only thought_level option.
        let switch_reply = r#""result":{"configOptions":[{"configId":"model","category":"model","currentValue":"model-b","options":[{"value":"model-a"},{"value":"model-b"}]},{"configId":"effort","category":"thought_level","currentValue":"low","options":[{"value":"low"},{"value":"high"}]}]}"#;
        let acp = spawn_switch_acp(session_new, switch_reply).await;
        let mut agent = switching_agent_with_effort(acp, "model-b", "high");
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("session creation must succeed");

        let cap = capture(&obs);
        assert_eq!(
            effort_option_current_value(&cap).as_deref(),
            Some("high"),
            "startup effort must apply against the target model's thought_level option"
        );
    }

    /// F5.6: an applied switch whose target response echoes NO options must not
    /// apply the held startup effort against the STALE pre-switch options. The
    /// pre-switch model-a advertised a `thought_level` option; the optionless
    /// target response means the effort has no target option and must be
    /// skipped — so the cached configOptions are Null, never the pre-switch
    /// model-a options with a falsely patched `high`.
    #[tokio::test]
    async fn test_startup_effort_skips_stale_options_on_optionless_switch() {
        // session/new: model-a WITH a thought_level option.
        let session_new = r#"[{"configId":"model","category":"model","currentValue":"model-a","options":[{"value":"model-a"},{"value":"model-b"}]},{"configId":"effort","category":"thought_level","currentValue":"low","options":[{"value":"low"},{"value":"high"}]}]"#;
        // switch reply: applied, but NO echoed options.
        let switch_reply = r#""result":{"ok":true}"#;
        let acp = spawn_switch_acp(session_new, switch_reply).await;
        let mut agent = switching_agent_with_effort(acp, "model-b", "high");
        let obs = observer::ObserverHandle::in_process();
        agent.acp.set_observer(Some(obs.clone()), 0);

        let ctx = make_prompt_context_no_owner();
        create_session_and_apply_model(
            &mut agent,
            &ctx,
            None,
            NewSessionChannelContext {
                huddle_instructions: None,
                canvas: None,
                name: None,
                id: None,
                channel_type: None,
            },
        )
        .await
        .expect("session creation must succeed");

        let cap = capture(&obs);
        assert_eq!(
            cap["modelOverridden"], true,
            "the switch still applied even with no echoed options"
        );
        assert!(
            cap["configOptions"].is_null(),
            "an optionless switch caches the target's (empty) options, never the pre-switch model-a options with a patched effort"
        );
    }
}
