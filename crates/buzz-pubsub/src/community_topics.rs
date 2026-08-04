//! Level-triggered subscription state for community-scoped Redis pub/sub
//! channel families (cache invalidation, connection control).
//!
//! ElastiCache Serverless rejects `PSUBSCRIBE`, so these families can no longer
//! ride a single `buzz:*:...` wildcard. Each pod instead subscribes to the exact
//! per-community channels it currently needs. Two sets carry that:
//!
//! * **desired** — owned by whoever holds the interest (live sockets for
//!   connection control; local cache residency for cache invalidation),
//!   supplied as a closure and recomputed from scratch on every reconcile. This
//!   module never caches it. Crucially, the *withdrawal* direction re-reads it
//!   under the same lock that removes establishment (`withdraw_undesired`), so
//!   interest renewed after the diff was computed cannot have its subscription
//!   pulled out from under it.
//! * **established** — the communities Redis has acknowledged a `SUBSCRIBE` for
//!   on the *current* connection. A community enters only after the ack returns,
//!   and the whole set is cleared when the connection ends.
//!
//! Reconciliation is level-triggered: the subscriber task diffs desired against
//! established on connect, on a wake, and on a fixed tick. A coalesced or lost
//! wake therefore costs latency, never convergence — and the tick is the
//! automatic restore path that any cleared state needs. A healthy subscription
//! must never sit marked unestablished waiting on an unrelated external event.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use buzz_core::CommunityId;
use futures_util::StreamExt;
use tokio::sync::Notify;

/// Initial reconnect backoff (1 second).
const BACKOFF_INITIAL_SECS: u64 = 1;
/// Maximum reconnect backoff (30 seconds).
const BACKOFF_MAX_SECS: u64 = 30;
/// Upper bound on how long desired and established can diverge without a wake.
/// This is the unconditional convergence path, so it must never be disabled.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(1);

/// Communities whose channel this pod wants subscribed, recomputed on demand.
pub type DesiredCommunities = Arc<dyn Fn() -> HashSet<CommunityId> + Send + Sync>;

/// Subscription state for one community-scoped channel family, shared between
/// the subscriber task and the interest holders that read it.
pub struct CommunityTopics {
    /// Log label, e.g. `cache-invalidation`.
    name: &'static str,
    /// Communities Redis has acknowledged on the live connection. Written only
    /// by the subscriber task, after an ack.
    established: RwLock<HashSet<CommunityId>>,
    /// Latency shortcut: producers signal new interest instead of waiting for
    /// the next tick. Coalescing and loss are both harmless.
    wake: Notify,
}

impl CommunityTopics {
    /// Creates empty subscription state for a named channel family.
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            established: RwLock::new(HashSet::new()),
            wake: Notify::new(),
        }
    }

    /// Log label for this channel family.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Whether Redis has acknowledged this community's subscription on the
    /// current connection.
    ///
    /// This is an *observation*, not a gate: it is true only until the lock is
    /// released. Anything that must not become readable without a live
    /// invalidation topic has to run inside [`Self::with_established`].
    pub fn is_established(&self, community: CommunityId) -> bool {
        self.established
            .read()
            .expect("community topics lock poisoned")
            .contains(&community)
    }

    /// Runs `f` with this community's establishment state, holding it against
    /// withdrawal for the whole call.
    ///
    /// This is the gate for state whose readability depends on the
    /// subscription — in the relay, an authorization cache insert. Withdrawal
    /// takes the matching write lock (`withdraw_undesired`), so the two cannot
    /// interleave: either `f` completes and the withdrawal decision that
    /// follows re-reads desire and sees whatever interest `f` recorded, or the
    /// withdrawal already happened and `f` is told `false`. Reading
    /// establishment and then acting on it in a second step reopens that gap,
    /// however small the gap looks.
    ///
    /// `f` must not call back into this `CommunityTopics`: the read lock is
    /// held, so a nested write would deadlock and a nested read can too if a
    /// writer is already queued. It may touch state the `desired` closure
    /// reads — that closure only ever runs under the write lock, which excludes
    /// this one, so there is no lock cycle.
    pub fn with_established<R>(&self, community: CommunityId, f: impl FnOnce(bool) -> R) -> R {
        let established = self
            .established
            .read()
            .expect("community topics lock poisoned");
        f(established.contains(&community))
    }

    /// Number of established communities, for the connect log line and for
    /// tests asserting how much of a desired set has been acked.
    pub fn established_count(&self) -> usize {
        self.established
            .read()
            .expect("community topics lock poisoned")
            .len()
    }

    /// Ask the subscriber task to reconcile now rather than at the next tick.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Waits for a [`Self::wake`]. The wait side of the same signal, public so
    /// interest holders in other crates can assert they really nudge the
    /// reconciler; the subscriber loop below is its production caller.
    ///
    /// One permit is stored, so a wake raised before the wait still completes.
    pub async fn wake_notified(&self) {
        self.wake.notified().await;
    }

    /// Marks `community` established, standing in for a Redis ack.
    ///
    /// Test seam only: in production the subscriber records establishment, and
    /// only after `SUBSCRIBE` returns. It is public because the gate's consumers
    /// live in another crate and have to be able to exercise both sides of it.
    #[doc(hidden)]
    pub fn insert_established_for_test(&self, community: CommunityId) {
        self.insert_established(community);
    }

    /// Re-reads `desired` and withdraws establishment for everything it no
    /// longer names, **atomically**, returning the communities whose channel the
    /// caller must now `UNSUBSCRIBE`.
    ///
    /// This is the ordering invariant the establishment gate rests on, and the
    /// reason the decision cannot be made from a snapshot taken earlier in the
    /// reconcile. Interest renewal (`CacheResidency::admit_and_insert` in the
    /// relay)
    /// records residency and *then* reads establishment under the read lock;
    /// this method re-reads residency under the matching write lock. Mutual
    /// exclusion makes the two orderings the only possibilities:
    ///
    /// * renewal's establishment read precedes this critical section — then this
    ///   `desired()` call happens after the renewal recorded interest, sees the
    ///   community, and does not withdraw it. The admitted entry stays covered.
    /// * renewal's establishment read follows this critical section — then the
    ///   withdrawal already happened, the read returns false, and nothing is
    ///   admitted in the first place.
    ///
    /// So an entry admitted against establishment is never left readable behind
    /// a withdrawn subscription. A `desired()` re-read *outside* the lock would
    /// not give this: renewal could still land between the check and the removal.
    ///
    /// `desired` must not acquire this lock, or this would deadlock. Both
    /// production closures read only their own registry.
    pub(crate) fn withdraw_undesired(&self, desired: &DesiredCommunities) -> Vec<CommunityId> {
        let mut established = self
            .established
            .write()
            .expect("community topics lock poisoned");
        Self::withdraw_locked(&mut established, desired)
    }

    /// The withdrawal decision itself, given the write lock. Shared so the
    /// non-blocking test twin below cannot drift away from the real path.
    fn withdraw_locked(
        established: &mut HashSet<CommunityId>,
        desired: &DesiredCommunities,
    ) -> Vec<CommunityId> {
        let want = desired();
        let withdrawn: Vec<CommunityId> = established.difference(&want).copied().collect();
        for community in &withdrawn {
            established.remove(community);
        }
        withdrawn
    }

    /// Test seam for `withdraw_undesired`, whose invariant is shared with
    /// the cache-residency gate in another crate.
    #[doc(hidden)]
    pub fn withdraw_undesired_for_test(&self, desired: &DesiredCommunities) -> Vec<CommunityId> {
        self.withdraw_undesired(desired)
    }

    /// Non-blocking `withdraw_undesired`: `None` when the establishment lock is
    /// held elsewhere.
    ///
    /// Test seam for the exclusion itself. A test can call this from inside a
    /// [`Self::with_established`] closure and assert `None` — structural proof
    /// that no withdrawal can interleave with a gated insert, with no threads
    /// and no scheduler dependence. Asserting that through a parked thread
    /// makes the *mutation bite* probabilistic even though the passing case is
    /// deterministic.
    #[doc(hidden)]
    pub fn try_withdraw_undesired_for_test(
        &self,
        desired: &DesiredCommunities,
    ) -> Option<Vec<CommunityId>> {
        let mut established = match self.established.try_write() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return None,
            Err(e) => panic!("community topics lock poisoned: {e}"),
        };
        Some(Self::withdraw_locked(&mut established, desired))
    }

    fn snapshot(&self) -> HashSet<CommunityId> {
        self.established
            .read()
            .expect("community topics lock poisoned")
            .clone()
    }

    fn insert_established(&self, community: CommunityId) {
        self.established
            .write()
            .expect("community topics lock poisoned")
            .insert(community);
    }

    fn clear_established(&self) {
        self.established
            .write()
            .expect("community topics lock poisoned")
            .clear();
    }
}

/// The per-family parts of a community-scoped subscriber: channel naming,
/// channel parsing, and payload decode plus local delivery.
pub(crate) trait CommunityChannelFamily: Send + 'static {
    /// Exact Redis channel for one community. Never a pattern.
    fn channel(&self, community: CommunityId) -> String;
    /// Recover the community from a received channel name.
    fn parse_channel(&self, channel: &str) -> Option<CommunityId>;
    /// Decode one payload and hand it to local consumers.
    fn deliver(&self, community: CommunityId, payload: &str);
}

/// Runs a community-scoped subscriber with automatic reconnection.
///
/// Never returns — spawn it in a background task. Reconnects with exponential
/// backoff (1s → 2s → 4s → … → 30s max), rebuilding the exact subscription set
/// from `desired` on every connect.
///
/// The withdrawal ordering below is enforced for **both** families, not just
/// cache invalidation, because it lives in the shared reconciler. Cache
/// invalidation is what makes it a correctness requirement — a lost invalidation
/// leaves a readable authorization entry with nothing behind it. Connection
/// control's exposure is milder: a `disconnect` command lost in an unsubscribe
/// gap leaves a socket open that a ban has already closed elsewhere, and the DB
/// ban row still refuses that pubkey's next authorization, so the miss costs one
/// stale socket rather than a wrong access decision. The invariant is applied
/// uniformly anyway: one reconciler is less to get wrong than two, and nothing
/// here is per-family.
pub(crate) async fn run_community_subscriber<F: CommunityChannelFamily>(
    redis_url: String,
    family: F,
    topics: Arc<CommunityTopics>,
    desired: DesiredCommunities,
) {
    let name = topics.name();
    let mut backoff_secs = BACKOFF_INITIAL_SECS;

    loop {
        let outcome = connect_and_serve(&redis_url, &family, &topics, &desired).await;
        // The connection is gone, so nothing is subscribed any more. Clearing
        // here (not only on the next connect) is what stops dependent state
        // from being treated as protected during the backoff.
        topics.clear_established();

        match outcome {
            Ok(()) => {
                backoff_secs = BACKOFF_INITIAL_SECS;
                tracing::warn!(
                    "Redis {name} stream ended (clean disconnect) — reconnecting in {backoff_secs}s"
                );
            }
            Err(e) => {
                tracing::error!("Redis {name} error: {e} — reconnecting in {backoff_secs}s");
            }
        }

        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(BACKOFF_MAX_SECS);

        tracing::info!("Attempting to reconnect to Redis {name}...");
    }
}

async fn connect_and_serve<F: CommunityChannelFamily>(
    redis_url: &str,
    family: &F,
    topics: &CommunityTopics,
    desired: &DesiredCommunities,
) -> Result<(), redis::RedisError> {
    let name = topics.name();
    let client = redis::Client::open(redis_url)?;
    let conn = client.get_async_pubsub().await?;
    let (mut sink, mut stream) = conn.split();

    // A fresh connection has nothing subscribed, whatever the previous one had.
    topics.clear_established();
    reconcile(&mut sink, family, topics, desired).await?;

    tracing::info!(
        established = topics.established_count(),
        "Redis {name} subscriber connected with exact per-community subscriptions"
    );

    let mut ticker = tokio::time::interval(RECONCILE_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = topics.wake.notified() => {
                reconcile(&mut sink, family, topics, desired).await?;
            }
            _ = ticker.tick() => {
                reconcile(&mut sink, family, topics, desired).await?;
            }
            msg = stream.next() => {
                let Some(msg) = msg else {
                    // Stream returned None — Redis connection closed.
                    return Ok(());
                };

                let channel = msg.get_channel_name();
                let Some(community_id) = family.parse_channel(channel) else {
                    tracing::warn!("Received {name} message on unexpected channel: {channel}");
                    continue;
                };

                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("Failed to get {name} payload: {e}");
                        continue;
                    }
                };

                family.deliver(community_id, &payload);
            }
        }
    }
}

/// Bring the live connection's subscriptions in line with current desire.
///
/// The two directions are deliberately asymmetric, because only one of them can
/// break an invariant:
///
/// * **Subscribing** from the pass's initial `desired()` read is safe. Holding a
///   subscription nobody wants any more costs one idle channel until the next
///   pass withdraws it; it can never make an entry readable without coverage.
/// * **Unsubscribing** must not be decided from that same read. Between the read
///   and the command, interest can be renewed and an insert admitted against the
///   still-present establishment bit — and a pub/sub message lost in the
///   resulting gap is lost permanently, because pub/sub has no replay. The tick
///   restores *state*; nothing restores a dropped invalidation. So withdrawal
///   re-reads desire under the establishment write lock, via
///   [`CommunityTopics::withdraw_undesired`], and only the communities that
///   round-trip out of that critical section are unsubscribed.
async fn reconcile<F: CommunityChannelFamily>(
    sink: &mut redis::aio::PubSubSink,
    family: &F,
    topics: &CommunityTopics,
    desired: &DesiredCommunities,
) -> Result<(), redis::RedisError> {
    let want = desired();
    let have = topics.snapshot();

    for community in want.difference(&have) {
        // `subscribe` awaits Redis's reply, so this returning `Ok` *is* the
        // acknowledgement. Only then does the community count as established.
        sink.subscribe(&family.channel(*community)).await?;
        topics.insert_established(*community);
    }

    // Establishment is withdrawn inside the lock, so a reader that admits an
    // entry either observed the bit before the withdrawal (and its renewed
    // interest was seen here, keeping the subscription) or observes it after
    // (and admits nothing).
    for community in topics.withdraw_undesired(desired) {
        sink.unsubscribe(&family.channel(community)).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn community(id: u128) -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(id))
    }

    #[test]
    fn nothing_is_established_before_an_ack() {
        let topics = CommunityTopics::new("test");
        assert!(!topics.is_established(community(1)));
        assert_eq!(topics.established_count(), 0);
    }

    #[test]
    fn established_is_cleared_wholesale_on_disconnect() {
        let topics = CommunityTopics::new("test");
        topics.insert_established(community(1));
        topics.insert_established(community(2));
        assert_eq!(topics.established_count(), 2);

        topics.clear_established();

        assert!(!topics.is_established(community(1)));
        assert!(!topics.is_established(community(2)));
    }

    #[test]
    fn removing_one_community_leaves_the_others_established() {
        let topics = CommunityTopics::new("test");
        topics.insert_established(community(1));
        topics.insert_established(community(2));

        let keep = community(2);
        let desired: DesiredCommunities = Arc::new(move || HashSet::from([keep]));
        assert_eq!(topics.withdraw_undesired(&desired), vec![community(1)]);

        assert!(!topics.is_established(community(1)));
        assert!(topics.is_established(community(2)));
    }

    #[tokio::test]
    async fn a_wake_raised_before_the_wait_is_not_lost() {
        let topics = CommunityTopics::new("test");
        topics.wake();

        // Notify stores one permit, so a wake that arrives before the subscriber
        // reaches its select still reconciles immediately. Convergence does not
        // depend on this — the tick covers a lost wake — but latency does.
        tokio::time::timeout(Duration::from_millis(50), topics.wake.notified())
            .await
            .expect("a wake raised before the wait must still complete");
    }

    /// Withdrawal must decide from desire read *inside* its own critical
    /// section, not from a snapshot taken earlier in the reconcile pass.
    ///
    /// This is the race Wren found in review. The subscribe loop awaits Redis
    /// round-trips, so an arbitrary amount of time passes between the pass's
    /// `desired()` read and the withdrawal — long enough for interest to be
    /// renewed and a cache entry admitted against the still-present
    /// establishment bit. Here the stale view says "withdraw c", live desire says
    /// "keep c", and the assertion is that live desire wins.
    #[test]
    fn withdrawal_reads_live_desire_not_the_passs_stale_view() {
        let topics = CommunityTopics::new("test");
        let c = community(1);
        topics.insert_established(c);

        // The pass's initial read saw an empty desired set (c not wanted). By
        // the time withdrawal runs, interest has been renewed.
        let desired: DesiredCommunities = Arc::new(move || HashSet::from([c]));

        assert!(
            topics.withdraw_undesired(&desired).is_empty(),
            "a community desired at withdrawal time must not be unsubscribed"
        );
        assert!(
            topics.is_established(c),
            "renewed interest must keep its establishment bit"
        );
    }

    /// The other direction of the same call: still-undesired communities are
    /// withdrawn, and establishment is dropped before the caller unsubscribes.
    #[test]
    fn withdrawal_still_removes_communities_nobody_wants() {
        let topics = CommunityTopics::new("test");
        let (kept, dropped) = (community(1), community(2));
        topics.insert_established(kept);
        topics.insert_established(dropped);

        let desired: DesiredCommunities = Arc::new(move || HashSet::from([kept]));

        assert_eq!(topics.withdraw_undesired(&desired), vec![dropped]);
        assert!(
            !topics.is_established(dropped),
            "establishment must be gone before the UNSUBSCRIBE is issued"
        );
        assert!(topics.is_established(kept));
    }

    /// The invariant itself, forced deterministically: a renewal that observes
    /// establishment can never have its subscription withdrawn by a
    /// concurrently-running reconcile.
    ///
    /// The `desired` closure parks *inside* the withdrawal critical section and
    /// releases the renewal thread from there, so the renewal's
    /// `is_established` read is guaranteed to happen while withdrawal holds the
    /// write lock. That is the interleaving a sleep-based test only reaches by
    /// luck. The reader must then observe `false` (withdrawal won the lock and
    /// this admission is correctly refused) — never `true` while the community
    /// also ends up unsubscribed, which is the stale-authz hole.
    #[test]
    fn a_renewal_racing_withdrawal_never_sees_establishment_it_loses() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc;

        let topics = Arc::new(CommunityTopics::new("test"));
        let c = community(1);
        topics.insert_established(c);

        // Signals the renewal thread that withdrawal now holds the write lock.
        let (in_section_tx, in_section_rx) = mpsc::channel::<()>();
        // Set by the renewal thread once its establishment read has returned.
        let renewal_read = Arc::new(AtomicBool::new(false));
        let read_result = Arc::new(AtomicBool::new(false));

        let renewal_topics = Arc::clone(&topics);
        let renewal_read_flag = Arc::clone(&renewal_read);
        let renewal_result = Arc::clone(&read_result);
        let renewal = std::thread::spawn(move || {
            in_section_rx
                .recv()
                .expect("withdrawal must signal from inside its critical section");
            // Blocks until withdrawal releases the write lock.
            let admitted = renewal_topics.is_established(c);
            renewal_result.store(admitted, Ordering::SeqCst);
            renewal_read_flag.store(true, Ordering::SeqCst);
        });

        // Desire says "nobody wants c" — but hands the renewal thread its cue
        // first, from inside the lock.
        let desired: DesiredCommunities = Arc::new(move || {
            in_section_tx.send(()).expect("renewal thread alive");
            // Give the renewal thread every chance to reach its read while the
            // write lock is still held; RwLock write exclusion is what makes
            // the outcome deterministic regardless of how far it gets.
            std::thread::sleep(Duration::from_millis(50));
            HashSet::new()
        });

        assert_eq!(topics.withdraw_undesired(&desired), vec![c]);
        renewal.join().expect("renewal thread panicked");

        assert!(renewal_read.load(Ordering::SeqCst));
        assert!(
            !read_result.load(Ordering::SeqCst),
            "a renewal whose interest was not seen by withdrawal must be refused, \
             never admitted against an establishment bit that is being removed"
        );
        assert!(!topics.is_established(c));
    }

    /// `with_established` must exclude withdrawal for the whole closure, not
    /// just for the establishment read.
    ///
    /// This is the lock half of the lifetime invariant, and it is asserted
    /// *structurally*: from inside the closure, `try_write()` must report
    /// `WouldBlock`. No threads, no sleeps, no scheduler. Under a mutation that
    /// releases the lock before calling `f`, `try_write()` succeeds and this
    /// fails on every run — which is what a deterministic mutation kill means.
    /// An earlier draft signalled a withdrawal thread and slept 50ms to "give
    /// it every chance"; that is deterministic on correct code but its
    /// *mutation bite* depends on the scheduler, so it could pass vacuously.
    /// Wren caught it pre-push.
    #[test]
    fn with_established_holds_the_gate_for_the_whole_closure() {
        let topics = CommunityTopics::new("test");
        let c = community(1);
        topics.insert_established(c);

        let established = topics.with_established(c, |established| {
            assert!(
                matches!(
                    topics.established.try_write(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "withdrawal must be excluded for the whole closure; a gate that \
                 releases the lock before the caller acts is the round-2 bug"
            );
            established
        });

        assert!(established);
        // And the exclusion ends with the closure, or the reconciler would
        // never be able to withdraw anything.
        assert!(topics.established.try_write().is_ok());
    }
}

/// Redis-backed reconciliation tests.
///
/// These are `#[ignore]`d because they need a live server, and CI selects them
/// explicitly (`--run-ignored all` over `package(buzz-pubsub)`). They route
/// through `test_util::test_redis_url()`, so `REDIS_URL` points them at either
/// standalone Redis or a cluster-mode server; when it is set but unreachable
/// they must **fail**, never self-skip — a suite that quietly passes when the
/// server is missing is exactly the fake-green class this lane is guarding
/// against.
#[cfg(test)]
mod redis_tests {
    use super::*;
    use crate::cache_invalidation::{
        cache_invalidation_channel_for, run_cache_invalidation_subscriber, CacheInvalidation,
        ScopedCacheInvalidation,
    };
    use crate::test_util::{await_established, await_unestablished, test_redis_url};
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;
    use tokio::sync::broadcast;
    use uuid::Uuid;

    /// A mutable desired set a test can steer, plus the closure over it.
    fn steerable() -> (Arc<StdMutex<HashSet<CommunityId>>>, DesiredCommunities) {
        let cell = Arc::new(StdMutex::new(HashSet::new()));
        let reader = Arc::clone(&cell);
        (
            cell,
            Arc::new(move || reader.lock().expect("desired lock poisoned").clone()),
        )
    }

    fn set_desired(cell: &Arc<StdMutex<HashSet<CommunityId>>>, want: &[CommunityId]) {
        *cell.lock().expect("desired lock poisoned") = want.iter().copied().collect();
    }

    /// Distinct communities per test run: these tests share one Redis, and a
    /// fixed id would let a previous run's channel traffic bleed in.
    fn fresh(n: usize) -> Vec<CommunityId> {
        (0..n)
            .map(|_| CommunityId::from_uuid(Uuid::new_v4()))
            .collect()
    }

    /// A plain connection for issuing commands. Panics if the server named by
    /// `REDIS_URL` is unreachable — these tests must fail loudly rather than
    /// self-skip when their dependency is missing.
    async fn control_conn() -> redis::aio::MultiplexedConnection {
        redis::Client::open(test_redis_url())
            .expect("redis client")
            .get_multiplexed_async_connection()
            .await
            .expect("REDIS_URL must be reachable for the ignored Redis suite")
    }

    /// Publishes `invalidation` on `community`'s exact channel.
    ///
    /// The return of `PUBLISH` is deliberately ignored: it counts *pattern*
    /// matches as well as exact ones, so any unrelated `PSUBSCRIBE buzz:*`
    /// client on the same server inflates it. Subscription state is asserted
    /// with [`exact_subscribers`] instead, and delivery by actually receiving.
    async fn publish(community: CommunityId, invalidation: &CacheInvalidation) {
        let _: i64 = redis::cmd("PUBLISH")
            .arg(cache_invalidation_channel_for(community))
            .arg(serde_json::to_string(invalidation).expect("serialize"))
            .query_async(&mut control_conn().await)
            .await
            .expect("PUBLISH");
    }

    /// Exact (non-pattern) subscriber count for `community`'s channel.
    ///
    /// `PUBSUB NUMSUB` ignores pattern subscribers, which is exactly the
    /// distinction this lane turns on: it proves an exact `SUBSCRIBE` is in
    /// place, and cannot be satisfied by a leftover wildcard. Combined with a
    /// per-run random community id, no other client on a shared server can
    /// contribute to this count.
    async fn exact_subscribers(community: CommunityId) -> i64 {
        let channel = cache_invalidation_channel_for(community);
        let (_, count): (String, i64) = redis::cmd("PUBSUB")
            .arg("NUMSUB")
            .arg(&channel)
            .query_async(&mut control_conn().await)
            .await
            .expect("PUBSUB NUMSUB");
        count
    }

    /// Client ids that currently hold `n` exact subscriptions and no patterns.
    ///
    /// Used to single out one subscriber's own connection for a targeted sever.
    /// The tests in this module run in parallel inside one binary, so several
    /// clients have exact subscriptions at any moment; the caller disambiguates
    /// by diffing this before and after its own subscriber connects, and by
    /// choosing an `n` no other test in the crate uses.
    async fn clients_with_exact_subs(
        conn: &mut redis::aio::MultiplexedConnection,
        n: usize,
    ) -> HashSet<i64> {
        let list: String = redis::cmd("CLIENT")
            .arg("LIST")
            .query_async(conn)
            .await
            .expect("CLIENT LIST");
        list.lines()
            .filter(|line| line.contains(" psub=0 ") && line.contains(&format!(" sub={n} ")))
            .filter_map(|line| {
                line.split_whitespace()
                    .find_map(|field| field.strip_prefix("id="))
                    .and_then(|id| id.parse().ok())
            })
            .collect()
    }

    fn membership(seed: u128) -> CacheInvalidation {
        CacheInvalidation::Membership {
            channel_id: Uuid::from_u128(seed),
            pubkey: vec![(seed & 0xff) as u8; 32],
        }
    }

    /// Spawns a cache-invalidation subscriber over `desired` and returns its
    /// topics handle plus a receiver of delivered invalidations.
    fn spawn_subscriber(
        desired: DesiredCommunities,
    ) -> (
        Arc<CommunityTopics>,
        broadcast::Receiver<ScopedCacheInvalidation>,
    ) {
        let topics = Arc::new(CommunityTopics::new(
            crate::cache_invalidation::CACHE_INVALIDATION_NAME,
        ));
        let (tx, rx) = broadcast::channel(4096);
        let spawn_topics = Arc::clone(&topics);
        tokio::spawn(async move {
            run_cache_invalidation_subscriber(test_redis_url(), tx, spawn_topics, desired).await
        });
        (topics, rx)
    }

    /// The core replacement claim: 100 exact `SUBSCRIBE`s on ONE RESP2
    /// connection route every `(channel, payload)` pair byte-exactly to the
    /// community it was published on.
    ///
    /// This is what `PSUBSCRIBE buzz:*:cache-invalidate` used to do in one
    /// command. A wildcard cannot be used on ElastiCache Serverless, so the
    /// property that has to hold now is that N exact subscriptions on a single
    /// connection are equivalent — including that no message is delivered under
    /// the wrong community label, which is the failure that would silently
    /// cross-wire tenants.
    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn hundred_exact_subscriptions_route_every_payload_to_its_own_community() {
        let communities = fresh(100);
        let (cell, desired) = steerable();
        set_desired(&cell, &communities);
        let (topics, mut rx) = spawn_subscriber(desired);
        await_established(&topics, &communities, "100 exact subscriptions").await;
        assert_eq!(topics.established_count(), 100);

        // One distinguishable payload per community, all on one connection.
        let mut expected = HashMap::new();
        for (i, community) in communities.iter().enumerate() {
            let invalidation = membership(0x1000 + i as u128);
            assert_eq!(
                exact_subscribers(*community).await,
                1,
                "exactly one exact SUBSCRIBE should hold {community}'s channel"
            );
            publish(*community, &invalidation).await;
            expected.insert(*community, invalidation);
        }

        let mut seen: HashMap<CommunityId, CacheInvalidation> = HashMap::new();
        while seen.len() < communities.len() {
            let scoped = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("timed out before all 100 payloads arrived")
                .expect("broadcast closed");
            assert!(
                seen.insert(scoped.community_id, scoped.invalidation)
                    .is_none(),
                "community {} delivered twice",
                scoped.community_id
            );
        }
        assert_eq!(
            seen, expected,
            "payload must arrive under its own community"
        );
    }

    /// Reconciliation is level-triggered in BOTH directions: growing the
    /// desired set subscribes, shrinking it unsubscribes, and the communities
    /// that stayed desired keep working across the change.
    ///
    /// The `subscriber_count == 0` assertion is the real teeth. Establishment
    /// bookkeeping saying "unsubscribed" proves nothing on its own; Redis
    /// reporting no subscriber for the channel proves the `UNSUBSCRIBE` was
    /// actually issued.
    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn unsubscribing_a_subset_leaves_the_rest_routing() {
        let c = fresh(3);
        let (cell, desired) = steerable();
        set_desired(&cell, &c);
        let (topics, mut rx) = spawn_subscriber(desired);
        await_established(&topics, &c, "initial three").await;

        // Drop the middle one. No explicit command: only the desired set moves.
        set_desired(&cell, &[c[0], c[2]]);
        topics.wake();
        await_unestablished(&topics, c[1], "the withdrawn community").await;

        assert_eq!(
            exact_subscribers(c[1]).await,
            0,
            "Redis must report no exact subscriber for the unsubscribed channel"
        );
        assert!(topics.is_established(c[0]) && topics.is_established(c[2]));

        // The retained communities still route, and the withdrawn one's publish
        // does not arrive — checked by identity, not by absence of traffic.
        let kept = membership(0x2002);
        assert_eq!(exact_subscribers(c[2]).await, 1);
        publish(c[2], &kept).await;
        let scoped = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("timed out on retained community")
            .expect("broadcast closed");
        assert_eq!(scoped.community_id, c[2]);
        assert_eq!(scoped.invalidation, kept);

        // And re-adding it converges again with no command, closing the loop.
        set_desired(&cell, &c);
        topics.wake();
        await_established(&topics, &c, "re-added community").await;
        assert_eq!(exact_subscribers(c[1]).await, 1);
    }

    /// Convergence must not depend on the wake: with wakes deliberately never
    /// issued, the periodic reconcile alone has to subscribe a newly desired
    /// community.
    ///
    /// This is the restore-path property. A cleared or missing subscription
    /// that only recovers when some external event fires is the bug class I want
    /// structurally excluded, so the tick is asserted in isolation.
    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn the_periodic_reconcile_converges_without_any_wake() {
        let c = fresh(1);
        let (cell, desired) = steerable();
        let (topics, _rx) = spawn_subscriber(desired);
        // Subscriber is up with an empty desired set.
        await_established(&topics, &[], "empty start").await;

        set_desired(&cell, &c);
        // Deliberately no `topics.wake()`.
        await_established(&topics, &c, "tick-only convergence").await;
        assert_eq!(exact_subscribers(c[0]).await, 1);
    }

    /// Sever the connection under the subscriber and it must rebuild the whole
    /// desired set on reconnect — and, before that, report nothing established,
    /// so the cache gate cannot treat entries as protected during the gap.
    ///
    /// `CLIENT KILL` is the sever: it is a real server-side disconnect, not a
    /// simulated error return, so the reconnect path runs for the same reason it
    /// would in production.
    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn a_severed_connection_rebuilds_the_entire_desired_set() {
        // Five communities: a subscription count no other test in this crate
        // uses, so this subscriber's connection is identifiable in CLIENT LIST
        // even with the rest of the suite running in parallel.
        const SEVER_TEST_COMMUNITIES: usize = 5;
        let c = fresh(SEVER_TEST_COMMUNITIES);
        let mut conn = control_conn().await;
        let before = clients_with_exact_subs(&mut conn, SEVER_TEST_COMMUNITIES).await;

        let (cell, desired) = steerable();
        set_desired(&cell, &c);
        let (topics, _rx) = spawn_subscriber(desired);
        await_established(&topics, &c, "pre-sever").await;

        // Sever ONLY this subscriber, by id. `CLIENT KILL TYPE pubsub` would
        // also kill every other pub/sub client on the server — a concurrent
        // test here, or an unrelated process sharing the dev Redis.
        let after = clients_with_exact_subs(&mut conn, SEVER_TEST_COMMUNITIES).await;
        let new_clients: Vec<i64> = after.difference(&before).copied().collect();
        assert_eq!(
            new_clients.len(),
            1,
            "expected exactly one new {SEVER_TEST_COMMUNITIES}-subscription client to be ours, got {new_clients:?}"
        );
        let killed: i64 = redis::cmd("CLIENT")
            .arg("KILL")
            .arg("ID")
            .arg(new_clients[0])
            .query_async(&mut conn)
            .await
            .expect("CLIENT KILL ID");
        assert_eq!(killed, 1, "expected to sever exactly our own subscriber");

        // The sever must be observable as lost establishment, not papered over:
        // the gate has to read closed while the connection is gone.
        await_unestablished(&topics, c[0], "establishment during the reconnect gap").await;

        // Rebuilt from `desired`, not from a remembered active set: all five are
        // back, and each channel really has an exact subscriber again.
        await_established(&topics, &c, "post-sever rebuild").await;
        for community in &c {
            assert_eq!(
                exact_subscribers(*community).await,
                1,
                "channel for {community} must be re-subscribed after the sever"
            );
        }
    }

    /// The establishment gate's precondition: while nothing is established,
    /// `is_established` is false for a community the pod desires, and it flips
    /// to true only once Redis has acked.
    ///
    /// This is the stale-authz window. The relay consults exactly this to decide
    /// whether caching an authorization decision is safe, so "unestablished
    /// while disconnected" has to be observable rather than merely intended.
    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn a_desired_community_is_unestablished_until_redis_acks() {
        let c = fresh(1);
        let (cell, desired) = steerable();
        set_desired(&cell, &c);

        let topics = Arc::new(CommunityTopics::new(
            crate::cache_invalidation::CACHE_INVALIDATION_NAME,
        ));
        // Desired, but no subscriber running: nothing can have been acked, so
        // the gate must read closed even though interest exists.
        assert!(
            !topics.is_established(c[0]),
            "desire alone must not open the cache gate"
        );

        let (tx, _rx) = broadcast::channel(16);
        let spawn_topics = Arc::clone(&topics);
        tokio::spawn(async move {
            run_cache_invalidation_subscriber(test_redis_url(), tx, spawn_topics, desired).await
        });
        await_established(&topics, &c, "gate opens on ack").await;

        // And it closes again for a community that leaves the desired set.
        set_desired(&cell, &[]);
        topics.wake();
        await_unestablished(&topics, c[0], "gate closes on withdrawal").await;
    }

    /// End-to-end against live Redis: interest renewed *during* a reconcile pass
    /// must not lose its exact subscription.
    ///
    /// The unit test above proves the lock discipline in isolation; this proves
    /// the property survives the real `reconcile` path, where awaited Redis
    /// round-trips sit between the pass's desire read and the withdrawal.
    ///
    /// The closure is a phase machine keyed on read count, which is what makes
    /// the interleaving deterministic: once the race is armed, the pass's *first*
    /// read reports the community undesired (so a stale-snapshot reconciler would
    /// queue an `UNSUBSCRIBE`), and every read after it reports the community
    /// desired again (the renewal). Correct code re-reads inside the withdrawal
    /// critical section, sees the renewal, and keeps the subscription. Verified
    /// to bite: reinstating the `have.difference(&want)` withdrawal fails this
    /// test on the `PUBSUB NUMSUB` assertion.
    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn interest_renewed_mid_reconcile_keeps_its_exact_subscription() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        let c = fresh(1)[0];
        let armed = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicUsize::new(0));

        let closure_armed = Arc::clone(&armed);
        let closure_reads = Arc::clone(&reads);
        let racing: DesiredCommunities = Arc::new(move || {
            if !closure_armed.load(Ordering::SeqCst) {
                return HashSet::from([c]);
            }
            // First read after arming: interest looks gone. Every later read
            // (crucially, the one inside the withdrawal lock) sees it renewed.
            if closure_reads.fetch_add(1, Ordering::SeqCst) == 0 {
                HashSet::new()
            } else {
                HashSet::from([c])
            }
        });

        let (topics, _rx) = spawn_subscriber(racing);
        await_established(&topics, &[c], "pre-race establishment").await;
        assert_eq!(exact_subscribers(c).await, 1);

        armed.store(true, Ordering::SeqCst);
        topics.wake();

        // The discriminator, and why this bites where a coarser wait did not:
        // correct code reads desire TWICE in one pass (the pass read, then the
        // withdrawal read inside the lock), microseconds apart. A stale-snapshot
        // reconciler reads once, unsubscribes, and only reads again at the next
        // tick — a whole `RECONCILE_INTERVAL` later, by which point it has
        // re-subscribed and a naive end-state assertion passes vacuously.
        //
        // So: poll far faster than the tick, require the second read inside a
        // window well below it, and assert establishment never drops along the
        // way. The transient loss is the bug; the repaired end state is not proof.
        let window = RECONCILE_INTERVAL / 4;
        let deadline = std::time::Instant::now() + window;
        while reads.load(Ordering::SeqCst) < 2 {
            assert!(
                topics.is_established(c),
                "establishment was withdrawn for renewed interest — an \
                 invalidation published now would be lost, and the later tick \
                 that re-subscribes cannot bring it back"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "withdrawal did not re-read desire within its own pass ({window:?}); \
                 a second read this late means it decided from the stale snapshot"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        assert!(
            topics.is_established(c),
            "renewed interest must not lose establishment"
        );
        assert_eq!(
            exact_subscribers(c).await,
            1,
            "renewed interest must still hold an exact SUBSCRIBE in Redis"
        );
    }
}
