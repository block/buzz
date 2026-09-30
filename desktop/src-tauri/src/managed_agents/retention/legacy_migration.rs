//! One-time migration of the pre-scoping global retention database into the
//! active relay+owner scope.
//!
//! Before community scoping, every durable event lived in one
//! `<managed-agents-base>/retention.db`. Scoped storage
//! ([`super::scoped_retention_db_path`]) reads a different file, so an upgrade
//! would otherwise abandon whatever the previous release left pending —
//! including signed kind:5 tombstones and NIP-IA archive requests queued while
//! offline, which no reconcile can reconstruct (boot reconcile rebuilds upserts
//! from records still on disk, and deletions have no reconcile at all).
//!
//! # Crash safety
//!
//! Two guards, each written transactionally, make the migration exactly-once
//! without a completion file:
//!
//! 1. A **claim** in the legacy database naming the scope that owns its rows.
//!    Legacy rows were queued for whichever single relay the old build had
//!    active, so exactly one scope may take them; every other scope skips. This
//!    is what keeps the migration from fanning one community's pending events
//!    out to all of them — the leak class scoping exists to close.
//! 2. A **marker** in the scoped database, committed in the same transaction as
//!    the copied rows. A crash mid-copy therefore leaves neither rows nor
//!    marker, and the next boot copies from scratch; once the marker is there
//!    the copy never repeats.
//!
//! The relay dimension is not recoverable from the legacy file — only the owner
//! pubkey is — so the claiming scope is the first one this owner activates after
//! upgrading. That is the workspace the app restores at launch, i.e. the same
//! relay the stranded rows were queued for in all but a contrived
//! switch-before-first-flush case.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use super::{open_retention_db, RetainedEvent};

/// Marker/claim identifier for this migration.
const MIGRATION_NAME: &str = "legacy_global_retention_db";

/// The pre-scoping global retention database path.
pub fn legacy_retention_db_path(base_dir: &Path) -> PathBuf {
    base_dir.join("retention.db")
}

/// Copy the legacy global database's rows for `owner_pubkey` into the scoped
/// database at `scope_db_path`.
///
/// Returns the number of rows copied — `0` both when there is nothing to do and
/// when another scope already claimed the legacy rows. Best-effort by design:
/// the caller logs a failure and proceeds, and the guards make a later retry
/// safe.
pub fn migrate_legacy_retention_db(
    base_dir: &Path,
    scope_db_path: &Path,
    owner_pubkey: &str,
) -> Result<usize, String> {
    let legacy_path = legacy_retention_db_path(base_dir);
    if !legacy_path.exists() || legacy_path == scope_db_path {
        return Ok(0);
    }

    let scope_id = scope_identifier(scope_db_path);
    let mut scope_conn = open_retention_db(scope_db_path)?;
    if migration_marker_present(&scope_conn)? {
        return Ok(0);
    }

    let legacy_conn = open_retention_db(&legacy_path)?;
    if !claim_legacy_rows(&legacy_conn, &scope_id)? {
        return Ok(0); // another scope owns these rows
    }

    let rows = legacy_rows_for_owner(&legacy_conn, owner_pubkey)?;
    let copied = rows.len();

    let transaction = scope_conn
        .transaction()
        .map_err(|e| format!("failed to open retention migration transaction: {e}"))?;
    for row in &rows {
        // The scoped database is authoritative for any coordinate it already
        // holds: those rows were written after the upgrade, so they are newer
        // than anything legacy by construction. Legacy rows only fill gaps.
        transaction
            .execute(
                "INSERT INTO persona_events
                    (kind, pubkey, d_tag, content, created_at, raw_event, pending_sync)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (kind, pubkey, d_tag) DO NOTHING",
                params![
                    row.kind,
                    row.pubkey,
                    row.d_tag,
                    row.content,
                    row.created_at,
                    row.raw_event,
                    row.pending_sync as i32,
                ],
            )
            .map_err(|e| format!("failed to copy legacy retained event: {e}"))?;
    }
    write_migration_marker(&transaction, &scope_id)?;
    transaction
        .commit()
        .map_err(|e| format!("failed to commit retention migration: {e}"))?;

    Ok(copied)
}

/// Read every retained row authored by `owner_pubkey` from the legacy database.
///
/// Owner-filtered because the flush loop only publishes rows matching the
/// active owner anyway; a different identity's rows belong to that identity's
/// scope, not this one.
fn legacy_rows_for_owner(
    conn: &Connection,
    owner_pubkey: &str,
) -> Result<Vec<RetainedEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, pubkey, d_tag, content, created_at, raw_event, pending_sync
             FROM persona_events
             WHERE pubkey = ?1
             ORDER BY (kind != 5), created_at ASC",
        )
        .map_err(|e| format!("failed to prepare legacy retention query: {e}"))?;

    let rows = stmt
        .query_map(params![owner_pubkey], |row| {
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
        .map_err(|e| format!("failed to query legacy retained events: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to read legacy retained row: {e}"))
}

/// Identify a scope by its database file stem — the relay+owner hash
/// [`super::scoped_retention_db_path`] already computes.
fn scope_identifier(scope_db_path: &Path) -> String {
    scope_db_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn ensure_migration_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS retention_migrations (
            name TEXT PRIMARY KEY,
            scope_id TEXT NOT NULL
        );",
    )
    .map_err(|e| format!("failed to create retention migration table: {e}"))
}

fn migration_marker_present(conn: &Connection) -> Result<bool, String> {
    ensure_migration_table(conn)?;
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM retention_migrations WHERE name = ?1)",
        params![MIGRATION_NAME],
        |row| row.get(0),
    )
    .map_err(|e| format!("failed to read retention migration marker: {e}"))
}

fn write_migration_marker(conn: &Connection, scope_id: &str) -> Result<(), String> {
    ensure_migration_table(conn)?;
    conn.execute(
        "INSERT OR REPLACE INTO retention_migrations (name, scope_id) VALUES (?1, ?2)",
        params![MIGRATION_NAME, scope_id],
    )
    .map_err(|e| format!("failed to write retention migration marker: {e}"))?;
    Ok(())
}

/// Record `scope_id` as the owner of the legacy rows, or confirm it already is.
///
/// `INSERT OR IGNORE` then read-back is atomic enough for this purpose: the
/// loser of a race reads the winner's scope id and returns `false`.
fn claim_legacy_rows(legacy_conn: &Connection, scope_id: &str) -> Result<bool, String> {
    ensure_migration_table(legacy_conn)?;
    legacy_conn
        .execute(
            "INSERT OR IGNORE INTO retention_migrations (name, scope_id) VALUES (?1, ?2)",
            params![MIGRATION_NAME, scope_id],
        )
        .map_err(|e| format!("failed to claim legacy retention rows: {e}"))?;

    let claimed_by: Option<String> = legacy_conn
        .query_row(
            "SELECT scope_id FROM retention_migrations WHERE name = ?1",
            params![MIGRATION_NAME],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("failed to read legacy retention claim: {e}"))?;

    Ok(claimed_by.as_deref() == Some(scope_id))
}

#[cfg(test)]
mod tests;
