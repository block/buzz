#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-pubsub` — Redis pub/sub fan-out, presence tracking, and typing indicators.
//!
//! # Architecture
//!
//! ```text
//! buzz-relay process
//!   │
//!   ├── deadpool-redis pool → PUBLISH, SET, ZADD, etc.
//!   │
//!   └── dedicated redis::aio::PubSub connection (NOT from pool)
//!         └── dynamic SUBSCRIBE buzz:{community}:channel:{id} / buzz:{community}:global
//!               └── run_subscriber() → broadcast::channel(4096) → N WS receivers
//! ```
//!
//! The subscriber reconnects automatically on Redis disconnect with exponential
//! backoff (1s → 2s → 4s → … → 30s max).
//!
//! Dedicated pub/sub connection is stateful and cannot be shared.
//! Pool connections handle all other commands.
//! Lagged receivers get `RecvError::Lagged`.

/// Cross-pod cache-key invalidation over Redis pub/sub.
pub mod cache_invalidation;
/// Error types for pub/sub operations.
pub mod error;
/// Redis-backed NIP-98 replay seen-set.
pub mod nip98_replay;
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
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::cache_invalidation::{
    cache_invalidation_channel, CacheInvalidation, ScopedCacheInvalidation,
};
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

/// Central pub/sub manager for a Buzz relay instance.
pub struct PubSubManager {
    pool: deadpool_redis::Pool,
    /// Redis URL used by the reconnect loop to re-establish pub/sub connections.
    redis_url: String,
    /// Delay before unsubscribing after the last local interest is released.
    unsubscribe_debounce: Duration,
    /// Local desired topic refcounts; source of truth across Redis reconnects.
    desired_topics: subscriber::DesiredTopics,
    subscription_tx: mpsc::Sender<subscriber::SubscriptionCommand>,
    subscription_rx: Mutex<Option<mpsc::Receiver<subscriber::SubscriptionCommand>>>,
    broadcast_tx: broadcast::Sender<ChannelEvent>,
    cache_invalidation_tx: broadcast::Sender<ScopedCacheInvalidation>,
}

impl PubSubManager {
    /// Creates a new `PubSubManager` connected to the given Redis URL.
    pub async fn new(redis_url: &str, pool: deadpool_redis::Pool) -> Result<Self, PubSubError> {
        Self::with_config(PubSubConfig::new(redis_url), pool).await
    }

    /// Creates a new `PubSubManager` using explicit pub/sub configuration.
    pub async fn with_config(
        config: PubSubConfig,
        pool: deadpool_redis::Pool,
    ) -> Result<Self, PubSubError> {
        let (broadcast_tx, _) = broadcast::channel(4096);
        let (cache_invalidation_tx, _) = broadcast::channel(4096);
        let (subscription_tx, subscription_rx) = mpsc::channel(4096);

        Ok(Self {
            pool,
            redis_url: config.redis_url,
            unsubscribe_debounce: config.unsubscribe_debounce,
            desired_topics: Arc::new(Mutex::new(HashMap::new())),
            subscription_tx,
            subscription_rx: Mutex::new(Some(subscription_rx)),
            broadcast_tx,
            cache_invalidation_tx,
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

    /// Returns a new broadcast receiver for locally-published channel events.
    pub fn subscribe_local(&self) -> broadcast::Receiver<ChannelEvent> {
        self.broadcast_tx.subscribe()
    }

    /// Retain local interest in a scoped Redis event topic.
    ///
    /// The first retain for a topic asks the subscriber task to `SUBSCRIBE`.
    /// Additional retains only increment the local desired refcount.
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
            let _ = self
                .subscription_tx
                .send(subscriber::SubscriptionCommand::Subscribe(topic_key))
                .await;
        }
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

    /// Publish a cache-key drop to all pods. Fire-and-forget at the call site:
    /// the local cache is already dropped synchronously; this carries the same
    /// drop cross-pod. A dropped publish is backstopped by the REQ denial-path
    /// DB confirmation, so callers may spawn this without awaiting delivery.
    pub async fn publish_cache_invalidation(
        &self,
        ctx: &TenantContext,
        invalidation: &CacheInvalidation,
    ) -> Result<i64, PubSubError> {
        let mut conn = self.pool.get().await?;
        let payload = serde_json::to_string(invalidation)?;
        let subscriber_count: i64 = redis::cmd("PUBLISH")
            .arg(cache_invalidation_channel(ctx))
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

    /// Set presence with 60s TTL. Call on connect and every 30s heartbeat.
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
    pub fn make_test_pool() -> deadpool_redis::Pool {
        let cfg = deadpool_redis::Config::from_url("redis://127.0.0.1:6379");
        cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("Failed to create Redis pool")
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
        let sent = CacheInvalidation::Membership {
            channel_id,
            pubkey: pubkey.clone(),
        };

        let ctx = ctx(0xaaaa, "a.example");

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

        let mut conn = pool.get().await.unwrap();
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
