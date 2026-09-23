//! In-process observer bus for ACP session activity.
//!
//! This is intentionally process-local infrastructure: it lets the harness
//! collect raw ACP JSON-RPC activity and publish owner-scoped encrypted relay
//! frames without exposing a local HTTP port.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use serde::Serialize;
use tokio::sync::broadcast;

const OBSERVER_BUFFER_CAP: usize = 1_000;

/// Best-effort metadata attached to observer events.
#[derive(Clone, Debug, Default)]
pub struct ObserverContext {
    /// Buzz channel UUID for the current turn, when channel-scoped.
    pub channel_id: Option<String>,
    /// ACP session ID associated with the current turn, once known.
    pub session_id: Option<String>,
    /// Local UUID for one prompt turn.
    pub turn_id: Option<String>,
    /// RFC3339 timestamp at which the current turn began, when known.
    pub started_at: Option<String>,
}

/// Authorization envelope attached to permission-related observer events.
///
/// Present on the single `acp_read` emitted after a permission request passes
/// the admission preflight, and on the corresponding `acp_write` after the
/// response is confirmed written.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationEnvelope {
    /// Single-use nonce bound to this request — delivered to the desktop and
    /// consumed exactly once when the owner makes a decision.
    pub request_nonce: String,
    /// `true` when the owner can take action (policy=ask, preflight passed,
    /// owner/observer available). `false` for auto-deny / fail-closed paths.
    pub actionable: bool,
    /// Human-readable reason when `actionable` is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Wire card-expiry (unix seconds) for an actionable card — the same value
    /// stored in the kind-9 sentinel. The desktop bounds its
    /// retransmit-until-acked loop by this deadline, so a decision published
    /// while the harness socket is down keeps being resent until the card
    /// expires (never past it). `None` on non-actionable / already-resolved
    /// frames, where no owner decision is awaited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

/// Handle used by the harness to publish local observer events.
#[derive(Clone)]
pub struct ObserverHandle {
    inner: Arc<ObserverInner>,
}

struct ObserverInner {
    tx: broadcast::Sender<ObserverEvent>,
    buffer: Mutex<VecDeque<ObserverEvent>>,
    seq: AtomicU64,
}

fn new_observer_handle() -> ObserverHandle {
    let (tx, _) = broadcast::channel(OBSERVER_BUFFER_CAP);
    ObserverHandle {
        inner: Arc::new(ObserverInner {
            tx,
            buffer: Mutex::new(VecDeque::with_capacity(OBSERVER_BUFFER_CAP)),
            seq: AtomicU64::new(1),
        }),
    }
}

/// Event delivered through the in-process observer bus.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserverEvent {
    /// Monotonic process-local sequence number.
    pub seq: u64,
    /// RFC3339 UTC timestamp.
    pub timestamp: String,
    /// Observer event kind, for example `acp_read` or `turn_started`.
    pub kind: String,
    /// Pool slot index for the agent process that emitted the event.
    pub agent_index: Option<usize>,
    /// Buzz channel UUID for channel-scoped events.
    pub channel_id: Option<String>,
    /// ACP session ID when known.
    pub session_id: Option<String>,
    /// Local UUID for one prompt turn.
    pub turn_id: Option<String>,
    /// RFC3339 timestamp at which the current turn began, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Authorization envelope — present only on permission `acp_read` /
    /// `acp_write` frames, and on the observer-only `permission_terminal` frame
    /// (which carries `reason = "uncertain"` and is never sent on the ACP wire).
    /// `None` on all other event kinds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization: Option<AuthorizationEnvelope>,
    /// Raw or semantic event payload.
    pub payload: serde_json::Value,
}

impl ObserverHandle {
    /// Create an in-process observer feed.
    pub fn in_process() -> Self {
        new_observer_handle()
    }

    /// Subscribe to live observer events.
    pub fn subscribe(&self) -> broadcast::Receiver<ObserverEvent> {
        self.inner.tx.subscribe()
    }

    /// Return the current replay buffer.
    pub fn snapshot(&self) -> Vec<ObserverEvent> {
        match self.inner.buffer.lock() {
            Ok(buffer) => buffer.iter().cloned().collect(),
            Err(error) => {
                tracing::warn!(target: "observer", "observer replay buffer lock poisoned: {error}");
                Vec::new()
            }
        }
    }

    /// Emit a local observer event.
    pub fn emit(
        &self,
        kind: impl Into<String>,
        agent_index: Option<usize>,
        context: &ObserverContext,
        payload: serde_json::Value,
    ) {
        self.emit_inner(kind, agent_index, context, None, payload);
    }

    /// Emit a local observer event with an authorization envelope.
    ///
    /// Used for permission `acp_read` and `acp_write` frames.
    pub fn emit_authorized(
        &self,
        kind: impl Into<String>,
        agent_index: Option<usize>,
        context: &ObserverContext,
        authorization: AuthorizationEnvelope,
        payload: serde_json::Value,
    ) {
        self.emit_inner(kind, agent_index, context, Some(authorization), payload);
    }

    fn emit_inner(
        &self,
        kind: impl Into<String>,
        agent_index: Option<usize>,
        context: &ObserverContext,
        authorization: Option<AuthorizationEnvelope>,
        payload: serde_json::Value,
    ) {
        let event = ObserverEvent {
            seq: self.inner.seq.fetch_add(1, Ordering::Relaxed),
            timestamp: chrono::Utc::now().to_rfc3339(),
            kind: kind.into(),
            agent_index,
            channel_id: context.channel_id.clone(),
            session_id: context.session_id.clone(),
            turn_id: context.turn_id.clone(),
            started_at: context.started_at.clone(),
            authorization,
            payload,
        };

        match self.inner.buffer.lock() {
            Ok(mut buffer) => {
                if buffer.len() >= OBSERVER_BUFFER_CAP {
                    buffer.pop_front();
                }
                buffer.push_back(event.clone());
            }
            Err(error) => {
                tracing::warn!(target: "observer", "observer replay buffer lock poisoned: {error}");
            }
        }

        let _ = self.inner.tx.send(event);
    }
}

/// Build observer context values from optional channel/session/turn IDs.
pub fn context_for(
    channel_id: Option<uuid::Uuid>,
    session_id: Option<String>,
    turn_id: Option<String>,
) -> ObserverContext {
    ObserverContext {
        channel_id: channel_id.map(|id| id.to_string()),
        session_id,
        turn_id,
        started_at: None,
    }
}

/// Attach the authoritative start timestamp to every observer frame for a turn.
pub fn context_for_turn(
    channel_id: Option<uuid::Uuid>,
    session_id: Option<String>,
    turn_id: String,
    started_at: String,
) -> ObserverContext {
    ObserverContext {
        channel_id: channel_id.map(|id| id.to_string()),
        session_id,
        turn_id: Some(turn_id),
        started_at: Some(started_at),
    }
}
