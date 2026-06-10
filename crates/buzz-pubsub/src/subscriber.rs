//! Redis pub/sub subscriber — fans out messages to local WS connections via broadcast.

use futures_util::StreamExt;
use nostr::JsonUtil;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::ChannelEvent;

/// Initial reconnect backoff (1 second).
const BACKOFF_INITIAL_SECS: u64 = 1;
/// Maximum reconnect backoff (30 seconds).
const BACKOFF_MAX_SECS: u64 = 30;

/// Pattern-subscribes to `sprout:channel:*` and forwards events to broadcast.
///
/// Runs a reconnect loop with exponential backoff (1s → 2s → 4s → … → 30s max).
/// Logs `error!` on disconnect and `info!` on successful reconnect.
/// Never returns — the task runs for the lifetime of the relay.
pub async fn run_subscriber(redis_url: String, broadcast_tx: broadcast::Sender<ChannelEvent>) {
    let mut backoff_secs = BACKOFF_INITIAL_SECS;

    loop {
        match connect_and_subscribe(&redis_url, &broadcast_tx).await {
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

        tokio::time::sleep(tokio::time::Duration::from_secs(backoff_secs)).await;
        backoff_secs = (backoff_secs * 2).min(BACKOFF_MAX_SECS);

        tracing::info!("Attempting to reconnect to Redis pub/sub...");
    }
}

/// Establish a Redis pub/sub connection, subscribe, and run the fan-out loop
/// until the stream ends or an error occurs.
///
/// Returns `Ok(())` if the stream ends cleanly (disconnect), `Err` on
/// connection or subscription failure.
async fn connect_and_subscribe(
    redis_url: &str,
    broadcast_tx: &broadcast::Sender<ChannelEvent>,
) -> Result<(), redis::RedisError> {
    let client = redis::Client::open(redis_url)?;
    let mut conn = client.get_async_pubsub().await?;

    conn.psubscribe("sprout:channel:*").await?;

    tracing::info!("Redis pub/sub subscriber connected — listening on sprout:channel:*");

    // Note: backoff is NOT reset here on connect. It resets in the outer loop
    // only after this function returns Ok(()) — i.e., after the connection ran
    // to completion (natural disconnect). A transient connect that immediately
    // drops would not reset backoff.

    let mut stream = conn.on_message();
    while let Some(msg) = stream.next().await {
        let payload: String = match msg.get_payload() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("Failed to get pub/sub message payload: {e}");
                continue;
            }
        };

        let channel_name = msg.get_channel_name();
        let channel_id = channel_name
            .strip_prefix("sprout:channel:")
            .and_then(|s| Uuid::parse_str(s).ok());

        let channel_id = match channel_id {
            Some(id) => id,
            None => {
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

        let channel_event = ChannelEvent { channel_id, event };

        if let Err(_e) = broadcast_tx.send(channel_event) {
            tracing::trace!("No broadcast receivers for channel {channel_id} — message dropped");
        }
    }

    // Stream returned None — Redis connection closed.
    Ok(())
}
