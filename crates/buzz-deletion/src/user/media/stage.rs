//! Concrete PostgreSQL/S3 handler used by an approved workflow stage.
use super::*;
use buzz_db::{
    media_deletion::{MediaScanBudget, MediaSource, SubjectScope},
    Db,
};
use std::collections::{BTreeMap, BTreeSet};

/// Stable inputs bound to a media stage across retries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageOptions {
    /// Human public key; must equal the approved request target.
    pub human: String,
    /// Include currently owned agents, scoped independently per community.
    pub include_owned_agents: bool,
    /// Identity of the actual bucket and endpoint being operated on.
    pub storage_namespace: String,
    /// Additional media URL aliases for reference parsing.
    pub media_bases: Vec<String>,
    /// Resource bounds; exceeding them never publishes a partial inventory.
    pub scan_budget: MediaScanBudget,
    /// Print-only stops after the same decision procedure, before deletion.
    pub mode: Mode,
}

/// Run all three steps against real PostgreSQL and storage. The workflow owns
/// the approved request lease and heartbeat, and starts the purge_media stage.
/// PrintOnly observes without writing source data or media journal records and
/// fails if any decision is unresolved. Delete gates uploads, drains earlier
/// requests, reserves files, rechecks live uses, and deletes exact versions.
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
    let store = db.media_deletion_store();
    let run = store
        .execution_run(lease, stage, serde_json::to_value(&options)?)
        .await?;
    let executing = options.mode != Mode::PrintOnly;
    if store.load(&run).await?.is_none() {
        let mut scan = scan(db, &options).await?;
        if executing {
            store.gate_uploads(&run, &scan.scope).await?;
            store.verify_upload_gate(&run, &scan.scope).await?;
            // The first scan resolved ownership; the second collects evidence
            // after new subject uploads are gated and older uploads have drained.
            let gated = scan.scope;
            scan = self::scan(db, &options).await?;
            if serde_json::to_value(&scan.scope)? != serde_json::to_value(&gated)? {
                bail!("agent ownership changed while gating uploads");
            }
        }
        storage::supplement(&mut scan, media, &options.storage_namespace).await?;
        store.publish(&run, &scan.files, json!({"tables":scan.scanned,"scope":scan.scope,"unresolved":scan.unresolved,"database_snapshot_at":scan.database_snapshot_at})).await?;
    }
    let receipt = store.receipt(&run).await?;
    let scope: SubjectScope = serde_json::from_value(receipt["scans"]["scope"].clone())?;
    let saved = store.load(&run).await?.context("discovery missing")?;
    let mut current: BTreeMap<String, MediaFile> = saved
        .iter()
        .cloned()
        .map(|f| (f.file_hash.clone(), f))
        .collect();
    if executing {
        store.verify_upload_gate(&run, &scope).await?;
        store.validate_removal_scope(&run).await?;
        store.reserve(&run).await?;
        let mut live = scan(db, &options).await?;
        storage::supplement(&mut live, media, &options.storage_namespace).await?;
        if serde_json::to_value(&live.scope)? != serde_json::to_value(&scope)? {
            bail!("saved subject scope changed");
        }
        let live: BTreeMap<_, _> = live
            .files
            .into_iter()
            .map(|f| (f.file_hash.clone(), f))
            .collect();
        for file in &saved {
            if file.status == "unknown" {
                if let Some(fresh) = live.get(&file.file_hash) {
                    current.insert(file.file_hash.clone(), fresh.clone());
                } else {
                    bail!("candidate attribution disappeared before its decision");
                }
            } else if file.status == "to_be_deleted" {
                // Retry keeps the frozen targets even when earlier deletes removed
                // upload metadata. Every remaining source must still be accounted for.
                if let Some(fresh) = live.get(&file.file_hash) {
                    validate_remaining_sources(file, fresh)?;
                }
            }
        }
        if live.keys().any(|hash| !current.contains_key(hash)) {
            bail!("new subject files appeared after discovery; saved inventory cannot be extended");
        }
    }
    let markers = current
        .values()
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
        current,
        executing,
        markers,
    };
    handle(&store, &run, options.mode, &concrete).await?;
    Ok(json!({"supported_scope_exhausted":true,"account_erasure_complete":false}))
}

pub(super) async fn scan(db: &Db, options: &StageOptions) -> Result<buzz_db::media_deletion::MediaScan> {
    Ok(db
        .scan_user_media_with_budget(
            &options.human,
            options.include_owned_agents,
            &options.storage_namespace,
            &options.media_bases,
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
    current: BTreeMap<String, MediaFile>,
    executing: bool,
    markers: BTreeSet<(String, String)>,
}

impl ProtectedMedia for StorageDeletion<'_> {
    async fn verify(&self, run: &MediaRun) -> Result<()> {
        if self.executing {
            self.store.verify_upload_gate(run, &self.scope).await?;
            self.store.verify_reserved(run).await?;
        }
        Ok(())
    }
    async fn ready(&self, run: &MediaRun) -> Result<()> {
        self.store.validate_removal_scope(run).await?;
        self.verify(run).await
    }
    async fn decide(&self, _: &MediaRun, file: &MediaFile) -> Result<MediaFile> {
        let fresh = self
            .current
            .get(&file.file_hash)
            .context("missing current file evidence")?;
        decision::decide(self.media, fresh.clone()).await
    }
    async fn present(&self, target: &MediaTarget) -> Result<bool> {
        version_present(self.media, target).await
    }
    async fn delete(&self, target: &MediaTarget) -> Result<()> {
        if !self.executing {
            bail!("print-only cannot delete");
        }
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
        if !self.executing {
            bail!("print-only cannot remove uploader claims");
        }
        self.store.cleanup_uploaders(run).await?;
        self.store.release_reserved(run).await?;
        Ok(())
    }
}
