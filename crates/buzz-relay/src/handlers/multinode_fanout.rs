//! Multi-node fan-out: deliver events other relay nodes published to Redis to
//! this node's local subscribers.
//!
//! One dispatcher reads the pub/sub broadcast and only routes. Delivery runs on
//! [`WORKERS`] workers, each with a bounded queue. Every event of one
//! `(community, topic)` goes to the same worker, so each channel's events keep
//! their order while different channels are delivered concurrently.
//!
//! Live delivery never loses an event silently. An event is lost when its
//! worker's queue is full, or when it is not delivered within
//! [`MAX_QUEUE_DELAY`] of this node reading it from Redis. Either way the syncs
//! it would have reached get `CLOSED "resync: ..."` (NIP-AR Sync rule 5), and
//! the connections it would have reached are fenced and closed so their
//! clients catch up from their cursor. When the dispatcher itself lags the
//! broadcast, it cannot tell which events it skipped, so it does the same for
//! every subscribed connection.
//!
//! An ephemeral event (presence, typing, observer frames) is never stored, so
//! no reconnect can replay it. Losing one is only counted: closing its
//! recipients would recover nothing, and a global presence event reaches most
//! of a community's connections.
//!
//! A client resumes from the newest `created_at` it received, minus a few
//! seconds of slack. Recovery depends on that cursor never getting further
//! ahead of a lost event than the slack:
//! - A fence is immediate and atomic with the live send path, so after it no
//!   newer frame is queued to the connection. Frames queued before it were
//!   produced no later than the fence.
//! - [`MAX_QUEUE_DELAY`] is the cutoff for attempting delivery: no event is
//!   delivered, and so none moves a cursor, later than that after this node
//!   read it from Redis. A worker that has not delivered an event by then
//!   fences its recipients at once.
//!
//! So a missed event is replayed as long as `MAX_QUEUE_DELAY`, plus its
//! writer's clock skew and its publish-to-Redis delay, stays under the
//! client's slack. Three cases fall outside that bound:
//! - The relay accepts `created_at` far outside the slack, and an ordinary
//!   reconnect has the same limit. Only a client-side rewind can recover an
//!   event whose writer's clock is further behind.
//! - A dispatcher that falls behind without overflowing the broadcast detects
//!   the loss late: the event is still never delivered, but its fence lands
//!   only when the dispatcher reaches it.
//! - Dispatcher lag, where the skipped events are never seen.
//!
//! The socket close itself goes through one paced closer so a hot channel
//! cannot turn into a reconnect storm (Review-Proven Rule 9).
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use buzz_core::kind::{event_kind_u32, is_ephemeral};
use buzz_core::CommunityId;
use buzz_pubsub::ChannelEvent;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{broadcast, mpsc, Notify};
use tokio::time::Instant;
use tracing::warn;

use super::event::{deliver_pubsub_event, pubsub_stored_event, take_local_echo};
use crate::state::AppState;
use crate::subscription::ConnId;
use buzz_core::event::StoredEvent;

/// Delivery workers per node.
pub const WORKERS: usize = 16;
/// Events each worker may hold before the dispatcher drops new ones.
pub const WORKER_QUEUE: usize = 1024;
/// The cutoff for delivering an event, from when this node read it from Redis.
/// An event not delivered by then counts as lost.
/// It must stay well under the clients' replay slack (5 s), so a client whose
/// cursor moved on through faster paths still replays what it missed.
pub const MAX_QUEUE_DELAY: Duration = Duration::from_secs(2);
/// Connections the closer closes per tick.
const CLOSE_BATCH: usize = 50;
/// The closer's tick: with [`CLOSE_BATCH`], at most 5,000 closes per second.
const CLOSE_TICK: Duration = Duration::from_millis(10);
/// The resync reason for a sync whose live entry was lost here.
const LAG_RESYNC: &str = "resync: live delivery lagged";

/// An event waiting for its worker, and when it counts as lost: its deadline
/// runs from when this node read it from Redis, not from dispatch.
struct Queued {
    community_id: CommunityId,
    stored: StoredEvent,
    deadline: Instant,
}

impl Queued {
    fn new(channel_event: ChannelEvent) -> Self {
        Self {
            community_id: channel_event.community_id,
            deadline: channel_event.received_at + MAX_QUEUE_DELAY,
            stored: pubsub_stored_event(channel_event),
        }
    }
}

/// Start multi-node fan-out for `rx`: the dispatcher, its workers, and the
/// closer. The returned handle ends when the broadcast closes.
pub fn spawn(
    state: Arc<AppState>,
    rx: broadcast::Receiver<ChannelEvent>,
) -> tokio::task::JoinHandle<()> {
    let (dispatcher, queues) = Dispatcher::new(WORKERS, WORKER_QUEUE);
    for queue in queues {
        tokio::spawn(run_worker(
            Arc::clone(&state),
            Arc::clone(&dispatcher.closer),
            queue,
        ));
    }
    tokio::spawn(Arc::clone(&dispatcher.closer).run(Arc::clone(&state)));
    tokio::spawn(dispatcher.run(state, rx))
}

/// Delivers one worker's queue in order.
async fn run_worker(
    state: Arc<AppState>,
    closer: Arc<LagCloser>,
    mut queue: mpsc::Receiver<Queued>,
) {
    while let Some(queued) = queue.recv().await {
        deliver_or_lose(&state, &closer, queued).await;
    }
}

/// Deliver a queued event by its deadline, or handle it as lost. A delivery
/// cut off at the deadline may have reached some recipients; they close too.
async fn deliver_or_lose(state: &Arc<AppState>, closer: &LagCloser, queued: Queued) {
    let Queued {
        community_id,
        stored,
        deadline,
    } = queued;
    let delivered = Instant::now() < deadline
        && tokio::time::timeout_at(deadline, deliver_pubsub_event(state, community_id, &stored))
            .await
            .is_ok();
    if !delivered {
        lose(state, closer, community_id, &stored, "stale");
    }
}

/// One known event will not be delivered: resync the syncs and fence and
/// close the connections it would have reached. Matching skips the access
/// gate, so a recipient that would have been filtered out may be closed too;
/// that costs a reconnect, never a leak. A lost ephemeral event is only
/// counted, since no reconnect can replay it.
fn lose(
    state: &AppState,
    closer: &LagCloser,
    community_id: CommunityId,
    stored: &StoredEvent,
    reason: &'static str,
) {
    if is_ephemeral(event_kind_u32(&stored.event)) {
        metrics::counter!("buzz_multinode_fanout_dropped_total", "reason" => "ephemeral")
            .increment(1);
        return;
    }
    metrics::counter!("buzz_multinode_fanout_dropped_total", "reason" => reason).increment(1);
    let matches = state.sub_registry.fan_out_scoped(community_id, stored);
    state.artifact_syncs.resync_matches(&matches, LAG_RESYNC);
    closer.queue(state, matches.into_iter().map(|(conn_id, _)| conn_id));
}

/// Routes broadcast events to worker queues and handles every loss.
struct Dispatcher {
    queues: Vec<mpsc::Sender<Queued>>,
    closer: Arc<LagCloser>,
}

impl Dispatcher {
    /// A dispatcher over `workers` queues of `capacity` events, and the
    /// receiving ends for the workers.
    fn new(workers: usize, capacity: usize) -> (Self, Vec<mpsc::Receiver<Queued>>) {
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
        match queue.try_send(Queued::new(channel_event)) {
            Ok(()) => {}
            Err(TrySendError::Full(lost) | TrySendError::Closed(lost)) => {
                lose(
                    state,
                    &self.closer,
                    lost.community_id,
                    &lost.stored,
                    "queue_full",
                );
            }
        }
    }

    fn worker_for(&self, channel_event: &ChannelEvent) -> usize {
        let mut hasher = DefaultHasher::new();
        (channel_event.community_id, channel_event.topic).hash(&mut hasher);
        (hasher.finish() % self.queues.len() as u64) as usize
    }

    /// The dispatcher skipped `skipped` events it never saw. Any subscription
    /// may have missed one, so every sync resyncs and every subscribed
    /// connection is fenced and closes.
    fn lagged(&self, state: &AppState, skipped: u64) {
        metrics::counter!("buzz_multinode_fanout_lag_total").increment(skipped);
        warn!("Multi-node fan-out lagged by {skipped} messages");
        state.artifact_syncs.resync_all(LAG_RESYNC);
        self.closer
            .queue(state, state.sub_registry.subscribed_connections());
    }
}

/// Fences connections that lost a live event at once, then closes them at most
/// [`CLOSE_BATCH`] per [`CLOSE_TICK`]. The pending set holds each connection
/// once and never more than this node's connections.
#[derive(Default)]
struct LagCloser {
    pending: Mutex<HashSet<ConnId>>,
    wake: Notify,
}

impl LagCloser {
    /// Fence `conns` now and queue them for a paced close. Fencing happens
    /// before this returns, so no event dispatched after the loss reaches them.
    fn queue(&self, state: &AppState, conns: impl IntoIterator<Item = ConnId>) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let before = pending.len();
        for conn_id in conns {
            state.conn_manager.fence_live_delivery(conn_id);
            pending.insert(conn_id);
        }
        if pending.len() > before {
            self.wake.notify_one();
        }
    }

    /// Close up to [`CLOSE_BATCH`] pending connections, and count them.
    /// Returns how many it closed.
    fn close_batch(&self, state: &AppState) -> usize {
        let batch: Vec<ConnId> = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            let batch: Vec<ConnId> = pending.iter().take(CLOSE_BATCH).copied().collect();
            for conn_id in &batch {
                pending.remove(conn_id);
            }
            batch
        };
        for conn_id in &batch {
            state.conn_manager.cancel_conn(*conn_id);
        }
        metrics::counter!("buzz_multinode_fanout_lag_closed_total").increment(batch.len() as u64);
        batch.len()
    }

    async fn run(self: Arc<Self>, state: Arc<AppState>) {
        loop {
            self.wake.notified().await;
            let mut closed = 0usize;
            loop {
                let batch = self.close_batch(&state);
                if batch == 0 {
                    break;
                }
                closed += batch;
                tokio::time::sleep(CLOSE_TICK).await;
            }
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
    use crate::handlers::event::fan_out_event_to_local_subscribers;

    fn community() -> CommunityId {
        CommunityId::from_uuid(Uuid::nil())
    }

    /// A stored kind: a lost event of it can be replayed, so its loss closes.
    const STORED: u16 = 1;
    /// An ephemeral kind: a lost event of it cannot be replayed.
    const PRESENCE: u16 = KIND_PRESENCE_UPDATE as u16;

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
            received_at: Instant::now(),
        }
    }

    fn pending(dispatcher: &Dispatcher) -> HashSet<ConnId> {
        dispatcher.closer.pending.lock().expect("pending").clone()
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
        let (reached, _frames, _cancel) = subscriber(&state, STORED);
        let other = Uuid::new_v4();
        state.sub_registry.register(
            other,
            "other-kind".to_string(),
            vec![Filter::new().kind(Kind::Custom(7))],
            None,
        );
        // Hold the only queue's receiver without draining it.
        let (dispatcher, _queues) = Dispatcher::new(1, 1);

        dispatcher.dispatch(&state, event(STORED, 0));
        assert!(pending(&dispatcher).is_empty(), "the first event fits");
        dispatcher.dispatch(&state, event(STORED, 1));
        assert_eq!(pending(&dispatcher), HashSet::from([reached]));
    }

    /// A local echo arriving on a full queue was already delivered, so it is
    /// not a loss.
    #[tokio::test]
    async fn local_echo_on_a_full_queue_closes_nothing() {
        let state = crate::state::tests::test_state().await;
        let _subscriber = subscriber(&state, STORED);
        let (dispatcher, _queues) = Dispatcher::new(1, 1);
        dispatcher.dispatch(&state, event(STORED, 0));

        let echo = event(STORED, 1);
        state.mark_local_event(echo.community_id, &echo.event.id);
        dispatcher.dispatch(&state, echo);
        assert!(pending(&dispatcher).is_empty());
    }

    /// An unknown loss closes every subscribed connection, and only those.
    #[tokio::test]
    async fn dispatcher_lag_closes_every_subscribed_connection() {
        let state = crate::state::tests::test_state().await;
        let (first, _f1, _c1) = subscriber(&state, STORED);
        let (second, _f2, _c2) = subscriber(&state, STORED);
        let (dispatcher, _queues) = Dispatcher::new(1, 1);

        dispatcher.lagged(&state, 7);
        assert_eq!(pending(&dispatcher), HashSet::from([first, second]));
    }

    /// The closer cancels queued connections, at most one batch per tick.
    #[tokio::test(start_paused = true)]
    async fn closer_cancels_queued_connections_in_paced_batches() {
        let state = crate::state::tests::test_state().await;
        let subscribers: Vec<_> = (0..CLOSE_BATCH + 1)
            .map(|_| subscriber(&state, STORED))
            .collect();
        let closer = Arc::new(LagCloser::default());
        tokio::spawn(Arc::clone(&closer).run(Arc::clone(&state)));

        closer.queue(&state, subscribers.iter().map(|(conn_id, _, _)| *conn_id));
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

    /// Deliver `event` the way a worker does, and report whether the
    /// subscriber got a frame.
    async fn worker_delivers(
        state: &Arc<AppState>,
        closer: &LagCloser,
        frames: &mut mpsc::Receiver<Message>,
        event: ChannelEvent,
    ) -> bool {
        deliver_or_lose(state, closer, Queued::new(event)).await;
        frames.try_recv().is_ok()
    }

    /// After a known loss, a recipient gets no newer live event while it
    /// waits for its paced close, so its client cannot resume past the loss.
    #[tokio::test]
    async fn dropped_event_fences_its_recipients_before_their_close() {
        let state = crate::state::tests::test_state().await;
        let (_conn, mut frames, cancel) = subscriber(&state, STORED);
        let (dispatcher, _queues) = Dispatcher::new(1, 1);
        assert!(
            worker_delivers(&state, &dispatcher.closer, &mut frames, event(STORED, 0)).await,
            "delivers before any loss"
        );
        dispatcher.dispatch(&state, event(STORED, 1));
        dispatcher.dispatch(&state, event(STORED, 2)); // dropped

        assert!(
            !worker_delivers(&state, &dispatcher.closer, &mut frames, event(STORED, 3)).await,
            "an event after the loss must not reach the fenced connection"
        );
        assert!(
            !cancel.is_cancelled(),
            "the close itself waits for the closer"
        );
    }

    /// The same fence holds after an unknown loss.
    #[tokio::test]
    async fn dispatcher_lag_fences_every_subscribed_connection() {
        let state = crate::state::tests::test_state().await;
        let (_conn, mut frames, cancel) = subscriber(&state, STORED);
        let (dispatcher, _queues) = Dispatcher::new(1, 1);
        dispatcher.lagged(&state, 3);

        assert!(!worker_delivers(&state, &dispatcher.closer, &mut frames, event(STORED, 0)).await);
        assert!(!cancel.is_cancelled());
    }

    /// Gandalf's backlog case: R waits behind a backlog on its worker while
    /// local fan-out and other workers keep moving its recipient's cursor. R
    /// must reach the recipient by its deadline, or the recipient must be
    /// fenced and closed so its client replays R. It is never delivered late.
    #[tokio::test(start_paused = true)]
    async fn backlogged_event_is_delivered_by_its_deadline_or_lost() {
        let state = crate::state::tests::test_state().await;
        let (conn, mut frames, cancel) = subscriber(&state, STORED);
        let (dispatcher, mut queues) = Dispatcher::new(1, 4);
        let queue = &mut queues[0];

        dispatcher.dispatch(&state, event(STORED, 0));
        tokio::time::advance(MAX_QUEUE_DELAY - Duration::from_millis(1)).await;
        let in_time = queue.try_recv().expect("queued");
        deliver_or_lose(&state, &dispatcher.closer, in_time).await;
        assert!(frames.try_recv().is_ok(), "delivered inside its deadline");

        dispatcher.dispatch(&state, event(STORED, 1)); // R
        tokio::time::advance(MAX_QUEUE_DELAY).await;
        let late = queue.try_recv().expect("queued");
        deliver_or_lose(&state, &dispatcher.closer, late).await;
        assert!(frames.try_recv().is_err(), "R is not delivered late");
        assert_eq!(pending(&dispatcher), HashSet::from([conn]));
        assert!(
            !worker_delivers(&state, &dispatcher.closer, &mut frames, event(STORED, 2)).await,
            "R's recipient is fenced until it closes"
        );
        assert!(!cancel.is_cancelled(), "the close waits for the closer");
    }

    /// The deadline runs from Redis receipt: an event that already waited out
    /// its deadline in the broadcast before dispatch is lost, not delivered.
    #[tokio::test(start_paused = true)]
    async fn event_that_waited_before_dispatch_is_lost_not_delivered() {
        let state = crate::state::tests::test_state().await;
        let (conn, mut frames, _cancel) = subscriber(&state, STORED);
        let (dispatcher, mut queues) = Dispatcher::new(1, 4);

        let received = event(STORED, 0);
        tokio::time::advance(MAX_QUEUE_DELAY).await;
        dispatcher.dispatch(&state, received);
        let queued = queues[0].try_recv().expect("queued");
        deliver_or_lose(&state, &dispatcher.closer, queued).await;
        assert!(frames.try_recv().is_err(), "not delivered late");
        assert_eq!(pending(&dispatcher), HashSet::from([conn]));
    }

    /// A delivery that stalls (here on a database that accepts connections
    /// but never answers) is cut off at its deadline and handled as lost.
    #[tokio::test(start_paused = true)]
    async fn delivery_stalled_past_its_deadline_is_lost() {
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
        let (conn, mut frames, _cancel) = subscriber(&state, STORED);
        state.sub_registry.register(
            conn,
            "channel".to_string(),
            vec![Filter::new().kind(Kind::Custom(9))],
            Some(channel_id),
        );
        let event = ChannelEvent {
            community_id: community(),
            topic: EventTopic::Channel(channel_id),
            event: EventBuilder::new(Kind::Custom(9), "stalls")
                .sign_with_keys(&Keys::generate())
                .expect("sign"),
            received_at: Instant::now(),
        };
        let closer = LagCloser::default();

        let started = Instant::now();
        deliver_or_lose(&state, &closer, Queued::new(event)).await;
        assert_eq!(
            started.elapsed(),
            MAX_QUEUE_DELAY,
            "cut off at the deadline"
        );
        assert!(frames.try_recv().is_err());
        assert_eq!(
            closer.pending.lock().expect("pending").clone(),
            HashSet::from([conn])
        );
    }

    /// Each batch is counted when it closes, not when the backlog drains.
    #[tokio::test]
    async fn each_close_batch_is_counted_as_it_closes() {
        let state = crate::state::tests::test_state().await;
        let subscribers: Vec<_> = (0..CLOSE_BATCH + 1)
            .map(|_| subscriber(&state, STORED))
            .collect();
        let closer = LagCloser::default();
        closer.queue(&state, subscribers.iter().map(|(conn_id, _, _)| *conn_id));

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let closed = metrics::with_local_recorder(&recorder, || closer.close_batch(&state));
        assert_eq!(closed, CLOSE_BATCH);
        assert_eq!(closer.pending.lock().expect("pending").len(), 1);
        let counted: u64 = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, ..)| key.key().name() == "buzz_multinode_fanout_lag_closed_total")
            .map(|(.., value)| match value {
                metrics_util::debugging::DebugValue::Counter(n) => n,
                other => panic!("expected a counter, got {other:?}"),
            })
            .sum();
        assert_eq!(counted, CLOSE_BATCH as u64);
    }

    /// A lost ephemeral event cannot be replayed, so it is only counted: its
    /// recipients are neither fenced nor closed. A stored event lost on the
    /// same full queue still closes them.
    #[tokio::test(start_paused = true)]
    async fn lost_ephemeral_event_is_counted_and_closes_nothing() {
        let state = crate::state::tests::test_state().await;
        let (conn, mut frames, cancel) = subscriber(&state, PRESENCE);
        state.sub_registry.register(
            conn,
            "stored".to_string(),
            vec![Filter::new().kind(Kind::Custom(STORED))],
            None,
        );
        let (dispatcher, _queues) = Dispatcher::new(1, 1);
        dispatcher.dispatch(&state, event(PRESENCE, 0));

        let recorder = metrics_util::debugging::DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        metrics::with_local_recorder(&recorder, || {
            dispatcher.dispatch(&state, event(PRESENCE, 1)); // queue full
        });
        let reasons: Vec<String> = snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .filter(|(key, ..)| key.key().name() == "buzz_multinode_fanout_dropped_total")
            .flat_map(|(key, ..)| {
                key.key()
                    .labels()
                    .map(|label| label.value().to_string())
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(reasons, ["ephemeral"], "counted as an ephemeral drop");
        assert!(
            pending(&dispatcher).is_empty(),
            "a full queue closes nothing"
        );

        let stale = event(PRESENCE, 2);
        tokio::time::advance(MAX_QUEUE_DELAY).await;
        deliver_or_lose(&state, &dispatcher.closer, Queued::new(stale)).await;
        assert!(frames.try_recv().is_err(), "still never delivered late");
        assert!(
            pending(&dispatcher).is_empty(),
            "a stale one closes nothing"
        );

        assert!(
            worker_delivers(&state, &dispatcher.closer, &mut frames, event(PRESENCE, 3)).await,
            "nor fences the connection"
        );
        dispatcher.dispatch(&state, event(STORED, 4)); // queue full
        assert_eq!(pending(&dispatcher), HashSet::from([conn]));
        assert!(!cancel.is_cancelled());
    }

    /// The fence holds on relay-local fan-out too, not only on the workers.
    #[tokio::test]
    async fn fence_withholds_local_fan_out() {
        let state = crate::state::tests::test_state().await;
        let (conn, mut frames, _cancel) = subscriber(&state, STORED);
        let local = |n| pubsub_stored_event(event(STORED, n));

        fan_out_event_to_local_subscribers(&state, community(), &local(0)).await;
        assert!(frames.try_recv().is_ok(), "delivers before the fence");
        state.conn_manager.fence_live_delivery(conn);
        fan_out_event_to_local_subscribers(&state, community(), &local(1)).await;
        assert!(
            frames.try_recv().is_err(),
            "a fenced connection gets nothing"
        );
    }
}
