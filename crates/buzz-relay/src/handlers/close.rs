use std::sync::Arc;

use tracing::debug;

use crate::connection::ConnectionState;
use crate::handlers::req::SubscriptionTopics;
use crate::protocol::RelayMessage;
use crate::state::AppState;

/// Handle a CLOSE command — remove the subscription and send CLOSED acknowledgement.
pub async fn handle_close(sub_id: String, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let conn_id = conn.conn_id;

    remove_subscription(&sub_id, &conn, &state.sub_registry, state.pubsub.as_ref()).await;

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
    pubsub: &dyn SubscriptionTopics,
) {
    let mut pending_subscriptions = conn.pending_subscriptions.lock().await;
    if let Some(pending) = pending_subscriptions.remove(sub_id) {
        pending.cancel_lineage();
    }

    conn.subscriptions.lock().await.remove(sub_id);

    if let Some(removed) = registry.remove_subscription(conn.conn_id, sub_id) {
        if removed.scope.is_global() {
            pubsub
                .release_topic(&conn.tenant, buzz_pubsub::EventTopic::Global)
                .await;
        }
        for &channel_id in removed.scope.channel_ids() {
            pubsub
                .release_topic(&conn.tenant, buzz_pubsub::EventTopic::Channel(channel_id))
                .await;
        }
    }

    drop(pending_subscriptions);
}
