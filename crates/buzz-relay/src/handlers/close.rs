use std::sync::Arc;

use tracing::debug;

use crate::connection::ConnectionState;
use crate::protocol::RelayMessage;
use crate::state::AppState;
use buzz_pubsub::EventTopic;

/// Handle a CLOSE command — remove the subscription and send CLOSED acknowledgement.
pub async fn handle_close(sub_id: String, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let conn_id = conn.conn_id;

    remove_subscription(&sub_id, &conn, &state.sub_registry, &state.pubsub).await;

    conn.send(RelayMessage::closed(&sub_id, ""));

    debug!(conn_id = %conn_id, sub_id = %sub_id, "Subscription closed");
}

/// Cancel an in-flight REQ or remove its committed subscription. The pending
/// lock is held through topic release to serialize this cleanup with the REQ's
/// registration/topic-retain commit.
pub(crate) async fn remove_subscription(
    sub_id: &str,
    conn: &ConnectionState,
    registry: &crate::subscription::SubscriptionRegistry,
    pubsub: &buzz_pubsub::PubSubManager,
) {
    // Serialize CLOSE with the REQ registration/topic-retain commit. If the
    // REQ is still awaiting access checks, removing and cancelling its pending
    // lease makes the later registration check fail closed.
    let mut pending_subscriptions = conn.pending_subscriptions.lock().await;
    if let Some(pending) = pending_subscriptions.remove(sub_id) {
        pending.cancel();
    }

    conn.subscriptions.lock().await.remove(sub_id);

    // Deregister from the fan-out index before sending CLOSED so no new
    // messages are routed to this sub after the client's CLOSE is acknowledged.
    if let Some(removed) = registry.remove_subscription(conn.conn_id, sub_id) {
        pubsub
            .release_topic(&conn.tenant, topic_for_subscription(removed.channel_id))
            .await;
    }

    drop(pending_subscriptions);
}

fn topic_for_subscription(channel_id: Option<uuid::Uuid>) -> EventTopic {
    match channel_id {
        Some(channel_id) => EventTopic::Channel(channel_id),
        None => EventTopic::Global,
    }
}
