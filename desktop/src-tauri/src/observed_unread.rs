//! Native observed-unread read model.
//!
//! The renderer is the only writer today, so request/response ordering is the
//! delivery mechanism: there is no push channel. If native relay ingestion adds
//! a second writer, that assumption breaks; consumers must then use the same
//! revision-gap rule here to request a fresh snapshot.
//!
//! Failure contract: sequence + revision advance in the same SQLite transaction
//! as events, markers, pruning, and migration. A lost ack is replayed as a no-op;
//! a gap is rejected; stale-scope responses are fenced in the renderer. Legacy
//! rows and their migration marker commit together, and localStorage is removed
//! only after the renderer observes that marker.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

const SCHEMA_VERSION: i64 = 1;
const PER_CHANNEL_CAP: i64 = 1_000;
const GLOBAL_CAP: i64 = 5_000;
const HORIZON_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Serializes the two observed-unread commands against each other.
///
/// `Arc` because the guard is taken *inside* the blocking closure the commands
/// hand to `spawn_blocking`: a `std::sync::MutexGuard` is not `Send`, so it
/// cannot be acquired on the caller side of an await. Cloning the handle into
/// the closure keeps serialization identical while moving the wait off the
/// thread that runs the IPC handler.
#[derive(Default)]
pub(crate) struct ObservedUnreadStore {
    write_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObservedUnreadScope {
    pub(crate) pubkey: String,
    pub(crate) relay_url: String,
}

impl ObservedUnreadScope {
    fn key(&self) -> String {
        format!(
            "{}:{}",
            self.pubkey.trim().to_ascii_lowercase(),
            self.relay_url.trim().trim_end_matches('/')
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct IngestEvent {
    channel_id: String,
    id: String,
    created_at: u64,
    root_id: Option<String>,
    high_priority: bool,
    counts_toward_badge: bool,
    counts_toward_app_badge: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChannelLatestUpdate {
    channel_id: String,
    created_at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MarkerUpdate {
    context_id: String,
    read_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MembershipUpdate {
    kind: String,
    value: String,
    present: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MembershipSeed {
    participated_root_ids: Vec<String>,
    authored_root_ids: Vec<String>,
    mentioned_root_ids: Vec<String>,
    followed_root_ids: Vec<String>,
    muted_root_ids: Vec<String>,
    muted_channel_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenScopeRequest {
    scope: ObservedUnreadScope,
    legacy_payload: Option<serde_json::Value>,
    membership_seed: Option<MembershipSeed>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IngestRequest {
    scope: ObservedUnreadScope,
    sequence: u64,
    base_revision: u64,
    events: Vec<IngestEvent>,
    channel_latest: Vec<ChannelLatestUpdate>,
    markers: Vec<MarkerUpdate>,
    membership: Vec<MembershipUpdate>,
    clear_channels: Vec<String>,
    clear_all: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChannelProjection {
    channel_id: String,
    latest: u64,
    count: u64,
    badge_count: u64,
    app_badge_count: u64,
    top_level_unread: bool,
    high_priority_unread: bool,
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum ObservedUnreadResponse {
    Snapshot {
        scope: ObservedUnreadScope,
        generation: String,
        revision: u64,
        last_acked_sequence: u64,
        migration_complete: bool,
        membership_seeded: bool,
        channels: Vec<ChannelProjection>,
    },
    Delta {
        scope: ObservedUnreadScope,
        generation: String,
        base_revision: u64,
        revision: u64,
        acked_sequence: u64,
        upserts: Vec<ChannelProjection>,
        removed: Vec<String>,
    },
    SnapshotRequired {
        scope: ObservedUnreadScope,
        generation: String,
        revision: u64,
        last_acked_sequence: u64,
    },
}

fn db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("resolve observed-unread data dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create observed-unread data dir: {e}"))?;
    Ok(dir.join("observed-unread.db"))
}

fn open_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("open observed-unread db: {e}"))?;
    conn.pragma_update(None, "busy_timeout", 5_000)
        .map_err(|e| format!("configure observed-unread db: {e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("configure observed-unread WAL: {e}"))?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_meta(version INTEGER NOT NULL);
        INSERT INTO schema_meta(version) SELECT 1 WHERE NOT EXISTS(SELECT 1 FROM schema_meta);
        CREATE TABLE IF NOT EXISTS scope_state(
          scope TEXT PRIMARY KEY, generation TEXT NOT NULL, revision INTEGER NOT NULL DEFAULT 0,
          last_sequence INTEGER NOT NULL DEFAULT 0, migration_complete INTEGER NOT NULL DEFAULT 0,
          membership_seeded INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS observed_events(
          scope TEXT NOT NULL, event_id TEXT NOT NULL, channel_id TEXT NOT NULL,
          created_at INTEGER NOT NULL, root_id TEXT, high_priority INTEGER NOT NULL,
          counts_badge INTEGER NOT NULL, counts_app_badge INTEGER NOT NULL,
          PRIMARY KEY(scope,event_id));
        CREATE INDEX IF NOT EXISTS observed_events_channel ON observed_events(scope,channel_id,created_at,event_id);
        CREATE TABLE IF NOT EXISTS channel_latest(
          scope TEXT NOT NULL, channel_id TEXT NOT NULL, created_at INTEGER NOT NULL,
          PRIMARY KEY(scope,channel_id));
        CREATE TABLE IF NOT EXISTS read_markers(
          scope TEXT NOT NULL, context_id TEXT NOT NULL, read_at INTEGER NOT NULL,
          PRIMARY KEY(scope,context_id));
        CREATE TABLE IF NOT EXISTS unread_membership(
          scope TEXT NOT NULL, kind TEXT NOT NULL, value TEXT NOT NULL,
          PRIMARY KEY(scope,kind,value));")
        .map_err(|e| format!("initialize observed-unread db: {e}"))?;
    let version: i64 = conn
        .query_row("SELECT version FROM schema_meta LIMIT 1", [], |row| {
            row.get(0)
        })
        .map_err(|e| format!("read observed-unread schema: {e}"))?;
    if version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported observed-unread schema version {version}"
        ));
    }
    Ok(conn)
}

fn ensure_scope(tx: &Transaction<'_>, scope: &str) -> Result<(), String> {
    tx.execute(
        "INSERT OR IGNORE INTO scope_state(scope,generation) VALUES(?1,?2)",
        params![scope, uuid::Uuid::new_v4().to_string()],
    )
    .map_err(|e| format!("initialize observed-unread scope: {e}"))?;
    Ok(())
}

fn state(tx: &Transaction<'_>, scope: &str) -> Result<(String, u64, u64, bool, bool), String> {
    tx.query_row("SELECT generation,revision,last_sequence,migration_complete,membership_seeded FROM scope_state WHERE scope=?1", [scope], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get::<_,i64>(3)? != 0,r.get::<_,i64>(4)? != 0)))
      .map_err(|e| format!("read observed-unread scope state: {e}"))
}

fn valid_legacy_event(value: &serde_json::Value, channel_id: &str) -> Option<IngestEvent> {
    let object = value.as_object()?;
    Some(IngestEvent {
        channel_id: channel_id.to_string(),
        id: object.get("id")?.as_str()?.to_string(),
        created_at: object.get("createdAt")?.as_u64()?,
        root_id: match object.get("rootId")? {
            serde_json::Value::Null => None,
            v => Some(v.as_str()?.to_string()),
        },
        high_priority: object.get("highPriority")?.as_bool()?,
        counts_toward_badge: object.get("countsTowardBadge")?.as_bool()?,
        counts_toward_app_badge: object.get("countsTowardAppBadge")?.as_bool()?,
    })
}

fn upsert_event(tx: &Transaction<'_>, scope: &str, event: &IngestEvent) -> Result<(), String> {
    tx.execute("INSERT INTO observed_events(scope,event_id,channel_id,created_at,root_id,high_priority,counts_badge,counts_app_badge)
      VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(scope,event_id) DO NOTHING",
      params![scope,event.id,event.channel_id,event.created_at,event.root_id,event.high_priority,event.counts_toward_badge,event.counts_toward_app_badge])
      .map_err(|e| format!("upsert observed-unread event: {e}"))?;
    Ok(())
}

fn seed_membership(tx: &Transaction<'_>, scope: &str, seed: &MembershipSeed) -> Result<(), String> {
    // The renderer snapshot is authoritative while it remains the only writer.
    // Replace transactionally so removals made while Buzz was closed are not
    // silently resurrected by an insert-only seed.
    tx.execute("DELETE FROM unread_membership WHERE scope=?1", [scope])
        .map_err(|e| format!("reset unread membership: {e}"))?;
    for (kind, values) in [
        ("participated", &seed.participated_root_ids),
        ("authored", &seed.authored_root_ids),
        ("mentioned", &seed.mentioned_root_ids),
        ("followed", &seed.followed_root_ids),
        ("muted_root", &seed.muted_root_ids),
        ("muted_channel", &seed.muted_channel_ids),
    ] {
        for value in values {
            tx.execute(
                "INSERT OR IGNORE INTO unread_membership(scope,kind,value) VALUES(?1,?2,?3)",
                params![scope, kind, value],
            )
            .map_err(|e| format!("seed unread membership: {e}"))?;
        }
    }
    tx.execute(
        "UPDATE scope_state SET membership_seeded=1 WHERE scope=?1",
        [scope],
    )
    .map_err(|e| format!("mark unread membership seeded: {e}"))?;
    Ok(())
}

fn advance_channel_latest(
    tx: &Transaction<'_>,
    scope: &str,
    channel_id: &str,
    created_at: u64,
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO channel_latest(scope,channel_id,created_at) VALUES(?1,?2,?3) ON CONFLICT(scope,channel_id) DO UPDATE SET created_at=MAX(created_at,excluded.created_at)",
        params![scope, channel_id, created_at],
    )
    .map_err(|e| format!("advance channel latest: {e}"))?;
    Ok(())
}

fn seed_membership_once(
    tx: &Transaction<'_>,
    scope: &str,
    membership_seeded: bool,
    seed: Option<&MembershipSeed>,
) -> Result<(), String> {
    if membership_seeded {
        return Ok(());
    }
    if let Some(seed) = seed {
        seed_membership(tx, scope, seed)?;
    }
    Ok(())
}

fn prune(tx: &Transaction<'_>, scope: &str) -> Result<(), String> {
    let cutoff = chrono::Utc::now().timestamp() - HORIZON_SECONDS;
    tx.execute(
        "DELETE FROM observed_events WHERE scope=?1 AND created_at<=?2",
        params![scope, cutoff],
    )
    .map_err(|e| format!("age-prune observed unread: {e}"))?;
    tx.execute("DELETE FROM observed_events WHERE rowid IN (SELECT rowid FROM (SELECT rowid,ROW_NUMBER() OVER(PARTITION BY channel_id ORDER BY created_at DESC,event_id DESC) rank FROM observed_events WHERE scope=?1) WHERE rank>?2)", params![scope,PER_CHANNEL_CAP]).map_err(|e| format!("channel-prune observed unread: {e}"))?;
    tx.execute("DELETE FROM observed_events WHERE rowid IN (SELECT rowid FROM observed_events WHERE scope=?1 ORDER BY created_at DESC,event_id DESC LIMIT -1 OFFSET ?2)", params![scope,GLOBAL_CAP]).map_err(|e| format!("global-prune observed unread: {e}"))?;
    Ok(())
}

fn marker(markers: &HashMap<String, u64>, key: &str) -> u64 {
    markers.get(key).copied().unwrap_or(0)
}

fn projections(tx: &Transaction<'_>, scope: &str) -> Result<Vec<ChannelProjection>, String> {
    let mut marker_stmt = tx
        .prepare("SELECT context_id,read_at FROM read_markers WHERE scope=?1")
        .map_err(|e| format!("prepare unread markers: {e}"))?;
    let markers: HashMap<String, u64> = marker_stmt
        .query_map([scope], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("query unread markers: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("read unread markers: {e}"))?;
    let mut by_channel: HashMap<String, ChannelProjection> = HashMap::new();
    let mut latest_stmt = tx
        .prepare("SELECT channel_id,created_at FROM channel_latest WHERE scope=?1")
        .map_err(|e| format!("prepare channel latest: {e}"))?;
    for row in latest_stmt
        .query_map([scope], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
        })
        .map_err(|e| format!("query channel latest: {e}"))?
    {
        let (channel_id, latest) = row.map_err(|e| format!("read channel latest: {e}"))?;
        by_channel.insert(
            channel_id.clone(),
            ChannelProjection {
                channel_id,
                latest,
                count: 0,
                badge_count: 0,
                app_badge_count: 0,
                top_level_unread: false,
                high_priority_unread: false,
            },
        );
    }
    let mut stmt = tx.prepare("SELECT event_id,channel_id,created_at,root_id,high_priority,counts_badge,counts_app_badge FROM observed_events WHERE scope=?1 ORDER BY channel_id,created_at,event_id").map_err(|e| format!("prepare observed projection: {e}"))?;
    let rows = stmt
        .query_map([scope], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, bool>(5)?,
                r.get::<_, bool>(6)?,
            ))
        })
        .map_err(|e| format!("query observed projection: {e}"))?;
    for row in rows {
        let (id, channel, created, root, high, badge, app) =
            row.map_err(|e| format!("read observed projection: {e}"))?;
        let mut read_at = marker(&markers, &channel).max(marker(&markers, &format!("msg:{id}")));
        if let Some(root) = &root {
            read_at = read_at.max(marker(&markers, &format!("thread:{root}")));
        }
        if created <= read_at {
            continue;
        }
        let entry = by_channel
            .entry(channel.clone())
            .or_insert(ChannelProjection {
                channel_id: channel,
                latest: 0,
                count: 0,
                badge_count: 0,
                app_badge_count: 0,
                top_level_unread: false,
                high_priority_unread: false,
            });
        entry.latest = entry.latest.max(created);
        entry.count += 1;
        entry.badge_count += u64::from(badge);
        entry.app_badge_count += u64::from(app);
        entry.top_level_unread |= root.is_none();
        entry.high_priority_unread |= high;
    }
    let mut result: Vec<_> = by_channel.into_values().collect();
    result.sort_by(|a, b| a.channel_id.cmp(&b.channel_id));
    Ok(result)
}

/// Runs one observed-unread SQLite unit on the blocking pool.
///
/// A sync `#[tauri::command]` is `ExecutionContext::Blocking`, which runs the
/// body inline in the IPC handler — the main thread on macOS. The projection is
/// linear in the whole scope (measured 7.2 ms release / 27.6 ms debug at
/// 15 channels / 5000 events, and callers issue these in per-root loops during
/// catch-up), so inline execution holds the UI thread past the 16.7 ms frame
/// budget. `archive_events` next door already routes its SQLite work this way;
/// these two were the exception.
///
/// The token is what makes that structural rather than a convention. Its field
/// is private to this module, so `OnBlockingThread` cannot be constructed
/// anywhere else — and since the bodies below require one, a command that
/// stopped going through [`blocking::run`] would not compile. That covers both
/// regressions: dropping `async` leaves no way to await this, and keeping
/// `async` while calling a body directly leaves no way to obtain the token.
mod blocking {
    /// Evidence that the holder is executing on the blocking pool.
    pub(super) struct OnBlockingThread(());

    pub(super) async fn run<T, F>(task: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(OnBlockingThread) -> Result<T, String> + Send + 'static,
    {
        tauri::async_runtime::spawn_blocking(move || task(OnBlockingThread(())))
            .await
            .map_err(|error| format!("observed-unread db task failed: {error}"))?
    }
}
use blocking::OnBlockingThread;

/// Off-thread execution lets two invocations reach the lock in an order the IPC
/// arrival order no longer fixes. Nothing here depends on that order: the
/// renderer keeps one call per scope in flight, and a request that arrives
/// against a moved revision is rejected with `SnapshotRequired` rather than
/// applied — the same gate that already covers a lost ack.
#[tauri::command]
pub(crate) async fn observed_unread_open_scope(
    request: OpenScopeRequest,
    app: AppHandle,
    store: State<'_, ObservedUnreadStore>,
) -> Result<ObservedUnreadResponse, String> {
    let write_lock = Arc::clone(&store.write_lock);
    blocking::run(move |proof| open_scope_locked(proof, &write_lock, &app, request)).await
}

fn open_scope_locked(
    _proof: OnBlockingThread,
    write_lock: &Mutex<()>,
    app: &AppHandle,
    request: OpenScopeRequest,
) -> Result<ObservedUnreadResponse, String> {
    let _guard = write_lock.lock().map_err(|e| e.to_string())?;
    let mut conn = open_db(&db_path(app)?)?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("begin observed-unread open: {e}"))?;
    let scope = request.scope.key();
    ensure_scope(&tx, &scope)?;
    let (_, _, _, migration_complete, membership_seeded) = state(&tx, &scope)?;
    if !migration_complete {
        if let Some(payload) = &request.legacy_payload {
            if let Some(channels) = payload
                .get("eventsByChannel")
                .and_then(serde_json::Value::as_object)
            {
                for (channel, events) in channels {
                    if let Some(events) = events.as_array() {
                        for value in events {
                            if let Some(event) = valid_legacy_event(value, channel) {
                                upsert_event(&tx, &scope, &event)?;
                            }
                        }
                    }
                }
            }
        }
        tx.execute(
            "UPDATE scope_state SET migration_complete=1 WHERE scope=?1",
            [&scope],
        )
        .map_err(|e| format!("mark observed migration: {e}"))?;
    }
    seed_membership_once(
        &tx,
        &scope,
        membership_seeded,
        request.membership_seed.as_ref(),
    )?;
    prune(&tx, &scope)?;
    let channels = projections(&tx, &scope)?;
    let (generation, revision, last, migrated, seeded) = state(&tx, &scope)?;
    tx.commit()
        .map_err(|e| format!("commit observed-unread open: {e}"))?;
    Ok(ObservedUnreadResponse::Snapshot {
        scope: request.scope,
        generation,
        revision,
        last_acked_sequence: last,
        migration_complete: migrated,
        membership_seeded: seeded,
        channels,
    })
}

#[tauri::command]
pub(crate) async fn observed_unread_ingest(
    request: IngestRequest,
    app: AppHandle,
    store: State<'_, ObservedUnreadStore>,
) -> Result<ObservedUnreadResponse, String> {
    let write_lock = Arc::clone(&store.write_lock);
    blocking::run(move |proof| ingest_locked(proof, &write_lock, &app, request)).await
}

fn ingest_locked(
    _proof: OnBlockingThread,
    write_lock: &Mutex<()>,
    app: &AppHandle,
    request: IngestRequest,
) -> Result<ObservedUnreadResponse, String> {
    let _guard = write_lock.lock().map_err(|e| e.to_string())?;
    let mut conn = open_db(&db_path(app)?)?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("begin observed ingest: {e}"))?;
    let scope = request.scope.key();
    ensure_scope(&tx, &scope)?;
    let (generation, revision, last, _, _) = state(&tx, &scope)?;
    if request.sequence <= last {
        let channels = projections(&tx, &scope)?;
        tx.commit()
            .map_err(|e| format!("commit observed replay: {e}"))?;
        return Ok(ObservedUnreadResponse::Snapshot {
            scope: request.scope,
            generation,
            revision,
            last_acked_sequence: last,
            migration_complete: true,
            membership_seeded: true,
            channels,
        });
    }
    if request.sequence != last + 1 || request.base_revision != revision {
        return Ok(ObservedUnreadResponse::SnapshotRequired {
            scope: request.scope,
            generation,
            revision,
            last_acked_sequence: last,
        });
    }
    let before = projections(&tx, &scope)?;
    let before_by_channel: HashMap<_, _> = before
        .into_iter()
        .map(|projection| (projection.channel_id.clone(), projection))
        .collect();
    if request.clear_all {
        tx.execute("DELETE FROM observed_events WHERE scope=?1", [&scope])
            .map_err(|e| format!("clear observed scope: {e}"))?;
        tx.execute("DELETE FROM channel_latest WHERE scope=?1", [&scope])
            .map_err(|e| format!("clear channel latest scope: {e}"))?;
    }
    for channel in &request.clear_channels {
        tx.execute(
            "DELETE FROM observed_events WHERE scope=?1 AND channel_id=?2",
            params![scope, channel],
        )
        .map_err(|e| format!("clear observed channel: {e}"))?;
        tx.execute(
            "DELETE FROM channel_latest WHERE scope=?1 AND channel_id=?2",
            params![scope, channel],
        )
        .map_err(|e| format!("clear channel latest: {e}"))?;
    }
    for event in &request.events {
        upsert_event(&tx, &scope, event)?;
    }
    for update in &request.channel_latest {
        advance_channel_latest(&tx, &scope, &update.channel_id, update.created_at)?;
    }
    for update in &request.membership {
        if update.present {
            tx.execute(
                "INSERT OR IGNORE INTO unread_membership(scope,kind,value) VALUES(?1,?2,?3)",
                params![scope, update.kind, update.value],
            )
        } else {
            tx.execute(
                "DELETE FROM unread_membership WHERE scope=?1 AND kind=?2 AND value=?3",
                params![scope, update.kind, update.value],
            )
        }
        .map_err(|e| format!("update unread membership: {e}"))?;
    }
    for update in &request.markers {
        match update.read_at { Some(read_at)=>{tx.execute("INSERT INTO read_markers(scope,context_id,read_at) VALUES(?1,?2,?3) ON CONFLICT(scope,context_id) DO UPDATE SET read_at=MAX(read_at,excluded.read_at)",params![scope,update.context_id,read_at])},None=>tx.execute("DELETE FROM read_markers WHERE scope=?1 AND context_id=?2",params![scope,update.context_id])}.map_err(|e| format!("update observed marker: {e}"))?;
    }
    prune(&tx, &scope)?;
    let after = projections(&tx, &scope)?;
    let after_ids: HashSet<_> = after
        .iter()
        .map(|projection| projection.channel_id.clone())
        .collect();
    let removed: Vec<_> = before_by_channel
        .keys()
        .filter(|channel_id| !after_ids.contains(*channel_id))
        .cloned()
        .collect();
    let upserts: Vec<_> = after
        .into_iter()
        .filter(|projection| before_by_channel.get(&projection.channel_id) != Some(projection))
        .collect();
    let next_revision = revision + 1;
    tx.execute(
        "UPDATE scope_state SET revision=?2,last_sequence=?3 WHERE scope=?1",
        params![scope, next_revision, request.sequence],
    )
    .map_err(|e| format!("advance observed sequence: {e}"))?;
    tx.commit()
        .map_err(|e| format!("commit observed ingest: {e}"))?;
    Ok(ObservedUnreadResponse::Delta {
        scope: request.scope,
        generation,
        base_revision: revision,
        revision: next_revision,
        acked_sequence: request.sequence,
        upserts,
        removed,
    })
}

pub(crate) fn load_membership(
    app: &AppHandle,
    scope: &ObservedUnreadScope,
) -> Result<HashMap<String, HashSet<String>>, String> {
    let conn = open_db(&db_path(app)?)?;
    let key = scope.key();
    let mut stmt = conn
        .prepare("SELECT kind,value FROM unread_membership WHERE scope=?1")
        .map_err(|e| format!("prepare unread membership: {e}"))?;
    let rows = stmt
        .query_map([key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query unread membership: {e}"))?;
    let mut result: HashMap<String, HashSet<String>> = HashMap::new();
    for row in rows {
        let (kind, value) = row.map_err(|e| format!("read unread membership: {e}"))?;
        result.entry(kind).or_default().insert(value);
    }
    Ok(result)
}

pub(crate) fn flush(app: &AppHandle) {
    if let Ok(path) = db_path(app) {
        if let Ok(conn) = open_db(&path) {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> ObservedUnreadScope {
        ObservedUnreadScope {
            pubkey: "PK".into(),
            relay_url: "wss://relay/".into(),
        }
    }
    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_db(&dir.path().join("observed-unread.db")).unwrap();
        (dir, conn)
    }
    #[test]
    fn ingest_replay_gap_prune_and_projection() {
        let (_d, mut conn) = db();
        let tx = conn.transaction().unwrap();
        let key = scope().key();
        ensure_scope(&tx, &key).unwrap();
        upsert_event(
            &tx,
            &key,
            &IngestEvent {
                channel_id: "ch".into(),
                id: "e".into(),
                created_at: chrono::Utc::now().timestamp() as u64,
                root_id: Some("root".into()),
                high_priority: true,
                counts_toward_badge: true,
                counts_toward_app_badge: false,
            },
        )
        .unwrap();
        tx.execute(
            "INSERT INTO read_markers(scope,context_id,read_at) VALUES(?1,'thread:root',0)",
            [&key],
        )
        .unwrap();
        let p = projections(&tx, &key).unwrap();
        assert_eq!(p[0].count, 1);
        assert_eq!(p[0].badge_count, 1);
        tx.commit().unwrap();
    }
    #[test]
    fn latest_anchor_survives_without_a_notify_event_and_seed_is_one_shot() {
        let (_d, mut conn) = db();
        let tx = conn.transaction().unwrap();
        let key = scope().key();
        ensure_scope(&tx, &key).unwrap();
        let first = MembershipSeed {
            participated_root_ids: vec!["kept".into()],
            ..Default::default()
        };
        seed_membership(&tx, &key, &first).unwrap();
        let empty = MembershipSeed::default();
        let (_, _, _, _, seeded) = state(&tx, &key).unwrap();
        if !seeded {
            seed_membership(&tx, &key, &empty).unwrap();
        }
        tx.execute(
            "INSERT INTO channel_latest(scope,channel_id,created_at) VALUES(?1,'ch',42)",
            [&key],
        )
        .unwrap();
        let membership: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM unread_membership WHERE scope=?1 AND value='kept'",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(membership, 1);
        let projected = projections(&tx, &key).unwrap();
        assert_eq!(projected[0].latest, 42);
        assert_eq!(projected[0].count, 0);
    }
    #[test]
    fn ingest_request_wire_accepts_channel_latest() {
        let request: IngestRequest = serde_json::from_value(serde_json::json!({
            "scope":{"pubkey":"PK","relayUrl":"wss://relay/"},
            "sequence":1,"baseRevision":0,"events":[],
            "channelLatest":[{"channelId":"ch","createdAt":42}],
            "markers":[],"membership":[],"clearChannels":[],"clearAll":false
        }))
        .unwrap();
        assert_eq!(request.channel_latest[0].channel_id, "ch");
        assert_eq!(request.channel_latest[0].created_at, 42);
    }
    /// Both commands must stay `async`. A sync `#[tauri::command]` is
    /// `ExecutionContext::Blocking` and runs its body inline in the IPC
    /// handler — the main thread on macOS — which is the defect this fix
    /// closes. The bound is the assertion: dropping `async` makes the return
    /// type `Result`, which is not a `Future`, and this stops compiling.
    ///
    /// The companion half — `async` kept but the body called directly, skipping
    /// `spawn_blocking` — is held by `blocking::OnBlockingThread`, which the
    /// bodies require and only `blocking::run` can mint. This test survived that
    /// mutant while it asserted the helper's own behavior; the token is what
    /// killed it, so the invariant lives in the types, not here.
    const _: () = {
        fn returns_future<A, B, C, F: std::future::Future>(_: fn(A, B, C) -> F) {}
        fn assert() {
            returns_future(
                observed_unread_open_scope
                    as fn(OpenScopeRequest, AppHandle, State<'static, ObservedUnreadStore>) -> _,
            );
            returns_future(
                observed_unread_ingest
                    as fn(IngestRequest, AppHandle, State<'static, ObservedUnreadStore>) -> _,
            );
        }
        let _ = assert;
    };
    /// `blocking::run` must actually leave the caller's thread. This pins the
    /// helper only; that the commands go *through* it is the token's job.
    #[test]
    fn blocking_run_leaves_the_calling_thread() {
        let caller = std::thread::current().id();
        let observed = tauri::async_runtime::block_on(blocking::run(move |_proof| {
            Ok::<_, String>(std::thread::current().id())
        }))
        .unwrap();
        assert_ne!(observed, caller);
    }
    #[test]
    fn second_seed_cannot_erase_discovered_membership() {
        let (_d, mut conn) = db();
        let tx = conn.transaction().unwrap();
        let key = scope().key();
        ensure_scope(&tx, &key).unwrap();
        // First open seeds from the renderer.
        let (_, _, _, _, seeded) = state(&tx, &key).unwrap();
        seed_membership_once(
            &tx,
            &key,
            seeded,
            Some(&MembershipSeed {
                participated_root_ids: vec!["from-seed".into()],
                ..Default::default()
            }),
        )
        .unwrap();
        // Native discovers a root incrementally (the ingest path).
        tx.execute(
            "INSERT INTO unread_membership(scope,kind,value) VALUES(?1,'participated','discovered')",
            [&key],
        )
        .unwrap();
        // Second open with an EMPTY seed must not erase it.
        let (_, _, _, _, seeded) = state(&tx, &key).unwrap();
        seed_membership_once(&tx, &key, seeded, Some(&MembershipSeed::default())).unwrap();
        let kept: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM unread_membership WHERE scope=?1 AND value='discovered'",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "an empty second seed erased discovered membership");
    }

    #[test]
    fn channel_latest_anchor_never_moves_backward() {
        let (_d, mut conn) = db();
        let tx = conn.transaction().unwrap();
        let key = scope().key();
        ensure_scope(&tx, &key).unwrap();
        let advance = |created_at: u64| {
            advance_channel_latest(&tx, &key, "ch", created_at).unwrap();
        };
        advance(500);
        advance(100); // an older catch-up trigger arriving late
        let anchor: u64 = tx
            .query_row(
                "SELECT created_at FROM channel_latest WHERE scope=?1 AND channel_id='ch'",
                [&key],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            anchor, 500,
            "a late older trigger rewound the latest anchor"
        );
    }

    #[test]
    fn serialized_response_matches_typescript_contract() {
        let actual = serde_json::to_value(ObservedUnreadResponse::Delta {
            scope: scope(),
            generation: "gen".into(),
            base_revision: 4,
            revision: 5,
            acked_sequence: 7,
            upserts: vec![ChannelProjection {
                channel_id: "ch".into(),
                latest: 42,
                count: 2,
                badge_count: 1,
                app_badge_count: 1,
                top_level_unread: true,
                high_priority_unread: false,
            }],
            removed: vec!["old".into()],
        })
        .unwrap();
        let expected = serde_json::json!({"kind":"delta","scope":{"pubkey":"PK","relayUrl":"wss://relay/"},"generation":"gen","baseRevision":4,"revision":5,"ackedSequence":7,"upserts":[{"channelId":"ch","latest":42,"count":2,"badgeCount":1,"appBadgeCount":1,"topLevelUnread":true,"highPriorityUnread":false}],"removed":["old"]});
        assert_eq!(actual, expected);
    }
}
