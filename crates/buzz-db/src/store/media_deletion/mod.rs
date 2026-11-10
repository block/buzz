//! One-table media journal. Discovery publication is all-or-none, including zero files.
use crate::{Db, DbError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

mod execution;
mod reservation;
pub use reservation::{guard_hashes, guard_payload};
mod scan;
pub use scan::{MediaScan, MediaScanBudget, MediaSource, SubjectScope};

/// Maximum journal payload per file; exceeding it fails rather than truncates.
pub const MAX_FILE_BYTES: usize = 1_048_576;
/// Maximum number of distinct candidates in an atomic publication.
pub const MAX_FILES: usize = 100_000;

/// Exact, immutable storage target. A null/unversioned identifier is not executable.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaTarget {
    /// Storage object path.
    pub key: String,
    /// Immutable storage version.
    pub version_id: String,
}

/// One logical file and its evidence, decision and selected objects.
#[derive(Clone, Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct MediaFile {
    /// Shared bucket/namespace identity.
    pub storage_namespace: String,
    /// SHA-256 of file content.
    pub file_hash: String,
    /// unknown, dont_delete, to_be_deleted or deleted.
    pub status: String,
    /// Source keys, fingerprints and checks; never source bodies.
    pub evidence_json: Value,
    /// Selected storage keys and versions.
    pub delete_targets_json: Value,
    /// Explanation of the decision, including any retained components.
    pub reason: Option<String>,
    /// Unresolved check or execution error.
    pub last_error: Option<String>,
}

/// Fenced ownership of a standalone observation stage.
#[derive(Clone, Debug)]
pub struct MediaRun {
    /// Stable stage identifier supplied by the operator for recovery.
    pub id: Uuid,
    /// Increments on every invocation; stale workers cannot publish.
    pub generation: i64,
    pub(super) lease: Option<crate::entity_deletion::EntityLease>,
    /// Bound subject, namespace, source identity, parser version and mode.
    pub binding: Value,
}

/// Writer connection to the operator control database (may differ from source DB).
#[derive(Clone)]
pub struct MediaDeletionStore {
    pool: PgPool,
}
impl Db {
    /// Access the operator-global media journal.
    pub fn media_deletion_store(&self) -> MediaDeletionStore {
        MediaDeletionStore {
            pool: self.pool().clone(),
        }
    }
}
fn invalid(message: &str) -> DbError {
    DbError::DeletionSafety(message.into())
}

impl MediaDeletionStore {
    /// Start/resume an isolated preview. It cannot be approved or executed by the
    /// user worker: its parent is terminal and no stage is current.
    pub async fn open_preview(&self, id: Uuid, binding: Value) -> Result<MediaRun> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET LOCAL lock_timeout = '5s'")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO entity_deletion_requests
            (id,target,status,requested_by,reason,completed_at)
            VALUES ($1,$2,'completed','media-preview','Non-executable media observation',now())
            ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(format!("media-preview:{id}"))
        .execute(&mut *tx)
        .await?;
        let parent: (String, String) = sqlx::query_as(
            "SELECT target,status FROM entity_deletion_requests WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if parent != (format!("media-preview:{id}"), "completed".into()) {
            return Err(invalid("run ID belongs to another workflow"));
        }
        sqlx::query(
            "INSERT INTO entity_deletion_stage_history
            (id,request_id,stage,is_current,metadata_json)
            VALUES ($1,$1,'media_preview',false,$2) ON CONFLICT (id) DO NOTHING",
        )
        .bind(id)
        .bind(json!({"binding":binding,"generation":0}))
        .execute(&mut *tx)
        .await?;
        let (metadata, stage, current): (Value,String,bool) = sqlx::query_as("SELECT metadata_json,stage,is_current FROM entity_deletion_stage_history WHERE id=$1 AND request_id=$1 FOR UPDATE")
            .bind(id).fetch_one(&mut *tx).await?;
        if metadata["binding"] != binding || stage != "media_preview" || current {
            return Err(invalid(
                "run binding changed; use a new run ID for a new observation",
            ));
        }
        let generation = metadata["generation"]
            .as_i64()
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| invalid("invalid media generation"))?;
        sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=jsonb_set(metadata_json,'{generation}',$2), updated_at=now() WHERE id=$1")
            .bind(id).bind(json!(generation)).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(MediaRun {
            id,
            generation,
            binding,
            lease: None,
        })
    }

    async fn lock(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        run: &MediaRun,
    ) -> Result<Value> {
        if let Some(lease) = &run.lease {
            let request = super::entity_deletion::lock_owned(tx, lease).await?;
            super::entity_deletion::validate_approved_inventory(tx, &request).await?;
        }
        let metadata: Value = sqlx::query_scalar("SELECT metadata_json FROM entity_deletion_stage_history WHERE id=$1 AND ((stage='media_preview' AND NOT is_current AND request_id=$1 AND $2 IS NULL) OR (stage='purge_media' AND is_current AND request_id=$2 AND status='running')) FOR UPDATE")
            .bind(run.id).bind(run.lease.as_ref().map(|l|l.request_id)).fetch_one(&mut **tx).await?;
        if metadata["generation"] != run.generation || metadata["binding"] != run.binding {
            return Err(invalid("media stage ownership lost"));
        }
        Ok(metadata)
    }

    /// Read completion plus rows from one snapshot. Counts detect missing rows;
    /// an empty table without the committed marker is never successful discovery.
    pub async fn load(&self, run: &MediaRun) -> Result<Option<Vec<MediaFile>>> {
        let mut tx = self.pool.begin().await?;
        let metadata = self.lock(&mut tx, run).await?;
        let rows: Vec<MediaFile> = sqlx::query_as("SELECT storage_namespace,file_hash,status,evidence_json,delete_targets_json,reason,last_error FROM entity_deletion_media_files WHERE stage_id=$1 ORDER BY storage_namespace,file_hash")
            .bind(run.id).fetch_all(&mut *tx).await?;
        let complete = metadata["media_discovery"]["complete"] == true;
        if (!complete && !rows.is_empty())
            || (complete && metadata["media_discovery"]["candidate_count"] != json!(rows.len()))
        {
            return Err(invalid("media discovery marker/count disagrees with rows"));
        }
        tx.commit().await?;
        Ok(complete.then_some(rows))
    }

    /// Return the committed scan receipt, including scope and zero-result scans.
    pub async fn receipt(&self, run: &MediaRun) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        let metadata = self.lock(&mut tx, run).await?;
        tx.commit().await?;
        Ok(metadata["media_discovery"].clone())
    }

    /// Insert every candidate and the completion marker in ONE transaction.
    /// Retrying after a lost commit acknowledgement preserves the first inventory.
    pub async fn publish(&self, run: &MediaRun, files: &[MediaFile], scans: Value) -> Result<()> {
        if files.len() > MAX_FILES {
            return Err(invalid("media candidate limit exceeded"));
        }
        let total_bytes = files.iter().try_fold(0usize, |n, f| {
            serde_json::to_vec(f).map(|bytes| n.saturating_add(bytes.len()))
        })?;
        if total_bytes > 128 * 1024 * 1024 {
            return Err(invalid("media inventory exceeds 128 MiB"));
        }
        let mut tx = self.pool.begin().await?;
        let metadata = self.lock(&mut tx, run).await?;
        let existing: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM entity_deletion_media_files WHERE stage_id=$1",
        )
        .bind(run.id)
        .fetch_one(&mut *tx)
        .await?;
        if metadata["media_discovery"]["complete"] == true {
            if metadata["media_discovery"]["candidate_count"] != json!(existing) {
                return Err(invalid("published media row count changed"));
            }
            return Ok(());
        }
        if existing != 0 {
            return Err(invalid("unpublished media rows exist"));
        }
        for file in files {
            if file.status != "unknown"
                || file.delete_targets_json != json!([])
                || serde_json::to_vec(file)?.len() > MAX_FILE_BYTES
            {
                return Err(invalid("invalid or oversized discovery candidate"));
            }
            sqlx::query("INSERT INTO entity_deletion_media_files (stage_id,storage_namespace,file_hash,evidence_json) VALUES ($1,$2,$3,$4)")
                .bind(run.id).bind(&file.storage_namespace).bind(&file.file_hash).bind(&file.evidence_json).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE entity_deletion_stage_history SET status='running', started_at=COALESCE(started_at,now()), metadata_json=jsonb_set(metadata_json,'{media_discovery}',$2),updated_at=now() WHERE id=$1")
            .bind(run.id).bind(json!({"complete":true,"candidate_count":files.len(),"scans":scans})).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Save the same decisions and exact targets as execution, but keep the
    /// isolated preview non-executable. It never acquires deletion authority.
    pub async fn save_preview_decision(&self, run: &MediaRun, file: &MediaFile) -> Result<()> {
        if run.lease.is_some() {
            return Err(invalid("execution must use execution decision writes"));
        }
        self.save_decision(run, file).await
    }
}

#[cfg(test)]
mod postgres_tests;
