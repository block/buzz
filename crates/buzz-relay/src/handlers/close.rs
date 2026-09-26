use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use buzz_core::tenant::TenantContext;
use buzz_pubsub::EventTopic;

use tracing::debug;

use crate::connection::ConnectionState;
use crate::protocol::RelayMessage;
use crate::state::AppState;
use crate::subscription::SubscriptionScope;

/// Handle a CLOSE command — remove the subscription and send CLOSED acknowledgement.
pub async fn handle_close(sub_id: String, conn: Arc<ConnectionState>, state: Arc<AppState>) {
    // Client CLOSE targets whatever currently holds the ID, so it is
    // unconditional. Retiring before CLOSED means nothing is routed to this
    // sub after the acknowledgement.
    let mut subs = conn.subscriptions.lock().await;
    retire_locked(&mut subs, &sub_id, &conn, &state).await;
    conn.send(RelayMessage::closed(&sub_id, ""));
    drop(subs);

    debug!(conn_id = %conn.conn_id, sub_id = %sub_id, "Subscription closed");
}

/// Monotonic source of subscription owner tokens. Each REQ claims its sub ID
/// with a fresh token, so a superseded request can tell it no longer owns it.
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_owner() -> u64 {
    NEXT_OWNER.fetch_add(1, Ordering::Relaxed)
}

// Lifecycle lock: `conn.subscriptions` serializes every change to a
// connection's subscription state — map token, registry entry, topic
// retention, and the terminal CLOSED frame. Neither the registry nor pubsub
// takes it, so holding it across their awaits cannot invert lock order.

/// Drop `sub_id` from the map, the fan-out index, and its topic retention.
/// The caller holds the lifecycle lock (`subs`).
pub(crate) async fn retire_locked(
    subs: &mut HashMap<String, u64>,
    sub_id: &str,
    conn: &ConnectionState,
    state: &AppState,
) {
    subs.remove(sub_id);
    if let Some(removed) = state.sub_registry.remove_subscription(conn.conn_id, sub_id) {
        release_scope_topics(state, &conn.tenant, &removed.scope).await;
    }
}

/// Retire `sub_id` only if the request holding `owner` still owns it, then
/// send `closed` (if any) before releasing the lifecycle lock, so a replacing
/// REQ can never claim the ID between the teardown and its terminal frame.
/// Returns whether this request still owned the ID. A superseded request
/// touches nothing and says nothing.
pub(crate) async fn close_if_owner(
    sub_id: &str,
    owner: u64,
    closed: Option<&str>,
    conn: &ConnectionState,
    state: &AppState,
) -> bool {
    let mut subs = conn.subscriptions.lock().await;
    if subs.get(sub_id) != Some(&owner) {
        return false;
    }
    retire_locked(&mut subs, sub_id, conn, state).await;
    #[cfg(test)]
    test_seam::pause().await;
    if let Some(reason) = closed {
        if !conn.send(RelayMessage::closed(sub_id, reason)) {
            // The outbound channel is full or closed: the terminal frame is
            // lost. Cancel the connection so the subscription is not silently
            // orphaned — the client will reconnect and resubscribe.
            conn.cancel.cancel();
        }
    }
    true
}

/// Final subscription cleanup for a closed connection. Its cancellation token
/// must already be cancelled: claims check it under the lifecycle lock, so no
/// detached REQ task can register after this runs.
pub(crate) async fn release_connection_subscriptions(conn: &ConnectionState, state: &AppState) {
    debug_assert!(conn.cancel.is_cancelled());
    let mut subs = conn.subscriptions.lock().await;
    subs.clear();
    for removed in state.sub_registry.remove_connection(conn.conn_id) {
        release_scope_topics(state, &conn.tenant, &removed.scope).await;
    }
}

/// Release the pubsub topics a subscription scope retained.
pub(crate) async fn release_scope_topics(
    state: &AppState,
    tenant: &TenantContext,
    scope: &SubscriptionScope,
) {
    if scope.is_global() {
        state.pubsub.release_topic(tenant, EventTopic::Global).await;
    }
    for &channel_id in scope.channel_ids() {
        state
            .pubsub
            .release_topic(tenant, EventTopic::Channel(channel_id))
            .await;
    }
}

/// Test-only pause point inside `close_if_owner`, between teardown and the
/// terminal frame, scoped to one task so parallel tests don't interfere.
#[cfg(test)]
pub(crate) mod test_seam {
    use std::sync::Arc;
    use tokio::sync::Notify;

    #[derive(Default)]
    pub(crate) struct Pause {
        pub(crate) reached: Notify,
        pub(crate) resume: Notify,
    }

    tokio::task_local! {
        pub(crate) static PAUSE: Arc<Pause>;
    }

    pub(super) async fn pause() {
        if let Ok(pause) = PAUSE.try_with(Arc::clone) {
            pause.reached.notify_one();
            pause.resume.notified().await;
        }
    }
}
