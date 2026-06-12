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
//!         └── PSUBSCRIBE buzz:channel:*
//!               └── run_subscriber() → broadcast::channel(4096) → N WS receivers
//! ```
//!
//! The subscriber reconnects automatically on Redis disconnect with exponential
//! backoff (1s → 2s → 4s → … → 30s max).
//!
//! Dedicated pub/sub connection is stateful and cannot be shared.
//! Pool connections handle all other commands.
//! Lagged receivers get `RecvError::Lagged`.

/// Error types for pub/sub operations.
pub mod error;
/// Online/offline presence tracking in Redis.
pub mod presence;
/// Redis PUBLISH for channel event fan-out.
pub mod publisher;
/// Redis-backed rate limiter (fixed-window INCR + EXPIRE).
pub mod rate_limiter;
/// Redis SUBSCRIBE for channel event delivery.
pub mod subscriber;
/// Typing indicator tracking in Redis.
pub use error::PubSubError;

use std::collections::HashMap;
use std::sync::Arc;

use nostr::PublicKey;
use tokio::sync::broadcast;
use uuid::Uuid;

/// A Nostr event received on a specific channel, broadcast to local subscribers.
#[derive(Debug, Clone)]
pub struct ChannelEvent {
    /// Channel the event belongs to.
    pub channel_id: Uuid,
    /// The Nostr event payload.
    pub event: nostr::Event,
}

/// Configuration for the pub/sub subsystem.
#[derive(Debug, Clone)]
pub struct PubSubConfig {
    /// Redis connection URL (e.g. `redis://127.0.0.1:6379`).
    pub redis_url: String,
}

impl PubSubConfig {
    /// Creates a new `PubSubConfig` with the given Redis URL.
    pub fn new(redis_url: impl Into<String>) -> Self {
        Self {
            redis_url: redis_url.into(),
        }
    }
}

/// Central pub/sub manager for a Buzz relay instance.
pub struct PubSubManager {
    pool: deadpool_redis::Pool,
    /// Redis URL used by the reconnect loop to re-establish pub/sub connections.
    redis_url: String,
    broadcast_tx: broadcast::Sender<ChannelEvent>,
}

impl PubSubManager {
    /// Creates a new `PubSubManager` connected to the given Redis URL.
    pub async fn new(redis_url: &str, pool: deadpool_redis::Pool) -> Result<Self, PubSubError> {
        let (broadcast_tx, _) = broadcast::channel(4096);

        Ok(Self {
            pool,
            redis_url: redis_url.to_string(),
            broadcast_tx,
        })
    }

    /// Starts the pub/sub fan-out loop with automatic reconnection.
    ///
    /// Runs forever — spawn this in a background task. The loop reconnects
    /// with exponential backoff on Redis disconnect (1s → 2s → 4s → … → 30s).
    pub async fn run_subscriber(self: Arc<Self>) {
        subscriber::run_subscriber(self.redis_url.clone(), self.broadcast_tx.clone()).await;
    }

    /// Returns a new broadcast receiver for locally-published channel events.
    pub fn subscribe_local(&self) -> broadcast::Receiver<ChannelEvent> {
        self.broadcast_tx.subscribe()
    }

    /// Publish an event to the Redis channel. Returns subscriber count.
    pub async fn publish_event(
        &self,
        channel_id: Uuid,
        event: &nostr::Event,
    ) -> Result<i64, PubSubError> {
        publisher::publish_event(&self.pool, channel_id, event).await
    }

    /// Set presence with 60s TTL. Call on connect and every 30s heartbeat.
    pub async fn set_presence(&self, pubkey: &PublicKey, status: &str) -> Result<(), PubSubError> {
        presence::set_presence(&self.pool, pubkey, status).await
    }

    /// Remove presence for `pubkey`. Call on clean disconnect.
    pub async fn clear_presence(&self, pubkey: &PublicKey) -> Result<(), PubSubError> {
        presence::clear_presence(&self.pool, pubkey).await
    }

    /// Returns the current presence status for `pubkey`, or `None` if not set.
    pub async fn get_presence(&self, pubkey: &PublicKey) -> Result<Option<String>, PubSubError> {
        presence::get_presence(&self.pool, pubkey).await
    }

    /// Returns presence statuses for multiple pubkeys as a `pubkey_hex → status` map.
    pub async fn get_presence_bulk(
        &self,
        pubkeys: &[PublicKey],
    ) -> Result<HashMap<String, String>, PubSubError> {
        presence::get_presence_bulk(&self.pool, pubkeys).await
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
    use nostr::{EventBuilder, Keys, Kind};

    async fn make_manager() -> Arc<PubSubManager> {
        let pool = make_test_pool();
        Arc::new(
            PubSubManager::new("redis://127.0.0.1:6379", pool)
                .await
                .expect("Failed to create PubSubManager"),
        )
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_publish_and_subscribe_roundtrip() {
        let manager = make_manager().await;
        let mut rx = manager.subscribe_local();

        let manager_clone = manager.clone();
        tokio::spawn(async move { manager_clone.run_subscriber().await });
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let channel_id = Uuid::new_v4();
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::TextNote, "hello pubsub")
            .tags([])
            .sign_with_keys(&keys)
            .expect("signing failed");
        let event_id = event.id;

        manager
            .publish_event(channel_id, &event)
            .await
            .expect("publish failed");

        let received = tokio::time::timeout(tokio::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert_eq!(received.channel_id, channel_id);
        assert_eq!(received.event.id, event_id);
    }

    #[tokio::test]
    #[ignore = "requires Redis"]
    async fn test_presence_set_and_get() {
        let pool = make_test_pool();
        let pubkey = Keys::generate().public_key();

        let status = presence::get_presence(&pool, &pubkey).await.unwrap();
        assert!(status.is_none());

        presence::set_presence(&pool, &pubkey, "online")
            .await
            .unwrap();
        let status = presence::get_presence(&pool, &pubkey).await.unwrap();
        assert_eq!(status.as_deref(), Some("online"));

        let mut conn = pool.get().await.unwrap();
        let ttl: i64 = redis::cmd("TTL")
            .arg(presence::presence_key(&pubkey))
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(
            ttl > 0 && ttl <= presence::PRESENCE_TTL_SECS as i64,
            "TTL should be 1-{}s, got {ttl}",
            presence::PRESENCE_TTL_SECS
        );

        presence::clear_presence(&pool, &pubkey).await.unwrap();
        let status = presence::get_presence(&pool, &pubkey).await.unwrap();
        assert!(status.is_none());
    }
}
