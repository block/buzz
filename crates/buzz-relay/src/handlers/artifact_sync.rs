//! NIP-AR Sync: a `REQ` that keeps an exact copy of a set of artifacts.
//!
//! The relay never forwards the published event to a sync. A live entry only
//! says that one `d` changed; the sync task then reads that artifact's current
//! head and sends it (NIP-AR Sync rules 3 and 4). Any possible loss of a live
//! entry ends the sync with `CLOSED "resync: ..."` (rule 5).
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use axum::extract::ws::Message as WsMessage;
use buzz_auth::Scope;
use buzz_core::artifact_sync::{ArtifactSyncFilter, MAX_SYNC_EVENTS};
use buzz_core::StoredEvent;
use buzz_pubsub::EventTopic;
use dashmap::DashMap;
use nostr::{Alphabet, Event, Filter, JsonUtil, Kind, SingleLetterTag};
use tokio::sync::{Notify, OwnedSemaphorePermit};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use uuid::Uuid;

use super::close::refuse;
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
/// The most bytes of kind-45011 removals one sync holds (rule 5). Other
/// held changes are a `d` only, so `MAX_SYNC_EVENTS` bounds them. Not
/// advertised: a sync past either bound resyncs.
const MAX_HELD_BYTES: usize = 4 << 20;
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
    /// Serialized size of the removals in `changed`.
    held_bytes: usize,
    /// The first reason this sync must resync, if any. Once set, nothing
    /// more is held: the sync ends without reading its changes.
    resync: Option<&'static str>,
}

impl Pending {
    fn end(&mut self, reason: &'static str) {
        self.resync.get_or_insert(reason);
        self.changed = HashMap::new();
        self.held_bytes = 0;
    }

    /// Hold one change, refusing it before it is stored when it would pass
    /// a bound.
    fn hold(&mut self, d: Uuid, removal: Option<Event>) {
        if self.resync.is_some() {
            return;
        }
        if !self.changed.contains_key(&d) && self.changed.len() >= MAX_SYNC_EVENTS {
            return self.end("resync: too many changes");
        }
        let Some(removal) = removal else {
            self.changed.entry(d).or_default();
            return;
        };
        let size = removal.as_json().len();
        let replaced = self
            .changed
            .get(&d)
            .and_then(Option::as_ref)
            .map_or(0, |old| old.as_json().len());
        let held = self.held_bytes - replaced + size;
        if held > MAX_HELD_BYTES {
            return self.end("resync: too many changes");
        }
        self.held_bytes = held;
        self.changed.insert(d, Some(removal));
    }
}

/// The artifact identity (`d`) of a revision or removal.
fn artifact_id(event: &Event) -> Option<Uuid> {
    event
        .tags
        .iter()
        .find(|tag| tag.as_slice().first().map(String::as_str) == Some("d"))
        .and_then(|tag| tag.as_slice().get(1)?.parse().ok())
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
        change(&mut self.pending.lock().unwrap_or_else(PoisonError::into_inner));
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
        let Some(d) = artifact_id(&stored.event) else {
            return;
        };
        let removal = (kind == 45011).then(|| stored.event.clone());
        self.update(|pending| pending.hold(d, removal));
    }

    /// Ask the sync to end with `CLOSED reason`, where reason is `resync: ...`.
    pub fn resync(&self, reason: &'static str) {
        self.update(|pending| pending.end(reason));
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

#[cfg(test)]
impl ArtifactSyncs {
    /// Install an idle sync on `sub`, as a sync claim does.
    pub(crate) fn insert_idle(&self, conn: ConnId, sub: &str) {
        self.insert(conn, sub, Arc::new(SyncHandle::new()));
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
            refuse(&sub_id, &format!("invalid: {reason}"), &conn, &state).await;
            return;
        }
    };
    let (reader, bound) = match conn.auth_state_snapshot() {
        AuthState::Authenticated(ctx) => {
            if !ctx.scopes.is_empty() && !ctx.scopes.contains(&Scope::MessagesRead) {
                refuse(&sub_id, "restricted: insufficient scope", &conn, &state).await;
                return;
            }
            // A channel-scoped token reaches only its own channels; outside
            // them the answer matches an unreadable channel.
            if let Some(allowed) = ctx.channel_ids.as_deref() {
                if filter.channels.iter().any(|id| !allowed.contains(id)) {
                    refuse(&sub_id, RESTRICTED, &conn, &state).await;
                    return;
                }
            }
            (ctx.pubkey.to_bytes().to_vec(), ctx.channel_ids)
        }
        _ => {
            refuse(&sub_id, "auth-required: not authenticated", &conn, &state).await;
            return;
        }
    };
    let _effect = match conn.nip_fi_gate.acquire_effect().await {
        Ok(permit) => permit,
        Err(crate::nip_fi_gate::SessionExpired) => {
            refuse(&sub_id, "restricted: authorization denied", &conn, &state).await;
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

    // Rule 2: the initial set from one snapshot and its EOSE, all inside the
    // time budget and the cancellation race, so a slow reader cannot hold
    // the handler slot past the budget or get EOSE after retirement.
    let initial = async {
        let sent = send_initial_set(&sub_id, owner, &filter, &reader, &conn, &state).await?;
        // A resync requested before EOSE is queued ends the sync without it.
        send_eose(&sub_id, owner, &handle, &conn).await?;
        Ok::<_, Stop>(sent)
    };
    let outcome = tokio::select! {
        biased;
        () = conn.nip_fi_gate.cancelled() => None,
        () = handle.cancel.cancelled() => return,
        out = tokio::time::timeout(SNAPSHOT_BUDGET, initial) => Some(out),
    };
    let in_set = match outcome {
        None => {
            super::close::close_if_owner(&sub_id, owner, None, &conn, &state).await;
            return;
        }
        Some(Err(_)) => {
            let reason = "error: sync snapshot budget exceeded";
            close(&sub_id, owner, reason, &conn, &state).await;
            return;
        }
        Some(Ok(Err(Stop::Retired))) => return,
        Some(Ok(Err(Stop::Close(reason)))) => {
            close(&sub_id, owner, reason, &conn, &state).await;
            return;
        }
        Some(Ok(Ok(sent))) => sent,
    };
    drop(_effect);
    drop(permit);

    run_live(
        &sub_id,
        owner,
        &filter,
        &reader,
        bound.as_deref(),
        in_set,
        &handle,
        &conn,
        &state,
    )
    .await;
}

const RESTRICTED: &str = "restricted: channel not readable";
const OVER_MAX_EVENTS: &str = "error: sync set exceeds max_events";

/// Why a sync stops sending.
enum Stop {
    /// End with `CLOSED reason`.
    Close(&'static str),
    /// Another request now owns the ID, or it was closed: say nothing.
    Retired,
}

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

/// Read and send every head in scope from one snapshot, and return their
/// `d`. After an `Err` nothing more is sent.
async fn send_initial_set(
    sub_id: &str,
    owner: u64,
    filter: &ArtifactSyncFilter,
    reader: &[u8],
    conn: &ConnectionState,
    state: &AppState,
) -> Result<HashSet<Uuid>, Stop> {
    let mut snapshot = match state
        .db
        .open_artifact_snapshot(conn.tenant.community(), reader, &filter.channels)
        .await
    {
        Ok(Ok(snapshot)) => snapshot,
        Ok(Err(buzz_db::SnapshotDenied::Restricted)) => return Err(Stop::Close(RESTRICTED)),
        Err(e) => {
            warn!(conn_id = %conn.conn_id, "artifact sync snapshot failed: {e}");
            return Err(Stop::Close("error: sync read failed"));
        }
    };
    let mut sent = HashSet::new();
    loop {
        let page = snapshot
            .next_page(&filter.tags, SNAPSHOT_PAGE)
            .await
            .map_err(|e| {
                warn!(conn_id = %conn.conn_id, "artifact sync page failed: {e}");
                Stop::Close("error: sync read failed")
            })?;
        if page.is_empty() {
            break;
        }
        if sent.len() + page.len() > MAX_SYNC_EVENTS {
            return Err(Stop::Close(OVER_MAX_EVENTS));
        }
        for stored in page {
            sent.extend(artifact_id(&stored.event));
            send_owned(
                sub_id,
                owner,
                RelayMessage::event(sub_id, &stored.event),
                conn,
            )
            .await?;
        }
    }
    snapshot
        .finish()
        .await
        .map_err(|_| Stop::Close("error: sync read failed"))?;
    Ok(sent)
}

/// Queue a frame while this request owns `sub_id`, waiting for room: the
/// snapshot budget, not the slow-client grace limit, bounds a slow reader
/// during the initial set. The slot is reserved first and the owner checked
/// under the lifecycle lock, so the lock is never held while waiting, and a
/// CLOSE or replacing REQ (which retire under that lock) fences the frame.
async fn send_owned(
    sub_id: &str,
    owner: u64,
    frame: String,
    conn: &ConnectionState,
) -> Result<(), Stop> {
    send_reserved(sub_id, owner, frame, None, conn).await
}

/// Queue `EOSE` like `send_owned`, unless the sync must resync. The pending
/// resync is checked under the inbox lock while the frame is queued, so a
/// resync recorded at any time before `EOSE` enters the queue ends the sync
/// with `CLOSED` and no `EOSE` (rule 1); a later one closes after it.
async fn send_eose(
    sub_id: &str,
    owner: u64,
    handle: &SyncHandle,
    conn: &ConnectionState,
) -> Result<(), Stop> {
    send_reserved(
        sub_id,
        owner,
        RelayMessage::eose(sub_id),
        Some(handle),
        conn,
    )
    .await
}

async fn send_reserved(
    sub_id: &str,
    owner: u64,
    frame: String,
    unless_resync: Option<&SyncHandle>,
    conn: &ConnectionState,
) -> Result<(), Stop> {
    let slot = conn
        .send_tx
        .reserve()
        .await
        .map_err(|_| Stop::Close("error: connection closed"))?;
    let subs = conn.subscriptions.lock().await;
    if subs.get(sub_id) != Some(&owner) {
        return Err(Stop::Retired);
    }
    let pending = unless_resync.map(|handle| {
        handle
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    });
    if let Some(reason) = pending.as_ref().and_then(|pending| pending.resync) {
        return Err(Stop::Close(reason));
    }
    slot.send(WsMessage::Text(frame.into()));
    Ok(())
}

/// Queue a live frame without waiting, fenced like `send_owned`. A full
/// queue is a lost frame.
async fn try_send_owned(
    sub_id: &str,
    owner: u64,
    frame: String,
    conn: &ConnectionState,
) -> Result<(), Stop> {
    let subs = conn.subscriptions.lock().await;
    if subs.get(sub_id) != Some(&owner) {
        return Err(Stop::Retired);
    }
    conn.send_tx
        .try_send(WsMessage::Text(frame.into()))
        .map_err(|_| Stop::Close("resync: send queue full"))
}

/// Rules 3–5 after `EOSE`: read each changed head, one read at a time, and
/// send it. A dropped frame or a resync request ends the sync. `in_set` is
/// the client's copy as this sync sent it: the `d` of each artifact in scope.
/// `bound` is a channel-scoped token's channels: the reader reads no head
/// outside them, even where its pubkey could.
#[allow(clippy::too_many_arguments)]
async fn run_live(
    sub_id: &str,
    owner: u64,
    filter: &ArtifactSyncFilter,
    reader: &[u8],
    bound: Option<&[Uuid]>,
    mut in_set: HashSet<Uuid>,
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
            // A retired sync reads nothing more.
            if handle.cancel.is_cancelled() {
                return;
            }
            let head = match state
                .db
                .artifact_head(conn.tenant.community(), reader, d, &filter.channels)
                .await
            {
                Ok(head) => head.filter(|(head, _)| {
                    bound.is_none_or(|allowed| {
                        head.channel_id.is_some_and(|id| allowed.contains(&id))
                    })
                }),
                Err(e) => {
                    warn!(conn_id = %conn.conn_id, "artifact sync head read failed: {e}");
                    close(sub_id, owner, "error: sync read failed", conn, state).await;
                    return;
                }
            };
            // Rule 3: send a change only for an artifact in scope before it
            // (in the client's copy) or after it (a live head that matches
            // every predicate). The change signal is per channel, so it
            // reaches syncs that never matched this artifact.
            let in_scope = head
                .as_ref()
                .is_some_and(|(head, deleted)| !deleted && filter.matches(&head.event));
            if !in_scope && !in_set.contains(&d) {
                continue;
            }
            // A head the reader cannot read leaves only the source removal.
            // Without one, nothing the reader may read can remove the
            // artifact from its copy, so the copy is no longer exact.
            let Some(event) = head.map(|(head, _)| head.event).or(removal) else {
                close(sub_id, owner, "resync: artifact left scope", conn, state).await;
                return;
            };
            #[cfg(test)]
            super::close::test_seam::pause_at(&test_seam::LIVE_SEND_PAUSE).await;
            match try_send_owned(sub_id, owner, RelayMessage::event(sub_id, &event), conn).await {
                Ok(()) => {}
                Err(Stop::Retired) => return,
                // One lost frame breaks the copy; the slow-client grace limit
                // does not apply. `close` drops the connection when even the
                // CLOSED frame cannot be queued.
                Err(Stop::Close(reason)) => {
                    close(sub_id, owner, reason, conn, state).await;
                    return;
                }
            }
            if !in_scope {
                in_set.remove(&d);
            } else if in_set.insert(d) && in_set.len() > MAX_SYNC_EVENTS {
                // A new sync of this set would be refused the same way.
                close(sub_id, owner, OVER_MAX_EVENTS, conn, state).await;
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
pub(crate) mod test_seam {
    use std::sync::Arc;

    use super::super::close::test_seam::Pause;

    tokio::task_local! {
        /// After a live head read, before its frame is queued.
        pub(crate) static LIVE_SEND_PAUSE: Arc<Pause>;
    }
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

    /// The held list is a hard bound: a change past the limit is refused
    /// before it is stored, and nothing is held after the sync must resync.
    /// Mutation: insert before the size check, or keep holding after
    /// `resync` → `changed` grows past `MAX_SYNC_EVENTS` → RED.
    #[test]
    fn held_list_holds_nothing_past_its_limit() {
        let handle = SyncHandle::new();
        for _ in 0..MAX_SYNC_EVENTS {
            handle.signal(&stored(45010, Uuid::new_v4()));
        }
        {
            let pending = handle.pending.lock().unwrap();
            assert_eq!(pending.changed.len(), MAX_SYNC_EVENTS);
            assert!(pending.resync.is_none());
        }
        for _ in 0..100 {
            handle.signal(&stored(45010, Uuid::new_v4()));
            handle.signal(&stored(45011, Uuid::new_v4()));
        }
        let pending = handle.pending.lock().unwrap();
        assert_eq!(pending.resync, Some("resync: too many changes"));
        assert!(pending.changed.is_empty());
        assert_eq!(pending.held_bytes, 0);
    }

    fn removal(d: Uuid, size: usize) -> StoredEvent {
        let event = EventBuilder::new(Kind::Custom(45011), "x".repeat(size))
            .tags([Tag::parse(["d", &d.to_string()]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        StoredEvent::new(event, Some(Uuid::new_v4()))
    }

    /// Held removals are whole events, so their bytes have their own bound.
    /// A newer removal for the same `d` replaces the older one's bytes.
    #[test]
    fn held_removals_have_a_byte_bound() {
        let handle = SyncHandle::new();
        let d = Uuid::new_v4();
        for _ in 0..2 * (MAX_HELD_BYTES >> 16) {
            handle.signal(&removal(d, 1 << 16));
        }
        {
            let pending = handle.pending.lock().unwrap();
            assert!(pending.resync.is_none());
            assert!(pending.held_bytes < 1 << 17, "{}", pending.held_bytes);
        }
        for _ in 0..=MAX_HELD_BYTES >> 16 {
            handle.signal(&removal(Uuid::new_v4(), 1 << 16));
        }
        let pending = handle.pending.lock().unwrap();
        assert_eq!(pending.resync, Some("resync: too many changes"));
        assert!(pending.changed.is_empty());
    }

    fn conn_with_sub(owner: u64) -> (Arc<ConnectionState>, tokio::sync::mpsc::Receiver<WsMessage>) {
        let tenant = buzz_core::TenantContext::resolved(
            buzz_core::CommunityId::from_uuid(Uuid::new_v4()),
            "relay.example",
        );
        let (conn, rx) =
            super::super::group_state_postgres_tests::test_conn(&tenant, &Keys::generate());
        conn.subscriptions
            .try_lock()
            .unwrap()
            .insert("s".into(), owner);
        (conn, rx)
    }

    /// A live frame goes out only while its request still owns the ID.
    /// Mutation: drop the owner check in `try_send_owned` → RED.
    #[tokio::test]
    async fn live_frame_is_fenced_by_the_owner() {
        let (conn, mut rx) = conn_with_sub(1);
        assert!(matches!(
            try_send_owned("s", 2, "x".into(), &conn).await,
            Err(Stop::Retired)
        ));
        conn.subscriptions.lock().await.remove("s");
        assert!(matches!(
            try_send_owned("s", 1, "x".into(), &conn).await,
            Err(Stop::Retired)
        ));
        assert!(rx.try_recv().is_err());
        conn.subscriptions.lock().await.insert("s".into(), 1);
        assert!(try_send_owned("s", 1, "x".into(), &conn).await.is_ok());
        assert!(rx.try_recv().is_ok());
    }

    /// A frame (such as `EOSE`) waiting for queue room is checked against
    /// the owner after the room appears, not before it waited. Mutation:
    /// check the owner before `reserve` → the stale frame is queued → RED.
    #[tokio::test]
    async fn waiting_frame_is_fenced_when_replaced_during_the_wait() {
        let (conn, mut rx) = conn_with_sub(1);
        let capacity = conn.send_tx.capacity();
        for _ in 0..capacity {
            conn.send_tx
                .try_send(WsMessage::Text("fill".into()))
                .unwrap();
        }
        let mut waiting = std::pin::pin!(send_owned("s", 1, "eose".into(), &conn));
        // Polled once, the frame is now waiting in `reserve`.
        assert!(futures_util::poll!(&mut waiting).is_pending());
        // A replacing REQ takes the ID while the frame waits.
        conn.subscriptions.lock().await.insert("s".into(), 2);
        rx.recv().await.unwrap();
        assert!(matches!(waiting.await, Err(Stop::Retired)));
        let mut rest = 0;
        while let Ok(WsMessage::Text(text)) = rx.try_recv() {
            assert_eq!(text.as_str(), "fill");
            rest += 1;
        }
        assert_eq!(rest, capacity - 1);
    }

    /// A resync recorded while `EOSE` waits for queue room ends the sync
    /// without `EOSE` (rule 1). Mutation: check the pending resync before
    /// `reserve` instead of while queueing → `EOSE` is queued → RED.
    #[tokio::test]
    async fn resync_during_the_eose_wait_sends_no_eose() {
        let (conn, mut rx) = conn_with_sub(1);
        let handle = SyncHandle::new();
        let capacity = conn.send_tx.capacity();
        for _ in 0..capacity {
            conn.send_tx
                .try_send(WsMessage::Text("fill".into()))
                .unwrap();
        }
        let mut waiting = std::pin::pin!(send_eose("s", 1, &handle, &conn));
        assert!(futures_util::poll!(&mut waiting).is_pending());
        // The held list overflows while EOSE waits.
        for _ in 0..=MAX_SYNC_EVENTS {
            handle.signal(&stored(45010, Uuid::new_v4()));
        }
        rx.recv().await.unwrap();
        assert!(matches!(
            waiting.await,
            Err(Stop::Close("resync: too many changes"))
        ));
        while let Ok(WsMessage::Text(text)) = rx.try_recv() {
            assert_eq!(text.as_str(), "fill");
        }

        // Without a pending resync, EOSE is queued.
        let clean = SyncHandle::new();
        assert!(send_eose("s", 1, &clean, &conn).await.is_ok());
        let Ok(WsMessage::Text(text)) = rx.try_recv() else {
            panic!("no EOSE");
        };
        assert!(text.as_str().starts_with(r#"["EOSE""#), "{text}");
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
