//! Deployment-global relay operator/moderator roster persistence.
//!
//! Backs the `relay_operators` table from `migrations/0035_relay_operators.sql`.
//!
//! Config-backed operators (`RELAY_OPERATOR_PUBKEYS`, owner-fallback) are
//! resolved at request time in the relay — this module only handles DB rows.
//! Config outranks DB: a DB moderator row for a config-backed Operator is
//! never returned as authoritative; that check happens at the relay layer.
//!
//! Lane ownership: relay admin API (Duncan).

use buzz_datastore_tracing::datastore_span;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row as _, Transaction};

use crate::error::{DbError, Result};

/// Advisory-lock namespace for per-target roster mutation serialization. The
/// hashed key is `<namespace><hex-pubkey>`, scoping the lock to one target so
/// mutations of different operators never contend.
const OPERATOR_LOCK_NAMESPACE: &str = "relay_operator:";

/// Well-known advisory-lock key for roster-wide serialization. Operator-
/// removing mutations (demote/delete) take this single lock so the last-
/// operator invariant is computed against a snapshot no concurrent removal can
/// invalidate — a per-target lock cannot see a race between two *different*
/// targets both dropping to zero. Always acquired before any per-target lock,
/// giving a consistent lock order (deadlock-free: `remove` takes only this
/// lock, `upsert` takes this then per-target).
const OPERATOR_ROSTER_LOCK: &str = "relay_operator_roster";

/// Take a transaction-scoped advisory lock keyed by the target pubkey. Held
/// until the transaction commits or rolls back; serializes the read/upsert/audit
/// sequence against a concurrent mutation of the same target (see `upsert`).
async fn acquire_operator_lock(tx: &mut Transaction<'_, Postgres>, pubkey: &[u8]) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("{OPERATOR_LOCK_NAMESPACE}{}", hex::encode(pubkey)))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Take the transaction-scoped roster-wide advisory lock. Serializes all
/// operator-removing mutations against each other so the last-operator check
/// sees a stable count.
async fn acquire_roster_lock(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(OPERATOR_ROSTER_LOCK)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Number of DB rows currently carrying the `operator` role, read inside the
/// mutation transaction after the change is applied.
async fn db_operator_count(tx: &mut Transaction<'_, Postgres>) -> Result<i64> {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_operators WHERE role = 'operator'")
            .fetch_one(&mut **tx)
            .await?;
    Ok(count)
}

/// A row in `relay_operators`.
#[derive(Debug, Clone)]
pub struct RelayOperatorRecord {
    /// 32-byte pubkey (binary).
    pub pubkey: Vec<u8>,
    /// `"operator"` | `"moderator"`.
    pub role: String,
    /// Pubkey of the operator who added this entry (32 bytes binary).
    pub added_by: Vec<u8>,
    /// Row creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Insert or update a relay operator/moderator DB row, recording the mutation
/// in the append-only `relay_operator_audit` trail within the same transaction.
///
/// If the pubkey already exists, updates the role and added_by atomically. The
/// pre-image (`prev_role`) is read under the transaction so the audit row
/// captures the role the upsert overwrites. A per-target advisory lock (held
/// for the transaction) serializes concurrent mutations of the same target so
/// the pre-image can never be misread across the absent-row race.
///
/// `config_operator_exists` is the request-time snapshot of whether any
/// config-backed operator (`RELAY_OPERATOR_PUBKEYS` or active owner fallback)
/// is effective. A demotion (`role == "moderator"`) that would leave no
/// effective operator — no config operator and no remaining DB `operator` row —
/// is rolled back with [`DbError::LastOperator`]. Grants and promotions to
/// operator never remove an operator, so they take neither the roster lock nor
/// the invariant check.
pub async fn upsert(
    pool: &PgPool,
    pubkey: &[u8],
    role: &str,
    added_by: &[u8],
    config_operator_exists: bool,
) -> Result<()> {
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

    // A demotion to moderator can drop the effective-operator count; serialize
    // it against every other operator-removing mutation via the roster-wide
    // lock BEFORE the per-target lock so the post-mutation count is race-free (a
    // per-target lock cannot observe a concurrent removal of a *different*
    // operator). We take the lock whenever the target role is `moderator` —
    // before the pre-image is known — and only enforce the invariant below once
    // the pre-image confirms this actually demoted an operator.
    let demotion_candidate = role == "moderator";
    if demotion_candidate {
        acquire_roster_lock(&mut tx).await?;
    }

    // Serialize concurrent mutations of the SAME target before the pre-image
    // read. `SELECT ... FOR UPDATE` locks nothing when the row is absent, so
    // two concurrent first-time grants could both read `prev_role = NULL`; the
    // loser of the insert race would then overwrite the winner's row while
    // still auditing `prev_role = NULL`, erasing the first grant from the very
    // history this audit exists to record. A transaction-scoped advisory lock
    // keyed by the pubkey makes the read/upsert/audit atomic against a
    // concurrent mutation of the same target. `remove` needs no such lock: its
    // `DELETE ... RETURNING` takes the row lock and reads the pre-image in one
    // statement, so there is no absent-row read gap to widen.
    acquire_operator_lock(&mut tx, pubkey).await?;

    // Pre-image read inside the transaction: the role the upsert overwrites,
    // or NULL when the target has no prior row.
    let prev_role: Option<String> =
        sqlx::query_scalar("SELECT role FROM relay_operators WHERE pubkey = $1 FOR UPDATE")
            .bind(pubkey)
            .fetch_optional(&mut *tx)
            .await?;

    sqlx::query(
        r#"
        INSERT INTO relay_operators (pubkey, role, added_by)
        VALUES ($1, $2, $3)
        ON CONFLICT (pubkey) DO UPDATE SET
            role = EXCLUDED.role,
            added_by = EXCLUDED.added_by
        "#,
    )
    .bind(pubkey)
    .bind(role)
    .bind(added_by)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO relay_operator_audit
            (actor_pubkey, target_pubkey, op, prev_role, new_role)
        VALUES ($1, $2, 'grant', $3, $4)
        "#,
    )
    .bind(added_by)
    .bind(pubkey)
    .bind(prev_role.as_deref())
    .bind(role)
    .execute(&mut *tx)
    .await?;

    // Enforce the last-operator invariant only when this mutation actually
    // demoted an operator (`prev_role == "operator"`, `new_role == "moderator"`).
    // A fresh moderator grant (prev_role NULL) or a moderator→moderator no-op
    // never removed an operator, so it must not trip the invariant even when the
    // roster is empty. Dropping the tx without committing rolls the demotion and
    // its audit row back.
    let demotion = demotion_candidate && prev_role.as_deref() == Some("operator");
    if demotion && !config_operator_exists && db_operator_count(&mut tx).await? == 0 {
        return Err(DbError::LastOperator);
    }

    tx.commit().await?;
    Ok(())
}

/// Remove a relay operator/moderator DB row, recording the revocation in the
/// append-only `relay_operator_audit` trail within the same transaction.
///
/// Returns `true` if a row was deleted (and audited), `false` if the pubkey was
/// not found (idempotent no-op; no audit row is written).
///
/// `config_operator_exists` is the request-time snapshot of whether any
/// config-backed operator is effective. Deleting the sole effective operator —
/// no config operator and no remaining DB `operator` row — is rolled back with
/// [`DbError::LastOperator`]. The roster-wide lock (taken before the delete)
/// serializes this against every other operator-removing mutation so two
/// concurrent deletes of different operators cannot both race to zero.
pub async fn remove(
    pool: &PgPool,
    pubkey: &[u8],
    actor: &[u8],
    config_operator_exists: bool,
) -> Result<bool> {
    let connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;

    // Serialize against every other operator-removing mutation before the
    // delete so the post-delete count reflects a stable roster.
    acquire_roster_lock(&mut tx).await?;

    // Capture the pre-image role the delete removes; also gates the audit row
    // so a no-op delete of an absent pubkey writes nothing.
    let prev_role: Option<String> =
        sqlx::query_scalar("DELETE FROM relay_operators WHERE pubkey = $1 RETURNING role")
            .bind(pubkey)
            .fetch_optional(&mut *tx)
            .await?;

    let removed = prev_role.is_some();
    if removed {
        sqlx::query(
            r#"
            INSERT INTO relay_operator_audit
                (actor_pubkey, target_pubkey, op, prev_role, new_role)
            VALUES ($1, $2, 'revoke', $3, NULL)
            "#,
        )
        .bind(actor)
        .bind(pubkey)
        .bind(prev_role.as_deref())
        .execute(&mut *tx)
        .await?;

        // Deleting an operator can empty the roster. Dropping the tx here rolls
        // the delete and its audit row back.
        if !config_operator_exists && db_operator_count(&mut tx).await? == 0 {
            return Err(DbError::LastOperator);
        }
    }

    tx.commit().await?;
    Ok(removed)
}

/// Fetch one relay operator/moderator row by pubkey.
pub async fn get(pool: &PgPool, pubkey: &[u8]) -> Result<Option<RelayOperatorRecord>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let row = sqlx::query(
        "SELECT pubkey, role, added_by, created_at FROM relay_operators WHERE pubkey = $1",
    )
    .bind(pubkey)
    .fetch_optional(&mut *connection)
    .await?;

    row.map(
        |r| -> std::result::Result<RelayOperatorRecord, sqlx::Error> {
            Ok(RelayOperatorRecord {
                pubkey: r.try_get("pubkey")?,
                role: r.try_get("role")?,
                added_by: r.try_get("added_by")?,
                created_at: r.try_get("created_at")?,
            })
        },
    )
    .transpose()
    .map_err(crate::error::DbError::from)
}

/// List all relay operator/moderator rows, ordered by creation time.
pub async fn list(pool: &PgPool) -> Result<Vec<RelayOperatorRecord>> {
    let mut connection = crate::observability::acquire_writer(
        pool,
        crate::observability::WriterOperation::Authorization,
    )
    .await?;
    let rows = sqlx::query(
        "SELECT pubkey, role, added_by, created_at FROM relay_operators ORDER BY created_at ASC",
    )
    .fetch_all(&mut *connection)
    .await?;

    rows.into_iter()
        .map(
            |r| -> std::result::Result<RelayOperatorRecord, sqlx::Error> {
                Ok(RelayOperatorRecord {
                    pubkey: r.try_get("pubkey")?,
                    role: r.try_get("role")?,
                    added_by: r.try_get("added_by")?,
                    created_at: r.try_get("created_at")?,
                })
            },
        )
        .collect::<std::result::Result<Vec<_>, sqlx::Error>>()
        .map_err(crate::error::DbError::from)
}

impl crate::Db {
    /// Fetch one relay operator/moderator row by pubkey (32-byte binary).
    #[datastore_span(name = "get_relay_operator", system = "postgresql")]
    pub async fn get_relay_operator(&self, pubkey: &[u8]) -> Result<Option<RelayOperatorRecord>> {
        get(&self.pool, pubkey).await
    }

    /// List all relay operator/moderator rows ordered by creation time.
    #[datastore_span(name = "list_relay_operators", system = "postgresql")]
    pub async fn list_relay_operators(&self) -> Result<Vec<RelayOperatorRecord>> {
        list(&self.pool).await
    }

    /// Insert or update a relay operator/moderator row (upsert by pubkey).
    ///
    /// `config_operator_exists` is the caller's request-time snapshot of
    /// whether a config-backed operator is effective; a demotion that would
    /// leave no effective operator is rejected with [`DbError::LastOperator`].
    #[datastore_span(name = "upsert_relay_operator", system = "postgresql")]
    pub async fn upsert_relay_operator(
        &self,
        pubkey: &[u8],
        role: &str,
        added_by: &[u8],
        config_operator_exists: bool,
    ) -> Result<()> {
        upsert(&self.pool, pubkey, role, added_by, config_operator_exists).await
    }

    /// Remove a relay operator/moderator row. Returns `true` if deleted.
    /// Records the revocation in the append-only audit trail; `actor` is the
    /// authenticated operator performing the removal. `config_operator_exists`
    /// is the caller's request-time snapshot of whether a config-backed
    /// operator is effective; deleting the sole effective operator is rejected
    /// with [`DbError::LastOperator`].
    #[datastore_span(name = "remove_relay_operator", system = "postgresql")]
    pub async fn remove_relay_operator(
        &self,
        pubkey: &[u8],
        actor: &[u8],
        config_operator_exists: bool,
    ) -> Result<bool> {
        remove(&self.pool, pubkey, actor, config_operator_exists).await
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use sqlx::PgPool;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1

    async fn setup_pool() -> PgPool {
        let url =
            std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
        PgPool::connect(&url).await.expect("connect to test DB")
    }

    /// One audit row per mutation, capturing the pre-image role across a
    /// grant → role-change → revoke sequence — the history the in-place
    /// upsert/delete would otherwise destroy.
    #[tokio::test]
    #[ignore = "requires Postgres — roster audit trail across grant/change/revoke"]
    async fn roster_mutations_write_pre_image_audit_rows() {
        let pool = setup_pool().await;
        let actor = vec![7u8; 32];
        let target: Vec<u8> = {
            let id = uuid::Uuid::new_v4();
            id.as_bytes().iter().chain(id.as_bytes()).copied().collect()
        };

        async fn audit_rows(
            pool: &PgPool,
            target: &[u8],
        ) -> Vec<(String, Option<String>, Option<String>)> {
            sqlx::query_as(
                "SELECT op, prev_role, new_role FROM relay_operator_audit \
                 WHERE target_pubkey = $1 ORDER BY seq ASC",
            )
            .bind(target)
            .fetch_all(pool)
            .await
            .expect("read audit rows")
        }

        // Grant moderator: no prior row → prev_role NULL, new_role moderator.
        upsert(&pool, &target, "moderator", &actor, true)
            .await
            .expect("grant");
        // Elevate to operator: prev_role moderator, new_role operator.
        upsert(&pool, &target, "operator", &actor, true)
            .await
            .expect("elevate");
        // Revoke: prev_role operator, new_role NULL.
        assert!(remove(&pool, &target, &actor, true).await.expect("revoke"));
        // Idempotent no-op revoke writes no audit row.
        assert!(!remove(&pool, &target, &actor, true)
            .await
            .expect("no-op revoke"));

        let rows = audit_rows(&pool, &target).await;
        assert_eq!(
            rows,
            vec![
                ("grant".to_string(), None, Some("moderator".to_string())),
                (
                    "grant".to_string(),
                    Some("moderator".to_string()),
                    Some("operator".to_string())
                ),
                ("revoke".to_string(), Some("operator".to_string()), None),
            ],
            "audit trail must record pre-image on every mutation and nothing for the no-op delete"
        );
    }

    /// The per-target advisory lock serializes concurrent roster mutations so
    /// the audit pre-image can never be misread. Two facets, both required:
    ///
    /// - *Causality of the lock:* holding the exact advisory key `upsert` takes
    ///   must block a concurrent first grant. Removing the lock from `upsert`
    ///   makes the spawned call return immediately and fails this assertion.
    /// - *Semantics:* the second committed upsert must record the FIRST
    ///   committed role as its pre-image, never NULL — the false pre-image the
    ///   absent-row race would otherwise produce.
    #[tokio::test]
    #[ignore = "requires Postgres — per-target lock serializes concurrent roster mutations"]
    async fn concurrent_upserts_serialize_and_record_true_pre_image() {
        let pool = setup_pool().await;
        let actor_a = vec![8u8; 32];
        let actor_b = vec![9u8; 32];
        let target: Vec<u8> = {
            let id = uuid::Uuid::new_v4();
            id.as_bytes().iter().chain(id.as_bytes()).copied().collect()
        };

        // Phase 1 — hold the exact advisory key upsert() takes; a concurrent
        // first grant must make no progress until the key is released.
        let mut holder = pool.begin().await.expect("begin lock holder");
        acquire_operator_lock(&mut holder, &target)
            .await
            .expect("hold operator key");

        let (pool2, t2, a2) = (pool.clone(), target.clone(), actor_a.clone());
        let mut grant =
            tokio::spawn(async move { upsert(&pool2, &t2, "moderator", &a2, true).await });
        let blocked = tokio::time::timeout(std::time::Duration::from_millis(750), &mut grant).await;
        assert!(
            blocked.is_err(),
            "first grant must serialize on the per-target operator key"
        );

        // Release the key; the first grant commits with prev_role NULL.
        holder.rollback().await.expect("release operator key");
        tokio::time::timeout(std::time::Duration::from_secs(10), grant)
            .await
            .expect("grant must proceed once the key is released")
            .expect("join grant task")
            .expect("grant");

        // Phase 2 — a second upsert reads the first committed role, not NULL.
        upsert(&pool, &target, "operator", &actor_b, true)
            .await
            .expect("elevate");

        let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT op, prev_role, new_role FROM relay_operator_audit \
             WHERE target_pubkey = $1 ORDER BY seq ASC",
        )
        .bind(&target)
        .fetch_all(&pool)
        .await
        .expect("read audit rows");
        assert_eq!(
            rows,
            vec![
                ("grant".to_string(), None, Some("moderator".to_string())),
                (
                    "grant".to_string(),
                    Some("moderator".to_string()),
                    Some("operator".to_string())
                ),
            ],
            "second committed upsert must record the first committed role as its pre-image"
        );
    }

    /// Ordered audit reads must follow `seq` (insertion order under the
    /// serializing lock), never `created_at`. A wall clock is not monotonic:
    /// an NTP step backward between two serialized mutations can hand the later
    /// mutation a smaller `clock_timestamp()`, so ordering by the timestamp
    /// would still invert the privilege chain. `seq` is the sole ordering
    /// authority; the timestamp is informational.
    ///
    /// This inserts same-target rows with DELIBERATELY INVERTED `created_at`
    /// (the grant, inserted first, gets a *future* stamp; the revoke, inserted
    /// second, gets a *past* stamp) to simulate the backward-clock step. The
    /// `ORDER BY seq` read must still return grant→revoke — the true insertion
    /// order. Ordering by `created_at` instead would return the impossible
    /// revoke→grant, so dropping `seq` from the read fails this assertion.
    #[tokio::test]
    #[ignore = "requires Postgres — audit order follows seq, not the non-monotonic wall clock"]
    async fn audit_order_follows_seq_under_backward_clock() {
        let pool = setup_pool().await;
        let actor = vec![5u8; 32];
        let target: Vec<u8> = {
            let id = uuid::Uuid::new_v4();
            id.as_bytes().iter().chain(id.as_bytes()).copied().collect()
        };

        // Grant inserted FIRST (lower seq) but stamped in the FUTURE.
        sqlx::query(
            "INSERT INTO relay_operator_audit \
             (actor_pubkey, target_pubkey, op, prev_role, new_role, created_at) \
             VALUES ($1, $2, 'grant', NULL, 'moderator', now() + interval '1 hour')",
        )
        .bind(&actor)
        .bind(&target)
        .execute(&pool)
        .await
        .expect("insert grant audit row");

        // Revoke inserted SECOND (higher seq) but stamped in the PAST — the
        // inversion a backward clock step would produce.
        sqlx::query(
            "INSERT INTO relay_operator_audit \
             (actor_pubkey, target_pubkey, op, prev_role, new_role, created_at) \
             VALUES ($1, $2, 'revoke', 'moderator', NULL, now() - interval '1 hour')",
        )
        .bind(&actor)
        .bind(&target)
        .execute(&pool)
        .await
        .expect("insert revoke audit row");

        // ORDER BY seq must return grant→revoke (insertion order); ordering by
        // created_at would return the impossible revoke→grant.
        let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT op, prev_role, new_role FROM relay_operator_audit \
             WHERE target_pubkey = $1 ORDER BY seq ASC",
        )
        .bind(&target)
        .fetch_all(&pool)
        .await
        .expect("read audit rows");
        assert_eq!(
            rows,
            vec![
                ("grant".to_string(), None, Some("moderator".to_string())),
                ("revoke".to_string(), Some("moderator".to_string()), None),
            ],
            "ordered read must follow seq (insertion order), not the non-monotonic wall clock"
        );
    }

    /// The roster mutation and its audit row share one transaction, so an audit
    /// INSERT failure must roll the roster mutation back. Injected via a BEFORE
    /// INSERT trigger that raises only for a fixed sentinel target pubkey (the
    /// `WHEN` guard keeps every other target — including concurrent audit tests
    /// — unaffected). Moving the audit INSERT outside the transaction would
    /// leave the roster row behind and fail this test.
    #[tokio::test]
    #[ignore = "requires Postgres — audit INSERT failure rolls back the roster mutation"]
    async fn audit_insert_failure_rolls_back_roster_mutation() {
        let pool = setup_pool().await;
        let actor = vec![4u8; 32];
        // Fixed sentinel the trigger's WHEN guard matches (see the trigger DDL).
        let target = vec![0xABu8; 32];

        // Clean any prior roster row for the sentinel so the assertion is about
        // this run's rollback, not a leaked row.
        sqlx::query("DELETE FROM relay_operators WHERE pubkey = $1")
            .bind(&target)
            .execute(&pool)
            .await
            .expect("clear sentinel roster row");

        // Install a trigger that fails the audit INSERT for the sentinel only.
        // Static SQL (no interpolation): fixed names + fixed sentinel bytea.
        sqlx::query(
            "CREATE OR REPLACE FUNCTION reject_operator_audit_sentinel() RETURNS trigger \
             AS $$ BEGIN RAISE EXCEPTION 'injected audit failure'; END; $$ LANGUAGE plpgsql",
        )
        .execute(&pool)
        .await
        .expect("create reject fn");
        sqlx::query(
            "DROP TRIGGER IF EXISTS trg_reject_operator_audit_sentinel ON relay_operator_audit",
        )
        .execute(&pool)
        .await
        .expect("drop stale trigger");
        sqlx::query(
            "CREATE TRIGGER trg_reject_operator_audit_sentinel BEFORE INSERT ON relay_operator_audit \
             FOR EACH ROW WHEN (NEW.target_pubkey = \
             '\\xabababababababababababababababababababababababababababababababab'::bytea) \
             EXECUTE FUNCTION reject_operator_audit_sentinel()",
        )
        .execute(&pool)
        .await
        .expect("create reject trigger");

        // The in-transaction audit INSERT raises, so the upsert must error.
        let result = upsert(&pool, &target, "moderator", &actor, true).await;
        assert!(result.is_err(), "audit failure must surface as an error");

        // Coupling: the roster mutation shares the audit transaction, so it
        // must have rolled back — no roster row for the target.
        let roster = get(&pool, &target).await.expect("get roster");
        assert!(
            roster.is_none(),
            "roster row must roll back when the audit INSERT fails"
        );

        // Remove the trigger/function to keep the shared DB clean.
        sqlx::query(
            "DROP TRIGGER IF EXISTS trg_reject_operator_audit_sentinel ON relay_operator_audit",
        )
        .execute(&pool)
        .await
        .ok();
        sqlx::query("DROP FUNCTION IF EXISTS reject_operator_audit_sentinel()")
            .execute(&pool)
            .await
            .ok();
    }

    /// Fresh unique 32-byte pubkey for a last-operator test, so parallel or
    /// repeated runs never collide on the same target row.
    fn unique_pubkey() -> Vec<u8> {
        let id = uuid::Uuid::new_v4();
        id.as_bytes().iter().chain(id.as_bytes()).copied().collect()
    }

    /// Empty the roster so `db_operator_count` reflects only rows this test
    /// creates — the last-operator invariant counts every `role='operator'`
    /// row in the table. Safe for `#[ignore]` PG tests run individually.
    async fn clear_roster(pool: &PgPool) {
        sqlx::query("DELETE FROM relay_operators")
            .execute(pool)
            .await
            .expect("clear roster");
    }

    async fn audit_count(pool: &PgPool, target: &[u8]) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM relay_operator_audit WHERE target_pubkey = $1")
            .bind(target)
            .fetch_one(pool)
            .await
            .expect("count audit rows")
    }

    /// Demoting the sole DB operator with no config-backed operator must roll
    /// back with `LastOperator`: the row stays `operator` and no demotion audit
    /// row is written. Without the in-transaction invariant the demotion would
    /// commit and empty the effective roster.
    #[tokio::test]
    #[ignore = "requires Postgres — last-operator invariant on self-demotion"]
    async fn demoting_sole_db_operator_without_config_is_rejected() {
        let pool = setup_pool().await;
        clear_roster(&pool).await;
        let target = unique_pubkey();

        upsert(&pool, &target, "operator", &target, false)
            .await
            .expect("grant sole operator");

        let result = upsert(&pool, &target, "moderator", &target, false).await;
        assert!(
            matches!(result, Err(DbError::LastOperator)),
            "demoting the sole operator with no config fallback must be rejected, got {result:?}"
        );

        let row = get(&pool, &target)
            .await
            .expect("get")
            .expect("row present");
        assert_eq!(row.role, "operator", "demotion must have rolled back");
        assert_eq!(
            audit_count(&pool, &target).await,
            1,
            "only the grant audit row survives; the rejected demotion writes none"
        );
    }

    /// Deleting the sole DB operator with no config-backed operator must roll
    /// back with `LastOperator`: the row stays and no revoke audit row is
    /// written.
    #[tokio::test]
    #[ignore = "requires Postgres — last-operator invariant on self-delete"]
    async fn deleting_sole_db_operator_without_config_is_rejected() {
        let pool = setup_pool().await;
        clear_roster(&pool).await;
        let target = unique_pubkey();

        upsert(&pool, &target, "operator", &target, false)
            .await
            .expect("grant sole operator");

        let result = remove(&pool, &target, &target, false).await;
        assert!(
            matches!(result, Err(DbError::LastOperator)),
            "deleting the sole operator with no config fallback must be rejected, got {result:?}"
        );

        assert!(
            get(&pool, &target).await.expect("get").is_some(),
            "delete must have rolled back — row still present"
        );
        assert_eq!(
            audit_count(&pool, &target).await,
            1,
            "only the grant audit row survives; the rejected delete writes none"
        );
    }

    /// A config-backed operator (or active owner fallback) is signalled by
    /// `config_operator_exists = true`; with it set, deleting the last DB
    /// operator is allowed because config still guarantees an effective
    /// operator — the invariant only guards the empty-config case.
    #[tokio::test]
    #[ignore = "requires Postgres — config fallback allows emptying the DB roster"]
    async fn config_present_allows_deleting_last_db_operator() {
        let pool = setup_pool().await;
        clear_roster(&pool).await;
        let target = unique_pubkey();

        upsert(&pool, &target, "operator", &target, true)
            .await
            .expect("grant operator");

        let removed = remove(&pool, &target, &target, true)
            .await
            .expect("delete allowed when config operator exists");
        assert!(removed, "row was deleted");
        assert!(
            get(&pool, &target).await.expect("get").is_none(),
            "row must be gone"
        );
    }

    /// The roster-wide advisory lock serializes operator-removing mutations
    /// ACROSS targets — the property the per-target lock cannot provide. Two
    /// facets, both required:
    ///
    /// - *Causality:* while a holder transaction owns the roster lock, a
    ///   concurrent `remove` of a DIFFERENT operator must make no progress
    ///   (it blocks acquiring the same roster lock). Dropping the roster lock
    ///   from `remove` makes the spawned call return immediately and fails the
    ///   block assertion — the per-target lock keys on the pubkey and never
    ///   contends across targets.
    /// - *Semantics:* once serialized, the second delete sees the first's
    ///   committed removal, so the two operators cannot both race to zero — the
    ///   loser is rejected with `LastOperator`, leaving one operator standing.
    #[tokio::test]
    #[ignore = "requires Postgres — roster lock serializes concurrent cross-target deletes"]
    async fn concurrent_deletes_racing_to_zero_leave_one_operator() {
        let pool = setup_pool().await;
        clear_roster(&pool).await;
        let a = unique_pubkey();
        let b = unique_pubkey();

        upsert(&pool, &a, "operator", &a, false)
            .await
            .expect("grant operator a");
        upsert(&pool, &b, "operator", &b, false)
            .await
            .expect("grant operator b");

        // Phase 1 — hold the roster lock; a concurrent delete of a DIFFERENT
        // target must serialize on it and make no progress until released.
        let mut holder = pool.begin().await.expect("begin lock holder");
        acquire_roster_lock(&mut holder)
            .await
            .expect("hold roster lock");

        let (p2, t2) = (pool.clone(), a.clone());
        let mut del_a = tokio::spawn(async move { remove(&p2, &t2, &t2, false).await });
        let blocked = tokio::time::timeout(std::time::Duration::from_millis(750), &mut del_a).await;
        assert!(
            blocked.is_err(),
            "a delete of a different target must serialize on the roster-wide lock"
        );

        // Release the roster lock; the first delete now commits (b still an
        // operator, so the roster is not emptied).
        holder.rollback().await.expect("release roster lock");
        let removed_a = tokio::time::timeout(std::time::Duration::from_secs(10), del_a)
            .await
            .expect("delete a must proceed once the lock is released")
            .expect("join delete a")
            .expect("delete a");
        assert!(removed_a, "first delete removes operator a");

        // Phase 2 — b is now the sole operator; deleting it races the roster to
        // zero and must be rejected, leaving one operator standing.
        let result_b = remove(&pool, &b, &b, false).await;
        assert!(
            matches!(result_b, Err(DbError::LastOperator)),
            "deleting the last remaining operator must be rejected, got {result_b:?}"
        );

        let remaining = db_operator_count(&mut pool.begin().await.expect("begin"))
            .await
            .expect("count operators");
        assert_eq!(remaining, 1, "operator b must remain standing");
    }
}
