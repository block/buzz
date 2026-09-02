//! Local archive retention configuration + size accounting.
//!
//! v4 (Will's ruling, 2026-08-19): retention is a single global setting — how
//! many days observer frames (kind 24200) are kept locally. NIP-AM metrics
//! (44200) and every other archived kind are kept indefinitely with no
//! retention machinery. The setting lives as one row in the `archive_meta` k/v
//! table (`observer_retention_days`), seeded by migration M4.
//!
//! This module owns the `archive_meta` schema, the scope-age index used by the
//! Phase-2 prune scan, the get/set accessors for the observer window, and the
//! PRAGMA-based size readout. The prune worker itself lands in Phase 2.
//!
//! Kept in a sibling file (not `store.rs`) to respect the 1500-line gate, per
//! the existing `metric_store.rs` / `pipeline.rs` / `store_migrations.rs`
//! precedent.

use rusqlite::{params, Connection, OptionalExtension};

// ── Constants ────────────────────────────────────────────────────────────────

/// `archive_meta` key holding the observer-frame retention window, in days,
/// stored as its decimal text.
pub const OBSERVER_RETENTION_DAYS_KEY: &str = "observer_retention_days";

/// Default rolling window for observer frames (Will's ruling: "~14–30"; 30 is
/// the shipped default, trivially changeable). Seeded into `archive_meta` by M4.
pub const DEFAULT_OBSERVER_RETENTION_DAYS: i64 = 30;

/// Upper bound on the retention window (~100 years). Guards against a day count
/// large enough to overflow `archived_at` cutoff arithmetic while still
/// admitting any realistic user choice.
pub const MAX_RETENTION_DAYS: i64 = 36_500;

// ── Schema (created by migration M4, not the base SCHEMA) ─────────────────────

/// `archive_meta`. Created inside M4 under `BEGIN IMMEDIATE` (see
/// `store_migrations::migrate_add_archive_meta`). Plain `CREATE TABLE`
/// (no `IF NOT EXISTS`): M4 creates it once on a DB that provably lacks it (the
/// fail-closed guard rejects a pre-existing one), so the clause would be dead
/// weight. Holds the observer-retention window and the Phase-2 prune timestamp.
pub(super) const ARCHIVE_META_SCHEMA: &str = "
CREATE TABLE archive_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Covering index for the Phase-2 retention prune candidate scan. V4 prunes
/// one global observer window, so the scan filters `(identity_pubkey,
/// relay_url, archived_at < cutoff)` and joins `archived_events` for
/// `kind = 24200` — it does NOT constrain `scope_type`/`scope_value`. Age
/// therefore has to lead the non-equality keys: putting `archived_at`
/// immediately after the two identity/relay equality keys lets SQLite
/// range-seek `archived_at < ?` directly (`SEARCH ... USING COVERING INDEX
/// ... (identity_pubkey=? AND relay_url=? AND archived_at<?)`). The v3 order
/// buried `archived_at` behind the two now-unconstrained scope keys, so the
/// planner fell back to a bare identity/relay seek plus a temp b-tree.
///
/// The trailing `id, scope_type, scope_value` complete the scope-row primary
/// key, so the index both covers the scan (no table lookup) and hands Phase 2
/// the exact keys its bounded `DELETE` needs to materialize candidates.
///
/// Built in M4 so the one-time cost over the existing 1.3M-row archive is paid
/// behind the init barrier rather than on the prune hot path. M4
/// `DROP INDEX IF EXISTS`es under this name and recreates it unconditionally,
/// so a fresh build is always correct — plain `CREATE INDEX`, no `IF NOT
/// EXISTS`.
pub(super) const SCOPE_AGE_INDEX_DDL: &str = "
CREATE INDEX idx_archived_event_scopes_age
    ON archived_event_scopes
       (identity_pubkey, relay_url, archived_at, id, scope_type, scope_value);
";

/// Name of the scope-age index, shared by the DDL and M4's unconditional
/// drop-and-rebuild.
pub(super) const SCOPE_AGE_INDEX_NAME: &str = "idx_archived_event_scopes_age";

// ── Types ─────────────────────────────────────────────────────────────────────

/// Physical and logical size accounting for the archive DB file, returned by
/// `archive_size_stats`. All figures come from cheap PRAGMAs and file metadata
/// — no payload scans.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveSizeStats {
    /// On-disk size of the main DB file, in bytes (from filesystem metadata).
    pub main_file_bytes: i64,
    /// On-disk size of the `-wal` sidecar file, in bytes; `0` when absent.
    pub wal_file_bytes: i64,
    /// SQLite page size, in bytes (`PRAGMA page_size`).
    pub page_size: i64,
    /// Total pages in the main DB (`PRAGMA page_count`).
    pub page_count: i64,
    /// Free (reclaimable) pages on the freelist (`PRAGMA freelist_count`).
    pub freelist_count: i64,
}

// ── Validation ─────────────────────────────────────────────────────────────────

/// Fail-closed validation for the observer retention window: a day count in
/// `1..=MAX_RETENTION_DAYS`. Zero, negatives, and out-of-range values are
/// rejected so the setting can never encode a non-expiring "0 days" or an
/// overflowing cutoff.
pub fn validate_days(days: i64) -> Result<(), String> {
    if !(1..=MAX_RETENTION_DAYS).contains(&days) {
        return Err(format!(
            "retention days must be between 1 and {MAX_RETENTION_DAYS}; got {days}"
        ));
    }
    Ok(())
}

// ── Observer retention window (archive_meta accessor) ───────────────────────────

/// Read the observer-frame retention window (days). Returns the seeded default
/// when the row is somehow absent (older DB opened before M4 seeded it, or an
/// externally cleared row) so the setting always resolves to a bounded window
/// rather than silently becoming Forever.
pub fn get_observer_retention_days(conn: &Connection) -> Result<i64, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM archive_meta WHERE key = ?1",
            params![OBSERVER_RETENTION_DAYS_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("read observer retention days: {e}"))?;

    match raw {
        Some(s) => s
            .parse::<i64>()
            .map_err(|e| format!("observer retention days not an integer ({s:?}): {e}")),
        None => Ok(DEFAULT_OBSERVER_RETENTION_DAYS),
    }
}

/// Set the observer-frame retention window (days). Fail-closed: rejects an
/// out-of-range value before writing (see [`validate_days`]). A single
/// idempotent upsert, atomic under autocommit.
pub fn set_observer_retention_days(conn: &Connection, days: i64) -> Result<(), String> {
    validate_days(days)?;
    conn.execute(
        "INSERT INTO archive_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![OBSERVER_RETENTION_DAYS_KEY, days.to_string()],
    )
    .map_err(|e| format!("set observer retention days: {e}"))?;
    Ok(())
}

// ── Size accounting ─────────────────────────────────────────────────────────────

/// Collect physical (file) and logical (page) size figures for the archive DB.
/// PRAGMAs only — no row or payload scans. The main DB file path is read from
/// the connection itself (`PRAGMA database_list`), and the `-wal` sidecar is
/// measured by appending `-wal` to it.
pub fn size_stats(conn: &Connection) -> Result<ArchiveSizeStats, String> {
    let page_size: i64 = conn
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .map_err(|e| format!("read page_size: {e}"))?;
    let page_count: i64 = conn
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .map_err(|e| format!("read page_count: {e}"))?;
    let freelist_count: i64 = conn
        .pragma_query_value(None, "freelist_count", |row| row.get(0))
        .map_err(|e| format!("read freelist_count: {e}"))?;

    // `PRAGMA database_list` yields (seq, name, file) rows; the `main` schema's
    // `file` is the on-disk DB path (empty for a `:memory:` DB). File sizes are
    // read from filesystem metadata rather than page arithmetic so WAL frames
    // not yet checkpointed into the main file are accounted for separately.
    let main_path: Option<String> = conn
        .query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("read database_list: {e}"))?
        .filter(|p| !p.is_empty());

    let (main_file_bytes, wal_file_bytes) = match main_path {
        Some(p) => {
            let main = std::path::PathBuf::from(&p);
            (file_len(&main), file_len(&wal_path(&main)))
        }
        None => (0, 0),
    };

    Ok(ArchiveSizeStats {
        main_file_bytes,
        wal_file_bytes,
        page_size,
        page_count,
        freelist_count,
    })
}

/// Size of a file in bytes, or `0` when it does not exist / cannot be stat'd.
/// A missing `-wal` (fully checkpointed DB) is the normal case, not an error.
fn file_len(path: &std::path::Path) -> i64 {
    std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0)
}

/// The `-wal` sidecar path for a main DB file (`archive.db` → `archive.db-wal`).
fn wal_path(db_path: &std::path::Path) -> std::path::PathBuf {
    let mut name = db_path.as_os_str().to_os_string();
    name.push("-wal");
    std::path::PathBuf::from(name)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "retention_tests.rs"]
mod retention_tests;
