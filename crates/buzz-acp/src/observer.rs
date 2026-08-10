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
    /// Authors whose triggering events started this turn.
    ///
    /// Internal routing metadata only. It is never serialized into the
    /// encrypted observer payload. Cloned contexts share this recipient set
    /// so a requester accepted through a native mid-turn steer is visible to
    /// liveness and terminal observers for the same turn.
    requester_pubkeys: Arc<Mutex<Vec<String>>>,
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
    /// ACP session ID when known.
    pub session_id: Option<String>,
    /// Local UUID for one prompt turn.
    pub turn_id: Option<String>,
    /// RFC3339 timestamp at which the current turn began, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Per-turn requester recipients. Internal publisher metadata only.
    #[serde(skip)]
    pub requester_pubkeys: Vec<String>,
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
            session_id: context.session_id.clone(),
            turn_id: context.turn_id.clone(),
            started_at: context.started_at.clone(),
            requester_pubkeys: context.requester_pubkeys(),
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
        requester_pubkeys: Arc::new(Mutex::new(Vec::new())),
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
        requester_pubkeys: Arc::new(Mutex::new(Vec::new())),
    }
}

impl ObserverContext {
    /// Attach the normalized, de-duplicated authors whose events triggered
    /// this turn. The list remains process-local and is used only to select
    /// NIP-44 recipients.
    pub fn with_requester_pubkeys(mut self, requester_pubkeys: Vec<String>) -> Self {
        let mut requester_pubkeys = requester_pubkeys
            .into_iter()
            .map(|pubkey| pubkey.to_ascii_lowercase())
            .collect::<Vec<_>>();
        requester_pubkeys.sort();
        requester_pubkeys.dedup();
        self.requester_pubkeys = Arc::new(Mutex::new(requester_pubkeys));
        self
    }

    /// Add a requester to this turn's shared recipient set.
    pub fn add_requester_pubkey(&self, requester_pubkey: impl Into<String>) {
        let requester_pubkey = requester_pubkey.into().to_ascii_lowercase();
        let mut requester_pubkeys = match self.requester_pubkeys.lock() {
            Ok(requester_pubkeys) => requester_pubkeys,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !requester_pubkeys.contains(&requester_pubkey) {
            requester_pubkeys.push(requester_pubkey);
            requester_pubkeys.sort();
        }
    }

    fn requester_pubkeys(&self) -> Vec<String> {
        match self.requester_pubkeys.lock() {
            Ok(requester_pubkeys) => requester_pubkeys.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requester_routing_metadata_is_normalized_but_not_serialized() {
        let context = context_for_turn(None, None, "turn-1".into(), "2026-07-24T10:00:00Z".into())
            .with_requester_pubkeys(vec!["BBBB".into(), "aaaa".into(), "AAAA".into()]);
        let cloned_context = context.clone();
        cloned_context.add_requester_pubkey("CCCC");
        let observer = ObserverHandle::in_process();
        observer.emit("turn_started", Some(0), &context, serde_json::json!({}));
        let event = observer.snapshot().pop().expect("observer event");

        assert_eq!(event.requester_pubkeys, ["aaaa", "bbbb", "cccc"]);
        let serialized = serde_json::to_value(&event).expect("serialize observer event");
        assert!(
            serialized.get("requesterPubkeys").is_none(),
            "requester routing metadata must stay outside encrypted payloads"
        );
    }
}
