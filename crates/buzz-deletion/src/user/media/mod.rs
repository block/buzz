//! Direct media inventory and the shared three-step handler. The direct command
//! defaults to observation; explicit deletion requires an approved media stage.
use anyhow::{bail, Context, Result};
use buzz_db::media_deletion::{MediaDeletionStore, MediaFile, MediaRun, MediaTarget};
use buzz_media::{MediaStorage, ObjectVersionRef};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;
mod command;
mod decision;
mod execute;
mod pipeline;
mod preview;
mod stage;
mod storage;
mod upload;
pub use command::{configured_storage, run, Options};
pub use preview::preview;
pub use stage::{run_stage, StageOptions};
pub use upload::UploadClaims;

/// Output/execution policy for the media stage.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Print saved step-2 inventory and stop before any deletion.
    #[default]
    PrintOnly,
    /// Print saved step-2 inventory, then execute its protected targets.
    Delete,
}

/// The workflow's safety boundary, distinct from its worker lease. The provider
/// must verify approved scope/removal fingerprints, storage retention/holds,
/// upload evidence and crash-persistent hash reservations.
/// The concrete implementation is used by run_stage. The standalone preview
/// cannot grant execution authority.
#[allow(async_fn_in_trait)]
pub trait ProtectedMedia {
    /// Validate the same durable protection and scope before each external action.
    async fn verify(&self, run: &MediaRun) -> Result<()>;
    /// Validate the completed decisions once before the first storage deletion.
    async fn ready(&self, run: &MediaRun) -> Result<()> {
        self.verify(run).await
    }
    /// Confirm whether this exact version (including delete markers) is present.
    async fn present(&self, target: &MediaTarget) -> Result<bool>;
    /// Delete only this exact version. Errors/timeout are retryable, never absence.
    async fn delete(&self, target: &MediaTarget) -> Result<()>;
    /// Remove only the exact departing uploader-proof rows after all selected
    /// storage work succeeds. Must be idempotent; preserve media_publications.
    async fn cleanup(&self, run: &MediaRun) -> Result<()>;
}

/// Apply the classified inventory saved by the shared pipeline.
/// The caller owns workflow claim, heartbeat and routing.
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
    crate::print_json(&json!({"stage_id":run.id,"mode":mode,"files":files}))?;
    if mode == Mode::PrintOnly {
        return Ok(());
    }
    if files.iter().any(|f| f.status == "unknown") {
        bail!("media decisions unresolved; step 3 cannot start");
    }
    protected.ready(run).await?;
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

#[cfg(test)]
mod postgres_tests;
