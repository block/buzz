//! Multi-node fan-out: deliver events other relay nodes published to Redis to
//! this node's local subscribers.
//!
//! One dispatcher reads the pub/sub broadcast and only routes. Delivery runs on
//! [`WORKERS`] workers, each with a bounded queue. Every event of one
//! `(community, topic)` goes to the same worker, so each channel's events keep
//! their order while different channels are delivered concurrently.
//!
//! Live delivery never loses an event silently. When a worker's queue is full,
//! the dispatcher drops that event, ends the syncs it would have reached with
//! `CLOSED "resync: ..."` (NIP-AR Sync rule 5), and closes the connections it
//! would have reached so their clients catch up from their cursor. When the
//! dispatcher itself lags the broadcast, it cannot tell which events it
//! skipped, so it does the same for every subscribed connection. Closes go
//! through one paced closer so a hot channel cannot turn into a reconnect storm
//! (Review-Proven Rule 9).
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use buzz_pubsub::ChannelEvent;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{broadcast, mpsc, Notify};
use tracing::warn;

use super::event::{deliver_pubsub_event, pubsub_stored_event, take_local_echo};
use crate::state::AppState;
use crate::subscription::ConnId;

/// Delivery workers per node.
pub const WORKERS: usize = 16;
/// Events each worker may hold before the dispatcher drops new ones.
pub const WORKER_QUEUE: usize = 1024;
/// Connections the closer closes per tick.
const CLOSE_BATCH: usize = 50;
/// The closer's tick: with [`CLOSE_BATCH`], at most 5,000 closes per second.
const CLOSE_TICK: Duration = Duration::from_millis(10);
/// The resync reason for a sync whose live entry was lost here.
const LAG_RESYNC: &str = "resync: live delivery lagged";

/// Start multi-node fan-out for `rx`: the dispatcher, its workers, and the
/// closer. The returned handle ends when the broadcast closes.
pub fn spawn(
    state: Arc<AppState>,
    rx: broadcast::Receiver<ChannelEvent>,
) -> tokio::task::JoinHandle<()> {
    let (dispatcher, queues) = Dispatcher::new(WORKERS, WORKER_QUEUE);
    for queue in queues {
        tokio::spawn(run_worker(Arc::clone(&state), queue));
    }
    tokio::spawn(Arc::clone(&dispatcher.closer).run(Arc::clone(&state)));
    tokio::spawn(dispatcher.run(state, rx))
}

/// Delivers one worker's queue in order.
async fn run_worker(state: Arc<AppState>, mut queue: mpsc::Receiver<ChannelEvent>) {
    while let Some(channel_event) = queue.recv().await {
        deliver_pubsub_event(&state, channel_event).await;
    }
}

/// Routes broadcast events to worker queues and handles every loss.
struct Dispatcher {
    queues: Vec<mpsc::Sender<ChannelEvent>>,
    closer: Arc<LagCloser>,
}

impl Dispatcher {
    /// A dispatcher over `workers` queues of `capacity` events, and the
    /// receiving ends for the workers.
    fn new(workers: usize, capacity: usize) -> (Self, Vec<mpsc::Receiver<ChannelEvent>>) {
        let (queues, receivers) = (0..workers.max(1))
            .map(|_| mpsc::channel(capacity.max(1)))
            .unzip();
        let dispatcher = Self {
            queues,
            closer: Arc::new(LagCloser::default()),
        };
        (dispatcher, receivers)
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

    /// Queue one event on its worker, or handle it as lost when that worker
    /// is full. A local echo is no loss, so it is dropped before queueing.
    fn dispatch(&self, state: &AppState, channel_event: ChannelEvent) {
        if take_local_echo(state, &channel_event) {
            return;
        }
        let queue = &self.queues[self.worker_for(&channel_event)];
        match queue.try_send(channel_event) {
            Ok(()) => {}
            Err(TrySendError::Full(channel_event) | TrySendError::Closed(channel_event)) => {
                self.dropped(state, channel_event);
            }
        }
    }

    fn worker_for(&self, channel_event: &ChannelEvent) -> usize {
        let mut hasher = DefaultHasher::new();
        (channel_event.community_id, channel_event.topic).hash(&mut hasher);
        (hasher.finish() % self.queues.len() as u64) as usize
    }

    /// One known event was not delivered: resync and close what it would
    /// have reached. Matching skips the access gate, so a recipient that would
    /// have been filtered out may be closed too; that costs a reconnect, never
    /// a leak.
    fn dropped(&self, state: &AppState, channel_event: ChannelEvent) {
        metrics::counter!("buzz_multinode_fanout_dropped_total").increment(1);
        let community_id = channel_event.community_id;
        let stored = pubsub_stored_event(channel_event);
        let matches = state.sub_registry.fan_out_scoped(community_id, &stored);
        state.artifact_syncs.resync_matches(&matches, LAG_RESYNC);
        self.closer
            .queue(matches.into_iter().map(|(conn_id, _)| conn_id));
    }

    /// The dispatcher skipped `skipped` events it never saw. Any subscription
    /// may have missed one, so every sync resyncs and every subscribed
    /// connection closes.
    fn lagged(&self, state: &AppState, skipped: u64) {
        metrics::counter!("buzz_multinode_fanout_lag_total").increment(skipped);
        warn!("Multi-node fan-out lagged by {skipped} messages");
        state.artifact_syncs.resync_all(LAG_RESYNC);
        self.closer
            .queue(state.sub_registry.subscribed_connections());
    }
}

/// Closes connections that lost a live event, at most [`CLOSE_BATCH`] per
/// [`CLOSE_TICK`]. The pending set holds each connection once and never more
/// than this node's connections.
#[derive(Default)]
struct LagCloser {
    pending: Mutex<HashSet<ConnId>>,
    wake: Notify,
}

impl LagCloser {
    fn queue(&self, conns: impl IntoIterator<Item = ConnId>) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let before = pending.len();
        pending.extend(conns);
        if pending.len() > before {
            self.wake.notify_one();
        }
    }

    /// Take up to [`CLOSE_BATCH`] pending connections.
    fn take_batch(&self) -> Vec<ConnId> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let batch: Vec<ConnId> = pending.iter().take(CLOSE_BATCH).copied().collect();
        for conn_id in &batch {
            pending.remove(conn_id);
        }
        batch
    }

    async fn run(self: Arc<Self>, state: Arc<AppState>) {
        loop {
            self.wake.notified().await;
            let mut closed = 0usize;
            loop {
                let batch = self.take_batch();
                if batch.is_empty() {
                    break;
                }
                for conn_id in &batch {
                    state.conn_manager.cancel_conn(*conn_id);
                }
                closed += batch.len();
                tokio::time::sleep(CLOSE_TICK).await;
            }
            metrics::counter!("buzz_multinode_fanout_lag_closed_total").increment(closed as u64);
            warn!(
                closed,
                "multi-node fan-out: closed connections that missed live events"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU8;

    use axum::extract::ws::Message;
    use buzz_core::kind::KIND_PRESENCE_UPDATE;
    use buzz_core::tenant::CommunityId;
    use buzz_pubsub::EventTopic;
    use nostr::{EventBuilder, Filter, Keys, Kind};
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use super::*;

    fn community() -> CommunityId {
        CommunityId::from_uuid(Uuid::nil())
    }

    /// A connection with one presence subscription, its frame receiver, and
    /// its cancellation token.
    fn presence_subscriber(
        state: &AppState,
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
            "presence".to_string(),
            vec![Filter::new().kind(Kind::Custom(KIND_PRESENCE_UPDATE as u16))],
            None,
        );
        (conn_id, rx, cancel)
    }

    fn presence(n: usize) -> ChannelEvent {
        ChannelEvent {
            community_id: community(),
            topic: EventTopic::Global,
            event: EventBuilder::new(Kind::Custom(KIND_PRESENCE_UPDATE as u16), n.to_string())
                .sign_with_keys(&Keys::generate())
                .expect("sign presence"),
        }
    }

    fn pending(dispatcher: &Dispatcher) -> HashSet<ConnId> {
        dispatcher.closer.pending.lock().expect("pending").clone()
    }

    /// Events reach their subscriber through the workers, in publish order.
    #[tokio::test]
    async fn delivers_one_topic_in_order_through_workers() {
        let state = crate::state::tests::test_state().await;
        let (_conn, mut frames, cancel) = presence_subscriber(&state);
        let (tx, rx) = broadcast::channel(64);
        let handle = spawn(Arc::clone(&state), rx);

        let sent: Vec<_> = (0..20).map(presence).collect();
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
        assert!(!cancel.is_cancelled(), "no loss, no close");
        drop(tx);
        handle
            .await
            .expect("dispatcher ends when the broadcast closes");
    }

    /// A full worker queue drops the event and closes exactly the connections
    /// it would have reached.
    #[tokio::test]
    async fn full_worker_queue_closes_the_dropped_events_recipients() {
        let state = crate::state::tests::test_state().await;
        let (reached, _frames, _cancel) = presence_subscriber(&state);
        let other = Uuid::new_v4();
        state.sub_registry.register(
            other,
            "other-kind".to_string(),
            vec![Filter::new().kind(Kind::Custom(1))],
            None,
        );
        // Hold the only queue's receiver without draining it.
        let (dispatcher, _queues) = Dispatcher::new(1, 1);

        dispatcher.dispatch(&state, presence(0));
        assert!(pending(&dispatcher).is_empty(), "the first event fits");
        dispatcher.dispatch(&state, presence(1));
        assert_eq!(pending(&dispatcher), HashSet::from([reached]));
    }

    /// A local echo arriving on a full queue was already delivered, so it is
    /// not a loss.
    #[tokio::test]
    async fn local_echo_on_a_full_queue_closes_nothing() {
        let state = crate::state::tests::test_state().await;
        let _subscriber = presence_subscriber(&state);
        let (dispatcher, _queues) = Dispatcher::new(1, 1);
        dispatcher.dispatch(&state, presence(0));

        let echo = presence(1);
        state.mark_local_event(echo.community_id, &echo.event.id);
        dispatcher.dispatch(&state, echo);
        assert!(pending(&dispatcher).is_empty());
    }

    /// An unknown loss closes every subscribed connection, and only those.
    #[tokio::test]
    async fn dispatcher_lag_closes_every_subscribed_connection() {
        let state = crate::state::tests::test_state().await;
        let (first, _f1, _c1) = presence_subscriber(&state);
        let (second, _f2, _c2) = presence_subscriber(&state);
        let (dispatcher, _queues) = Dispatcher::new(1, 1);

        dispatcher.lagged(&state, 7);
        assert_eq!(pending(&dispatcher), HashSet::from([first, second]));
    }

    /// The closer cancels queued connections, at most one batch per tick.
    #[tokio::test(start_paused = true)]
    async fn closer_cancels_queued_connections_in_paced_batches() {
        let state = crate::state::tests::test_state().await;
        let subscribers: Vec<_> = (0..CLOSE_BATCH + 1)
            .map(|_| presence_subscriber(&state))
            .collect();
        let closer = Arc::new(LagCloser::default());
        tokio::spawn(Arc::clone(&closer).run(Arc::clone(&state)));

        closer.queue(subscribers.iter().map(|(conn_id, _, _)| *conn_id));
        let cancelled = || {
            subscribers
                .iter()
                .filter(|(_, _, cancel)| cancel.is_cancelled())
                .count()
        };
        tokio::time::sleep(CLOSE_TICK / 2).await;
        assert_eq!(cancelled(), CLOSE_BATCH, "one batch before the first tick");
        tokio::time::sleep(CLOSE_TICK).await;
        assert_eq!(cancelled(), CLOSE_BATCH + 1, "the rest after one tick");
    }
}
