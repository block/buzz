//! Append-only evidence read under the caller's admitted land transaction.
use crate::{AdmittedTx, DbError, Result};
use buzz_core::governance::{Evidence, RepoCoordinate};
use sqlx::Row;

/// Maximum evidence history evaluated in one land; exceeding it is an error.
pub const MAX_EVIDENCE_ROWS: i64 = 10_000;

/// Producer statement with the authoritative audit ordering.
#[derive(Clone, Debug)]
pub struct EvidenceRow {
    /// Sequence assigned when the statement was accepted.
    pub seq: i64,
    /// Producer public key; standing must be resolved at land time.
    pub actor: String,
    /// Exact signed subject and statement, without reusable credentials.
    pub evidence: Evidence,
}

/// Read after holding the repository's land fence. Do not fold withdrawals or
/// filter by ref, base, producer eligibility or trust here. Only candidate
/// heads can count; unrelated historical commits do not consume the cap.
pub async fn read_in_transaction(
    tx: &mut AdmittedTx,
    repo: &RepoCoordinate,
    heads: &[String],
) -> Result<Vec<EvidenceRow>> {
    let community = tx.community();
    let rows = sqlx::query(
        "SELECT seq, encode(actor_pubkey,'hex') AS actor, detail->'evidence' AS evidence \
         FROM audit_log WHERE community_id=$1 AND action='git_evidence' \
         AND detail->'evidence'->>'repo'=$2 AND detail->'evidence'->>'head'=ANY($3) ORDER BY seq LIMIT $4",
    )
    .bind(community.as_uuid())
    .bind(repo.to_string())
    .bind(heads)
    .bind(MAX_EVIDENCE_ROWS + 1)
    .fetch_all(tx.conn())
    .await?;
    if rows.len() > MAX_EVIDENCE_ROWS as usize {
        return Err(DbError::InvalidData(
            "unavailable: evidence history exceeds limit".into(),
        ));
    }
    rows.into_iter()
        .map(|row| {
            let evidence: Evidence = serde_json::from_value(row.try_get("evidence")?)?;
            evidence.validate().map_err(DbError::InvalidData)?;
            if evidence.community != *community.as_uuid() || evidence.repo != repo.to_string() {
                return Err(DbError::InvalidData("invalid stored evidence scope".into()));
            }
            Ok(EvidenceRow {
                seq: row.try_get("seq")?,
                actor: row.try_get("actor")?,
                evidence,
            })
        })
        .collect()
}

/// Validate and fence an evidence append. The caller must append its audit row
/// and commit in this same transaction. True means an already recorded event.
/// `grant` is the Git grant the event's signer presented, verified.
pub async fn prepare(
    tx: &mut AdmittedTx,
    event: &nostr::Event,
    grant: Option<&buzz_core::governance::GitGrant>,
    open: bool,
) -> Result<(Evidence, bool)> {
    use super::governance;
    use buzz_core::governance::DocumentId;
    let evidence = Evidence::parse(event).map_err(DbError::InvalidData)?;
    let community = tx.community();
    if evidence.community != *community.as_uuid() {
        return Err(DbError::AccessDenied(
            "restricted: evidence community mismatch".into(),
        ));
    }
    let repo: RepoCoordinate = evidence.repo.parse().map_err(DbError::InvalidData)?;
    governance::fence_community_shared(tx, community).await?;
    // Every kind can supersede evidence a land counted. One exclusive rule.
    governance::fence_repo_exclusive(tx, &repo).await?;
    governance::authorize_read(
        tx,
        &DocumentId::Rules(repo),
        event.pubkey.as_bytes(),
        grant,
        open,
    )
    .await?;
    let replay: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM audit_log WHERE community_id=$1 AND action='git_evidence' AND object_id=$2)")
        .bind(community.as_uuid()).bind(event.id.to_hex()).fetch_one(tx.conn()).await?;
    if !replay {
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(tx.conn())
                .await?;
        let timestamp = i64::try_from(event.created_at.as_secs())
            .map_err(|_| DbError::InvalidData("invalid evidence time".into()))?;
        if now.abs_diff(timestamp) > 120 {
            return Err(DbError::InvalidData("invalid: stale evidence".into()));
        }
    }
    Ok((evidence, replay))
}
