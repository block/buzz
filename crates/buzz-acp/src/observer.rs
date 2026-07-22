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
    /// Canonical NIP-10 thread root for a thread-scoped conversation.
    /// Absent for unthreaded channels, DMs, heartbeats, and legacy frames.
    pub thread_root: Option<String>,
    /// ACP session ID associated with the current turn, once known.
    pub session_id: Option<String>,
    /// Local UUID for one prompt turn.
    pub turn_id: Option<String>,
    /// RFC3339 timestamp at which the current turn began, when known.
    pub started_at: Option<String>,
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
    /// Canonical NIP-10 thread root for a thread-scoped conversation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_root: Option<String>,
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
            thread_root: context.thread_root.clone(),
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
#[cfg(test)]
pub fn context_for(
    channel_id: Option<uuid::Uuid>,
    session_id: Option<String>,
    turn_id: Option<String>,
) -> ObserverContext {
    ObserverContext {
        channel_id: channel_id.map(|id| id.to_string()),
        thread_root: None,
        session_id,
        turn_id,
        started_at: None,
    }
}

/// Attach the authoritative start timestamp to every observer frame for a turn.
#[cfg(test)]
pub fn context_for_turn(
    channel_id: Option<uuid::Uuid>,
    session_id: Option<String>,
    turn_id: String,
    started_at: String,
) -> ObserverContext {
    ObserverContext {
        channel_id: channel_id.map(|id| id.to_string()),
        thread_root: None,
        session_id,
        turn_id: Some(turn_id),
        started_at: Some(started_at),
    }
}

/// Build observer context from a typed conversation scope.
pub fn context_for_scope(
    scope: Option<crate::scope::ConversationScope>,
    session_id: Option<String>,
    turn_id: Option<String>,
) -> ObserverContext {
    ObserverContext {
        channel_id: scope.map(|scope| scope.channel_id.to_string()),
        thread_root: scope.and_then(|scope| scope.thread_root.map(|root| root.to_hex())),
        session_id,
        turn_id,
        started_at: None,
    }
}

/// Build turn context from a typed conversation scope, including identity on
/// pre-session frames.
pub fn context_for_scope_turn(
    scope: Option<crate::scope::ConversationScope>,
    session_id: Option<String>,
    turn_id: String,
    started_at: String,
) -> ObserverContext {
    ObserverContext {
        channel_id: scope.map(|scope| scope.channel_id.to_string()),
        thread_root: scope.and_then(|scope| scope.thread_root.map(|root| root.to_hex())),
        session_id,
        turn_id: Some(turn_id),
        started_at: Some(started_at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::EventId;
    use uuid::Uuid;

    #[test]
    fn thread_scope_is_present_before_session_resolution_and_on_emitted_frames() {
        let channel_id = Uuid::new_v4();
        let root = EventId::from_hex(&"a".repeat(64)).expect("valid root");
        let scope = crate::scope::ConversationScope::thread(channel_id, root);
        let context = context_for_scope_turn(
            Some(scope),
            None,
            "turn-a".to_string(),
            "2026-07-28T00:00:00Z".to_string(),
        );
        let channel_text = channel_id.to_string();
        let root_text = "a".repeat(64);
        assert_eq!(context.channel_id.as_deref(), Some(channel_text.as_str()));
        assert_eq!(context.thread_root.as_deref(), Some(root_text.as_str()));
        assert_eq!(context.session_id, None);

        let observer = ObserverHandle::in_process();
        observer.emit("turn_started", Some(0), &context, serde_json::json!({}));
        let event = observer.snapshot().pop().expect("one observer event");
        assert_eq!(event.thread_root.as_deref(), Some(root_text.as_str()));
        assert_eq!(event.session_id, None);
        assert_eq!(
            serde_json::to_value(event).expect("serialize")["threadRoot"],
            "a".repeat(64)
        );
    }

    #[test]
    fn unthreaded_context_remains_backwards_compatible() {
        let context = context_for_scope(
            Some(crate::scope::ConversationScope::channel(Uuid::new_v4())),
            Some("session-1".to_string()),
            Some("turn-1".to_string()),
        );
        let observer = ObserverHandle::in_process();
        observer.emit("session_resolved", Some(0), &context, serde_json::json!({}));
        let event = observer.snapshot().pop().expect("one observer event");
        let serialized = serde_json::to_value(event).expect("serialize");
        assert!(
            serialized.get("threadRoot").is_none(),
            "legacy/unthreaded frames must omit the optional field"
        );
        assert_eq!(serialized["sessionId"], "session-1");
    }
}
