//! Event publishing — PUBLISH to Redis via pool connection.

use buzz_core::TenantContext;
use deadpool_redis::Pool;
use nostr::JsonUtil;
use uuid::Uuid;

use crate::error::PubSubError;
use crate::topic::{self, EventTopic};

/// Returns the Redis pub/sub channel key for `channel_id` under `ctx`.
pub fn channel_key(ctx: &TenantContext, channel_id: Uuid) -> String {
    topic::channel_key(ctx, channel_id)
}

/// Returns the Redis pub/sub channel key for community-global events under `ctx`.
pub fn global_key(ctx: &TenantContext) -> String {
    topic::global_key(ctx)
}

/// Returns the number of subscribers that received the message.
pub async fn publish_event(
    pool: &Pool,
    ctx: &TenantContext,
    topic: EventTopic,
    event: &nostr::Event,
) -> Result<i64, PubSubError> {
    let mut conn = pool.get().await?;
    let key = crate::topic::EventTopicKey::from_context(ctx, topic).redis_channel();
    let payload = event.as_json();
    let subscriber_count: i64 = redis::cmd("PUBLISH")
        .arg(&key)
        .arg(&payload)
        .query_async(&mut conn)
        .await?;
    Ok(subscriber_count)
}
