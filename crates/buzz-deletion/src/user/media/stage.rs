//! Concrete PostgreSQL/S3 handler used by an approved workflow stage.
use super::*;
use buzz_db::{
    media_deletion::{MediaScanBudget, MediaSource, SubjectScope},
    Db,
};
use std::collections::BTreeSet;

/// Stable inputs bound to a media stage across retries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageOptions {
    /// Human public key; must equal the approved request target.
    pub human: String,
    /// Include currently owned agents, scoped independently per community.
    pub include_owned_agents: bool,
    /// Identity of the actual bucket and endpoint being operated on.
    pub storage_namespace: String,
    /// Resource bounds; exceeding them never publishes a partial inventory.
    pub scan_budget: MediaScanBudget,
    /// Print-only stops after the same decision procedure, before deletion.
    pub mode: Mode,
}

impl StageOptions {
    /// Bind immutable policy semantics along with operator inputs.
    pub(super) fn binding(&self) -> Result<serde_json::Value> {
        let mut binding = serde_json::to_value(self)?;
        binding["policy"] = json!("upload-ownership-v1");
        Ok(binding)
    }
}

/// Run all three steps against real PostgreSQL and storage. The workflow owns
/// the approved request lease and heartbeat, and starts the purge_media stage.
/// PrintOnly observes without writing source data or media journal records and
/// fails if any decision is unresolved. Delete verifies the completed user gate,
/// reserves hashes, revalidates upload claims, and deletes exact versions.
/// The caller persists the returned summary in its own workflow journal.
pub async fn run_stage(
    db: &Db,
    media: &MediaStorage,
    lease: &buzz_db::entity_deletion::EntityLease,
    stage: Uuid,
    options: StageOptions,
) -> Result<serde_json::Value> {
    if options.mode == Mode::PrintOnly {
        let proposal = preview(db, media, &options).await?;
        crate::print_json(&proposal)?;
        if proposal["decision_complete"] != true {
            bail!("media read-only scan has unresolved decisions; stage has not completed");
        }
        // Detailed evidence is printed; do not duplicate it in workflow metadata.
        return Ok(json!({"candidate_count":proposal["candidate_count"],
            "unknown_count":proposal["unknown_count"], "supported_scope_exhausted":true,
            "account_erasure_complete":false}));
    }
    let request = db.entity_deletion_store().get(lease.request_id).await?;
    if request.entity_type != "user" || request.target != options.human {
        bail!("media subject must match the approved user request");
    }
    let scope = db
        .user_media_scope(&options.human, options.include_owned_agents)
        .await?;
    db.user_deletion_store()
        .verify_user_gate(lease, &scope)
        .await?;
    let store = db.media_deletion_store();
    let run = store
        .execution_run(lease, stage, options.binding()?)
        .await?;
    let scan = pipeline::prepare(
        Some(db),
        media,
        &options,
        pipeline::Journal::Execution(&store, &run, &scope),
    )
    .await?;
    db.user_deletion_store()
        .verify_user_gate(lease, &scan.scope)
        .await?;
    let markers = scan
        .files
        .iter()
        .flat_map(|f| {
            f.evidence_json["observed_objects"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .filter(|o| o["kind"] == "delete_marker")
        .filter_map(|o| {
            Some((
                o["key"].as_str()?.to_owned(),
                o["version_id"].as_str()?.to_owned(),
            ))
        })
        .collect();
    let concrete = StorageDeletion {
        media,
        store: &store,
        scope,
        db,
        lease,
        options: &options,
        markers,
    };
    handle(&store, &run, options.mode, &concrete).await?;
    Ok(json!({"supported_scope_exhausted":true,"account_erasure_complete":false}))
}

pub(super) async fn scan(
    db: &Db,
    options: &StageOptions,
) -> Result<buzz_db::media_deletion::MediaScan> {
    Ok(db
        .scan_user_media_with_budget(
            &options.human,
            options.include_owned_agents,
            &options.storage_namespace,
            options.scan_budget,
        )
        .await?)
}

fn validate_remaining_sources(saved: &MediaFile, fresh: &MediaFile) -> Result<()> {
    let before: Vec<MediaSource> = serde_json::from_value(saved.evidence_json["sources"].clone())?;
    let after: Vec<MediaSource> = serde_json::from_value(fresh.evidence_json["sources"].clone())?;
    for source in after {
        if source.surviving
            || !before.iter().any(|old| {
                old.table == source.table
                    && old.community == source.community
                    && old.key == source.key
                    && old.fingerprint == source.fingerprint
                    && old.field == source.field
            })
        {
            bail!("source changed after deletion targets were saved");
        }
    }
    Ok(())
}

struct StorageDeletion<'a> {
    media: &'a MediaStorage,
    store: &'a MediaDeletionStore,
    scope: SubjectScope,
    db: &'a Db,
    lease: &'a buzz_db::entity_deletion::EntityLease,
    options: &'a StageOptions,
    markers: BTreeSet<(String, String)>,
}

impl ProtectedMedia for StorageDeletion<'_> {
    async fn verify(&self, run: &MediaRun) -> Result<()> {
        self.db
            .user_deletion_store()
            .verify_user_gate(self.lease, &self.scope)
            .await?;
        self.store.verify_reserved(run).await?;
        Ok(())
    }
    async fn ready(&self, run: &MediaRun) -> Result<()> {
        self.store.validate_removal_scope(run).await?;
        self.verify(run).await?;
        let saved = self.store.load(run).await?.context("discovery missing")?;
        let hashes: Vec<_> = saved
            .iter()
            .filter(|f| f.status == "to_be_deleted")
            .map(|f| f.file_hash.clone())
            .collect();
        if hashes.is_empty() {
            return Ok(());
        }
        // Separate from classification: only candidate DB upload evidence is
        // reread. Admission persists a claim before storage writes. The offline
        // prerequisite and completed drain exclude in-flight storage writers;
        // reservations exclude subsequent claims. Saved exact versions remain
        // immutable, including across interrupted deletions.
        let live = self
            .db
            .scan_media_uploads(
                &self.scope,
                &self.options.storage_namespace,
                Some(&hashes),
                self.options.scan_budget,
            )
            .await?;
        let saved: std::collections::BTreeMap<_, _> =
            saved.iter().map(|f| (f.file_hash.as_str(), f)).collect();
        for fresh in live.files {
            let before = saved
                .get(fresh.file_hash.as_str())
                .context("unexpected candidate")?;
            validate_remaining_sources(before, &fresh)?;
        }
        self.verify(run).await
    }
    async fn present(&self, target: &MediaTarget) -> Result<bool> {
        version_present(self.media, target).await
    }
    async fn delete(&self, target: &MediaTarget) -> Result<()> {
        if !self
            .markers
            .contains(&(target.key.clone(), target.version_id.clone()))
            && self
                .media
                .version_retention(&target.key, &target.version_id)
                .await?
                .prevents_deletion(chrono::Utc::now())
        {
            bail!("storage restriction changed; selected version retained");
        }
        delete_version(self.media, target).await
    }
    async fn cleanup(&self, run: &MediaRun) -> Result<()> {
        self.store.cleanup_uploaders(run).await?;
        self.store.release_reserved(run).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revalidation_rejects_new_uploaders_and_changed_evidence_but_allows_removed_records() {
        let source = MediaSource {
            table: "media_uploaders".into(),
            community: Some(Uuid::new_v4()),
            key: json!(["a".repeat(64), "b".repeat(64)]),
            fingerprint: "original".into(),
            field: "direct_hash".into(),
            selected_subject: true,
            upload_claim: true,
            surviving: false,
        };
        let saved = MediaFile {
            storage_namespace: "fixture".into(),
            file_hash: "a".repeat(64),
            status: "to_be_deleted".into(),
            evidence_json: json!({"sources":[source]}),
            delete_targets_json: json!([]),
            reason: None,
            last_error: None,
        };
        assert!(validate_remaining_sources(&saved, &saved).is_ok());
        let mut fresh = saved.clone();
        fresh.evidence_json["sources"] = json!([]);
        assert!(validate_remaining_sources(&saved, &fresh).is_ok());
        for change in [
            json!({"fingerprint":"changed"}),
            json!({"surviving":true,"selected_subject":false}),
            json!({"key":["new", "uploader"]}),
        ] {
            fresh = saved.clone();
            for (field, value) in change.as_object().unwrap() {
                fresh.evidence_json["sources"][0][field] = value.clone();
            }
            assert!(validate_remaining_sources(&saved, &fresh).is_err());
        }
    }
}
