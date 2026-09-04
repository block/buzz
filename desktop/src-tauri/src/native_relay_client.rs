//! Shared native relay session.
//!
//! Owns the authenticated relay socket for backend features that need live
//! subscriptions (archive sync today; persona catalog and catch-up next). One
//! session per (relay, pubkey) scope, multiplexing every subscription over a
//! single socket — a second socket per feature would multiply relay connection
//! slots and duplicate the NIP-42 handshake for no benefit.
//!
//! Built on `buzz-ws-client`, which owns the wire format and the NIP-42
//! handshake. That crate is request/response shaped (one caller, `next_event`
//! off a buffer); the session lifecycle lives here instead of being pushed down
//! into it, because `buzz-cli` and `buzz-test-client` consume that crate and do
//! not want subscription bookkeeping.
//!
//! # Caller contract
//!
//! A subscription id's filter is immutable for the life of a session: to change
//! a filter, use a new id. See [`Subscription::id`] for why this cannot be
//! relaxed from inside this module.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use buzz_ws_client_pkg::{NostrWsConnection, RelayMessage};
use nostr::{Event, Keys};
use tokio::{
    sync::{mpsc, oneshot, Mutex},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

/// Backoff floor for reconnect attempts.
const RECONNECT_BASE_DELAY: Duration = Duration::from_millis(500);
/// Backoff ceiling. Matches the renderer session's ceiling so a relay outage
/// produces one retry cadence across the app rather than two competing ones.
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);
/// How long a read may block before the loop re-checks cancellation. Not a
/// connection timeout: an idle relay is normal, so a lapsed read just loops.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Backoff floor for reopening a subscription the relay CLOSED. Matches
/// `RETRY_BASE_DELAY_MS` in `relayClosedRecovery.ts`.
const CLOSED_RETRY_BASE_DELAY: Duration = Duration::from_secs(1);
/// Backoff ceiling for reopening a CLOSED subscription. Matches
/// `RETRY_MAX_DELAY_MS` in `relayClosedRecovery.ts`.
const CLOSED_RETRY_MAX_DELAY: Duration = Duration::from_secs(30);
/// Delay for a `rate-limited:` CLOSED that carries no `retry in Ns` hint.
/// Matches `DEFAULT_RATE_LIMIT_SECONDS` on both sides of the client.
const CLOSED_RATE_LIMIT_DEFAULT: Duration = Duration::from_secs(10);

/// A live subscription request: a filter plus where its events go.
#[derive(Clone)]
pub(crate) struct Subscription {
    /// Caller-stable key. Reused verbatim as the relay subscription id so a
    /// resubscribe after reconnect replaces rather than duplicates.
    ///
    /// **An id's filter is immutable for the life of a session.** To change a
    /// filter, use a new id — as `archive::sync` does by hashing scope and
    /// kinds into the id. Reusing an id for a different filter is unsound and
    /// cannot be made sound here: a CLOSED frame carries only the id, so a
    /// rejection caused by the old filter is indistinguishable from one caused
    /// by the new one, and would latch backoff (or a terminal stop) onto a
    /// subscription that never failed.
    pub(crate) id: String,
    pub(crate) filter: serde_json::Value,
}

/// An event delivered to the session owner, tagged with the subscription that
/// matched it. Callers demultiplex on `subscription_id`.
#[derive(Clone)]
pub(crate) struct MatchedEvent {
    pub(crate) subscription_id: String,
    pub(crate) event: Box<Event>,
}

/// App-wide owner of the one native socket for the active `(relay, pubkey)`
/// scope. Features subscribe independently.
///
/// Only the archive lifecycle may replace the installed scope, and only while
/// holding [`crate::archive::sync::ArchiveOwnership`]; see [`Self::session`]
/// for why finite callers get a non-destructive lease instead.
#[derive(Default)]
pub(crate) struct NativeRelayClient {
    current: Mutex<Option<ManagedSession>>,
}

struct ManagedSession {
    scope: (String, String),
    session: Arc<RelaySession>,
}

/// A session borrowed by a finite-request caller, plus whether that caller owns
/// it. Dropping the lease shuts down a private session and leaves a shared one
/// running for the feature that installed it.
///
/// Exists because a finite caller cannot be trusted to shut the session down by
/// hand: it must not call [`RelaySession::shutdown`] on the shared session, and
/// it must call it on a private one or the socket outlives the request. Tying
/// both to the drop makes the correct behavior the only reachable one.
pub(crate) struct SessionLease {
    session: Arc<RelaySession>,
    /// Set only for a session this lease alone can see, which is therefore the
    /// lease's to cancel.
    private: bool,
}

impl std::ops::Deref for SessionLease {
    type Target = RelaySession;

    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl SessionLease {
    /// Clones the underlying handle for a task that outlives this binding, as
    /// the catch-up fan-out does. Only the lease cancels the session, so the
    /// clone must not outlive it.
    pub(crate) fn handle(&self) -> Arc<RelaySession> {
        Arc::clone(&self.session)
    }
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        if self.private {
            self.session.shutdown();
        }
    }
}

impl NativeRelayClient {
    /// Installs the session for `scope`, shutting down whatever scope held the
    /// slot. Destructive on entry, so every caller must already hold proof it
    /// is the current owner — today that is
    /// [`crate::archive::sync::ArchiveOwnership`].
    async fn ensure_session(&self, relay_url: String, keys: Keys) -> Arc<RelaySession> {
        let scope = (relay_url.clone(), keys.public_key().to_hex());
        let mut current = self.current.lock().await;
        if let Some(managed) = current.as_ref().filter(|managed| managed.scope == scope) {
            return Arc::clone(&managed.session);
        }
        if let Some(previous) = current.take() {
            previous.session.shutdown();
        }
        let session = start_managed(relay_url, keys, None);
        *current = Some(ManagedSession {
            scope,
            session: Arc::clone(&session),
        });
        session
    }

    /// Leases a session for a finite request, never displacing another scope.
    ///
    /// Finite callers (persona catalog, unread catch-up) hold no ownership
    /// proof and cannot obtain one: they are not part of the archive lifecycle.
    /// So this is the non-destructive half of the split — it shares the
    /// installed session when the scope matches, and otherwise runs the request
    /// on a private session that the lease shuts down on drop.
    ///
    /// A mismatch is deliberately NOT treated as "the caller is stale". These
    /// commands are not ordered against archive lifecycle in either direction:
    /// a catalog fetch for the community the user just opened routinely arrives
    /// *before* that community's `start_archive_sync`, while the previous
    /// scope's session is still installed. From inside this lock an early
    /// caller and a late one are indistinguishable — both differ from the
    /// installed scope — so refusing (or fencing on a generation counter, which
    /// answers the same question) would fail the current caller as often as the
    /// stale one. Serving both on their own socket is correct for either, and
    /// whichever is genuinely stale has its result discarded by the scope
    /// re-check each command performs before returning.
    ///
    /// Filling an empty slot is deliberate: at startup the catalog fetch
    /// commonly precedes archive sync, and installing here means the archive
    /// start that follows reuses this socket instead of opening a second one.
    pub(crate) async fn session(&self, relay_url: String, keys: Keys) -> SessionLease {
        let scope = (relay_url.clone(), keys.public_key().to_hex());
        let mut current = self.current.lock().await;
        if let Some(managed) = current.as_ref() {
            return if managed.scope == scope {
                SessionLease {
                    session: Arc::clone(&managed.session),
                    private: false,
                }
            } else {
                SessionLease {
                    session: start_managed(relay_url, keys, None),
                    private: true,
                }
            };
        }
        let session = start_managed(relay_url, keys, None);
        *current = Some(ManagedSession {
            scope,
            session: Arc::clone(&session),
        });
        SessionLease {
            session,
            private: false,
        }
    }

    /// Returns the shared session for `(relay_url, keys)` plus the archive
    /// event stream, replacing any session for a different scope.
    ///
    /// Requires proof of archive-sync ownership because both halves are
    /// destructive on entry: `ensure_session` shuts down a different scope's
    /// socket, and `attach_archive` replaces the session's archive sender, so a
    /// superseded caller would steal the live stream from the current owner.
    /// The token is un-constructible outside `archive::sync` and holds the
    /// ownership locks for its lifetime, so a stale start cannot reach this
    /// call. See [`crate::archive::sync::ArchiveOwnership`].
    pub(crate) async fn archive_session(
        &self,
        relay_url: String,
        keys: Keys,
        _ownership: &crate::archive::sync::ArchiveOwnership<'_>,
    ) -> (Arc<RelaySession>, mpsc::Receiver<MatchedEvent>) {
        let session = self.ensure_session(relay_url, keys).await;
        let event_rx = session.attach_archive().await;
        (session, event_rx)
    }
}

pub(crate) struct RelaySession {
    state: Arc<Mutex<SessionState>>,
    requests: Arc<Mutex<HashMap<String, PendingRequest>>>,
    /// The archive is the sole persistent-event consumer. Sending through its
    /// bounded channel is awaited by the socket loop, preserving the
    /// backpressure required by live-only (`limit: 0`) subscriptions: dropping
    /// an event here cannot be repaired by replaying it later.
    archive_events: Arc<Mutex<Option<mpsc::Sender<MatchedEvent>>>>,
    wake: mpsc::Sender<()>,
    cancel: CancellationToken,
}

struct PendingRequest {
    events: Vec<Event>,
    complete: oneshot::Sender<Result<Vec<Event>, String>>,
}

/// Desired set plus the write-time record of what has left it.
///
/// One lock covers both because reconcile must read them together: snapshotting
/// the desired set and draining `removed` in separate acquisitions lets a
/// `set_subscriptions` land in the gap, so the drain would be consumed against
/// a stale snapshot and could reopen a subscription the caller just dropped.
#[derive(Default)]
struct SessionState {
    desired: Vec<Subscription>,
    transient: Vec<Subscription>,
    /// Ids whose exact subscription has left `desired` since the last
    /// reconcile drained this. Written here rather than derived at reconcile
    /// time because reconcile cannot derive it: wakes coalesce, so a remove
    /// followed by a re-add is observed as a single pass whose desired set
    /// never lost the id. See the eviction table on `retries`.
    removed: HashSet<String>,
}

impl SessionState {
    /// Installs a new desired set, recording every departure.
    ///
    /// Returns the ids whose filter changed under a reused id — a violation of
    /// the immutable-filter-per-id contract on [`Subscription::id`]. This is
    /// the only place that can detect one: the write side alone holds the old
    /// and new filter for an id. Behavior after a violation is deliberately
    /// unspecified; detection is all this offers.
    fn replace_desired(&mut self, subscriptions: Vec<Subscription>) -> Vec<String> {
        let mut violations = Vec::new();
        for previous in std::mem::replace(&mut self.desired, subscriptions) {
            // Departure is keyed on the exact subscription, not the id alone:
            // the relay replaces by id, so a changed filter retires the old
            // subscription just as surely as dropping the id would, and its
            // backoff must not be inherited.
            let survivor = self.desired.iter().find(|next| next.id == previous.id);
            if survivor.is_some_and(|next| next.filter == previous.filter) {
                continue;
            }
            if survivor.is_some() {
                violations.push(previous.id.clone());
            }
            self.removed.insert(previous.id);
        }
        violations
    }
}

impl RelaySession {
    async fn attach_archive(&self) -> mpsc::Receiver<MatchedEvent> {
        let (events, receiver) = mpsc::channel(256);
        *self.archive_events.lock().await = Some(events);
        receiver
    }

    /// Fetches one finite page over this session without disturbing persistent
    /// feature subscriptions. Request ids are fresh, so CLOSED/backoff history
    /// can never leak between pages or into a long-lived subscription.
    pub(crate) async fn fetch_events(
        &self,
        filter: serde_json::Value,
        timeout: Duration,
    ) -> Result<Vec<Event>, String> {
        let id = format!("native-fetch-{}", uuid::Uuid::new_v4());
        let (complete, result) = oneshot::channel();
        self.requests.lock().await.insert(
            id.clone(),
            PendingRequest {
                events: Vec::new(),
                complete,
            },
        );
        {
            let mut state = self.state.lock().await;
            state.transient.push(Subscription {
                id: id.clone(),
                filter,
            });
        }
        let _ = self.wake.try_send(());

        let outcome = tokio::select! {
            _ = self.cancel.cancelled() => Err("relay session cancelled".to_string()),
            value = tokio::time::timeout(timeout, result) => match value {
                Ok(Ok(value)) => value,
                Ok(Err(_)) => Err("relay request ended before EOSE".to_string()),
                Err(_) => Err("relay request timed out".to_string()),
            }
        };
        self.finish_request(&id).await;
        outcome
    }

    async fn finish_request(&self, id: &str) {
        self.requests.lock().await.remove(id);
        let mut state = self.state.lock().await;
        state.transient.retain(|subscription| subscription.id != id);
        state.removed.insert(id.to_string());
        drop(state);
        let _ = self.wake.try_send(());
    }

    /// Replaces the desired subscription set and wakes the loop to reconcile.
    ///
    /// Reconciliation is declarative rather than incremental: callers state
    /// what they want and the loop diffs. An incremental add/remove API would
    /// have to be replayed in order across a reconnect, which is exactly the
    /// bug class this avoids.
    ///
    /// It is also why `open` needs no revision/generation guard. Every
    /// reconcile re-reads the current desired set, so a change that lands
    /// mid-pass is picked up by the wake it queued rather than having to
    /// invalidate work already in flight.
    ///
    /// That argument holds only for state that is a function of the final
    /// desired set. It does not hold for `retries`, whose validity depends on
    /// the id having been *continuously* desired — history that coalescing
    /// erases. So departures are recorded here, at the only point that can see
    /// them.
    pub(crate) async fn set_subscriptions(&self, subscriptions: Vec<Subscription>) {
        let violations = self.state.lock().await.replace_desired(subscriptions);
        for id in violations {
            eprintln!(
                "buzz-desktop: native_relay_client: subscription {id} changed filter under a \
                 reused id; ids must be derived from their filter"
            );
        }
        // A full channel already means "reconcile pending", so a failed send
        // is success: the loop has not yet consumed the previous wake.
        let _ = self.wake.try_send(());
    }

    pub(crate) fn shutdown(&self) {
        self.cancel.cancel();
    }
}

/// Starts a session against `relay_url` authenticated as `keys`.
///
/// Returns the handle plus the receiver for matched events. The session
/// reconnects on drop with exponential backoff and resubscribes the current
/// desired set — never a snapshot captured at connect time, so a subscription
/// change during an outage is honored by the reconnect that follows.
#[cfg(test)]
pub(crate) async fn start(
    relay_url: String,
    keys: Keys,
    auth_tag: Option<nostr::Tag>,
) -> (Arc<RelaySession>, mpsc::Receiver<MatchedEvent>) {
    let session = start_managed(relay_url, keys, auth_tag);
    let events = session.attach_archive().await;
    (session, events)
}

fn start_managed(relay_url: String, keys: Keys, auth_tag: Option<nostr::Tag>) -> Arc<RelaySession> {
    let (wake, wake_rx) = mpsc::channel(1);
    let session = Arc::new(RelaySession {
        state: Arc::new(Mutex::new(SessionState::default())),
        requests: Arc::new(Mutex::new(HashMap::new())),
        archive_events: Arc::new(Mutex::new(None)),
        wake,
        cancel: CancellationToken::new(),
    });

    tauri::async_runtime::spawn(run_session(
        relay_url,
        keys,
        auth_tag,
        Arc::clone(&session),
        wake_rx,
    ));

    session
}

async fn run_session(
    relay_url: String,
    keys: Keys,
    auth_tag: Option<nostr::Tag>,
    session: Arc<RelaySession>,
    mut wake_rx: mpsc::Receiver<()>,
) {
    let mut delay = RECONNECT_BASE_DELAY;
    loop {
        if session.cancel.is_cancelled() {
            return;
        }

        match NostrWsConnection::connect_authenticated(&relay_url, &keys, auth_tag.as_ref()).await {
            Ok(conn) => {
                // A connection that authenticated is healthy regardless of how
                // long it then lived, so backoff resets here rather than on
                // clean exit — a socket that drops after one event must not
                // inherit the previous failure's delay.
                delay = RECONNECT_BASE_DELAY;
                run_connection(conn, &session, &mut wake_rx).await;
            }
            Err(error) => {
                eprintln!("buzz-desktop: native_relay_client: connect failed: {error}");
            }
        }

        if session.cancel.is_cancelled() {
            return;
        }
        tokio::select! {
            _ = session.cancel.cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
        delay = (delay * 2).min(RECONNECT_MAX_DELAY);
    }
}

/// Drives one connected socket until it drops or the session is cancelled.
async fn run_connection(
    mut conn: NostrWsConnection,
    session: &RelaySession,
    wake_rx: &mut mpsc::Receiver<()>,
) {
    // Subscription ids currently open ON THIS SOCKET. Deliberately local: a new
    // socket has none, so reconnect resubscribes the full desired set without
    // any explicit "resubscribe" path that could drift from the normal one.
    let mut open: HashMap<String, serde_json::Value> = HashMap::new();
    // Reopen schedule for ids the relay CLOSED, keyed the same way and equally
    // local — for the same reason and one more. Backoff state cannot live in
    // `desired`: that set is reloaded from SQLite by the archive task, so a
    // subscription deleted there is re-added by the next reload. The JS port
    // could delete from its subscription map because that map WAS the desired
    // set; here the two are separate, and only this one is per-socket.
    //
    // An entry is valid only while its id has been continuously desired since
    // the CLOSED that created it, which makes eviction the whole design:
    //
    // | Eviction trigger | Where | Why it is the right edge |
    // |---|---|---|
    // | event delivered | the EVENT arm below | the subscription is demonstrably healthy |
    // | EOSE | the EOSE arm below | the relay served it, so the cause has cleared |
    // | id leaves the desired set, including intermediate states the loop never observes | `SessionState::removed`, drained at the top of `reconcile` | validity depends on history, and coalesced wakes erase it — see `set_subscriptions` |
    // | socket drops | this map is per-connection | relay policy and our own auth can change across a reconnect |
    //
    // Reconcile deliberately does NOT also prune ids merely absent from the
    // desired snapshot. That clause is unreachable: entries are minted only for
    // ids present in `open` (the CLOSED arm's guard below), ids enter `open`
    // only from a desired snapshot, and every departure from desired is
    // recorded at write time. It would kill no mutant these tests do not
    // already kill, while masking the drain that does the work.
    let mut retries: HashMap<String, ClosedRetry> = HashMap::new();

    if !reconcile(&mut conn, session, &mut open, &mut retries).await {
        return;
    }

    loop {
        // Earliest pending reopen, or `None` when nothing is scheduled. The arm
        // below is disabled in that case rather than sleeping on a far-future
        // instant, so an idle connection never wakes on this branch.
        let retry_at = retries.values().filter_map(|retry| retry.due_at).min();

        tokio::select! {
            _ = session.cancel.cancelled() => {
                let _ = conn.disconnect().await;
                return;
            }
            Some(()) = wake_rx.recv() => {
                if !reconcile(&mut conn, session, &mut open, &mut retries).await {
                    return;
                }
            }
            // The edge that makes a CLOSED recoverable. Without it, nothing
            // re-enters `reconcile` unless the desired set changes again, and
            // for a stable set that means the subscription is dead for the life
            // of the socket.
            _ = tokio::time::sleep_until(retry_at.unwrap_or_else(Instant::now)),
                if retry_at.is_some() =>
            {
                for retry in retries.values_mut() {
                    if retry.due_at.is_some_and(|due| due <= Instant::now()) {
                        retry.due_at = None;
                    }
                }
                if !reconcile(&mut conn, session, &mut open, &mut retries).await {
                    return;
                }
            }
            message = conn.next_event(READ_TIMEOUT) => {
                match message {
                    Ok(RelayMessage::Event { subscription_id, event }) => {
                        // Only forward events for a subscription we still want.
                        // A CLOSE races in flight with events already queued at
                        // the relay, so this is the last line of defense
                        // against delivering out-of-scope events after a change.
                        //
                        // This arm drops rather than heals: an event for an id
                        // we do not have open is generation-ambiguous — it may
                        // predate a deletion — so it cannot serve as the fence
                        // an EOSE does. The EOSE arm below is where an
                        // open-map mismatch is repaired.
                        if !open.contains_key(&subscription_id) {
                            continue;
                        }
                        let pending = session
                            .requests
                            .lock()
                            .await
                            .contains_key(&subscription_id);
                        if pending {
                            // Reject forged finite-request events before
                            // retaining them, bounding memory at the transport
                            // seam. The catalog re-verifies defensively before
                            // head selection.
                            if event.verify().is_err() {
                                continue;
                            }
                            if let Some(request) = session
                                .requests
                                .lock()
                                .await
                                .get_mut(&subscription_id)
                            {
                                request.events.push(*event);
                            }
                            continue;
                        }
                        // Delivery proves the subscription is healthy, so any
                        // accumulated backoff for it is stale. Mirrors the JS
                        // port's per-event `closedRetryAttempt = 0`.
                        retries.remove(&subscription_id);
                        // Persistent archive subscriptions are live-only, so
                        // losing an event cannot be repaired with a later REQ.
                        // Await the bounded archive channel to push back on the
                        // socket read loop instead. Finite catalog requests are
                        // fulfilled above and never enter this channel.
                        // Because this await is outside the session-cancel select,
                        // teardown depends on `run_sync` dropping its receiver; moving
                        // ownership or spawning that teardown can strand the socket loop.
                        let sender = session.archive_events.lock().await.clone();
                        if let Some(sender) = sender {
                            let _ = sender
                                .send(MatchedEvent {
                                    subscription_id,
                                    event,
                                })
                                .await;
                        }
                    }
                    Ok(RelayMessage::Closed { subscription_id, message }) => {
                        // The relay dropped it; forget it so a reopen re-sends
                        // REQ rather than assuming it is still live.
                        //
                        // A CLOSED for a subscription this socket is not
                        // running is stale — our own CLOSE raced it, exactly as
                        // the EVENT arm above guards. Minting retry state from
                        // it would resurrect the entry the drain just pruned,
                        // and nothing would evict it: the id is gone from
                        // `desired`, so no future removal can record it again.
                        if open.remove(&subscription_id).is_none() {
                            continue;
                        }
                        if let Some(request) = session.requests.lock().await.remove(&subscription_id) {
                            let _ = request.complete.send(Err(format!("relay closed request: {message}")));
                            let mut state = session.state.lock().await;
                            state.transient.retain(|subscription| subscription.id != subscription_id);
                            state.removed.insert(subscription_id.clone());
                            drop(state);
                            let _ = session.wake.try_send(());
                            continue;
                        }
                        let retry = retries.entry(subscription_id.clone()).or_default();
                        retry.schedule(&message);
                        eprintln!(
                            "buzz-desktop: native_relay_client: relay closed {subscription_id}: {message}"
                        );
                    }
                    Ok(RelayMessage::Eose { subscription_id }) => {
                        // The relay served this subscription, so whatever
                        // caused an earlier CLOSED has cleared. Same reset the
                        // JS port performs in `handleSubscriptionEose`, and it
                        // is what keeps an intermittent relay from ratcheting
                        // its way to the 30s ceiling and staying there.
                        let was_open = open.contains_key(&subscription_id);
                        if let Some(request) = session.requests.lock().await.remove(&subscription_id) {
                            let _ = request.complete.send(Ok(request.events));
                            let mut state = session.state.lock().await;
                            state.transient.retain(|subscription| subscription.id != subscription_id);
                            state.removed.insert(subscription_id.clone());
                            drop(state);
                            let _ = session.wake.try_send(());
                            continue;
                        }
                        retries.remove(&subscription_id);
                        // The relay is running a subscription this socket does
                        // not think is open, so the two disagree. EOSE is the
                        // fence that makes this recoverable: frames on one
                        // socket are ordered, so a stale CLOSED from a previous
                        // generation of this id necessarily precedes the
                        // recreated generation's EOSE. Without this wake a
                        // terminal stale CLOSED is a blackhole — it clears
                        // `open`, sets no `due_at`, and so leaves no edge back
                        // into reconcile while the relay delivers events the
                        // EVENT arm silently drops.
                        //
                        // Deliberately not on the EVENT arm: an event for an
                        // absent id may belong to the old generation, so it is
                        // not a fence. Converges rather than storms — the
                        // reconcile this triggers reopens the id, and the
                        // replacement EOSE then finds it open.
                        if !was_open {
                            let _ = session.wake.try_send(());
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if !is_read_timeout(&error) {
                            eprintln!("buzz-desktop: native_relay_client: read failed: {error}");
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// Brings the socket's open subscriptions in line with the desired set.
///
/// Returns false when the socket failed and the caller should reconnect.
async fn reconcile(
    conn: &mut NostrWsConnection,
    session: &RelaySession,
    open: &mut HashMap<String, serde_json::Value>,
    retries: &mut HashMap<String, ClosedRetry>,
) -> bool {
    // Snapshot and drain in ONE acquisition. Taking them separately would let a
    // `set_subscriptions` land in the gap, spending its removal against a
    // desired set captured before it — reopening a subscription the caller had
    // just dropped, with no record left to catch it on the next pass.
    let (desired, removed) = {
        let mut state = session.state.lock().await;
        let removed = std::mem::take(&mut state.removed);
        (
            state
                .desired
                .iter()
                .chain(&state.transient)
                .cloned()
                .collect::<Vec<_>>(),
            removed,
        )
    };

    // Retry state is only valid while its id has been continuously desired
    // since the CLOSED that created it. Every departure is here even when the
    // id is desired again now, because the loop cannot see the gap: coalesced
    // wakes make remove-then-re-add one pass whose desired set never lost it.
    for id in removed {
        retries.remove(&id);
    }

    for id in open.keys().cloned().collect::<Vec<_>>() {
        if desired.iter().any(|s| s.id == id) {
            continue;
        }
        if conn
            .send_raw(&serde_json::json!(["CLOSE", id]))
            .await
            .is_err()
        {
            return false;
        }
        open.remove(&id);
    }

    for sub in desired {
        // A filter change under the same id must reopen, not be skipped: the
        // relay replaces a subscription by id, so re-sending REQ is the update.
        if open.get(&sub.id) == Some(&sub.filter) {
            continue;
        }
        // Held back by a CLOSED: either waiting out its backoff, or terminal
        // and never to be retried on this socket. Both are `is_blocked`, which
        // is what keeps a relay that rejects on policy from being re-asked at
        // the speed of the event loop.
        if retries.get(&sub.id).is_some_and(ClosedRetry::is_blocked) {
            continue;
        }
        if conn
            .send_raw(&serde_json::json!(["REQ", sub.id, sub.filter]))
            .await
            .is_err()
        {
            return false;
        }
        open.insert(sub.id, sub.filter);
    }

    true
}

/// Reopen schedule for one subscription the relay CLOSED.
#[derive(Default)]
struct ClosedRetry {
    /// When the reopen is due. `None` means "not waiting": either the delay has
    /// elapsed and reconcile may re-send, or `terminal` latched.
    due_at: Option<Instant>,
    /// Consecutive CLOSEDs, driving the exponential delay. Reset by a delivered
    /// event or EOSE, both of which drop the whole entry.
    attempts: u32,
    /// The relay rejected this filter for a reason retrying cannot change.
    terminal: bool,
}

impl ClosedRetry {
    /// True while reconcile must leave this subscription closed.
    fn is_blocked(&self) -> bool {
        self.terminal || self.due_at.is_some_and(|due| due > Instant::now())
    }

    /// Records a CLOSED and schedules the reopen its class calls for.
    fn schedule(&mut self, message: &str) {
        match classify_closed(message) {
            // Auth, access, or filter errors will fail identically until
            // something outside this socket changes, so stop asking. Scoped to
            // this socket by construction: the state lives in `run_connection`,
            // so a reconnect retries once through the normal path. That is
            // deliberate — relay policy and our own auth can change across a
            // reconnect, and one REQ per reconnect is bounded.
            ClosedClass::Terminal => {
                self.terminal = true;
                self.due_at = None;
            }
            ClosedClass::RateLimited => {
                // Arm the process-wide gate so the HTTP bridge backs off too,
                // rather than keeping a second private notion of the same
                // relay's back-pressure.
                let hint = parse_retry_in_seconds(message);
                crate::relay_admission::activate_rate_limit(hint);
                let hinted = hint
                    .map(Duration::from_secs)
                    .unwrap_or(CLOSED_RATE_LIMIT_DEFAULT);
                // The longer of the two: a short hint must not undercut a
                // backoff already grown by repeated rejections.
                self.due_at = Some(Instant::now() + self.backoff().max(hinted));
                self.attempts = self.attempts.saturating_add(1);
            }
            ClosedClass::Retryable => {
                self.due_at = Some(Instant::now() + self.backoff());
                self.attempts = self.attempts.saturating_add(1);
            }
        }
    }

    /// Exponential delay for the current attempt, capped. The shift is bounded
    /// before it is taken, so a long-lived rejection cannot overflow its way
    /// back down to a short delay.
    fn backoff(&self) -> Duration {
        CLOSED_RETRY_BASE_DELAY
            .saturating_mul(1_u32 << self.attempts.min(16))
            .min(CLOSED_RETRY_MAX_DELAY)
    }
}

/// How a CLOSED message should be handled.
///
/// Ported from `classifyRelayClosed` in `relayClosedPolicy.ts`; the prefixes are
/// the relay's own machine-readable NIP-01 classes and must stay in step with
/// that file.
#[derive(Debug, PartialEq, Eq)]
enum ClosedClass {
    Retryable,
    RateLimited,
    Terminal,
}

fn classify_closed(message: &str) -> ClosedClass {
    let normalized = message.trim().to_ascii_lowercase();
    if normalized.starts_with("rate-limited:") {
        return ClosedClass::RateLimited;
    }
    // `auth-required:` is deliberately absent, i.e. retryable: it occurs
    // transiently when a REQ races the AUTH handshake after a reconnect, and
    // the backoff reopen re-sends once authenticated. A session that is
    // genuinely unauthenticated fails at `connect_authenticated` instead, so
    // this cannot loop forever.
    if [
        "restricted:",
        "blocked:",
        "invalid:",
        "pow:",
        "duplicate:",
        "unsupported:",
        "error: mixed search",
        "error: too many subscriptions",
    ]
    .iter()
    .any(|prefix| normalized.starts_with(prefix))
    {
        return ClosedClass::Terminal;
    }
    ClosedClass::Retryable
}

/// Parses the relay's canonical `retry in Ns` hint. Same format the HTTP bridge
/// parses in `relay::extract_retry_in_hint`.
fn parse_retry_in_seconds(message: &str) -> Option<u64> {
    let after = &message[message.find("retry in ")? + "retry in ".len()..];
    after
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// A lapsed read is an idle relay, not a failure. Distinguished by variant
/// rather than by message text so a reworded error cannot turn every idle
/// period into a reconnect storm.
fn is_read_timeout(error: &buzz_ws_client_pkg::WsClientError) -> bool {
    matches!(error, buzz_ws_client_pkg::WsClientError::Timeout)
}

#[cfg(test)]
#[path = "native_relay_client_tests.rs"]
mod closed_recovery_tests;

#[cfg(test)]
mod relay_backed_tests {
    use super::*;
    use nostr::{EventBuilder, Tag};

    /// Relay-backed proof that the session's wire shape is one a real relay
    /// accepts and answers.
    ///
    /// Every other test in this commit drives `run_sync` through a fake
    /// [`crate::archive::sync::ArchiveSyncIo`], which is the right default:
    /// batching and demultiplexing are the logic worth pinning, and they must
    /// not need a socket. But a fake cannot fail the one way this layer
    /// actually can — by sending a REQ the relay rejects, or by filtering on a
    /// tag key that matches nothing. The JS manager's filters were validated by
    /// years of production traffic; this port's have been validated by my
    /// reading of that code, which is exactly the claim a real relay can check
    /// and I cannot.
    ///
    /// `#[ignore]`d because it needs a relay on `BUZZ_TEST_RELAY_URL`. Run:
    ///
    /// ```text
    /// ./scripts/start-isolated-test-relay.sh          # ws://localhost:3030
    /// BUZZ_TEST_RELAY_URL=ws://localhost:3030 \
    ///   cargo test -p buzz-desktop -- --ignored archive_sync_session
    /// ```
    #[tokio::test]
    #[ignore = "requires a local relay (set BUZZ_TEST_RELAY_URL)"]
    async fn archive_sync_session_receives_live_events_from_a_real_relay() {
        let Ok(relay_url) = std::env::var("BUZZ_TEST_RELAY_URL") else {
            panic!("set BUZZ_TEST_RELAY_URL to a running relay");
        };

        let owner = Keys::generate();
        let author = Keys::generate();
        let owner_pk = owner.public_key();

        // Kind 1 rather than the archive's own kind 24200. Publishing a real
        // observer frame requires a registered agent-owner binding in the
        // relay's database — a relay ACL concern that says nothing about this
        // layer. What this test can prove, and what no fake can, is the wire
        // shape: that the `#p` tag key and the `limit: 0` live tail produce a
        // REQ a real relay accepts and answers. Scope demultiplexing on the
        // archive side is covered in `archive/sync_tests.rs`.
        let (session, mut events) = start(relay_url.clone(), owner.clone(), None).await;
        session
            .set_subscriptions(vec![Subscription {
                id: "archive:owner_p:test".to_string(),
                filter: serde_json::json!({
                    "kinds": [1],
                    "limit": 0,
                    "#p": [owner_pk.to_hex()],
                }),
            }])
            .await;

        // The subscription must be live at the relay before the event is
        // published. A `limit: 0` filter is a live tail: it replays nothing,
        // so anything published into a not-yet-open subscription is missed.
        // That is the same ordering hazard the renderer start gate exists to
        // prevent for the ephemeral archive kind.
        tokio::time::sleep(Duration::from_secs(1)).await;

        let mut publisher = NostrWsConnection::connect_authenticated(&relay_url, &author, None)
            .await
            .expect("publisher connect");
        let frame = EventBuilder::text_note("archive-sync-probe")
            .tag(Tag::public_key(owner_pk))
            .sign_with_keys(&author)
            .expect("sign event");
        let frame_id = frame.id.to_hex();
        let ok = publisher.send_event(frame).await.expect("publish frame");
        assert!(
            ok.accepted,
            "relay rejected the observer frame, so a delivery timeout below would \
             blame the subscription for a publish failure: {}",
            ok.message
        );

        let received = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("timed out waiting for the relay to deliver the frame")
            .expect("session channel closed");

        assert_eq!(
            received.subscription_id, "archive:owner_p:test",
            "delivered event must carry the subscription id the loop demultiplexes on"
        );
        assert_eq!(
            received.event.id.to_hex(),
            frame_id,
            "must deliver the published frame"
        );

        session.shutdown();
    }
}
