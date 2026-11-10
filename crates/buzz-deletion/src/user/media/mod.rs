//! Direct media inventory and the shared three-step handler. The direct command
//! is observational; a workflow must supply durable protection before execution.
use anyhow::{bail, Context, Result};
use buzz_db::media_deletion::{MediaDeletionStore, MediaFile, MediaRun, MediaTarget};
use buzz_media::{MediaStorage, ObjectVersionRef};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;
mod command;
mod storage;
pub use command::{run, Options};

/// Output/execution policy for the media stage.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Print saved step-2 inventory and stop before any deletion.
    PrintOnly,
    /// Print saved step-2 inventory, then execute its protected targets.
    Print,
    /// Execute protected targets without printing inventory.
    NoPrint,
}

/// The workflow's safety boundary, distinct from its worker lease. The provider
/// must verify approved scope/removal fingerprints, storage retention/holds,
/// complete cross-community scans and crash-persistent all-writer reservations.
/// There is deliberately no permissive production implementation until those
/// writer integrations exist. The standalone command cannot mint this authority.
#[allow(async_fn_in_trait)]
pub trait ProtectedMedia {
    /// Validate the same durable protection and scope before each external action.
    async fn verify(&self, run: &MediaRun) -> Result<()>;
    /// Perform the final checks and return keep, unresolved or exact delete targets.
    /// Failure must leave the saved row unknown, never manufacture a keep decision.
    async fn decide(&self, run: &MediaRun, file: &MediaFile) -> Result<MediaFile>;
    /// Confirm whether this exact version (including delete markers) is present.
    async fn present(&self, target: &MediaTarget) -> Result<bool>;
    /// Delete only this exact version. Errors/timeout are retryable, never absence.
    async fn delete(&self, target: &MediaTarget) -> Result<()>;
    /// Remove only the exact departing uploader-proof rows after all selected
    /// storage work succeeds. Must be idempotent; preserve media_publications.
    async fn cleanup(&self, run: &MediaRun) -> Result<()>;
}

/// Run steps 2 and 3 over a committed inventory. The caller publishes step 1
/// through MediaDeletionStore::publish and owns workflow claim/heartbeat/routing.
/// PrintOnly always returns after printing, including when checks are unresolved.
pub async fn handle<P: ProtectedMedia>(
    store: &MediaDeletionStore,
    run: &MediaRun,
    mode: Mode,
    protected: &P,
) -> Result<()> {
    let files = store
        .load(run)
        .await?
        .context("media discovery has not completed")?;
    for file in files.iter().filter(|f| f.status == "unknown") {
        protected.verify(run).await?;
        let decision = match protected.decide(run, file).await {
            Ok(decision) => decision,
            Err(error) => {
                let mut pending = file.clone();
                pending.last_error = Some(format!("{error:#}"));
                pending
            }
        };
        if decision.file_hash != file.file_hash
            || decision.storage_namespace != file.storage_namespace
        {
            bail!("media decision changed file identity");
        }
        store.save_execution_decision(run, &decision).await?;
    }
    let files = store
        .load(run)
        .await?
        .context("media discovery disappeared")?;
    if mode != Mode::NoPrint {
        crate::print_json(&json!({"stage_id":run.id,"mode":mode,"files":files}))?;
    }
    if mode == Mode::PrintOnly {
        return Ok(());
    }
    if files.iter().any(|f| f.status == "unknown") {
        bail!("media decisions unresolved; step 3 cannot start");
    }
    for file in files.iter().filter(|f| f.status == "to_be_deleted") {
        let targets: Vec<MediaTarget> = serde_json::from_value(file.delete_targets_json.clone())?;
        let result = delete_targets(run, &targets, protected).await;
        match result {
            Ok(()) => store.save_execution_outcome(run, file, None).await?,
            Err(error) => {
                store
                    .save_execution_outcome(run, file, Some(&format!("{error:#}")))
                    .await?;
                return Err(error);
            }
        }
    }
    protected.verify(run).await?;
    protected.cleanup(run).await?;
    Ok(())
}

async fn delete_targets<P: ProtectedMedia>(
    run: &MediaRun,
    targets: &[MediaTarget],
    protected: &P,
) -> Result<()> {
    if targets.is_empty() {
        bail!("delete decision has no targets");
    }
    for target in targets {
        if target.version_id.is_empty() || target.version_id == "null" {
            bail!("unversioned deletion target");
        }
        protected.verify(run).await?;
        if protected.present(target).await? {
            protected.verify(run).await?;
            protected.delete(target).await?;
            if protected.present(target).await? {
                bail!("storage version still present after deletion");
            }
        }
    }
    protected.verify(run).await?;
    Ok(())
}

/// Exact-version storage operations for a workflow's protected adapter.
/// Listing proves absence of object versions and delete markers alike; a 403,
/// missing continuation or repeated page is an error, never an absent version.
pub async fn version_present(media: &MediaStorage, target: &MediaTarget) -> Result<bool> {
    let (mut key, mut version) = (None, None);
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..10_000 {
        let page = media
            .list_prefix_versions_page(&target.key, key.clone(), version.clone(), 1000)
            .await?;
        if page
            .entries
            .iter()
            .any(|e| e.key == target.key && e.version_id == target.version_id)
        {
            return Ok(true);
        }
        if !page.is_truncated {
            return Ok(false);
        }
        let next = (page.next_key_marker, page.next_version_id_marker);
        if next.0.is_none() || !seen.insert(next.clone()) {
            bail!("invalid storage listing continuation");
        }
        (key, version) = next;
    }
    bail!("version verification exceeded page limit")
}

/// Delete a single selected version, preserving per-version error attribution.
pub async fn delete_version(media: &MediaStorage, target: &MediaTarget) -> Result<()> {
    if target.version_id.is_empty() || target.version_id == "null" {
        bail!("immutable version required");
    }
    let outcome = media
        .delete_object_versions(&[ObjectVersionRef {
            key: target.key.clone(),
            version_id: target.version_id.clone(),
        }])
        .await?;
    if !outcome.failed.is_empty() {
        bail!("storage refused selected version deletion");
    }
    Ok(())
}

/// Classify an unfenced observation. Shared references are concrete keep evidence;
/// absence of known survivors cannot authorize deletion of a live file.
pub fn classify_preview(mut file: MediaFile) -> Result<MediaFile> {
    let sources = file.evidence_json["sources"]
        .as_array()
        .context("missing media source evidence")?;
    let surviving = sources.iter().any(|s| s["surviving"] == true);
    file.evidence_json["preview_checks"] = json!({
        "database_sources": "scanned",
        "storage_versions": "listed",
        "surviving_reference_or_upload": surviving,
        "exact_source_removal_approval": "unresolved",
        "historical_upload_coverage": "unresolved",
        "version_retention_and_holds": "unresolved",
        "upload_record_retention_policy": "unresolved",
        "durable_all_writer_reservation": "unavailable"
    });
    if surviving {
        file.status = "dont_delete".into();
        file.reason =
            Some("surviving reference or uploader claim; retain all components in preview".into());
        file.last_error = None;
    } else {
        file.last_error=Some("unresolved: exact source-removal approval, historical upload coverage, version retention/holds, upload-record retention policy, and durable file reservation".into());
        file.reason = Some(
            "associated with subject; no known surviving reference in this observation".into(),
        );
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_does_not_confuse_missing_checks_with_keep_or_delete() {
        let file = MediaFile {
            storage_namespace: "fixture".into(),
            file_hash: "a".repeat(64),
            status: "unknown".into(),
            evidence_json: json!({"sources":[{"selected_subject":true,"surviving":false}]}),
            delete_targets_json: json!([]),
            reason: None,
            last_error: None,
        };
        let unknown = classify_preview(file.clone()).unwrap();
        assert_eq!(unknown.status, "unknown");
        assert!(unknown.last_error.is_some());
        let mut shared = file;
        shared.evidence_json["sources"]
            .as_array_mut()
            .unwrap()
            .push(json!({"surviving":true}));
        assert_eq!(classify_preview(shared).unwrap().status, "dont_delete");
    }
}

#[cfg(test)]
mod postgres_tests;
