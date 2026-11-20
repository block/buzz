//! Multi-node fan-out: deliver events other relay nodes published to Redis to
//! this node's local subscribers.
//!
//! One dispatcher reads the pub/sub broadcast and only routes. Delivery runs on
//! [`WORKERS`] workers, each with a bounded queue. Every event of one
//! `(community, topic)` goes to the same worker, so each channel's events keep
//! their order while different channels are delivered concurrently.
//!
//! A slow delivery holds only its own worker and is delivered late. Falling
//! behind never closes a connection: a database stall slows every node's
//! deliveries at once, and closing their recipients then would send the whole
//! fleet back to that database at the same moment (Review-Proven Rule 9).
//!
//! An event is dropped on this node when its worker's queue is full, when one
//! delivery attempt runs past [`MAX_DELIVERY_ATTEMPT`] (a database connection
//! that stops answering would otherwise hold its worker forever), or when the
//! dispatcher lags the broadcast and skips it. Both are counted, and the
//! syncs the dropped events would have reached get `CLOSED "resync: ..."`
//! (NIP-AR Sync rule 5). A plain subscription is not told. Its client may
//! replay the event when it next reconnects, but only if later deliveries did
//! not move its resume cursor past the event by more than the client's replay
//! slack.
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use buzz_core::event::StoredEvent;
use buzz_core::CommunityId;
use buzz_pubsub::ChannelEvent;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{broadcast, mpsc};
use tracing::warn;

use super::event::{deliver_pubsub_event, pubsub_stored_event, take_local_echo};
use crate::state::AppState;

/// Delivery workers per node.
pub const WORKERS: usize = 16;
/// Events each worker may hold before the dispatcher drops new ones.
pub const WORKER_QUEUE: usize = 1024;
/// How long a worker spends on one delivery before dropping it. Measured
/// from when the worker starts the delivery, so time spent waiting in the
/// queue never counts: a stall delays a backlog instead of dropping it. The
/// bound only cuts off a delivery that hangs, such as an access-gate read on
/// a database connection that stopped answering.
pub const MAX_DELIVERY_ATTEMPT: Duration = Duration::from_secs(30);
/// The resync reason for a sync whose live entry was dropped here.
const LAG_RESYNC: &str = "resync: live delivery lagged";

/// An event waiting for its worker.
type Queued = (CommunityId, StoredEvent);

/// Start multi-node fan-out for `rx`: the dispatcher and its workers. The
/// returned handle ends when the broadcast closes.
pub fn spawn(
    state: Arc<AppState>,
    rx: broadcast::Receiver<ChannelEvent>,
) -> tokio::task::JoinHandle<()> {
    let (dispatcher, queues) = Dispatcher::new(WORKERS, WORKER_QUEUE);
    for queue in queues {
        tokio::spawn(run_worker(Arc::clone(&state), queue));
    }
    tokio::spawn(dispatcher.run(state, rx))
}

/// Delivers one worker's queue in order.
async fn run_worker(state: Arc<AppState>, mut queue: mpsc::Receiver<Queued>) {
    while let Some((community_id, stored)) = queue.recv().await {
        let delivery = deliver_pubsub_event(&state, community_id, &stored);
        if tokio::time::timeout(MAX_DELIVERY_ATTEMPT, delivery)
            .await
            .is_err()
        {
            drop_event(&state, community_id, &stored, "stale");
        }
    }
}

/// Count a dropped event and resync the syncs it would have reached. A
/// delivery cut off partway may have reached some of them; a resync costs
/// them a re-read, never a gap. No connection is closed.
fn drop_event(
    state: &AppState,
    community_id: CommunityId,
    stored: &StoredEvent,
    reason: &'static str,
) {
    metrics::counter!("buzz_multinode_fanout_dropped_total", "reason" => reason).increment(1);
    let matches = state.sub_registry.fan_out_scoped(community_id, stored);
    state.artifact_syncs.resync_matches(&matches, LAG_RESYNC);
}

/// Routes broadcast events to worker queues.
struct Dispatcher {
    queues: Vec<mpsc::Sender<Queued>>,
}

impl Dispatcher {
    /// A dispatcher over `workers` queues of `capacity` events, and the
    /// receiving ends for the workers.
    fn new(workers: usize, capacity: usize) -> (Self, Vec<mpsc::Receiver<Queued>>) {
        let (queues, receivers) = (0..workers.max(1))
            .map(|_| mpsc::channel(capacity.max(1)))
            .unzip();
        (Self { queues }, receivers)
    }

    async fn run(self, state: Arc<AppState>, mut rx: broadcast::Receiver<ChannelEvent>) {
        loop {
            match rx.recv().await {
                Ok(channel_event) => self.dispatch(&state, channel_event),
                Err(broadcast::error::RecvError::Lagged(skipped)) => self.lagged(&state, skipped),
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::error!("Multi-node fan-out broadcast channel closed");
                    return;
                }
            }
        }
    }

    /// Queue one event on its worker, or drop it when that worker is full. A
    /// local echo was already delivered, so it is dropped before queueing.
    fn dispatch(&self, state: &AppState, channel_event: ChannelEvent) {
        if take_local_echo(state, &channel_event) {
            return;
        }
        let queue = &self.queues[self.worker_for(&channel_event)];
        let queued = (
            channel_event.community_id,
            pubsub_stored_event(channel_event),
        );
        if let Err(
            TrySendError::Full((community_id, stored))
            | TrySendError::Closed((community_id, stored)),
        ) = queue.try_send(queued)
        {
            drop_event(state, community_id, &stored, "queue_full");
        }
    }

    fn worker_for(&self, channel_event: &ChannelEvent) -> usize {
        let mut hasher = DefaultHasher::new();
        (channel_event.community_id, channel_event.topic).hash(&mut hasher);
        (hasher.finish() % self.queues.len() as u64) as usize
    }

    /// The dispatcher skipped `skipped` events it never saw. Any sync may have
    /// missed one, so every sync resyncs.
    fn lagged(&self, state: &AppState, skipped: u64) {
        metrics::counter!("buzz_multinode_fanout_lag_total").increment(skipped);
        warn!("Multi-node fan-out lagged by {skipped} messages");
        state.artifact_syncs.resync_all(LAG_RESYNC);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU8;

    use axum::extract::ws::Message;
    use buzz_core::tenant::CommunityId;
    use buzz_pubsub::EventTopic;
    use nostr::{EventBuilder, Filter, Keys, Kind};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use std::time::Duration;

    use super::*;
    use crate::subscription::ConnId;

    fn community() -> CommunityId {
        CommunityId::from_uuid(Uuid::nil())
    }

    const STORED: u16 = 1;

    /// A connection with one subscription to `kind`, its frame receiver, and
    /// its cancellation token.
    fn subscriber(
        state: &AppState,
        kind: u16,
    ) -> (ConnId, mpsc::Receiver<Message>, CancellationToken) {
        let conn_id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(64);
        let cancel = CancellationToken::new();
        state.conn_manager.register(
            conn_id,
            tx,
            mpsc::channel(10).0,
            mpsc::channel(1).0,
            None,
            cancel.clone(),
            community(),
            Arc::new(AtomicU8::new(0)),
            Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            3,
            crate::state::CommunityConnectionControl::new(CancellationToken::new()),
        );
        state.sub_registry.register(
            conn_id,
            "sub".to_string(),
            vec![Filter::new().kind(Kind::Custom(kind))],
            None,
        );
        (conn_id, rx, cancel)
    }

    fn event(kind: u16, n: usize) -> ChannelEvent {
        ChannelEvent {
            community_id: community(),
            topic: EventTopic::Global,
            event: EventBuilder::new(Kind::Custom(kind), n.to_string())
                .sign_with_keys(&Keys::generate())
                .expect("sign event"),
        }
    }

    /// Events reach their subscriber through the workers, in publish order.
    #[tokio::test]
    async fn delivers_one_topic_in_order_through_workers() {
        let state = crate::state::tests::test_state().await;
        let (_conn, mut frames, cancel) = subscriber(&state, STORED);
        let (tx, rx) = broadcast::channel(64);
        let handle = spawn(Arc::clone(&state), rx);

        let sent: Vec<_> = (0..20).map(|n| event(STORED, n)).collect();
        for event in &sent {
            tx.send(event.clone()).expect("broadcast");
        }
        for event in &sent {
            let frame = tokio::time::timeout(Duration::from_secs(5), frames.recv())
                .await
                .expect("delivered in time")
                .expect("frame");
            let Message::Text(text) = frame else {
                panic!("expected an EVENT frame");
            };
            let v: serde_json::Value = serde_json::from_str(&text).expect("frame JSON");
            assert_eq!(v[2]["id"], event.event.id.to_hex());
        }
        assert!(!cancel.is_cancelled());
        drop(tx);
        handle
            .await
            .expect("dispatcher ends when the broadcast closes");
    }

    /// A connection with an idle sync on `kind`: the sync is what a dropped
    /// event must resync.
    fn sync(state: &AppState, kind: u16) -> (ConnId, CancellationToken) {
        let (conn_id, _frames, cancel) = subscriber(state, kind);
        state.artifact_syncs.insert_idle(conn_id, "sub");
        (conn_id, cancel)
    }

    /// How many events were counted as dropped for `reason`.
    fn dropped(snapshotter: &metrics_util::debugging::Snapshotter, reason: &str) -> u64 {
        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, ..)| {
                key.key().name() == "buzz_multinode_fanout_dropped_total"
                    && key.key().labels().any(|label| label.value() == reason)
            })
            .map(|(.., value)| match value {
                metrics_util::debugging::DebugValue::Counter(n) => n,
                other => panic!("expected a counter, got {other:?}"),
            })
            .sum()
    }

    /// A full worker queue drops the event: it is counted, the syncs it would
    /// have reached resync, and no connection closes. A local echo arriving on
    /// the full queue was already delivered, so it is no drop.
    #[tokio::test]
    async fn full_worker_queue_resyncs_the_dropped_events_syncs_and_closes_nothing() {
        let state = crate::state::tests::test_state().await;
        let (_plain, _frames, plain_cancel) = subscriber(&state, STORED);
        let (synced, sync_cancel) = sync(&state, STORED);
        let (unmatched, _) = sync(&state, 7);
        // Hold the only queue's receiver without draining it.
        let (dispatcher, _queues) = Dispatcher::new(1, 1);
        dispatcher.dispatch(&state, event(STORED, 0));

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            let echo = event(STORED, 1);
            state.mark_local_event(echo.community_id, &echo.event.id);
            dispatcher.dispatch(&state, echo);
        });
        assert_eq!(dropped(&snapshotter, "queue_full"), 0, "an echo is no drop");
        assert_eq!(state.artifact_syncs.pending_resync(synced, "sub"), None);

        metrics::with_local_recorder(&recorder, || dispatcher.dispatch(&state, event(STORED, 2)));
        assert_eq!(dropped(&snapshotter, "queue_full"), 1);
        assert_eq!(
            state.artifact_syncs.pending_resync(synced, "sub"),
            Some(LAG_RESYNC)
        );
        assert_eq!(
            state.artifact_syncs.pending_resync(unmatched, "sub"),
            None,
            "only the syncs the event would have reached"
        );
        assert!(!plain_cancel.is_cancelled() && !sync_cancel.is_cancelled());
    }

    /// Dispatcher lag resyncs every sync and closes no connection.
    #[tokio::test]
    async fn dispatcher_lag_resyncs_every_sync_and_closes_nothing() {
        let state = crate::state::tests::test_state().await;
        let (_plain, _frames, plain_cancel) = subscriber(&state, STORED);
        let (first, first_cancel) = sync(&state, STORED);
        let (second, second_cancel) = sync(&state, 7);
        let (dispatcher, _queues) = Dispatcher::new(1, 1);

        dispatcher.lagged(&state, 7);
        for conn in [first, second] {
            assert_eq!(
                state.artifact_syncs.pending_resync(conn, "sub"),
                Some(LAG_RESYNC)
            );
        }
        assert!(
            !plain_cancel.is_cancelled()
                && !first_cancel.is_cancelled()
                && !second_cancel.is_cancelled()
        );
    }

    /// A recipient the access gate cannot check (every database read fails)
    /// misses the event without a retry and stays connected.
    #[tokio::test(start_paused = true)]
    async fn recipient_the_gate_cannot_check_misses_the_event_and_stays_connected() {
        let state = crate::state::tests::test_state().await;
        state.db.pool().close().await;
        let channel_id = Uuid::new_v4();
        state
            .channel_visibility_cache
            .insert((community(), channel_id), "private".to_string());
        let (conn, mut frames, cancel) = subscriber(&state, STORED);
        state
            .conn_manager
            .set_authenticated_pubkey(conn, vec![7; 32]);
        state.sub_registry.register(
            conn,
            "channel".to_string(),
            vec![Filter::new().kind(Kind::Custom(STORED))],
            Some(channel_id),
        );
        let in_channel = ChannelEvent {
            topic: EventTopic::Channel(channel_id),
            ..event(STORED, 0)
        };

        let started = tokio::time::Instant::now();
        deliver_pubsub_event(&state, community(), &pubsub_stored_event(in_channel)).await;
        assert_eq!(started.elapsed(), Duration::ZERO, "no retry");
        assert!(frames.try_recv().is_err(), "the gate let nothing through");
        assert!(!cancel.is_cancelled());

        let open = event(STORED, 1);
        let id = open.event.id.to_hex();
        deliver_pubsub_event(&state, community(), &pubsub_stored_event(open)).await;
        let Ok(Message::Text(text)) = frames.try_recv() else {
            panic!("expected the open event");
        };
        let v: serde_json::Value = serde_json::from_str(&text).expect("frame JSON");
        assert_eq!(v[2]["id"], id);
    }

    /// A delivery that hangs (here on a database that accepts the connection
    /// but never answers) is cut off at the bound: it is counted, the sync it
    /// matched resyncs, no connection closes, and the next event queued on
    /// the same worker is delivered.
    #[tokio::test(start_paused = true)]
    async fn hung_delivery_is_dropped_at_the_bound_and_the_worker_moves_on() {
        let silent_db = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!(
            "postgres://buzz@{}/buzz",
            silent_db.local_addr().expect("addr")
        );
        let state = crate::state::tests::test_state_with_database_url_and_pool_options(
            &url,
            sqlx::postgres::PgPoolOptions::new().acquire_timeout(Duration::from_secs(3600)),
        )
        .await;
        let channel_id = Uuid::new_v4();
        let (conn, mut frames, cancel) = subscriber(&state, STORED);
        state.sub_registry.register(
            conn,
            "channel".to_string(),
            vec![Filter::new().kind(Kind::Custom(STORED))],
            Some(channel_id),
        );
        state.artifact_syncs.insert_idle(conn, "channel");
        let hangs = ChannelEvent {
            topic: EventTopic::Channel(channel_id),
            ..event(STORED, 0)
        };
        let next = event(STORED, 1);
        let next_id = next.event.id.to_hex();

        let (dispatcher, mut queues) = Dispatcher::new(1, 8);
        dispatcher.dispatch(&state, hangs);
        dispatcher.dispatch(&state, next);
        drop(dispatcher);

        // The worker drains both events, then ends because the queue closed.
        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let started = tokio::time::Instant::now();
        WithRecorder {
            recorder: &recorder,
            future: Box::pin(run_worker(Arc::clone(&state), queues.pop().expect("queue"))),
        }
        .await;
        assert_eq!(
            started.elapsed(),
            MAX_DELIVERY_ATTEMPT,
            "cut off at the bound"
        );
        let Ok(Message::Text(text)) = frames.try_recv() else {
            panic!("expected the next event");
        };
        let v: serde_json::Value = serde_json::from_str(&text).expect("frame JSON");
        assert_eq!(v[2]["id"], next_id, "the hung event was not delivered");
        assert!(frames.try_recv().is_err());
        assert_eq!(dropped(&snapshotter, "stale"), 1);
        assert_eq!(
            state.artifact_syncs.pending_resync(conn, "channel"),
            Some(LAG_RESYNC)
        );
        assert!(!cancel.is_cancelled());
    }

    /// Polls `future` with `recorder` as the local metrics recorder, so a
    /// task's metrics are visible to the test that awaits it.
    struct WithRecorder<'a, F> {
        recorder: &'a metrics_util::debugging::DebuggingRecorder,
        future: std::pin::Pin<Box<F>>,
    }

    impl<F: std::future::Future> std::future::Future for WithRecorder<'_, F> {
        type Output = F::Output;

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<F::Output> {
            let this = &mut *self;
            metrics::with_local_recorder(this.recorder, || this.future.as_mut().poll(cx))
        }
    }
}
