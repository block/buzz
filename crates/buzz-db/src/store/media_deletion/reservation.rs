//! Per-file write exclusion. A committed reservation outlives every worker.
//! Writers hold shared locks through their mutation; reservation installation
//! takes the exclusive counterpart before publishing the durable flag.
use super::*;
use sha2::{Digest, Sha256};
use sqlx::PgConnection;

fn lock_key(hash: &str) -> i64 {
    let digest = Sha256::digest(format!("buzz:media-deletion:v1:{hash}"));
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(bytes)
}

/// Guard hashes on a READ COMMITTED transaction before adding references or
/// writing storage. The caller must retain the transaction through the write.
pub async fn guard_hashes(conn: &mut PgConnection, hashes: &[String]) -> Result<()> {
    if hashes.is_empty() {
        return Ok(());
    }
    if hashes.len() > 10_000
        || hashes
            .iter()
            .any(|h| !buzz_core::media_inventory::valid_hash(h))
    {
        return Err(invalid("invalid media write hash set"));
    }
    let isolation: String = sqlx::query_scalar("SELECT current_setting('transaction_isolation')")
        .fetch_one(&mut *conn)
        .await?;
    if isolation != "read committed" {
        return Err(invalid("media write guard requires READ COMMITTED"));
    }
    let mut keys: Vec<_> = hashes.iter().map(|h| lock_key(h)).collect();
    keys.sort_unstable();
    keys.dedup();
    let timeout: String = sqlx::query_scalar("SELECT current_setting('lock_timeout')")
        .fetch_one(&mut *conn)
        .await?;
    sqlx::query("SELECT set_config('lock_timeout', (EXTRACT(EPOCH FROM LEAST(NULLIF(current_setting('lock_timeout')::interval, interval '0'), interval '5 seconds')) * 1000)::bigint::text, true)")
        .execute(&mut *conn).await?;
    sqlx::query("SELECT pg_advisory_xact_lock_shared(k) FROM unnest($1::bigint[]) AS k ORDER BY k")
        .bind(keys)
        .execute(&mut *conn)
        .await?;
    let reserved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_deletion_media_files WHERE file_hash=ANY($1) AND reserved)")
        .bind(hashes).fetch_one(&mut *conn).await?;
    if reserved {
        return Err(DbError::AccessDenied(
            "file is being deleted; retry after deletion finishes".into(),
        ));
    }
    sqlx::query("SELECT set_config('lock_timeout',$1,true)")
        .bind(timeout)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Guard every canonical hash appearing in a new media-bearing payload. This
/// intentionally also recognizes foreign-host URLs: a reserved hash must not
/// acquire a new local use through aliases or embedded/animated-avatar JSON.
pub async fn guard_payload(conn: &mut PgConnection, value: &Value) -> Result<()> {
    let hashes = buzz_core::media_inventory::write_hashes(value).map_err(|e| invalid(&e))?;
    guard_hashes(conn, &hashes.into_iter().collect::<Vec<_>>()).await
}

impl MediaDeletionStore {
    /// Remove only captured, departing SQL uploader claims. Audit history and
    /// sticky publication records remain intact, including for retained files.
    pub async fn cleanup_uploaders(&self, run: &MediaRun) -> Result<()> {
        if run.lease.is_none() {
            return Err(invalid("preview cannot remove uploader proofs"));
        }
        let mut tx = self.pool.begin().await?;
        self.lock(&mut tx, run).await?;
        let rows: Vec<(String, String, Value)> = sqlx::query_as("SELECT file_hash,status,evidence_json FROM entity_deletion_media_files WHERE stage_id=$1 FOR UPDATE")
            .bind(run.id).fetch_all(&mut *tx).await?;
        for (hash, status, evidence) in rows {
            if !matches!(status.as_str(), "dont_delete" | "deleted") {
                return Err(invalid("media work unfinished"));
            }
            let sources: Vec<MediaSource> = serde_json::from_value(evidence["sources"].clone())?;
            for source in sources
                .iter()
                .filter(|s| s.table == "media_uploaders" && s.selected_subject && !s.surviving)
            {
                if source.key[0] != hash {
                    return Err(invalid("uploader proof hash mismatch"));
                }
                let actor = source.key[1]
                    .as_str()
                    .ok_or_else(|| invalid("invalid uploader proof key"))?;
                sqlx::query("DELETE FROM media_uploaders WHERE community_id=$1 AND sha256=$2 AND pubkey=decode($3,'hex')")
                    .bind(source.community).bind(&hash).bind(actor).execute(&mut *tx).await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Reserve this stage's entire inventory before rechecking live uses. A retry
    /// preserves its own reservations; another deletion cannot claim the same hash.
    pub async fn reserve(&self, run: &MediaRun) -> Result<()> {
        if run.lease.is_none() {
            return Err(invalid("preview cannot reserve live files"));
        }
        let files = self
            .load(run)
            .await?
            .ok_or_else(|| invalid("discovery unfinished"))?;
        // Bound PostgreSQL's advisory-lock table use. Partial reservation is safe:
        // no decision/deletion runs until every batch has committed and live uses
        // have been scanned again. A retry preserves the committed reservations.
        let hashes: Vec<_> = files
            .iter()
            .filter(|f| matches!(f.status.as_str(), "unknown" | "to_be_deleted"))
            .map(|f| f.file_hash.clone())
            .collect();
        for batch in hashes.chunks(256) {
            let mut tx = self.pool.begin().await?;
            sqlx::query("SET LOCAL lock_timeout='5s'")
                .execute(&mut *tx)
                .await?;
            self.lock(&mut tx, run).await?;
            let mut keys: Vec<_> = batch.iter().map(|h| lock_key(h)).collect();
            keys.sort_unstable();
            keys.dedup();
            sqlx::query(
                "SELECT pg_advisory_xact_lock(k) FROM unnest($1::bigint[]) AS k ORDER BY k",
            )
            .bind(keys)
            .execute(&mut *tx)
            .await?;
            let conflict: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_deletion_media_files WHERE file_hash=ANY($1) AND reserved AND stage_id<>$2)")
                .bind(batch).bind(run.id).fetch_one(&mut *tx).await?;
            if conflict {
                return Err(invalid("file reserved by another deletion"));
            }
            sqlx::query("UPDATE entity_deletion_media_files SET reserved=true WHERE stage_id=$1 AND file_hash=ANY($2) AND status IN ('unknown','to_be_deleted')")
                .bind(run.id).bind(batch).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        let mut tx = self.pool.begin().await?;
        self.lock(&mut tx, run).await?;
        let missing: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_deletion_media_files WHERE stage_id=$1 AND status IN ('unknown','to_be_deleted') AND NOT reserved)")
            .bind(run.id).fetch_one(&mut *tx).await?;
        if missing {
            return Err(invalid("media reservation missing"));
        }
        sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{media_reservations_complete}','true'::jsonb) WHERE id=$1")
            .bind(run.id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Check the durable receipt and current ownership before each external action.
    /// Only reserve publishes it, after checking every pending row under the job lock.
    pub async fn verify_reserved(&self, run: &MediaRun) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let metadata = self.lock(&mut tx, run).await?;
        if metadata["media_reservations_complete"] != true {
            return Err(invalid("media reservations incomplete"));
        }
        tx.commit().await?;
        Ok(())
    }

    /// Release only after all decisions and deletions finish, and after the caller
    /// has removed departing uploader proofs. Errors preserve the reservations.
    pub async fn release_reserved(&self, run: &MediaRun) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        self.lock(&mut tx, run).await?;
        let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_deletion_media_files WHERE stage_id=$1 AND status NOT IN ('dont_delete','deleted'))")
            .bind(run.id).fetch_one(&mut *tx).await?;
        if pending {
            return Err(invalid("media work unfinished"));
        }
        sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=metadata_json-'media_reservations_complete' WHERE id=$1")
            .bind(run.id).execute(&mut *tx).await?;
        sqlx::query("UPDATE entity_deletion_media_files SET reserved=false WHERE stage_id=$1")
            .bind(run.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
