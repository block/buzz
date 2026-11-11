//! Read-only object inventory. This adapter never has to call a delete method.
use anyhow::{bail, Context, Result};
use buzz_db::media_deletion::{MediaFile, MediaScan, MediaSource, MAX_FILES};
use buzz_media::{MediaStorage, ObjectVersionEntry, ObjectVersionKind};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Enumerate originals, derivatives, sidecars and attribution versions. Unknown
/// layouts do not become deletion targets. Failed listings abort discovery.
pub(super) async fn supplement(
    scan: &mut MediaScan,
    media: &MediaStorage,
    namespace: &str,
) -> Result<()> {
    let mut files: BTreeMap<String, MediaFile> = std::mem::take(&mut scan.files)
        .into_iter()
        .map(|f| (f.file_hash.clone(), f))
        .collect();
    let mut objects: BTreeMap<String, Vec<ObjectVersionEntry>> = BTreeMap::new();
    let mut key_marker = None;
    let mut version_marker = None;
    let mut seen = BTreeSet::new();
    let mut count = 0usize;
    let mut object_bytes = 0usize;
    loop {
        let page = media
            .list_prefix_versions_page("", key_marker.clone(), version_marker.clone(), 1000)
            .await?;
        for entry in page.entries {
            object_bytes += serde_json::to_vec(&entry)?.len();
            if object_bytes > 128 * 1024 * 1024 {
                bail!("storage inventory exceeds 128 MiB");
            }
            count += 1;
            if count > 1_000_000 {
                bail!(
                    "storage listing exceeds one million versions; no partial discovery committed"
                );
            }
            let segments: Vec<_> = entry.key.split('/').collect();
            let hash = if segments.len() == 1 {
                segments[0].split('.').next()
            } else if segments.first() == Some(&"_uploads") && segments.len() == 4 {
                Some(segments[2])
            } else if segments.first() == Some(&"_meta") && segments.len() == 3 {
                segments[2].strip_suffix(".json")
            } else {
                None
            };
            let Some(hash) = hash
                .filter(|h| buzz_db::media_deletion::valid_hash(h))
                .map(str::to_owned)
            else {
                continue;
            };
            if segments.first() == Some(&"_uploads") && entry.kind == ObjectVersionKind::Object {
                let bytes = media
                    .get_small_version(&entry.key, &entry.version_id, 65_536)
                    .await?;
                let record: serde_json::Value =
                    serde_json::from_slice(&bytes).context("invalid storage upload record")?;
                let community = Uuid::parse_str(
                    record["community_id"]
                        .as_str()
                        .context("upload record missing community")?,
                )?;
                let actor = record["uploader_id"]
                    .as_str()
                    .context("upload record missing uploader")?;
                let actor = buzz_db::user_deletion::normalize_user_deletion_pubkey(actor)?;
                let event = record["event_id"]
                    .as_str()
                    .context("upload record missing ID")?;
                let valid_ulid = event.len() == 26
                    && event.as_bytes()[0] <= b'7'
                    && event
                        .bytes()
                        .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b));
                if record["version"] != 1
                    || record["sha256"] != hash
                    || segments[1] != community.to_string()
                    || segments[3] != format!("{event}.json")
                    || !valid_ulid
                {
                    bail!("storage upload record/path disagreement");
                }
                let selected = scan.scope.contains(Some(community), Some(&actor));
                let source = MediaSource {
                    table: "storage_uploads".into(),
                    community: Some(community),
                    key: json!({"key":entry.key,"version_id":entry.version_id}),
                    fingerprint: buzz_db::entity_deletion::inventory_digest(&record)?,
                    field: "uploader_id".into(),
                    selected_subject: selected,
                    upload_claim: true,
                    surviving: !selected,
                };
                let file = files.entry(hash.clone()).or_insert_with(|| MediaFile {
                    storage_namespace: namespace.into(),
                    file_hash: hash.clone(),
                    status: "unknown".into(),
                    evidence_json: json!({"sources":[]}),
                    delete_targets_json: json!([]),
                    reason: None,
                    last_error: None,
                });
                file.evidence_json["sources"]
                    .as_array_mut()
                    .context("invalid file evidence")?
                    .push(serde_json::to_value(source)?);
            }
            objects.entry(hash).or_default().push(entry);
            if files.len() > MAX_FILES || objects.len() > MAX_FILES {
                bail!("storage file inventory limit exceeded");
            }
        }
        if !page.is_truncated {
            break;
        }
        let next = (page.next_key_marker, page.next_version_id_marker);
        if next.0.is_none() || !seen.insert(next.clone()) {
            bail!("storage listing continuation missing or repeated");
        }
        (key_marker, version_marker) = next;
    }
    scan.scanned.insert("storage_versions".into(), count);
    for (hash, mut file) in files {
        let selected = file.evidence_json["sources"]
            .as_array()
            .context("invalid source evidence")?
            .iter()
            .any(|s| s["selected_subject"] == true);
        if !selected {
            continue;
        }
        file.evidence_json["observed_objects"] = json!(objects.remove(&hash).unwrap_or_default());
        if serde_json::to_vec(&file)?.len() > buzz_db::media_deletion::MAX_FILE_BYTES {
            bail!("file evidence/versions exceed size bound");
        }
        scan.files.push(file);
    }
    Ok(())
}
