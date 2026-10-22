//! NIP-AR Sync: a `REQ` that keeps an exact copy of a set of artifacts.
//!
//! The relay never forwards the published event to a sync. A live entry only
//! says that one `d` changed; the sync task then reads that artifact's current
//! head and sends it (NIP-AR Sync rules 3 and 4). Any possible loss of a live
//! entry ends the sync with `CLOSED "resync: ..."` (rule 5).
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use axum::extract::ws::Message as WsMessage;
use buzz_auth::Scope;
use buzz_core::artifact_sync::{ArtifactSyncFilter, MAX_SYNC_EVENTS};
use buzz_core::StoredEvent;
use buzz_pubsub::EventTopic;
use dashmap::DashMap;
use nostr::{Alphabet, Event, Filter, Kind, SingleLetterTag};
use tokio::sync::{Notify, OwnedSemaphorePermit};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use crate::connection::{AuthState, ConnectionState};
use crate::protocol::RelayMessage;
use crate::state::AppState;
use crate::subscription::{ConnId, SubId};

/// Heads read per snapshot page. One page is the most the relay buffers.
const SNAPSHOT_PAGE: i64 = 256;
/// The longest the relay holds one sync's snapshot open, including the time
/// a slow reader takes to receive it (rule 2). Not advertised.
const SNAPSHOT_BUDGET: Duration = Duration::from_secs(30);
/// How long a new sync waits for Redis to confirm its live interest (rule 1).
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
/// Kind-9005 redaction; it can retire a head that is in scope (rule 6).
const KIND_REDACTION: u16 = 9005;
/// NIP-29 kinds that can change a reader's access to a channel: remove user,
/// edit metadata (visibility), delete group, leave (rule 9).
const ACCESS_KINDS: [u16; 4] = [9001, 9002, 9008, 9022];
/// Every kind a sync consumes from live fan-out. A failed publish of one of
/// these is retried (rule 5).
pub(crate) const LIVE_KINDS: [u16; 7] = [45010, 45011, KIND_REDACTION, 9001, 9002, 9008, 9022];

/// Live changes held for one sync until its task reads their heads.
#[derive(Default)]
struct Pending {
    /// Changed artifacts, with the latest kind-45011 removal seen for each.
    changed: HashMap<Uuid, Option<Event>>,
    /// The first reason this sync must resync, if any.
    resync: Option<&'static str>,
}

/// One live sync's signal inbox. Fan-out and loss detection write to it; the
/// sync's own task is its only reader.
pub struct SyncHandle {
    pending: StdMutex<Pending>,
    notify: Notify,
    cancel: CancellationToken,
}

impl SyncHandle {
    fn new() -> Self {
        Self {
            pending: StdMutex::new(Pending::default()),
            notify: Notify::new(),
            cancel: CancellationToken::new(),
        }
    }

    fn update(&self, change: impl FnOnce(&mut Pending)) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut pending);
        if pending.changed.len() > MAX_SYNC_EVENTS {
            pending.resync.get_or_insert("resync: too many changes");
        }
        drop(pending);
        self.notify.notify_one();
    }

    /// Record a fanned-out event that falls in this sync's channels.
    fn signal(&self, stored: &StoredEvent) {
        let kind = stored.event.kind.as_u16();
        if kind == KIND_REDACTION {
            // The redacted head is withheld, so only a new sync can show the
            // client that the artifact left the set. Any redaction in the
            // channels counts: telling which ones retire a head needs a read.
            return self.resync("resync: redaction");
        }
        if ACCESS_KINDS.contains(&kind) {
            return self.resync("resync: channel access changed");
        }
        let Some(d) = stored
            .event
            .tags
            .iter()
            .find(|tag| tag.as_slice().first().map(String::as_str) == Some("d"))
            .and_then(|tag| tag.as_slice().get(1)?.parse::<Uuid>().ok())
        else {
            return;
        };
        let removal = (kind == 45011).then(|| stored.event.clone());
        self.update(|pending| {
            let entry = pending.changed.entry(d).or_default();
            if removal.is_some() {
                *entry = removal;
            }
        });
    }

    /// Ask the sync to end with `CLOSED reason`, where reason is `resync: ...`.
    pub fn resync(&self, reason: &'static str) {
        self.update(|pending| {
            pending.resync.get_or_insert(reason);
        });
    }

    fn take(&self) -> Pending {
        std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Every live sync this node serves, by connection and subscription ID.
#[derive(Default)]
pub struct ArtifactSyncs {
    syncs: DashMap<(ConnId, SubId), Arc<SyncHandle>>,
}

impl ArtifactSyncs {
    fn insert(&self, conn: ConnId, sub: &str, handle: Arc<SyncHandle>) {
        if let Some(old) = self.syncs.insert((conn, sub.to_owned()), handle) {
            old.cancel.cancel();
        }
    }

    /// Stop the sync that holds `sub` on `conn`, if one does. Called wherever
    /// the subscription ID is retired or replaced.
    /// Returns whether `sub` was a sync.
    pub fn remove(&self, conn: ConnId, sub: &str) -> bool {
        match self.syncs.remove(&(conn, sub.to_owned())) {
            Some((_, handle)) => {
                handle.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Whether `sub` on `conn` is a sync.
    pub fn contains(&self, conn: ConnId, sub: &str) -> bool {
        self.syncs.contains_key(&(conn, sub.to_owned()))
    }

    /// Stop every sync of a closed connection.
    pub fn remove_connection(&self, conn: ConnId) {
        self.syncs.retain(|(owner, _), handle| {
            if *owner == conn {
                handle.cancel.cancel();
            }
            *owner != conn
        });
    }

    /// End every sync on this node: a loss here can affect any of them.
    pub fn resync_all(&self, reason: &'static str) {
        for entry in self.syncs.iter() {
            entry.value().resync(reason);
        }
    }

    /// Signal syncs among the fan-out matches *before* the access filter. A
    /// reader who just lost access to a channel no longer passes that filter,
    /// yet the change that removed it must still resync its syncs (rule 9).
    /// Only kinds that resync are signalled here; nothing is sent.
    pub fn signal_access_change(&self, stored: &StoredEvent, matches: &[(ConnId, SubId)]) {
        let kind = stored.event.kind.as_u16();
        if self.syncs.is_empty() || !(kind == KIND_REDACTION || ACCESS_KINDS.contains(&kind)) {
            return;
        }
        for key in matches {
            if let Some(handle) = self.syncs.get(key) {
                handle.signal(stored);
            }
        }
    }

    /// Divert fan-out matches that are syncs into their signal inboxes, and
    /// return the matches that receive the event itself.
    pub fn divert(
        &self,
        stored: &StoredEvent,
        matches: Vec<(ConnId, SubId)>,
    ) -> Vec<(ConnId, SubId)> {
        if self.syncs.is_empty() {
            return matches;
        }
        matches
            .into_iter()
            .filter(|key| match self.syncs.get(key) {
                Some(handle) => {
                    handle.signal(stored);
                    false
                }
                None => true,
            })
            .collect()
    }
}

/// Resync every sync on this node whenever its Redis link may have lost a
/// message (rule 5). A sync records the epoch its interest was confirmed on,
/// so any later change reaches it here. Runs until the link owner stops.
pub async fn watch_link(state: Arc<AppState>) {
    let mut link = state.pubsub.link_state();
    let mut seen = link.borrow_and_update().epoch;
    while link.changed().await.is_ok() {
        let epoch = link.borrow_and_update().epoch;
        if epoch != seen {
            seen = epoch;
            state
                .artifact_syncs
                .resync_all("resync: live delivery interrupted");
        }
    }
}

/// Publish `stored` to Redis. A failed publish of a kind that syncs consume
/// is retried in the background until it succeeds or the node stops (rule
/// 5): order does not matter, because a sync reads the current head.
pub(crate) async fn publish_reliably(
    state: &Arc<AppState>,
    tenant: &buzz_core::TenantContext,
    topic: EventTopic,
    stored: &StoredEvent,
) {
    if publish_once(state, tenant, topic, &stored.event).await {
        return;
    }
    if !LIVE_KINDS.contains(&stored.event.kind.as_u16()) {
        return;
    }
    metrics::counter!("buzz_artifact_sync_publish_retry_total").increment(1);
    let state = Arc::clone(state);
    let tenant = tenant.clone();
    let event = stored.event.clone();
    tokio::spawn(async move {
        let mut delay = Duration::from_millis(100);
        loop {
            tokio::time::sleep(delay).await;
            if publish_once(&state, &tenant, topic, &event).await {
                return;
            }
            delay = (delay * 2).min(Duration::from_secs(5));
        }
    });
}

/// One publish attempt. The local-echo mark is set first, so this node does
/// not fan the event out a second time when Redis echoes it back.
async fn publish_once(
    state: &AppState,
    tenant: &buzz_core::TenantContext,
    topic: EventTopic,
    event: &Event,
) -> bool {
    state.mark_local_event(tenant.community(), &event.id);
    #[cfg(test)]
    crate::nip_fi_test_hooks::before_event_publish(tenant.community());
    match state.pubsub.publish_event(tenant, topic, event).await {
        Ok(_) => true,
        Err(e) => {
            state
                .local_event_ids
                .invalidate(&(tenant.community(), event.id.to_bytes()));
            warn!(event_id = %event.id.to_hex(), "Redis publish failed: {e}");
            false
        }
    }
}

/// The fan-out filter a sync registers: every change signal in its channels.
fn live_filter(channels: &[Uuid]) -> Filter {
    Filter::new()
        .kinds(LIVE_KINDS.map(Kind::Custom))
        .custom_tags(
            SingleLetterTag::lowercase(Alphabet::H),
            channels.iter().map(Uuid::to_string),
        )
}

/// Serve one sync `REQ`. `permit` is the handler slot; it is held only for
/// the initial read.
pub async fn handle_sync(
    sub_id: String,
    filter: Result<ArtifactSyncFilter, &'static str>,
    conn: Arc<ConnectionState>,
    state: Arc<AppState>,
    permit: OwnedSemaphorePermit,
) {
    let filter = match filter {
        Ok(filter) => filter,
        Err(reason) => {
            conn.send(RelayMessage::closed(&sub_id, &format!("invalid: {reason}")));
            return;
        }
    };
    let reader = match conn.auth_state_snapshot() {
        AuthState::Authenticated(ctx) => {
            if !ctx.scopes.is_empty() && !ctx.scopes.contains(&Scope::MessagesRead) {
                conn.send(RelayMessage::closed(
                    &sub_id,
                    "restricted: insufficient scope",
                ));
                return;
            }
            // A channel-scoped token reaches only its own channels; outside
            // them the answer matches an unreadable channel.
            if let Some(allowed) = ctx.channel_ids.as_deref() {
                if filter.channels.iter().any(|id| !allowed.contains(id)) {
                    conn.send(RelayMessage::closed(&sub_id, RESTRICTED));
                    return;
                }
            }
            ctx.pubkey.to_bytes().to_vec()
        }
        _ => {
            conn.send(RelayMessage::closed(
                &sub_id,
                "auth-required: not authenticated",
            ));
            return;
        }
    };
    let _effect = match conn.nip_fi_gate.acquire_effect().await {
        Ok(permit) => permit,
        Err(crate::nip_fi_gate::SessionExpired) => {
            conn.send(RelayMessage::closed(
                &sub_id,
                "restricted: authorization denied",
            ));
            return;
        }
    };

    // Rule 1: live interest first, confirmed by Redis on an unbroken link.
    let start = *state.pubsub.link_state().borrow();
    let handle = Arc::new(SyncHandle::new());
    let Some(owner) = claim(&sub_id, &filter.channels, &handle, &conn, &state).await else {
        return;
    };
    if !start.up || !confirm(&filter.channels, start.epoch, &conn, &state).await {
        close(
            &sub_id,
            owner,
            "error: live delivery unavailable",
            &conn,
            &state,
        )
        .await;
        return;
    }

    // Rule 2: the initial set from one snapshot, inside the time budget.
    let initial = tokio::time::timeout(
        SNAPSHOT_BUDGET,
        send_initial_set(&sub_id, &filter, &reader, &conn, &state),
    );
    let outcome = tokio::select! {
        biased;
        () = conn.nip_fi_gate.cancelled() => None,
        () = handle.cancel.cancelled() => return,
        out = initial => Some(out),
    };
    let reason = match outcome {
        None => {
            super::close::close_if_owner(&sub_id, owner, None, &conn, &state).await;
            return;
        }
        Some(Err(_)) => Some("error: sync snapshot budget exceeded"),
        Some(Ok(result)) => result.err(),
    };
    // A resync requested during the read also ends the sync before EOSE.
    let reason = reason.or_else(|| handle.pending.lock().ok().and_then(|p| p.resync));
    if let Some(reason) = reason {
        close(&sub_id, owner, reason, &conn, &state).await;
        return;
    }
    if !send_awaiting(&conn, RelayMessage::eose(&sub_id)).await {
        return;
    }
    drop(_effect);
    drop(permit);

    run_live(&sub_id, owner, &filter, &reader, &handle, &conn, &state).await;
}

const RESTRICTED: &str = "restricted: channel not readable";

/// Claim `sub_id` under the lifecycle lock. The sync's inbox is installed
/// before fan-out registration, so no event frame is ever sent to it.
async fn claim(
    sub_id: &str,
    channels: &[Uuid],
    handle: &Arc<SyncHandle>,
    conn: &ConnectionState,
    state: &AppState,
) -> Option<u64> {
    let mut subs = conn.subscriptions.lock().await;
    if conn.cancel.is_cancelled() {
        return None;
    }
    if !subs.contains_key(sub_id) && subs.len() >= super::req::MAX_SUBSCRIPTIONS {
        conn.send(RelayMessage::closed(
            sub_id,
            "error: too many subscriptions",
        ));
        return None;
    }
    super::close::retire_locked(&mut subs, sub_id, conn, state).await;
    let owner = super::close::next_owner();
    subs.insert(sub_id.to_owned(), owner);
    state
        .artifact_syncs
        .insert(conn.conn_id, sub_id, Arc::clone(handle));
    state.sub_registry.register_channels_scoped(
        conn.tenant.community(),
        conn.conn_id,
        sub_id.to_owned(),
        vec![live_filter(channels)],
        channels.to_vec(),
    );
    for &channel in channels {
        state
            .pubsub
            .retain_topic(&conn.tenant, EventTopic::Channel(channel))
            .await;
    }
    Some(owner)
}

/// Wait until Redis confirms every channel topic on link epoch `epoch`. A
/// later change of the epoch resyncs this sync through `resync_all`.
async fn confirm(channels: &[Uuid], epoch: u64, conn: &ConnectionState, state: &AppState) -> bool {
    let confirm_all = async {
        for &channel in channels {
            match state
                .pubsub
                .confirm_topic(&conn.tenant, EventTopic::Channel(channel))
                .await
            {
                Ok(confirmed) if confirmed == epoch => {}
                _ => return false,
            }
        }
        true
    };
    tokio::time::timeout(CONFIRM_TIMEOUT, confirm_all)
        .await
        .unwrap_or(false)
}

/// Read and send every head in scope from one snapshot. An `Err` is the
/// `CLOSED` reason; nothing after it is sent.
async fn send_initial_set(
    sub_id: &str,
    filter: &ArtifactSyncFilter,
    reader: &[u8],
    conn: &ConnectionState,
    state: &AppState,
) -> Result<(), &'static str> {
    let mut snapshot = match state
        .db
        .open_artifact_snapshot(conn.tenant.community(), reader, &filter.channels)
        .await
    {
        Ok(Ok(snapshot)) => snapshot,
        Ok(Err(buzz_db::SnapshotDenied::Restricted)) => return Err(RESTRICTED),
        Err(e) => {
            warn!(conn_id = %conn.conn_id, "artifact sync snapshot failed: {e}");
            return Err("error: sync read failed");
        }
    };
    let mut sent = 0usize;
    loop {
        let page = snapshot
            .next_page(&filter.tags, SNAPSHOT_PAGE)
            .await
            .map_err(|e| {
                warn!(conn_id = %conn.conn_id, "artifact sync page failed: {e}");
                "error: sync read failed"
            })?;
        if page.is_empty() {
            break;
        }
        sent += page.len();
        if sent > MAX_SYNC_EVENTS {
            return Err("error: sync set exceeds max_events");
        }
        for stored in page {
            if !send_awaiting(conn, RelayMessage::event(sub_id, &stored.event)).await {
                return Err("error: connection closed");
            }
        }
    }
    snapshot
        .finish()
        .await
        .map_err(|_| "error: sync read failed")?;
    Ok(())
}

/// Queue a frame, waiting for room: the snapshot budget, not the slow-client
/// grace limit, bounds a slow reader during the initial set.
async fn send_awaiting(conn: &ConnectionState, frame: String) -> bool {
    conn.send_tx
        .send(WsMessage::Text(frame.into()))
        .await
        .is_ok()
}

/// Rules 3–5 after `EOSE`: read each changed head, one read at a time, and
/// send it. A dropped frame or a resync request ends the sync.
async fn run_live(
    sub_id: &str,
    owner: u64,
    filter: &ArtifactSyncFilter,
    reader: &[u8],
    handle: &SyncHandle,
    conn: &ConnectionState,
    state: &AppState,
) {
    loop {
        let pending = handle.take();
        if let Some(reason) = pending.resync {
            close(sub_id, owner, reason, conn, state).await;
            return;
        }
        for (d, removal) in pending.changed {
            let head = match state
                .db
                .artifact_head(conn.tenant.community(), reader, d, &filter.channels)
                .await
            {
                Ok(head) => head,
                Err(e) => {
                    warn!(conn_id = %conn.conn_id, "artifact sync head read failed: {e}");
                    close(sub_id, owner, "error: sync read failed", conn, state).await;
                    return;
                }
            };
            // A head of another type (or `d`) was never in scope; the change
            // signal is per channel, so it reaches every sync there.
            if head
                .as_ref()
                .is_some_and(|head| filter.never_in_scope(&head.event))
            {
                continue;
            }
            // A head the reader cannot read leaves only the source removal.
            let Some(event) = head.map(|head| head.event).or(removal) else {
                continue;
            };
            if conn
                .send_tx
                .try_send(WsMessage::Text(RelayMessage::event(sub_id, &event).into()))
                .is_err()
            {
                // One lost frame breaks the copy; the slow-client grace limit
                // does not apply. `close` drops the connection when even the
                // CLOSED frame cannot be queued.
                close(sub_id, owner, "resync: send queue full", conn, state).await;
                return;
            }
        }
        tokio::select! {
            () = handle.cancel.cancelled() => return,
            () = conn.cancel.cancelled() => return,
            () = handle.notify.notified() => {}
        }
    }
}

/// End the sync with `CLOSED reason` if this request still owns its ID.
async fn close(sub_id: &str, owner: u64, reason: &str, conn: &ConnectionState, state: &AppState) {
    super::close::close_if_owner(sub_id, owner, Some(reason), conn, state).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Tag};

    fn stored(kind: u16, d: Uuid) -> StoredEvent {
        let event = EventBuilder::new(Kind::Custom(kind), "")
            .tags([Tag::parse(["d", &d.to_string()]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        StoredEvent::new(event, Some(Uuid::new_v4()))
    }

    #[test]
    fn divert_signals_syncs_and_keeps_ordinary_matches() {
        let syncs = ArtifactSyncs::default();
        let conn = Uuid::new_v4();
        let handle = Arc::new(SyncHandle::new());
        syncs.insert(conn, "sync", Arc::clone(&handle));
        let d = Uuid::new_v4();
        let rest = syncs.divert(
            &stored(45010, d),
            vec![(conn, "sync".into()), (conn, "plain".into())],
        );
        assert_eq!(rest, vec![(conn, "plain".to_string())]);
        let removal = stored(45011, d);
        syncs.divert(&removal, vec![(conn, "sync".into())]);
        // A later revision keeps the removal: only the head read decides.
        syncs.divert(&stored(45010, d), vec![(conn, "sync".into())]);
        let pending = handle.take();
        assert_eq!(pending.changed.len(), 1);
        assert_eq!(
            pending.changed[&d].as_ref().map(|e| e.id),
            Some(removal.event.id)
        );
        assert!(pending.resync.is_none());

        syncs.divert(&stored(KIND_REDACTION, d), vec![(conn, "sync".into())]);
        assert_eq!(handle.take().resync, Some("resync: redaction"));

        syncs.remove(conn, "sync");
        assert!(handle.cancel.is_cancelled());
        assert_eq!(
            syncs
                .divert(&stored(45010, d), vec![(conn, "sync".into())])
                .len(),
            1
        );
    }

    /// The access-change signal runs before the access filter, so a reader
    /// who already lost access still gets its resync. Content kinds wait for
    /// the filtered `divert`. Mutation: drop the `ACCESS_KINDS` check in
    /// `signal_access_change` → the 45010 is recorded unfiltered → RED.
    #[test]
    fn access_changes_signal_before_the_access_filter() {
        let syncs = ArtifactSyncs::default();
        let conn = Uuid::new_v4();
        let handle = Arc::new(SyncHandle::new());
        syncs.insert(conn, "sync", Arc::clone(&handle));
        let matches = vec![(conn, "sync".to_string())];
        syncs.signal_access_change(&stored(45010, Uuid::new_v4()), &matches);
        let pending = handle.take();
        assert!(pending.changed.is_empty() && pending.resync.is_none());
        for kind in ACCESS_KINDS {
            syncs.signal_access_change(&stored(kind, Uuid::new_v4()), &matches);
            assert_eq!(
                handle.take().resync,
                Some("resync: channel access changed"),
                "{kind}"
            );
        }
    }

    #[test]
    fn held_changes_past_max_events_resync() {
        let handle = SyncHandle::new();
        for _ in 0..=MAX_SYNC_EVENTS {
            handle.signal(&stored(45010, Uuid::new_v4()));
        }
        assert_eq!(handle.take().resync, Some("resync: too many changes"));
    }

    #[test]
    fn resync_all_reaches_every_sync() {
        let syncs = ArtifactSyncs::default();
        let handles: Vec<_> = (0..3).map(|_| Arc::new(SyncHandle::new())).collect();
        for (i, handle) in handles.iter().enumerate() {
            syncs.insert(Uuid::new_v4(), &i.to_string(), Arc::clone(handle));
        }
        syncs.resync_all("resync: live delivery interrupted");
        for handle in handles {
            assert_eq!(
                handle.take().resync,
                Some("resync: live delivery interrupted")
            );
        }
    }
}
