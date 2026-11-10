//! Atomic, bounded, writer-only snapshots of an author's kind-30078 state.

use buzz_core::{kind::KIND_READ_STATE, CommunityId};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;

use crate::{observability, Db, DbError, Result};

/// Maximum number of events in a complete snapshot; overflow is an error.
pub const MAX_SNAPSHOT_EVENTS: usize = 4096;
/// Maximum stored payload or encoded event-array bytes; overflow is an error.
pub const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;

/// A complete view at one writer statement's MVCC cut, not a live-delivery fence.
#[derive(Debug)]
pub struct ReadStateSnapshot {
    /// Content identifier bound to community, author, and ordered event ids.
    /// Not a monotone revision, CAS token, or live-stream cursor.
    pub snapshot_id: String,
    /// Every current own kind-30078 event, including unrelated application data.
    pub events: Vec<Value>,
}

impl Db {
    /// Load all current kind-30078 events for exactly one author and community.
    ///
    /// A single writer statement binds selection, resource preflight, and row
    /// retrieval to the same MVCC snapshot. No tag, horizon, replica, or ordinary
    /// query-page cap is involved. Any corrupt row or budget overflow fails the
    /// whole operation before a caller can assert completeness.
    pub async fn read_state_snapshot(
        &self,
        community: CommunityId,
        author: &nostr::PublicKey,
    ) -> Result<ReadStateSnapshot> {
        let mut conn = observability::acquire_writer(
            &self.pool,
            observability::WriterOperation::SubscriptionHistory,
        )
        .await?;
        // Select only bounded metadata first. A left join retains the budget
        // sentinel even for empty/oversized sets; oversized payloads never leave
        // Postgres. Both CTEs and the payload join share ONE statement snapshot,
        // including when a coordinate is hard-deleted/replaced concurrently.
        let rows = sqlx::query(
            "WITH candidates AS MATERIALIZED (
                SELECT id, created_at,
                       octet_length(content)::bigint + octet_length(tags::text)::bigint AS bytes
                FROM events
                WHERE community_id = $1 AND pubkey = $2 AND kind = $3
                  AND deleted_at IS NULL
                ORDER BY created_at DESC, id ASC LIMIT $4
             ), budget AS MATERIALIZED (
                SELECT count(*) AS n, COALESCE(sum(bytes), 0)::bigint AS bytes FROM candidates
             )
             SELECT budget.n AS snapshot_count, budget.bytes AS snapshot_bytes,
                    e.id, e.pubkey, e.created_at, e.kind, e.tags, e.content, e.sig,
                    e.received_at, e.channel_id
             FROM budget
             LEFT JOIN candidates c ON budget.n <= $5 AND budget.bytes <= $6
             LEFT JOIN events e ON e.community_id = $1 AND e.id = c.id
                 AND e.created_at = c.created_at
             ORDER BY e.created_at DESC, e.id ASC",
        )
        .bind(community.as_uuid())
        .bind(author.to_bytes().to_vec())
        .bind(KIND_READ_STATE as i32)
        .bind((MAX_SNAPSHOT_EVENTS + 1) as i64)
        .bind(MAX_SNAPSHOT_EVENTS as i64)
        .bind(MAX_SNAPSHOT_BYTES as i64)
        .fetch_all(&mut *conn)
        .await?;

        let first = rows.first().ok_or_else(|| {
            DbError::InvalidData("read-state snapshot missing budget sentinel".into())
        })?;
        let count: i64 = first.try_get("snapshot_count")?;
        let stored_bytes: i64 = first.try_get("snapshot_bytes")?;
        if count > MAX_SNAPSHOT_EVENTS as i64 || stored_bytes > MAX_SNAPSHOT_BYTES as i64 {
            return Err(DbError::ReadStateSnapshotTooLarge);
        }
        let mut events = Vec::with_capacity(count as usize);
        let mut encoded_bytes = 2usize; // JSON array brackets
        let mut hash = Sha256::new();
        hash.update(b"buzz-read-state-snapshot-v1\0");
        hash.update(community.as_uuid().as_bytes());
        hash.update(author.to_bytes());
        for row in rows {
            if count == 0 {
                break;
            }
            let stored = super::event::row_to_stored_event(row)?.ok_or_else(|| {
                DbError::InvalidData("unreadable read-state snapshot event".into())
            })?;
            let event = stored.event;
            if event.pubkey != *author || event.kind.as_u16() as u32 != KIND_READ_STATE {
                return Err(DbError::InvalidData(
                    "read-state snapshot scope mismatch".into(),
                ));
            }
            event.verify().map_err(|_| {
                DbError::InvalidData("invalid signed read-state snapshot event".into())
            })?;
            let value = serde_json::to_value(&event)?;
            encoded_bytes += serde_json::to_vec(&value)?.len() + usize::from(!events.is_empty());
            if encoded_bytes > MAX_SNAPSHOT_BYTES {
                return Err(DbError::ReadStateSnapshotTooLarge);
            }
            hash.update(event.id.as_bytes());
            events.push(value);
        }
        if events.len() != count as usize {
            return Err(DbError::InvalidData(
                "read-state snapshot row count mismatch".into(),
            ));
        }
        Ok(ReadStateSnapshot {
            snapshot_id: hex::encode(hash.finalize()),
            events,
        })
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use sqlx::{postgres::PgPoolOptions, PgPool};
    use std::time::Duration;

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cluster_global_read_state_snapshot_replacement_during_preflight_keeps_one_mvcc_cut() {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let community = db
            .ensure_configured_community(&format!("mvcc-{}.local", uuid::Uuid::new_v4().simple()))
            .await
            .unwrap()
            .id;
        let key = Keys::generate();
        let slot = format!("read-state:{}", uuid::Uuid::new_v4().simple());
        let make = |ts, content| {
            EventBuilder::new(Kind::Custom(30078), content)
                .tags([Tag::identifier(&slot), Tag::hashtag("read-state")])
                .custom_created_at(Timestamp::from(ts))
                .sign_with_keys(&key)
                .unwrap()
        };
        let before = make(1789090000, "before");
        let after = make(1789090001, "after");
        db.replace_parameterized_event(community, &before, &slot, None)
            .await
            .unwrap();

        // Gate the real production SQL at octet_length(content), after it has
        // taken its statement snapshot. Only this connection's private schema
        // shadows the builtin; the normal writer path is completely unchanged.
        let schema = format!("snapshot_gate_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE FUNCTION {schema}.octet_length(text) RETURNS integer
            LANGUAGE plpgsql VOLATILE AS $$ BEGIN
                PERFORM pg_catalog.pg_advisory_xact_lock(81193042);
                RETURN pg_catalog.octet_length($1); END $$"
        )))
        .execute(&pool)
        .await
        .unwrap();
        let application = schema.clone();
        let gate_schema = schema.clone();
        let snapshot_pool = PgPoolOptions::new().max_connections(1).after_connect(move |conn, _| {
            let setup = format!("SET search_path TO {gate_schema},pg_catalog,public; SET application_name='{gate_schema}'");
            Box::pin(async move { sqlx::raw_sql(sqlx::AssertSqlSafe(setup)).execute(conn).await?; Ok(()) })
        }).connect(&crate::test_support::database_url()).await.unwrap();
        let snapshot_db = Db::from_pool(snapshot_pool);
        let mut blocker = pool.acquire().await.unwrap();
        sqlx::query("SELECT pg_advisory_lock(81193042)")
            .execute(&mut *blocker)
            .await
            .unwrap();
        let reader = snapshot_db.clone();
        let pubkey = key.public_key();
        let pending =
            tokio::spawn(async move { reader.read_state_snapshot(community, &pubkey).await });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let waiting: bool = sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity
                    WHERE application_name=$1 AND wait_event='advisory')",
                )
                .bind(&application)
                .fetch_one(&pool)
                .await
                .unwrap();
                if waiting {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("snapshot must reach the real metadata read");
        assert!(
            db.replace_parameterized_event(community, &after, &slot, None)
                .await
                .unwrap()
                .1
        );
        sqlx::query("SELECT pg_advisory_unlock(81193042)")
            .execute(&mut *blocker)
            .await
            .unwrap();
        let cut = pending.await.unwrap().unwrap();
        assert_eq!(cut.events, vec![serde_json::to_value(&before).unwrap()]);
        let next = snapshot_db
            .read_state_snapshot(community, &key.public_key())
            .await
            .unwrap();
        assert_eq!(next.events, vec![serde_json::to_value(&after).unwrap()]);
        assert_ne!(cut.snapshot_id, next.snapshot_id);
        sqlx::query(sqlx::AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn read_state_snapshot_encoded_bytes_bound_and_writer_not_replica() {
        let pool = PgPool::connect(&crate::test_support::database_url())
            .await
            .unwrap();
        let db = Db::from_pool(pool.clone());
        let community = db
            .ensure_configured_community(&format!("bytes-{}.local", uuid::Uuid::new_v4().simple()))
            .await
            .unwrap()
            .id;
        let key = Keys::generate();
        // Stored bytes fit but escaped JSON event bytes do not.
        let event = EventBuilder::new(Kind::Custom(30078), "\u{0001}".repeat(2 * 1024 * 1024))
            .tags([Tag::identifier("escape-fixture")])
            .sign_with_keys(&key)
            .unwrap();
        db.insert_event(community, &event, None).await.unwrap();
        let dead_replica = PgPoolOptions::new()
            .connect_lazy("postgres://unused@127.0.0.1:1/absent")
            .unwrap();
        dead_replica.close().await;
        let routed = Db::from_pools(pool.clone(), dead_replica);
        assert!(matches!(
            routed
                .read_state_snapshot(community, &key.public_key())
                .await,
            Err(DbError::ReadStateSnapshotTooLarge)
        ));
        sqlx::query("DELETE FROM events WHERE community_id=$1")
            .bind(community.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        let empty = routed
            .read_state_snapshot(community, &key.public_key())
            .await
            .unwrap();
        assert!(empty.events.is_empty());
        assert_eq!(empty.snapshot_id.len(), 64);
    }
}
