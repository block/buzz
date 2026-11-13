#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-pubsub` — Redis pub/sub fan-out, presence tracking, and typing indicators.
//!
//! # Architecture
//!
//! ```text
//! buzz-relay process
//!   │
//!   ├── shared RedisConnection (one multiplexed socket) → PUBLISH, SET, ZADD, etc.
//!   │
//!   └── dedicated redis::aio::PubSub connection (NOT the shared one)
//!         └── dynamic SUBSCRIBE buzz:{community}:channel:{id} / buzz:{community}:global
//!               └── run_subscriber() → broadcast::channel(4096) → N WS receivers
//! ```
//!
//! The subscriber reconnects automatically on Redis disconnect with exponential
//! backoff (1s → 2s → 4s → … → 30s max).
//!
//! Dedicated pub/sub connection is stateful and cannot be shared.
//! The shared [`RedisConnection`] handles all other commands.
//! Lagged receivers get `RecvError::Lagged`.

/// Cross-pod cache-key invalidation over Redis pub/sub.
pub mod cache_invalidation;
/// Cross-pod connection-control commands over Redis pub/sub.
pub mod conn_control;
/// The shared Redis command connection.
pub mod connection;
pub use connection::{
    redis_connection, RedisConnection, REDIS_CONNECT_TIMEOUT, REDIS_RESPONSE_TIMEOUT,
};
/// Error types for pub/sub operations.
pub mod error;
/// Redis-backed NIP-98 replay seen-set.
pub mod nip98_replay;
pub use nip98_replay::RedisNip98ReplayGuard;
pub mod nip_fi_command_replay;
pub use nip_fi_command_replay::RedisCommandReplayGuard;
/// Online/offline presence tracking in Redis.
pub mod presence;
/// Redis PUBLISH for channel event fan-out.
pub mod publisher;
/// Redis-backed rate limiter (fixed-window INCR + EXPIRE).
pub mod rate_limiter;
/// Redis SUBSCRIBE for channel event delivery.
pub mod subscriber;
/// Community-scoped Redis event topics.
pub mod topic;
/// Typing indicator tracking in Redis.
pub use error::PubSubError;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use buzz_core::TenantContext;
use nostr::PublicKey;
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex, Notify};

use crate::cache_invalidation::{
    cache_invalidation_channel, CacheInvalidation, ScopedCacheInvalidation,
};
pub use crate::conn_control::NipFiDisconnect;
use crate::conn_control::{conn_control_channel, ConnControl, ScopedConnControl};
pub use crate::conn_control::{decode_nip_fi_disconnect, encode_nip_fi_disconnect};
pub use crate::topic::{channel_key, global_key, EventTopic, EventTopicKey};

/// A Nostr event received on a scoped Redis event topic, broadcast to local subscribers.
#[derive(Debug, Clone)]
pub struct ChannelEvent {
    /// Server-resolved community that scoped the Redis topic.
    pub community_id: buzz_core::CommunityId,
    /// Tenant-local routing scope for this event.
    pub topic: EventTopic,
    /// The Nostr event payload.
    pub event: nostr::Event,
}

/// Health of this node's Redis event subscription link.
///
/// `epoch` advances on every event that can lose messages without a replay:
/// the link connects, drops, or skips a message it could not decode. A reader
/// that needs every message records the epoch when its interest is confirmed
/// and treats any later change as a possible loss.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkState {
    /// Advances on every possible loss.
    pub epoch: u64,
    /// Whether the link is connected and its desired topics are subscribed.
    pub up: bool,
}

/// Configuration for the pub/sub subsystem.
#[derive(Debug, Clone)]
pub struct PubSubConfig {
    /// Redis connection URL (e.g. `redis://127.0.0.1:6379`).
    pub redis_url: String,
    /// Delay before unsubscribing after the last local interest is released.
    pub unsubscribe_debounce: Duration,
}

impl PubSubConfig {
    /// Default delay before unsubscribing after the last local interest is released.
    pub const DEFAULT_UNSUBSCRIBE_DEBOUNCE: Duration = Duration::from_millis(500);

    /// Creates a new `PubSubConfig` with the given Redis URL.
    pub fn new(redis_url: impl Into<String>) -> Self {
        Self {
            redis_url: redis_url.into(),
            unsubscribe_debounce: Self::DEFAULT_UNSUBSCRIBE_DEBOUNCE,
        }
    }

    /// Override the unsubscribe debounce delay.
    pub fn with_unsubscribe_debounce(mut self, debounce: Duration) -> Self {
        self.unsubscribe_debounce = debounce;
        self
    }
}

/// Capacity of the subscriber command queue. A full queue loses only a
/// subscribe hint: the desired-topic map is reconciled separately.
const SUBSCRIPTION_COMMAND_CAPACITY: usize = 4096;

/// Central pub/sub manager for a Buzz relay instance.
pub struct PubSubManager {
    pool: crate::RedisConnection,
    /// Redis URL used by the reconnect loop to re-establish pub/sub connections.
    redis_url: String,
    /// Delay before unsubscribing after the last local interest is released.
    unsubscribe_debounce: Duration,
    /// Local desired topic refcounts; source of truth across Redis reconnects.
    desired_topics: subscriber::DesiredTopics,
    subscription_tx: mpsc::Sender<subscriber::SubscriptionCommand>,
    subscription_rx: Mutex<Option<mpsc::Receiver<subscriber::SubscriptionCommand>>>,
    /// Coalesced wake-up: reconcile desired topics with the live Redis link.
    subscription_changed: Arc<Notify>,
    broadcast_tx: broadcast::Sender<ChannelEvent>,
    cache_invalidation_tx: broadcast::Sender<ScopedCacheInvalidation>,
    conn_control_tx: broadcast::Sender<ScopedConnControl>,
    nip_fi_disconnect_tx: broadcast::Sender<NipFiDisconnect>,
    link: watch::Sender<LinkState>,
}

impl PubSubManager {
    /// Creates a new `PubSubManager` connected to the given Redis URL.
    pub async fn new(redis_url: &str, pool: crate::RedisConnection) -> Result<Self, PubSubError> {
        Self::with_config(PubSubConfig::new(redis_url), pool).await
    }

    /// Creates a new `PubSubManager` using explicit pub/sub configuration.
    pub async fn with_config(
        config: PubSubConfig,
        pool: crate::RedisConnection,
    ) -> Result<Self, PubSubError> {
        let (broadcast_tx, _) = broadcast::channel(4096);
        let (cache_invalidation_tx, _) = broadcast::channel(4096);
        let (conn_control_tx, _) = broadcast::channel(4096);
        let (nip_fi_disconnect_tx, _) = broadcast::channel(4096);
        let (subscription_tx, subscription_rx) = mpsc::channel(SUBSCRIPTION_COMMAND_CAPACITY);

        Ok(Self {
            pool,
            redis_url: config.redis_url,
            unsubscribe_debounce: config.unsubscribe_debounce,
            desired_topics: Arc::new(Mutex::new(HashMap::new())),
            subscription_tx,
            subscription_rx: Mutex::new(Some(subscription_rx)),
            subscription_changed: Arc::new(Notify::new()),
            broadcast_tx,
            cache_invalidation_tx,
            conn_control_tx,
            nip_fi_disconnect_tx,
            link: watch::Sender::new(LinkState::default()),
        })
    }

    /// Starts the pub/sub fan-out loop with automatic reconnection.
    ///
    /// Runs forever — spawn this in a background task. The loop reconnects
    /// with exponential backoff on Redis disconnect (1s → 2s → 4s → … → 30s).
    pub async fn run_subscriber(self: Arc<Self>) {
        let Some(subscription_rx) = self.subscription_rx.lock().await.take() else {
            tracing::error!("Redis pub/sub subscriber already started");
            return;
        };

        subscriber::run_subscriber(
            self.redis_url.clone(),
            self.broadcast_tx.clone(),
            self.desired_topics.clone(),
            subscription_rx,
            self.subscription_changed.clone(),
            self.link.clone(),
        )
        .await;
    }

    /// Starts the cache-invalidation subscriber loop with automatic
    /// reconnection. Runs forever — spawn this in a background task.
    pub async fn run_cache_invalidation_subscriber(self: Arc<Self>) {
        cache_invalidation::run_cache_invalidation_subscriber(
            self.redis_url.clone(),
            self.cache_invalidation_tx.clone(),
        )
        .await;
    }

    /// Starts the connection-control subscriber loop with automatic
    /// reconnection. Runs forever — spawn this in a background task.
    pub async fn run_conn_control_subscriber(self: Arc<Self>) {
        conn_control::run_conn_control_subscriber(
            self.redis_url.clone(),
            self.conn_control_tx.clone(),
        )
        .await;
    }

    /// Starts the NIP-FI disconnect subscriber loop with automatic
    /// reconnection.  Runs forever — spawn this in a background task.
    ///
    /// Subscribes to `channels`: every NIP-FI pod hears the enforce channel,
    /// and a shadow pod also hears its own.  On receipt the pod merges the
    /// deny entry; an enforce pod closes matching local sessions.
    pub async fn run_nip_fi_disconnect_subscriber(
        self: Arc<Self>,
        channels: &'static [&'static str],
    ) {
        conn_control::run_nip_fi_disconnect_subscriber(
            self.redis_url.clone(),
            self.nip_fi_disconnect_tx.clone(),
            channels,
        )
        .await;
    }

    /// Returns a new broadcast receiver for locally-published channel events.
    pub fn subscribe_local(&self) -> broadcast::Receiver<ChannelEvent> {
        self.broadcast_tx.subscribe()
    }

    /// Retain local interest in a scoped Redis event topic.
    ///
    /// The first retain for a topic asks the subscriber task to `SUBSCRIBE`.
    /// Additional retains only increment the local desired refcount. It never
    /// waits on the subscriber: callers hold a connection's lifecycle lock.
    pub async fn retain_topic(&self, ctx: &TenantContext, topic: EventTopic) {
        let topic_key = EventTopicKey::from_context(ctx, topic);
        let should_subscribe = {
            let mut desired = self.desired_topics.lock().await;
            let count = desired.entry(topic_key).or_insert(0);
            let was_zero = *count == 0;
            *count += 1;
            was_zero
        };

        if should_subscribe {
            match self
                .subscription_tx
                .try_send(subscriber::SubscriptionCommand::Subscribe(topic_key))
            {
                // A queued command subscribes in order; a closed queue means
                // the subscriber is gone, and a restart re-subscribes the
                // whole desired-topic map.
                Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // The wake-up is lossless and coalesced: the subscriber
                    // reconciles the desired-topic map on the live link
                    // instead of waiting for a Redis reconnect.
                    self.subscription_changed.notify_one();
                    tracing::warn!(
                        ?topic_key,
                        "pubsub subscription command queue full; reconciliation scheduled"
                    );
                }
            }
        }
    }

    /// Watch this node's Redis event link (see [`LinkState`]).
    pub fn link_state(&self) -> watch::Receiver<LinkState> {
        self.link.subscribe()
    }

    /// Wait until Redis has confirmed a subscription to an already retained
    /// topic, and return the link epoch it was confirmed on. Fails when the
    /// link drops before it answers; the caller bounds the wait.
    pub async fn confirm_topic(
        &self,
        ctx: &TenantContext,
        topic: EventTopic,
    ) -> Result<u64, PubSubError> {
        let (reply, answer) = oneshot::channel();
        let topic_key = EventTopicKey::from_context(ctx, topic);
        self.subscription_tx
            .send(subscriber::SubscriptionCommand::Confirm(topic_key, reply))
            .await
            .map_err(|_| PubSubError::SubscriberStopped)?;
        answer.await.map_err(|_| PubSubError::SubscriberStopped)
    }

    /// Release local interest in a scoped Redis event topic.
    ///
    /// When the last retain is released, unsubscribe is delayed by the configured
    /// debounce. If another retain arrives during that delay, the pending
    /// unsubscribe becomes a no-op.
    pub async fn release_topic(&self, ctx: &TenantContext, topic: EventTopic) {
        let topic_key = EventTopicKey::from_context(ctx, topic);
        let became_zero = {
            let mut desired = self.desired_topics.lock().await;
            let Some(count) = desired.get_mut(&topic_key) else {
                tracing::warn!(?topic_key, "release_topic called for unretained topic");
                return;
            };

            *count -= 1;
            if *count == 0 {
                desired.remove(&topic_key);
                true
            } else {
                false
            }
        };

        if became_zero {
            let tx = self.subscription_tx.clone();
            let debounce = self.unsubscribe_debounce;
            tokio::spawn(async move {
                tokio::time::sleep(debounce).await;
                let _ = tx
                    .send(subscriber::SubscriptionCommand::UnsubscribeIfIdle(
                        topic_key,
                    ))
                    .await;
            });
        }
    }

    /// Current local desired refcount for tests and metrics.
    pub async fn topic_refcount(&self, ctx: &TenantContext, topic: EventTopic) -> usize {
        let topic_key = EventTopicKey::from_context(ctx, topic);
        self.desired_topics
            .lock()
            .await
            .get(&topic_key)
            .copied()
            .unwrap_or(0)
    }

    /// Returns a new broadcast receiver for cross-pod cache-invalidation drops.
    pub fn subscribe_cache_invalidations(&self) -> broadcast::Receiver<ScopedCacheInvalidation> {
        self.cache_invalidation_tx.subscribe()
    }

    /// Returns a new broadcast receiver for cross-pod connection-control commands.
    pub fn subscribe_conn_control(&self) -> broadcast::Receiver<ScopedConnControl> {
        self.conn_control_tx.subscribe()
    }

    /// Returns a new broadcast receiver for cross-pod NIP-FI disconnect commands.
    pub fn subscribe_nip_fi_disconnect(&self) -> broadcast::Receiver<NipFiDisconnect> {
        self.nip_fi_disconnect_tx.subscribe()
    }

    /// Publish a cache-key drop to all pods. Fire-and-forget at the call site:
    /// the local cache is already dropped synchronously; this carries the same
    /// drop cross-pod. A dropped publish is backstopped by the REQ denial-path
    /// DB confirmation, so callers may spawn this without awaiting delivery.
    pub async fn publish_cache_invalidation(
        &self,
        ctx: &TenantContext,
        invalidation: &CacheInvalidation,
    ) -> Result<i64, PubSubError> {
        let mut conn = self.pool.clone();
        let payload = serde_json::to_string(invalidation)?;
        let subscriber_count: i64 = redis::cmd("PUBLISH")
            .arg(cache_invalidation_channel(ctx))
            .arg(&payload)
            .query_async(&mut conn)
            .await?;
        Ok(subscriber_count)
    }

    /// Publish a connection-control command to all pods. Used for live ban
    /// enforcement: the banning pod disconnects any local sockets synchronously
    /// and calls this to reach the banned member's sockets on other pods. The DB
    /// ban row is the durable backstop, so a dropped publish still refuses the
    /// next auth attempt; callers may spawn this without awaiting delivery.
    pub async fn publish_conn_control(
        &self,
        ctx: &TenantContext,
        command: &ConnControl,
    ) -> Result<i64, PubSubError> {
        let mut conn = self.pool.clone();
        let payload = serde_json::to_string(command)?;
        let subscriber_count: i64 = redis::cmd("PUBLISH")
            .arg(conn_control_channel(ctx))
            .arg(&payload)
            .query_async(&mut conn)
            .await?;
        Ok(subscriber_count)
    }

    /// Publish a NIP-FI disconnect command to all pods listening on `channel`.
    ///
    /// Called after the local deny entry is inserted.  Remote pods receive this
    /// and apply the same `max(until)` merge + all-community session close.
    /// Fire-and-forget: the HTTP response does not wait on delivery.
    pub async fn publish_nip_fi_disconnect(
        &self,
        channel: &str,
        command: &NipFiDisconnect,
    ) -> Result<i64, PubSubError> {
        use crate::conn_control::encode_nip_fi_disconnect;
        let mut conn = self.pool.clone();
        let payload = encode_nip_fi_disconnect(command)?;
        let subscriber_count: i64 = redis::cmd("PUBLISH")
            .arg(channel)
            .arg(&payload)
            .query_async(&mut conn)
            .await?;
        Ok(subscriber_count)
    }

    /// Publish an event to the Redis channel. Returns subscriber count.
    ///
    /// Routing note (NIP-ER author-private reminders): events are keyed by
    /// `buzz:{community}:channel:{id}` / `buzz:{community}:global`, and
    /// relay nodes dynamically subscribe only to topics with local interest —
    /// so the topic key is a routing label, not an isolation boundary.
    /// Author-private reminders (kind:30300, stored under the nil channel
    /// sentinel) are therefore NOT protected by per-author Redis routing, and
    /// adding it would be pointless: the reminder's author may be connected to
    /// any node, so every node must still receive it. The actual author-only
    /// delivery boundary is `filter_fanout_by_access` in the relay, which runs
    /// on BOTH the in-process and the Redis cross-node (`subscribe_local`)
    /// fan-out paths and drops every recipient that is not the event author.
    /// Redis only ever carries events between nodes inside the relay trust
    /// domain; the ciphertext is NIP-44-encrypted to the author regardless.
    pub async fn publish_event(
        &self,
        ctx: &TenantContext,
        topic: EventTopic,
        event: &nostr::Event,
    ) -> Result<i64, PubSubError> {
        publisher::publish_event(&self.pool, ctx, topic, event).await
    }

    /// Set presence with 180s TTL. Call on connect and every 60s heartbeat.
    pub async fn set_presence(
        &self,
        ctx: &TenantContext,
        pubkey: &PublicKey,
        status: &str,
    ) -> Result<(), PubSubError> {
        presence::set_presence(&self.pool, ctx, pubkey, status).await
    }

    /// Remove presence for `pubkey`. Call on clean disconnect.
    pub async fn clear_presence(
        &self,
        ctx: &TenantContext,
        pubkey: &PublicKey,
    ) -> Result<(), PubSubError> {
        presence::clear_presence(&self.pool, ctx, pubkey).await
    }

    /// Set one status with the 180s TTL for each of `pubkeys`, in one round trip.
    pub async fn set_presence_many(
        &self,
        ctx: &TenantContext,
        pubkeys: &[PublicKey],
        status: &str,
    ) -> Result<(), PubSubError> {
        presence::set_presence_many(&self.pool, ctx, pubkeys, status).await
    }

    /// Remove presence for each of `pubkeys`.
    pub async fn clear_presence_many(
        &self,
        ctx: &TenantContext,
        pubkeys: &[PublicKey],
    ) -> Result<(), PubSubError> {
        presence::clear_presence_many(&self.pool, ctx, pubkeys).await
    }

    /// Returns the current presence status for `pubkey`, or `None` if not set.
    pub async fn get_presence(
        &self,
        ctx: &TenantContext,
        pubkey: &PublicKey,
    ) -> Result<Option<String>, PubSubError> {
        presence::get_presence(&self.pool, ctx, pubkey).await
    }

    /// Returns presence statuses for multiple pubkeys as a `pubkey_hex → status` map.
    pub async fn get_presence_bulk(
        &self,
        ctx: &TenantContext,
        pubkeys: &[PublicKey],
    ) -> Result<HashMap<String, String>, PubSubError> {
        presence::get_presence_bulk(&self.pool, ctx, pubkeys).await
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    pub fn make_test_pool() -> crate::RedisConnection {
        crate::redis_connection("redis://127.0.0.1:6379").expect("Failed to build Redis connection")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::make_test_pool;
    use buzz_core::{CommunityId, TenantContext};
    use nostr::{EventBuilder, Keys, Kind};
    use uuid::Uuid;

    async fn make_manager() -> Arc<PubSubManager> {
        let pool = make_test_pool();
        Arc::new(
            PubSubManager::new("redis://127.0.0.1:6379", pool)
                .await
                .expect("Failed to create PubSubManager"),
        )
    }

    fn ctx(id: u128, host: &str) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), host)
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_publish_and_subscribe_roundtrip() {
        let manager = make_manager().await;
        let mut rx = manager.subscribe_local();

        let manager_clone = manager.clone();
        tokio::spawn(async move { manager_clone.run_subscriber().await });
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let ctx = ctx(0xaaaa, "a.example");
        let channel_id = Uuid::new_v4();
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello pubsub")
            .tags([])
            .sign_with_keys(&keys)
            .expect("signing failed");
        let event_id = event.id;

        manager
            .retain_topic(&ctx, EventTopic::Channel(channel_id))
            .await;

        manager
            .publish_event(&ctx, EventTopic::Channel(channel_id), &event)
            .await
            .expect("publish failed");

        let received = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(received.community_id, ctx.community());
        assert_eq!(received.topic, EventTopic::Channel(channel_id));
        assert_eq!(received.event.id, event_id);
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_cache_invalidation_roundtrip() {
        let manager = make_manager().await;
        let mut rx = manager.subscribe_cache_invalidations();

        let manager_clone = manager.clone();
        tokio::spawn(async move { manager_clone.run_cache_invalidation_subscriber().await });
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let channel_id = Uuid::new_v4();
        let pubkey = Keys::generate().public_key().to_bytes().to_vec();
        let ctx = ctx(0xaaaa, "a.example");
        for sent in [
            CacheInvalidation::Membership {
                channel_id,
                pubkey: pubkey.clone(),
            },
            CacheInvalidation::CommunityHost {
                host: ctx.host().to_owned(),
            },
        ] {
            manager
                .publish_cache_invalidation(&ctx, &sent)
                .await
                .expect("publish failed");
            let received = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout")
                .expect("channel closed");
            assert_eq!(
                received,
                ScopedCacheInvalidation {
                    community_id: ctx.community(),
                    invalidation: sent,
                }
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_presence_set_and_get() {
        let pool = make_test_pool();
        let pubkey = Keys::generate().public_key();
        let ctx = ctx(0xaaaa, "a.example");

        let status = presence::get_presence(&pool, &ctx, &pubkey).await.unwrap();
        assert!(status.is_none());

        presence::set_presence(&pool, &ctx, &pubkey, "online")
            .await
            .unwrap();
        let status = presence::get_presence(&pool, &ctx, &pubkey).await.unwrap();
        assert_eq!(status.as_deref(), Some("online"));

        let mut conn = pool.clone();
        let ttl: i64 = redis::cmd("TTL")
            .arg(presence::presence_key(&ctx, &pubkey))
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(
            ttl > 0 && ttl <= presence::PRESENCE_TTL_SECS as i64,
            "TTL should be 1-{}s, got {ttl}",
            presence::PRESENCE_TTL_SECS
        );

        presence::clear_presence(&pool, &ctx, &pubkey)
            .await
            .unwrap();
        let status = presence::get_presence(&pool, &ctx, &pubkey).await.unwrap();
        assert!(status.is_none());
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn same_channel_id_in_two_communities_release_one_keeps_other_live() {
        let pool = make_test_pool();
        let manager = Arc::new(
            PubSubManager::with_config(
                PubSubConfig::new("redis://127.0.0.1:6379")
                    .with_unsubscribe_debounce(Duration::from_millis(25)),
                pool,
            )
            .await
            .expect("Failed to create PubSubManager"),
        );
        let mut rx = manager.subscribe_local();

        let manager_clone = manager.clone();
        tokio::spawn(async move { manager_clone.run_subscriber().await });
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let ctx_a = ctx(0xaaaa, "a.example");
        let ctx_b = ctx(0xbbbb, "b.example");
        let channel_id = Uuid::from_u128(0xcccc);
        let topic = EventTopic::Channel(channel_id);

        manager.retain_topic(&ctx_a, topic).await;
        manager.retain_topic(&ctx_b, topic).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        assert_eq!(manager.topic_refcount(&ctx_a, topic).await, 1);
        assert_eq!(manager.topic_refcount(&ctx_b, topic).await, 1);

        let keys = Keys::generate();
        let event_before_release = EventBuilder::new(Kind::TextNote, "before A release")
            .tags([])
            .sign_with_keys(&keys)
            .expect("signing failed");

        manager
            .publish_event(&ctx_b, topic, &event_before_release)
            .await
            .expect("publish before release failed");

        let received_before_release =
            tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout before release")
                .expect("channel closed before release");
        assert_eq!(received_before_release.community_id, ctx_b.community());
        assert_eq!(received_before_release.topic, topic);
        assert_eq!(received_before_release.event.id, event_before_release.id);

        manager.release_topic(&ctx_a, topic).await;
        assert_eq!(manager.topic_refcount(&ctx_a, topic).await, 0);
        assert_eq!(manager.topic_refcount(&ctx_b, topic).await, 1);

        // Wait past A's debounce. A buggy implementation that keyed active
        // Redis subscriptions by channel id alone would unsubscribe B here too.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let event_after_release = EventBuilder::new(Kind::TextNote, "after A release")
            .tags([])
            .sign_with_keys(&keys)
            .expect("signing failed");

        manager
            .publish_event(&ctx_b, topic, &event_after_release)
            .await
            .expect("publish after release failed");

        let received_after_release =
            tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
                .await
                .expect("timeout after release")
                .expect("channel closed after release");
        assert_eq!(received_after_release.community_id, ctx_b.community());
        assert_eq!(received_after_release.topic, topic);
        assert_eq!(received_after_release.event.id, event_after_release.id);

        manager.release_topic(&ctx_b, topic).await;
        assert_eq!(manager.topic_refcount(&ctx_b, topic).await, 0);
    }

    /// A retain past a full command queue neither waits for the subscriber
    /// (callers hold a lifecycle lock) nor loses the topic: it leaves a
    /// reconciliation wake-up, and the topic is in the desired map the
    /// subscriber re-reads. Retains that fit in the queue leave no wake-up,
    /// so first retains do not each make the subscriber rescan the map.
    /// Mutation: `send().await` instead of `try_send` hangs at the timeout
    /// below; no `notify_one` fails the wake-up check; `notify_one` on every
    /// first retain fails the no-wake-up check.
    #[tokio::test]
    async fn queue_full_retain_does_not_wait_and_schedules_reconciliation() {
        let manager = make_manager().await;
        let filler = ctx(0xaaaa, "a.example");
        for id in 0..SUBSCRIPTION_COMMAND_CAPACITY {
            manager
                .retain_topic(&filler, EventTopic::Channel(Uuid::from_u128(id as u128)))
                .await;
        }
        // Retains that fit in the queue leave no reconciliation wake-up.
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                manager.subscription_changed.notified()
            )
            .await
            .is_err(),
            "a queued subscribe needs no reconciliation"
        );

        let dropped_ctx = ctx(0xbbbb, "b.example");
        let dropped = EventTopic::Channel(Uuid::from_u128(0xcccc));
        tokio::time::timeout(
            Duration::from_secs(1),
            manager.retain_topic(&dropped_ctx, dropped),
        )
        .await
        .expect("a retain must not wait on a full command queue");
        assert_eq!(manager.topic_refcount(&dropped_ctx, dropped).await, 1);

        tokio::time::timeout(
            Duration::from_millis(10),
            manager.subscription_changed.notified(),
        )
        .await
        .expect("a queue-full retain must leave a reconciliation wake-up");
        let missing = subscriber::missing_desired_topics(
            &manager.desired_topics,
            &std::collections::HashSet::new(),
        )
        .await;
        assert!(missing.contains(&EventTopicKey::from_context(&dropped_ctx, dropped)));
    }

    /// Fill the command queue with first retains of fresh topics, then retain
    /// `topic`, without yielding: on this single-threaded runtime the
    /// subscriber cannot drain the queue in between, so `topic`'s subscribe
    /// hint is dropped.
    async fn retain_with_dropped_hint(
        manager: &PubSubManager,
        filler: &TenantContext,
        first_filler: u128,
        target: &TenantContext,
        topic: EventTopic,
    ) {
        tokio::task::unconstrained(async {
            let mut id = first_filler;
            while manager.subscription_tx.capacity() > 0 {
                manager
                    .retain_topic(filler, EventTopic::Channel(Uuid::from_u128(id)))
                    .await;
                id += 1;
            }
            manager.retain_topic(target, topic).await;
        })
        .await;
    }

    /// Publish to `topic` until the live link delivers it.
    async fn delivered(
        manager: &PubSubManager,
        rx: &mut broadcast::Receiver<ChannelEvent>,
        ctx: &TenantContext,
        topic: EventTopic,
    ) -> bool {
        let event = EventBuilder::new(Kind::TextNote, "reconciled")
            .sign_with_keys(&Keys::generate())
            .expect("sign");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                manager
                    .publish_event(ctx, topic, &event)
                    .await
                    .expect("publish");
                let wait = tokio::time::sleep(Duration::from_millis(100));
                tokio::pin!(wait);
                loop {
                    tokio::select! {
                        () = &mut wait => break,
                        received = rx.recv() => match received {
                            Ok(received) if received.event.id == event.id => return,
                            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(broadcast::error::RecvError::Closed) => panic!("closed"),
                        },
                    }
                }
            }
        })
        .await
        .is_ok()
    }

    /// A subscribe hint lost to a full command queue is recovered by the
    /// running subscriber on the same Redis link: first for a new topic, then
    /// for one released, unsubscribed, and retained again. The link never
    /// reconnects and nothing sends `Confirm` for the topic, so only the
    /// reconciliation wake-up can subscribe it. Mutation: drop the
    /// subscriber's `subscription_changed` arm, or the `notify_one` in
    /// `retain_topic` → the topic is never delivered → RED.
    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn live_subscriber_recovers_a_dropped_subscribe_hint() {
        let manager = Arc::new(
            PubSubManager::with_config(
                PubSubConfig::new("redis://127.0.0.1:6379")
                    .with_unsubscribe_debounce(Duration::from_millis(1)),
                make_test_pool(),
            )
            .await
            .unwrap(),
        );
        let mut rx = manager.subscribe_local();
        let mut link = manager.link_state();
        let subscriber = tokio::spawn(Arc::clone(&manager).run_subscriber());
        link.wait_for(|state| state.up).await.expect("link up");
        let epoch = link.borrow().epoch;

        let filler = ctx(Uuid::new_v4().as_u128(), "filler.example");
        let target = ctx(Uuid::new_v4().as_u128(), "target.example");
        let topic = EventTopic::Channel(Uuid::new_v4());

        retain_with_dropped_hint(&manager, &filler, 0, &target, topic).await;
        assert!(
            delivered(&manager, &mut rx, &target, topic).await,
            "a dropped hint for a new topic must be reconciled"
        );

        manager.release_topic(&target, topic).await;
        // Wait until Redis shows the debounced unsubscribe took effect, so
        // the re-retain below starts from an unsubscribed topic.
        let channel = EventTopicKey::from_context(&target, topic).redis_channel();
        let mut redis = redis::Client::open("redis://127.0.0.1:6379")
            .unwrap()
            .get_multiplexed_async_connection()
            .await
            .expect("redis");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let (_, subscribers): (String, i64) = redis::cmd("PUBSUB")
                    .arg("NUMSUB")
                    .arg(&channel)
                    .query_async(&mut redis)
                    .await
                    .expect("numsub");
                if subscribers == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the released topic must be unsubscribed");
        retain_with_dropped_hint(&manager, &filler, 1 << 32, &target, topic).await;
        assert!(
            delivered(&manager, &mut rx, &target, topic).await,
            "a dropped hint after an unsubscribe must be reconciled"
        );
        assert_eq!(link.borrow().epoch, epoch, "the link must not reconnect");
        subscriber.abort();
    }

    #[tokio::test]
    async fn retain_release_refcounts_and_debounces_last_release() {
        let pool = make_test_pool();
        let manager = PubSubManager::with_config(
            PubSubConfig::new("redis://127.0.0.1:6379")
                .with_unsubscribe_debounce(Duration::from_millis(1)),
            pool,
        )
        .await
        .unwrap();
        let ctx = ctx(0xaaaa, "a.example");
        let topic = EventTopic::Channel(Uuid::from_u128(0xbbbb));

        assert_eq!(manager.topic_refcount(&ctx, topic).await, 0);

        manager.retain_topic(&ctx, topic).await;
        manager.retain_topic(&ctx, topic).await;
        assert_eq!(manager.topic_refcount(&ctx, topic).await, 2);

        manager.release_topic(&ctx, topic).await;
        assert_eq!(manager.topic_refcount(&ctx, topic).await, 1);

        manager.release_topic(&ctx, topic).await;
        assert_eq!(manager.topic_refcount(&ctx, topic).await, 0);
    }

    #[test]
    fn config_defaults_debounce_but_allows_override() {
        let config = PubSubConfig::new("redis://example");
        assert_eq!(
            config.unsubscribe_debounce,
            PubSubConfig::DEFAULT_UNSUBSCRIBE_DEBOUNCE
        );

        let config = config.with_unsubscribe_debounce(Duration::from_millis(42));
        assert_eq!(config.unsubscribe_debounce, Duration::from_millis(42));
    }
}
