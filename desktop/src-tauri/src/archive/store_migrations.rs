//! One-shot schema migrations for the archive database.
//!
//! Applied by `store::open_archive_db` on every open and guarded by markers in
//! `archive_migrations`, so a migration that already ran is a no-op.
//!
//! Kept in a sibling file (not `store.rs`) to keep that file under the
//! 1500-line gate, per the existing `metric_store.rs` / `pipeline.rs`
//! precedent.

use rusqlite::{params, Connection};

/// One-shot, idempotent schema migrations recorded in `archive_migrations`.
///
/// Each migration is guarded by a `SELECT 1 FROM archive_migrations WHERE
/// name = '...'` check so it is safe to call on every open — a migration
/// that already ran is a no-op.
///
/// Ordering: M2 (column additions) runs before M1 (index rebuild) so that
/// the M1 rebuild, which calls `insert_metric_index_row`, always operates
/// against a schema that includes the cache-read columns. M4 (`archive_meta`
/// + scope-age index) runs last; it is independent of M1–M3.
pub(super) fn apply_schema_migrations(conn: &Connection) -> Result<(), String> {
    migrate_add_cache_read_tokens(conn)?;
    migrate_add_cache_write_and_pricing(conn)?;
    migrate_add_harness_to_metric_index(conn)?;
    migrate_add_archive_meta(conn)
}

/// M1: add `harness TEXT` column to `agent_metric_index` and rebuild index
/// rows so pre-existing rows gain harness populated from `archived_events`.
///
/// The entire migration — column addition, full DELETE of stale index rows,
/// fresh re-insertion from `archived_events.raw_json`, and the
/// `archive_migrations` marker — is wrapped in a single `unchecked_transaction`
/// (BEGIN DEFERRED). A crash anywhere before COMMIT leaves the DB in its
/// pre-migration state and the marker absent, so the next open re-runs the
/// migration from scratch.
///
/// The worklist is built from `archived_events` (the canonical store) rather
/// than from `agent_metric_index` so that a partially-rebuilt or entirely
/// empty index never causes us to forget which (identity, relay) scopes exist.
fn migrate_add_harness_to_metric_index(conn: &Connection) -> Result<(), String> {
    let already_run: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_harness_to_metric_index'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| format!("migration M1: guard check: {e}"))?
        > 0;

    if already_run {
        return Ok(());
    }

    // All steps run inside one transaction so a crash before COMMIT leaves the
    // DB fully pre-migration and the marker absent — the next open re-runs.
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("migration M1: begin transaction: {e}"))?;

    // 1. Add the `harness` column only when it is genuinely absent, so we
    //    propagate unexpected DDL errors instead of swallowing them.
    let harness_exists: bool = {
        let mut stmt = tx
            .prepare("PRAGMA table_info(agent_metric_index)")
            .map_err(|e| format!("migration M1: PRAGMA table_info prepare: {e}"))?;
        let names: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("migration M1: PRAGMA table_info query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("migration M1: PRAGMA table_info read: {e}"))?;
        names.iter().any(|n| n == "harness")
    };
    if !harness_exists {
        tx.execute_batch("ALTER TABLE agent_metric_index ADD COLUMN harness TEXT")
            .map_err(|e| format!("migration M1: ALTER TABLE failed: {e}"))?;
    }

    // 2. Collect all (identity, relay) scopes from `archived_events` — the
    //    canonical store — so the worklist is correct even if the index was
    //    partially rebuilt or empty before this migration runs.
    let scopes: Vec<(String, String)> = {
        let mut stmt = tx
            .prepare(
                "SELECT DISTINCT identity_pubkey, relay_url
                 FROM archived_events
                 WHERE kind = 44200",
            )
            .map_err(|e| format!("migration M1: prepare scope query: {e}"))?;
        let result = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("migration M1: query scopes: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("migration M1: read scopes: {e}"))?;
        result
    };

    if !scopes.is_empty() {
        // 3. Delete all stale index rows; re-insert them with harness populated
        //    from the shared `from_payload` parser below.  Both the DELETE and
        //    all inserts run inside the same outer transaction — no intermediate
        //    commit, so readers never observe an empty index.
        tx.execute_batch("DELETE FROM agent_metric_index")
            .map_err(|e| format!("migration M1: delete index rows: {e}"))?;

        for (identity, relay) in &scopes {
            rebuild_metric_index_in_tx(&tx, identity, relay)?;
        }
    }

    // 4. Record migration as applied — written last so the marker is only
    //    present in a fully committed transaction.
    tx.execute(
        "INSERT OR IGNORE INTO archive_migrations (name, applied_at) \
         VALUES ('add_harness_to_metric_index', ?1)",
        params![std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64],
    )
    .map_err(|e| format!("migration M1: record marker: {e}"))?;

    tx.commit()
        .map_err(|e| format!("migration M1: commit: {e}"))?;

    Ok(())
}

/// Re-insert all kind-44200 rows for `(identity, relay)` from
/// `archived_events.raw_json` through the shared `from_payload` parser.
///
/// Unlike `metric_store::backfill_agent_metric_index`, this function runs
/// entirely inside the caller's transaction — no nested `BEGIN`/`COMMIT`.
/// It is used exclusively by M1 where the outer transaction provides atomicity.
fn rebuild_metric_index_in_tx(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, pubkey, created_at, archived_at, raw_json
             FROM archived_events
             WHERE identity_pubkey = ?1
               AND relay_url       = ?2
               AND kind            = 44200",
        )
        .map_err(|e| format!("migration M1: prepare rebuild select: {e}"))?;

    let rows: Vec<(String, String, i64, i64, String)> = stmt
        .query_map(params![identity_pubkey, relay_url], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| format!("migration M1: query rebuild rows: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("migration M1: read rebuild row: {e}"))?;
    drop(stmt);

    for (id, pubkey, created_at, archived_at, raw_json) in &rows {
        let parsed = super::metric_store::AgentMetricIndexRow::from_payload(
            raw_json,
            id,
            pubkey,
            *created_at,
            *archived_at,
        );
        super::metric_store::insert_metric_index_row(conn, identity_pubkey, relay_url, &parsed)
            .map_err(|e| format!("migration M1: insert rebuilt row: {e}"))?;
    }

    Ok(())
}

/// M2: add `turn_cache_read_tokens TEXT` and `cumulative_cache_read_tokens TEXT`
/// columns to `agent_metric_index`.
///
/// These columns persist the optional NIP-AM `cacheReadTokens` fields (turn +
/// cumulative). They are informational subsets of `inputTokens`, not additions
/// to it. All pre-migration rows receive `NULL` — fail-closed, never estimated.
///
/// Each column is added independently so a crash between the two ALTERs leaves
/// the DB in a self-repairable partial state: the marker is only committed after
/// BOTH columns are present.
fn migrate_add_cache_read_tokens(conn: &Connection) -> Result<(), String> {
    let already_run: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_read_tokens'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| format!("migration M2: guard check: {e}"))?
        > 0;

    if already_run {
        return Ok(());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("migration M2: begin transaction: {e}"))?;

    // Query existing columns once; add each column only if absent (idempotent:
    // a crash between the two ALTERs leaves a partial schema that self-repairs
    // on the next open — exactly the M3 pattern).
    let names: Vec<String> = {
        let mut stmt = tx
            .prepare("PRAGMA table_info(agent_metric_index)")
            .map_err(|e| format!("migration M2: PRAGMA table_info prepare: {e}"))?;
        let ns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("migration M2: PRAGMA table_info query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("migration M2: PRAGMA table_info read: {e}"))?;
        ns
    };

    let cols = [
        ("turn_cache_read_tokens", "TEXT"),
        ("cumulative_cache_read_tokens", "TEXT"),
    ];
    for (col, ty) in &cols {
        if !names.iter().any(|n| n == col) {
            tx.execute_batch(&format!(
                "ALTER TABLE agent_metric_index ADD COLUMN {col} {ty}"
            ))
            .map_err(|e| format!("migration M2: ALTER TABLE ({col}) failed: {e}"))?;
        }
    }

    tx.execute(
        "INSERT OR IGNORE INTO archive_migrations (name, applied_at) \
         VALUES ('add_cache_read_tokens', ?1)",
        params![std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64],
    )
    .map_err(|e| format!("migration M2: record marker: {e}"))?;

    tx.commit()
        .map_err(|e| format!("migration M2: commit: {e}"))?;

    Ok(())
}

/// M3: add `turn_cache_write_tokens TEXT`, `cumulative_cache_write_tokens TEXT`,
/// `pricing_authority TEXT`, `pricing_model TEXT`, and `pricing_cache_class TEXT`
/// to `agent_metric_index`.
///
/// Cache-write columns store per-session-turn and session-cumulative write tokens
/// (same encode/decode contract as the existing cache-read columns). The pricing
/// columns store the three components of `pricingIdentity` from NIP-AM § billing
/// identity, denormalized for index queries.
///
/// New columns default to NULL for all pre-migration rows (they had no cache-write
/// reporting and no publisher-side identity proof). No rows are deleted: the columns
/// are additive and all pre-existing rows remain valid as "field not reported".
fn migrate_add_cache_write_and_pricing(conn: &Connection) -> Result<(), String> {
    // Guard: skip if already applied.
    let already_applied: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_cache_write_and_pricing'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("migration M3: guard check: {e}"))?;
    if already_applied > 0 {
        return Ok(());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("migration M3: begin transaction: {e}"))?;

    // Check which columns already exist (idempotent: ALTER TABLE is skipped for
    // each column that already exists, e.g. from a partial previous run that
    // committed the column but not the migration marker).
    let names: Vec<String> = {
        let mut stmt = tx
            .prepare("PRAGMA table_info(agent_metric_index)")
            .map_err(|e| format!("migration M3: PRAGMA table_info prepare: {e}"))?;
        let ns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("migration M3: PRAGMA table_info query: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("migration M3: PRAGMA table_info read: {e}"))?;
        ns
    };

    let cols = [
        ("turn_cache_write_tokens", "TEXT"),
        ("cumulative_cache_write_tokens", "TEXT"),
        ("pricing_authority", "TEXT"),
        ("pricing_model", "TEXT"),
        ("pricing_cache_class", "TEXT"),
    ];
    for (col, ty) in &cols {
        if !names.iter().any(|n| n == col) {
            tx.execute_batch(&format!(
                "ALTER TABLE agent_metric_index ADD COLUMN {col} {ty}"
            ))
            .map_err(|e| format!("migration M3: ALTER TABLE ({col}) failed: {e}"))?;
        }
    }

    tx.execute(
        "INSERT OR IGNORE INTO archive_migrations (name, applied_at) \
         VALUES ('add_cache_write_and_pricing', ?1)",
        rusqlite::params![std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64],
    )
    .map_err(|e| format!("migration M3: record marker: {e}"))?;

    tx.commit()
        .map_err(|e| format!("migration M3: commit: {e}"))?;

    Ok(())
}

/// M4: create the retention config storage — `archive_meta` (k/v state holding
/// the observer-frame retention window and the Phase-2 prune timestamp) and the
/// `archived_at`-covering scope-age index — then seed the default observer
/// retention window (`observer_retention_days = 30`).
///
/// Unlike M1–M3, M4 CANNOT use the "check marker → `BEGIN DEFERRED`" pattern.
/// On a fresh DB two connections can open concurrently (the observer- and
/// metric-archive seed hooks each `open_archive_db`), both read no marker, and
/// a `DEFERRED` transaction lets both proceed on the same snapshot — the loser
/// hits `SQLITE_BUSY_SNAPSHOT` or double-seeds. Instead M4 takes the write lock
/// up front with `BEGIN IMMEDIATE` and **rechecks the marker inside the lock**:
/// the race loser blocks on `busy_timeout`, then observes the winner's
/// committed marker and no-ops. The cheap pre-lock guard keeps steady-state
/// opens off the write lock entirely (M4 only takes it until the marker lands).
///
/// The table uses plain `CREATE TABLE` behind a fail-closed guard (a
/// pre-existing `archive_meta` with no marker is an externally-created object
/// M4 refuses to certify) and the index is dropped and recreated
/// unconditionally; the seed is `ON CONFLICT DO NOTHING`. The marker is written
/// last inside the same transaction, so a crash before COMMIT rolls back every
/// object and the next open re-runs from scratch.
fn migrate_add_archive_meta(conn: &Connection) -> Result<(), String> {
    // Cheap pre-lock guard: steady-state opens (marker already present) never
    // take the write lock. The marker is written last in M4's transaction, so
    // its presence implies the full schema + seed committed.
    if archive_meta_migration_applied(conn)? {
        return Ok(());
    }

    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("migration M4: begin immediate: {e}"))?;

    let result = migrate_add_archive_meta_locked(conn);

    if result.is_ok() {
        conn.execute_batch("COMMIT")
            .map_err(|e| format!("migration M4: commit: {e}"))?;
    } else {
        // Best-effort rollback; surface the original error to the caller.
        let _ = conn.execute_batch("ROLLBACK");
    }
    result
}

/// M4 body, run under the `BEGIN IMMEDIATE` write lock held by
/// `migrate_add_archive_meta`.
fn migrate_add_archive_meta_locked(conn: &Connection) -> Result<(), String> {
    // In-lock recheck: a concurrent first-opener may have committed the marker
    // while we were blocked on the write lock. If so, this connection has
    // nothing to do — the winner already created the schema and seeded.
    if archive_meta_migration_applied(conn)? {
        return Ok(());
    }

    // Fail closed on an externally-created table. M4's whole body runs inside
    // one `BEGIN IMMEDIATE` transaction with the marker written last, and
    // SQLite DDL is transactional — a crash anywhere rolls the whole thing
    // back. So no shipped code path can leave the table present without the
    // marker; the only way to reach here with it already existing is a
    // hand-edited DB or a foreign tool. Rather than certify a table we did not
    // create, refuse: roll back with no marker and let a corrected DB re-run.
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'archive_meta'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map_err(|e| format!("migration M4: probe archive_meta: {e}"))?
        > 0;
    if exists {
        return Err(
            "migration M4: archive_meta already exists without the M4 marker — \
             refusing to certify an externally-created table"
                .to_string(),
        );
    }

    conn.execute_batch(super::retention::ARCHIVE_META_SCHEMA)
        .map_err(|e| format!("migration M4: create archive_meta: {e}"))?;

    // Unconditionally rebuild the scope-age index. It carries no data, so a
    // fresh `CREATE` is always correct; dropping any index that happens to
    // share the name costs one rebuild and needs no shape inspection.
    conn.execute_batch(&format!(
        "DROP INDEX IF EXISTS {}",
        super::retention::SCOPE_AGE_INDEX_NAME
    ))
    .map_err(|e| format!("migration M4: drop any pre-existing scope-age index: {e}"))?;
    conn.execute_batch(super::retention::SCOPE_AGE_INDEX_DDL)
        .map_err(|e| format!("migration M4: create scope-age index: {e}"))?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Seed the default observer-frame retention window. `ON CONFLICT DO NOTHING`
    // in the underlying insert makes this a no-op if a value is somehow already
    // present — an existing choice always wins.
    conn.execute(
        "INSERT INTO archive_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO NOTHING",
        params![
            super::retention::OBSERVER_RETENTION_DAYS_KEY,
            super::retention::DEFAULT_OBSERVER_RETENTION_DAYS.to_string()
        ],
    )
    .map_err(|e| format!("migration M4: seed observer retention days: {e}"))?;

    conn.execute(
        "INSERT OR IGNORE INTO archive_migrations (name, applied_at) \
         VALUES ('add_archive_meta', ?1)",
        params![now],
    )
    .map_err(|e| format!("migration M4: record marker: {e}"))?;

    Ok(())
}

/// Whether the M4 marker is present. Its presence implies the retention schema
/// and default seed committed (the marker is written last in M4's transaction).
fn archive_meta_migration_applied(conn: &Connection) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM archive_migrations WHERE name = 'add_archive_meta'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("migration M4: guard check: {e}"))?;
    Ok(count > 0)
}
