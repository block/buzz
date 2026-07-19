use chrono::{DateTime, Utc};
use futures_util::FutureExt as _;
use sqlx::{Acquire, PgPool, QueryBuilder, Row};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use buzz_core::CommunityId;

use crate::{
    action::AuditAction,
    entry::{AuditEntry, NewAuditEntry},
    error::AuditError,
    hash::compute_hash,
};

/// Per-community advisory lock key. Derived in Postgres from the community UUID
/// so two communities never serialize each other's audit writes (which would be
/// both a throughput bottleneck and a cross-tenant timing oracle). The lock is
/// taken with `pg_advisory_lock(hashtextextended(...))` — see [`AuditService::log`].
const AUDIT_LOCK_NAMESPACE: &str = "buzz_audit:";

/// Append-only, per-community hash-chain audit log backed by Postgres.
///
/// Each community has an independent chain keyed `(community_id, seq)`. Writes
/// for one community are serialized by a per-community advisory lock so the chain
/// stays consistent across relay processes; different communities proceed in
/// parallel.
pub struct AuditService {
    pool: PgPool,
}

impl AuditService {
    /// Creates a new `AuditService` using the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Append a new entry to the calling community's chain.
    ///
    /// Delegates to [`Self::log_batch`] with a batch of one so there is exactly
    /// one chain-append implementation (lock protocol, head read, hash linkage).
    #[instrument(skip(self, entry), fields(action = %entry.action))]
    pub async fn log(&self, entry: NewAuditEntry) -> Result<AuditEntry, AuditError> {
        let mut logged = self.log_batch(vec![entry]).await?;
        Ok(logged
            .pop()
            .expect("log_batch of one entry returns one entry"))
    }

    /// Append a batch of entries — all for the **same community** — to that
    /// community's chain in one transaction.
    ///
    /// Contract:
    /// - Every entry must share one `community_id`; a mixed batch is rejected
    ///   with [`AuditError::MixedBatch`] before any lock is taken (callers
    ///   partition per community — batches never span tenants, so one
    ///   community's failure can never roll back another's rows).
    /// - Input order is preserved: entries are chained and inserted in the
    ///   order given.
    /// - Atomic: either every entry in the batch commits or none does.
    ///
    /// Serialized per-community via `pg_advisory_lock` (the same key domain as
    /// [`Self::log`]). Postgres advisory locks are session-scoped, so we
    /// acquire before the transaction and release after commit (or on any
    /// error path). The chain head is re-read from the DB inside the
    /// transaction, under the lock — there is no in-memory head authority, so
    /// this is safe across relay replicas.
    #[instrument(skip(self, entries), fields(batch_len = entries.len()))]
    pub async fn log_batch(
        &self,
        entries: Vec<NewAuditEntry>,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        let Some(first) = entries.first() else {
            return Ok(Vec::new());
        };
        let community = first.community_id;
        if entries.iter().any(|e| e.community_id != community) {
            return Err(AuditError::MixedBatch);
        }

        let mut conn = self.pool.acquire().await?;

        // Per-community advisory lock: hash the namespaced community id to an
        // i64 lock key inside Postgres. Communities lock independently.
        let lock_key = format!("{AUDIT_LOCK_NAMESPACE}{community}");
        sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
            .bind(&lock_key)
            .execute(&mut *conn)
            .await?;

        // Run the chain append and release the lock regardless of outcome.
        // catch_unwind so a panic still releases the lock before the connection
        // returns to the pool.
        let result = std::panic::AssertUnwindSafe(self.log_batch_inner(&mut conn, entries))
            .catch_unwind()
            .await;

        let _ = sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
            .bind(&lock_key)
            .execute(&mut *conn)
            .await;

        match result {
            Ok(inner_result) => inner_result,
            Err(panic_payload) => std::panic::resume_unwind(panic_payload),
        }
    }

    async fn log_batch_inner(
        &self,
        conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
        entries: Vec<NewAuditEntry>,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        let mut tx = conn.begin().await?;

        // The stored rows key on the raw UUID; the typed `CommunityId` on the
        // input is the provenance fence, dereferenced here at the DB boundary.
        // `log_batch` verified all entries share this community.
        let community_id = *entries[0].community_id.as_uuid();

        // Head of THIS community's chain — scoped by community_id.
        let head = sqlx::query(
            "SELECT seq, hash FROM audit_log
             WHERE community_id = $1
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(community_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (mut prev_seq, mut prev_hash): (i64, Option<Vec<u8>>) = match head {
            Some(row) => (
                row.get::<i64, _>("seq"),
                Some(row.get::<Vec<u8>, _>("hash")),
            ),
            None => (0, None), // community's first entry
        };

        // Chain the batch in input order: each entry links to its predecessor
        // (the last committed row for the first entry, the previous batch
        // member thereafter).
        let mut logged = Vec::with_capacity(entries.len());
        for entry in entries {
            let seq = prev_seq + 1;
            let created_at: DateTime<Utc> = Utc::now();

            let mut audit_entry = AuditEntry {
                community_id,
                seq,
                hash: Vec::new(),
                prev_hash,
                action: entry.action,
                actor_pubkey: entry.actor_pubkey,
                object_id: entry.object_id,
                detail: entry.detail,
                created_at,
            };
            audit_entry.hash = compute_hash(&audit_entry)?.to_vec();

            prev_seq = seq;
            prev_hash = Some(audit_entry.hash.clone());
            logged.push(audit_entry);
        }

        debug!(
            first_seq = logged.first().map(|e| e.seq),
            last_seq = logged.last().map(|e| e.seq),
            "writing audit entries"
        );

        // One multi-row INSERT for the whole batch.
        let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
            "INSERT INTO audit_log
                (community_id, seq, hash, prev_hash, action, actor_pubkey, object_id, detail, created_at) ",
        );
        qb.push_values(logged.iter(), |mut b, e| {
            b.push_bind(e.community_id)
                .push_bind(e.seq)
                .push_bind(&e.hash)
                .push_bind(e.prev_hash.as_deref())
                .push_bind(e.action.as_str())
                .push_bind(e.actor_pubkey.as_deref())
                .push_bind(e.object_id.as_deref())
                .push_bind(&e.detail)
                .push_bind(e.created_at);
        });
        qb.build().execute(&mut *tx).await?;

        tx.commit().await?;

        Ok(logged)
    }

    /// Verify the hash chain for one community over `[from_seq, to_seq]`.
    ///
    /// Reads exactly that community's chain — it can never observe another
    /// community's entries or head. Returns `Ok(false)` if the range is empty,
    /// `Ok(true)` if the segment is internally consistent.
    #[instrument(skip(self))]
    pub async fn verify_chain(
        &self,
        community: CommunityId,
        from_seq: i64,
        to_seq: i64,
    ) -> Result<bool, AuditError> {
        let rows = sqlx::query(
            r#"
            SELECT community_id, seq, hash, prev_hash, action, actor_pubkey,
                   object_id, detail, created_at
            FROM audit_log
            WHERE community_id = $1 AND seq BETWEEN $2 AND $3
            ORDER BY seq ASC
            "#,
        )
        .bind(community.as_uuid())
        .bind(from_seq)
        .bind(to_seq)
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(false);
        }

        let mut expected_prev: Option<Vec<u8>> = None;

        for row in &rows {
            let entry = row_to_audit_entry(row)?;

            if let Some(ref expected) = expected_prev {
                // The previous entry's hash must equal this entry's prev_hash.
                if entry.prev_hash.as_deref() != Some(expected.as_slice()) {
                    return Err(AuditError::ChainViolation { seq: entry.seq });
                }
            }

            let computed = compute_hash(&entry)?;
            if computed.as_slice() != entry.hash.as_slice() {
                return Err(AuditError::HashMismatch { seq: entry.seq });
            }

            expected_prev = Some(entry.hash);
        }

        Ok(true)
    }

    /// Returns up to `limit` entries from one community's chain starting at
    /// `from_seq`, ordered by sequence number. Scoped to `community` — never
    /// returns another community's rows.
    #[instrument(skip(self))]
    pub async fn get_entries(
        &self,
        community: CommunityId,
        from_seq: i64,
        limit: i64,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        let rows = sqlx::query(
            r#"
            SELECT community_id, seq, hash, prev_hash, action, actor_pubkey,
                   object_id, detail, created_at
            FROM audit_log
            WHERE community_id = $1 AND seq >= $2
            ORDER BY seq ASC
            LIMIT $3
            "#,
        )
        .bind(community.as_uuid())
        .bind(from_seq)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(row_to_audit_entry).collect()
    }
}

fn row_to_audit_entry(row: &sqlx::postgres::PgRow) -> Result<AuditEntry, AuditError> {
    let action_str: String = row.get("action");
    let action: AuditAction = action_str.parse().map_err(|_| {
        warn!("unknown action in audit log");
        AuditError::UnknownAction
    })?;

    Ok(AuditEntry {
        community_id: row.get::<Uuid, _>("community_id"),
        seq: row.get("seq"),
        hash: row.get("hash"),
        prev_hash: row.get("prev_hash"),
        action,
        actor_pubkey: row.get("actor_pubkey"),
        object_id: row.get("object_id"),
        detail: row.get("detail"),
        created_at: row.get("created_at"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::AuditAction;
    use crate::entry::NewAuditEntry;
    use std::sync::OnceLock;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    // The per-community advisory lock means different communities don't contend,
    // but tests share one table; serialize them so seq assertions are stable.
    static DB_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn db_lock() -> &'static Mutex<()> {
        DB_LOCK.get_or_init(|| Mutex::new(()))
    }

    async fn test_pool() -> Option<PgPool> {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".into());
        PgPool::connect(&url).await.ok()
    }

    /// A `community_id` known to exist in `communities` (FK target). Inserts a
    /// throwaway community row with a unique host and returns its id.
    async fn make_community(pool: &PgPool) -> Uuid {
        let id = Uuid::new_v4();
        let host = format!("test-{id}.example");
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(host)
            .execute(pool)
            .await
            .expect("insert test community");
        id
    }

    fn new_entry(community_id: Uuid, action: AuditAction) -> NewAuditEntry {
        NewAuditEntry {
            community_id: CommunityId::from_uuid(community_id),
            action,
            actor_pubkey: Some(vec![0xab; 32]),
            object_id: Some(format!("obj_{}", Uuid::new_v4())),
            detail: serde_json::json!({"test": true}),
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn community_chain_starts_at_seq_1_with_null_prev() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;

        let e = svc
            .log(new_entry(c, AuditAction::EventCreated))
            .await
            .unwrap();
        assert_eq!(e.seq, 1, "first entry in a community starts at seq 1");
        assert!(e.prev_hash.is_none(), "genesis entry has NULL prev_hash");
        assert_eq!(e.hash.len(), 32);
        assert_eq!(e.community_id, c);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn chain_links_within_one_community() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;

        let e1 = svc
            .log(new_entry(c, AuditAction::EventCreated))
            .await
            .unwrap();
        let e2 = svc
            .log(new_entry(c, AuditAction::ChannelCreated))
            .await
            .unwrap();
        let e3 = svc
            .log(new_entry(c, AuditAction::MemberAdded))
            .await
            .unwrap();

        assert_eq!(e1.seq, 1);
        assert_eq!(e2.seq, 2);
        assert_eq!(e3.seq, 3);
        assert!(e1.prev_hash.is_none());
        assert_eq!(e2.prev_hash.as_deref(), Some(e1.hash.as_slice()));
        assert_eq!(e3.prev_hash.as_deref(), Some(e2.hash.as_slice()));
        assert!(svc
            .verify_chain(CommunityId::from_uuid(c), 1, 3)
            .await
            .unwrap());
    }

    /// THE isolation property: two communities keep independent chains. Each
    /// starts at seq 1; interleaving writes does not link them; verifying one
    /// never traverses the other.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn chains_are_independent_per_community() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let a = make_community(&pool).await;
        let b = make_community(&pool).await;

        // Interleave A and B writes.
        let a1 = svc
            .log(new_entry(a, AuditAction::EventCreated))
            .await
            .unwrap();
        let b1 = svc
            .log(new_entry(b, AuditAction::EventCreated))
            .await
            .unwrap();
        let a2 = svc
            .log(new_entry(a, AuditAction::ChannelCreated))
            .await
            .unwrap();
        let b2 = svc
            .log(new_entry(b, AuditAction::ChannelCreated))
            .await
            .unwrap();

        // Each community's seq is independent and starts at 1.
        assert_eq!((a1.seq, a2.seq), (1, 2));
        assert_eq!((b1.seq, b2.seq), (1, 2));

        // A's chain links only within A; B's only within B. A2 must NOT chain to
        // B1 even though B1 was written between A1 and A2.
        assert_eq!(a2.prev_hash.as_deref(), Some(a1.hash.as_slice()));
        assert_eq!(b2.prev_hash.as_deref(), Some(b1.hash.as_slice()));
        assert_ne!(a2.prev_hash, b1.prev_hash);

        // Verifying A's chain traverses only A; same for B.
        assert!(svc
            .verify_chain(CommunityId::from_uuid(a), 1, 2)
            .await
            .unwrap());
        assert!(svc
            .verify_chain(CommunityId::from_uuid(b), 1, 2)
            .await
            .unwrap());

        // get_entries scoped to A returns only A's rows.
        let a_rows = svc
            .get_entries(CommunityId::from_uuid(a), 1, 100)
            .await
            .unwrap();
        assert!(
            a_rows.iter().all(|e| e.community_id == a),
            "A read leaked another community"
        );
        assert_eq!(a_rows.len(), 2);
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn verify_detects_tampering_within_a_community() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;

        svc.log(new_entry(c, AuditAction::EventCreated))
            .await
            .unwrap();
        let e2 = svc
            .log(new_entry(c, AuditAction::EventDeleted))
            .await
            .unwrap();
        svc.log(new_entry(c, AuditAction::ChannelDeleted))
            .await
            .unwrap();

        // Tamper with e2's stored actor_pubkey.
        let tampered: Vec<u8> = vec![0xff; 32];
        sqlx::query("UPDATE audit_log SET actor_pubkey = $1 WHERE community_id = $2 AND seq = $3")
            .bind(tampered)
            .bind(c)
            .bind(e2.seq)
            .execute(&pool)
            .await
            .unwrap();

        let r = svc.verify_chain(CommunityId::from_uuid(c), 1, 3).await;
        assert!(matches!(r, Err(AuditError::HashMismatch { seq }) if seq == e2.seq));
    }

    /// A row forged with another community's id cannot pass verification against
    /// the chain it was stamped for, because community_id is hashed in. (Models
    /// "a row can't be replayed across chains and still verify".)
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn cross_community_row_does_not_verify() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let a = make_community(&pool).await;
        let b = make_community(&pool).await;

        let a1 = svc
            .log(new_entry(a, AuditAction::EventCreated))
            .await
            .unwrap();

        // Forge: copy A's seq-1 row's hash into B's chain at seq 1.
        sqlx::query(
            "INSERT INTO audit_log (community_id, seq, hash, prev_hash, action, actor_pubkey, object_id, detail, created_at)
             VALUES ($1, 1, $2, NULL, $3, $4, $5, $6, NOW())",
        )
        .bind(b)
        .bind(&a1.hash) // A's hash, which was computed over community_id = A
        .bind(a1.action.as_str())
        .bind(a1.actor_pubkey.as_deref())
        .bind(a1.object_id.as_deref())
        .bind(&a1.detail)
        .execute(&pool)
        .await
        .unwrap();

        // Verifying B's chain recomputes the hash with community_id = B, which
        // won't match A's stored hash → HashMismatch. The forge is rejected.
        let r = svc.verify_chain(CommunityId::from_uuid(b), 1, 1).await;
        assert!(matches!(r, Err(AuditError::HashMismatch { seq: 1 })));
    }

    /// Batched appends chain exactly like sequential singles: within the
    /// batch each entry links to its predecessor, the first links to the
    /// pre-batch head, and the whole chain verifies.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn batch_chains_in_order_and_links_to_prior_head() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;

        // Pre-batch head via the single-entry path.
        let e1 = svc
            .log(new_entry(c, AuditAction::EventCreated))
            .await
            .unwrap();

        let logged = svc
            .log_batch(vec![
                new_entry(c, AuditAction::ChannelCreated),
                new_entry(c, AuditAction::MemberAdded),
                new_entry(c, AuditAction::EventDeleted),
            ])
            .await
            .unwrap();

        assert_eq!(logged.len(), 3);
        assert_eq!(logged[0].seq, 2);
        assert_eq!(logged[1].seq, 3);
        assert_eq!(logged[2].seq, 4);
        assert_eq!(logged[0].prev_hash.as_deref(), Some(e1.hash.as_slice()));
        assert_eq!(
            logged[1].prev_hash.as_deref(),
            Some(logged[0].hash.as_slice())
        );
        assert_eq!(
            logged[2].prev_hash.as_deref(),
            Some(logged[1].hash.as_slice())
        );
        assert!(svc
            .verify_chain(CommunityId::from_uuid(c), 1, 4)
            .await
            .unwrap());

        // And a single append after the batch links to the batch's tail.
        let e5 = svc
            .log(new_entry(c, AuditAction::ChannelDeleted))
            .await
            .unwrap();
        assert_eq!(e5.seq, 5);
        assert_eq!(e5.prev_hash.as_deref(), Some(logged[2].hash.as_slice()));
    }

    /// A mixed-community batch is rejected before any write; the error is
    /// classified deterministic (retrying identical input cannot succeed).
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn mixed_batch_is_rejected_without_writing() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let a = make_community(&pool).await;
        let b = make_community(&pool).await;

        let err = svc
            .log_batch(vec![
                new_entry(a, AuditAction::EventCreated),
                new_entry(b, AuditAction::EventCreated),
            ])
            .await
            .unwrap_err();
        assert!(matches!(err, AuditError::MixedBatch));
        assert!(err.is_deterministic());

        for community in [a, b] {
            let rows = svc
                .get_entries(CommunityId::from_uuid(community), 1, 10)
                .await
                .unwrap();
            assert!(rows.is_empty(), "mixed batch must write nothing");
        }
    }

    #[tokio::test]
    async fn empty_batch_is_a_noop() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool);
        assert!(svc.log_batch(Vec::new()).await.unwrap().is_empty());
    }

    /// Atomicity: a batch containing one entry Postgres rejects (jsonb cannot
    /// store \u0000 — SQLSTATE 22P05) commits nothing, and the error is
    /// classified deterministic so the caller bisects instead of retrying.
    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn poisoned_batch_is_atomic_and_deterministic() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;

        let mut poison = new_entry(c, AuditAction::MemberAdded);
        poison.detail = serde_json::json!({"bad": "nul\u{0}byte"});

        let err = svc
            .log_batch(vec![
                new_entry(c, AuditAction::EventCreated),
                poison,
                new_entry(c, AuditAction::ChannelCreated),
            ])
            .await
            .unwrap_err();
        assert!(
            err.is_deterministic(),
            "jsonb NUL rejection must classify deterministic, got: {err}"
        );

        let rows = svc
            .get_entries(CommunityId::from_uuid(c), 1, 10)
            .await
            .unwrap();
        assert!(rows.is_empty(), "failed batch must roll back atomically");
    }

    /// Classification unit checks that need no Postgres: infra-shaped sqlx
    /// errors must NOT be deterministic (outages are retried, not bisected).
    #[test]
    fn infra_errors_are_not_deterministic() {
        assert!(!AuditError::Database(sqlx::Error::PoolTimedOut).is_deterministic());
        assert!(!AuditError::Database(sqlx::Error::WorkerCrashed).is_deterministic());
        let ser = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        assert!(AuditError::Serialization(ser).is_deterministic());
        assert!(AuditError::MixedBatch.is_deterministic());
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn verify_empty_range_is_false() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;
        // No entries for this fresh community.
        assert!(!svc
            .verify_chain(CommunityId::from_uuid(c), 1, 100)
            .await
            .unwrap());
    }
}
