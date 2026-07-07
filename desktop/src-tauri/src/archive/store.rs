//! Local SQLite archive store for saved relay messages.
//!
//! Three tables:
//! - `archived_events`       — one raw event row per (identity, relay, event_id)
//! - `archived_event_scopes` — N scope membership rows per raw event (many-to-many)
//! - `save_subscriptions`    — which scopes the user has subscribed to save
//!
//! WAL + `busy_timeout=5000` matches `managed_agents/retention.rs`.
//! Raw event rows are GC'd when their last scope row is deleted.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

// ── Schema ─────────────────────────────────────────────────────────────────

pub(super) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS archived_events (
    identity_pubkey TEXT NOT NULL,
    relay_url       TEXT NOT NULL,
    id              TEXT NOT NULL,
    kind            INTEGER NOT NULL,
    pubkey          TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    raw_json        TEXT NOT NULL,
    archived_at     INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, relay_url, id)
);

CREATE TABLE IF NOT EXISTS archived_event_scopes (
    identity_pubkey TEXT NOT NULL,
    relay_url       TEXT NOT NULL,
    id              TEXT NOT NULL,
    scope_type      TEXT NOT NULL,
    scope_value     TEXT NOT NULL,
    archived_at     INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, relay_url, id, scope_type, scope_value)
);

CREATE TABLE IF NOT EXISTS save_subscriptions (
    identity_pubkey TEXT NOT NULL,
    relay_url       TEXT NOT NULL,
    scope_type      TEXT NOT NULL,
    scope_value     TEXT NOT NULL,
    kinds           TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    PRIMARY KEY (identity_pubkey, relay_url, scope_type, scope_value)
);
";

// ── Open / init ─────────────────────────────────────────────────────────────

/// Open (or create) the archive database at the given path.
///
/// Applies WAL journaling and `busy_timeout=5000` on every connection,
/// matching `managed_agents/retention.rs`. Creates all three tables if they
/// don't already exist.
pub fn open_archive_db(path: &Path) -> Result<Connection, String> {
    // Ensure the parent directory exists so `Connection::open` doesn't fail.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create archive dir: {e}"))?;
    }

    let conn = Connection::open(path).map_err(|e| format!("failed to open archive db: {e}"))?;

    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("failed to set WAL mode: {e}"))?;
    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(|e| format!("failed to set busy_timeout: {e}"))?;

    conn.execute_batch(SCHEMA)
        .map_err(|e| format!("failed to initialize archive schema: {e}"))?;

    Ok(conn)
}

// ── Save subscriptions ──────────────────────────────────────────────────────

/// A save subscription row.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SaveSubscription {
    pub identity_pubkey: String,
    pub relay_url: String,
    pub scope_type: String,
    pub scope_value: String,
    /// JSON-encoded integer array, e.g. `[1,6,39000]`.
    pub kinds: String,
    pub created_at: i64,
}

/// Insert or replace a save subscription. `kinds` must be a JSON int array.
pub fn upsert_save_subscription(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    scope_type: &str,
    scope_value: &str,
    kinds_json: &str,
    now: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO save_subscriptions
             (identity_pubkey, relay_url, scope_type, scope_value, kinds, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (identity_pubkey, relay_url, scope_type, scope_value)
         DO UPDATE SET kinds = excluded.kinds",
        params![
            identity_pubkey,
            relay_url,
            scope_type,
            scope_value,
            kinds_json,
            now
        ],
    )
    .map_err(|e| format!("failed to upsert save subscription: {e}"))?;
    Ok(())
}

/// List all save subscriptions for the given identity + relay.
pub fn list_save_subscriptions(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
) -> Result<Vec<SaveSubscription>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT identity_pubkey, relay_url, scope_type, scope_value, kinds, created_at
             FROM save_subscriptions
             WHERE identity_pubkey = ?1 AND relay_url = ?2
             ORDER BY created_at ASC",
        )
        .map_err(|e| format!("prepare list_save_subscriptions: {e}"))?;

    let rows = stmt
        .query_map(params![identity_pubkey, relay_url], |row| {
            Ok(SaveSubscription {
                identity_pubkey: row.get(0)?,
                relay_url: row.get(1)?,
                scope_type: row.get(2)?,
                scope_value: row.get(3)?,
                kinds: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|e| format!("query list_save_subscriptions: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("read list_save_subscriptions row: {e}"))
}

/// Delete a save subscription. Does NOT purge archived event data (retention
/// is decoupled in v1). Returns `true` if a row was deleted.
pub fn delete_save_subscription(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    scope_type: &str,
    scope_value: &str,
) -> Result<bool, String> {
    let affected = conn
        .execute(
            "DELETE FROM save_subscriptions
             WHERE identity_pubkey = ?1
               AND relay_url       = ?2
               AND scope_type      = ?3
               AND scope_value     = ?4",
            params![identity_pubkey, relay_url, scope_type, scope_value],
        )
        .map_err(|e| format!("failed to delete save subscription: {e}"))?;
    Ok(affected > 0)
}

/// Return true if a matching save subscription exists for the given scope.
#[allow(dead_code)]
pub fn has_save_subscription(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    scope_type: &str,
    scope_value: &str,
) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM save_subscriptions
             WHERE identity_pubkey = ?1
               AND relay_url       = ?2
               AND scope_type      = ?3
               AND scope_value     = ?4",
            params![identity_pubkey, relay_url, scope_type, scope_value],
            |row| row.get(0),
        )
        .map_err(|e| format!("failed to check save subscription: {e}"))?;
    Ok(count > 0)
}

/// Atomically merge `new_kind` into the `owner_p` save subscription for the
/// given identity + relay + scope_value.
///
/// Reads the current `kinds` array, unions in `new_kind`, and writes back —
/// all inside a single `BEGIN IMMEDIATE` SQLite transaction.  `IMMEDIATE`
/// acquires the write lock at `BEGIN` (before the `SELECT`), so a second
/// concurrent caller blocks on `busy_timeout` (5000 ms, set by
/// `open_archive_db`) and then reads the first caller's committed row.  This
/// guarantees the union is complete; a `DEFERRED` transaction would let two
/// concurrent callers both read the empty snapshot and produce a
/// `BUSY_SNAPSHOT` failure on the losing writer.
///
/// If no row exists yet it is created with `[new_kind]`. `now` is used as
/// `created_at` only on insert (the conflict clause never updates it).
pub fn merge_owner_p_kinds(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    scope_value: &str,
    new_kind: u32,
    now: i64,
) -> Result<(), String> {
    // BEGIN IMMEDIATE takes the write lock before the SELECT so concurrent
    // callers serialize here rather than racing to a BUSY_SNAPSHOT error.
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("merge_owner_p_kinds begin immediate: {e}"))?;

    let result = (|| -> Result<(), String> {
        // Read the current kinds, if any.
        let existing_json: Option<String> = conn
            .query_row(
                "SELECT kinds FROM save_subscriptions
                 WHERE identity_pubkey = ?1
                   AND relay_url       = ?2
                   AND scope_type      = 'owner_p'
                   AND scope_value     = ?3",
                params![identity_pubkey, relay_url, scope_value],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("merge_owner_p_kinds read: {e}"))?;

        // Parse, union, re-serialize.
        let mut kinds: Vec<u32> = match existing_json {
            Some(ref json) => serde_json::from_str(json).unwrap_or_default(),
            None => vec![],
        };
        if !kinds.contains(&new_kind) {
            kinds.push(new_kind);
        }
        let kinds_json = serde_json::to_string(&kinds)
            .map_err(|e| format!("merge_owner_p_kinds serialize: {e}"))?;

        // Upsert — INSERT on first call, UPDATE kinds on subsequent.
        conn.execute(
            "INSERT INTO save_subscriptions
                 (identity_pubkey, relay_url, scope_type, scope_value, kinds, created_at)
             VALUES (?1, ?2, 'owner_p', ?3, ?4, ?5)
             ON CONFLICT (identity_pubkey, relay_url, scope_type, scope_value)
             DO UPDATE SET kinds = excluded.kinds",
            params![identity_pubkey, relay_url, scope_value, kinds_json, now],
        )
        .map_err(|e| format!("merge_owner_p_kinds upsert: {e}"))?;

        Ok(())
    })();

    if result.is_ok() {
        conn.execute_batch("COMMIT")
            .map_err(|e| format!("merge_owner_p_kinds commit: {e}"))?;
    } else {
        // Best-effort rollback; ignore the rollback error to surface the
        // original error to the caller.
        let _ = conn.execute_batch("ROLLBACK");
    }

    result
}

/// Atomically remove `kind` from the `owner_p` save subscription for the
/// given identity + relay + scope_value.
///
/// Reads the current `kinds` array, removes `kind` if present, then:
/// - if the resulting list is **empty**, deletes the `owner_p` row entirely
///   (keeping parity with the UI behavior where the last kind off removes the
///   subscription row).
/// - otherwise, updates `kinds` to the reduced list.
///
/// Uses `BEGIN IMMEDIATE` for the same reason as `merge_owner_p_kinds`: the
/// write lock is acquired before the SELECT so concurrent callers serialize
/// rather than racing to a `BUSY_SNAPSHOT` error.
///
/// No-op (Ok) if the row does not exist or the kind is not present.
pub fn remove_owner_p_kind(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    scope_value: &str,
    kind: u32,
) -> Result<(), String> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| format!("remove_owner_p_kind begin immediate: {e}"))?;

    let result = (|| -> Result<(), String> {
        // Read the current kinds, if any.
        let existing_json: Option<String> = conn
            .query_row(
                "SELECT kinds FROM save_subscriptions
                 WHERE identity_pubkey = ?1
                   AND relay_url       = ?2
                   AND scope_type      = 'owner_p'
                   AND scope_value     = ?3",
                params![identity_pubkey, relay_url, scope_value],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|e| format!("remove_owner_p_kind read: {e}"))?;

        let existing_json = match existing_json {
            Some(j) => j,
            // Row doesn't exist — nothing to remove.
            None => return Ok(()),
        };

        let mut kinds: Vec<u32> = serde_json::from_str(&existing_json).unwrap_or_default();
        kinds.retain(|&k| k != kind);

        if kinds.is_empty() {
            // Last kind removed — delete the row entirely.
            conn.execute(
                "DELETE FROM save_subscriptions
                  WHERE identity_pubkey = ?1
                    AND relay_url       = ?2
                    AND scope_type      = 'owner_p'
                    AND scope_value     = ?3",
                params![identity_pubkey, relay_url, scope_value],
            )
            .map_err(|e| format!("remove_owner_p_kind delete: {e}"))?;
        } else {
            let kinds_json = serde_json::to_string(&kinds)
                .map_err(|e| format!("remove_owner_p_kind serialize: {e}"))?;
            conn.execute(
                "UPDATE save_subscriptions
                    SET kinds = ?4
                  WHERE identity_pubkey = ?1
                    AND relay_url       = ?2
                    AND scope_type      = 'owner_p'
                    AND scope_value     = ?3",
                params![identity_pubkey, relay_url, scope_value, kinds_json],
            )
            .map_err(|e| format!("remove_owner_p_kind update: {e}"))?;
        }

        Ok(())
    })();

    if result.is_ok() {
        conn.execute_batch("COMMIT")
            .map_err(|e| format!("remove_owner_p_kind commit: {e}"))?;
    } else {
        let _ = conn.execute_batch("ROLLBACK");
    }

    result
}

/// Return the `kinds` JSON string for a matching save subscription, or `None`
/// if no subscription exists.
pub fn get_subscription_kinds(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    scope_type: &str,
    scope_value: &str,
) -> Result<Option<String>, String> {
    let result = conn
        .query_row(
            "SELECT kinds FROM save_subscriptions
             WHERE identity_pubkey = ?1
               AND relay_url       = ?2
               AND scope_type      = ?3
               AND scope_value     = ?4",
            params![identity_pubkey, relay_url, scope_type, scope_value],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("failed to fetch subscription kinds: {e}"))?;
    Ok(result)
}

// ── Archived events ─────────────────────────────────────────────────────────

/// Upsert an event row (idempotent on the PK).
///
/// Does nothing if the event is already archived (same identity/relay/id).
pub fn upsert_archived_event(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    event_id: &str,
    kind: i64,
    pubkey: &str,
    created_at: i64,
    raw_json: &str,
    archived_at: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO archived_events
             (identity_pubkey, relay_url, id, kind, pubkey, created_at, raw_json, archived_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (identity_pubkey, relay_url, id) DO NOTHING",
        params![
            identity_pubkey,
            relay_url,
            event_id,
            kind,
            pubkey,
            created_at,
            raw_json,
            archived_at
        ],
    )
    .map_err(|e| format!("failed to upsert archived event: {e}"))?;
    Ok(())
}

/// Upsert a scope membership row for an event.
///
/// Idempotent: if the (identity, relay, id, scope_type, scope_value) PK already
/// exists the row is left unchanged.
pub fn upsert_event_scope(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    event_id: &str,
    scope_type: &str,
    scope_value: &str,
    archived_at: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO archived_event_scopes
             (identity_pubkey, relay_url, id, scope_type, scope_value, archived_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (identity_pubkey, relay_url, id, scope_type, scope_value) DO NOTHING",
        params![
            identity_pubkey,
            relay_url,
            event_id,
            scope_type,
            scope_value,
            archived_at
        ],
    )
    .map_err(|e| format!("failed to upsert event scope: {e}"))?;
    Ok(())
}

/// Read a paginated page of archived events for a given scope.
///
/// Returns the `raw_json` of matching events in newest-first order
/// (`ORDER BY created_at DESC, id DESC`). The optional compound cursor
/// `(before_created_at, before_id)` implements keyset pagination: both fields
/// must be `Some` together to activate the cursor (passing one `Some` and one
/// `None` is a logic error at the call site — the store treats mixed `Some`/
/// `None` as no cursor). The predicate mirrors the sort order exactly:
/// `(created_at < before_created_at) OR (created_at = before_created_at AND
/// id < before_id)`. Pass `None`/`None` to start at the newest end.
///
/// A scalar `created_at`-only cursor would skip same-second siblings at a page
/// boundary because rows are ordered by `(created_at DESC, id DESC)` — two
/// rows with equal `created_at` on different pages would both be excluded by
/// `created_at < before`. The compound cursor avoids this.
///
/// An optional `kinds` slice filters by event kind; `None` admits all kinds.
///
/// Returns at most `limit` rows (caller is responsible for a sane default).
pub fn read_archived_events(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
    scope_type: &str,
    scope_value: &str,
    kinds: Option<&[i64]>,
    before_created_at: Option<i64>,
    before_id: Option<&str>,
    limit: i64,
) -> Result<Vec<String>, String> {
    // Build clauses and positional params together so slot numbers are always
    // contiguous (rusqlite rejects gaps like ?4 then ?6 with no ?5 in between).
    //
    // Fixed params: identity_pubkey, relay_url, scope_type, scope_value = ?1–?4.
    // Optional params appended in declaration order, limit always last.

    let mut next_slot: usize = 5;
    let mut extra_clauses = String::new();
    let mut kinds_json: Option<String> = None;
    let mut before_at_val: Option<i64> = None;
    let mut before_id_val: Option<String> = None;

    if let Some(ks) = kinds {
        kinds_json = Some(serde_json::to_string(ks).unwrap_or_else(|_| "[]".to_string()));
        extra_clauses.push_str(&format!(
            " AND ae.kind IN (SELECT value FROM json_each(?{next_slot}))"
        ));
        next_slot += 1;
    }
    // Compound cursor: both fields must be Some to activate.  The predicate
    // mirrors ORDER BY (created_at DESC, id DESC) exactly so no same-second
    // sibling is skipped at a page boundary.
    if let (Some(bat), Some(bid)) = (before_created_at, before_id) {
        before_at_val = Some(bat);
        before_id_val = Some(bid.to_owned());
        extra_clauses.push_str(&format!(
            " AND (ae.created_at < ?{next_slot} \
              OR (ae.created_at = ?{next_slot} AND ae.id < ?{}))",
            next_slot + 1,
        ));
        next_slot += 2;
    }
    let limit_slot = next_slot;

    let sql = format!(
        "SELECT ae.raw_json \
         FROM archived_events ae \
         INNER JOIN archived_event_scopes aes \
             ON aes.identity_pubkey = ae.identity_pubkey \
            AND aes.relay_url       = ae.relay_url \
            AND aes.id              = ae.id \
         WHERE ae.identity_pubkey = ?1 \
           AND ae.relay_url       = ?2 \
           AND aes.scope_type     = ?3 \
           AND aes.scope_value    = ?4\
         {extra_clauses}\
         ORDER BY ae.created_at DESC, ae.id DESC \
         LIMIT ?{limit_slot}",
    );

    // Build the param list dynamically to match the generated SQL.
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![
        Box::new(identity_pubkey.to_owned()),
        Box::new(relay_url.to_owned()),
        Box::new(scope_type.to_owned()),
        Box::new(scope_value.to_owned()),
    ];
    if let Some(kj) = kinds_json {
        params.push(Box::new(kj));
    }
    if let (Some(bat), Some(bid)) = (before_at_val, before_id_val) {
        // Both slots use the same created_at value (the OR predicate references
        // it twice); the id slot follows.
        params.push(Box::new(bat));
        params.push(Box::new(bid));
    }
    params.push(Box::new(limit));

    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("prepare read_archived_events: {e}"))?;

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
        .map_err(|e| format!("query read_archived_events: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("read read_archived_events row: {e}"))
}

/// GC: delete orphaned event rows whose last scope row was just removed.
///
/// Called after any batch deletion of scope rows. Uses a LEFT JOIN so only
/// events with zero remaining scope rows are deleted.
#[allow(dead_code)] // Used by P4 purge commands; not yet wired to a Tauri command.
pub fn gc_orphaned_events(
    conn: &Connection,
    identity_pubkey: &str,
    relay_url: &str,
) -> Result<usize, String> {
    let affected = conn
        .execute(
            "DELETE FROM archived_events
             WHERE identity_pubkey = ?1
               AND relay_url       = ?2
               AND id NOT IN (
                   SELECT id FROM archived_event_scopes
                   WHERE identity_pubkey = ?1
                     AND relay_url       = ?2
               )",
            params![identity_pubkey, relay_url],
        )
        .map_err(|e| format!("failed to gc orphaned events: {e}"))?;
    Ok(affected)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;
