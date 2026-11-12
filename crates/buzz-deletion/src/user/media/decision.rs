//! The same decision procedure is used by previews and execution. A preview
//! records what the current observation permits; execution additionally reserves
//! the files and repeats discovery before using these decisions.
use super::*;
use buzz_db::media_deletion::MediaSource;
use buzz_media::{ObjectVersionEntry, ObjectVersionKind};
use std::collections::BTreeSet;

pub(super) async fn decide(media: &MediaStorage, mut file: MediaFile) -> Result<MediaFile> {
    let sources: Vec<MediaSource> = serde_json::from_value(file.evidence_json["sources"].clone())?;
    let objects: Vec<ObjectVersionEntry> =
        serde_json::from_value(file.evidence_json["observed_objects"].clone())?;
    let departing_communities: BTreeSet<_> = sources
        .iter()
        .filter(|s| s.selected_subject)
        .filter_map(|s| s.community)
        .collect();
    if sources.iter().any(|s| s.surviving) {
        return keep(file, "surviving reference or uploader claim");
    }
    if !sources.iter().any(|s| s.selected_subject && s.upload_claim) {
        bail!("file is referenced by the subject but has no uploader attribution; ownership is unresolved");
    }
    for object in &objects {
        if let Some(path) = object.key.strip_prefix("_meta/") {
            let (community, _) = path
                .split_once('/')
                .context("invalid metadata object path")?;
            let community = Uuid::parse_str(community)?;
            if !departing_communities.contains(&community) {
                return keep(
                    file,
                    "metadata in another community protects the shared file",
                );
            }
        }
    }
    // Save the exact rows whose references this decision assumes will disappear.
    // A caller must preserve this selection through subsequent database removal.
    file.evidence_json["remove_sources"] = serde_json::to_value(
        sources
            .iter()
            .filter(|s| {
                s.selected_subject
                    && !s.upload_claim
                    && !s.surviving
                    && s.table != "workflow_trigger_runs"
            })
            .collect::<Vec<_>>(),
    )?;
    let mut targets = Vec::new();
    let mut checks = Vec::new();
    // Delete byte-bearing versions before delete markers so an interrupted run
    // never exposes an older version simply by removing its hiding marker first.
    let mut objects = objects;
    objects.sort_by_key(|o| {
        (
            o.kind == ObjectVersionKind::DeleteMarker,
            o.key.clone(),
            o.version_id.clone(),
        )
    });
    for object in objects {
        if object.version_id.is_empty() || object.version_id == "null" {
            bail!("file has unversioned objects; exact-version deletion is unavailable");
        }
        if object.key.starts_with("_uploads/") {
            // Each byte-bearing upload record was read and attributed during
            // discovery. Delete only records positively attributed to this scope.
            if object.kind == ObjectVersionKind::Object
                && !sources.iter().any(|s| {
                    s.table == "storage_uploads"
                        && s.selected_subject
                        && s.key["key"] == object.key
                        && s.key["version_id"] == object.version_id
                })
            {
                bail!("upload record has no selected uploader evidence");
            }
        } else if !object.key.starts_with("_meta/") {
            let prefix = format!("{}.", file.file_hash);
            let suffix = object
                .key
                .strip_prefix(&prefix)
                .context("object does not belong to file")?;
            if suffix.contains('/') || suffix.is_empty() {
                bail!("unrecognized file object layout");
            }
        }
        if object.kind == ObjectVersionKind::Object {
            let restriction = media
                .version_retention(&object.key, &object.version_id)
                .await?;
            checks.push(
                json!({"key":object.key,"version_id":object.version_id,"restriction":restriction}),
            );
            if restriction.prevents_deletion(chrono::Utc::now()) {
                file.evidence_json["retention_checks"] = json!(checks);
                return keep(
                    file,
                    "storage legal hold or retention period prevents deletion",
                );
            }
        }
        targets.push(MediaTarget {
            key: object.key,
            version_id: object.version_id,
        });
    }
    file.evidence_json["retention_checks"] = json!(checks);
    if targets.is_empty() {
        return keep(file, "no stored versions remain");
    }
    file.status = "to_be_deleted".into();
    file.delete_targets_json = serde_json::to_value(targets)?;
    file.reason = Some("all observed uses and uploader claims are within the selected removal scope; storage restrictions permit deletion".into());
    file.last_error = None;
    Ok(file)
}

fn keep(mut file: MediaFile, reason: &str) -> Result<MediaFile> {
    file.status = "dont_delete".into();
    file.delete_targets_json = json!([]);
    file.reason = Some(reason.into());
    file.last_error = None;
    Ok(file)
}
