use chrono::{DateTime, Utc};
use futures_util::FutureExt as _;
use sqlx::{Acquire, PgPool, Row};
use tracing::{debug, instrument, warn};

use sprout_core::kind::KIND_AUTH;

use crate::{
    action::AuditAction,
    entry::{AuditEntry, NewAuditEntry},
    error::AuditError,
    hash::{compute_hash, GENESIS_HASH},
    schema::AUDIT_SCHEMA_SQL,
};

/// Advisory lock key derived from a stable hash of "sprout_audit".
const AUDIT_LOCK_KEY: i64 = 0x5370_7275_7441_7564; // "SprutAud" as hex

/// Append-only audit log service backed by Postgres.
///
/// Serialises writes via `pg_advisory_lock` so the hash chain remains consistent
/// even when multiple relay processes share the same database.
pub struct AuditService {
    pool: PgPool,
}

impl AuditService {
    /// Creates a new `AuditService` using the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Idempotent — safe to call on every startup.
    pub async fn ensure_schema(&self) -> Result<(), AuditError> {
        sqlx::raw_sql(AUDIT_SCHEMA_SQL).execute(&self.pool).await?;
        Ok(())
    }

    /// Append a new entry to the audit log. Single-writer via `pg_advisory_lock`.
    ///
    /// Postgres advisory locks are session-scoped, so we acquire before the
    /// transaction and release after commit (or on any error path).
    #[instrument(skip(self, entry), fields(action = %entry.action))]
    pub async fn log(&self, entry: NewAuditEntry) -> Result<AuditEntry, AuditError> {
        if entry.event_kind == KIND_AUTH {
            warn!("rejected attempt to audit AUTH event (kind 22242)");
            return Err(AuditError::AuthEventForbidden);
        }

        let mut conn = self.pool.acquire().await?;

        // Acquire session-level advisory lock (blocks until available).
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(AUDIT_LOCK_KEY)
            .execute(&mut *conn)
            .await?;

        // Run log_inner and release the lock regardless of outcome.
        // We use catch_unwind to handle panics so the lock is always released
        // before the connection is returned to the pool.
        let result = std::panic::AssertUnwindSafe(self.log_inner(&mut conn, entry))
            .catch_unwind()
            .await;

        let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(AUDIT_LOCK_KEY)
            .execute(&mut *conn)
            .await;

        match result {
            Ok(inner_result) => inner_result,
            Err(panic_payload) => std::panic::resume_unwind(panic_payload),
        }
    }

    async fn log_inner(
        &self,
        conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
        entry: NewAuditEntry,
    ) -> Result<AuditEntry, AuditError> {
        let mut tx = conn.begin().await?;

        let prev_hash: String = sqlx::query("SELECT hash FROM audit_log ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&mut *tx)
            .await?
            .map(|row| row.get::<String, _>("hash"))
            .unwrap_or_else(|| GENESIS_HASH.to_string());

        let seq: i64 =
            sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 AS next_seq FROM audit_log")
                .fetch_one(&mut *tx)
                .await?;

        let timestamp: DateTime<Utc> = Utc::now();

        let channel_id_bytes: Option<Vec<u8>> = entry.channel_id.map(|u| u.as_bytes().to_vec());

        let mut audit_entry = AuditEntry {
            seq,
            timestamp,
            event_id: entry.event_id,
            event_kind: entry.event_kind,
            actor_pubkey: entry.actor_pubkey,
            action: entry.action,
            channel_id: entry.channel_id,
            metadata: entry.metadata,
            prev_hash,
            hash: String::new(),
        };

        audit_entry.hash = compute_hash(&audit_entry)?;

        debug!(seq, hash = %audit_entry.hash, "writing audit entry");

        sqlx::query(
            r#"
            INSERT INTO audit_log
                (seq, timestamp, event_id, event_kind, actor_pubkey, action,
                 channel_id, metadata, prev_hash, hash)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(audit_entry.seq)
        .bind(audit_entry.timestamp)
        .bind(&audit_entry.event_id)
        .bind(audit_entry.event_kind as i32)
        .bind(&audit_entry.actor_pubkey)
        .bind(audit_entry.action.as_str())
        .bind(channel_id_bytes)
        .bind(&audit_entry.metadata)
        .bind(&audit_entry.prev_hash)
        .bind(&audit_entry.hash)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(audit_entry)
    }

    /// Verify the hash chain for `[from_seq, to_seq]`.
    /// Returns `Ok(false)` if range is empty, `Ok(true)` if valid.
    #[instrument(skip(self))]
    pub async fn verify_chain(&self, from_seq: i64, to_seq: i64) -> Result<bool, AuditError> {
        let rows = sqlx::query(
            r#"
            SELECT seq, timestamp, event_id, event_kind, actor_pubkey,
                   action, channel_id, metadata, prev_hash, hash
            FROM audit_log
            WHERE seq BETWEEN $1 AND $2
            ORDER BY seq ASC
            "#,
        )
        .bind(from_seq)
        .bind(to_seq)
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(false);
        }

        let mut expected_prev: Option<String> = None;

        for row in &rows {
            let entry = row_to_audit_entry(row)?;
            let prev_hash = entry.prev_hash.clone();
            let stored_hash = entry.hash.clone();

            if let Some(ref expected) = expected_prev {
                if &prev_hash != expected {
                    return Err(AuditError::ChainViolation {
                        seq: entry.seq,
                        expected: expected.clone(),
                        actual: prev_hash,
                    });
                }
            }

            let computed = compute_hash(&entry)?;
            if computed != stored_hash {
                return Err(AuditError::HashMismatch {
                    seq: entry.seq,
                    stored: stored_hash,
                    computed,
                });
            }

            expected_prev = Some(entry.hash);
        }

        Ok(true)
    }

    /// Returns up to `limit` entries starting at `from_seq`, ordered by sequence number.
    #[instrument(skip(self))]
    pub async fn get_entries(
        &self,
        from_seq: i64,
        limit: i64,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        let rows = sqlx::query(
            r#"
            SELECT seq, timestamp, event_id, event_kind, actor_pubkey,
                   action, channel_id, metadata, prev_hash, hash
            FROM audit_log
            WHERE seq >= $1
            ORDER BY seq ASC
            LIMIT $2
            "#,
        )
        .bind(from_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_audit_entry).collect()
    }
}

fn row_to_audit_entry(row: &sqlx::postgres::PgRow) -> Result<AuditEntry, AuditError> {
    let seq: i64 = row.get("seq");
    let action_str: String = row.get("action");
    let action: AuditAction = action_str.parse().map_err(|_| {
        warn!(seq, action = %action_str, "unknown action in audit log");
        AuditError::UnknownAction(action_str.clone())
    })?;

    let channel_id_bytes: Option<Vec<u8>> = row.get("channel_id");
    let channel_id = channel_id_bytes.and_then(|b| b.try_into().ok().map(uuid::Uuid::from_bytes));

    let raw_kind: i32 = row.get("event_kind");
    let event_kind = u32::try_from(raw_kind).map_err(|_| {
        AuditError::Database(sqlx::Error::Protocol(format!(
            "event_kind {raw_kind} out of u32 range at seq {seq}"
        )))
    })?;

    Ok(AuditEntry {
        seq,
        timestamp: row.get("timestamp"),
        event_id: row.get("event_id"),
        event_kind,
        actor_pubkey: row.get("actor_pubkey"),
        action,
        channel_id,
        metadata: row.get("metadata"),
        prev_hash: row.get("prev_hash"),
        hash: row.get("hash"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::AuditAction;
    use crate::entry::NewAuditEntry;
    use crate::hash::GENESIS_HASH;
    use std::sync::OnceLock;
    use tokio::sync::Mutex;

    static DB_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn db_lock() -> &'static Mutex<()> {
        DB_LOCK.get_or_init(|| Mutex::new(()))
    }

    async fn test_pool() -> Option<PgPool> {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://sprout:sprout_dev@localhost:5432/sprout".into());
        PgPool::connect(&url).await.ok()
    }

    fn sample_new_entry(kind: u32, action: AuditAction) -> NewAuditEntry {
        NewAuditEntry {
            event_id: format!("evt_{}", uuid::Uuid::new_v4()),
            event_kind: kind,
            actor_pubkey: "deadbeefdeadbeef".into(),
            action,
            channel_id: None,
            metadata: serde_json::json!({"test": true}),
        }
    }

    async fn reset_audit_table(pool: &PgPool) {
        sqlx::query("TRUNCATE TABLE audit_log")
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn genesis_entry() {
        let _guard = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        svc.ensure_schema().await.unwrap();
        reset_audit_table(&pool).await;

        let entry = svc
            .log(sample_new_entry(1, AuditAction::EventCreated))
            .await
            .unwrap();

        assert_eq!(entry.prev_hash, GENESIS_HASH);
        assert_eq!(entry.seq, 1);
        assert_eq!(entry.hash.len(), 64);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn chain_integrity() {
        let _guard = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        svc.ensure_schema().await.unwrap();
        reset_audit_table(&pool).await;

        let e1 = svc
            .log(sample_new_entry(1, AuditAction::EventCreated))
            .await
            .unwrap();
        let e2 = svc
            .log(sample_new_entry(1, AuditAction::ChannelCreated))
            .await
            .unwrap();
        let e3 = svc
            .log(sample_new_entry(1, AuditAction::MemberAdded))
            .await
            .unwrap();

        assert_eq!(e1.prev_hash, GENESIS_HASH);
        assert_eq!(e2.prev_hash, e1.hash);
        assert_eq!(e3.prev_hash, e2.hash);

        assert!(svc.verify_chain(e1.seq, e3.seq).await.unwrap());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn verify_chain_detects_tampering() {
        let _guard = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        svc.ensure_schema().await.unwrap();
        reset_audit_table(&pool).await;

        let e1 = svc
            .log(sample_new_entry(1, AuditAction::EventCreated))
            .await
            .unwrap();
        let e2 = svc
            .log(sample_new_entry(1, AuditAction::EventDeleted))
            .await
            .unwrap();
        let e3 = svc
            .log(sample_new_entry(1, AuditAction::ChannelDeleted))
            .await
            .unwrap();

        sqlx::query("UPDATE audit_log SET actor_pubkey = 'tampered' WHERE seq = $1")
            .bind(e2.seq)
            .execute(&pool)
            .await
            .unwrap();

        let result = svc.verify_chain(e1.seq, e3.seq).await;
        assert!(matches!(result, Err(AuditError::HashMismatch { seq, .. }) if seq == e2.seq));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn auth_events_rejected() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());

        let result = svc
            .log(sample_new_entry(KIND_AUTH, AuditAction::AuthSuccess))
            .await;

        assert!(matches!(result, Err(AuditError::AuthEventForbidden)));
    }
}
