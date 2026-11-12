//! Owned workflow writes. Preview rows cannot be promoted into an execution run.
use super::*;
use crate::entity_deletion::EntityLease;

impl MediaDeletionStore {
    /// Check every proposed departing source against the approved community
    /// inventory. Discovery may find departed communities that intake omitted;
    /// discovering them does not silently extend an existing approval.
    pub async fn validate_removal_scope(&self, run: &MediaRun) -> Result<()> {
        let lease = run
            .lease
            .as_ref()
            .ok_or_else(|| invalid("preview cannot authorize removal"))?;
        let mut tx = self.pool.begin().await?;
        let request = super::super::entity_deletion::lock_owned(&mut tx, lease).await?;
        self.lock(&mut tx, run).await?;
        let inventory = crate::user_deletion::UserDeletionInventory::read(&request)?;
        let approved: std::collections::BTreeSet<_> =
            inventory.communities.iter().map(|(id, _)| *id).collect();
        let evidence: Vec<Value> = sqlx::query_scalar(
            "SELECT evidence_json FROM entity_deletion_media_files WHERE stage_id=$1",
        )
        .bind(run.id)
        .fetch_all(&mut *tx)
        .await?;
        for file in evidence {
            let sources: Vec<MediaSource> = serde_json::from_value(file["sources"].clone())?;
            for source in sources
                .iter()
                .filter(|s| s.selected_subject && !s.surviving)
            {
                if !source.community.is_some_and(|id| approved.contains(&id)) {
                    return Err(invalid(
                        "discovered source is outside the approved community inventory",
                    ));
                }
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Verify the approved request lease and current execution-stage ownership.
    pub async fn verify_execution(&self, run: &MediaRun) -> Result<()> {
        if run.lease.is_none() {
            return Err(invalid("preview is not executable"));
        }
        let mut tx = self.pool.begin().await?;
        self.lock(&mut tx, run).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Bind a caller-started, approved purge_media stage to its saved inventory.
    /// The caller must keep all source and storage writers stopped throughout
    /// execution; the request lease only coordinates this deletion request.
    pub async fn execution_run(
        &self,
        lease: &EntityLease,
        stage: Uuid,
        binding: Value,
    ) -> Result<MediaRun> {
        let mut tx = self.pool.begin().await?;
        let request = super::super::entity_deletion::lock_owned(&mut tx, lease).await?;
        super::super::entity_deletion::validate_approved_inventory(&mut tx, &request).await?;
        let metadata:Value=sqlx::query_scalar("SELECT metadata_json FROM entity_deletion_stage_history WHERE id=$1 AND request_id=$2 AND stage='purge_media' AND is_current AND status='running' FOR UPDATE")
            .bind(stage).bind(lease.request_id).fetch_one(&mut *tx).await?;
        if metadata.get("binding").is_some_and(|b| b != &binding) {
            return Err(invalid("execution media binding changed"));
        }
        sqlx::query("UPDATE entity_deletion_stage_history SET metadata_json=metadata_json || $2,updated_at=now() WHERE id=$1")
            .bind(stage).bind(json!({"binding":binding,"generation":lease.generation})).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(MediaRun {
            id: stage,
            generation: lease.generation,
            binding,
            lease: Some(lease.clone()),
        })
    }

    /// Atomically save a complete decision with its immutable target list.
    /// Only unknown rows may acquire a plan; retries never replace saved targets.
    pub async fn save_execution_decision(&self, run: &MediaRun, file: &MediaFile) -> Result<()> {
        if run.lease.is_none() {
            return Err(invalid("preview is not executable"));
        }
        self.save_decision(run, file).await
    }

    pub(super) async fn save_decision(&self, run: &MediaRun, file: &MediaFile) -> Result<()> {
        if !matches!(
            file.status.as_str(),
            "unknown" | "dont_delete" | "to_be_deleted"
        ) || serde_json::to_vec(file)?.len() > MAX_FILE_BYTES
        {
            return Err(invalid("invalid media decision"));
        }
        let targets: Vec<MediaTarget> = serde_json::from_value(file.delete_targets_json.clone())?;
        if (file.status == "to_be_deleted") != !targets.is_empty() {
            return Err(invalid("decision and targets disagree"));
        }
        for target in &targets {
            if target.version_id.is_empty() || target.version_id == "null" || target.key.is_empty()
            {
                return Err(invalid("deletion requires immutable version IDs"));
            }
        }
        let mut tx = self.pool.begin().await?;
        let metadata = self.lock(&mut tx, run).await?;
        if metadata["media_discovery"]["complete"] != true {
            return Err(invalid("discovery unfinished"));
        }
        let changed=sqlx::query("UPDATE entity_deletion_media_files SET status=$4,evidence_json=$5,delete_targets_json=$6,reason=$7,last_error=$8,updated_at=now() WHERE stage_id=$1 AND storage_namespace=$2 AND file_hash=$3 AND status='unknown'")
            .bind(run.id).bind(&file.storage_namespace).bind(&file.file_hash).bind(&file.status).bind(&file.evidence_json).bind(&file.delete_targets_json).bind(&file.reason).bind(&file.last_error).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(invalid("decision row is no longer unknown"));
        }
        tx.commit().await?;
        Ok(())
    }

    /// Record confirmed absence or a retry error without changing saved targets.
    pub async fn save_execution_outcome(
        &self,
        run: &MediaRun,
        file: &MediaFile,
        error: Option<&str>,
    ) -> Result<()> {
        if run.lease.is_none() {
            return Err(invalid("preview is not executable"));
        }
        let mut tx = self.pool.begin().await?;
        self.lock(&mut tx, run).await?;
        let error = error.map(|s| s.chars().take(2048).collect::<String>());
        let changed=sqlx::query("UPDATE entity_deletion_media_files SET status=CASE WHEN $5::text IS NULL THEN 'deleted' ELSE 'to_be_deleted' END,last_error=$5,updated_at=now() WHERE stage_id=$1 AND storage_namespace=$2 AND file_hash=$3 AND delete_targets_json=$4 AND status='to_be_deleted'")
            .bind(run.id).bind(&file.storage_namespace).bind(&file.file_hash).bind(&file.delete_targets_json).bind(error).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(invalid("execution row or targets changed"));
        }
        tx.commit().await?;
        Ok(())
    }

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
}
