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

/// Refuse a `REQ` or sync `REQ` as it arrives, before it is admitted, with
/// `CLOSED reason`. A `REQ` replaces whatever held its ID (NIP-01), and
/// `CLOSED` tells the client the ID is closed, so the old holder and any
/// request still pending for the ID are retired before the frame, under the
/// lifecycle lock. Only the connection's read loop calls this, in arrival
/// order; an admitted request refuses with `close_if_owner`. If the frame
/// cannot be queued, the connection is cancelled: the client would otherwise
/// never learn that the old holder is gone.
pub(crate) async fn refuse(sub_id: &str, reason: &str, conn: &ConnectionState, state: &AppState) {
    let mut subs = conn.subscriptions.lock().await;
    retire_locked(&mut subs, sub_id, conn, state).await;
    if !conn.send(RelayMessage::closed(sub_id, reason)) {
        conn.cancel.cancel();
    }
}

/// Queue `frame` for `sub_id` only while the request holding `owner` still
/// owns the ID. A request that a newer same-ID REQ replaced sends nothing:
/// its late `EOSE` would otherwise tell a replacing sync that a set it never
/// sent is complete. Returns whether the frame was queued.
pub(crate) async fn send_if_owner(
    sub_id: &str,
    owner: u64,
    frame: String,
    conn: &ConnectionState,
) -> bool {
    #[cfg(test)]
    test_seam::pause_at(&test_seam::SEND_PAUSE).await;
    let subs = conn.subscriptions.lock().await;
    owns(&subs, sub_id, owner) && conn.send(frame)
}

/// Monotonic source of subscription owner tokens. Each REQ takes its sub ID
/// with a fresh token when it arrives, so a request that a later same-ID REQ
/// or a `CLOSE` superseded can tell it no longer owns the ID.
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_owner() -> u64 {
    NEXT_OWNER.fetch_add(1, Ordering::Relaxed)
}

/// What the lifecycle map holds for a subscription ID: the token of the
/// newest admitted request, and whether that request has claimed the ID.
/// Until it claims, whatever is registered for the ID belongs to an older
/// request it is replacing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubOwner {
    /// The request's token from `next_owner`.
    pub owner: u64,
    /// The request has claimed the ID: what is registered for it is its own.
    pub claimed: bool,
}

/// Whether the request holding `owner` still owns `sub_id`. The caller holds
/// the lifecycle lock.
pub(crate) fn owns(subs: &HashMap<String, SubOwner>, sub_id: &str, owner: u64) -> bool {
    subs.get(sub_id).is_some_and(|held| held.owner == owner)
}

/// Claim `sub_id` for the request holding `owner`: `false` once the
/// connection is closing or a `CLOSE` or newer same-ID REQ superseded it.
/// The caller holds the lifecycle lock and, on `true`, replaces whatever is
/// registered for the ID in the same critical section.
pub(crate) fn claim_locked(
    subs: &mut HashMap<String, SubOwner>,
    sub_id: &str,
    owner: u64,
    conn: &ConnectionState,
) -> bool {
    if conn.cancel.is_cancelled() {
        return false;
    }
    match subs.get_mut(sub_id) {
        Some(held) if held.owner == owner => {
            held.claimed = true;
            true
        }
        _ => false,
    }
}

/// Why `admit` did not admit a request.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum NotAdmitted {
    /// The connection is closing; the request exits silently.
    Closing,
    /// A new ID past `MAX_SUBSCRIPTIONS`; `CLOSED` was queued.
    TooManySubscriptions,
}

/// Admit a `REQ` or sync `REQ` for `sub_id` as it arrives, before its handler
/// is spawned, and return its owner token. Admission order is arrival order:
/// the newest request owns the ID from this point, so an older request still
/// in its access checks can no longer claim, refuse, or send for the ID, and a
/// `CLOSE` retires a request that has not yet claimed. The previous holder
/// stays registered until this request claims or is refused.
pub(crate) async fn admit(sub_id: &str, conn: &ConnectionState) -> Result<u64, NotAdmitted> {
    let mut subs = conn.subscriptions.lock().await;
    if conn.cancel.is_cancelled() {
        return Err(NotAdmitted::Closing);
    }
    if !subs.contains_key(sub_id) && subs.len() >= super::req::MAX_SUBSCRIPTIONS {
        // A new ID has no old holder to retire, but a lost CLOSED would leave
        // the client waiting on a refused request.
        if !conn.send(RelayMessage::closed(
            sub_id,
            "error: too many subscriptions",
        )) {
            conn.cancel.cancel();
        }
        return Err(NotAdmitted::TooManySubscriptions);
    }
    let owner = next_owner();
    subs.insert(
        sub_id.to_owned(),
        SubOwner {
            owner,
            claimed: false,
        },
    );
    Ok(owner)
}

// Lifecycle lock: `conn.subscriptions` serializes every change to a
// connection's subscription state — map token, registry entry, topic
// retention, and the terminal CLOSED frame. Neither the registry nor pubsub
// takes it, so holding it across their awaits cannot invert lock order. The
// map holds, per ID, the newest admitted request's `SubOwner`, whether it has
// claimed (registered) yet or is still pending.

/// Drop `sub_id` from the map, the fan-out index, and its topic retention.
/// The caller holds the lifecycle lock (`subs`).
pub(crate) async fn retire_locked(
    subs: &mut HashMap<String, SubOwner>,
    sub_id: &str,
    conn: &ConnectionState,
    state: &AppState,
) {
    subs.remove(sub_id);
    retire_holder_locked(sub_id, conn, state).await;
}

/// Drop the registered holder of `sub_id` from the fan-out index and its topic
/// retention, but keep the map token: the newer request that owns the ID is
/// replacing it. The caller holds the lifecycle lock.
pub(crate) async fn retire_holder_locked(sub_id: &str, conn: &ConnectionState, state: &AppState) {
    state.artifact_syncs.remove(conn.conn_id, sub_id);
    if let Some(removed) = state.sub_registry.remove_subscription(conn.conn_id, sub_id) {
        #[cfg(test)]
        test_seam::pause_at(&test_seam::RELEASE_PAUSE).await;
        release_scope_topics(state, &conn.tenant, &removed.scope).await;
    }
}

/// Retire `sub_id` only if the request holding `owner` still owns it, then
/// send `closed` (if any) before releasing the lifecycle lock, so a replacing
/// REQ can never claim the ID between the teardown and its terminal frame.
/// Returns whether this request still owned the ID. A superseded request
/// touches nothing and says nothing. An admitted request refuses through here:
/// owning the ID, it retires the previous holder (NIP-01 replacement) before
/// its `CLOSED`.
pub(crate) async fn close_if_owner(
    sub_id: &str,
    owner: u64,
    closed: Option<&str>,
    conn: &ConnectionState,
    state: &AppState,
) -> bool {
    close_owned(sub_id, owner, None, closed, conn, state).await
}

/// `close_if_owner` with `CLOSED reason`, preceded by `NOTICE notice` in the
/// same critical section, so a superseded request sends neither.
pub(crate) async fn refuse_if_owner_with_notice(
    sub_id: &str,
    owner: u64,
    notice: &str,
    reason: &str,
    conn: &ConnectionState,
    state: &AppState,
) -> bool {
    close_owned(sub_id, owner, Some(notice), Some(reason), conn, state).await
}

async fn close_owned(
    sub_id: &str,
    owner: u64,
    notice: Option<&str>,
    closed: Option<&str>,
    conn: &ConnectionState,
    state: &AppState,
) -> bool {
    let mut subs = conn.subscriptions.lock().await;
    if !owns(&subs, sub_id, owner) {
        return false;
    }
    if let Some(notice) = notice {
        conn.send(RelayMessage::notice(notice));
    }
    retire_locked(&mut subs, sub_id, conn, state).await;
    #[cfg(test)]
    test_seam::pause_at(&test_seam::PAUSE).await;
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
    #[cfg(test)]
    test_seam::pause_at(&test_seam::CLEANUP_PAUSE).await;
    let mut subs = conn.subscriptions.lock().await;
    subs.clear();
    state.artifact_syncs.remove_connection(conn.conn_id);
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

/// Test-only pause points, scoped to one task so parallel tests don't
/// interfere: `PAUSE` inside `close_if_owner` between teardown and the
/// terminal frame, `RELEASE_PAUSE` inside `retire_locked` between the
/// registry removal and the topic release, `CLEANUP_PAUSE` at the start of
/// `release_connection_subscriptions`.
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
        pub(crate) static RELEASE_PAUSE: Arc<Pause>;
        pub(crate) static CLEANUP_PAUSE: Arc<Pause>;
        pub(crate) static SEND_PAUSE: Arc<Pause>;
    }

    pub(crate) async fn pause_at(point: &'static tokio::task::LocalKey<Arc<Pause>>) {
        if let Ok(pause) = point.try_with(Arc::clone) {
            pause.reached.notify_one();
            pause.resume.notified().await;
        }
    }
}
