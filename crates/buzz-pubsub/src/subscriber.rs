//! Redis pub/sub subscriber — fans out messages to local WS connections via broadcast.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use nostr::JsonUtil;
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::topic::EventTopicKey;
use crate::ChannelEvent;

/// Initial reconnect backoff (1 second).
const BACKOFF_INITIAL_SECS: u64 = 1;
/// Maximum reconnect backoff (30 seconds).
const BACKOFF_MAX_SECS: u64 = 30;

/// Local desired topic refcounts, keyed by fully scoped Redis topic.
pub(crate) type DesiredTopics = Arc<Mutex<HashMap<EventTopicKey, usize>>>;

/// Commands sent from relay subscription registration/removal to the Redis
/// pub/sub task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubscriptionCommand {
    /// Ensure the topic is subscribed on the current Redis pub/sub connection.
    Subscribe(EventTopicKey),
    /// Unsubscribe only if the desired refcount is still zero when processed.
    UnsubscribeIfIdle(EventTopicKey),
}

/// Runs a dynamically scoped subscriber and forwards events to broadcast.
///
/// The desired refcount map is the source of truth. On every reconnect, this
/// task snapshots topics with count > 0 and subscribes to those exact Redis
/// channels before processing messages.
pub(crate) async fn run_subscriber(
    redis_url: String,
    broadcast_tx: broadcast::Sender<ChannelEvent>,
    desired_topics: DesiredTopics,
    mut subscription_rx: mpsc::Receiver<SubscriptionCommand>,
) {
    let mut backoff_secs = BACKOFF_INITIAL_SECS;

    loop {
        match connect_and_subscribe(
            &redis_url,
            &broadcast_tx,
            desired_topics.clone(),
            &mut subscription_rx,
        )
        .await
        {
            Ok(()) => {
                // Stream ended cleanly (Redis returned None). The connection was
                // established and ran successfully, so reset backoff to the initial
                // value — a brief Redis restart should reconnect quickly.
                backoff_secs = BACKOFF_INITIAL_SECS;
                tracing::warn!("Redis pub/sub stream ended (clean disconnect) — reconnecting in {backoff_secs}s");
            }
            Err(e) => {
                tracing::error!("Redis pub/sub error: {e} — reconnecting in {backoff_secs}s");
            }
        }

        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(BACKOFF_MAX_SECS);

        tracing::info!("Attempting to reconnect to Redis pub/sub...");
    }
}

/// Establish a Redis pub/sub connection, subscribe to the current desired-topic
/// snapshot, and run the fan-out / command loop until the connection ends.
async fn connect_and_subscribe(
    redis_url: &str,
    broadcast_tx: &broadcast::Sender<ChannelEvent>,
    desired_topics: DesiredTopics,
    subscription_rx: &mut mpsc::Receiver<SubscriptionCommand>,
) -> Result<(), redis::RedisError> {
    let client = redis::Client::open(redis_url)?;
    let conn = client.get_async_pubsub().await?;
    let (mut sink, mut stream) = conn.split();
    let mut active_topics = HashSet::new();

    let initial_topics: Vec<EventTopicKey> = {
        let desired = desired_topics.lock().await;
        desired
            .iter()
            .filter_map(|(topic, count)| (*count > 0).then_some(*topic))
            .collect()
    };

    for topic in initial_topics {
        let channel = topic.redis_channel();
        sink.subscribe(&channel).await?;
        active_topics.insert(channel);
    }

    tracing::info!(
        topic_count = active_topics.len(),
        "Redis pub/sub subscriber connected with dynamic scoped subscriptions"
    );

    loop {
        tokio::select! {
            Some(command) = subscription_rx.recv() => {
                match command {
                    SubscriptionCommand::Subscribe(topic) => {
                        let channel = topic.redis_channel();
                        if active_topics.insert(channel.clone()) {
                            sink.subscribe(&channel).await?;
                        }
                    }
                    SubscriptionCommand::UnsubscribeIfIdle(topic) => {
                        if desired_refcount(&desired_topics, topic).await == 0 {
                            let channel = topic.redis_channel();
                            if active_topics.remove(&channel) {
                                sink.unsubscribe(&channel).await?;
                            }
                        }
                    }
                }
            }
            msg = stream.next() => {
                let Some(msg) = msg else {
                    // Stream returned None — Redis connection closed.
                    return Ok(());
                };

                let payload: String = match msg.get_payload() {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("Failed to get pub/sub message payload: {e}");
                        continue;
                    }
                };

                let channel_name = msg.get_channel_name();
                let topic_key = match EventTopicKey::parse_redis_channel(channel_name) {
                    Ok(topic_key) => topic_key,
                    Err(_) => {
                        tracing::warn!("Received pub/sub message on unexpected channel: {channel_name}");
                        continue;
                    }
                };

                let event = match nostr::Event::from_json(&payload) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!("Failed to deserialize event from pub/sub: {e}");
                        continue;
                    }
                };

                let channel_event = ChannelEvent {
                    community_id: topic_key.community_id,
                    topic: topic_key.topic,
                    event,
                };

                if let Err(_e) = broadcast_tx.send(channel_event) {
                    tracing::trace!(topic = %channel_name, "No broadcast receivers for topic — message dropped");
                }
            }
            else => {
                // Command channel closed and stream ended; let the reconnect loop retry.
                return Ok(());
            }
        }
    }
}

async fn desired_refcount(desired_topics: &DesiredTopics, topic: EventTopicKey) -> usize {
    desired_topics
        .lock()
        .await
        .get(&topic)
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::{CommunityId, TenantContext};
    use uuid::Uuid;

    fn topic(id: u128) -> EventTopicKey {
        let ctx = TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(id)), "test");
        EventTopicKey::from_context(&ctx, crate::EventTopic::Global)
    }

    #[tokio::test]
    async fn desired_refcount_returns_zero_for_absent_topic() {
        let desired = Arc::new(Mutex::new(HashMap::new()));
        assert_eq!(desired_refcount(&desired, topic(1)).await, 0);
    }

    #[tokio::test]
    async fn desired_refcount_reads_present_topic() {
        let desired = Arc::new(Mutex::new(HashMap::from([(topic(1), 3)])));
        assert_eq!(desired_refcount(&desired, topic(1)).await, 3);
    }
}
