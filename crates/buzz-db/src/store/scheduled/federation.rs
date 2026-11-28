use buzz_core::CommunityId;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    deletion::guard_community_write,
    runtime::observability::{acquire_writer, WriterOperation},
    AdmittedTx, Db, DbError, Result,
};

/// Verified federated authority used to admit a scheduled message.
pub struct ScheduleAdmission {
    /// Issuer of the verified assertion.
    pub issuer: String,
    /// Owner of the assertion's agent key, if it names one.
    pub owner: Option<[u8; 32]>,
    /// Signed assertion issuance time.
    pub issued_at: DateTime<Utc>,
    /// Time the relay began evaluating the request's admission.
    pub admitted_at: DateTime<Utc>,
}

impl ScheduleAdmission {
    // Command cutoffs have one-second precision. Round a fractional issuance
    // up so that an assertion issued just after a cutoff does not match it.
    pub(super) fn issued_second(&self) -> i64 {
        self.issued_at.timestamp() + i64::from(self.issued_at.timestamp_subsec_nanos() != 0)
    }
}

pub(super) async fn begin_admission(
    pool: &PgPool,
    community: CommunityId,
    author: &[u8],
    admission: &ScheduleAdmission,
) -> Result<AdmittedTx> {
    let connection = acquire_writer(pool, WriterOperation::EventWrite).await?;
    let mut tx = sqlx::Transaction::begin(connection, None).await?;
    sqlx::query("SELECT pg_advisory_xact_lock_shared(hashtextextended('scheduled-fi:' || $1, 0))")
        .bind(&admission.issuer)
        .execute(&mut *tx)
        .await?;
    let refused: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM scheduled_message_disconnects
         WHERE issuer=$1 AND target IN ($2,$3) AND expires_at>clock_timestamp()
           AND (revoke_through IS NULL OR $4<=revoke_through)
           AND ($5<=recorded_at OR $5<deny_until))",
    )
    .bind(&admission.issuer)
    .bind(author)
    .bind(admission.owner.as_ref().map(|key| key.as_slice()))
    .bind(admission.issued_second())
    .bind(admission.admitted_at)
    .fetch_one(&mut *tx)
    .await?;
    if refused {
        return Err(DbError::AccessDenied(
            "scheduled admission was revoked".into(),
        ));
    }
    AdmittedTx::admit(
        tx,
        &crate::deletion::DeletionStore::new(pool.clone()),
        community,
    )
    .await
}

impl Db {
    /// Check whether the scheduling migration is present on the writer.
    pub async fn scheduled_disconnects_installed(&self) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT to_regclass('public.scheduled_message_disconnects') IS NOT NULL",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    /// Record an issuer disconnect and cancel its deferred actions atomically.
    /// Accepted queue writes take the shared form of the same issuer lock.
    pub async fn scheduled_disconnect(
        &self,
        issuer: &str,
        target: &[u8; 32],
        revoke_through: Option<i64>,
        until: DateTime<Utc>,
        retention_secs: u64,
    ) -> Result<()> {
        const MAX_RECORDS_PER_ISSUER: i64 = 50_000;
        let connection = acquire_writer(&self.pool, WriterOperation::EventWrite).await?;
        let mut tx = sqlx::Transaction::begin(connection, None).await?;
        sqlx::query("SET LOCAL lock_timeout = '3s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL statement_timeout = '5s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('scheduled-fi:' || $1, 0))")
            .bind(issuer)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM scheduled_message_disconnects WHERE issuer=$1 AND expires_at<=clock_timestamp()")
            .bind(issuer).execute(&mut *tx).await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM scheduled_message_disconnects WHERE issuer=$1",
        )
        .bind(issuer)
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_RECORDS_PER_ISSUER {
            return Err(DbError::InvalidData(
                "scheduled disconnect capacity reached".into(),
            ));
        }
        let recorded_at: DateTime<Utc> = sqlx::query_scalar(
            "WITH stamp AS (SELECT clock_timestamp() AS at)
             INSERT INTO scheduled_message_disconnects
                 (issuer,target,revoke_through,recorded_at,deny_until,expires_at)
             SELECT $1,$2,$3,at,$4,GREATEST(at,$4)+make_interval(secs=>$5) FROM stamp
             RETURNING recorded_at",
        )
        .bind(issuer)
        .bind(target.as_slice())
        .bind(revoke_through)
        .bind(until)
        .bind(retention_secs as f64)
        .fetch_one(&mut *tx)
        .await?;

        // Use the same predicate for discovering affected communities and
        // cancelling. A concurrent reschedule under a different issuer must
        // not be cancelled by this issuer.
        const MATCHING: &str = "status='scheduled' AND admission_issuer=$1
            AND (owner=$2 OR admission_owner=$2)
            AND ($3::bigint IS NULL OR admission_issued_second<=$3)
            AND (admitted_at<=$4 OR admitted_at<$5)";
        let discover = format!("SELECT DISTINCT community_id FROM scheduled_messages WHERE {MATCHING} ORDER BY community_id");
        let cancel = format!(
            "UPDATE scheduled_messages SET status='cancelled', revision=revision+1, error=NULL,
                updated_at=clock_timestamp(), prune_at=GREATEST(clock_timestamp(),to_timestamp(send_at))+interval '30 days'
             WHERE {MATCHING} AND community_id=$6",
        );
        let communities: Vec<Uuid> = sqlx::query_scalar(sqlx::AssertSqlSafe(discover))
            .bind(issuer)
            .bind(target.as_slice())
            .bind(revoke_through)
            .bind(recorded_at)
            .bind(until)
            .fetch_all(&mut *tx)
            .await?;
        for community in communities {
            guard_community_write(&mut tx, CommunityId::from_uuid(community)).await?;
            sqlx::query(sqlx::AssertSqlSafe(cancel.as_str()))
                .bind(issuer)
                .bind(target.as_slice())
                .bind(revoke_through)
                .bind(recorded_at)
                .bind(until)
                .bind(community)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
