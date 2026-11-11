//! Offline user media cleanup. Both modes use the same upload-only inventory.
use anyhow::{bail, Context, Result};
use buzz_db::{Db, DbConfig};
use buzz_media::{
    classify_key, KeyClass, MediaStorage, ObjectVersionEntry, ObjectVersionKind, ObjectVersionRef,
    UploadRecord, UPLOAD_RECORD_VERSION,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Direct media-handler arguments; no workflow request is needed.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Human public key (hex or npub), across all communities.
    #[arg(long, value_parser = buzz_db::user_deletion::normalize_user_deletion_pubkey)]
    pub pubkey: String,
    /// Print selected files without changing the database or storage (default).
    #[arg(long, visible_alias = "print-only", conflicts_with = "delete")]
    pub read_only: bool,
    /// Delete selected files. Run only while relay and other writers are stopped.
    #[arg(long)]
    pub delete: bool,
}

/// Invoke the handler directly against DATABASE_URL and BUZZ_S3_* storage.
pub async fn run(args: Args) -> Result<i32> {
    let db = Db::connect_cold_start(
        DbConfig {
            database_url: crate::required_env("DATABASE_URL")?,
            max_connections: 1,
            min_connections: 0,
            default_transaction_read_only: !args.delete,
            ..DbConfig::default()
        },
        "user_media",
    )
    .await?;
    crate::print_json(&handle(&db, &args.pubkey, args.delete).await?)?;
    Ok(0)
}

/// Collect upload claims, select files, then print or delete the same inventory.
/// Writers must remain stopped for delete mode. No URL references are inspected.
/// Unknown historical ownership cannot be recovered from a bare stored blob.
pub async fn handle(db: &Db, pubkey: &str, delete: bool) -> Result<Value> {
    let pubkey = buzz_db::user_deletion::normalize_user_deletion_pubkey(pubkey)?;
    let media = configured_storage()?;
    let mut uploaders = db.user_media_uploaders().await?;
    let objects = discover(&media, &mut uploaders).await?;
    let mut selected = 0usize;
    let mut kept = 0usize;
    let mut deleted_versions = 0usize;
    for (hash, owners) in uploaders
        .iter()
        .filter(|(_, owners)| owners.contains(&pubkey))
    {
        selected += 1;
        let keep = owners.iter().any(|owner| owner != &pubkey);
        let mut versions = objects.get(hash).cloned().unwrap_or_default();
        order_versions(&mut versions);
        crate::print_json(&json!({"file_hash":hash,"uploaders":owners,
            "decision":if keep {"keep"} else {"delete"},"versions":versions,
            "mode":if delete {"delete"} else {"read-only"}}))?;
        if keep {
            kept += 1;
            if delete {
                let removed = db.remove_user_media_uploader(&pubkey, hash).await?;
                crate::print_json(
                    &json!({"file_hash":hash,"deleted_versions":0,"removed_uploader_rows":removed}),
                )?;
            }
            continue;
        }
        if versions
            .iter()
            .any(|v| v.version_id.is_empty() || v.version_id == "null")
        {
            bail!("file {hash} has unversioned objects; immutable version IDs are required");
        }
        if !delete {
            continue;
        }
        for version in &versions {
            let outcome = media
                .delete_object_versions(&[ObjectVersionRef {
                    key: version.key.clone(),
                    version_id: version.version_id.clone(),
                }])
                .await?;
            if !outcome.failed.is_empty() || outcome.deleted + outcome.already_missing != 1 {
                bail!("storage refused deletion for {}", version.key);
            }
            deleted_versions += 1;
        }
        let removed = db.remove_user_media_uploader(&pubkey, hash).await?;
        crate::print_json(
            &json!({"file_hash":hash,"deleted_versions":versions.len(),"removed_uploader_rows":removed}),
        )?;
    }
    Ok(json!({"mode":if delete {"delete"} else {"read-only"},
        "selected_files":selected,"kept_files":kept,"deleted_versions":deleted_versions,
        "account_erasure_complete":false}))
}

// Retain upload attribution through every earlier storage action. Upload delete
// markers precede upload bytes so losing the final claim cannot orphan markers.
fn order_versions(versions: &mut [ObjectVersionEntry]) {
    versions.sort_by_key(|v| match (v.key.starts_with("_uploads/"), v.kind) {
        (false, ObjectVersionKind::Object) => 0,
        (false, ObjectVersionKind::DeleteMarker) => 1,
        (true, ObjectVersionKind::DeleteMarker) => 2,
        (true, ObjectVersionKind::Object) => 3,
    });
}

async fn discover(
    media: &MediaStorage,
    uploaders: &mut BTreeMap<String, BTreeSet<String>>,
) -> Result<BTreeMap<String, Vec<ObjectVersionEntry>>> {
    let mut objects: BTreeMap<String, Vec<ObjectVersionEntry>> = BTreeMap::new();
    let (mut key_marker, mut version_marker) = (None, None);
    let mut seen = BTreeSet::new();
    let mut count = 0usize;
    let mut bytes = 0usize;
    loop {
        let page = media
            .list_prefix_versions_page("", key_marker.clone(), version_marker.clone(), 1000)
            .await?;
        for entry in page.entries {
            count += 1;
            bytes += serde_json::to_vec(&entry)?.len();
            if count > 1_000_000 || bytes > 128 * 1024 * 1024 {
                bail!("media inventory exceeds one million versions or 128 MiB");
            }
            let hash = match classify_key(&entry.key) {
                KeyClass::Blob { sha256, .. }
                | KeyClass::Thumb { sha256 }
                | KeyClass::Sidecar { sha256, .. } => sha256,
                KeyClass::Auxiliary {
                    community,
                    sha256,
                    event_id,
                } => {
                    if entry.kind == ObjectVersionKind::Object {
                        let body = media
                            .get_small_version(&entry.key, &entry.version_id, 65_536)
                            .await?;
                        let record: UploadRecord =
                            serde_json::from_slice(&body).context("invalid upload record")?;
                        if record.version != UPLOAD_RECORD_VERSION
                            || record.sha256 != sha256
                            || record.community_id != community.to_string()
                            || record.event_id != event_id
                        {
                            bail!("upload record does not match its storage path");
                        }
                        let owner = buzz_db::user_deletion::normalize_user_deletion_pubkey(
                            &record.uploader_id,
                        )?;
                        uploaders.entry(sha256.clone()).or_default().insert(owner);
                    }
                    sha256
                }
                KeyClass::Unknown => continue,
            };
            objects.entry(hash).or_default().push(entry);
        }
        if !page.is_truncated {
            break;
        }
        let next = (page.next_key_marker, page.next_version_id_marker);
        if next.0.is_none() || !seen.insert(next.clone()) {
            bail!("invalid storage listing continuation");
        }
        (key_marker, version_marker) = next;
    }
    Ok(objects)
}

fn configured_storage() -> Result<MediaStorage> {
    let (s3_access_key, s3_secret_key) = crate::s3_key_pair_from_env();
    Ok(MediaStorage::new(&buzz_media::MediaConfig {
        s3_endpoint: crate::required_env("BUZZ_S3_ENDPOINT")?,
        s3_bucket: crate::required_env("BUZZ_S3_BUCKET")?,
        s3_access_key,
        s3_secret_key,
        s3_region: crate::s3_region_from_env(),
        s3_addressing_style: std::env::var("BUZZ_S3_ADDRESSING_STYLE")
            .unwrap_or_else(|_| "path".into())
            .parse()
            .map_err(anyhow::Error::msg)?,
        max_image_bytes: 1,
        max_gif_bytes: 1,
        max_video_bytes: 1,
        max_file_bytes: 1,
        public_base_url: "https://example.org/media".into(),
        upload_records_enabled: false,
        upload_ip_header: None,
        upload_port_header: None,
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        args: Args,
    }

    #[test]
    fn direct_command_defaults_to_read_only_and_requires_explicit_delete() {
        let key = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";
        for flag in [None, Some("--read-only"), Some("--print-only")] {
            let args =
                Cli::try_parse_from(["media", "--pubkey", key].into_iter().chain(flag)).unwrap();
            assert!(!args.args.delete);
        }
        assert!(
            Cli::try_parse_from(["media", "--pubkey", key, "--delete"])
                .unwrap()
                .args
                .delete
        );
        assert!(
            Cli::try_parse_from(["media", "--pubkey", key, "--delete", "--read-only"]).is_err()
        );
        assert!(Cli::try_parse_from(["media", "--pubkey", "demo"]).is_err());
    }

    #[test]
    fn retry_keeps_attribution_until_all_other_versions_are_removed() {
        let entry = |key: &str, kind| ObjectVersionEntry {
            key: key.into(),
            kind,
            version_id: "v1".into(),
            size: 1,
        };
        let mut versions = vec![
            entry("_uploads/claim.json", ObjectVersionKind::Object),
            entry("_uploads/claim.json", ObjectVersionKind::DeleteMarker),
            entry("hash.png", ObjectVersionKind::DeleteMarker),
            entry("hash.png", ObjectVersionKind::Object),
            entry("_meta/file.json", ObjectVersionKind::Object),
        ];
        order_versions(&mut versions);
        assert_eq!(versions[0].key, "hash.png");
        assert_eq!(versions[0].kind, ObjectVersionKind::Object);
        assert_eq!(versions[1].key, "_meta/file.json");
        assert_eq!(versions[2].kind, ObjectVersionKind::DeleteMarker);
        assert_eq!(versions[3].key, "_uploads/claim.json");
        assert_eq!(versions[3].kind, ObjectVersionKind::DeleteMarker);
        assert_eq!(versions[4].kind, ObjectVersionKind::Object);
    }
}
