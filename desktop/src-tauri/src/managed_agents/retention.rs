//! Local SQLite retention store for persona events.
//!
//! Provides durable client-side storage for persona events, enabling offline
//! boot when the relay is unreachable. Upserts via `ON CONFLICT DO UPDATE`
//! keyed on `(kind, pubkey, d_tag)`, replacing only on a newer-or-equal
//! `created_at` for NIP-33 latest-wins semantics.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use tauri::AppHandle;

use crate::app_state::AppState;

mod legacy_migration;
pub use legacy_migration::migrate_legacy_retention_db;

/// Durable event-retention scope for one community relay and owner identity.
///
/// Persona, team, and managed-agent definitions are workspace-global, but
/// their relay heads and pending publications are not. Keeping a separate
/// database per `(relay_url, owner_pubkey)` prevents a pending write created in
/// community A from being drained into community B after a workspace switch.
pub struct RetentionScope {
    pub db_path: PathBuf,
    pub relay_url: String,
    pub owner_keys: nostr::Keys,
}

/// Decide whether `scope` — the workspace's active retention scope — is the one
/// that owns an event delivered by `arrival_relay_url`.
///
/// Inbound reconcile resolves its retention database when it PROCESSES an event,
/// while the event belongs to the community that DELIVERED it. `None` means a
/// workspace switch happened in between and the caller must drop the event
/// rather than file community A's event into community B's store.
///
/// The comparison goes through the same normalization
/// [`scoped_retention_db_path`] hashes, so "same relay" can never disagree with
/// "same database".
pub fn scope_for_arrival(scope: RetentionScope, arrival_relay_url: &str) -> Option<RetentionScope> {
    let same_scope =
        normalized_relay_scope(&scope.relay_url) == normalized_relay_scope(arrival_relay_url);
    same_scope.then_some(scope)
}

/// Relay-URL form that identifies a retention scope: equivalent workspace URLs
/// (surrounding space, trailing slash) must resolve to one scope.
fn normalized_relay_scope(relay_url: &str) -> &str {
    relay_url.trim().trim_end_matches('/')
}

/// Resolve the retention database path for a relay + owner pair.
///
/// The normalized scope is hashed so relay URLs never become path components.
/// Trimming a trailing slash keeps equivalent workspace URLs on one scope.
pub fn scoped_retention_db_path(base_dir: &Path, relay_url: &str, owner_pubkey: &str) -> PathBuf {
    let normalized_relay = normalized_relay_scope(relay_url);
    let mut hasher = Sha256::new();
    hasher.update(owner_pubkey.trim().to_ascii_lowercase().as_bytes());
    hasher.update(b"\0");
    hasher.update(normalized_relay.as_bytes());
    let scope_id = hex::encode(hasher.finalize());
    base_dir.join("retention").join(format!("{scope_id}.db"))
}

/// Snapshot the active relay + owner and resolve their durable event store.
///
/// Callers keep the returned relay and keys alongside the path whenever work
/// crosses an `.await`; a later workspace switch cannot retarget that work.
pub fn active_retention_scope<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> Result<RetentionScope, String> {
    let relay_url = crate::relay::relay_ws_url_with_override(state);
    let owner_keys = state.signing_keys()?;
    let base_dir = super::managed_agents_base_dir(app)?;
    let db_path =
        scoped_retention_db_path(&base_dir, &relay_url, &owner_keys.public_key().to_hex());
    let parent = db_path
        .parent()
        .ok_or_else(|| "retention scope path has no parent".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create retention scope directory: {error}"))?;
    Ok(RetentionScope {
        db_path,
        relay_url,
        owner_keys,
    })
}

/// Snapshot the active relay + owner, but only when it is the scope that owns
/// events delivered by `arrival_relay_url`.
///
/// Resolving the scope and matching it in one step is what closes the gap: the
/// returned scope is both the one that will be written to and the one the event
/// arrived on. `Ok(None)` means the arrival community is no longer active and
/// the caller must drop the event — see [`scope_for_arrival`].
pub fn arrival_retention_scope<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
    arrival_relay_url: &str,
) -> Result<Option<RetentionScope>, String> {
    Ok(scope_for_arrival(
        active_retention_scope(app, state)?,
        arrival_relay_url,
    ))
}

/// A retained persona event row.
#[derive(Debug, Clone)]
pub struct RetainedEvent {
    pub kind: u32,
    pub pubkey: String,
    pub d_tag: String,
    pub content: String,
    pub created_at: i64,
    pub raw_event: String,
    pub pending_sync: bool,
}

/// Open (or create) the retention database at the given path.
///
/// Sets WAL journaling and a `busy_timeout` on every connection so the
/// flush-loop connection and command-path connections can write concurrently
/// without spurious `SQLITE_BUSY` errors.
pub fn open_retention_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("failed to open retention db: {e}"))?;

    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(|e| format!("failed to set busy_timeout: {e}"))?;
    set_wal_mode(&conn)?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS persona_events (
            kind INTEGER NOT NULL,
            pubkey TEXT NOT NULL,
            d_tag TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            raw_event TEXT NOT NULL,
            pending_sync INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (kind, pubkey, d_tag)
        );",
    )
    .map_err(|e| format!("failed to create retention table: {e}"))?;

    Ok(conn)
}

fn set_wal_mode(conn: &Connection) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Ok(()) => return Ok(()),
            Err(error) if sqlite_is_busy(&error) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(format!("failed to set WAL mode: {error}")),
        }
    }
}

fn sqlite_is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

/// Build the retention `d_tag` column value for a kind:5 tombstone row.
///
/// Tombstones for all target kinds share `kind = 5` in the retention table, so
/// keying a tombstone by the bare target d-tag would collide across kinds when
/// a persona slug, team id, and agent pubkey happen to coincide — one
/// tombstone row would clobber another's pending publish. Folding the target
/// kind into the key (`"<target_kind>:<d_tag>"`) gives each its own PK row.
/// This is the retention-store key only; the published NIP-09 event still
/// carries the plain `a`-tag coordinate.
pub fn tombstone_retention_d_tag(target_kind: u32, d_tag: &str) -> String {
    format!("{target_kind}:{d_tag}")
}

/// Whether a pending row must be deferred to the next sweep because a kind:5
/// tombstone covering its coordinate failed to publish earlier in the same
/// sweep.
///
/// `get_pending_sync` orders tombstones first so a deletion always reaches the
/// relay before the replacement that supersedes it — but the flush is
/// best-effort per row, so a tombstone that fails mid-sweep would otherwise be
/// leapfrogged by its own replacement and then wipe it on the next sweep.
/// Deferring the replacement restores the ordering guarantee: next sweep the
/// `ORDER BY` puts the tombstone first again. Kind:5 rows are never deferred.
pub fn deferred_behind_failed_tombstone(
    kind: u32,
    pubkey: &str,
    d_tag: &str,
    failed_tombstones: &std::collections::HashSet<(String, String)>,
) -> bool {
    kind != 5
        && failed_tombstones.contains(&(pubkey.to_string(), tombstone_retention_d_tag(kind, d_tag)))
}

/// Upsert a persona event into the retention store.
///
/// Only replaces if the new event has a newer or equal `created_at` (NIP-33 semantics).
pub fn retain_event(conn: &Connection, event: &RetainedEvent) -> Result<(), String> {
    conn.execute(
        "INSERT INTO persona_events (kind, pubkey, d_tag, content, created_at, raw_event, pending_sync)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (kind, pubkey, d_tag) DO UPDATE SET
            content = excluded.content,
            created_at = excluded.created_at,
            raw_event = excluded.raw_event,
            pending_sync = excluded.pending_sync
         WHERE excluded.created_at >= persona_events.created_at",
        params![
            event.kind,
            event.pubkey,
            event.d_tag,
            event.content,
            event.created_at,
            event.raw_event,
            event.pending_sync as i32,
        ],
    )
    .map_err(|e| format!("failed to retain event: {e}"))?;

    Ok(())
}

/// Outcome of an inbound retain — whether the local store now reflects the
/// inbound event, so the caller knows whether to patch `personas.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundOutcome {
    /// The inbound event was applied (no row, or it was strictly newer than a
    /// non-conflicting local row). The caller patches the local record store.
    Applied,
    /// The inbound event was NOT applied: either it is older than the retained
    /// row, or it collides at the same `created_at` with a pending local edit.
    /// The local record store is left untouched and the pending edit republishes.
    Skipped,
}

/// Retain an event arriving FROM the relay, resolving it against any local row.
///
/// Inbound events are already on the relay, so they are retained with
/// `pending_sync = 0`. The resolution is deliberately narrower than
/// [`retain_event`]'s blind newer-or-equal upsert, which would clobber a
/// pending local edit's `pending_sync` flag and silently drop its publish:
///
/// - No local row, or inbound strictly newer (`created_at >`): apply the
///   inbound event, clearing `pending_sync`. Inbound wins; a stale local edit
///   the relay already superseded stops republishing instead of looping.
/// - Equal `created_at`: NIP-01 addressable-event tiebreak — the event with
///   the lexicographically LOWEST id wins, exactly the head the relay itself
///   retains (`buzz-db` rejects an incoming coordinate whose id is `>=` the
///   accepted head's at equal time). Nostr time is seconds-granularity, so two
///   devices can retain distinct successors in the same second; without a
///   shared deterministic winner each side skips the other's head on every
///   replay and the devices diverge permanently. A pending local edit that
///   WINS the tie stays pending and republishes; one that LOSES is superseded —
///   the relay would refuse it as the head anyway, so clearing its
///   `pending_sync` converges both devices onto the relay's answer. (A
///   re-received echo has an equal id and stays a no-op; if either id is
///   unavailable the inbound event is skipped, preserving any pending publish.)
/// - Inbound older: skip — nothing to change.
///
/// Decide whether an inbound event is newer than the retained coordinate without
/// mutating retention. Callers that must update another durable store first use
/// this preflight, apply that store change, and only then commit with
/// [`retain_inbound_event`].
pub fn inbound_event_outcome(
    conn: &Connection,
    event: &RetainedEvent,
) -> Result<InboundOutcome, String> {
    let existing = get_retained_event(conn, event.kind, &event.pubkey, &event.d_tag)?;
    Ok(match existing {
        None => InboundOutcome::Applied,
        Some(row) if event.created_at > row.created_at => InboundOutcome::Applied,
        Some(row)
            if event.created_at == row.created_at
                && equal_second_inbound_wins(&event.raw_event, &row.raw_event) =>
        {
            InboundOutcome::Applied
        }
        // Older, or an equal-second loser/echo: skip. A pending local edit
        // that won (or an undecidable tie) keeps its `pending_sync`.
        Some(_) => InboundOutcome::Skipped,
    })
}

/// NIP-01 addressable-event tiebreak at equal `created_at`: the event with the
/// lexicographically lowest id is the head the relay retains. Returns `true`
/// only when BOTH ids are present and the inbound id is strictly lower — an
/// undecidable or equal comparison must not clobber the retained row (or a
/// pending local publish riding on it).
fn equal_second_inbound_wins(inbound_raw: &str, retained_raw: &str) -> bool {
    match (raw_event_id(inbound_raw), raw_event_id(retained_raw)) {
        (Some(inbound_id), Some(retained_id)) => inbound_id < retained_id,
        _ => false,
    }
}

/// Extract the `id` field from a raw event JSON string, if present.
fn raw_event_id(raw_event: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw_event)
        .ok()?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

/// Apply an inbound event's fallible local-store mutation, then advance the
/// durable retention head — never the other way around.
///
/// The head is the replay witness: `inbound_event_outcome` reports `Skipped`
/// for an event no newer than the retained head (equal `created_at` reads as
/// stale). If the head advanced before the JSON store write and that write then
/// failed, replay of the identical relay event would see the head as already
/// consumed and the projection would be lost forever. Ordering the commit after
/// the store write means a failed `apply_store` leaves the head un-advanced, so
/// the next replay retries and succeeds.
///
/// Returns `Skipped` without running `apply_store` when the event does not win
/// the preflight; the caller leaves its store untouched.
pub fn commit_inbound_with_store<F>(
    conn: &Connection,
    event: &RetainedEvent,
    apply_store: F,
) -> Result<InboundOutcome, String>
where
    F: FnOnce() -> Result<(), String>,
{
    if inbound_event_outcome(conn, event)? == InboundOutcome::Skipped {
        return Ok(InboundOutcome::Skipped);
    }
    apply_store()?;
    retain_inbound_event(conn, event)
}

/// Resolve and commit an inbound NIP-09 tombstone against BOTH its own kind:5
/// retention row AND the covered target head, matching the relay's
/// coordinate-deletion contract (a deletion removes only target rows with
/// `created_at <= tombstone.created_at`, `buzz-db`).
///
/// Order, so a crash or store failure never loses the recovery source:
/// 1. Covered head strictly NEWER than the tombstone → `Skipped`: a historical
///    delete replayed after a newer recreation; the relay keeps the head, so we
///    must preserve the local record.
/// 2. Tombstone-row preflight loses (re-received / superseded) → `Skipped`.
/// 3. Run the fallible `remove_json` FIRST. On failure nothing durable advances,
///    so replay of the identical tombstone retries.
/// 4. Commit the tombstone row and purge the covered head in ONE transaction. A
///    kill between them would otherwise advance the tombstone row (making replay
///    read as already-consumed) while leaving the covered head in retention with
///    no witness to remove it.
pub fn commit_inbound_tombstone_with_store<F>(
    conn: &Connection,
    tombstone: &RetainedEvent,
    target_kind: u32,
    target_owner: &str,
    target_d_tag: &str,
    remove_json: F,
) -> Result<InboundOutcome, String>
where
    F: FnOnce() -> Result<(), String>,
{
    let covered_head = get_retained_event(conn, target_kind, target_owner, target_d_tag)?;
    if covered_head
        .as_ref()
        .is_some_and(|head| head.created_at > tombstone.created_at)
    {
        return Ok(InboundOutcome::Skipped);
    }
    if inbound_event_outcome(conn, tombstone)? == InboundOutcome::Skipped {
        return Ok(InboundOutcome::Skipped);
    }
    remove_json()?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("failed to begin inbound tombstone transaction: {e}"))?;
    let result = (|| -> Result<(), String> {
        retain_inbound_event(conn, tombstone)?;
        delete_retained_event(conn, target_kind, target_owner, target_d_tag)
    })();
    match result {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .map_err(|e| format!("failed to commit inbound tombstone transaction: {e}"))?,
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(e);
        }
    }
    Ok(InboundOutcome::Applied)
}

pub fn retain_inbound_event(
    conn: &Connection,
    event: &RetainedEvent,
) -> Result<InboundOutcome, String> {
    let outcome = inbound_event_outcome(conn, event)?;

    if outcome == InboundOutcome::Skipped {
        return Ok(InboundOutcome::Skipped);
    }

    // Inbound is strictly newer (or there was no row): overwrite and clear
    // `pending_sync`. No upsert guard is needed — the Rust check above already
    // established that this event wins.
    conn.execute(
        "INSERT INTO persona_events (kind, pubkey, d_tag, content, created_at, raw_event, pending_sync)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)
         ON CONFLICT (kind, pubkey, d_tag) DO UPDATE SET
            content = excluded.content,
            created_at = excluded.created_at,
            raw_event = excluded.raw_event,
            pending_sync = 0",
        params![
            event.kind,
            event.pubkey,
            event.d_tag,
            event.content,
            event.created_at,
            event.raw_event,
        ],
    )
    .map_err(|e| format!("failed to retain inbound event: {e}"))?;

    Ok(InboundOutcome::Applied)
}

/// Load all retained persona events for a given pubkey.
#[cfg(test)]
pub fn get_retained_personas(
    conn: &Connection,
    pubkey: &str,
) -> Result<Vec<RetainedEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
             FROM persona_events
             WHERE pubkey = ?1
             ORDER BY d_tag",
        )
        .map_err(|e| format!("failed to prepare query: {e}"))?;

    let rows = stmt
        .query_map(params![pubkey], |row| {
            Ok(RetainedEvent {
                kind: row.get(0)?,
                pubkey: row.get(1)?,
                d_tag: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                raw_event: row.get(5)?,
                pending_sync: row.get::<_, i32>(6)? != 0,
            })
        })
        .map_err(|e| format!("failed to query retained events: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read retained event row: {e}"))
}

/// Get all events marked as pending sync (not yet confirmed on relay).
///
/// Tombstones (kind:5) sort FIRST: a delete retained in one session and its
/// coordinate's replacement retained in a later one (B5 backfill resurrecting
/// a deleted definition) must publish in that order — the relay's a-tag
/// deletion soft-deletes every live row at the coordinate with no timestamp
/// comparison, so a tombstone published AFTER the replacement would wipe it.
/// Within each group, oldest first for the same reason.
pub fn get_pending_sync(conn: &Connection) -> Result<Vec<RetainedEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
             FROM persona_events
             WHERE pending_sync = 1
             ORDER BY (kind != 5), created_at ASC",
        )
        .map_err(|e| format!("failed to prepare pending sync query: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(RetainedEvent {
                kind: row.get(0)?,
                pubkey: row.get(1)?,
                d_tag: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                raw_event: row.get(5)?,
                pending_sync: row.get::<_, i32>(6)? != 0,
            })
        })
        .map_err(|e| format!("failed to query pending sync events: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read pending sync row: {e}"))
}

/// Clear the `pending_sync` flag for an event the relay just confirmed.
///
/// Compare-and-clear: only clears the row if its `created_at` and `content`
/// still match what was published. A concurrent edit that upserted a newer
/// version at the same coordinate between the flush loop's read and this call
/// leaves `pending_sync` set, so the newer edit publishes on the next pass
/// instead of being silently dropped.
pub fn mark_synced(
    conn: &Connection,
    kind: u32,
    pubkey: &str,
    d_tag: &str,
    created_at: i64,
    content: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE persona_events SET pending_sync = 0
         WHERE kind = ?1 AND pubkey = ?2 AND d_tag = ?3
           AND created_at = ?4 AND content = ?5",
        params![kind, pubkey, d_tag, created_at, content],
    )
    .map_err(|e| format!("failed to mark event synced: {e}"))?;

    Ok(())
}

/// Delete a retained event by its coordinate.
///
/// Called from the synchronous, lock-held delete-persona command body so the
/// purge serializes against `retain_event` upserts at the same coordinate —
/// closing the same-second resurrect race where a pending edit would otherwise
/// publish after the deletion tombstone.
pub fn delete_retained_event(
    conn: &Connection,
    kind: u32,
    pubkey: &str,
    d_tag: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM persona_events
         WHERE kind = ?1 AND pubkey = ?2 AND d_tag = ?3",
        params![kind, pubkey, d_tag],
    )
    .map_err(|e| format!("failed to delete retained event: {e}"))?;

    Ok(())
}

/// Check if the retention store has any persona events for the given pubkey.
#[cfg(test)]
pub fn has_retained_personas(conn: &Connection, pubkey: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM persona_events WHERE pubkey = ?1)",
        params![pubkey],
        |row| row.get(0),
    )
    .map_err(|e| format!("failed to check retained personas: {e}"))
}

/// Look up a single retained event by its coordinate.
pub fn get_retained_event(
    conn: &Connection,
    kind: u32,
    pubkey: &str,
    d_tag: &str,
) -> Result<Option<RetainedEvent>, String> {
    conn.query_row(
        "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
         FROM persona_events
         WHERE kind = ?1 AND pubkey = ?2 AND d_tag = ?3",
        params![kind, pubkey, d_tag],
        |row| {
            Ok(RetainedEvent {
                kind: row.get(0)?,
                pubkey: row.get(1)?,
                d_tag: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                raw_event: row.get(5)?,
                pending_sync: row.get::<_, i32>(6)? != 0,
            })
        },
    )
    .optional()
    .map_err(|e| format!("failed to get retained event: {e}"))
}

/// Return every retained event for `pubkey` at the given kind.
///
/// Used by the team-catalog reconcile, which enumerates retained 30178 heads
/// as the authoritative worklist — not the current team store — so a shared
/// head whose team was later deleted stays visible and can be tombstoned.
pub fn get_retained_events_by_kind(
    conn: &Connection,
    kind: u32,
    pubkey: &str,
) -> Result<Vec<RetainedEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
             FROM persona_events
             WHERE kind = ?1 AND pubkey = ?2
             ORDER BY d_tag",
        )
        .map_err(|e| format!("failed to prepare query: {e}"))?;

    let rows = stmt
        .query_map(params![kind, pubkey], |row| {
            Ok(RetainedEvent {
                kind: row.get(0)?,
                pubkey: row.get(1)?,
                d_tag: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
                raw_event: row.get(5)?,
                pending_sync: row.get::<_, i32>(6)? != 0,
            })
        })
        .map_err(|e| format!("failed to query retained events: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read retained event row: {e}"))
}

#[cfg(test)]
mod tests;
