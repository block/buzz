//! Local SQLite mirror: events, channels, profiles, folds, artifacts.
//!
//! The store is the daemon's single source of local truth. Events are
//! content-addressed (id = hash), so persistence is an idempotent
//! `INSERT OR IGNORE` — reconnects and overlapping backfills cannot corrupt
//! anything. Artifacts use a `(fold, version)` primary key, which makes the
//! version fence atomic by construction: a losing concurrent run hits the
//! constraint instead of forking the chain.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::{QueryBuilder, Row};

use crate::{ArtifactPayload, FoldSpec, Selection, Signal};

/// A stored relay event, distilled to the columns the daemon queries.
#[derive(Debug, Clone)]
pub struct StoredEvent {
    /// 64-hex event id.
    pub id: String,
    /// Channel (`h` tag) the event belongs to, if any.
    pub channel: Option<String>,
    /// Author pubkey, 64-hex.
    pub pubkey: String,
    /// Event kind.
    pub kind: u32,
    /// Unix seconds.
    pub created_at: i64,
    /// Event content, verbatim.
    pub content: String,
    /// Full raw event JSON as received from the relay.
    pub raw: String,
}

/// A discovered channel and its backfill bookkeeping.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelRow {
    /// Channel UUID.
    pub id: String,
    /// Display name from kind-39000 metadata (None until resolved).
    pub name: Option<String>,
    /// `stream` | `private` | `dm` | `unknown`.
    pub channel_type: String,
    /// Next `until` cursor for backfill paging; `None` once backfill is done.
    pub backfill_cursor: Option<i64>,
    /// Whether historical backfill has reached the beginning.
    pub backfill_done: bool,
    /// False once the relay revoked access (kept for local history).
    pub active: bool,
    /// Unix seconds when the daemon first discovered the channel.
    pub discovered_at: i64,
}

/// Fold spec row plus chain summary, as listed over HTTP.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FoldRow {
    /// Fold name (unique).
    pub name: String,
    /// The validated spec.
    pub spec: FoldSpec,
    /// Number of artifact versions in the chain.
    pub versions: u32,
    /// Latest artifact version, if any.
    pub latest_version: Option<u32>,
    /// Unix seconds of creation / last spec update.
    pub created_at: i64,
    /// Unix seconds of the last spec update.
    pub updated_at: i64,
}

/// Storage handle. Cheap to clone.
#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    id         TEXT PRIMARY KEY,
    channel    TEXT,
    pubkey     TEXT NOT NULL,
    kind       INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    content    TEXT NOT NULL,
    raw        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_channel_time ON events(channel, created_at);
CREATE INDEX IF NOT EXISTS idx_events_kind_time    ON events(kind, created_at);
CREATE INDEX IF NOT EXISTS idx_events_author       ON events(pubkey);

CREATE TABLE IF NOT EXISTS channels (
    id              TEXT PRIMARY KEY,
    name            TEXT,
    channel_type    TEXT NOT NULL DEFAULT 'unknown',
    backfill_cursor INTEGER,
    backfill_done   INTEGER NOT NULL DEFAULT 0,
    active          INTEGER NOT NULL DEFAULT 1,
    discovered_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS profiles (
    pubkey     TEXT PRIMARY KEY,
    name       TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS folds (
    name       TEXT PRIMARY KEY,
    spec       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS artifacts (
    fold       TEXT NOT NULL,
    version    INTEGER NOT NULL,
    payload    TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (fold, version)
);
";

impl Store {
    /// Opens (creating if missing) the SQLite database at `path`.
    ///
    /// `path` may be a filesystem path or the literal `:memory:` for tests.
    pub async fn open(path: &str) -> Result<Self, sqlx::Error> {
        let options = if path == ":memory:" {
            SqliteConnectOptions::from_str("sqlite::memory:")?
        } else {
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
        }
        .journal_mode(SqliteJournalMode::Wal);
        // A single connection serializes writers; the daemon is the only
        // writer and SQLite write contention is not a bottleneck here. It is
        // also what keeps `:memory:` databases alive across calls in tests.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::raw_sql(SCHEMA).execute(&pool).await?;
        Ok(Self { pool })
    }

    // ---- events ----

    /// Idempotently persists a batch of events. Returns how many were new.
    pub async fn upsert_events(&self, events: &[StoredEvent]) -> Result<u64, sqlx::Error> {
        let mut inserted = 0;
        let mut tx = self.pool.begin().await?;
        for ev in events {
            let done = sqlx::query(
                "INSERT OR IGNORE INTO events (id, channel, pubkey, kind, created_at, content, raw)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&ev.id)
            .bind(&ev.channel)
            .bind(&ev.pubkey)
            .bind(ev.kind as i64)
            .bind(ev.created_at)
            .bind(&ev.content)
            .bind(&ev.raw)
            .execute(&mut *tx)
            .await?;
            inserted += done.rows_affected();
        }
        tx.commit().await?;
        Ok(inserted)
    }

    /// Total stored events, and per-channel counts.
    pub async fn event_counts(&self) -> Result<(i64, BTreeMap<String, i64>), sqlx::Error> {
        let total = sqlx::query("SELECT COUNT(*) AS n FROM events")
            .fetch_one(&self.pool)
            .await?
            .get::<i64, _>("n");
        let rows = sqlx::query(
            "SELECT channel, COUNT(*) AS n FROM events WHERE channel IS NOT NULL GROUP BY channel",
        )
        .fetch_all(&self.pool)
        .await?;
        let per_channel = rows
            .into_iter()
            .map(|r| (r.get::<String, _>("channel"), r.get::<i64, _>("n")))
            .collect();
        Ok((total, per_channel))
    }

    /// Newest stored `created_at` for a channel, if any events exist.
    pub async fn newest_ts(&self, channel: &str) -> Result<Option<i64>, sqlx::Error> {
        let row = sqlx::query("SELECT MAX(created_at) AS ts FROM events WHERE channel = ?")
            .bind(channel)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.try_get::<Option<i64>, _>("ts").unwrap_or(None))
    }

    /// Signals matched by `selection` over `[since, until_exclusive)`, ordered
    /// `created_at ASC, id ASC` (the engine re-materializes regardless).
    pub async fn query_signals(
        &self,
        selection: &Selection,
        since: i64,
        until_exclusive: i64,
    ) -> Result<Vec<Signal>, sqlx::Error> {
        let mut qb = signal_query(selection, since, until_exclusive);
        qb.push(" ORDER BY created_at ASC, id ASC");
        let rows = qb.build().fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(row_to_signal).collect())
    }

    /// One page of signals for a selection, in `created_at ASC, id ASC` order.
    ///
    /// `after` is a keyset cursor — the `(created_at, id)` of the last row of
    /// the previous page; rows strictly after it are returned. Content-addressed
    /// ids make the cursor stable across live-tail inserts.
    pub async fn page_signals(
        &self,
        selection: &Selection,
        since: i64,
        until_exclusive: i64,
        after: Option<(i64, &str)>,
        limit: i64,
    ) -> Result<Vec<Signal>, sqlx::Error> {
        let mut qb = signal_query(selection, since, until_exclusive);
        if let Some((ts, id)) = after {
            qb.push(" AND (created_at > ");
            qb.push_bind(ts);
            qb.push(" OR (created_at = ");
            qb.push_bind(ts);
            qb.push(" AND id > ");
            qb.push_bind(id.to_string());
            qb.push("))");
        }
        qb.push(" ORDER BY created_at ASC, id ASC LIMIT ");
        qb.push_bind(limit);
        let rows = qb.build().fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(row_to_signal).collect())
    }

    /// Resolves one stored event by 64-hex id.
    pub async fn event_by_id(&self, id: &str) -> Result<Option<Signal>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, channel, pubkey, kind, created_at, content FROM events WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_to_signal))
    }

    // ---- channels ----

    /// Registers a channel if unknown, seeding the backfill cursor at
    /// `discovered_at` (backfill pages descend from "now"). Known channels
    /// only refresh name/type/active.
    pub async fn upsert_channel(
        &self,
        id: &str,
        name: Option<&str>,
        channel_type: &str,
        discovered_at: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO channels (id, name, channel_type, backfill_cursor, backfill_done, active, discovered_at)
             VALUES (?, ?, ?, ?, 0, 1, ?)
             ON CONFLICT(id) DO UPDATE SET
                 name = COALESCE(excluded.name, channels.name),
                 channel_type = excluded.channel_type,
                 active = 1",
        )
        .bind(id)
        .bind(name)
        .bind(channel_type)
        .bind(discovered_at)
        .bind(discovered_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// All channel rows.
    pub async fn channels(&self) -> Result<Vec<ChannelRow>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, name, channel_type, backfill_cursor, backfill_done, active, discovered_at
             FROM channels ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ChannelRow {
                id: r.get("id"),
                name: r.get("name"),
                channel_type: r.get("channel_type"),
                backfill_cursor: r.get("backfill_cursor"),
                backfill_done: r.get::<i64, _>("backfill_done") != 0,
                active: r.get::<i64, _>("active") != 0,
                discovered_at: r.get("discovered_at"),
            })
            .collect())
    }

    /// Persists backfill progress for a channel.
    pub async fn set_backfill(
        &self,
        id: &str,
        cursor: Option<i64>,
        done: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE channels SET backfill_cursor = ?, backfill_done = ? WHERE id = ?")
            .bind(cursor)
            .bind(done as i64)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Marks a channel inactive (access revoked / archived after discovery).
    pub async fn deactivate_channel(&self, id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE channels SET active = 0 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- profiles ----

    /// Records a kind-0 profile name; newest `created_at` wins.
    pub async fn upsert_profile(
        &self,
        pubkey: &str,
        name: Option<&str>,
        created_at: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO profiles (pubkey, name, created_at) VALUES (?, ?, ?)
             ON CONFLICT(pubkey) DO UPDATE SET
                 name = excluded.name, created_at = excluded.created_at
             WHERE excluded.created_at >= profiles.created_at",
        )
        .bind(pubkey)
        .bind(name)
        .bind(created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Display names for the given pubkeys (missing entries are simply absent).
    pub async fn names(
        &self,
        pubkeys: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, String>, sqlx::Error> {
        if pubkeys.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut qb: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new(
            "SELECT pubkey, name FROM profiles WHERE name IS NOT NULL AND pubkey IN (",
        );
        let mut sep = qb.separated(", ");
        for pk in pubkeys {
            sep.push_bind(pk);
        }
        qb.push(")");
        let rows = qb.build().fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let name: Option<String> = r.get("name");
                name.map(|n| (r.get::<String, _>("pubkey"), n))
            })
            .collect())
    }

    // ---- folds ----

    /// Creates or replaces a fold spec (name is the identity).
    pub async fn put_fold(&self, spec: &FoldSpec, now: i64) -> Result<(), sqlx::Error> {
        let json = serde_json::to_string(spec).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        sqlx::query(
            "INSERT INTO folds (name, spec, created_at, updated_at) VALUES (?, ?, ?, ?)
             ON CONFLICT(name) DO UPDATE SET spec = excluded.spec, updated_at = excluded.updated_at",
        )
        .bind(&spec.name)
        .bind(json)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Loads one fold spec by name.
    pub async fn get_fold(&self, name: &str) -> Result<Option<FoldSpec>, sqlx::Error> {
        let row = sqlx::query("SELECT spec FROM folds WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            None => Ok(None),
            Some(r) => {
                let json: String = r.get("spec");
                serde_json::from_str(&json)
                    .map(Some)
                    .map_err(|e| sqlx::Error::Decode(Box::new(e)))
            }
        }
    }

    /// Deletes a fold spec. Artifacts are append-only and stay.
    pub async fn delete_fold(&self, name: &str) -> Result<bool, sqlx::Error> {
        let done = sqlx::query("DELETE FROM folds WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// All folds with chain summaries.
    pub async fn folds(&self) -> Result<Vec<FoldRow>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT f.name, f.spec, f.created_at, f.updated_at,
                    COUNT(a.version) AS versions, MAX(a.version) AS latest
             FROM folds f LEFT JOIN artifacts a ON a.fold = f.name
             GROUP BY f.name ORDER BY f.name",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let json: String = r.get("spec");
            let spec: FoldSpec =
                serde_json::from_str(&json).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
            out.push(FoldRow {
                name: r.get("name"),
                spec,
                versions: r.get::<i64, _>("versions") as u32,
                latest_version: r
                    .try_get::<Option<i64>, _>("latest")
                    .unwrap_or(None)
                    .map(|v| v as u32),
                created_at: r.get("created_at"),
                updated_at: r.get("updated_at"),
            });
        }
        Ok(out)
    }

    // ---- artifacts ----

    /// Appends the next artifact version.
    ///
    /// The `(fold, version)` primary key IS the version fence: if a concurrent
    /// run already appended this version, the insert fails and nothing forks.
    pub async fn insert_artifact(&self, payload: &ArtifactPayload) -> Result<(), sqlx::Error> {
        let json = serde_json::to_string(payload).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
        sqlx::query(
            "INSERT INTO artifacts (fold, version, payload, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&payload.fold)
        .bind(payload.version as i64)
        .bind(json)
        .bind(payload.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The full artifact chain for a fold, ordered by version ascending.
    pub async fn artifacts(&self, fold: &str) -> Result<Vec<ArtifactPayload>, sqlx::Error> {
        let rows = sqlx::query("SELECT payload FROM artifacts WHERE fold = ? ORDER BY version ASC")
            .bind(fold)
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let json: String = r.get("payload");
            out.push(serde_json::from_str(&json).map_err(|e| sqlx::Error::Decode(Box::new(e)))?);
        }
        Ok(out)
    }

    /// One artifact version, if present.
    pub async fn artifact(
        &self,
        fold: &str,
        version: u32,
    ) -> Result<Option<ArtifactPayload>, sqlx::Error> {
        let row = sqlx::query("SELECT payload FROM artifacts WHERE fold = ? AND version = ?")
            .bind(fold)
            .bind(version as i64)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            None => Ok(None),
            Some(r) => {
                let json: String = r.get("payload");
                serde_json::from_str(&json)
                    .map(Some)
                    .map_err(|e| sqlx::Error::Decode(Box::new(e)))
            }
        }
    }

    /// Total artifact count across all folds.
    pub async fn artifact_count(&self) -> Result<i64, sqlx::Error> {
        Ok(sqlx::query("SELECT COUNT(*) AS n FROM artifacts")
            .fetch_one(&self.pool)
            .await?
            .get::<i64, _>("n"))
    }
}

/// Base signal query: columns + window + selection predicates, no ordering.
fn signal_query(
    selection: &Selection,
    since: i64,
    until_exclusive: i64,
) -> QueryBuilder<sqlx::Sqlite> {
    let mut qb: QueryBuilder<sqlx::Sqlite> = QueryBuilder::new(
        "SELECT id, channel, pubkey, kind, created_at, content FROM events WHERE created_at >= ",
    );
    qb.push_bind(since);
    qb.push(" AND created_at < ");
    qb.push_bind(until_exclusive);
    if !selection.channels.is_empty() {
        qb.push(" AND channel IN (");
        let mut sep = qb.separated(", ");
        for c in &selection.channels {
            sep.push_bind(c);
        }
        qb.push(")");
    }
    if !selection.authors.is_empty() {
        qb.push(" AND pubkey IN (");
        let mut sep = qb.separated(", ");
        for a in &selection.authors {
            sep.push_bind(a);
        }
        qb.push(")");
    }
    qb.push(" AND kind IN (");
    let mut sep = qb.separated(", ");
    for k in selection.effective_kinds() {
        sep.push_bind(k as i64);
    }
    qb.push(")");
    qb
}

fn row_to_signal(r: sqlx::sqlite::SqliteRow) -> Signal {
    Signal {
        id: r.get("id"),
        pubkey: r.get("pubkey"),
        kind: r.get::<i64, _>("kind") as u32,
        created_at: r.get("created_at"),
        content: r.get("content"),
        channel: r.get("channel"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, channel: Option<&str>, kind: u32, ts: i64) -> StoredEvent {
        StoredEvent {
            id: id.repeat(64 / id.len().max(1)),
            channel: channel.map(str::to_string),
            pubkey: "a".repeat(64),
            kind,
            created_at: ts,
            content: format!("content-{id}"),
            raw: "{}".into(),
        }
    }

    fn selection(channels: &[&str]) -> Selection {
        Selection {
            channels: channels.iter().map(|s| s.to_string()).collect(),
            authors: vec![],
            kinds: vec![],
        }
    }

    #[tokio::test]
    async fn upsert_is_idempotent() {
        let store = Store::open(":memory:").await.expect("open");
        let events = vec![ev("1", Some("ch"), 9, 100), ev("2", Some("ch"), 9, 101)];
        assert_eq!(store.upsert_events(&events).await.expect("insert"), 2);
        assert_eq!(store.upsert_events(&events).await.expect("re-insert"), 0);
        let (total, per) = store.event_counts().await.expect("counts");
        assert_eq!(total, 2);
        assert_eq!(per.get("ch"), Some(&2));
    }

    #[tokio::test]
    async fn signals_filter_by_selection_window_and_kind() {
        let store = Store::open(":memory:").await.expect("open");
        store
            .upsert_events(&[
                ev("1", Some("cha"), 9, 100),
                ev("2", Some("chb"), 9, 100),
                ev("3", Some("cha"), 7, 100), // wrong kind (default = [9])
                ev("4", Some("cha"), 9, 200), // outside window
            ])
            .await
            .expect("insert");
        let got = store
            .query_signals(&selection(&["cha"]), 0, 200)
            .await
            .expect("query");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].created_at, 100);
        assert_eq!(got[0].channel.as_deref(), Some("cha"));
    }

    #[tokio::test]
    async fn artifact_pk_is_a_version_fence() {
        let store = Store::open(":memory:").await.expect("open");
        let payload = ArtifactPayload {
            fold: "f".into(),
            version: 1,
            output: "# Working Context\n\n# Log\n".into(),
            shown_ids: vec![],
            coverage_since: None,
            coverage_until: None,
            selection: selection(&[]),
            channels: vec![],
            model: "m".into(),
            schema: "channel-digest@v1".into(),
            prompt_sha256: "0".repeat(64),
            truncated: false,
            created_at: 1,
        };
        store.insert_artifact(&payload).await.expect("first insert");
        let second = store.insert_artifact(&payload).await;
        assert!(second.is_err(), "duplicate version must be refused");
        assert_eq!(store.artifacts("f").await.expect("chain").len(), 1);
    }

    #[tokio::test]
    async fn profile_newest_wins() {
        let store = Store::open(":memory:").await.expect("open");
        let pk = "b".repeat(64);
        store
            .upsert_profile(&pk, Some("new"), 200)
            .await
            .expect("insert");
        store
            .upsert_profile(&pk, Some("old"), 100)
            .await
            .expect("stale update");
        let names = store
            .names(&[pk.clone()].into_iter().collect())
            .await
            .expect("names");
        assert_eq!(names.get(&pk).map(String::as_str), Some("new"));
    }

    #[tokio::test]
    async fn fold_crud_roundtrip() {
        let store = Store::open(":memory:").await.expect("open");
        let mut spec = FoldSpec {
            name: "weekly".into(),
            selection: Selection {
                channels: vec!["6ba7b810-9dad-11d1-80b4-00c04fd430c8".into()],
                authors: vec![],
                kinds: vec![],
            },
            schema: "channel-digest@v1".into(),
            model: "haiku".into(),
            instructions: "digest".into(),
            meta: None,
        };
        spec.validate().expect("valid spec");
        store.put_fold(&spec, 1).await.expect("put");
        let loaded = store
            .get_fold("weekly")
            .await
            .expect("get")
            .expect("present");
        assert_eq!(loaded.model, "haiku");
        assert_eq!(store.folds().await.expect("list").len(), 1);
        assert!(store.delete_fold("weekly").await.expect("delete"));
        assert!(store.get_fold("weekly").await.expect("get").is_none());
    }
}
