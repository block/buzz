//! Event publishing — PUBLISH to Redis via pool connection.

use deadpool_redis::Pool;
use nostr::JsonUtil;
use uuid::Uuid;

use crate::error::PubSubError;

/// Returns the Redis pub/sub channel key for `channel_id`.
pub fn channel_key(channel_id: Uuid) -> String {
    format!("sprout:channel:{}", channel_id)
}

/// Returns the number of subscribers that received the message.
pub async fn publish_event(
    pool: &Pool,
    channel_id: Uuid,
    event: &nostr::Event,
) -> Result<i64, PubSubError> {
    let mut conn = pool.get().await?;
    let key = channel_key(channel_id);
    let payload = event.as_json();
    let subscriber_count: i64 = redis::cmd("PUBLISH")
        .arg(&key)
        .arg(&payload)
        .query_async(&mut conn)
        .await?;
    Ok(subscriber_count)
}
