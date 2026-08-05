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
use crate::scope::ConversationScope;

/// Window within which agent activity before a hard-cap death qualifies
/// the turn as "recently active" (eligible for requeue instead of dead-letter).
const RECENT_ACTIVITY_WINDOW: Duration = Duration::from_secs(60);

// FlushBatch and BatchEvent derive Clone (added in queue.rs) so we can store
// a recoverable copy in TaskMeta for panic recovery in Queue mode.

/// Metadata stored per in-flight task for panic recovery.
pub struct TaskMeta {
    pub agent_index: usize,
    /// Conversation scope of the in-flight prompt. `None` for heartbeats.
    pub scope: Option<ConversationScope>,
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
#[derive(Clone, Debug)]
#[allow(dead_code)] // Scaffolding for desktop integration — fields read via serde.
pub struct AgentModelCapabilities {
    /// Stable: configOptions with category "model" from session/new.
    pub config_options_raw: Vec<serde_json::Value>,
    /// Unstable: SessionModelState from session/new.
    pub available_models_raw: Option<serde_json::Value>,
}

/// Per-conversation session IDs and turn counters.
///
/// Keyed by [`ConversationScope`]: unthreaded channel traffic and DMs use the
/// channel-level scope; thread replies in regular channels get their own
/// scope (and therefore their own session, turn counter, and cached prompt
/// sections).
///
/// Separated from `OwnedAgent` so the state machine is testable without
/// spawning a real agent subprocess.
#[derive(Default)]
pub struct SessionState {
    /// conversation scope → session_id
    pub sessions: HashMap<ConversationScope, String>,
    pub heartbeat_session: Option<String>,
    /// Per-conversation turn counters for proactive session rotation.
    /// Incremented on each successful prompt; reset when the session is rotated.
    pub turn_counts: HashMap<ConversationScope, u32>,
    /// Turn counter for the heartbeat session.
    pub heartbeat_turn_count: u32,
    /// conversation scope → rendered NIP-AE core prompt section, populated
    /// once at session creation per Tyler's spec (no mid-session refresh).
    pub core_sections: HashMap<ConversationScope, String>,
    /// conversation scope → rendered `[Channel Canvas]` metadata section.
    ///
    /// Populated once before session creation (same lifecycle as `core_sections`).
    /// Absent when the channel has no canvas, the canvas content is blank, or the
    /// fetch fails — all fail open. Cleared on session invalidation alongside
    /// `core_sections` so the next session picks up any canvas change.
    pub canvas_sections: HashMap<ConversationScope, String>,
}

impl SessionState {
    /// Invalidate the session (and turn counter) for a specific prompt source.
    pub fn invalidate(&mut self, source: &PromptSource) {
        match source {
            PromptSource::Conversation(scope) => {
                self.invalidate_scope(scope);
            }
            PromptSource::Heartbeat => {
                self.heartbeat_session = None;
                self.heartbeat_turn_count = 0;
            }
        }
    }

    /// Invalidate a single conversation's session and turn counter.
    /// Returns `true` if the conversation had an active session.
    pub fn invalidate_scope(&mut self, scope: &ConversationScope) -> bool {
        self.turn_counts.remove(scope);
        self.core_sections.remove(scope);
        self.canvas_sections.remove(scope);
        self.sessions.remove(scope).is_some()
    }

    /// Invalidate every conversation under `channel_id` — the channel-level
    /// conversation and all of its thread conversations. Returns `true` if at
    /// least one active session was removed.
    ///
    /// Used for channel-wide teardown (membership removal); scope-targeted
    /// rotation goes through [`invalidate_scope`](Self::invalidate_scope).
    pub fn invalidate_channel(&mut self, channel_id: &Uuid) -> bool {
        self.turn_counts.retain(|s, _| s.channel_id != *channel_id);
        self.core_sections
            .retain(|s, _| s.channel_id != *channel_id);
        self.canvas_sections
            .retain(|s, _| s.channel_id != *channel_id);
        let before = self.sessions.len();
        self.sessions.retain(|s, _| s.channel_id != *channel_id);
        self.sessions.len() != before
    }

    /// Invalidate all sessions and turn counters (e.g. after agent exit).
    pub fn invalidate_all(&mut self) {
        self.sessions.clear();
        self.turn_counts.clear();
        self.heartbeat_session = None;
        self.heartbeat_turn_count = 0;
        self.core_sections.clear();
        self.canvas_sections.clear();
    }

    #[cfg(test)]
    fn has_scope_state(&self, scope: &ConversationScope) -> bool {
        self.sessions.contains_key(scope)
            || self.turn_counts.contains_key(scope)
            || self.core_sections.contains_key(scope)
            || self.canvas_sections.contains_key(scope)
    }
}

/// An agent with its session state, owned by the pool or a running task.
pub struct OwnedAgent {
    pub index: usize,
    pub acp: AcpClient,
    pub state: SessionState,
    /// Model catalog from first session/new. None until first session created.
    pub model_capabilities: Option<AgentModelCapabilities>,
    /// Configured/default model for this process. This is immutable for the
    /// lifetime of the agent and is the fallback whenever a claimed channel
    /// has no runtime override.
    pub baseline_model: Option<String>,
    /// Effective model for the currently claimed conversation (or heartbeat).
    /// Re-resolved from the channel override or [`baseline_model`](Self::baseline_model)
    /// on every claim before a session can be created.
    pub desired_model: Option<String>,
    /// Generation of the channel override in [`desired_model`](Self::desired_model).
    /// `None` means the configured baseline is effective.
    pub desired_model_generation: Option<u64>,
    /// Whether the effective model came from a live channel override rather
    /// than the configured baseline. Reset whenever the agent becomes idle or
    /// is claimed for a channel without an override.
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

    fn use_baseline_model(&mut self) {
        self.desired_model.clone_from(&self.baseline_model);
        self.desired_model_generation = None;
        self.model_overridden = false;
    }

    fn use_channel_model(&mut self, desired_model: Option<&ChannelDesiredModel>) {
        match desired_model {
            Some(desired) => {
                self.desired_model = Some(desired.model_id.clone());
                self.desired_model_generation = Some(desired.generation);
                self.model_overridden = true;
            }
            None => self.use_baseline_model(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChannelDesiredModel {
    model_id: String,
    generation: u64,
}

#[derive(Debug, Clone)]
struct PendingModelControl {
    model_id: String,
    generation: u64,
    request_id: Option<String>,
}

#[derive(Default)]
struct ChannelModelControlState {
    catalog: Option<AgentModelCapabilities>,
    desired_models: HashMap<Uuid, ChannelDesiredModel>,
    /// Latest state transition for each channel, including a baseline
    /// tombstone created when a once-supported override fails against a fresh
    /// session catalog. Pool slots reconcile this generation even when no
    /// desired model remains, so sessions opened under the failed override
    /// cannot survive on another holder.
    channel_generations: HashMap<Uuid, u64>,
    pending_controls: HashMap<Uuid, PendingModelControl>,
    next_generation: u64,
}

/// Process-lifetime authority for channel model controls.
///
/// This state deliberately lives outside [`AgentPool`]. Agents move out of the
/// pool while turns run, and a lazy startup replaces the placeholder pool
/// wholesale. Keeping the catalog, desired models, and pending generations in
/// one shared coordinator makes both lifecycle transitions lossless.
#[derive(Clone, Default)]
pub struct ChannelModelControls {
    inner: Arc<Mutex<ChannelModelControlState>>,
}

/// One authoritative result for a model-control request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelControlResult {
    pub channel_id: Uuid,
    pub model_id: String,
    pub generation: u64,
    pub request_id: Option<String>,
    pub status: &'static str,
}

/// Resolution of a newly received model-control request.
#[derive(Debug, PartialEq, Eq)]
pub enum ModelControlDecision {
    /// The cached catalog accepts the model. The desired state is live.
    Accepted {
        generation: u64,
        superseded: Vec<ModelControlResult>,
    },
    /// The cached catalog rejects the model. Desired state is unchanged.
    Unsupported {
        generation: u64,
        superseded: Vec<ModelControlResult>,
    },
    /// No session has exposed a catalog yet. The request is retained until
    /// the first `session/new`; at most one unresolved request is kept per
    /// channel, and any older request is resolved as `superseded`.
    PendingCatalog {
        generation: u64,
        superseded: Vec<ModelControlResult>,
    },
}

impl ModelControlDecision {
    pub fn generation(&self) -> u64 {
        match self {
            Self::Accepted { generation, .. }
            | Self::Unsupported { generation, .. }
            | Self::PendingCatalog { generation, .. } => *generation,
        }
    }

    pub fn superseded(&self) -> &[ModelControlResult] {
        match self {
            Self::Accepted { superseded, .. }
            | Self::Unsupported { superseded, .. }
            | Self::PendingCatalog { superseded, .. } => superseded,
        }
    }
}

impl ChannelModelControls {
    fn lock(&self) -> std::sync::MutexGuard<'_, ChannelModelControlState> {
        match self.inner.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                tracing::warn!(target: "pool::model", "channel model control lock poisoned");
                poisoned.into_inner()
            }
        }
    }

    fn next_generation(state: &mut ChannelModelControlState) -> u64 {
        let mut generation = state.next_generation.wrapping_add(1);
        if generation == 0 {
            generation = 1;
        }
        state.next_generation = generation;
        generation
    }

    /// Record a channel model request against the process-wide catalog.
    pub fn request(
        &self,
        channel_id: Uuid,
        model_id: &str,
        request_id: Option<&str>,
    ) -> ModelControlDecision {
        let mut state = self.lock();
        let generation = Self::next_generation(&mut state);
        let superseded = state
            .pending_controls
            .remove(&channel_id)
            .map(|pending| ModelControlResult {
                channel_id,
                model_id: pending.model_id,
                generation: pending.generation,
                request_id: pending.request_id,
                status: "superseded",
            })
            .into_iter()
            .collect();

        let Some(catalog) = state.catalog.as_ref() else {
            state.pending_controls.insert(
                channel_id,
                PendingModelControl {
                    model_id: model_id.to_string(),
                    generation,
                    request_id: request_id.map(ToOwned::to_owned),
                },
            );
            return ModelControlDecision::PendingCatalog {
                generation,
                superseded,
            };
        };

        if !model_in_catalog(
            &catalog.config_options_raw,
            catalog.available_models_raw.as_ref(),
            model_id,
        ) {
            return ModelControlDecision::Unsupported {
                generation,
                superseded,
            };
        }

        state.desired_models.insert(
            channel_id,
            ChannelDesiredModel {
                model_id: model_id.to_string(),
                generation,
            },
        );
        state.channel_generations.insert(channel_id, generation);
        ModelControlDecision::Accepted {
            generation,
            superseded,
        }
    }

    /// Cache the first authoritative session catalog and resolve every lazy
    /// pre-wake request exactly once.
    pub fn register_catalog(
        &self,
        capabilities: AgentModelCapabilities,
    ) -> Vec<ModelControlResult> {
        let mut state = self.lock();
        if state.catalog.is_some() {
            return Vec::new();
        }
        state.catalog = Some(capabilities);
        let pending = std::mem::take(&mut state.pending_controls);
        let Some(catalog) = state.catalog.clone() else {
            return Vec::new();
        };

        let mut results: Vec<ModelControlResult> = pending
            .into_iter()
            .map(|(channel_id, pending)| {
                let supported = model_in_catalog(
                    &catalog.config_options_raw,
                    catalog.available_models_raw.as_ref(),
                    &pending.model_id,
                );
                if supported {
                    state.desired_models.insert(
                        channel_id,
                        ChannelDesiredModel {
                            model_id: pending.model_id.clone(),
                            generation: pending.generation,
                        },
                    );
                    state
                        .channel_generations
                        .insert(channel_id, pending.generation);
                }
                ModelControlResult {
                    channel_id,
                    model_id: pending.model_id,
                    generation: pending.generation,
                    request_id: pending.request_id,
                    status: if supported {
                        "sent"
                    } else {
                        "unsupported_model"
                    },
                }
            })
            .collect();
        results.sort_by_key(|result| result.generation);
        results
    }

    fn desired_model(&self, channel_id: Uuid) -> Option<ChannelDesiredModel> {
        self.lock().desired_models.get(&channel_id).cloned()
    }

    fn channel_generations(&self) -> Vec<(Uuid, u64)> {
        self.lock()
            .channel_generations
            .iter()
            .map(|(channel_id, generation)| (*channel_id, *generation))
            .collect()
    }

    /// Clear a failed override only if the failed session was applying the
    /// channel's current generation. A superseded session must never erase a
    /// newer control. The returned tombstone generation makes every other
    /// holder invalidate any session it may already have opened under the
    /// failed override.
    fn clear_if_current(&self, channel_id: Uuid, generation: u64) -> Option<u64> {
        let mut state = self.lock();
        if state
            .desired_models
            .get(&channel_id)
            .is_some_and(|desired| desired.generation == generation)
        {
            state.desired_models.remove(&channel_id);
            let tombstone_generation = Self::next_generation(&mut state);
            state
                .channel_generations
                .insert(channel_id, tombstone_generation);
            Some(tombstone_generation)
        } else {
            None
        }
    }

    fn remove_channel(&self, channel_id: Uuid) {
        let mut state = self.lock();
        state.desired_models.remove(&channel_id);
        state.channel_generations.remove(&channel_id);
        state.pending_controls.remove(&channel_id);
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.lock().pending_controls.len()
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
    /// Stable conversation → agent-slot ownership. A scope's ACP session
    /// history lives inside exactly one agent process, so once a scope is
    /// dispatched to a slot, later turns for that scope must go back to the
    /// SAME slot — even if that means waiting while it is checked out on
    /// other work. Without this, a busy owner would let `try_claim` hand the
    /// scope to another idle agent, silently forking the conversation into a
    /// second divergent session.
    ///
    /// Lifecycle: recorded on claim; released when the owning agent returns
    /// without a live session for the scope (rotation, invalidation, failed
    /// create), when the scope/channel is explicitly invalidated, or when the
    /// owning slot dies (respawn/panic — the process and its sessions are
    /// gone, so re-owning elsewhere creates a fresh history, not a fork).
    scope_owners: HashMap<ConversationScope, usize>,
    /// Shared process-lifetime model authority. This survives checked-out
    /// agents and lazy placeholder-pool replacement.
    model_controls: ChannelModelControls,
    /// Last channel-model generation reconciled into each agent slot. A
    /// switched channel may have cached sessions inside a checked-out agent;
    /// comparing generations when it returns guarantees those stale sessions
    /// are invalidated before the slot is reused.
    slot_channel_model_generations: HashMap<(usize, Uuid), u64>,
    /// Channel invalidations deferred for slots that were checked out when
    /// membership was removed. This is slot-local rather than a global
    /// "removed channel" flag because the channel may be rejoined before an
    /// old holder returns; that old process must still discard its pre-removal
    /// sessions without invalidating sessions created after the rejoin on
    /// other slots.
    pending_channel_invalidations: HashMap<usize, HashSet<Uuid>>,
    /// Conversation rotations deferred for a checked-out owning slot. A
    /// `!rotate` may target a scope whose owner is busy on another scope, or
    /// whose current task has already consumed its one-shot control signal.
    /// Ownership can be released immediately so future work starts fresh, but
    /// the old holder must still discard its historical session on return.
    pending_scope_invalidations: HashMap<usize, HashSet<ConversationScope>>,
    /// Checked-out task slots whose current source batch was explicitly
    /// invalidated. Channel removal marks every active task under the channel;
    /// a late `!rotate` marks only the task running that exact scope. Keeping
    /// this per task slot lets a rapid channel rejoin accept fresh work without
    /// reviving an old batch.
    discard_result_batches: HashSet<usize>,
}

/// Outcome of [`AgentPool::try_claim`] for a conversation-scoped claim.
pub enum ClaimOutcome {
    /// An agent was claimed for the conversation (ownership recorded).
    Claimed(Box<OwnedAgent>),
    /// The conversation's owning agent is checked out on other work. The
    /// caller must leave the conversation queued and retry after the owner
    /// returns — claiming a different agent would fork the session history.
    OwnerBusy,
    /// No idle agent is available at all.
    NoIdleAgent,
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

/// Whether the prompt came from a conversation (channel or thread) event or a
/// heartbeat.
#[derive(Debug)]
pub enum PromptSource {
    Conversation(ConversationScope),
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
    // session. A SwitchModel re-resolves the shared desired generation during
    // that new session's creation.
    if matches!(
        control_signal,
        ControlSignal::Rotate | ControlSignal::SwitchModel { .. }
    ) {
        state.invalidate(source);
    }
}

/// Control signal for an in-flight channel turn.
///
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
    /// Requeue the triggering batch so it re-runs on a fresh session under
    /// the process-lifetime desired-model generation. The task never carries
    /// or writes model state itself; it re-resolves the coordinator after
    /// `session/new`, preventing a stale signal from overwriting a newer pick.
    SwitchModel { generation: u64 },
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
                    },
                ))
            })
            .collect();
        Self {
            cache: std::sync::Arc::new(std::sync::RwLock::new(cache)),
            rest_client,
        }
    }

    /// Return cached metadata without performing relay I/O.
    ///
    /// Event scope derivation uses this after startup discovery or the
    /// dynamic-subscription metadata gate has populated the resolver.
    pub(crate) fn cached(&self, channel_id: Uuid) -> Option<PromptChannelInfo> {
        let cache = self
            .cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.get(&channel_id).cloned()
    }

    pub async fn resolve(&self, channel_id: Uuid) -> Option<PromptChannelInfo> {
        if let Some(info) = self.cached(channel_id) {
            return Some(info);
        }

        let info = fetch_channel_info(channel_id, &self.rest_client).await?;
        let mut cache = self
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.entry(channel_id).or_insert_with(|| info.clone());
        Some(info)
    }

    #[cfg(test)]
    pub(crate) fn insert_for_test(&self, channel_id: Uuid, info: PromptChannelInfo) {
        let mut cache = self
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.insert(channel_id, info);
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
    /// Process-lifetime channel model authority shared with the agent pool.
    pub model_controls: ChannelModelControls,
}

impl AgentPool {
    /// Create a pool from pre-indexed slots (may contain None for failed startups).
    ///
    /// Slot positions are preserved so that `agent.index` always matches the
    /// index into `self.agents`. Use this instead of `new()` when the startup
    /// loop skips failed agents — `new()` would pack agents densely and break
    /// the index invariant.
    #[cfg(test)]
    pub fn from_slots(slots: Vec<Option<OwnedAgent>>) -> Self {
        Self::from_slots_with_model_controls(slots, ChannelModelControls::default())
    }

    /// Create a pool backed by an existing process-lifetime model authority.
    ///
    /// The main harness uses this for both the lazy placeholder and the
    /// awakened pool. Tests that do not exercise lifecycle replacement can use
    /// [`from_slots`](Self::from_slots).
    pub fn from_slots_with_model_controls(
        slots: Vec<Option<OwnedAgent>>,
        model_controls: ChannelModelControls,
    ) -> Self {
        for capabilities in slots
            .iter()
            .flatten()
            .filter_map(|agent| agent.model_capabilities.clone())
        {
            let _ = model_controls.register_catalog(capabilities);
        }
        let (result_tx, result_rx) = mpsc::unbounded_channel();
        Self {
            agents: slots,
            result_tx,
            result_rx,
            join_set: JoinSet::new(),
            task_map: HashMap::new(),
            scope_owners: HashMap::new(),
            model_controls,
            slot_channel_model_generations: HashMap::new(),
            pending_channel_invalidations: HashMap::new(),
            pending_scope_invalidations: HashMap::new(),
            discard_result_batches: HashSet::new(),
        }
    }

    /// Apply channel removals that occurred while `agent` was checked out.
    fn reconcile_pending_channel_invalidations(&mut self, agent: &mut OwnedAgent) {
        let Some(channel_ids) = self.pending_channel_invalidations.remove(&agent.index) else {
            return;
        };
        for channel_id in channel_ids {
            agent.state.invalidate_channel(&channel_id);
        }
    }

    /// Apply conversation rotations that occurred while `agent` was checked
    /// out, without disturbing sibling scopes.
    fn reconcile_pending_scope_invalidations(&mut self, agent: &mut OwnedAgent) {
        let Some(scopes) = self.pending_scope_invalidations.remove(&agent.index) else {
            return;
        };
        for scope in scopes {
            agent.state.invalidate_scope(&scope);
        }
    }

    /// Reconcile channel-model switches that occurred while `agent` was
    /// checked out. Generations include baseline tombstones created when a
    /// fresh session rejects the cached override, so sessions under a failed
    /// model are invalidated across every holder. Unrelated channels remain
    /// intact.
    fn reconcile_channel_model_generations(&mut self, agent: &mut OwnedAgent) {
        for (channel_id, generation) in self.model_controls.channel_generations() {
            let key = (agent.index, channel_id);
            if self.slot_channel_model_generations.get(&key).copied() != Some(generation) {
                agent.state.invalidate_channel(&channel_id);
                self.slot_channel_model_generations.insert(key, generation);
            }
        }
    }

    /// Drop ownership entries whose sessions were removed while reconciling
    /// an idle/returning slot.
    fn release_missing_ownerships(&mut self, agent: &OwnedAgent) {
        let idx = agent.index;
        self.scope_owners
            .retain(|scope, owner| *owner != idx || agent.state.sessions.contains_key(scope));
    }

    /// Whether the given slot is currently checked out on an in-flight task.
    fn slot_checked_out(&self, index: usize) -> bool {
        self.task_map.values().any(|m| m.agent_index == index)
    }

    /// Try to claim an agent for the given conversation.
    ///
    /// Ownership is stable: if `scope` was previously dispatched to a slot,
    /// only that slot may serve it again while its session history is alive.
    ///
    /// - Owner idle → claim the owner.
    /// - Owner checked out on other work → [`ClaimOutcome::OwnerBusy`]; the
    ///   caller must keep the scope queued until the owner returns.
    /// - Owner slot dead (neither idle nor checked out — process died and its
    ///   sessions with it) → re-own on any idle agent.
    /// - Unowned scope → any idle agent, prefering one that already holds a
    ///   session for the scope (covers ownership rebuilt from state after
    ///   explicit map cleanup).
    ///
    /// On every claim, the effective model is resolved afresh: the target
    /// channel override when present, otherwise the agent's configured
    /// baseline. Pending switch generations are reconciled first so sessions
    /// cached inside an agent that was checked out during a switch cannot be
    /// reused under the old model.
    pub fn try_claim_for_scope(&mut self, scope: ConversationScope) -> ClaimOutcome {
        let claim_idx = match self.scope_owners.get(&scope).copied() {
            Some(owner) if self.agents.get(owner).is_some_and(|s| s.is_some()) => Some(owner),
            Some(owner) if self.slot_checked_out(owner) => return ClaimOutcome::OwnerBusy,
            _ => {
                // Unowned, or owner slot is dead: prefer an idle agent that
                // already holds a session for this scope, else any idle agent.
                self.agents
                    .iter()
                    .position(|slot| {
                        slot.as_ref()
                            .is_some_and(|a| a.state.sessions.contains_key(&scope))
                    })
                    .or_else(|| self.agents.iter().position(|slot| slot.is_some()))
            }
        };
        let Some(idx) = claim_idx else {
            return ClaimOutcome::NoIdleAgent;
        };
        let Some(mut agent) = self.agents[idx].take() else {
            return ClaimOutcome::NoIdleAgent;
        };
        self.reconcile_pending_channel_invalidations(&mut agent);
        self.reconcile_pending_scope_invalidations(&mut agent);
        self.reconcile_channel_model_generations(&mut agent);
        self.release_missing_ownerships(&agent);
        self.scope_owners.insert(scope, idx);
        let channel_model = self.model_controls.desired_model(scope.channel_id);
        agent.use_channel_model(channel_model.as_ref());
        ClaimOutcome::Claimed(Box::new(agent))
    }

    /// Claim any idle agent for a heartbeat (no conversation ownership).
    pub fn try_claim_for_heartbeat(&mut self) -> Option<OwnedAgent> {
        let idx = self.agents.iter().position(|slot| slot.is_some());
        let mut agent = idx.and_then(|i| self.agents[i].take())?;
        self.reconcile_pending_channel_invalidations(&mut agent);
        self.reconcile_pending_scope_invalidations(&mut agent);
        self.reconcile_channel_model_generations(&mut agent);
        self.release_missing_ownerships(&agent);
        agent.use_baseline_model();
        Some(agent)
    }

    /// Return an agent to its slot after a task completes.
    ///
    /// Ownership cleanup happens here: any scope owned by this slot whose
    /// session no longer exists on the agent (rotated, invalidated, or never
    /// created) is released, so the next event for that scope may be served
    /// by any agent — there is no history left to protect.
    pub fn return_agent(&mut self, mut agent: OwnedAgent) {
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
        self.reconcile_pending_channel_invalidations(&mut agent);
        self.reconcile_pending_scope_invalidations(&mut agent);
        self.reconcile_channel_model_generations(&mut agent);
        self.release_missing_ownerships(&agent);
        // An idle agent is not associated with any channel. Reset the
        // ephemeral effective selection so no caller can mistake the last
        // claimed channel's override for agent-global state.
        agent.use_baseline_model();
        self.agents[idx] = Some(agent);
    }

    /// Release every conversation ownership held by `index`.
    ///
    /// Called when the slot's agent process is being replaced (respawn after
    /// exit/timeout/transport error, or panic recovery) — its sessions died
    /// with the process, so waiting scopes must be free to re-own elsewhere.
    pub fn release_agent_ownerships(&mut self, index: usize) {
        self.scope_owners.retain(|_, owner| *owner != index);
    }

    /// Consume the invalidation marker for an agent's completed task.
    pub(crate) fn take_discard_result_batch(&mut self, index: usize) -> bool {
        self.discard_result_batches.remove(&index)
    }

    /// Whether any agent is currently idle (sitting in its slot).
    pub fn any_idle(&self) -> bool {
        self.agents.iter().any(|slot| slot.is_some())
    }

    /// Whether any idle agent already has a session for `scope`.
    /// Used to compute `affinity_hit` before calling `try_claim`.
    pub fn has_session_for(&self, scope: ConversationScope) -> bool {
        self.agents.iter().any(|slot| {
            slot.as_ref()
                .map(|a| a.state.sessions.contains_key(&scope))
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
    /// `scope`.
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
    /// for `scope` (the prompt completed between the mode-gate check
    /// and this call, or the conversation was never in flight). This is
    /// semantically a soft no-op — the caller should release any withheld
    /// event and let normal dispatch handle delivery.
    pub fn send_steer(
        &mut self,
        scope: ConversationScope,
        request: SteerRequest,
    ) -> Result<(), SteerError> {
        let meta = self
            .task_map
            .values_mut()
            .find(|m| m.scope == Some(scope))
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

    /// Remove every session under `channel_id` (channel-level and thread
    /// conversations alike) from all agents.
    ///
    /// Called when the agent is removed from a channel — stale sessions must
    /// not be reused. Idle agents are invalidated immediately. Checked-out
    /// slots are recorded in `pending_channel_invalidations` and reconciled
    /// when their process returns, even if the channel has already been
    /// rejoined by then.
    ///
    /// Returns the number of agents that had at least one session invalidated.
    pub fn invalidate_channel_sessions(&mut self, channel_id: Uuid) -> usize {
        let mut count = 0;
        let checked_out: HashSet<usize> = self
            .task_map
            .values()
            .map(|meta| meta.agent_index)
            .collect();
        self.discard_result_batches.extend(
            self.task_map
                .values()
                .filter(|meta| {
                    meta.scope
                        .is_some_and(|scope| scope.channel_id == channel_id)
                })
                .map(|meta| meta.agent_index),
        );
        for (index, slot) in self.agents.iter_mut().enumerate() {
            if let Some(agent) = slot.as_mut() {
                if agent.state.invalidate_channel(&channel_id) {
                    count += 1;
                }
            } else if checked_out.contains(&index) {
                self.pending_channel_invalidations
                    .entry(index)
                    .or_default()
                    .insert(channel_id);
            }
        }
        // Explicit channel-wide invalidation ends every conversation under
        // the channel: release ownerships so future traffic (e.g. after a
        // re-add) can be served by any agent, and drop the channel's
        // desired-model override.
        self.scope_owners
            .retain(|scope, _| scope.channel_id != channel_id);
        self.model_controls.remove_channel(channel_id);
        self.slot_channel_model_generations
            .retain(|(_, channel), _| *channel != channel_id);
        count
    }

    /// Remove the session for a single conversation `scope` from all agents,
    /// leaving sibling conversations in the same channel untouched.
    ///
    /// Used by the idle `!rotate` path so rotating one thread (or the
    /// channel-level conversation) never destroys unrelated thread sessions.
    ///
    /// Idle holders are invalidated immediately. If the sticky owner is
    /// checked out, its invalidation is deferred until return; ownership is
    /// still released now so the next queued turn starts a fresh session.
    ///
    /// Returns the number of idle sessions invalidated immediately.
    pub fn invalidate_scope_sessions(&mut self, scope: ConversationScope) -> usize {
        let mut count = 0;
        let owner = self.scope_owners.get(&scope).copied();
        for slot in &mut self.agents {
            if let Some(agent) = slot.as_mut() {
                if agent.state.invalidate_scope(&scope) {
                    count += 1;
                }
            }
        }
        if let Some(owner) = owner {
            if self.slot_checked_out(owner) {
                self.pending_scope_invalidations
                    .entry(owner)
                    .or_default()
                    .insert(scope);
                if self
                    .task_map
                    .values()
                    .any(|meta| meta.agent_index == owner && meta.scope == Some(scope))
                {
                    // The task's control one-shot may already be consumed.
                    // Rotation still owns the final word on batch fate: its
                    // triggering request must not be re-prompted.
                    self.discard_result_batches.insert(owner);
                }
            }
        }
        // The conversation's history is gone — release ownership so the next
        // event may be served by any agent with a fresh session.
        self.scope_owners.remove(&scope);
        count
    }

    /// Channel-level model switch (desktop `switch_model` control frame).
    ///
    /// Desktop model selection is **channel-level desired state**: a
    /// channel's conversations (channel-level + threads) may be spread across
    /// several agents, idle and active, so a single-agent switch would leave
    /// stale sessions behind. This method:
    ///
    /// 1. Validates `model_id` against the first cached catalog (all slots
    ///    run the same agent binary, so one catalog answers for all). No
    ///    catalog cached anywhere → defer validation to apply time, as
    ///    before.
    /// 2. Records the desired model for the channel, so any agent that later
    ///    claims a conversation under it inherits the model at claim time
    ///    (see [`try_claim_for_scope`](Self::try_claim_for_scope)).
    /// 3. Invalidates sessions under the channel on **every idle holder** and
    ///    stamps its slot with this switch generation. Checked-out holders are
    ///    reconciled when they return, including sessions for sibling scopes
    ///    that were not themselves in flight.
    ///
    /// Active (checked-out) conversations are NOT handled here — the caller
    /// signals each in-flight task under the channel with
    /// `ControlSignal::SwitchModel`, which invalidates and requeues the turn.
    ///
    /// Returns [`IdleSwitchResult`] describing what happened. The model does
    /// not take effect — and the panel does not reflect it — until an agent
    /// next runs a turn in the channel (no live session exists to re-emit
    /// `session_config_captured` from an idle agent). This lag is
    /// intentional: faking the emit would surface an override the session
    /// has not actually applied.
    pub fn set_channel_desired_model(
        &mut self,
        channel_id: Uuid,
        model_id: &str,
        request_id: Option<&str>,
    ) -> (ModelControlDecision, IdleSwitchResult) {
        let decision = self
            .model_controls
            .request(channel_id, model_id, request_id);
        if matches!(decision, ModelControlDecision::Unsupported { .. }) {
            return (decision, IdleSwitchResult::UnsupportedModel);
        }
        if matches!(decision, ModelControlDecision::PendingCatalog { .. }) {
            return (decision, IdleSwitchResult::PendingCatalog);
        }
        let generation = decision.generation();

        let mut applied = 0;
        let mut reconciled_idle_slots = HashSet::new();
        for agent in self.agents.iter_mut().flatten() {
            reconciled_idle_slots.insert(agent.index);
            if agent.state.invalidate_channel(&channel_id) {
                applied += 1;
            }
            self.slot_channel_model_generations
                .insert((agent.index, channel_id), generation);
        }
        // Invalidation destroys the history that required sticky ownership.
        // Release ownership only for idle slots reconciled above; checked-out
        // holders retain ownership until their sessions are invalidated on
        // return (or by their active SwitchModel signal).
        self.scope_owners.retain(|scope, owner| {
            scope.channel_id != channel_id || !reconciled_idle_slots.contains(owner)
        });
        let idle_result = if applied > 0 {
            IdleSwitchResult::Switched
        } else {
            IdleSwitchResult::NoIdleAgent
        };
        (decision, idle_result)
    }

    #[cfg(test)]
    pub fn model_controls(&self) -> ChannelModelControls {
        self.model_controls.clone()
    }

    /// Test-only visibility into conversation ownership.
    #[cfg(test)]
    pub fn scope_owner(&self, scope: &ConversationScope) -> Option<usize> {
        self.scope_owners.get(scope).copied()
    }
}

/// Outcome of [`AgentPool::set_channel_desired_model`].
#[derive(Debug, PartialEq, Eq)]
pub enum IdleSwitchResult {
    /// `desired_model` set and the channel session invalidated.
    Switched,
    /// Desired model is not in the agent's cached catalog — pick rejected,
    /// session untouched.
    UnsupportedModel,
    /// No ACP session has returned a model catalog yet. No sessions or turns
    /// were disturbed; resolution is deferred until the first catalog.
    PendingCatalog,
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
async fn create_session_and_apply_model(
    agent: &mut OwnedAgent,
    ctx: &PromptContext,
    agent_core: Option<&str>,
    agent_canvas: Option<&str>,
    channel_name: Option<&str>,
    conversation_scope: Option<ConversationScope>,
    channel_type: Option<&str>,
) -> Result<String, AcpError> {
    // Build base_prompt + system_prompt + agent core + canvas metadata into a
    // single prompt. Standard protocol-v2 agents receive it in `session/new`;
    // Goose receives it through the custom request below. Legacy agents receive
    // the same content as user-message sections via `format_prompt`. Core carries
    // its own `[Agent Memory — core]` header, and canvas carries its own
    // `[Channel Canvas]` header; both are appended with a blank-line separator.
    let is_goose = agent.agent_name == "goose";
    let combined_system_prompt = with_canvas(
        with_core(
            with_team(
                framed_system_prompt(&ctx.cwd, ctx.base_prompt, ctx.system_prompt.as_deref()),
                ctx.team_instructions.as_deref(),
            ),
            agent_core,
        ),
        agent_canvas,
    );

    let session_title = ctx.session_title.as_deref().map(|agent_name| {
        let thread_root = conversation_scope
            .and_then(|scope| scope.thread_root)
            .map(|root| root.to_hex());
        compose_session_title(agent_name, channel_name, thread_root.as_deref())
    });
    let mcp_servers = mcp_servers_with_git_origin(
        &ctx.mcp_servers,
        conversation_scope.map(|scope| scope.channel_id),
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

    // Publish the first session catalog to the process-lifetime coordinator.
    // This resolves lazy pre-wake controls once, even though the placeholder
    // pool has already been replaced and this agent is checked out.
    let capabilities = AgentModelCapabilities {
        config_options_raw: extract_model_config_options(&resp.raw),
        available_models_raw: extract_model_state(&resp.raw),
    };
    if agent.model_capabilities.is_none() {
        agent.model_capabilities = Some(capabilities.clone());
    }
    for result in ctx.model_controls.register_catalog(capabilities) {
        if let Some(observer) = agent.acp.observer_handle() {
            observer.emit(
                "control_result",
                agent.acp.observer_agent_index(),
                &observer::ObserverContext {
                    channel_id: Some(result.channel_id.to_string()),
                    thread_root: None,
                    session_id: None,
                    turn_id: None,
                    started_at: None,
                },
                serde_json::json!({
                    "type": "switch_model",
                    "status": result.status,
                    "channelId": result.channel_id,
                    "modelId": result.model_id,
                    "generation": result.generation,
                    "requestId": result.request_id,
                }),
            );
        }
    }

    // A control can arrive while session/new is in flight. Resolve the latest
    // channel generation only after the response, immediately before apply.
    // Heartbeats deliberately remain on the configured baseline.
    if let Some(scope) = conversation_scope {
        let desired = ctx.model_controls.desired_model(scope.channel_id);
        agent.use_channel_model(desired.as_ref());
    } else {
        agent.use_baseline_model();
    }

    // Apply desired_model if set, matching against the fresh session/new response.
    // Track whether the switch succeeded so session_config_captured reflects
    // the post-switch state (not the pre-switch desired state).
    let desired_to_apply = agent.desired_model.clone();
    let switch_succeeded = if let Some(desired) = desired_to_apply.as_deref() {
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
                if let (Some(scope), Some(generation)) =
                    (conversation_scope, agent.desired_model_generation)
                {
                    // Catalog validation should make this unreachable unless
                    // an agent changes its catalog between sessions. Never let
                    // that stale failure erase a newer concurrent generation.
                    if ctx
                        .model_controls
                        .clear_if_current(scope.channel_id, generation)
                        .is_some()
                    {
                        agent.use_baseline_model();
                    }
                }
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

/// Prepend the `[Channel Canvas]` section to the legacy initial-message body.
///
/// Protocol-v2 agents already receive the canvas in `systemPrompt`; only
/// legacy (protocol_version < 2) agents need it injected here so it arrives
/// before the first prompt — the same "every turn" semantics as per-turn core.
/// Heartbeats never have an initial_message, so the caller is responsible for
/// not passing a canvas when `source` is `Heartbeat`.
pub(crate) fn prepend_canvas_for_legacy(
    protocol_version: u32,
    agent_canvas: Option<&str>,
    body: &str,
) -> String {
    match agent_canvas {
        Some(canvas) if protocol_version < 2 => format!("{canvas}\n\n{body}"),
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
    // Is this a conversation prompt or a heartbeat?
    let source = match &batch {
        Some(b) => PromptSource::Conversation(b.scope),
        None => PromptSource::Heartbeat,
    };
    let observer_scope = match &source {
        PromptSource::Conversation(scope) => Some(*scope),
        PromptSource::Heartbeat => None,
    };
    let observer_channel_id = observer_scope.map(|scope| scope.channel_id);
    let turn_started_at = chrono::Utc::now().to_rfc3339();
    agent
        .acp
        .set_observer_context(observer::context_for_scope_turn(
            observer_scope,
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
                PromptSource::Conversation(_) => "channel",
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
        observer_scope,
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
        observer::context_for_scope_turn(
            observer_scope,
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
    // it is identical across conversations; the per-conversation cache just
    // avoids a re-fetch on each new session and is cleared on session
    // invalidation.
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
        if let (PromptSource::Conversation(scope), Some(owner_pk)) =
            (&source, ctx.agent_owner_pubkey.as_ref())
        {
            let is_new_session = !agent.state.sessions.contains_key(scope);
            if is_new_session && !agent.state.core_sections.contains_key(scope) {
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
                            scope = %scope,
                            timeout_ms = CORE_FETCH_TIMEOUT.as_millis() as u64,
                            "core fetch timed out — emitting no section"
                        );
                        None
                    }
                };
                if let Some(rendered) = section {
                    tracing::info!(
                        target: "engram::core",
                        scope = %scope,
                        section_len = rendered.len(),
                        "injected NIP-AE core section into system prompt"
                    );
                    agent.state.core_sections.insert(*scope, rendered);
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
    let mut pending_canvas: Option<(ConversationScope, String)> = None;
    // Channel name for the session title, from the same single resolve the
    // canvas DM check uses — see `resolve_new_session_channel_context`.
    let mut title_channel: Option<String> = None;
    let mut origin_channel_type: Option<String> = None;
    if let PromptSource::Conversation(scope) = &source {
        let is_new_session = !agent.state.sessions.contains_key(scope);
        let needs_canvas = is_new_session && !agent.state.canvas_sections.contains_key(scope);
        if is_new_session {
            let cid = scope.channel_id;
            let (is_dm, resolved_channel, resolved_channel_type) =
                resolve_new_session_channel_context(&ctx.channel_info, cid).await;
            title_channel = resolved_channel;
            origin_channel_type = resolved_channel_type;
            // A confirmed DM never receives a canvas section; an undeterminable
            // channel type fails closed as a DM for the same reason.
            if needs_canvas && !is_dm {
                if let Some(section) = fetch_canvas_section(cid, &ctx.rest_client).await {
                    pending_canvas = Some((*scope, section));
                }
            }
        }
    }

    // The core section to fold into the system prompt for this turn's session.
    // Conversation-scoped; heartbeats carry no owner core.
    let agent_core: Option<String> = match &source {
        PromptSource::Conversation(scope) => agent.state.core_sections.get(scope).cloned(),
        PromptSource::Heartbeat => None,
    };

    // The canvas metadata section — conversation-scoped, absent for heartbeats/DMs.
    // Prefer the committed cache; fall back to pending (for new sessions being created now).
    let agent_canvas: Option<String> = match &source {
        PromptSource::Conversation(scope) => agent
            .state
            .canvas_sections
            .get(scope)
            .cloned()
            .or_else(|| pending_canvas.as_ref().map(|(_, s)| s.clone())),
        PromptSource::Heartbeat => None,
    };

    let (session_id, is_new_session) = match &source {
        PromptSource::Conversation(scope) => {
            if let Some(sid) = agent.state.sessions.get(scope) {
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
                    agent_canvas.as_deref(),
                    title_channel.as_deref(),
                    Some(*scope),
                    origin_channel_type.as_deref(),
                )
                .await
                {
                    Ok(sid) => {
                        tracing::info!(
                            target: "pool::session",
                            "created session {sid} for conversation {scope}"
                        );
                        agent.state.sessions.insert(*scope, sid.clone());
                        // Commit canvas only after session creation succeeds (I3).
                        if let Some((pending_scope, section)) = pending_canvas.take() {
                            agent.state.canvas_sections.insert(pending_scope, section);
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
                match create_session_and_apply_model(&mut agent, &ctx, None, None, None, None, None)
                    .await
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
    agent
        .acp
        .set_observer_context(observer::context_for_scope_turn(
            observer_scope,
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
        if let (PromptSource::Conversation(scope), Some(ref initial_msg)) =
            (&source, &ctx.initial_message)
        {
            tracing::info!(
                target: "pool::session",
                "sending initial_message to session {session_id} for conversation {scope}"
            );
            // For agents with systemPrompt support (protocol_version >= 2),
            // base_prompt is delivered via the system role in session/new.
            // Legacy agents receive it via [Base] in the user message instead.
            // Canvas is also injected here for legacy agents: protocol-v2 agents
            // already have it in systemPrompt; legacy agents need it before the
            // first prompt, matching the "every turn" per-turn delivery semantics.
            let init_msg = prepend_base_for_legacy(
                if agent.has_system_prompt_support() {
                    2
                } else {
                    1
                },
                ctx.base_prompt,
                initial_msg,
            );
            let init_msg = prepend_canvas_for_legacy(
                if agent.has_system_prompt_support() {
                    2
                } else {
                    1
                },
                agent_canvas.as_deref(),
                &init_msg,
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
                        "initial_message complete for conversation {scope}: {stop_reason:?}"
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
                        "initial_message idle timeout ({}s) for conversation {scope} — cancelling",
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
                        "hard timeout ({}s cap, silence {silence:?}, recently_active={recently_active}) during initial_message for conversation {scope} — agent process is unrecoverable",
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
                        "initial_message failed for conversation {scope}: {e} — invalidating session"
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
        // Try startup cache first; lazy-fetch via REST for dynamic channels.
        let channel_info = ctx.channel_info.resolve(b.channel_id()).await;

        let conversation_context = if ctx.context_message_limit > 0 {
            fetch_conversation_context(b, &channel_info, &ctx).await
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
                channel = %b.scope,
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
                agent_canvas: agent_canvas.as_deref(),
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
                        PromptSource::Conversation(scope) => {
                            let count = agent.state.turn_counts.entry(*scope).or_insert(0);
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

/// Resolve and cache metadata for a dynamically joined channel.
///
/// Called by the membership-add resolver before opening event delivery, so a
/// channel joined after startup gains correct DM/thread scoping on its first
/// event. Returns `true` once metadata is cached; callers retry failures while
/// the membership remains pending.
pub(crate) async fn refresh_channel_info(ctx: &PromptContext, channel_id: Uuid) -> bool {
    match ctx.channel_info.resolve(channel_id).await {
        Some(info) => {
            tracing::info!(
                channel_id = %channel_id,
                channel_type = %info.channel_type,
                "cached metadata for dynamically joined channel"
            );
            true
        }
        None => {
            tracing::warn!(
                channel_id = %channel_id,
                "could not resolve metadata for dynamically joined channel — \
                 subscription remains closed until retry succeeds"
            );
            false
        }
    }
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
                for tag in tags {
                    if let Some(arr) = tag.as_array() {
                        if arr.first().and_then(|v| v.as_str()) == Some("name") {
                            name = arr.get(1).and_then(|v| v.as_str());
                        }
                    }
                }
                let channel_type = crate::relay::channel_type_from_tags(tags);
                Some(PromptChannelInfo {
                    name: name.unwrap_or(UNKNOWN_CHANNEL_NAME).to_string(),
                    channel_type,
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

/// Fetch conversation context (thread or DM) for a batch before prompting.
///
/// Returns `None` if:
/// - The event is a plain channel message (not a thread reply, not a DM)
/// - The REST fetch fails or times out (graceful degradation)
/// - `context_message_limit` is 0
///
/// The typed batch scope is authoritative for non-DM threads. DM sessions stay
/// channel-scoped, so a DM reply falls back to the last event's NIP-10 root to
/// preserve the existing reply-hydration behavior without changing session
/// ownership.
fn context_thread_root(batch: &FlushBatch, is_dm: bool) -> Option<nostr::EventId> {
    batch.scope.thread_root.or_else(|| {
        if is_dm {
            batch
                .events
                .last()
                .and_then(|event| buzz_core::thread::parse_nip10_thread(&event.event))
                .map(|thread| thread.root_event_id)
        } else {
            None
        }
    })
}

fn context_excluded_event_ids(batch: &FlushBatch) -> HashSet<nostr::EventId> {
    batch
        .events
        .iter()
        .chain(batch.cancelled_events.iter())
        .map(|event| event.event.id)
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThreadContextType {
    Stream,
    Forum,
    Dm,
}

impl ThreadContextType {
    fn from_channel_info(channel_info: Option<&PromptChannelInfo>) -> Self {
        match channel_info.map(|info| info.channel_type.as_str()) {
            Some("forum") => Self::Forum,
            Some("dm") => Self::Dm,
            _ => Self::Stream,
        }
    }

    fn root_kinds(self) -> Vec<nostr::Kind> {
        match self {
            Self::Stream | Self::Dm => vec![
                nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE as u16),
                nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE_V2 as u16),
            ],
            Self::Forum => {
                vec![nostr::Kind::Custom(buzz_core::kind::KIND_FORUM_POST as u16)]
            }
        }
    }

    fn reply_kinds(self) -> Vec<nostr::Kind> {
        match self {
            Self::Stream | Self::Dm => vec![
                nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE as u16),
                nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE_V2 as u16),
            ],
            Self::Forum => {
                vec![nostr::Kind::Custom(
                    buzz_core::kind::KIND_FORUM_COMMENT as u16,
                )]
            }
        }
    }

    fn allows_root(self, kind: nostr::Kind) -> bool {
        let kind = kind.as_u16() as u32;
        match self {
            Self::Stream | Self::Dm => matches!(
                kind,
                buzz_core::kind::KIND_STREAM_MESSAGE | buzz_core::kind::KIND_STREAM_MESSAGE_V2
            ),
            Self::Forum => kind == buzz_core::kind::KIND_FORUM_POST,
        }
    }

    fn allows_reply(self, kind: nostr::Kind) -> bool {
        let kind = kind.as_u16() as u32;
        match self {
            Self::Stream | Self::Dm => matches!(
                kind,
                buzz_core::kind::KIND_STREAM_MESSAGE | buzz_core::kind::KIND_STREAM_MESSAGE_V2
            ),
            Self::Forum => kind == buzz_core::kind::KIND_FORUM_COMMENT,
        }
    }
}

async fn fetch_conversation_context(
    batch: &FlushBatch,
    channel_info: &Option<PromptChannelInfo>,
    ctx: &PromptContext,
) -> Option<ConversationContext> {
    let limit = ctx.context_message_limit;
    let context_type = ThreadContextType::from_channel_info(channel_info.as_ref());
    let is_dm = context_type == ThreadContextType::Dm;

    // A non-DM thread's typed scope is the single source of truth: queueing,
    // session ownership, and hydrated context must all use the same canonical
    // root. DMs are intentionally channel-scoped, so their reply context uses
    // the last event's NIP-10 root without changing session ownership.
    let excluded_event_ids = context_excluded_event_ids(batch);
    if let Some(root_id) = context_thread_root(batch, is_dm) {
        return fetch_thread_context(
            batch.channel_id(),
            root_id,
            limit,
            &excluded_event_ids,
            context_type,
            ctx.agent_keys.public_key(),
            &ctx.rest_client,
        )
        .await;
    }

    // DM non-reply: fetch recent conversation history.
    if is_dm {
        return fetch_dm_context(
            batch.channel_id(),
            limit,
            &excluded_event_ids,
            &ctx.rest_client,
        )
        .await;
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
    root_event_id: nostr::EventId,
    limit: u32,
    excluded_event_ids: &HashSet<nostr::EventId>,
    context_type: ThreadContextType,
    agent_pubkey: nostr::PublicKey,
    rest: &RestClient,
) -> Option<ConversationContext> {
    fetch_thread_context_with(
        ThreadContextFetchRequest {
            channel_id,
            root_event_id,
            limit,
            excluded_event_ids,
            context_type,
            agent_pubkey,
        },
        |filters| async move { rest.query(&filters).await },
        |filters| async move { rest.count(&filters).await },
    )
    .await
}

#[derive(Clone, Copy)]
struct ThreadContextFetchRequest<'a> {
    channel_id: Uuid,
    root_event_id: nostr::EventId,
    limit: u32,
    excluded_event_ids: &'a HashSet<nostr::EventId>,
    context_type: ThreadContextType,
    agent_pubkey: nostr::PublicKey,
}

async fn fetch_thread_context_with<Query, QueryFut, Count, CountFut>(
    request: ThreadContextFetchRequest<'_>,
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

    let ThreadContextFetchRequest {
        channel_id,
        root_event_id,
        limit,
        excluded_event_ids,
        context_type,
        agent_pubkey,
    } = request;

    let e_tag = SingleLetterTag::lowercase(Alphabet::E);
    let h_tag = SingleLetterTag::lowercase(Alphabet::H);
    let ch_str = channel_id.to_string();
    let root_hex = root_event_id.to_hex();
    // Fetch one validated sentinel beyond the display limit, plus enough room
    // for every current/cancelled source ID that the relay may return. This
    // preserves the requested historical window after exclusions and lets the
    // parser retain its truncation signal.
    let query_limit = (limit as usize)
        .saturating_add(excluded_event_ids.len())
        .saturating_add(1);

    // Three filters: (1) root event by ID, (2) recent replies with #e=root +
    // #h=channel plus a sentinel, and (3) the agent's newest reply for pinning.
    // All filters are channel- and kind-scoped. The response is still treated
    // as untrusted below: every signed event is re-checked against these
    // constraints because a compromised/misbehaving relay can ignore filters.
    let root_filter = nostr::Filter::new()
        .id(root_event_id)
        .kinds(context_type.root_kinds())
        .custom_tags(h_tag, [ch_str.as_str()]);
    let replies_filter = nostr::Filter::new()
        .kinds(context_type.reply_kinds())
        .custom_tags(e_tag, [root_hex.as_str()])
        .custom_tags(h_tag, [ch_str.as_str()])
        .limit(query_limit);
    // The newest agent-authored event may be one of the current/cancelled
    // source IDs excluded below. Fetch enough author-filtered candidates to
    // retain the preceding valid agent reply in that case.
    let agent_query_limit = excluded_event_ids.len().saturating_add(1);
    let agent_reply_filter = replies_filter
        .clone()
        .author(agent_pubkey)
        .limit(agent_query_limit);

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
            Ok(Ok(json)) => parse_nostr_thread_response_with_meta(
                json,
                channel_id,
                root_event_id,
                limit,
                excluded_event_ids,
                context_type,
                &agent_pubkey,
            ),
            Ok(Err(e)) => {
                tracing::warn!(
                    channel_id = %channel_id,
                    root = %root_event_id,
                    "thread context fetch failed: {e} — will retry"
                );
                None
            }
            Err(_) => {
                tracing::warn!(
                    channel_id = %channel_id,
                    root = %root_event_id,
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
    excluded_event_ids: &HashSet<nostr::EventId>,
    rest: &RestClient,
) -> Option<ConversationContext> {
    use nostr::{Alphabet, SingleLetterTag};

    let h_tag = SingleLetterTag::lowercase(Alphabet::H);
    let ch_str = channel_id.to_string();
    let query_limit = (limit as usize)
        .saturating_add(excluded_event_ids.len())
        .saturating_add(1);
    let filter = nostr::Filter::new()
        .kinds([
            nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE as u16),
            nostr::Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE_V2 as u16),
        ])
        .custom_tags(h_tag, [ch_str.as_str()])
        .limit(query_limit);

    fetch_with_retry(|| async {
        match timeout(
            CONTEXT_FETCH_TIMEOUT,
            rest.query(std::slice::from_ref(&filter)),
        )
        .await
        {
            Ok(Ok(json)) => parse_nostr_dm_response(json, channel_id, limit, excluded_event_ids),
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
#[cfg(test)]
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

fn context_event_channel(event: &nostr::Event) -> Option<Uuid> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.first().map(String::as_str) == Some("h"))
            .then(|| parts.get(1)?.parse::<Uuid>().ok())
            .flatten()
    })
}

fn verified_context_event(raw: &serde_json::Value, channel_id: Uuid) -> Option<nostr::Event> {
    let event = serde_json::from_value::<nostr::Event>(raw.clone()).ok()?;
    event.verify().ok()?;
    (context_event_channel(&event) == Some(channel_id)).then_some(event)
}

fn context_message_from_event(event: &nostr::Event) -> ContextMessage {
    let timestamp = chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
        .map(|value| value.to_rfc3339())
        .unwrap_or_else(|| event.created_at.as_secs().to_string());
    ContextMessage {
        pubkey: event.pubkey.to_hex(),
        timestamp,
        content: event.content.clone(),
    }
}

fn is_dm_context_kind(kind: nostr::Kind) -> bool {
    matches!(
        kind.as_u16() as u32,
        buzz_core::kind::KIND_STREAM_MESSAGE | buzz_core::kind::KIND_STREAM_MESSAGE_V2
    )
}

/// Parse an untrusted Nostr query response into thread context.
///
/// Every event must have a valid signature and ID, resolve to the requested
/// channel under the relay's first-valid-`h` rule, use an allowed message kind,
/// and either be the exact root or carry a canonical NIP-10 reference to that
/// root. Mention-only `e` tags cannot admit a cross-root event. The newest
/// `limit` validated replies are displayed chronologically; if the agent's
/// newest validated reply falls outside that window, it replaces the oldest
/// displayed reply so the next prompt retains the agent's prior turn.
#[cfg(test)]
fn parse_nostr_thread_response(
    json: serde_json::Value,
    channel_id: Uuid,
    root_event_id: nostr::EventId,
    limit: u32,
    excluded_event_ids: &HashSet<nostr::EventId>,
    context_type: ThreadContextType,
    agent_pubkey: &nostr::PublicKey,
) -> Option<ConversationContext> {
    parse_nostr_thread_response_with_meta(
        json,
        channel_id,
        root_event_id,
        limit,
        excluded_event_ids,
        context_type,
        agent_pubkey,
    )
    .map(|parsed| parsed.context)
}

struct ParsedThreadContext {
    context: ConversationContext,
    root_present: bool,
}

fn parse_nostr_thread_response_with_meta(
    json: serde_json::Value,
    channel_id: Uuid,
    root_event_id: nostr::EventId,
    limit: u32,
    excluded_event_ids: &HashSet<nostr::EventId>,
    context_type: ThreadContextType,
    agent_pubkey: &nostr::PublicKey,
) -> Option<ParsedThreadContext> {
    let events = json.as_array()?;
    let mut root_msg = None;
    let mut reply_msgs: Vec<(u64, String, bool, ContextMessage)> = Vec::new();
    let mut seen = HashSet::new();

    for raw in events {
        let Some(event) = verified_context_event(raw, channel_id) else {
            continue;
        };
        if excluded_event_ids.contains(&event.id) || !seen.insert(event.id) {
            continue;
        }
        if event.id == root_event_id {
            // A canonical root cannot itself point into another thread. Relay
            // ancestry validation enforces the same shape for accepted
            // replies; repeat it here because query responses are untrusted.
            if context_type.allows_root(event.kind)
                && buzz_core::thread::parse_nip10_thread(&event).is_none()
            {
                root_msg = Some(context_message_from_event(&event));
            }
            continue;
        }
        if !context_type.allows_reply(event.kind) {
            continue;
        }
        let Some(thread) = buzz_core::thread::parse_nip10_thread(&event) else {
            continue;
        };
        if thread.root_event_id != root_event_id {
            continue;
        }
        reply_msgs.push((
            event.created_at.as_secs(),
            event.id.to_hex(),
            event.pubkey == *agent_pubkey,
            context_message_from_event(&event),
        ));
    }

    let root_present = root_msg.is_some();
    let fetched_total = reply_msgs.len() + usize::from(root_present);
    let newest_agent_reply = reply_msgs
        .iter()
        .filter(|(_, _, is_agent, _)| *is_agent)
        .max_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)))
        .cloned();

    // Select the newest validated reply window deterministically, independent
    // of the relay's response ordering, then render oldest-first.
    reply_msgs.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    let truncated = reply_msgs.len() > limit as usize;
    if truncated {
        let keep_from = reply_msgs.len().saturating_sub(limit as usize);
        reply_msgs.drain(..keep_from);
    }

    if let Some(agent_reply) = newest_agent_reply {
        let agent_reply_already_displayed =
            reply_msgs.iter().any(|(_, id, _, _)| *id == agent_reply.1);
        if !agent_reply_already_displayed {
            if let Some(oldest) = reply_msgs.first_mut() {
                *oldest = agent_reply;
            }
        }
    }

    reply_msgs.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));

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

/// Parse an untrusted Nostr query response into channel-scoped DM context.
fn parse_nostr_dm_response(
    json: serde_json::Value,
    channel_id: Uuid,
    limit: u32,
    excluded_event_ids: &HashSet<nostr::EventId>,
) -> Option<ConversationContext> {
    let events = json.as_array()?;
    let mut seen = HashSet::new();
    let mut messages: Vec<(u64, String, ContextMessage)> = events
        .iter()
        .filter_map(|raw| verified_context_event(raw, channel_id))
        .filter(|event| {
            is_dm_context_kind(event.kind)
                && !excluded_event_ids.contains(&event.id)
                && seen.insert(event.id)
        })
        .map(|event| {
            (
                event.created_at.as_secs(),
                event.id.to_hex(),
                context_message_from_event(&event),
            )
        })
        .collect();

    messages.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    let truncated = messages.len() > limit as usize;
    if truncated {
        let keep_from = messages.len().saturating_sub(limit as usize);
        messages.drain(..keep_from);
    }
    let messages: Vec<ContextMessage> = messages
        .into_iter()
        .map(|(_, _, message)| message)
        .collect();

    if messages.is_empty() {
        return None;
    }
    let total = messages.len() + usize::from(truncated);

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
        PromptSource::Conversation(scope) => format!("conversation {scope}"),
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
    scope: Option<ConversationScope>,
    turn_id: String,
}

impl TurnCompletionGuard {
    fn new(
        observer: Option<observer::ObserverHandle>,
        agent_index: Option<usize>,
        scope: Option<ConversationScope>,
        turn_id: String,
    ) -> Self {
        Self {
            observer,
            agent_index,
            scope,
            turn_id,
        }
    }
}

impl Drop for TurnCompletionGuard {
    fn drop(&mut self) {
        if let Some(observer) = self.observer.take() {
            let context = observer::context_for_scope(self.scope, None, Some(self.turn_id.clone()));
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

    fn test_mcp_server() -> McpServer {
        McpServer {
            name: "dev".into(),
            command: "buzz-dev-mcp".into(),
            args: vec![],
            env: vec![],
        }
    }

    fn model_capabilities(models: &[&str]) -> AgentModelCapabilities {
        AgentModelCapabilities {
            config_options_raw: Vec::new(),
            available_models_raw: Some(json!({
                "availableModels": models
                    .iter()
                    .map(|model_id| json!({"modelId": model_id}))
                    .collect::<Vec<_>>()
            })),
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
    fn model_control_generations_resolve_once_and_never_erase_a_newer_override() {
        let controls = ChannelModelControls::default();
        let channel_id = Uuid::new_v4();

        let first = controls.request(channel_id, "model-a", Some("request-a"));
        assert!(matches!(
            first,
            ModelControlDecision::PendingCatalog {
                generation: 1,
                ref superseded
            } if superseded.is_empty()
        ));
        let second = controls.request(channel_id, "model-b", Some("request-b"));
        assert!(matches!(
            second,
            ModelControlDecision::PendingCatalog {
                generation: 2,
                ref superseded
            } if superseded == &[ModelControlResult {
                channel_id,
                model_id: "model-a".to_string(),
                generation: 1,
                request_id: Some("request-a".to_string()),
                status: "superseded",
            }]
        ));
        assert_eq!(
            controls.pending_len(),
            1,
            "pending state is channel-bounded"
        );

        let resolved = controls.register_catalog(model_capabilities(&["model-a", "model-b"]));
        assert_eq!(
            resolved,
            vec![ModelControlResult {
                channel_id,
                model_id: "model-b".to_string(),
                generation: 2,
                request_id: Some("request-b".to_string()),
                status: "sent",
            }]
        );
        assert!(
            controls
                .register_catalog(model_capabilities(&["model-a", "model-b"]))
                .is_empty(),
            "catalog replay must not duplicate a control result"
        );
        assert_eq!(
            controls.desired_model(channel_id),
            Some(ChannelDesiredModel {
                model_id: "model-b".to_string(),
                generation: 2,
            })
        );

        assert!(
            controls.clear_if_current(channel_id, 1).is_none(),
            "a stale generation must not clear the current override"
        );
        assert_eq!(
            controls
                .desired_model(channel_id)
                .map(|desired| desired.model_id),
            Some("model-b".to_string())
        );
        assert_eq!(
            controls.clear_if_current(channel_id, 2),
            Some(3),
            "clearing the current override publishes a baseline tombstone"
        );
        assert_eq!(controls.desired_model(channel_id), None);
        assert_eq!(
            controls.channel_generations(),
            vec![(channel_id, 3)],
            "other holders must reconcile the cleared override"
        );
    }

    #[test]
    fn lazy_placeholder_replacement_preserves_pre_wake_model_control() {
        let controls = ChannelModelControls::default();
        let channel_id = Uuid::new_v4();
        let placeholder = AgentPool::from_slots_with_model_controls(vec![None], controls.clone());

        assert!(matches!(
            placeholder
                .model_controls()
                .request(channel_id, "model-b", Some("lazy-request"),),
            ModelControlDecision::PendingCatalog { generation: 1, .. }
        ));
        drop(placeholder);

        let ready = AgentPool::from_slots_with_model_controls(vec![None], controls.clone());
        assert_eq!(
            controls.register_catalog(model_capabilities(&["model-b"])),
            vec![ModelControlResult {
                channel_id,
                model_id: "model-b".to_string(),
                generation: 1,
                request_id: Some("lazy-request".to_string()),
                status: "sent",
            }]
        );
        assert_eq!(
            ready.model_controls().desired_model(channel_id),
            Some(ChannelDesiredModel {
                model_id: "model-b".to_string(),
                generation: 1,
            }),
            "the awakened pool must see state accepted by the placeholder"
        );
    }

    #[tokio::test]
    async fn clearing_current_override_invalidates_every_holder_via_tombstone() {
        async fn inert_agent(index: usize) -> OwnedAgent {
            OwnedAgent {
                index,
                acp: AcpClient::spawn(
                    "bash",
                    &["-c".to_string(), "sleep 10".to_string()],
                    &[],
                    false,
                )
                .await
                .expect("spawn inert ACP process"),
                state: SessionState::default(),
                model_capabilities: None,
                baseline_model: Some("baseline".to_string()),
                desired_model: Some("baseline".to_string()),
                desired_model_generation: None,
                model_overridden: false,
                agent_name: "test".to_string(),
                goose_system_prompt_supported: None,
                protocol_version: 2,
            }
        }

        let controls = ChannelModelControls::default();
        let channel_id = Uuid::new_v4();
        let scope_a = ConversationScope::thread(
            channel_id,
            nostr::EventId::from_hex(&"a".repeat(64)).expect("root A"),
        );
        let scope_b = ConversationScope::thread(
            channel_id,
            nostr::EventId::from_hex(&"b".repeat(64)).expect("root B"),
        );
        assert!(controls
            .register_catalog(model_capabilities(&["model-a"]))
            .is_empty());
        assert!(matches!(
            controls.request(channel_id, "model-a", None),
            ModelControlDecision::Accepted { generation: 1, .. }
        ));

        let mut agent_a = inert_agent(0).await;
        agent_a.state.sessions.insert(scope_a, "session-a".into());
        let mut agent_b = inert_agent(1).await;
        agent_b.state.sessions.insert(scope_b, "session-b".into());
        let mut pool = AgentPool::from_slots_with_model_controls(
            vec![Some(agent_a), Some(agent_b)],
            controls.clone(),
        );

        assert_eq!(controls.clear_if_current(channel_id, 1), Some(2));

        let claimed_a = match pool.try_claim_for_scope(scope_a) {
            ClaimOutcome::Claimed(agent) => *agent,
            _ => panic!("holder A must be claimable"),
        };
        assert!(
            !claimed_a.state.sessions.contains_key(&scope_a),
            "holder A must discard its session opened under the failed override"
        );
        assert_eq!(claimed_a.desired_model.as_deref(), Some("baseline"));
        pool.return_agent(claimed_a);

        let claimed_b = match pool.try_claim_for_scope(scope_b) {
            ClaimOutcome::Claimed(agent) => *agent,
            _ => panic!("holder B must be claimable"),
        };
        assert!(
            !claimed_b.state.sessions.contains_key(&scope_b),
            "holder B must independently reconcile the baseline tombstone"
        );
        assert_eq!(claimed_b.desired_model.as_deref(), Some("baseline"));
        pool.return_agent(claimed_b);
    }

    // These pin the initial_message dispatch path (run_prompt_task, ~line 855):
    // a legacy agent WITH a base_prompt must get [Base] prepended to the user
    // message. This is the exact regression that shipped in the round-2 bug.

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

    // ── prepend_canvas_for_legacy ─────────────────────────────────────────────

    #[test]
    fn test_initial_message_legacy_agent_gets_canvas_prepended() {
        // Legacy agents (protocol_version < 2) receive the canvas section before
        // the initial-message body so it arrives before the first prompt.
        let canvas = "[Channel Canvas]\nCanvas revision (event ID): abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234\nLast modified: 2024-01-15T10:30:00Z\nFetch current content with: buzz canvas get --channel 00f1ccaf-1506-4dd7-9a0e-fa67e9e486ae";
        let composed = prepend_canvas_for_legacy(1, Some(canvas), "do the thing");
        assert!(
            composed.starts_with("[Channel Canvas]"),
            "canvas must precede the body"
        );
        assert!(
            composed.ends_with("do the thing"),
            "body must follow the canvas"
        );
        assert!(
            composed.contains("\n\ndo the thing"),
            "canvas and body separated by blank line"
        );
    }

    #[test]
    fn test_initial_message_modern_agent_omits_canvas_from_body() {
        // Protocol-v2 agents receive canvas in systemPrompt; it must NOT be
        // duplicated in the initial-message user turn.
        let canvas = "[Channel Canvas]\nsome section";
        let composed = prepend_canvas_for_legacy(2, Some(canvas), "do the thing");
        assert_eq!(
            composed, "do the thing",
            "modern agent initial message must not contain canvas"
        );
        assert!(
            !composed.contains("[Channel Canvas]"),
            "canvas must be absent from modern agent initial message"
        );
    }

    #[test]
    fn test_initial_message_legacy_agent_no_canvas_is_unchanged() {
        // No canvas present: body passes through unmodified.
        let composed = prepend_canvas_for_legacy(1, None, "do the thing");
        assert_eq!(composed, "do the thing");
    }

    #[test]
    fn test_initial_message_legacy_canvas_and_base_compose_correctly() {
        // Verify the full composition order when both base and canvas are present:
        // [Base] → canvas section → initial-message body.
        let canvas = "[Channel Canvas]\ncanvas content";
        let base_composed = prepend_base_for_legacy(1, Some("be helpful"), "do the thing");
        let full = prepend_canvas_for_legacy(1, Some(canvas), &base_composed);
        assert!(
            full.starts_with("[Channel Canvas]"),
            "canvas must be first in composed message"
        );
        assert!(
            full.contains("[Base]"),
            "base must be present in composed message"
        );
        assert!(
            full.ends_with("do the thing"),
            "body must be last in composed message"
        );
        // Order: canvas → base → body
        let canvas_pos = full.find("[Channel Canvas]").unwrap();
        let base_pos = full.find("[Base]").unwrap();
        let body_pos = full.find("do the thing").unwrap();
        assert!(
            canvas_pos < base_pos && base_pos < body_pos,
            "order must be: canvas → base → body"
        );
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
        let channel_id = Uuid::new_v4();
        let root = signed_context_event(9, channel_id, "root", 1000, vec![]);
        let newest = signed_context_event_with_keys(
            9,
            channel_id,
            "newest agent reply",
            4000,
            direct_reply_tags(root.id),
            &agent,
        );
        let middle = signed_context_event(
            9,
            channel_id,
            "middle reply",
            3000,
            direct_reply_tags(root.id),
        );
        let sentinel = signed_context_event(
            9,
            channel_id,
            "sentinel omitted reply",
            2000,
            direct_reply_tags(root.id),
        );

        let ctx = parse_nostr_thread_response(
            context_json(&[&root, &newest, &middle, &sentinel]),
            channel_id,
            root.id,
            2,
            &HashSet::new(),
            ThreadContextType::Stream,
            &agent.public_key(),
        )
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
        let channel_id = Uuid::new_v4();
        let root = signed_context_event(9, channel_id, "root", 1000, vec![]);
        let reply = signed_context_event(9, channel_id, "reply", 2000, direct_reply_tags(root.id));

        let ctx = parse_nostr_thread_response(
            context_json(&[&root, &reply]),
            channel_id,
            root.id,
            2,
            &HashSet::new(),
            ThreadContextType::Stream,
            &agent.public_key(),
        )
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
        let channel_id = Uuid::new_v4();
        let root = signed_context_event(9, channel_id, "root", 1000, vec![]);
        let newer = signed_context_event(
            9,
            channel_id,
            "newer human reply",
            5000,
            direct_reply_tags(root.id),
        );
        let middle = signed_context_event(
            9,
            channel_id,
            "middle human reply",
            4000,
            direct_reply_tags(root.id),
        );
        let oldest = signed_context_event(
            9,
            channel_id,
            "oldest displayed reply without agent pin",
            3000,
            direct_reply_tags(root.id),
        );
        let agent_reply = signed_context_event_with_keys(
            9,
            channel_id,
            "agent reply outside recent window",
            2000,
            direct_reply_tags(root.id),
            &agent,
        );

        let ctx = parse_nostr_thread_response(
            context_json(&[&root, &newer, &middle, &oldest, &agent_reply]),
            channel_id,
            root.id,
            2,
            &HashSet::new(),
            ThreadContextType::Stream,
            &agent.public_key(),
        )
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
        let channel_id = Uuid::new_v4();
        let agent_pubkey = agent.public_key();
        let (root_id, json) = stream_thread_fixture(
            channel_id,
            &agent,
            true,
            &[
                ("newest reply", 4000, false),
                ("middle reply", 3000, false),
                ("sentinel reply", 2000, false),
            ],
        );
        let excluded = HashSet::from([
            signed_context_event(9, channel_id, "current", 5000, vec![]).id,
            signed_context_event(9, channel_id, "cancelled", 4500, vec![]).id,
        ]);

        let ctx = fetch_thread_context_with(
            ThreadContextFetchRequest {
                channel_id,
                root_event_id: root_id,
                limit: 2,
                excluded_event_ids: &excluded,
                context_type: ThreadContextType::Stream,
                agent_pubkey,
            },
            move |filters| {
                assert_thread_query_filters(&filters, channel_id, root_id, agent_pubkey, 5, 3);
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
        let channel_id = Uuid::new_v4();
        let (root_id, json) = stream_thread_fixture(
            channel_id,
            &agent,
            false,
            &[
                ("newest reply", 4000, false),
                ("middle reply", 3000, false),
                ("sentinel reply", 2000, false),
            ],
        );
        let excluded = HashSet::new();

        let ctx = fetch_thread_context_with(
            ThreadContextFetchRequest {
                channel_id,
                root_event_id: root_id,
                limit: 2,
                excluded_event_ids: &excluded,
                context_type: ThreadContextType::Stream,
                agent_pubkey: agent.public_key(),
            },
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
        let channel_id = Uuid::new_v4();
        let (root_id, json) = stream_thread_fixture(
            channel_id,
            &agent,
            true,
            &[
                ("newest reply", 4000, false),
                ("middle reply", 3000, false),
                ("sentinel reply", 2000, false),
            ],
        );
        let excluded = HashSet::new();

        let ctx = fetch_thread_context_with(
            ThreadContextFetchRequest {
                channel_id,
                root_event_id: root_id,
                limit: 2,
                excluded_event_ids: &excluded,
                context_type: ThreadContextType::Stream,
                agent_pubkey: agent.public_key(),
            },
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
        let channel_id = Uuid::new_v4();
        let (root_id, json) = stream_thread_fixture(
            channel_id,
            &agent,
            true,
            &[
                ("newest reply", 4000, false),
                ("middle reply", 3000, false),
                ("sentinel reply", 2000, false),
            ],
        );
        let excluded = HashSet::new();

        let ctx = fetch_thread_context_with(
            ThreadContextFetchRequest {
                channel_id,
                root_event_id: root_id,
                limit: 2,
                excluded_event_ids: &excluded,
                context_type: ThreadContextType::Stream,
                agent_pubkey: agent.public_key(),
            },
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
        let channel_id = Uuid::new_v4();
        let root = signed_context_event(9, channel_id, "root", 1000, vec![]);
        let newer = signed_context_event(
            9,
            channel_id,
            "newer human reply",
            5000,
            direct_reply_tags(root.id),
        );
        let middle = signed_context_event(
            9,
            channel_id,
            "middle human reply",
            4000,
            direct_reply_tags(root.id),
        );
        let agent_reply = signed_context_event_with_keys(
            9,
            channel_id,
            "agent reply outside recent window",
            2000,
            direct_reply_tags(root.id),
            &agent,
        );
        // The final duplicate models the same event arriving from both the
        // recent-window and separately author-filtered relay queries.
        let json = context_json(&[&root, &newer, &middle, &agent_reply, &agent_reply]);
        let excluded = HashSet::new();

        let ctx = fetch_thread_context_with(
            ThreadContextFetchRequest {
                channel_id,
                root_event_id: root.id,
                limit: 2,
                excluded_event_ids: &excluded,
                context_type: ThreadContextType::Stream,
                agent_pubkey: agent.public_key(),
            },
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
    async fn test_fetch_thread_context_pins_previous_agent_reply_when_newest_is_excluded() {
        let agent = Keys::generate();
        let channel_id = Uuid::new_v4();
        let root = signed_context_event(9, channel_id, "root", 1000, vec![]);
        let current_agent_reply = signed_context_event_with_keys(
            9,
            channel_id,
            "current excluded agent reply",
            5000,
            direct_reply_tags(root.id),
            &agent,
        );
        let newer_human_reply = signed_context_event(
            9,
            channel_id,
            "newer human reply",
            4000,
            direct_reply_tags(root.id),
        );
        let previous_agent_reply = signed_context_event_with_keys(
            9,
            channel_id,
            "previous agent reply",
            2000,
            direct_reply_tags(root.id),
            &agent,
        );
        let json = context_json(&[
            &root,
            &current_agent_reply,
            &newer_human_reply,
            &previous_agent_reply,
        ]);
        let excluded = HashSet::from([current_agent_reply.id]);
        let agent_pubkey = agent.public_key();

        let ctx = fetch_thread_context_with(
            ThreadContextFetchRequest {
                channel_id,
                root_event_id: root.id,
                limit: 1,
                excluded_event_ids: &excluded,
                context_type: ThreadContextType::Stream,
                agent_pubkey,
            },
            move |filters| {
                assert_thread_query_filters(&filters, channel_id, root.id, agent_pubkey, 3, 2);
                std::future::ready(Ok(json.clone()))
            },
            |_filters| std::future::ready(Ok(json!({ "count": 3 }))),
        )
        .await
        .expect("thread context");

        let ConversationContext::Thread { messages, .. } = ctx else {
            panic!("expected Thread context");
        };
        assert!(messages
            .iter()
            .any(|message| message.content == "previous agent reply"));
        assert!(messages
            .iter()
            .all(|message| message.content != "current excluded agent reply"));
    }

    #[tokio::test]
    async fn test_fetch_thread_context_uses_distinct_fetched_replies_as_minimum() {
        let agent = Keys::generate();
        let channel_id = Uuid::new_v4();
        let (root_id, json) = stream_thread_fixture(
            channel_id,
            &agent,
            true,
            &[
                ("newest human reply", 5000, false),
                ("middle human reply", 4000, false),
                ("sentinel human reply", 3000, false),
                ("older distinct agent reply", 2000, true),
            ],
        );
        let excluded = HashSet::new();

        let ctx = fetch_thread_context_with(
            ThreadContextFetchRequest {
                channel_id,
                root_event_id: root_id,
                limit: 2,
                excluded_event_ids: &excluded,
                context_type: ThreadContextType::Stream,
                agent_pubkey: agent.public_key(),
            },
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
        root_id: nostr::EventId,
        agent_pubkey: nostr::PublicKey,
        reply_limit: u64,
        agent_limit: u64,
    ) {
        assert_eq!(
            filters.len(),
            3,
            "root, recent replies, and agent reply filters"
        );

        let root = serde_json::to_value(&filters[0]).expect("serialize root filter");
        assert_eq!(root.get("ids"), Some(&json!([root_id.to_hex()])));
        assert_eq!(root.get("kinds"), Some(&json!([9, 40002])));
        assert_eq!(root.get("#h"), Some(&json!([channel_id.to_string()])));
        assert!(root.get("limit").is_none());

        let replies = serde_json::to_value(&filters[1]).expect("serialize replies filter");
        assert_eq!(replies.get("kinds"), Some(&json!([9, 40002])));
        assert_eq!(replies.get("#e"), Some(&json!([root_id.to_hex()])));
        assert_eq!(replies.get("#h"), Some(&json!([channel_id.to_string()])));
        assert_eq!(replies.get("limit"), Some(&json!(reply_limit)));
        assert!(replies.get("authors").is_none());

        let agent = serde_json::to_value(&filters[2]).expect("serialize agent filter");
        assert_eq!(agent.get("kinds"), Some(&json!([9, 40002])));
        assert_eq!(agent.get("#e"), Some(&json!([root_id.to_hex()])));
        assert_eq!(agent.get("#h"), Some(&json!([channel_id.to_string()])));
        assert_eq!(agent.get("authors"), Some(&json!([agent_pubkey.to_hex()])));
        assert_eq!(agent.get("limit"), Some(&json!(agent_limit)));
    }

    fn assert_thread_count_filter(
        filters: &[nostr::Filter],
        channel_id: Uuid,
        root_id: nostr::EventId,
    ) {
        assert_eq!(filters.len(), 1, "count should query only matching replies");

        let count = serde_json::to_value(&filters[0]).expect("serialize count filter");
        assert_eq!(count.get("kinds"), Some(&json!([9, 40002])));
        assert_eq!(count.get("#e"), Some(&json!([root_id.to_hex()])));
        assert_eq!(count.get("#h"), Some(&json!([channel_id.to_string()])));
        assert_eq!(count.get("limit"), Some(&json!(0)));
        assert!(count.get("ids").is_none());
        assert!(count.get("authors").is_none());
    }

    fn stream_thread_fixture(
        channel_id: Uuid,
        agent: &Keys,
        include_root: bool,
        replies: &[(&str, u64, bool)],
    ) -> (nostr::EventId, serde_json::Value) {
        let root = signed_context_event(9, channel_id, "root", 1000, vec![]);
        let mut events = Vec::new();
        if include_root {
            events.push(serde_json::to_value(&root).expect("serialize root"));
        }
        events.extend(replies.iter().map(|(content, created_at, is_agent)| {
            let event = if *is_agent {
                signed_context_event_with_keys(
                    9,
                    channel_id,
                    content,
                    *created_at,
                    direct_reply_tags(root.id),
                    agent,
                )
            } else {
                signed_context_event(
                    9,
                    channel_id,
                    content,
                    *created_at,
                    direct_reply_tags(root.id),
                )
            };
            serde_json::to_value(event).expect("serialize reply")
        }));
        (root.id, serde_json::Value::Array(events))
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

    fn signed_context_event(
        kind: u16,
        channel_id: Uuid,
        content: &str,
        created_at: u64,
        extra_tags: Vec<Vec<String>>,
    ) -> nostr::Event {
        signed_context_event_with_keys(
            kind,
            channel_id,
            content,
            created_at,
            extra_tags,
            &Keys::generate(),
        )
    }

    fn signed_context_event_with_keys(
        kind: u16,
        channel_id: Uuid,
        content: &str,
        created_at: u64,
        extra_tags: Vec<Vec<String>>,
        keys: &Keys,
    ) -> nostr::Event {
        let mut tags =
            vec![Tag::parse(["h", channel_id.to_string().as_str()]).expect("channel tag")];
        tags.extend(
            extra_tags
                .into_iter()
                .map(|parts| Tag::parse(parts).expect("test tag")),
        );
        EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(keys)
            .expect("sign context event")
    }

    fn direct_reply_tags(root: nostr::EventId) -> Vec<Vec<String>> {
        vec![vec![
            "e".to_string(),
            root.to_hex(),
            String::new(),
            "reply".to_string(),
        ]]
    }

    fn nested_reply_tags(root: nostr::EventId, parent: nostr::EventId) -> Vec<Vec<String>> {
        vec![
            vec![
                "e".to_string(),
                root.to_hex(),
                String::new(),
                "root".to_string(),
            ],
            vec![
                "e".to_string(),
                parent.to_hex(),
                String::new(),
                "reply".to_string(),
            ],
        ]
    }

    fn context_json(events: &[&nostr::Event]) -> serde_json::Value {
        serde_json::Value::Array(
            events
                .iter()
                .map(|event| serde_json::to_value(event).expect("serialize event"))
                .collect(),
        )
    }

    #[test]
    fn nostr_thread_context_rejects_cross_root_channel_shape_and_source_duplicates() {
        let agent = Keys::generate();
        let channel_id = Uuid::new_v4();
        let wrong_channel = Uuid::new_v4();
        let root_a = signed_context_event(9, channel_id, "root-a", 10, vec![]);
        let root_b = signed_context_event(9, channel_id, "root-b", 11, vec![]);
        let history =
            signed_context_event(9, channel_id, "history-a", 20, direct_reply_tags(root_a.id));
        let trigger = signed_context_event(
            9,
            channel_id,
            "current-trigger",
            30,
            nested_reply_tags(root_a.id, history.id),
        );
        let cancelled = signed_context_event(
            9,
            channel_id,
            "cancelled-trigger",
            25,
            nested_reply_tags(root_a.id, history.id),
        );

        // This event matches a marker-agnostic #e=root-a query through its
        // mention tag, but its canonical thread is root B.
        let mut cross_root_tags = direct_reply_tags(root_b.id);
        cross_root_tags.push(vec![
            "e".to_string(),
            root_a.id.to_hex(),
            String::new(),
            "mention".to_string(),
        ]);
        let cross_root = signed_context_event(9, channel_id, "cross-root", 21, cross_root_tags);
        let wrong_channel_reply = signed_context_event(
            9,
            wrong_channel,
            "wrong-channel",
            22,
            direct_reply_tags(root_a.id),
        );
        let malformed_shape = signed_context_event(
            9,
            channel_id,
            "root-only-is-not-a-reply",
            23,
            vec![vec![
                "e".to_string(),
                root_a.id.to_hex(),
                String::new(),
                "root".to_string(),
            ]],
        );
        let wrong_kind = signed_context_event(
            buzz_core::kind::KIND_REACTION as u16,
            channel_id,
            "wrong-kind",
            24,
            direct_reply_tags(root_a.id),
        );
        let mut tampered = serde_json::to_value(&history).expect("serialize history");
        tampered["content"] = serde_json::Value::String("tampered".to_string());

        let mut values = context_json(&[
            &root_a,
            &history,
            &history,
            &trigger,
            &cancelled,
            &cross_root,
            &wrong_channel_reply,
            &malformed_shape,
            &wrong_kind,
        ])
        .as_array()
        .expect("array")
        .clone();
        values.push(tampered);
        let excluded = HashSet::from([trigger.id, cancelled.id]);

        let context = parse_nostr_thread_response(
            serde_json::Value::Array(values),
            channel_id,
            root_a.id,
            12,
            &excluded,
            ThreadContextType::Stream,
            &agent.public_key(),
        )
        .expect("root and one valid historical reply remain");
        let ConversationContext::Thread {
            messages,
            total,
            truncated,
        } = context
        else {
            panic!("expected thread context");
        };
        assert_eq!(
            messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            ["root-a", "history-a"]
        );
        assert_eq!(total, 2);
        assert!(!truncated);
        assert!(
            parse_nostr_thread_response(
                context_json(&[&cross_root]),
                channel_id,
                cross_root.id,
                12,
                &HashSet::new(),
                ThreadContextType::Stream,
                &agent.public_key(),
            )
            .is_none(),
            "an exact-ID lookup cannot promote a reply in another canonical thread to a root"
        );
    }

    #[test]
    fn forum_root_and_comment_context_use_canonical_shapes_without_repeating_trigger() {
        let channel_id = Uuid::new_v4();
        let keys = Keys::generate();
        let post = buzz_sdk::build_forum_post(channel_id, "forum post", &[], &[])
            .expect("build forum post")
            .custom_created_at(Timestamp::from(10))
            .sign_with_keys(&keys)
            .expect("sign forum post");
        let history = buzz_sdk::build_forum_comment(
            channel_id,
            "older comment",
            &buzz_sdk::ThreadRef {
                root_event_id: post.id,
                parent_event_id: post.id,
            },
            &[],
            &[],
        )
        .expect("build forum comment")
        .custom_created_at(Timestamp::from(20))
        .sign_with_keys(&keys)
        .expect("sign forum comment");
        let trigger = buzz_sdk::build_forum_comment(
            channel_id,
            "current comment",
            &buzz_sdk::ThreadRef {
                root_event_id: post.id,
                parent_event_id: history.id,
            },
            &[],
            &[],
        )
        .expect("build nested forum comment")
        .custom_created_at(Timestamp::from(30))
        .sign_with_keys(&keys)
        .expect("sign nested forum comment");
        let wrong_type = signed_context_event(
            buzz_core::kind::KIND_STREAM_MESSAGE as u16,
            channel_id,
            "stream message cannot enter forum context",
            25,
            direct_reply_tags(post.id),
        );

        let comment_context = parse_nostr_thread_response(
            context_json(&[&post, &history, &wrong_type, &trigger]),
            channel_id,
            post.id,
            12,
            &HashSet::from([trigger.id]),
            ThreadContextType::Forum,
            &keys.public_key(),
        )
        .expect("forum history");
        let ConversationContext::Thread { messages, .. } = comment_context else {
            panic!("expected thread context");
        };
        assert_eq!(
            messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            ["forum post", "older comment"]
        );

        assert!(
            parse_nostr_thread_response(
                context_json(&[&post]),
                channel_id,
                post.id,
                12,
                &HashSet::from([post.id]),
                ThreadContextType::Forum,
                &keys.public_key(),
            )
            .is_none(),
            "a forum-root trigger must not repeat itself as hydrated context"
        );
    }

    #[test]
    fn dm_reply_context_is_validated_and_current_reply_is_excluded() {
        let agent = Keys::generate();
        let channel_id = Uuid::new_v4();
        let root = signed_context_event(9, channel_id, "dm root", 10, vec![]);
        let history =
            signed_context_event(9, channel_id, "dm history", 20, direct_reply_tags(root.id));
        let trigger = signed_context_event(
            9,
            channel_id,
            "dm current",
            30,
            nested_reply_tags(root.id, history.id),
        );
        let forum_root = signed_context_event(
            buzz_core::kind::KIND_FORUM_POST as u16,
            channel_id,
            "not a DM root",
            11,
            vec![],
        );
        let forum_comment = signed_context_event(
            buzz_core::kind::KIND_FORUM_COMMENT as u16,
            channel_id,
            "not a DM reply",
            21,
            direct_reply_tags(root.id),
        );

        let context = parse_nostr_thread_response(
            context_json(&[&trigger, &forum_comment, &history, &root]),
            channel_id,
            root.id,
            12,
            &HashSet::from([trigger.id]),
            ThreadContextType::Dm,
            &agent.public_key(),
        )
        .expect("validated DM reply history");
        let ConversationContext::Thread { messages, .. } = context else {
            panic!("expected thread context");
        };
        assert_eq!(
            messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            ["dm root", "dm history"]
        );
        assert!(
            parse_nostr_thread_response(
                context_json(&[&forum_root]),
                channel_id,
                forum_root.id,
                12,
                &HashSet::new(),
                ThreadContextType::Dm,
                &agent.public_key(),
            )
            .is_none(),
            "DM hydration must reject a forum root even when the relay returns it by exact ID"
        );
    }

    #[test]
    fn dm_context_preserves_chronological_limit_after_batch_exclusions() {
        let channel_id = Uuid::new_v4();
        let oldest = signed_context_event(9, channel_id, "oldest", 10, vec![]);
        let kept_a = signed_context_event(9, channel_id, "kept-a", 20, vec![]);
        let kept_b = signed_context_event(40002, channel_id, "kept-b", 30, vec![]);
        let cancelled = signed_context_event(9, channel_id, "cancelled", 40, vec![]);
        let trigger = signed_context_event(9, channel_id, "trigger", 50, vec![]);

        let context = parse_nostr_dm_response(
            context_json(&[&trigger, &cancelled, &kept_b, &kept_a, &oldest]),
            channel_id,
            2,
            &HashSet::from([trigger.id, cancelled.id]),
        )
        .expect("DM history");
        let ConversationContext::Dm {
            messages,
            total,
            truncated,
        } = context
        else {
            panic!("expected DM context");
        };
        assert_eq!(
            messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>(),
            ["kept-a", "kept-b"]
        );
        assert!(truncated);
        assert_eq!(total, 3);
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
            scope: ConversationScope::channel(Uuid::new_v4()),
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

    fn make_state() -> (SessionState, ConversationScope, ConversationScope) {
        let ch_a = ConversationScope::channel(Uuid::new_v4());
        let ch_b = ConversationScope::channel(Uuid::new_v4());
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
            &PromptSource::Conversation(ch_a),
            &ControlSignal::Rotate,
        );

        assert!(!s.sessions.contains_key(&ch_a));
        assert!(!s.turn_counts.contains_key(&ch_a));
        assert!(!s.core_sections.contains_key(&ch_a));
        assert!(!s.has_scope_state(&ch_a));
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
            &PromptSource::Conversation(ch_a),
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
        s.invalidate(&PromptSource::Conversation(ch_a));

        assert!(!s.sessions.contains_key(&ch_a));
        assert!(!s.turn_counts.contains_key(&ch_a));
        assert!(!s.core_sections.contains_key(&ch_a));
        assert!(!s.has_scope_state(&ch_a));
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
        let ghost = ConversationScope::channel(Uuid::new_v4());
        s.invalidate(&PromptSource::Conversation(ghost));

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
    fn test_invalidate_scope_returns_true_when_session_existed() {
        let (mut s, ch_a, ch_b) = make_state();
        assert!(s.invalidate_scope(&ch_a));
        assert!(!s.sessions.contains_key(&ch_a));
        assert!(!s.turn_counts.contains_key(&ch_a));
        assert!(!s.core_sections.contains_key(&ch_a));
        assert!(!s.has_scope_state(&ch_a));
        // ch_b untouched
        assert_eq!(s.sessions.get(&ch_b).unwrap(), "sess-b");
        assert_eq!(*s.turn_counts.get(&ch_b).unwrap(), 3);
        assert_eq!(s.core_sections.get(&ch_b).unwrap(), "core-b");
        // heartbeat untouched
        assert_eq!(s.heartbeat_session.as_deref(), Some("sess-hb"));
        assert_eq!(s.heartbeat_turn_count, 7);
    }

    #[test]
    fn test_invalidate_scope_returns_false_when_no_session() {
        let (mut s, _ch_a, _ch_b) = make_state();
        let ghost = ConversationScope::channel(Uuid::new_v4());
        assert!(!s.invalidate_scope(&ghost));
        // Nothing changed.
        assert_eq!(s.sessions.len(), 2);
        assert_eq!(s.turn_counts.len(), 2);
    }

    #[test]
    fn test_removed_channels_cleaned_via_invalidate_channel() {
        // Simulates handle_prompt_result: channels removed while agent
        // was checked out should have both sessions and turn_counts stripped.
        let (mut s, ch_a, ch_b) = make_state();
        let removed = vec![ch_a.channel_id];
        for ch in &removed {
            s.invalidate_channel(ch);
        }
        assert!(!s.sessions.contains_key(&ch_a));
        assert!(!s.turn_counts.contains_key(&ch_a));
        assert!(!s.core_sections.contains_key(&ch_a));
        assert!(!s.has_scope_state(&ch_a));
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
            &PromptSource::Conversation(ch_a),
            &ControlSignal::SwitchModel { generation: 1 },
        );

        assert!(!s.has_scope_state(&ch_a));
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

    fn one_event_batch(scope: ConversationScope) -> FlushBatch {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(9), "test")
            .sign_with_keys(&keys)
            .unwrap();
        FlushBatch {
            scope,
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
                ControlSignal::SwitchModel { generation: 1 },
                Some(CancelReason::Interrupt),
            ),
            (ControlSignal::Cancel, None),
            (ControlSignal::Rotate, None),
        ];
        let mut ctx = make_prompt_context_no_owner();
        ctx.dedup_mode = DedupMode::Queue;

        for (signal, expected_reason) in cases {
            let batch = one_event_batch(ConversationScope::channel(Uuid::new_v4()));
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
                signal: ControlSignal::SwitchModel { generation: 1 },
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
            let batch = one_event_batch(ConversationScope::channel(Uuid::new_v4()));
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
            baseline_model: None,
            desired_model: None,
            desired_model_generation: None,
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
            baseline_model: None,
            desired_model: None,
            desired_model_generation: None,
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
            model_controls: ChannelModelControls::default(),
        }
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
    fn test_invalidate_scope_clears_canvas_section() {
        let ch = ConversationScope::channel(Uuid::new_v4());
        let mut s = SessionState::default();
        s.sessions.insert(ch, "sess".into());
        s.canvas_sections
            .insert(ch, "[Channel Canvas]\nrev abc".into());

        s.invalidate_scope(&ch);

        assert!(!s.canvas_sections.contains_key(&ch));
        assert!(!s.sessions.contains_key(&ch));
    }

    #[test]
    fn test_invalidate_all_clears_canvas_sections() {
        let ch_a = ConversationScope::channel(Uuid::new_v4());
        let ch_b = ConversationScope::channel(Uuid::new_v4());
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
        let ch_a = ConversationScope::channel(Uuid::new_v4());
        let ch_b = ConversationScope::channel(Uuid::new_v4());
        let mut s = SessionState::default();
        s.sessions.insert(ch_a, "sess-a".into());
        s.sessions.insert(ch_b, "sess-b".into());
        s.canvas_sections.insert(ch_a, "canvas-a".into());
        s.canvas_sections.insert(ch_b, "canvas-b".into());

        s.invalidate_channel(&ch_a.channel_id);

        assert!(!s.canvas_sections.contains_key(&ch_a));
        assert_eq!(s.canvas_sections.get(&ch_b).unwrap(), "canvas-b");
    }

    #[test]
    fn test_has_scope_state_true_when_only_canvas_section_present() {
        let ch = ConversationScope::channel(Uuid::new_v4());
        let mut s = SessionState::default();
        s.canvas_sections.insert(ch, "canvas".into());
        assert!(s.has_scope_state(&ch));
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
mod thread_scope_session_tests {
    use super::*;
    use crate::scope::ConversationScope;
    use nostr::EventId;
    use uuid::Uuid;

    fn thread_scope(channel_id: Uuid, root_byte: char) -> ConversationScope {
        let root = EventId::from_hex(&root_byte.to_string().repeat(64)).expect("valid hex");
        ConversationScope::thread(channel_id, root)
    }

    fn context_batch(scope: ConversationScope, tagged_root: Option<char>) -> FlushBatch {
        let tags = tagged_root
            .map(|root| {
                nostr::Tag::parse(&[
                    "e".to_string(),
                    root.to_string().repeat(64),
                    String::new(),
                    "reply".to_string(),
                ])
                .expect("reply tag")
            })
            .into_iter()
            .collect::<Vec<_>>();
        let event = nostr::EventBuilder::new(nostr::Kind::Custom(9), "context")
            .tags(tags)
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign context event");
        FlushBatch {
            scope,
            events: vec![crate::queue::BatchEvent {
                event,
                prompt_tag: "test".into(),
                received_at: std::time::Instant::now(),
            }],
            cancelled_events: vec![],
            cancel_reason: None,
        }
    }

    /// Hydration must follow the same canonical root that keys the queue and
    /// session. DM replies are the exception: their session remains channel
    /// scoped while their last event's root still selects reply context.
    #[test]
    fn context_hydration_uses_typed_root_and_preserves_dm_reply_context() {
        let channel_id = Uuid::new_v4();
        let typed_scope = thread_scope(channel_id, 'a');

        let conflicting_tag = context_batch(typed_scope, Some('b'));
        assert_eq!(
            context_thread_root(&conflicting_tag, false).map(|root| root.to_hex()),
            Some("a".repeat(64)),
            "an event tag may not redirect context away from the typed session root"
        );

        let forum_root_without_reply_tags = context_batch(typed_scope, None);
        assert_eq!(
            context_thread_root(&forum_root_without_reply_tags, false).map(|root| root.to_hex()),
            Some("a".repeat(64)),
            "a forum root hydrates by its typed own-event scope"
        );

        let dm_reply = context_batch(ConversationScope::channel(channel_id), Some('b'));
        assert_eq!(
            context_thread_root(&dm_reply, true).map(|root| root.to_hex()),
            Some("b".repeat(64)),
            "DM replies preserve thread-context hydration"
        );
        assert_eq!(
            context_thread_root(&dm_reply, false),
            None,
            "an ordinary channel-scoped message must not import a tagged root"
        );
    }

    /// Two threads in one channel — and the channel-level conversation — hold
    /// distinct session entries; replies in one thread reuse that thread's
    /// session key.
    #[test]
    fn threads_and_channel_hold_distinct_sessions() {
        let ch = Uuid::new_v4();
        let thread_a = thread_scope(ch, 'a');
        let thread_b = thread_scope(ch, 'b');
        let channel = ConversationScope::channel(ch);

        let mut s = SessionState::default();
        s.sessions.insert(thread_a, "sess-thread-a".into());
        s.sessions.insert(thread_b, "sess-thread-b".into());
        s.sessions.insert(channel, "sess-channel".into());

        // A reply in thread A resolves to thread A's session (reuse), not the
        // channel session and not thread B's.
        assert_eq!(s.sessions.get(&thread_a).unwrap(), "sess-thread-a");
        assert_eq!(s.sessions.get(&thread_b).unwrap(), "sess-thread-b");
        assert_eq!(s.sessions.get(&channel).unwrap(), "sess-channel");
        assert_eq!(s.sessions.len(), 3);
    }

    /// Rotating one thread's scope leaves the sibling thread and the
    /// channel-level conversation untouched.
    #[test]
    fn invalidate_scope_targets_only_that_thread() {
        let ch = Uuid::new_v4();
        let thread_a = thread_scope(ch, 'a');
        let thread_b = thread_scope(ch, 'b');
        let channel = ConversationScope::channel(ch);

        let mut s = SessionState::default();
        for (scope, sid) in [(thread_a, "a"), (thread_b, "b"), (channel, "c")] {
            s.sessions.insert(scope, sid.into());
            s.turn_counts.insert(scope, 2);
            s.core_sections.insert(scope, "core".into());
            s.canvas_sections.insert(scope, "canvas".into());
        }

        assert!(s.invalidate_scope(&thread_a));

        assert!(!s.sessions.contains_key(&thread_a));
        assert!(!s.turn_counts.contains_key(&thread_a));
        assert!(!s.core_sections.contains_key(&thread_a));
        assert!(!s.canvas_sections.contains_key(&thread_a));
        assert_eq!(s.sessions.get(&thread_b).unwrap(), "b");
        assert_eq!(s.sessions.get(&channel).unwrap(), "c");
        assert_eq!(*s.turn_counts.get(&thread_b).unwrap(), 2);
    }

    /// Channel removal invalidates every conversation scope under that
    /// channel — channel-level and all threads — and nothing else.
    #[test]
    fn invalidate_channel_sweeps_channel_and_thread_scopes() {
        let ch = Uuid::new_v4();
        let other = Uuid::new_v4();
        let thread_a = thread_scope(ch, 'a');
        let thread_b = thread_scope(ch, 'b');
        let channel = ConversationScope::channel(ch);
        let other_scope = ConversationScope::channel(other);
        let other_thread = thread_scope(other, 'e');

        let mut s = SessionState::default();
        for scope in [thread_a, thread_b, channel, other_scope, other_thread] {
            s.sessions.insert(scope, "sess".into());
            s.turn_counts.insert(scope, 1);
            s.core_sections.insert(scope, "core".into());
            s.canvas_sections.insert(scope, "canvas".into());
        }

        assert!(s.invalidate_channel(&ch));

        for scope in [thread_a, thread_b, channel] {
            assert!(!s.has_scope_state(&scope), "{scope} must be swept");
        }
        for scope in [other_scope, other_thread] {
            assert!(s.has_scope_state(&scope), "{scope} must survive");
        }
        // Idempotent: nothing left to remove.
        assert!(!s.invalidate_channel(&ch));
    }

    /// Proactive rotation counters are per-conversation: turns in one thread
    /// must not advance the rotation clock of siblings.
    #[test]
    fn turn_counts_are_per_conversation() {
        let ch = Uuid::new_v4();
        let thread_a = thread_scope(ch, 'a');
        let channel = ConversationScope::channel(ch);

        let mut s = SessionState::default();
        *s.turn_counts.entry(thread_a).or_insert(0) += 1;
        *s.turn_counts.entry(thread_a).or_insert(0) += 1;
        *s.turn_counts.entry(channel).or_insert(0) += 1;

        assert_eq!(*s.turn_counts.get(&thread_a).unwrap(), 2);
        assert_eq!(*s.turn_counts.get(&channel).unwrap(), 1);
    }
}
