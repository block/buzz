//! The same decision procedure is used by previews and execution. A preview
//! records what the current observation permits; execution additionally
//! revalidates candidate upload claims before deletion.
use super::*;
use buzz_db::media_deletion::MediaSource;
use buzz_media::{ObjectVersionEntry, ObjectVersionKind};

pub(super) async fn decide(media: &MediaStorage, mut file: MediaFile) -> Result<MediaFile> {
    let sources: Vec<MediaSource> = serde_json::from_value(file.evidence_json["sources"].clone())?;
    let objects: Vec<ObjectVersionEntry> =
        serde_json::from_value(file.evidence_json["observed_objects"].clone())?;
    if sources
        .iter()
        .any(|s| s.upload_claim && !s.selected_subject)
    {
        return keep(file, "another recorded uploader remains");
    }
    if !sources.iter().any(|s| s.selected_subject && s.upload_claim) {
        bail!("file has no selected uploader attribution; ownership is unresolved");
    }
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
        } else if let Some(path) = object.key.strip_prefix("_meta/") {
            let (community, name) = path.split_once('/').context("invalid metadata path")?;
            Uuid::parse_str(community)?;
            if name != format!("{}.json", file.file_hash) {
                bail!("metadata does not belong to file");
            }
        } else {
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
    file.reason = Some("all observed uploader claims are within the selected removal scope; storage restrictions permit deletion".into());
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
