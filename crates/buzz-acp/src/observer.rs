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
/// Reserved capacity for terminal control results. Ordinary ACP telemetry can
/// saturate its own broadcast/replay buffers without consuming these slots.
const OBSERVER_CONTROL_RESULT_CAP: usize = 64;

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
    control_result_tx: broadcast::Sender<ObserverEvent>,
    buffer: Mutex<VecDeque<ObserverEvent>>,
    control_result_buffer: Mutex<VecDeque<ObserverEvent>>,
    seq: AtomicU64,
}

fn new_observer_handle() -> ObserverHandle {
    let (tx, _) = broadcast::channel(OBSERVER_BUFFER_CAP);
    let (control_result_tx, _) = broadcast::channel(OBSERVER_CONTROL_RESULT_CAP);
    ObserverHandle {
        inner: Arc::new(ObserverInner {
            tx,
            control_result_tx,
            buffer: Mutex::new(VecDeque::with_capacity(OBSERVER_BUFFER_CAP)),
            control_result_buffer: Mutex::new(VecDeque::with_capacity(OBSERVER_CONTROL_RESULT_CAP)),
            seq: AtomicU64::new(1),
        }),
    }
}

/// Priority-aware receiver for the observer publisher.
///
/// Terminal control results have reserved broadcast capacity and win when both
/// lanes are ready. The public result shape intentionally matches Tokio's
/// broadcast receiver so existing lag/close handling remains applicable.
pub struct ObserverReceiver {
    control_result_rx: broadcast::Receiver<ObserverEvent>,
    telemetry_rx: broadcast::Receiver<ObserverEvent>,
    control_result_closed: bool,
    telemetry_closed: bool,
}

impl ObserverReceiver {
    /// Receive only from the reserved terminal-control lane.
    ///
    /// The relay publisher uses this while ordinary telemetry is being paced
    /// so a newly emitted terminal result can interrupt that wait.
    pub(crate) async fn recv_control_result(
        &mut self,
    ) -> Result<ObserverEvent, broadcast::error::RecvError> {
        if self.control_result_closed {
            return Err(broadcast::error::RecvError::Closed);
        }
        let result = self.control_result_rx.recv().await;
        if matches!(result, Err(broadcast::error::RecvError::Closed)) {
            self.control_result_closed = true;
        }
        result
    }

    /// Receive the next observer event, draining terminal control results first.
    pub async fn recv(&mut self) -> Result<ObserverEvent, broadcast::error::RecvError> {
        loop {
            match (self.control_result_closed, self.telemetry_closed) {
                (true, true) => return Err(broadcast::error::RecvError::Closed),
                (false, true) => match self.control_result_rx.recv().await {
                    Err(broadcast::error::RecvError::Closed) => {
                        self.control_result_closed = true;
                    }
                    result => return result,
                },
                (true, false) => match self.telemetry_rx.recv().await {
                    Err(broadcast::error::RecvError::Closed) => {
                        self.telemetry_closed = true;
                    }
                    result => return result,
                },
                (false, false) => {
                    tokio::select! {
                        biased;
                        result = self.control_result_rx.recv() => {
                            match result {
                                Err(broadcast::error::RecvError::Closed) => {
                                    self.control_result_closed = true;
                                }
                                result => return result,
                            }
                        }
                        result = self.telemetry_rx.recv() => {
                            match result {
                                Err(broadcast::error::RecvError::Closed) => {
                                    self.telemetry_closed = true;
                                }
                                result => return result,
                            }
                        }
                    }
                }
            }
        }
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
    pub fn subscribe(&self) -> ObserverReceiver {
        ObserverReceiver {
            control_result_rx: self.inner.control_result_tx.subscribe(),
            telemetry_rx: self.inner.tx.subscribe(),
            control_result_closed: false,
            telemetry_closed: false,
        }
    }

    /// Return the current replay buffers with terminal control results first.
    ///
    /// Separate storage prevents a telemetry flood from evicting a result
    /// before a newly started publisher snapshots the observer.
    pub fn snapshot(&self) -> Vec<ObserverEvent> {
        let mut snapshot = match self.inner.control_result_buffer.lock() {
            Ok(buffer) => buffer.iter().cloned().collect::<Vec<_>>(),
            Err(error) => {
                tracing::warn!(
                    target: "observer",
                    "observer control-result replay buffer lock poisoned: {error}"
                );
                Vec::new()
            }
        };
        match self.inner.buffer.lock() {
            Ok(buffer) => snapshot.extend(buffer.iter().cloned()),
            Err(error) => {
                tracing::warn!(target: "observer", "observer replay buffer lock poisoned: {error}");
            }
        }
        snapshot
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

        if event.kind == "control_result" {
            match self.inner.control_result_buffer.lock() {
                Ok(mut buffer) => {
                    if buffer.len() >= OBSERVER_CONTROL_RESULT_CAP {
                        buffer.pop_front();
                    }
                    buffer.push_back(event.clone());
                }
                Err(error) => {
                    tracing::warn!(
                        target: "observer",
                        "observer control-result replay buffer lock poisoned: {error}"
                    );
                }
            }
            let _ = self.inner.control_result_tx.send(event);
        } else {
            match self.inner.buffer.lock() {
                Ok(mut buffer) => {
                    if buffer.len() >= OBSERVER_BUFFER_CAP {
                        buffer.pop_front();
                    }
                    buffer.push_back(event.clone());
                }
                Err(error) => {
                    tracing::warn!(
                        target: "observer",
                        "observer replay buffer lock poisoned: {error}"
                    );
                }
            }
            let _ = self.inner.tx.send(event);
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

    fn emit_kind(observer: &ObserverHandle, kind: &str, marker: usize) {
        observer.emit(
            kind,
            None,
            &ObserverContext::default(),
            serde_json::json!({"marker": marker}),
        );
    }

    #[tokio::test]
    async fn control_result_drains_ahead_of_lagged_telemetry_broadcast() {
        let observer = ObserverHandle::in_process();
        let mut rx = observer.subscribe();
        for marker in 0..=OBSERVER_BUFFER_CAP {
            emit_kind(&observer, "acp_read", marker);
        }
        emit_kind(&observer, "control_result", 7);

        let delivered = rx.recv().await.expect("protected result remains readable");
        assert_eq!(delivered.kind, "control_result");
        assert_eq!(delivered.payload["marker"], 7);
    }

    #[test]
    fn control_result_snapshot_capacity_is_reserved_from_telemetry_overflow() {
        let observer = ObserverHandle::in_process();
        emit_kind(&observer, "control_result", 11);
        for marker in 0..=OBSERVER_BUFFER_CAP {
            emit_kind(&observer, "acp_read", marker);
        }

        let snapshot = observer.snapshot();
        assert_eq!(
            snapshot.first().map(|event| event.kind.as_str()),
            Some("control_result"),
            "protected results drain before the saturated telemetry snapshot"
        );
        assert_eq!(snapshot[0].payload["marker"], 11);
        assert_eq!(
            snapshot
                .iter()
                .filter(|event| event.kind == "acp_read")
                .count(),
            OBSERVER_BUFFER_CAP
        );
    }
}
