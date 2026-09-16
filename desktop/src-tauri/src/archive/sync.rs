//! Rust archive sync task — the backend replacement for the renderer's
//! `archiveSyncManager`.
//!
//! Opens one live relay subscription per saved archive config and forwards
//! matched events to the existing archive pipeline in debounced batches. The
//! renderer no longer sees archive traffic at all: previously every matched
//! event crossed the IPC boundary twice (relay -> renderer, renderer ->
//! `archive_events`) purely to be written to a SQLite file the backend owns.
//!
//! # Start gate
//!
//! The task is NOT self-starting. Kind 24200 is relay-*ephemeral*: frames that
//! arrive before the listener opens are permanently lost, so the renderer must
//! finish observer reconciliation (which seeds kind 24200 into the owner_p
//! subscription) before any listener opens. That ordering is the whole reason
//! `useArchiveSync` gated on `observerReconciled`, and it survives the move as
//! an explicit `start_archive_sync` command issued after the same gate.

use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use nostr::JsonUtil;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, Mutex, Notify};
use tokio_util::sync::CancellationToken;

use super::{
    store::SaveSubscription, ArchiveBatchResult, ArchiveCandidate, MatchedScope, ScopeType,
};
use crate::app_state::AppState;
#[path = "sync_ingress.rs"]
mod ingress;
use ingress::{archive_scoped, SyncAdmission, SyncOutcome, SyncScope};
#[path = "sync_runner.rs"]
mod runner;
use crate::native_relay_client::{MatchedEvent, NativeRelayClient, RelaySession, Subscription};
use runner::*;

/// Flush once this many events are buffered. Parity with the renderer manager.
const FLUSH_BATCH_SIZE: usize = 25;
/// Maximum time an event waits in the buffer before being flushed.
///
/// This is a deadline measured from the FIRST buffered event, not an idle
/// timer that each arrival extends. The renderer constant was named
/// `FLUSH_IDLE_MS`, but its `scheduleFlush` returned early when a timer was
/// already pending, so a steady trickle still flushed every 2s rather than
/// never. The behavior is preserved; the name is corrected.
const FLUSH_DEADLINE: Duration = Duration::from_millis(2_000);

/// Emitted after a batch persists new agent-metric rows, so the renderer can
/// invalidate its usage queries. Replaces the in-process `notifyAgentMetrics
/// Changed()` call the manager made on the JS side of that same batch.
const AGENT_METRICS_CHANGED_EVENT: &str = "archive-agent-metrics-changed";

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Everything the sync loop needs from the outside world.
///
/// Injected rather than reached for so the loop's batching, demultiplexing,
/// and reload behavior are testable without a relay, a database, or a Tauri
/// app handle.
pub(crate) trait ArchiveSyncIo: Send + Sync + 'static {
    fn list_subscriptions(&self) -> BoxFuture<'_, Result<Vec<SaveSubscription>, String>>;
    fn set_subscriptions(&self, subscriptions: Vec<Subscription>) -> BoxFuture<'_, ()>;
    fn archive(
        &self,
        candidates: Vec<ArchiveCandidate>,
    ) -> BoxFuture<'_, Result<ArchiveBatchResult, String>>;
    fn archive_retry(&self, candidates: Vec<ArchiveCandidate>) -> BoxFuture<'_, SyncOutcome> {
        Box::pin(async move {
            let retry = candidates.clone();
            match self.archive(candidates).await {
                Ok(committed) => SyncOutcome {
                    committed,
                    retry: Vec::new(),
                },
                Err(_) => SyncOutcome::retry(retry),
            }
        })
    }
    fn invalidated(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
    fn report_degraded(&self, reason: &'static str, count: usize) {
        eprintln!("buzz-desktop: archive degraded: {reason} ({count})");
    }
    fn notify_agent_metrics_changed(&self);
}

// ── Subscription planning ────────────────────────────────────────────────────

/// The relay subscription set for `subscriptions`, plus the scope each
/// subscription id maps back to when its events arrive.
///
/// The id encodes scope AND kinds, so a kinds change produces a different id:
/// the session then closes the old subscription and opens the new one instead
/// of leaving a stale filter live. Same reason the renderer keyed on both.
fn plan_subscriptions(
    subscriptions: &[SaveSubscription],
) -> (Vec<Subscription>, HashMap<String, MatchedScope>) {
    let mut planned = Vec::new();
    let mut scopes = HashMap::new();

    for sub in subscriptions {
        let Some(scope_type) = parse_scope_type(&sub.scope_type) else {
            eprintln!(
                "buzz-desktop: archive sync: unknown scope_type {:?}, skipping",
                sub.scope_type
            );
            continue;
        };
        // A malformed `kinds` column decodes as empty, matching the renderer
        // decoder. The resulting filter matches nothing, which is the correct
        // failure for a row we cannot interpret: archive nothing, drop nothing.
        let kinds: Vec<u64> = serde_json::from_str(&sub.kinds).unwrap_or_default();
        let id = subscription_id(&scope_type, &sub.scope_value, &kinds);
        if scopes.contains_key(&id) {
            continue;
        }
        planned.push(Subscription {
            id: id.clone(),
            filter: build_filter(&scope_type, &sub.scope_value, &kinds),
        });
        scopes.insert(
            id,
            MatchedScope {
                scope_type,
                scope_value: sub.scope_value.clone(),
            },
        );
    }

    (planned, scopes)
}

fn parse_scope_type(raw: &str) -> Option<ScopeType> {
    match raw {
        "channel_h" => Some(ScopeType::ChannelH),
        "owner_p" => Some(ScopeType::OwnerP),
        "referenced_e" => Some(ScopeType::ReferencedE),
        _ => None,
    }
}

/// `limit: 0` — live tail only. Stored events are archived by the explicit
/// backfill paths, so a non-zero limit would re-deliver history on every
/// reconnect.
fn build_filter(scope_type: &ScopeType, scope_value: &str, kinds: &[u64]) -> serde_json::Value {
    let tag = match scope_type {
        ScopeType::ChannelH => "#h",
        ScopeType::OwnerP => "#p",
        ScopeType::ReferencedE => "#e",
    };
    json!({ "kinds": kinds, "limit": 0, tag: [scope_value] })
}

fn subscription_id(scope_type: &ScopeType, scope_value: &str, kinds: &[u64]) -> String {
    let mut sorted = kinds.to_vec();
    sorted.sort_unstable();
    let kinds = sorted
        .iter()
        .map(|k| k.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("archive:{}:{scope_value}:{kinds}", scope_type.as_str())
}

// ── Production wiring ────────────────────────────────────────────────────────

struct AppIo {
    app: AppHandle,
    session: Arc<RelaySession>,
    scope: Arc<SyncScope>,
}

impl ArchiveSyncIo for AppIo {
    fn list_subscriptions(&self) -> BoxFuture<'_, Result<Vec<SaveSubscription>, String>> {
        Box::pin(async move {
            let state: State<'_, AppState> = self.app.state();
            let (identity_pk, relay_url) = self.scope.data_scope();
            state
                .archive_db
                .with_conn(move |conn| {
                    super::store::list_save_subscriptions(conn, &identity_pk, &relay_url)
                })
                .await
        })
    }

    fn set_subscriptions(&self, subscriptions: Vec<Subscription>) -> BoxFuture<'_, ()> {
        Box::pin(async move { self.session.set_subscriptions(subscriptions).await })
    }

    fn archive(
        &self,
        candidates: Vec<ArchiveCandidate>,
    ) -> BoxFuture<'_, Result<ArchiveBatchResult, String>> {
        Box::pin(async move {
            let outcome = self.archive_retry(candidates).await;
            if outcome.retry.is_empty() {
                Ok(outcome.committed)
            } else {
                Err("archive requires retry".into())
            }
        })
    }

    fn archive_retry(&self, candidates: Vec<ArchiveCandidate>) -> BoxFuture<'_, SyncOutcome> {
        Box::pin(archive_scoped(&self.app, self.scope.clone(), candidates))
    }
    fn invalidated(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {
            let _ = self
                .scope
                .signer
                .run(std::future::pending::<Result<(), String>>())
                .await;
        })
    }
    fn report_degraded(&self, reason: &'static str, count: usize) {
        self.scope
            .notify_current(&self.app.state::<AppState>(), || {
                let _ = self.app.emit(
                    "archive-sync-degraded",
                    json!({"reason": reason, "count": count}),
                );
            });
    }
    fn notify_agent_metrics_changed(&self) {
        self.scope
            .notify_current(&self.app.state::<AppState>(), || {
                let _ = self.app.emit(AGENT_METRICS_CHANGED_EVENT, ());
            });
    }
}

/// Managed handle for the running sync task.
#[derive(Default)]
pub struct ArchiveSyncState {
    running: Mutex<Option<RunningSync>>,
    parked: Mutex<Option<ParkedSync>>,
    /// Highest `(epoch, lease)` this process has seen from either command.
    ///
    /// The renderer allocates leases synchronously in effect order, so they are
    /// the app's intent order — which the IPC completion order is not. Both
    /// commands ignore anything older, which is what makes a stale cleanup
    /// harmless and a delayed start unable to resurrect a stopped task.
    ///
    /// The epoch is minted here, not in the renderer, because a lease counter
    /// only exists for as long as the JS realm that holds it. A renderer reload
    /// (`useReloadShortcut`, `RootErrorBoundary`, `useCommunityInit`) resets the
    /// counter to zero while this state persists in the Tauri process, so
    /// without an epoch the first post-reload start looks older than what the
    /// backend already saw and is rejected forever. Ordering lexicographically
    /// on `(epoch, lease)` means a newer realm outranks the old one no matter
    /// where its local counter restarted.
    ///
    /// Every boundary the intent-order authority crosses, and why it holds:
    ///
    /// - effect remount, same realm: leases strictly increase within a realm.
    /// - IPC arrival order: the lease is minted before `invoke`, so intent
    ///   order is fixed before the calls can race.
    /// - renderer realm reload: a new epoch from this authority outranks the
    ///   dead realm's, whatever its counter said.
    /// - webview recreation of the owning window: same as reload.
    /// - a second window: it does not participate, by ownership rule. Archive
    ///   sync is app-global and main-window-owned, exactly as the main window
    ///   remains the owner of microphone capture (see `huddle::window`).
    ///   Epochs order realms in time; a companion window is a second realm in
    ///   space, and newest-wins cannot model two concurrent owners — a
    ///   companion's cleanup would cancel the live main-window task. Secondary
    ///   realms therefore never announce and never issue lifecycle commands.
    ///   Any future second-realm mount must revisit this.
    /// - Tauri process restart: both clocks die together, so there is nothing
    ///   to order against.
    latest: Mutex<(u64, u64)>,
}

struct RunningSync {
    /// Identity + relay this task is bound to. A start request for the same
    /// scope is a no-op, so a renderer remount does not churn the socket.
    scope: (String, String),
    generation: Option<u64>,
    signer: Option<crate::active_user_signer::ActiveUserSigner>,
    admission: Arc<SyncAdmission>,
    task: Option<tokio::task::JoinHandle<Vec<ArchiveCandidate>>>,
    cancel: CancellationToken,
    reload: Arc<Notify>,
}

struct ParkedSync {
    scope: (String, String),
    candidates: Vec<ArchiveCandidate>,
}

/// Proof that the holder is the current archive-sync owner, and the lock that
/// makes it true. Minted only by [`ArchiveSyncState::begin`], and required by
/// [`NativeRelayClient::archive_session`].
///
/// Acquiring the shared relay session has to happen *inside* the ownership
/// critical section, not after it. `NativeRelayClient::ensure_session` shuts
/// down the previous scope's socket and installs its own within its own lock,
/// and `attach_archive` replaces the session's archive event sender outright.
/// Both are destructive on entry, so a superseded start that merely
/// re-validated its mark *after* acquiring would already have torn down the
/// newer owner's session — with nothing to restore it from, since a session is
/// spawned rather than handed back. The damage is done by the call, so the
/// fence has to be around the call.
///
/// Holding both guards across that acquisition is sound because it performs no
/// I/O: `ensure_session` and `attach_archive` await only mutex acquisitions,
/// `shutdown` is a synchronous cancel, and the socket connects on the task
/// `start_managed` spawns. If either half ever grows an awaited network
/// round-trip, this design must be revisited rather than quietly extended.
///
/// The lock order through the whole unit is `latest` -> `running` -> `current`
/// -> `archive_events`, and nothing acquires in the reverse direction:
/// [`ArchiveSyncState::end`] and
/// [`ArchiveSyncState::notify_subscriptions_changed`] take the archive locks in
/// the same order and never reach into the session, and the session's own paths
/// (`session`, `fetch_events`, `run_session`) never reach back into archive
/// state. So there is no cycle to deadlock on.
///
/// **What this token does not cover.** It serializes archive lifecycle against
/// archive lifecycle, and nothing else needs it to: [`NativeRelayClient::session`]
/// — the persona catalog's and unread catch-up's entry point — cannot replace
/// the installed scope at all. It shares the session only on an exact scope
/// match and otherwise leases a private one, so a finite request that arrives
/// while a different scope is installed can neither shut this session down nor
/// steal its archive sender. That is the only reason holding the token across
/// acquisition is sufficient rather than merely necessary; if a second
/// destructive path is ever added, it must take this token too.
///
/// The fields are private and the type is un-constructible outside this module,
/// so the stale-start path is a compile error rather than a race to remember.
/// Dropping the token releases ownership, which is why the command holds it
/// until the sync task is spawned.
pub(crate) struct ArchiveOwnership<'a> {
    cleanup_uninstalled: bool,
    /// Field order is the lock order `begin` and `end` both take: `latest`,
    /// then `running`. Rust drops fields in declaration order, so releasing
    /// mirrors acquiring and the two halves can never interleave.
    _latest: tokio::sync::MutexGuard<'a, (u64, u64)>,
    _running: tokio::sync::MutexGuard<'a, Option<RunningSync>>,
}

// An installation owns cleanup until the runner is installed. This also covers
// canceled IPC futures and every `?` after ownership was reserved.
impl Drop for ArchiveOwnership<'_> {
    fn drop(&mut self) {
        if self.cleanup_uninstalled
            && self
                ._running
                .as_ref()
                .is_some_and(|running| running.task.is_none())
        {
            if let Some(running) = self._running.take() {
                running.admission.revoke();
                running.cancel.cancel();
            }
        }
    }
}

impl ArchiveOwnership<'_> {
    /// Call after waiting for ownership, before any destructive relay operation.
    /// The closure captures current workspace authority, not replacement authority
    /// for the operation itself. A failure is cleaned up by this token's Drop.
    fn validate_install<T>(
        &self,
        validate: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        validate()
    }
}

impl ArchiveSyncState {
    /// Wakes the sync task so it reloads saved subscriptions.
    ///
    /// Called by the archive commands that mutate `save_subscriptions`. This
    /// replaces the renderer's `onSubscriptionChange` notifier: the mutations
    /// were already backend commands, so routing the signal through JS only
    /// created a window where a write landed but nothing resubscribed.
    pub(super) async fn notify_subscriptions_changed(&self) {
        if let Some(running) = self.running.lock().await.as_ref() {
            running.reload.notify_one();
        }
    }

    /// Mints the epoch a renderer realm must hold before it may issue any
    /// lifecycle command, and publishes it as the current mark in the same
    /// critical section.
    ///
    /// A realm has to obtain this *before* its archive effect runs, and the
    /// renderer awaits it. If announcing were just another unawaited `invoke`
    /// beside the lifecycle calls, it would race them and recreate the
    /// arrival-order bug one level up — the epoch would order announcements
    /// rather than realms.
    ///
    /// Minting and publishing are one lock acquisition because announcing is
    /// what supersedes the old realm. Holding the epoch counter separately —
    /// so a mint could not block an in-flight lifecycle call — leaves a window
    /// between mint and first use in which `latest` still names the dead realm,
    /// and its delayed `start`/`stop` with any lease still wins. The new realm
    /// cannot close that window itself: the reconciliation gate can keep its
    /// first lifecycle call arbitrarily far behind its announcement.
    ///
    /// The published lease is `0`, the one value that outranks every mark the
    /// previous realm can hold while still sitting below this realm's own first
    /// lease. Publishing higher would reject the announcing realm's own start
    /// and leave sync permanently unstarted.
    async fn announce(&self) -> u64 {
        let mut latest = self.latest.lock().await;
        let epoch = latest.0 + 1;
        *latest = (epoch, 0);
        epoch
    }

    /// Takes ownership for a start under `(epoch, lease)`, installing the task
    /// when it wins. Returns `None` when the caller must not proceed — either
    /// the mark is stale or an equivalent task is already running.
    ///
    /// The whole ownership policy lives here rather than in the command so the
    /// regression tests drive the same code production does. A test that
    /// re-implemented "claim, then install" would pass against a command that
    /// had stopped calling either one.
    ///
    /// The mark must be strictly newer than anything seen: that rejects both a
    /// start a newer start already superseded, and one delayed past its own
    /// stop. Comparison is lexicographic on `(epoch, lease)`, so any call from
    /// a superseded realm loses regardless of how far its lease counter ran.
    ///
    /// The winner receives an [`ArchiveOwnership`] that keeps both guards held,
    /// and [`NativeRelayClient::archive_session`] cannot be called without one.
    /// Acquiring the shared session is therefore inside this critical section
    /// rather than after it — see [`ArchiveOwnership`] for why "revalidate the
    /// mark afterwards" cannot work here.
    #[cfg(test)]
    async fn begin(
        &self,
        mark: (u64, u64),
        scope: (String, String),
        cancel: CancellationToken,
        reload: Arc<Notify>,
    ) -> Option<ArchiveOwnership<'_>> {
        self.begin_generation(mark, scope, None, cancel, reload)
            .await
    }

    async fn begin_generation(
        &self,
        mark: (u64, u64),
        scope: (String, String),
        generation: Option<u64>,
        cancel: CancellationToken,
        reload: Arc<Notify>,
    ) -> Option<ArchiveOwnership<'_>> {
        let mut latest = self.latest.lock().await;
        if mark <= *latest {
            return None;
        }
        *latest = mark;

        let mut running = self.running.lock().await;
        // A same-scope remount keeps its socket: reinstalling would tear down a
        // healthy relay session to replace it with an identical one.
        if running.as_ref().is_some_and(|current| {
            current.scope == scope
                && current.generation == generation
                && !current.cancel.is_cancelled()
                && current.task.as_ref().is_none_or(|task| !task.is_finished())
        }) {
            return None;
        }
        if let Some(previous) = running.take() {
            self.join_and_park(previous).await;
        }
        *running = Some(RunningSync {
            scope,
            generation,
            signer: None,
            admission: Arc::new(SyncAdmission::default()),
            task: None,
            cancel,
            reload,
        });
        Some(ArchiveOwnership {
            cleanup_uninstalled: false,
            _latest: latest,
            _running: running,
        })
    }

    async fn take_parked(&self, scope: &(String, String)) -> Vec<ArchiveCandidate> {
        self.parked
            .lock()
            .await
            .take()
            .filter(|p| &p.scope == scope)
            .map_or_else(Vec::new, |p| p.candidates)
    }

    // latest/running serialize handoff; the runner never needs either lock.
    // No auth/workspace/DB/run-admission guard survives this join.
    async fn join_and_park(&self, mut running: RunningSync) {
        running.admission.revoke();
        running.cancel.cancel();
        let candidates = match running.task.take() {
            Some(task) => match task.await {
                Ok(pending) => pending,
                Err(error) => {
                    eprintln!("buzz-desktop: archive runner failed: {error}");
                    Vec::new()
                }
            },
            None => Vec::new(),
        };
        *self.parked.lock().await = Some(ParkedSync {
            scope: running.scope,
            candidates,
        });
    }

    /// Workspace replacement invalidates and joins before changing relay/keys.
    /// The workspace lock may be held: runner admission uses try_lock, never waits.
    pub(crate) async fn stop_for_workspace(&self) {
        let _latest = self.latest.lock().await;
        if let Some(old) = self.running.lock().await.take() {
            self.join_and_park(old).await;
        }
    }

    /// Auth invalidation precedes this call. Join only revoked generations;
    /// a concurrently installed healthy generation must not be torn down.
    pub(crate) async fn stop_invalidated(&self) {
        let _latest = self.latest.lock().await;
        let mut running = self.running.lock().await;
        if running.as_ref().is_some_and(|r| {
            r.signer.as_ref().is_some_and(|s| s.check_valid().is_err())
                || r.cancel.is_cancelled()
                || r.task.as_ref().is_some_and(|t| t.is_finished())
        }) {
            if let Some(old) = running.take() {
                self.join_and_park(old).await;
            }
        }
    }

    /// Releases ownership for a stop under `(epoch, lease)`, cancelling the
    /// running task when it wins.
    ///
    /// Equality succeeds here, unlike [`Self::begin`]: a stop is the
    /// counterpart of the start that minted its lease, so its own mark is
    /// exactly the case it must act on. Advancing the mark is what stops a
    /// start delayed past its own cleanup from resurrecting the task.
    ///
    /// The guard is held across the cancellation, exactly as [`Self::begin`]
    /// holds it across the install. Releasing it first would reopen the very
    /// window this ordering closes: a stop could clear its check, yield, let a
    /// newer start install its task, and then cancel that task on resume —
    /// stale cleanup stranding the newest owner. Both halves take `latest` then
    /// `running`, so the two can never interleave and the order is deadlock-free.
    async fn end(&self, mark: (u64, u64)) {
        let mut latest = self.latest.lock().await;
        if mark < *latest {
            return;
        }
        *latest = mark;

        if let Some(running) = self.running.lock().await.take() {
            self.join_and_park(running).await;
        }
    }
}

/// Announce a renderer realm and obtain its epoch.
///
/// The renderer awaits this before its archive effect may issue any lifecycle
/// command; see [`ArchiveSyncState::latest`] for the boundaries this closes.
/// Only the main window announces — archive sync is app-global and
/// main-window-owned.
#[tauri::command]
pub async fn announce_archive_sync_epoch(
    sync_state: State<'_, ArchiveSyncState>,
) -> Result<u64, String> {
    Ok(sync_state.announce().await)
}

/// Start archive sync for the current identity.
///
/// Idempotent for the same identity + relay. Issued by the renderer only after
/// observer reconciliation completes — see the module docs for why that gate
/// cannot be moved into the backend.
///
/// `epoch` identifies the calling realm and `lease` orders this call against
/// that realm's other lifecycle calls; see [`ArchiveSyncState::latest`].
#[tauri::command]
pub async fn start_archive_sync(
    app: AppHandle,
    state: State<'_, AppState>,
    sync_state: State<'_, ArchiveSyncState>,
    relay_client: State<'_, NativeRelayClient>,
    epoch: u64,
    lease: u64,
    expected_generation: Option<u64>,
) -> Result<(), String> {
    let keys = crate::native_identity::renderer_signer(&state, expected_generation)?;
    let relay_url = crate::relay::relay_ws_url_with_override(&state);
    let scope = (keys.public_key().to_hex(), relay_url.clone());

    // Only cheap handles before `begin`: a start that lost its mark, or a
    // same-scope remount, must not open a relay socket just to drop it again.
    let cancel = CancellationToken::new();
    let reload = Arc::new(Notify::new());
    let Some(mut ownership) = sync_state
        .begin_generation(
            (epoch, lease),
            scope.clone(),
            keys.generation(),
            cancel.clone(),
            Arc::clone(&reload),
        )
        .await
    else {
        return Ok(());
    };

    ownership.cleanup_uninstalled = true;
    // Ownership acquisition may have waited across logout/relogin. Validate
    // the ORIGINAL signer and workspace before touching the shared session.
    let captured = ownership.validate_install(|| {
        keys.check_valid()?;
        let current = crate::native_identity::renderer_signer(&state, expected_generation)?;
        if current.public_key() != keys.public_key() || current.generation() != keys.generation() {
            return Err("archive scope changed during start".into());
        }
        let running = ownership
            ._running
            .as_ref()
            .ok_or("archive ownership missing")?;
        let captured = Arc::new(SyncScope::capture(
            &state,
            cancel.clone(),
            running.admission.clone(),
        )?);
        if captured.signer.public_key() != keys.public_key()
            || captured.signer.generation() != keys.generation()
            || captured.data_scope() != scope
        {
            return Err("archive scope changed during start".into());
        }
        Ok(captured)
    })?;
    // No auth/SQLite guard crosses relay mutex acquisition. NativeRelayClient
    // also checks signer validity under its session-slot lock.
    let (session, events) = relay_client
        .archive_session(relay_url, keys.clone(), &ownership)
        .await;
    // Auth can invalidate during relay mutex waits as well. Never install a
    // runner with revoked authority; token Drop cancels every failed install.
    keys.check_valid()?;
    let running = ownership
        ._running
        .as_mut()
        .ok_or("archive ownership missing")?;
    running.signer = Some(keys);
    let initial = sync_state.take_parked(&scope).await;
    let io = AppIo {
        app: app.clone(),
        session: Arc::clone(&session),
        scope: captured,
    };
    running.task = Some(tokio::spawn(async move {
        let pending = run_sync_pending(&io, reload, events, cancel, initial).await;
        session.set_subscriptions(Vec::new()).await;
        pending
    }));
    Ok(())
}

/// Stop archive sync. Mirrors the renderer teardown that ran when the gate
/// closed (identity change, community switch, unmount).
///
/// `(epoch, lease)` is the mark its own start allocated; a cleanup that has
/// been superseded is a no-op rather than cancelling a newer owner's task.
#[tauri::command]
pub async fn stop_archive_sync(
    sync_state: State<'_, ArchiveSyncState>,
    epoch: u64,
    lease: u64,
) -> Result<(), String> {
    sync_state.end((epoch, lease)).await;
    Ok(())
}

#[cfg(test)]
#[path = "sync_start_tests.rs"]
mod sync_start_tests;

#[cfg(test)]
#[path = "sync_tests.rs"]
mod sync_tests;
