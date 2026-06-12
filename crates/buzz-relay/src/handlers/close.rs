use std::sync::Arc;

use tracing::debug;

use crate::connection::ConnectionState;
use crate::protocol::RelayMessage;
use crate::state::AppState;

/// Handle a CLOSE command — remove the subscription and send CLOSED acknowledgement.
pub async fn handle_close(sub_id: String, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    let conn_id = conn.conn_id;

    conn.subscriptions.lock().await.remove(&sub_id);

    // Deregister from the fan-out index before sending CLOSED so no new
    // messages are routed to this sub after the client's CLOSE is acknowledged.
    state.sub_registry.remove_subscription(conn_id, &sub_id);

    conn.send(RelayMessage::closed(&sub_id, ""));

    debug!(conn_id = %conn_id, sub_id = %sub_id, "Subscription closed");
}
