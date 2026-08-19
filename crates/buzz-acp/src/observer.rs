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
    /// NIP-10 thread root for the current turn, when thread-scoped.
    pub thread_head_id: Option<String>,
    /// ACP session ID associated with the current turn, once known.
    pub session_id: Option<String>,
    /// Local UUID for one prompt turn.
    pub turn_id: Option<String>,
    /// RFC3339 timestamp at which the current turn began, when known.
    pub started_at: Option<String>,
}

/// Shared thread scope for a live observer turn.
///
/// Native steering can move one in-flight ACP turn between Buzz threads. Raw
/// ACP frames, liveness pings, and the completion guard all hold clones of this
/// handle so a successful steer updates every later frame atomically.
#[derive(Clone, Debug)]
pub struct ObserverTurnScope {
    thread_head_id: Arc<Mutex<Option<String>>>,
}

impl ObserverTurnScope {
    /// Create a turn scope with its initial NIP-10 thread root.
    pub fn new(thread_head_id: Option<String>) -> Self {
        Self {
            thread_head_id: Arc::new(Mutex::new(thread_head_id)),
        }
    }

    /// Return the turn's current NIP-10 thread root.
    pub fn thread_head_id(&self) -> Option<String> {
        match self.thread_head_id.lock() {
            Ok(thread_head_id) => thread_head_id.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Move the live turn to another NIP-10 thread root.
    pub fn set_thread_head_id(&self, thread_head_id: Option<String>) {
        match self.thread_head_id.lock() {
            Ok(mut current) => *current = thread_head_id,
            Err(poisoned) => *poisoned.into_inner() = thread_head_id,
        }
    }
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
#[derive(Clone, Serialize)]
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
    /// NIP-10 thread root for this frame's turn. Serializes as null for the
    /// channel root so consumers can distinguish an explicit root rescope from
    /// legacy frames that omitted scope metadata.
    pub thread_head_id: Option<String>,
    /// ACP session ID when known.
    pub session_id: Option<String>,
    /// Local UUID for one prompt turn.
    pub turn_id: Option<String>,
    /// RFC3339 timestamp at which the current turn began, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
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
        let event = ObserverEvent {
            seq: self.inner.seq.fetch_add(1, Ordering::Relaxed),
            timestamp: chrono::Utc::now().to_rfc3339(),
            kind: kind.into(),
            agent_index,
            channel_id: context.channel_id.clone(),
            thread_head_id: context.thread_head_id.clone(),
            session_id: context.session_id.clone(),
            turn_id: context.turn_id.clone(),
            started_at: context.started_at.clone(),
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
        thread_head_id: None,
        session_id,
        turn_id,
        started_at: None,
    }
}

/// Attach the authoritative start timestamp to every observer frame for a turn.
pub fn context_for_turn(
    channel_id: Option<uuid::Uuid>,
    thread_head_id: Option<String>,
    session_id: Option<String>,
    turn_id: String,
    started_at: String,
) -> ObserverContext {
    ObserverContext {
        channel_id: channel_id.map(|id| id.to_string()),
        thread_head_id,
        session_id,
        turn_id: Some(turn_id),
        started_at: Some(started_at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_root_scope_serializes_as_explicit_null() {
        let event = ObserverEvent {
            seq: 1,
            timestamp: "2026-08-18T00:00:00Z".into(),
            kind: "turn_liveness".into(),
            agent_index: Some(0),
            channel_id: Some("channel-1".into()),
            thread_head_id: None,
            session_id: Some("session-1".into()),
            turn_id: Some("turn-1".into()),
            started_at: None,
            payload: serde_json::json!({}),
        };

        let json = serde_json::to_value(event).expect("serialize observer event");
        assert!(json.get("threadHeadId").is_some());
        assert!(json["threadHeadId"].is_null());
    }
}
