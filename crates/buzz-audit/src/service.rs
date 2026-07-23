use chrono::{DateTime, Utc};
use futures_util::FutureExt as _;
use sqlx::{Acquire, PgPool, Row};
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
    /// Serialized per-community via `pg_advisory_lock`. Postgres advisory locks
    /// are session-scoped, so we acquire before the transaction and release
    /// after commit (or on any error path).
    #[instrument(skip(self, entry), fields(action = %entry.action))]
    pub async fn log(&self, entry: NewAuditEntry) -> Result<AuditEntry, AuditError> {
        let mut conn = self.pool.acquire().await?;

        // Per-community advisory lock: hash the namespaced community id to an
        // i64 lock key inside Postgres. Communities lock independently.
        let lock_key = format!("{AUDIT_LOCK_NAMESPACE}{}", entry.community_id);
        sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
            .bind(&lock_key)
            .execute(&mut *conn)
            .await?;

        // Run the chain append and release the lock regardless of outcome.
        // catch_unwind so a panic still releases the lock before the connection
        // returns to the pool.
        let result = std::panic::AssertUnwindSafe(self.log_inner(&mut conn, entry))
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

    async fn log_inner(
        &self,
        conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
        entry: NewAuditEntry,
    ) -> Result<AuditEntry, AuditError> {
        let mut tx = conn.begin().await?;

        // The stored row keys on the raw UUID; the typed `CommunityId` on the
        // input is the provenance fence, dereferenced here at the DB boundary.
        let community_id = *entry.community_id.as_uuid();

        // Head of THIS community's chain — scoped by community_id.
        let head = sqlx::query(
            "SELECT seq, hash FROM audit_log
             WHERE community_id = $1
             ORDER BY seq DESC LIMIT 1",
        )
        .bind(community_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (prev_seq, prev_hash): (i64, Option<Vec<u8>>) = match head {
            Some(row) => (
                row.get::<i64, _>("seq"),
                Some(row.get::<Vec<u8>, _>("hash")),
            ),
            None => (0, None), // community's first entry
        };
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

        debug!(seq, "writing audit entry");

        sqlx::query(
            r#"
            INSERT INTO audit_log
                (community_id, seq, hash, prev_hash, action, actor_pubkey, object_id, detail, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(audit_entry.community_id)
        .bind(audit_entry.seq)
        .bind(&audit_entry.hash)
        .bind(audit_entry.prev_hash.as_deref())
        .bind(audit_entry.action.as_str())
        .bind(audit_entry.actor_pubkey.as_deref())
        .bind(audit_entry.object_id.as_deref())
        .bind(&audit_entry.detail)
        .bind(audit_entry.created_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(audit_entry)
    }

    /// Verify the hash chain for one community over `[from_seq, to_seq]`.
    ///
    /// Reads exactly that community's chain — it can never observe another
    /// community's entries or head. Returns `Ok(false)` if the range is empty,
    /// `Ok(true)` if the segment verifies.
    ///
    /// The segment is anchored on the left, so a verified range can't hide
    /// prefix truncation:
    ///
    /// - `from_seq <= 1`: the first entry must be the genesis (seq 1 with no
    ///   predecessor). Otherwise the earliest entries were deleted and the
    ///   walk fails with [`AuditError::MissingGenesis`].
    /// - `from_seq > 1`: the entry at `from_seq - 1` must exist (append-only
    ///   chains have no legitimate holes), its stored hash must match a
    ///   recomputation, and the first entry in the range must link to it.
    ///   A missing anchor fails with [`AuditError::MissingAnchor`].
    ///
    /// `to_seq` past the current head is not an error — the walk covers what
    /// is stored. Detecting *tail* truncation requires an externally recorded
    /// head; see [`AuditService::verify_full_chain`].
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

        let entries: Vec<AuditEntry> = rows
            .iter()
            .map(row_to_audit_entry)
            .collect::<Result<_, _>>()?;

        if from_seq > 1 {
            let anchor_seq = from_seq - 1;
            let Some(anchor) = self.fetch_entry(community, anchor_seq).await? else {
                return Err(AuditError::MissingAnchor { anchor_seq });
            };
            // The anchor's own stored hash must match its content — otherwise
            // a tampered anchor could vouch for the segment above it.
            let computed = compute_hash(&anchor)?;
            if computed.as_slice() != anchor.hash.as_slice() {
                return Err(AuditError::HashMismatch { seq: anchor.seq });
            }
            verify_entries(&entries, from_seq, Some(&anchor.hash))?;
        } else {
            verify_entries(&entries, 1, None)?;
        }

        Ok(true)
    }

    /// Verify one community's entire chain, genesis to head, and report what
    /// was covered.
    ///
    /// This is the operational entry point behind `buzz-admin audit verify`:
    /// it runs the [`verify_entries`] seq-contiguity pass over the whole chain
    /// in pages (genesis linkage, seq contiguity, prev-hash linkage, per-entry
    /// hash recomputation), then applies that same "seq must reach where it
    /// should" rule to the tail.
    ///
    /// Tail truncation — deleting the *newest* entries — is invisible to a
    /// walk over stored rows, because the head is defined by what is stored.
    /// Callers close that hole by recording the returned
    /// [`ChainVerification::head_seq`] / [`ChainVerification::head_hash`]
    /// externally and passing the recorded seq as `expected_head_seq` on the
    /// next run: a head behind it fails with [`AuditError::TruncatedTail`].
    ///
    /// An empty chain yields a zeroed report (nothing to verify) unless
    /// `expected_head_seq` says entries should exist.
    #[instrument(skip(self))]
    pub async fn verify_full_chain(
        &self,
        community: CommunityId,
        expected_head_seq: Option<i64>,
    ) -> Result<ChainVerification, AuditError> {
        let mut next_seq = 1i64;
        let mut anchor: Option<Vec<u8>> = None;
        let mut entries_verified = 0u64;
        let mut head_seq = 0i64;
        let mut head_hash: Option<String> = None;

        loop {
            let batch = self
                .get_entries(community, next_seq, VERIFY_PAGE_SIZE)
                .await?;
            let Some(last) = batch.last() else { break };

            verify_entries(&batch, next_seq, anchor.as_deref())?;

            entries_verified += batch.len() as u64;
            head_seq = last.seq;
            head_hash = Some(hex::encode(&last.hash));
            anchor = Some(last.hash.clone());
            next_seq = last.seq + 1;

            if (batch.len() as i64) < VERIFY_PAGE_SIZE {
                break;
            }
        }

        if let Some(expected_seq) = expected_head_seq {
            if head_seq < expected_seq {
                return Err(AuditError::TruncatedTail {
                    head_seq,
                    expected_seq,
                });
            }
        }

        Ok(ChainVerification {
            entries_verified,
            head_seq,
            head_hash,
        })
    }

    /// Fetch a single entry of one community's chain, if present.
    async fn fetch_entry(
        &self,
        community: CommunityId,
        seq: i64,
    ) -> Result<Option<AuditEntry>, AuditError> {
        let row = sqlx::query(
            r#"
            SELECT community_id, seq, hash, prev_hash, action, actor_pubkey,
                   object_id, detail, created_at
            FROM audit_log
            WHERE community_id = $1 AND seq = $2
            "#,
        )
        .bind(community.as_uuid())
        .bind(seq)
        .fetch_optional(&self.pool)
        .await?;

        row.as_ref().map(row_to_audit_entry).transpose()
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

/// Entries fetched per round-trip by [`AuditService::verify_full_chain`].
const VERIFY_PAGE_SIZE: i64 = 1000;

/// Outcome of a successful [`AuditService::verify_full_chain`] walk.
///
/// `head_seq` and `head_hash` are the external anchor: record them after each
/// verification, and pass the recorded seq as `expected_head_seq` next time so
/// tail truncation (which a stored-rows walk cannot see) becomes detectable.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChainVerification {
    /// Number of entries that were fetched and verified.
    pub entries_verified: u64,
    /// Highest sequence number verified — the chain head. `0` when the chain
    /// is empty.
    pub head_seq: i64,
    /// Hex-encoded SHA-256 of the head entry; `None` when the chain is empty.
    pub head_hash: Option<String>,
}

/// The audit chain's **seq-contiguity pass**: verify an already-fetched,
/// ordered run of one community's entries.
///
/// This is a single uniform walk that enumerates entries in `seq` order and
/// makes two assertions per entry — (1) `seq` is contiguous with the expected
/// next value and (2) `prev_hash` equals the previous entry's `hash` — then
/// recomputes the stored hash. Truncation is not a special case of this walk;
/// it *is* the walk failing:
///
/// - **prefix truncation** (earliest entries deleted) leaves the run starting
///   at the wrong `seq`, so contiguity breaks at the front;
/// - **interior deletion** leaves a hole, so contiguity breaks mid-run;
/// - **suffix truncation** (newest entries deleted) leaves the sequence ending
///   short — invisible here because the run *is* the stored rows, but caught by
///   the same "seq must reach where it should" check once
///   [`AuditService::verify_full_chain`] compares the head against an
///   externally recorded one.
///
/// So prefix, interior, and suffix truncation all fail the way tampered entries
/// do: as a break in the seq/prev_hash chain, with no per-case detection path.
/// The distinct [`AuditError`] variants ([`AuditError::MissingGenesis`],
/// [`AuditError::SequenceGap`], [`AuditError::TruncatedTail`]) are diagnostic
/// labels naming *which* edge broke — the underlying check is one pass. (Naming
/// this the seq-contiguity pass, and the observation that both truncation ends
/// surface as seq gaps, are due to @SaravananJaichandar on #2620.)
///
/// Shared by [`AuditService::verify_chain`] and
/// [`AuditService::verify_full_chain`], and usable directly by callers that
/// obtained entries through another read path.
///
/// The left edge of the run is explicit, so a valid-looking interior can't
/// hide a truncated front:
///
/// - `anchor_hash` is `None` for a run that must start at the genesis:
///   `expected_first_seq` should be `1` and the first entry must be seq 1
///   with no `prev_hash` ([`AuditError::MissingGenesis`] /
///   [`AuditError::ChainViolation`] otherwise).
/// - `anchor_hash` is `Some(hash)` for a run anchored to the stored entry at
///   `expected_first_seq - 1`; the first entry must link to that hash.
///
/// Every entry must continue the sequence contiguously
/// ([`AuditError::SequenceGap`]), link to its predecessor's hash
/// ([`AuditError::ChainViolation`]), and hash to its stored value
/// ([`AuditError::HashMismatch`]). An empty run verifies vacuously.
pub fn verify_entries(
    entries: &[AuditEntry],
    expected_first_seq: i64,
    anchor_hash: Option<&[u8]>,
) -> Result<(), AuditError> {
    let mut expected_prev: Option<Vec<u8>> = anchor_hash.map(<[u8]>::to_vec);

    for (expected_seq, entry) in (expected_first_seq..).zip(entries.iter()) {
        // Assertion 1 — seq contiguity. A break at the genesis position is
        // prefix truncation; a break anywhere else is an interior gap. Both
        // are the same missing-seq failure, split only for a clearer message.
        if entry.seq != expected_seq {
            if expected_seq == 1 && expected_prev.is_none() {
                return Err(AuditError::MissingGenesis {
                    found_seq: entry.seq,
                });
            }
            return Err(AuditError::SequenceGap {
                expected_seq,
                found_seq: entry.seq,
            });
        }

        // Assertion 2 — prev-hash linkage: this entry's prev_hash must equal
        // the previous entry's hash (the anchor for the first entry, or nothing
        // at all for the genesis).
        let linked = match (&expected_prev, &entry.prev_hash) {
            // Genesis: the first entry of a chain has no predecessor.
            (None, None) => true,
            // Interior: prev_hash must equal the preceding entry's hash.
            (Some(want), Some(got)) => want == got,
            // Genesis with a predecessor, or interior without one.
            _ => false,
        };
        if !linked {
            return Err(AuditError::ChainViolation { seq: entry.seq });
        }

        // Independent of the chain links, every stored hash must match a
        // recomputation over the entry's own fields.
        let computed = compute_hash(entry)?;
        if computed.as_slice() != entry.hash.as_slice() {
            return Err(AuditError::HashMismatch { seq: entry.seq });
        }

        expected_prev = Some(entry.hash.clone());
    }

    Ok(())
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

    // ---- pure chain-walk tests (no database) --------------------------------

    /// Build a valid in-memory chain of `n` entries for one community.
    fn build_chain(n: i64) -> Vec<AuditEntry> {
        let community_id = Uuid::from_u128(0xC0FFEE);
        let created_at = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut prev: Option<Vec<u8>> = None;
        (1..=n)
            .map(|seq| {
                let mut e = AuditEntry {
                    community_id,
                    seq,
                    hash: Vec::new(),
                    prev_hash: prev.clone(),
                    action: AuditAction::EventCreated,
                    actor_pubkey: Some(vec![0xab; 32]),
                    object_id: Some(format!("obj-{seq}")),
                    detail: serde_json::json!({ "seq": seq }),
                    created_at,
                };
                e.hash = compute_hash(&e).unwrap().to_vec();
                prev = Some(e.hash.clone());
                e
            })
            .collect()
    }

    #[test]
    fn verify_entries_accepts_valid_genesis_run() {
        let chain = build_chain(5);
        assert!(verify_entries(&chain, 1, None).is_ok());
        // Empty runs verify vacuously.
        assert!(verify_entries(&[], 1, None).is_ok());
    }

    #[test]
    fn verify_entries_detects_prefix_truncation() {
        // Prefix truncation, the front edge of the seq-contiguity pass. Delete
        // the genesis: the remaining interior still self-links, which is
        // exactly why the old walk accepted it. Now the run starts at seq 2
        // where seq 1 was expected, so contiguity breaks at the front.
        let chain = build_chain(5);
        let r = verify_entries(&chain[1..], 1, None);
        assert!(matches!(
            r,
            Err(AuditError::MissingGenesis { found_seq: 2 })
        ));
    }

    #[test]
    fn verify_entries_rejects_regrafted_genesis() {
        // A seq-1 entry whose prev_hash is non-null (chain re-rooted onto a
        // fabricated predecessor) — hash is honestly recomputed, so only the
        // genesis linkage rule can catch it.
        let mut chain = build_chain(3);
        chain[0].prev_hash = Some(vec![0xee; 32]);
        chain[0].hash = compute_hash(&chain[0]).unwrap().to_vec();
        let r = verify_entries(&chain[..1], 1, None);
        assert!(matches!(r, Err(AuditError::ChainViolation { seq: 1 })));
    }

    #[test]
    fn verify_entries_detects_interior_gap() {
        let mut chain = build_chain(5);
        chain.remove(2); // drop seq 3
        let r = verify_entries(&chain, 1, None);
        assert!(matches!(
            r,
            Err(AuditError::SequenceGap {
                expected_seq: 3,
                found_seq: 4
            })
        ));
    }

    #[test]
    fn verify_entries_detects_tampering() {
        // Mutated content without a recomputed hash → HashMismatch there.
        let mut chain = build_chain(3);
        chain[1].object_id = Some("forged".into());
        let r = verify_entries(&chain, 1, None);
        assert!(matches!(r, Err(AuditError::HashMismatch { seq: 2 })));

        // Mutated content *with* a recomputed hash → the next link breaks.
        let mut chain = build_chain(3);
        chain[1].object_id = Some("forged".into());
        chain[1].hash = compute_hash(&chain[1]).unwrap().to_vec();
        let r = verify_entries(&chain, 1, None);
        assert!(matches!(r, Err(AuditError::ChainViolation { seq: 3 })));
    }

    #[test]
    fn verify_entries_anchored_run_checks_left_edge() {
        let chain = build_chain(5);

        // Correctly anchored mid-segment verifies.
        assert!(verify_entries(&chain[2..], 3, Some(&chain[1].hash)).is_ok());

        // A wrong anchor hash is a broken link at the first entry.
        let r = verify_entries(&chain[2..], 3, Some(&[0xff; 32]));
        assert!(matches!(r, Err(AuditError::ChainViolation { seq: 3 })));

        // A segment that starts past its declared anchor point is a gap.
        let r = verify_entries(&chain[3..], 3, Some(&chain[1].hash));
        assert!(matches!(
            r,
            Err(AuditError::SequenceGap {
                expected_seq: 3,
                found_seq: 4
            })
        ));
    }

    #[test]
    fn verify_entries_suffix_truncation_needs_the_recorded_head() {
        // Suffix truncation, the back edge of the seq-contiguity pass. Unlike
        // the front and interior edges, a deleted tail leaves a shorter but
        // still-contiguous run, so verify_entries alone verifies it clean —
        // the run *is* the stored rows, and nothing in them says how far the
        // seq should have reached.
        let chain = build_chain(5);
        assert!(verify_entries(&chain[..4], 1, None).is_ok()); // seq 5 deleted

        // Closing the back edge therefore needs an externally recorded head to
        // compare against: that comparison lives in verify_full_chain and is
        // exercised end-to-end (record head, delete tail, re-verify against the
        // recorded seq -> TruncatedTail) by full_chain_report_and_tail_truncation.
    }

    // ---- truncation detection against the database --------------------------

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn verify_detects_deleted_genesis_prefix() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;
        for action in [
            AuditAction::EventCreated,
            AuditAction::ChannelCreated,
            AuditAction::MemberAdded,
        ] {
            svc.log(new_entry(c, action)).await.unwrap();
        }

        sqlx::query("DELETE FROM audit_log WHERE community_id = $1 AND seq = 1")
            .bind(c)
            .execute(&pool)
            .await
            .unwrap();

        let r = svc.verify_chain(CommunityId::from_uuid(c), 1, 3).await;
        assert!(matches!(
            r,
            Err(AuditError::MissingGenesis { found_seq: 2 })
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn verify_anchors_mid_chain_segments() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;
        for action in [
            AuditAction::EventCreated,
            AuditAction::ChannelCreated,
            AuditAction::MemberAdded,
        ] {
            svc.log(new_entry(c, action)).await.unwrap();
        }

        // A mid-chain segment verifies while its anchor entry exists…
        assert!(svc
            .verify_chain(CommunityId::from_uuid(c), 2, 3)
            .await
            .unwrap());

        // …and fails once the entry in front of it is gone.
        sqlx::query("DELETE FROM audit_log WHERE community_id = $1 AND seq = 1")
            .bind(c)
            .execute(&pool)
            .await
            .unwrap();
        let r = svc.verify_chain(CommunityId::from_uuid(c), 2, 3).await;
        assert!(matches!(
            r,
            Err(AuditError::MissingAnchor { anchor_seq: 1 })
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn verify_detects_interior_deletion() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;
        for action in [
            AuditAction::EventCreated,
            AuditAction::ChannelCreated,
            AuditAction::MemberAdded,
        ] {
            svc.log(new_entry(c, action)).await.unwrap();
        }

        sqlx::query("DELETE FROM audit_log WHERE community_id = $1 AND seq = 2")
            .bind(c)
            .execute(&pool)
            .await
            .unwrap();

        let r = svc.verify_chain(CommunityId::from_uuid(c), 1, 3).await;
        assert!(matches!(
            r,
            Err(AuditError::SequenceGap {
                expected_seq: 2,
                found_seq: 3
            })
        ));
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn full_chain_report_and_tail_truncation() {
        let _g = db_lock().lock().await;
        let Some(pool) = test_pool().await else {
            return;
        };
        let svc = AuditService::new(pool.clone());
        let c = make_community(&pool).await;
        let mut last_hash = Vec::new();
        for action in [
            AuditAction::EventCreated,
            AuditAction::ChannelCreated,
            AuditAction::MemberAdded,
        ] {
            last_hash = svc.log(new_entry(c, action)).await.unwrap().hash;
        }

        // The report covers the whole chain and exposes the head anchor.
        let report = svc
            .verify_full_chain(CommunityId::from_uuid(c), None)
            .await
            .unwrap();
        assert_eq!(report.entries_verified, 3);
        assert_eq!(report.head_seq, 3);
        assert_eq!(
            report.head_hash.as_deref(),
            Some(hex::encode(&last_hash).as_str())
        );

        // With the recorded head, the same chain still verifies.
        assert!(svc
            .verify_full_chain(CommunityId::from_uuid(c), Some(3))
            .await
            .is_ok());

        // Delete the newest entry. Without an external anchor the shortened
        // chain still verifies — the head is defined by what is stored — which
        // is exactly why the recorded head must be passed back in.
        sqlx::query("DELETE FROM audit_log WHERE community_id = $1 AND seq = 3")
            .bind(c)
            .execute(&pool)
            .await
            .unwrap();
        let shortened = svc
            .verify_full_chain(CommunityId::from_uuid(c), None)
            .await
            .unwrap();
        assert_eq!(shortened.head_seq, 2);

        let r = svc
            .verify_full_chain(CommunityId::from_uuid(c), Some(3))
            .await;
        assert!(matches!(
            r,
            Err(AuditError::TruncatedTail {
                head_seq: 2,
                expected_seq: 3
            })
        ));
    }
}
