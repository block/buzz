//! Use-limited relay invite persistence (v2 opaque tokens).
//!
//! Unlike the stateless v1 HMAC invite tokens in `buzz-relay::invite_token`,
//! v2 invites are backed by durable rows in `relay_invites`. The table stores
//! only `SHA-256(code)` — never the reusable bearer secret — so a leaked
//! database does not immediately yield valid invite codes.
//!
//! Every lookup binds both `(community_id, token_hash)` to prevent cross-tenant
//! authorization seams: a code minted on tenant A presented to tenant B returns
//! `Invalid`, not a membership.
//!
//! ## Atomic redemption
//!
//! `claim_relay_invite` executes the full redemption in one PostgreSQL
//! transaction: `SELECT FOR UPDATE` on the invite row, membership insert,
//! join-policy evidence insert, and `use_count` increment all commit together.
//! `FOR UPDATE` serializes concurrent claims for one invite across relay
//! processes — exactly one claimant can win the final slot.

use buzz_core::invite::{
    encode_v2_code, hash_v2_code, MAX_INVITE_TTL_SECS, MAX_INVITE_USES, MIN_INVITE_TTL_SECS,
    V2_SECRET_LEN,
};
use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row as _};

use crate::error::Result;
use crate::{CommunityId, Db};

/// Outcome of a v2 invite claim. Expected invalid/expired/exhausted states are
/// typed variants so the relay layer can map them to distinct HTTP responses
/// without inspecting database errors.
#[derive(Debug, PartialEq)]
pub enum ClaimOutcome {
    /// A new relay member was inserted. `use_count` is the post-increment count;
    /// `uses_remaining` is `None` for unlimited invites.
    Joined {
        /// Post-claim use count.
        use_count: i32,
        /// Remaining slots, or `None` when the invite is unlimited.
        uses_remaining: Option<i32>,
    },
    /// The claimer was already a member. `use_count` was NOT incremented.
    AlreadyMember {
        /// Current use count (unchanged by this claim).
        use_count: i32,
        /// Remaining slots, or `None` when the invite is unlimited.
        uses_remaining: Option<i32>,
    },
    /// The invite's `expires_at` has passed.
    Expired,
    /// The invite's use budget is fully consumed.
    Exhausted,
    /// No invite row matches `(community_id, token_hash)`.
    Invalid,
}

/// A freshly minted v2 invite, including the plaintext code and metadata.
#[derive(Debug)]
pub struct MintedInvite {
    /// The full v2 code string (`v2.<base64url secret>`). Returned to the caller
    /// exactly once; the database stores only the SHA-256 hash.
    pub code: String,
    /// When the invite expires (UTC).
    pub expires_at: DateTime<Utc>,
    /// `None` means unlimited; `Some(n)` means at most `n` uses.
    pub max_uses: Option<i32>,
    /// Remaining uses at mint time (equals `max_uses` when bounded, `None`
    /// when unlimited).
    pub uses_remaining: Option<i32>,
    /// The invite's database-generated UUID.
    pub invite_id: uuid::Uuid,
}

fn validate_mint_inputs(ttl_secs: u64, max_uses: Option<i32>) -> Result<()> {
    if !(MIN_INVITE_TTL_SECS..=MAX_INVITE_TTL_SECS).contains(&ttl_secs) {
        return Err(crate::error::DbError::InvalidData(format!(
            "ttl_secs must be between {MIN_INVITE_TTL_SECS} and {MAX_INVITE_TTL_SECS}"
        )));
    }

    if let Some(max_uses) = max_uses {
        if !(1..=MAX_INVITE_USES).contains(&max_uses) {
            return Err(crate::error::DbError::InvalidData(format!(
                "max_uses must be between 1 and {MAX_INVITE_USES}"
            )));
        }
    }

    Ok(())
}

/// Mint a v2 invite: generate a 32-byte random secret, hash it, persist the
/// row, and return the plaintext code plus metadata.
///
/// `ttl_secs` must be in the shared invite lifetime range.
/// `max_uses` must be `None` (unlimited) or `Some(1..=10000)`.
pub async fn mint_relay_invite(
    pool: &PgPool,
    community: CommunityId,
    created_by: &str,
    ttl_secs: u64,
    max_uses: Option<i32>,
) -> Result<MintedInvite> {
    validate_mint_inputs(ttl_secs, max_uses)?;

    // Generate 32 random bytes and encode as base64url — this is the secret.
    let secret: [u8; V2_SECRET_LEN] = rand::random();
    let code = encode_v2_code(&secret);
    let token_hash = hash_v2_code(&code);
    let now = Utc::now();
    let expires_at = now + chrono::Duration::seconds(ttl_secs as i64);

    // Mint a v2 opaque invite inside the same lifecycle gate as every other
    // community-scoped database write. The trigger remains the final backstop,
    // but this typed guard keeps a quiescing community from surfacing as an
    // opaque SQLSTATE/HTTP 500 at the API boundary.
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;
    crate::deletion::DeletionStore::new(pool.clone())
        .guard_transaction(&mut tx, community)
        .await?;
    let row = sqlx::query(
        "INSERT INTO relay_invites (community_id, token_hash, max_uses, expires_at, created_by) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING id",
    )
    .bind(community.as_uuid())
    .bind(token_hash.as_slice())
    .bind(max_uses)
    .bind(expires_at)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    let invite_id: uuid::Uuid = row.try_get("id")?;

    Ok(MintedInvite {
        code,
        expires_at,
        max_uses,
        uses_remaining: max_uses,
        invite_id,
    })
}

fn log_claim_outcome(
    community: CommunityId,
    invite_id: Option<uuid::Uuid>,
    outcome: &'static str,
    max_uses: Option<i32>,
    use_count: Option<i32>,
) {
    tracing::info!(
        community = %community,
        invite_id = ?invite_id,
        outcome,
        max_uses = ?max_uses,
        use_count = ?use_count,
        "relay invite claim completed"
    );
}

/// Maximum rows deleted by one retention sweep so cleanup cannot monopolize
/// the invite table on a busy deployment.
const RETENTION_SWEEP_BATCH_SIZE: i64 = 1_000;

/// Delete one bounded batch of invite rows expired before `cutoff`.
///
/// The relay calls this from its leader-only periodic tick. Ordering by the
/// expiry index makes old rows drain first without turning cleanup into an
/// unbounded transaction.
pub async fn reap_expired_relay_invites(pool: &PgPool, cutoff: DateTime<Utc>) -> Result<u64> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Maintenance,
    )
    .await?;
    let result = sqlx::query(
        "DELETE FROM relay_invites \
         WHERE (community_id, id) IN (\
             SELECT community_id, id FROM relay_invites \
             WHERE expires_at < $1 \
               AND community_write_allowed(community_id) \
             ORDER BY expires_at \
             LIMIT $2\
         )",
    )
    .bind(cutoff)
    .bind(RETENTION_SWEEP_BATCH_SIZE)
    .execute(&mut *connection)
    .await?;

    Ok(result.rows_affected())
}

/// Atomically claim a v2 relay invite.
///
/// Executes the full redemption in one PostgreSQL transaction:
/// 1. Hash the presented code.
/// 2. `SELECT ... FOR UPDATE` on the invite row scoped by `(community, token_hash)`.
/// 3. If no row → `Invalid`.
/// 4. If `expires_at <= now()` → `Expired`.
/// 5. Check existing membership.
/// 6. If already a member → insert policy evidence (if configured), commit,
///    return `AlreadyMember` (no increment).
/// 7. If `max_uses` is set and `use_count >= max_uses` → `Exhausted`.
/// 8. Insert relay member with role `member`, `added_by = 'invite'`.
/// 9. Insert join-policy acceptance evidence (if configured).
/// 10. Increment `use_count`.
/// 11. Commit.
///
/// `FOR UPDATE` serializes concurrent claims so exactly one claimant wins the
/// final slot. Membership insertion, policy evidence, and consumption share
/// one commit — a failure in any rolls back all.
pub async fn claim_relay_invite(
    pool: &PgPool,
    community: CommunityId,
    token_hash: &[u8; 32],
    claimer_pubkey: &str,
    policy_version: Option<&str>,
) -> Result<ClaimOutcome> {
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

    // 2. SELECT FOR UPDATE — lock the invite row for the duration of this txn.
    let row = sqlx::query(
        "SELECT id, max_uses, use_count, expires_at \
         FROM relay_invites \
         WHERE community_id = $1 AND token_hash = $2 \
         FOR UPDATE",
    )
    .bind(community.as_uuid())
    .bind(token_hash)
    .fetch_optional(&mut *tx)
    .await?;

    // 3. No matching invite.
    let Some(invite) = row else {
        tx.rollback().await?;
        log_claim_outcome(community, None, "invalid", None, None);
        return Ok(ClaimOutcome::Invalid);
    };

    let invite_id: uuid::Uuid = invite.try_get("id")?;
    let max_uses: Option<i32> = invite.try_get("max_uses")?;
    let use_count: i32 = invite.try_get("use_count")?;
    let expires_at: DateTime<Utc> = invite.try_get("expires_at")?;

    // Expiry is checked before membership deliberately. An expired bearer must
    // not authorize fresh policy-acceptance evidence, even for an existing
    // member; exhausted-but-live invites remain valid for idempotent retries.
    if expires_at <= Utc::now() {
        tx.rollback().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "expired",
            max_uses,
            Some(use_count),
        );
        return Ok(ClaimOutcome::Expired);
    }

    let uses_remaining = || max_uses.map(|mu| mu - use_count);

    // 5. Check existing membership.
    let existing =
        sqlx::query("SELECT 1 FROM relay_members WHERE community_id = $1 AND pubkey = $2")
            .bind(community.as_uuid())
            .bind(claimer_pubkey)
            .fetch_optional(&mut *tx)
            .await?;

    if existing.is_some() {
        // 6. Already a member — insert policy evidence but do NOT increment.
        if let Some(version) = policy_version {
            sqlx::query(
                "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            )
            .bind(community.as_uuid())
            .bind(claimer_pubkey)
            .bind(version)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "already_member",
            max_uses,
            Some(use_count),
        );
        return Ok(ClaimOutcome::AlreadyMember {
            use_count,
            uses_remaining: uses_remaining(),
        });
    }

    // 7. Capacity check.
    if let Some(mu) = max_uses {
        if use_count >= mu {
            tx.rollback().await?;
            log_claim_outcome(
                community,
                Some(invite_id),
                "exhausted",
                max_uses,
                Some(use_count),
            );
            return Ok(ClaimOutcome::Exhausted);
        }
    }

    // 8. Insert relay member. The conflict branch covers a claimant admitted
    // concurrently through a different invite: only the transaction that
    // actually inserted membership may consume this invite.
    let inserted = sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) \
         VALUES ($1, $2, 'member', 'invite') \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(community.as_uuid())
    .bind(claimer_pubkey)
    .execute(&mut *tx)
    .await?
    .rows_affected()
        > 0;

    // 9. Insert join-policy acceptance evidence. This is required for both a
    // new member and a claimant whose concurrent membership insert won first.
    if let Some(version) = policy_version {
        sqlx::query(
            "INSERT INTO join_policy_acceptances (community_id, pubkey, policy_version) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(community.as_uuid())
        .bind(claimer_pubkey)
        .bind(version)
        .execute(&mut *tx)
        .await?;
    }

    if !inserted {
        tx.commit().await?;
        log_claim_outcome(
            community,
            Some(invite_id),
            "already_member",
            max_uses,
            Some(use_count),
        );
        return Ok(ClaimOutcome::AlreadyMember {
            use_count,
            uses_remaining: uses_remaining(),
        });
    }

    // 10. Increment use_count (for every new member, even unlimited).
    let new_use_count = use_count + 1;
    sqlx::query("UPDATE relay_invites SET use_count = $1 WHERE community_id = $2 AND id = $3")
        .bind(new_use_count)
        .bind(community.as_uuid())
        .bind(invite_id)
        .execute(&mut *tx)
        .await?;

    // 11. Commit.
    tx.commit().await?;

    let new_uses_remaining = max_uses.map(|mu| mu - new_use_count);

    log_claim_outcome(
        community,
        Some(invite_id),
        "joined",
        max_uses,
        Some(new_use_count),
    );

    Ok(ClaimOutcome::Joined {
        use_count: new_use_count,
        uses_remaining: new_uses_remaining,
    })
}

impl Db {
    /// Mints a v2 use-limited relay invite. The plaintext code is returned
    /// exactly once; only its SHA-256 hash is persisted.
    ///
    /// `max_uses` is `None` for unlimited or `Some(1..=10000)`.
    /// `ttl_secs` must be in the shared invite lifetime range.
    #[datastore_span(name = "mint_relay_invite", system = "postgresql")]
    pub async fn mint_relay_invite(
        &self,
        community: CommunityId,
        created_by: &str,
        ttl_secs: u64,
        max_uses: Option<i32>,
    ) -> Result<MintedInvite> {
        mint_relay_invite(&self.pool, community, created_by, ttl_secs, max_uses).await
    }

    /// Delete one bounded batch of invites expired before `cutoff`.
    #[datastore_span(name = "reap_expired_relay_invites", system = "postgresql")]
    pub async fn reap_expired_relay_invites(&self, cutoff: DateTime<Utc>) -> Result<u64> {
        reap_expired_relay_invites(&self.pool, cutoff).await
    }

    /// Atomically claims a v2 relay invite. The full redemption (membership
    /// insert, policy evidence, use_count increment) runs in one PostgreSQL
    /// transaction with `FOR UPDATE` on the invite row.
    ///
    /// `token_hash` is the SHA-256 of the presented v2 code (32 bytes).
    #[datastore_span(name = "claim_relay_invite", system = "postgresql")]
    pub async fn claim_relay_invite(
        &self,
        community: CommunityId,
        token_hash: &[u8; 32],
        claimer_pubkey: &str,
        policy_version: Option<&str>,
    ) -> Result<ClaimOutcome> {
        claim_relay_invite(
            &self.pool,
            community,
            token_hash,
            claimer_pubkey,
            policy_version,
        )
        .await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use crate::relay_members::is_relay_member;
    use sha2::Digest;
    use sqlx::PgPool;
    use uuid::Uuid;

    async fn setup_pool() -> PgPool {
        PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect to test DB")
    }

    async fn create_scratch_database(prefix: &str) -> (PgPool, String, String) {
        let admin_url = crate::test_support::database_url();
        let admin = PgPool::connect(&admin_url)
            .await
            .expect("connect to test database server");
        let name = format!("{}_{}", prefix, Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch database");
        let path_start = admin_url
            .rfind('/')
            .expect("database URL has a path segment");
        let scratch_url = format!("{}/{}", &admin_url[..path_start], name);
        (admin, name, scratch_url)
    }

    async fn drop_scratch_database(admin: PgPool, db: crate::Db, name: &str) {
        db.pool.close().await;
        drop(db);
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop scratch database");
        admin.close().await;
    }

    async fn make_test_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(format!("relay-invite-test-{}.example", id.simple()))
            .execute(pool)
            .await
            .expect("insert test community");
        CommunityId::from_uuid(id)
    }

    async fn delete_test_community(pool: &PgPool, community: CommunityId) {
        let mut tx = pool.begin().await.expect("begin test cleanup");
        sqlx::query("DELETE FROM relay_invites WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test invites");
        sqlx::query("DELETE FROM relay_members WHERE community_id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test members");
        sqlx::query("DELETE FROM communities WHERE id = $1")
            .bind(community.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("delete test community");
        tx.commit().await.expect("commit test cleanup");
    }

    fn test_pubkey() -> String {
        format!("{:064x}", Uuid::new_v4().as_u128())
    }

    async fn use_count(pool: &PgPool, community: CommunityId, invite_id: Uuid) -> i32 {
        sqlx::query_scalar(
            "SELECT use_count FROM relay_invites WHERE community_id = $1 AND id = $2",
        )
        .bind(community.as_uuid())
        .bind(invite_id)
        .fetch_one(pool)
        .await
        .expect("read invite use_count")
    }

    #[test]
    fn mint_validation_rejects_invalid_bounds_before_database_access() {
        for (ttl, max_uses) in [
            (MIN_INVITE_TTL_SECS - 1, None),
            (MAX_INVITE_TTL_SECS + 1, None),
            (3600, Some(0)),
            (3600, Some(-1)),
            (3600, Some(MAX_INVITE_USES + 1)),
        ] {
            let error = validate_mint_inputs(ttl, max_uses).expect_err("invalid mint contract");
            assert!(matches!(error, crate::DbError::InvalidData(_)), "{error:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn mint_after_quiescing_returns_typed_fence_without_persisting() {
        let (admin, database_name, database_url) =
            create_scratch_database("relay_invite_fence").await;
        let db = crate::Db::new(&crate::DbConfig {
            database_url,
            max_connections: 5,
            min_connections: 0,
            ..crate::DbConfig::default()
        })
        .await
        .expect("connect invite deletion test DB");
        db.migrate().await.expect("migrate invite deletion test DB");
        let pool = db.pool.clone();
        let store = db.deletion_store();
        let host = format!("relay-invite-fence-{}.example", Uuid::new_v4().simple());
        let community = db
            .ensure_configured_community(&host)
            .await
            .expect("create fenced invite community")
            .id;
        let request = store
            .submit(&host, "owner", None)
            .await
            .expect("submit deletion request");
        let empty_digest = hex::encode(sha2::Sha256::digest([]));
        let inventory = crate::deletion::FrozenInventory {
            schema: store
                .inventory_schema(community)
                .await
                .expect("inventory schema"),
            storage: crate::deletion::StorageManifest {
                version: 4,
                prefixes: [
                    format!("_meta/{community}/"),
                    format!("_uploads/{community}/"),
                    format!("repos/{community}/"),
                ]
                .into_iter()
                .map(|prefix| crate::deletion::PrefixManifest {
                    prefix,
                    object_count: 0,
                    total_bytes: 0,
                    keys_digest: empty_digest.clone(),
                })
                .collect(),
            },
        };
        store
            .freeze_inventory(request.id, &inventory)
            .await
            .expect("freeze inventory");
        store
            .approve(request.id, "owner", None)
            .await
            .expect("approve deletion");
        let claim = store
            .claim_specific(
                request.id,
                "executor",
                crate::deletion::DEFAULT_LEASE_DURATION,
            )
            .await
            .expect("claim deletion")
            .expect("runnable deletion");
        store
            .begin_quiescing(&claim.lease)
            .await
            .expect("begin quiescing");

        let error = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
            .await
            .expect_err("quiescing must reject invite minting");
        assert!(matches!(error, crate::error::DbError::AccessDenied(_)));

        let invite_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM relay_invites WHERE community_id = $1")
                .bind(community.as_uuid())
                .fetch_one(&pool)
                .await
                .expect("count relay invites");
        assert_eq!(invite_count, 0, "rejected mint must not persist an invite");

        drop(store);
        drop(pool);
        drop_scratch_database(admin, db, &database_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn bounded_claim_exhausts_and_existing_member_retry_does_not_consume() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let first = test_pubkey();
        let second = test_pubkey();
        let invite = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &first, None)
                .await
                .expect("first claim"),
            ClaimOutcome::Joined {
                use_count: 1,
                uses_remaining: Some(0),
            }
        );
        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &first, None)
                .await
                .expect("idempotent retry"),
            ClaimOutcome::AlreadyMember {
                use_count: 1,
                uses_remaining: Some(0),
            }
        );
        assert_eq!(
            claim_relay_invite(&pool, community, &hash, &second, None)
                .await
                .expect("exhausted claim"),
            ClaimOutcome::Exhausted
        );
        assert_eq!(use_count(&pool, community, invite.invite_id).await, 1);
        assert!(is_relay_member(&pool, community, &first)
            .await
            .expect("first membership"));
        assert!(!is_relay_member(&pool, community, &second)
            .await
            .expect("second membership"));
        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn concurrent_claims_serialize_the_final_slot() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let first = test_pubkey();
        let second = test_pubkey();
        let invite = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        let (first_outcome, second_outcome) = tokio::join!(
            claim_relay_invite(&pool, community, &hash, &first, None),
            claim_relay_invite(&pool, community, &hash, &second, None),
        );
        let outcomes = [
            first_outcome.expect("first concurrent claim"),
            second_outcome.expect("second concurrent claim"),
        ];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, ClaimOutcome::Joined { .. }))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, ClaimOutcome::Exhausted))
                .count(),
            1
        );
        assert_eq!(use_count(&pool, community, invite.invite_id).await, 1);
        let admitted = is_relay_member(&pool, community, &first)
            .await
            .expect("first membership") as u8
            + is_relay_member(&pool, community, &second)
                .await
                .expect("second membership") as u8;
        assert_eq!(admitted, 1);
        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn expiry_and_tenant_scope_return_typed_failures() {
        let pool = setup_pool().await;
        let community_a = make_test_community(&pool).await;
        let community_b = make_test_community(&pool).await;
        let invite = mint_relay_invite(&pool, community_a, "owner", 3600, Some(2))
            .await
            .expect("mint invite");
        let hash = hash_v2_code(&invite.code);

        assert_eq!(
            claim_relay_invite(&pool, community_b, &hash, &test_pubkey(), None)
                .await
                .expect("cross-tenant claim"),
            ClaimOutcome::Invalid
        );

        sqlx::query(
            "UPDATE relay_invites SET expires_at = now() - interval '1 second' \
             WHERE community_id = $1 AND id = $2",
        )
        .bind(community_a.as_uuid())
        .bind(invite.invite_id)
        .execute(&pool)
        .await
        .expect("expire invite");
        assert_eq!(
            claim_relay_invite(&pool, community_a, &hash, &test_pubkey(), None)
                .await
                .expect("expired claim"),
            ClaimOutcome::Expired
        );
        assert_eq!(use_count(&pool, community_a, invite.invite_id).await, 0);
        delete_test_community(&pool, community_a).await;
        delete_test_community(&pool, community_b).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn retention_sweep_deletes_only_invites_older_than_cutoff() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let old = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
            .await
            .expect("mint old invite");
        let recent = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
            .await
            .expect("mint recent invite");
        let cutoff = Utc::now() - chrono::Duration::days(30);

        sqlx::query("UPDATE relay_invites SET expires_at = $1 WHERE community_id = $2 AND id = $3")
            .bind(cutoff - chrono::Duration::seconds(1))
            .bind(community.as_uuid())
            .bind(old.invite_id)
            .execute(&pool)
            .await
            .expect("age old invite");

        assert_eq!(
            reap_expired_relay_invites(&pool, cutoff)
                .await
                .expect("reap expired invites"),
            1
        );
        let remaining: Vec<Uuid> =
            sqlx::query_scalar("SELECT id FROM relay_invites WHERE community_id = $1 ORDER BY id")
                .bind(community.as_uuid())
                .fetch_all(&pool)
                .await
                .expect("read remaining invites");
        assert_eq!(remaining, vec![recent.invite_id]);

        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn retention_sweep_skips_quiescing_tenant_while_active_bystanders_progress() {
        let (admin, database_name, database_url) =
            create_scratch_database("relay_invite_liveness").await;
        let db = crate::Db::new(&crate::DbConfig {
            database_url,
            max_connections: 5,
            min_connections: 0,
            ..crate::DbConfig::default()
        })
        .await
        .expect("connect invite liveness database");
        db.migrate()
            .await
            .expect("migrate invite liveness database");
        let pool = db.pool.clone();
        let active_a = make_test_community(&pool).await;
        let target = make_test_community(&pool).await;
        let active_x = make_test_community(&pool).await;
        let cutoff = Utc::now();
        for community in [active_a, target, active_x] {
            sqlx::query(
                "INSERT INTO relay_invites \
                 (community_id, token_hash, expires_at, created_by) \
                 VALUES ($1, $2, $3, 'test')",
            )
            .bind(community.as_uuid())
            .bind(sha2::Sha256::digest(community.as_uuid().as_bytes()).as_slice())
            .bind(cutoff - chrono::Duration::seconds(1))
            .execute(&pool)
            .await
            .expect("seed expired invite");
        }
        let mut lifecycle = pool.begin().await.expect("begin lifecycle fixture");
        sqlx::query(
            "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                    set_config('buzz.deletion_fence_generation', '0', true)",
        )
        .bind(target.to_string())
        .execute(&mut *lifecycle)
        .await
        .expect("authorize lifecycle fixture");
        sqlx::query("UPDATE communities SET deletion_state = 'quiescing' WHERE id = $1")
            .bind(target.as_uuid())
            .execute(&mut *lifecycle)
            .await
            .expect("quiesce target");
        lifecycle.commit().await.expect("commit lifecycle fixture");

        assert_eq!(
            reap_expired_relay_invites(&pool, cutoff)
                .await
                .expect("reap active bystanders"),
            2
        );
        let remaining: Vec<Uuid> =
            sqlx::query_scalar("SELECT community_id FROM relay_invites ORDER BY community_id")
                .fetch_all(&pool)
                .await
                .expect("read remaining invite attribution");
        assert_eq!(remaining, vec![*target.as_uuid()]);

        drop(pool);
        drop_scratch_database(admin, db, &database_name).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn unlimited_invites_count_each_new_member() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let invite = mint_relay_invite(&pool, community, "owner", 3600, None)
            .await
            .expect("mint unlimited invite");
        let hash = hash_v2_code(&invite.code);

        for (expected_count, pubkey) in [(1, test_pubkey()), (2, test_pubkey())] {
            assert_eq!(
                claim_relay_invite(&pool, community, &hash, &pubkey, None)
                    .await
                    .expect("unlimited claim"),
                ClaimOutcome::Joined {
                    use_count: expected_count,
                    uses_remaining: None,
                }
            );
        }
        assert_eq!(use_count(&pool, community, invite.invite_id).await, 2);
        delete_test_community(&pool, community).await;
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn policy_evidence_failure_rolls_back_membership_and_consumption() {
        let pool = setup_pool().await;
        let community = make_test_community(&pool).await;
        let pubkey = test_pubkey();
        let invite = mint_relay_invite(&pool, community, "owner", 3600, Some(1))
            .await
            .expect("mint bounded invite");
        let hash = hash_v2_code(&invite.code);

        let error = claim_relay_invite(&pool, community, &hash, &pubkey, Some("too-short"))
            .await
            .expect_err("policy CHECK must reject an invalid version");
        assert!(matches!(error, crate::DbError::Sqlx(_)), "{error:?}");
        assert!(!is_relay_member(&pool, community, &pubkey)
            .await
            .expect("membership after rollback"));
        assert_eq!(use_count(&pool, community, invite.invite_id).await, 0);

        assert!(matches!(
            claim_relay_invite(&pool, community, &hash, &pubkey, None)
                .await
                .expect("claim after rollback"),
            ClaimOutcome::Joined { use_count: 1, .. }
        ));
        delete_test_community(&pool, community).await;
    }
}
