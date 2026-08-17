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
/// Upper bound on the total serialized size of the replay buffer.
///
/// The entry cap alone does not bound memory: a single agent stdout line can
/// carry megabytes of payload, so 1000 entries can be arbitrarily large.
const OBSERVER_BYTE_BUDGET: usize = 4 * 1024 * 1024;

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

/// Handle used by the harness to publish local observer events.
#[derive(Clone)]
pub struct ObserverHandle {
    inner: Arc<ObserverInner>,
}

struct ObserverInner {
    tx: broadcast::Sender<ObserverEvent>,
    /// Replay entries paired with their serialized size in bytes.
    buffer: Mutex<VecDeque<(ObserverEvent, u64)>>,
    /// Running total of the sizes in `buffer`; only mutated under that lock.
    buffer_bytes: AtomicU64,
    seq: AtomicU64,
}

fn new_observer_handle() -> ObserverHandle {
    let (tx, _) = broadcast::channel(OBSERVER_BUFFER_CAP);
    ObserverHandle {
        inner: Arc::new(ObserverInner {
            tx,
            buffer: Mutex::new(VecDeque::with_capacity(OBSERVER_BUFFER_CAP)),
            buffer_bytes: AtomicU64::new(0),
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
            Ok(buffer) => buffer.iter().map(|(event, _)| event.clone()).collect(),
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
            payload,
        };

        let event_bytes = serde_json::to_vec(&event)
            .map(|bytes| bytes.len() as u64)
            .unwrap_or(0);

        match self.inner.buffer.lock() {
            Ok(mut buffer) => {
                let mut total_bytes = self.inner.buffer_bytes.load(Ordering::Relaxed);
                while buffer.len() >= OBSERVER_BUFFER_CAP
                    || total_bytes.saturating_add(event_bytes) > OBSERVER_BYTE_BUDGET as u64
                {
                    match buffer.pop_front() {
                        Some((_, evicted_bytes)) => {
                            total_bytes = total_bytes.saturating_sub(evicted_bytes)
                        }
                        None => break,
                    }
                }
                total_bytes = total_bytes.saturating_add(event_bytes);
                self.inner
                    .buffer_bytes
                    .store(total_bytes, Ordering::Relaxed);
                buffer.push_back((event.clone(), event_bytes));
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

#[cfg(test)]
mod sequence_buffer_tests {
    //! Sequence numbering and replay-buffer rotation invariants.
    use super::*;

    #[test]
    fn seq_is_monotonic_across_repeated_emit_calls() {
        let observer = ObserverHandle::in_process();
        observer.emit(
            "first",
            None,
            &ObserverContext::default(),
            serde_json::json!({}),
        );
        observer.emit(
            "second",
            None,
            &ObserverContext::default(),
            serde_json::json!({}),
        );
        observer.emit(
            "third",
            None,
            &ObserverContext::default(),
            serde_json::json!({}),
        );

        let snapshot = observer.snapshot();
        let kinds: Vec<&str> = snapshot.iter().map(|event| event.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["first", "second", "third"],
            "replay buffer must preserve emit order"
        );
        let seqs: Vec<u64> = snapshot.iter().map(|event| event.seq).collect();
        assert!(
            seqs.windows(2).all(|pair| pair[0] < pair[1]),
            "seq numbers must increase monotonically: {seqs:?}"
        );
    }

    #[test]
    fn cloned_handles_share_one_monotonic_sequence() {
        let observer = ObserverHandle::in_process();
        let clone = observer.clone();
        observer.emit(
            "original",
            None,
            &ObserverContext::default(),
            serde_json::json!({}),
        );
        clone.emit(
            "clone",
            None,
            &ObserverContext::default(),
            serde_json::json!({}),
        );
        observer.emit(
            "original-again",
            None,
            &ObserverContext::default(),
            serde_json::json!({}),
        );

        let snapshot = observer.snapshot();
        let seqs: Vec<u64> = snapshot.iter().map(|event| event.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3]);
    }

    #[test]
    fn dropped_subscribers_do_not_break_subsequent_emits() {
        let observer = ObserverHandle::in_process();
        let receiver = observer.subscribe();
        drop(receiver);
        observer.emit(
            "after-drop",
            None,
            &ObserverContext::default(),
            serde_json::json!({}),
        );
        let snapshot = observer.snapshot();
        assert_eq!(
            snapshot.last().map(|event| event.kind.as_str()),
            Some("after-drop")
        );
    }

    #[test]
    fn buffer_evicts_oldest_when_at_capacity() {
        let observer = ObserverHandle::in_process();
        for i in 0..(OBSERVER_BUFFER_CAP as u64) + 1 {
            observer.emit(
                "frame",
                None,
                &ObserverContext::default(),
                serde_json::json!({ "i": i }),
            );
        }

        let snapshot = observer.snapshot();
        assert_eq!(snapshot.len(), OBSERVER_BUFFER_CAP);
        assert_eq!(snapshot.first().map(|event| event.seq), Some(2));
        assert_eq!(
            snapshot
                .first()
                .and_then(|event| event.payload["i"].as_u64()),
            Some(1),
            "the first-emitted frame must be evicted"
        );
        assert_eq!(
            snapshot.last().map(|event| event.seq),
            Some((OBSERVER_BUFFER_CAP as u64) + 1)
        );
    }

    #[test]
    fn buffer_evicts_when_byte_budget_exceeded() {
        let observer = ObserverHandle::in_process();
        // Each payload is ~1 MiB, so the 4 MiB budget is exhausted long before
        // the 1000-entry cap is reached.
        let chunk = "x".repeat(1024 * 1024);
        for i in 0..8u64 {
            observer.emit(
                "frame",
                None,
                &ObserverContext::default(),
                serde_json::json!({ "i": i, "blob": chunk }),
            );
        }

        let snapshot = observer.snapshot();
        assert!(
            snapshot.len() < OBSERVER_BUFFER_CAP,
            "byte budget must evict before the entry cap is reached, got {} entries",
            snapshot.len()
        );
        let total: usize = snapshot
            .iter()
            .map(|event| serde_json::to_vec(event).map(|b| b.len()).unwrap_or(0))
            .sum();
        assert!(
            total <= OBSERVER_BYTE_BUDGET,
            "replay buffer holds {total} bytes, over the {OBSERVER_BYTE_BUDGET}-byte budget"
        );
        assert_eq!(
            snapshot
                .last()
                .and_then(|event| event.payload["i"].as_u64()),
            Some(7),
            "the newest frame must survive eviction"
        );
    }

    #[test]
    fn buffer_caps_by_byte_budget() {
        let observer = ObserverHandle::in_process();
        observer.emit(
            "small",
            None,
            &ObserverContext::default(),
            serde_json::json!({ "marker": "pre-oversized-sentinel" }),
        );
        // One payload larger than the whole budget must evict everything else
        // rather than sitting alongside it.
        observer.emit(
            "oversized",
            None,
            &ObserverContext::default(),
            serde_json::json!({ "blob": "y".repeat(OBSERVER_BYTE_BUDGET + 1) }),
        );

        let snapshot = observer.snapshot();
        assert_eq!(
            snapshot.len(),
            1,
            "the oversized event must evict every prior entry"
        );
        assert_eq!(snapshot[0].kind, "oversized");
        assert!(
            !snapshot
                .iter()
                .any(|event| event.payload["marker"] == "pre-oversized-sentinel"),
            "the pre-oversized entry must be evicted"
        );
    }
}
