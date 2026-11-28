//! Transaction-bound persistence for the audit chain. Hashing stays in buzz-audit.
use crate::{AdmittedTx, Result};
use chrono::{DateTime, Utc};

/// Lock the community audit chain and return its current sequence/hash.
pub async fn lock_head(tx: &mut AdmittedTx) -> Result<Option<(i64, Vec<u8>)>> {
    let community = tx.community();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("buzz_audit:{community}"))
        .execute(tx.conn())
        .await?;
    Ok(sqlx::query_as(
        "SELECT seq, hash FROM audit_log WHERE community_id = $1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(community.as_uuid())
    .fetch_optional(tx.conn())
    .await?)
}

/// A hash-complete audit row supplied by the audit service.
pub struct Append<'a> {
    /// Next sequence under the chain lock.
    pub seq: i64,
    /// Hash format.
    pub hash_version: i16,
    /// Complete computed hash.
    pub hash: &'a [u8],
    /// Previous chain hash.
    pub prev_hash: Option<&'a [u8]>,
    /// Stable action name.
    pub action: &'a str,
    /// Actor's key bytes.
    pub actor: Option<&'a [u8]>,
    /// Object identifier.
    pub object: Option<&'a str>,
    /// Private retained details.
    pub detail: &'a serde_json::Value,
    /// Timestamp used in the hash.
    pub created_at: DateTime<Utc>,
    /// Optional decision expiry, checked by the insertion statement.
    pub valid_until: Option<i64>,
}

/// Append under the caller's admitted transaction and previously acquired chain lock.
pub async fn append(tx: &mut AdmittedTx, row: Append<'_>) -> Result<()> {
    let community = tx.community();
    let result = sqlx::query("INSERT INTO audit_log (community_id, seq, hash, prev_hash, action, actor_pubkey, object_id, detail, created_at, hash_version) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10 WHERE $11::bigint IS NULL OR clock_timestamp() < to_timestamp($11::double precision)")
        .bind(community.as_uuid()).bind(row.seq).bind(row.hash).bind(row.prev_hash)
        .bind(row.action).bind(row.actor).bind(row.object).bind(row.detail)
        .bind(row.created_at).bind(row.hash_version).bind(row.valid_until).execute(tx.conn()).await?;
    if result.rows_affected() != 1 {
        return Err(crate::DbError::AccessDenied(
            "restricted: governance authority expired".into(),
        ));
    }
    Ok(())
}
