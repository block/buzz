//! In-process observer bus for ACP session activity.
//!
//! This is intentionally process-local infrastructure: it lets the harness
//! collect raw ACP JSON-RPC activity and publish owner-scoped encrypted relay
//! frames without exposing a local HTTP port.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use serde::Serialize;
use tokio::sync::broadcast;

const OBSERVER_BUFFER_CAP: usize = 1_000;
/// Reserved broadcast capacity for terminal control results. This is only the
/// live wake-up lane: terminal results themselves remain in the acknowledged
/// replay ledger until the relay publisher confirms delivery.
const OBSERVER_CONTROL_RESULT_CAP: usize = 64;
/// Hard process-local retention bound for terminal results awaiting relay ACK.
///
/// The live publisher normally owns at most one result at a time. This larger
/// reserve absorbs an extended relay outage without allowing an authenticated
/// control stream to grow process memory indefinitely. Overflow rejects the
/// newest result, logs loudly, and permanently marks shutdown proof unverified
/// for this observer lifecycle; it never evicts an older accepted result.
const OBSERVER_CONTROL_RESULT_LEDGER_CAP: usize = 1_024;

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
    control_results: Arc<Mutex<BTreeMap<u64, ObserverEvent>>>,
    control_result_rejected: Arc<AtomicU64>,
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
            control_results: Arc::new(Mutex::new(BTreeMap::new())),
            control_result_rejected: Arc::new(AtomicU64::new(0)),
            seq: AtomicU64::new(1),
        }),
    }
}

/// Priority-aware receiver for the observer publisher.
///
/// Terminal control results have reserved broadcast capacity and win when both
/// lanes are ready. This receiver is the single acknowledging consumer for
/// terminal results: the ledger retains each result until this publisher
/// confirms relay handoff, and any remainder is released when the process-local
/// observer lifecycle is dropped.
pub struct ObserverReceiver {
    control_result_rx: broadcast::Receiver<ObserverEvent>,
    telemetry_rx: broadcast::Receiver<ObserverEvent>,
    control_results: Arc<Mutex<BTreeMap<u64, ObserverEvent>>>,
    control_result_rejected: Arc<AtomicU64>,
    control_result_replay: VecDeque<ObserverEvent>,
    in_flight_control_results: HashSet<u64>,
    control_result_failures: HashMap<u64, u8>,
    control_result_closed: bool,
    telemetry_closed: bool,
}

impl ObserverReceiver {
    /// Whether this observer lifecycle rejected any terminal result because its
    /// bounded acknowledgement ledger was full.
    pub(crate) fn control_result_admission_failed(&self) -> bool {
        self.control_result_rejected.load(Ordering::Acquire) > 0
    }

    fn queue_retained_control_results(&mut self) {
        let retained = match self.control_results.lock() {
            Ok(results) => results
                .values()
                .filter(|event| !self.in_flight_control_results.contains(&event.seq))
                .cloned()
                .collect::<Vec<_>>(),
            Err(error) => {
                tracing::warn!(
                    target: "observer",
                    "observer control-result ledger lock poisoned; recovering retained results"
                );
                error
                    .into_inner()
                    .values()
                    .filter(|event| !self.in_flight_control_results.contains(&event.seq))
                    .cloned()
                    .collect()
            }
        };
        for event in retained {
            self.control_result_replay.push_back(event);
        }
    }

    fn pop_retained_control_result(&mut self) -> Option<ObserverEvent> {
        while let Some(event) = self.control_result_replay.pop_front() {
            let retained = match self.control_results.lock() {
                Ok(results) => results.contains_key(&event.seq),
                Err(error) => {
                    tracing::warn!(
                        target: "observer",
                        "observer control-result ledger lock poisoned; recovering delivery state"
                    );
                    error.into_inner().contains_key(&event.seq)
                }
            };
            if retained && self.in_flight_control_results.insert(event.seq) {
                return Some(event);
            }
        }
        None
    }

    fn accept_live_control_result(&mut self, event: ObserverEvent) -> Option<ObserverEvent> {
        let retained = match self.control_results.lock() {
            Ok(results) => results.contains_key(&event.seq),
            Err(error) => {
                tracing::warn!(
                    target: "observer",
                    "observer control-result ledger lock poisoned; recovering live delivery state"
                );
                error.into_inner().contains_key(&event.seq)
            }
        };
        (retained && self.in_flight_control_results.insert(event.seq)).then_some(event)
    }

    /// Confirm that a terminal result reached the relay publisher.
    ///
    /// This is the sole normal-lifecycle removal point for retained terminal
    /// outcomes. A lagging receiver resnapshots everything not acknowledged;
    /// dropping the in-process observer lifecycle drops any remaining ledger.
    pub(crate) fn acknowledge_control_result(&mut self, seq: u64) {
        self.in_flight_control_results.remove(&seq);
        self.control_result_failures.remove(&seq);
        self.control_result_replay.retain(|event| event.seq != seq);
        match self.control_results.lock() {
            Ok(mut results) => {
                results.remove(&seq);
            }
            Err(error) => {
                tracing::warn!(
                    target: "observer",
                    "observer control-result ledger lock poisoned; recovering acknowledgement"
                );
                error.into_inner().remove(&seq);
            }
        }
    }

    /// Release one failed terminal delivery for a single end-to-end retry.
    ///
    /// Relay publication already performs reconnect and rate-gate recovery
    /// within its own bounded acknowledgement window. This receiver grants one
    /// additional replay for failures that escape that window, then leaves a
    /// second failure retained and in-flight so shutdown reports unverified
    /// delivery without spinning forever.
    pub(crate) fn retry_control_result(&mut self, seq: u64) -> bool {
        if self
            .control_result_failures
            .get(&seq)
            .is_some_and(|failures| *failures >= 1)
        {
            return false;
        }
        let retained = match self.control_results.lock() {
            Ok(results) => results.get(&seq).cloned(),
            Err(error) => {
                tracing::warn!(
                    target: "observer",
                    "observer control-result ledger lock poisoned; recovering retry state"
                );
                error.into_inner().get(&seq).cloned()
            }
        };
        let Some(event) = retained else {
            return false;
        };

        self.control_result_failures.insert(seq, 1);
        self.in_flight_control_results.remove(&seq);
        if !self
            .control_result_replay
            .iter()
            .any(|queued| queued.seq == seq)
        {
            self.control_result_replay.push_back(event);
        }
        true
    }

    /// Receive only from the reserved terminal-control lane.
    ///
    /// The relay publisher uses this while ordinary telemetry is being paced
    /// so a newly emitted terminal result can interrupt that wait.
    pub(crate) async fn recv_control_result(
        &mut self,
    ) -> Result<ObserverEvent, broadcast::error::RecvError> {
        loop {
            if let Some(event) = self.pop_retained_control_result() {
                return Ok(event);
            }
            if self.control_result_closed {
                self.queue_retained_control_results();
                return self
                    .pop_retained_control_result()
                    .ok_or(broadcast::error::RecvError::Closed);
            }
            match self.control_result_rx.recv().await {
                Ok(event) => {
                    if let Some(event) = self.accept_live_control_result(event) {
                        return Ok(event);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!(
                        skipped_wakeups = count,
                        "observer terminal-result receiver lagged; replaying retained sequences"
                    );
                    self.queue_retained_control_results();
                }
                Err(broadcast::error::RecvError::Closed) => {
                    self.control_result_closed = true;
                    self.queue_retained_control_results();
                }
            }
        }
    }

    /// Receive the next observer event, draining terminal control results first.
    pub async fn recv(&mut self) -> Result<ObserverEvent, broadcast::error::RecvError> {
        loop {
            if let Some(event) = self.pop_retained_control_result() {
                return Ok(event);
            }
            match (self.control_result_closed, self.telemetry_closed) {
                (true, true) => {
                    self.queue_retained_control_results();
                    return self
                        .pop_retained_control_result()
                        .ok_or(broadcast::error::RecvError::Closed);
                }
                (false, true) => return self.recv_control_result().await,
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
                                Ok(event) => {
                                    if let Some(event) = self.accept_live_control_result(event) {
                                        return Ok(event);
                                    }
                                }
                                Err(broadcast::error::RecvError::Lagged(count)) => {
                                    tracing::warn!(
                                        skipped_wakeups = count,
                                        "observer terminal-result receiver lagged; replaying retained sequences"
                                    );
                                    self.queue_retained_control_results();
                                    if let Some(event) = self.pop_retained_control_result() {
                                        return Ok(event);
                                    }
                                }
                                Err(broadcast::error::RecvError::Closed) => {
                                    self.control_result_closed = true;
                                    self.queue_retained_control_results();
                                    if let Some(event) = self.pop_retained_control_result() {
                                        return Ok(event);
                                    }
                                }
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
            control_results: Arc::clone(&self.inner.control_results),
            control_result_rejected: Arc::clone(&self.inner.control_result_rejected),
            control_result_replay: VecDeque::new(),
            in_flight_control_results: HashSet::new(),
            control_result_failures: HashMap::new(),
            control_result_closed: false,
            telemetry_closed: false,
        }
    }

    /// Return the current replay buffers with terminal control results first.
    ///
    /// Separate storage prevents a telemetry flood from evicting a result
    /// before a newly started publisher snapshots the observer.
    pub fn snapshot(&self) -> Vec<ObserverEvent> {
        let mut snapshot = match self.inner.control_results.lock() {
            Ok(results) => results.values().cloned().collect::<Vec<_>>(),
            Err(error) => {
                tracing::warn!(
                    target: "observer",
                    "observer control-result ledger lock poisoned; recovering snapshot"
                );
                error.into_inner().values().cloned().collect()
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
            let admitted = match self.inner.control_results.lock() {
                Ok(mut results) => {
                    if results.len() >= OBSERVER_CONTROL_RESULT_LEDGER_CAP {
                        false
                    } else {
                        results.insert(event.seq, event.clone());
                        true
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        target: "observer",
                        "observer control-result ledger lock poisoned; recovering emission"
                    );
                    let mut results = error.into_inner();
                    if results.len() >= OBSERVER_CONTROL_RESULT_LEDGER_CAP {
                        false
                    } else {
                        results.insert(event.seq, event.clone());
                        true
                    }
                }
            };
            if !admitted {
                let rejected_total = self
                    .inner
                    .control_result_rejected
                    .fetch_add(1, Ordering::AcqRel)
                    .saturating_add(1);
                tracing::error!(
                    target: "observer",
                    seq = event.seq,
                    rejected_total,
                    capacity = OBSERVER_CONTROL_RESULT_LEDGER_CAP,
                    "observer terminal-result ledger full; rejected newest result and invalidated delivery proof"
                );
                return;
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

    #[tokio::test]
    async fn control_result_lag_replays_every_unacknowledged_sequence() {
        let observer = ObserverHandle::in_process();
        let mut rx = observer.subscribe();
        let burst = OBSERVER_CONTROL_RESULT_CAP + 17;
        for marker in 0..burst {
            emit_kind(&observer, "control_result", marker);
        }

        let mut delivered = Vec::new();
        for _ in 0..burst {
            let event = rx
                .recv_control_result()
                .await
                .expect("lag recovery must replay every retained result");
            delivered.push(event.payload["marker"].as_u64().unwrap() as usize);
        }

        assert_eq!(
            delivered,
            (0..burst).collect::<Vec<_>>(),
            "lag recovery must preserve every accepted terminal result exactly once"
        );
    }

    #[tokio::test]
    async fn control_result_remains_retained_until_explicit_acknowledgement() {
        let observer = ObserverHandle::in_process();
        let mut rx = observer.subscribe();
        emit_kind(&observer, "control_result", 23);

        let delivered = rx
            .recv_control_result()
            .await
            .expect("terminal result must reach its consumer");
        assert!(
            observer
                .snapshot()
                .iter()
                .any(|event| event.seq == delivered.seq),
            "consumer receipt alone must not evict an unacknowledged result"
        );

        rx.acknowledge_control_result(delivered.seq);
        assert!(
            observer
                .snapshot()
                .iter()
                .all(|event| event.seq != delivered.seq),
            "explicit acknowledgement is the terminal ledger removal point"
        );
    }

    #[tokio::test]
    async fn failed_control_result_delivery_is_released_for_one_bounded_retry() {
        let observer = ObserverHandle::in_process();
        let mut rx = observer.subscribe();
        emit_kind(&observer, "control_result", 29);

        let first = rx
            .recv_control_result()
            .await
            .expect("terminal result must reach its consumer");
        assert!(
            rx.retry_control_result(first.seq),
            "the first failed delivery must be retained and scheduled once"
        );

        let retry = rx
            .recv_control_result()
            .await
            .expect("released terminal result must replay");
        assert_eq!(retry.seq, first.seq);
        assert_eq!(retry.payload["marker"], 29);
        assert!(
            !rx.retry_control_result(retry.seq),
            "a second failed attempt must exhaust the bounded retry contract"
        );

        rx.acknowledge_control_result(retry.seq);
        assert!(
            observer
                .snapshot()
                .iter()
                .all(|event| event.seq != retry.seq),
            "a later confirmed delivery must clear retained and retry state"
        );
    }

    #[test]
    fn control_result_retention_is_bounded_and_rejection_is_observable() {
        const EXPECTED_TERMINAL_LEDGER_CAP: usize = 1_024;
        let observer = ObserverHandle::in_process();
        let rx = observer.subscribe();
        for marker in 0..=EXPECTED_TERMINAL_LEDGER_CAP {
            emit_kind(&observer, "control_result", marker);
        }

        assert_eq!(
            observer
                .snapshot()
                .iter()
                .filter(|event| event.kind == "control_result")
                .count(),
            EXPECTED_TERMINAL_LEDGER_CAP,
            "terminal retention must reject newest work instead of growing without bound"
        );
        assert!(
            rx.control_result_admission_failed(),
            "a rejected terminal result must make delivery verification fail closed"
        );
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
