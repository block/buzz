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
    time::Instant,
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

/// Handle used by the harness to publish local observer events.
#[derive(Clone)]
pub struct ObserverHandle {
    inner: Arc<ObserverInner>,
}

struct ObserverInner {
    tx: broadcast::Sender<ObserverEvent>,
    buffer: Mutex<VecDeque<ObserverEvent>>,
    seq: AtomicU64,
    monotonic_origin: Instant,
    latency: Mutex<crate::latency::LatencyCollector>,
}

fn new_observer_handle() -> ObserverHandle {
    let (tx, _) = broadcast::channel(OBSERVER_BUFFER_CAP);
    ObserverHandle {
        inner: Arc::new(ObserverInner {
            tx,
            buffer: Mutex::new(VecDeque::with_capacity(OBSERVER_BUFFER_CAP)),
            seq: AtomicU64::new(1),
            monotonic_origin: Instant::now(),
            latency: Mutex::new(crate::latency::LatencyCollector::default()),
        }),
    }
}

/// Event delivered through the in-process observer bus.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserverEvent {
    /// Monotonic process-local sequence number.
    pub seq: u64,
    /// Milliseconds since this process-local observer feed was created.
    ///
    /// This clock is authoritative for durations between events from the same
    /// harness process. It must not be compared across processes.
    pub monotonic_ms: u64,
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
            monotonic_ms: u64::try_from(self.inner.monotonic_origin.elapsed().as_millis())
                .unwrap_or(u64::MAX),
            timestamp: chrono::Utc::now().to_rfc3339(),
            kind: kind.into(),
            agent_index,
            channel_id: context.channel_id.clone(),
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

        let _ = self.inner.tx.send(event.clone());

        // Build content-free mention-to-reply traces synchronously from the
        // semantic observer boundaries. Synchronous ingestion preserves the
        // source event order and guarantees the derived sample is buffered
        // immediately after the event that completed it.
        let completed = match self.inner.latency.lock() {
            Ok(mut latency) => latency.ingest(&event),
            Err(error) => {
                tracing::warn!(target: "observer", "latency collector lock poisoned: {error}");
                None
            }
        };
        if let Some(completed) = completed {
            self.emit(
                "mention_reply_latency",
                completed.agent_index,
                &completed.context,
                completed.payload,
            );
        }
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
mod tests {
    use super::*;

    #[test]
    fn observer_serializes_process_local_monotonic_time() {
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

        let events = observer.snapshot();
        assert_eq!(events.len(), 2);
        assert!(events[0].monotonic_ms <= events[1].monotonic_ms);
        assert_eq!(
            serde_json::to_value(&events[0]).unwrap()["monotonicMs"],
            events[0].monotonic_ms
        );
    }

    #[test]
    fn observer_emits_derived_latency_sample() {
        let observer = ObserverHandle::in_process();
        let channel_id = uuid::Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let trigger_context = context_for(Some(channel_id), None, None);
        let turn_context = context_for_turn(
            Some(channel_id),
            Some("session-1".into()),
            "turn-1".into(),
            "2026-07-22T00:00:00Z".into(),
        );

        observer.emit(
            "event_received",
            None,
            &trigger_context,
            serde_json::json!({"eventId": "trigger-1", "receiptToObserverMs": 0}),
        );
        observer.emit(
            "event_queued",
            None,
            &trigger_context,
            serde_json::json!({"eventId": "trigger-1"}),
        );
        observer.emit(
            "turn_started",
            Some(0),
            &turn_context,
            serde_json::json!({"triggeringEventIds": ["trigger-1"]}),
        );
        observer.emit(
            "session_resolved",
            Some(0),
            &turn_context,
            serde_json::json!({"isNewSession": false}),
        );
        observer.emit(
            "prompt_dispatched",
            Some(0),
            &turn_context,
            serde_json::json!({}),
        );
        observer.emit(
            "turn_first_output",
            Some(0),
            &turn_context,
            serde_json::json!({}),
        );
        observer.emit(
            "reply_observed",
            Some(0),
            &turn_context,
            serde_json::json!({"eventId": "reply-1"}),
        );
        observer.emit(
            "turn_completed",
            Some(0),
            &turn_context,
            serde_json::json!({}),
        );

        let events = observer.snapshot();
        let sample = events
            .iter()
            .find(|event| event.kind == "mention_reply_latency")
            .expect("derived latency sample");
        assert_eq!(sample.payload["traceId"], "trigger-1");
        assert_eq!(sample.payload["path"], "warm");
        assert_eq!(sample.payload["summary"]["windowSize"], 1);
    }
}
