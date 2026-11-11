//! Offline user media cleanup. Both modes use the same upload-only inventory.
use anyhow::{bail, Context, Result};
use buzz_db::{Db, DbConfig};
use buzz_media::{
    classify_key, KeyClass, MediaStorage, ObjectVersionEntry, ObjectVersionKind, ObjectVersionRef,
    ObjectVersionsPage, UploadRecord, UPLOAD_RECORD_VERSION,
};
use serde_json::{json, Value};
#[path = "media_inventory.rs"]
mod inventory;
use inventory::{Inventory, PAGE_SIZE};

const MAX_STORAGE_PAGES: usize = 100_000;

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
    run_handler(db, &media, &pubkey, delete).await
}

// Fakes replace only external DB/storage boundaries; the tests run the same
// candidate collection, two passes, disk inventory and deletion loop.
trait Database {
    async fn candidates(&self, pubkey: &str, after: &str) -> Result<Vec<String>>;
    async fn shared(&self, pubkey: &str, hashes: &[String]) -> Result<Vec<String>>;
    async fn remove(&self, pubkey: &str, hash: &str) -> Result<u64>;
}
impl Database for Db {
    async fn candidates(&self, pubkey: &str, after: &str) -> Result<Vec<String>> {
        Ok(self.user_media_candidates(pubkey, after, PAGE_SIZE).await?)
    }
    async fn shared(&self, pubkey: &str, hashes: &[String]) -> Result<Vec<String>> {
        Ok(self.user_media_shared_hashes(pubkey, hashes).await?)
    }
    async fn remove(&self, pubkey: &str, hash: &str) -> Result<u64> {
        Ok(self.remove_user_media_uploader(pubkey, hash).await?)
    }
}
trait Storage {
    async fn list(
        &self,
        prefix: &str,
        key: Option<String>,
        version: Option<String>,
    ) -> Result<ObjectVersionsPage>;
    async fn read(&self, entry: &ObjectVersionEntry) -> Result<Vec<u8>>;
    async fn delete(&self, entry: &ObjectVersionEntry) -> Result<()>;
}
impl Storage for MediaStorage {
    async fn list(
        &self,
        prefix: &str,
        key: Option<String>,
        version: Option<String>,
    ) -> Result<ObjectVersionsPage> {
        Ok(self
            .list_prefix_versions_page(prefix, key, version, 1000)
            .await?)
    }
    async fn read(&self, entry: &ObjectVersionEntry) -> Result<Vec<u8>> {
        Ok(self
            .get_small_version(&entry.key, &entry.version_id, 65_536)
            .await?)
    }
    async fn delete(&self, entry: &ObjectVersionEntry) -> Result<()> {
        let outcome = self
            .delete_object_versions(&[ObjectVersionRef {
                key: entry.key.clone(),
                version_id: entry.version_id.clone(),
            }])
            .await?;
        if !outcome.failed.is_empty() || outcome.deleted + outcome.already_missing != 1 {
            bail!("storage refused deletion for {}", entry.key);
        }
        Ok(())
    }
}

async fn run_handler(
    db: &impl Database,
    media: &impl Storage,
    pubkey: &str,
    delete: bool,
) -> Result<Value> {
    let mut inventory = Inventory::new().await?;
    let mut after = String::new();
    loop {
        let page = db.candidates(pubkey, &after).await?;
        let Some(last) = page.last() else {
            break;
        };
        if page.len() > usize::from(PAGE_SIZE) || last <= &after {
            bail!("invalid database candidate page");
        }
        after = last.clone();
        inventory.add_candidates(&page).await?;
    }
    // Pass one finishes candidate discovery before any sharing decisions.
    scan_storage(media, pubkey, &mut inventory, true).await?;
    after.clear();
    loop {
        let page = inventory.candidates(&after).await?;
        let Some((last, _)) = page.last() else {
            break;
        };
        after = last.clone();
        let hashes: Vec<_> = page.into_iter().map(|(hash, _)| hash).collect();
        inventory
            .mark_shared(&db.shared(pubkey, &hashes).await?)
            .await?;
    }
    // Pass two can now recognize another uploader seen before Alice in pass one.
    // It also spools exact candidate versions, without retaining the whole bucket.
    scan_storage(media, pubkey, &mut inventory, false).await?;
    inventory.require_immutable_versions().await?;
    let result = execute(db, media, pubkey, delete, &mut inventory).await?;
    inventory.close().await?;
    Ok(result)
}

async fn scan_storage(
    media: &impl Storage,
    pubkey: &str,
    inventory: &mut Inventory,
    candidates_only: bool,
) -> Result<()> {
    let prefix = if candidates_only { "_uploads/" } else { "" };
    let (mut key_marker, mut version_marker) = (None, None);
    for _ in 0..MAX_STORAGE_PAGES {
        let page = media
            .list(prefix, key_marker.clone(), version_marker.clone())
            .await?;
        if page.entries.len() > 1000 {
            bail!("storage listing exceeds requested page size");
        }
        let mut candidates = Vec::new();
        let mut shared = Vec::new();
        let mut versions = Vec::new();
        for entry in page.entries {
            let class = classify_key(&entry.key);
            let hash = match &class {
                KeyClass::Blob { sha256, .. }
                | KeyClass::Thumb { sha256 }
                | KeyClass::Sidecar { sha256, .. }
                | KeyClass::Auxiliary { sha256, .. } => sha256,
                KeyClass::Unknown => continue,
            };
            if !candidates_only && !inventory.contains(hash).await? {
                continue;
            }
            if let KeyClass::Auxiliary {
                community,
                sha256,
                event_id,
            } = &class
            {
                if entry.kind == ObjectVersionKind::Object {
                    let body = media.read(&entry).await?;
                    if body.len() > 65_536 {
                        bail!("upload record exceeds 64 KiB");
                    }
                    let record: UploadRecord =
                        serde_json::from_slice(&body).context("invalid upload record")?;
                    if record.version != UPLOAD_RECORD_VERSION
                        || record.sha256 != *sha256
                        || record.community_id != community.to_string()
                        || record.event_id != *event_id
                    {
                        bail!("upload record does not match its storage path");
                    }
                    let owner = buzz_db::user_deletion::normalize_user_deletion_pubkey(
                        &record.uploader_id,
                    )?;
                    if candidates_only && owner == pubkey {
                        candidates.push(hash.clone());
                    }
                    if !candidates_only && owner != pubkey {
                        shared.push(hash.clone());
                    }
                }
            }
            if !candidates_only {
                versions.push((hash.clone(), entry));
            }
        }
        inventory.add_candidates(&candidates).await?;
        inventory.mark_shared(&shared).await?;
        inventory.add_versions(&versions).await?;
        if !page.is_truncated {
            return Ok(());
        }
        let next = (page.next_key_marker, page.next_version_id_marker);
        if next.0.is_none()
            || next == (key_marker.clone(), version_marker.clone())
            || next.0.as_ref() < key_marker.as_ref()
        {
            bail!("invalid storage listing continuation");
        }
        if !inventory
            .remember_marker(
                prefix,
                next.0.as_deref().unwrap_or_default(),
                next.1.as_deref().unwrap_or_default(),
            )
            .await?
        {
            bail!("repeated storage listing continuation");
        }
        (key_marker, version_marker) = next;
    }
    bail!("media scan exceeds 100000 storage pages")
}

async fn execute(
    db: &impl Database,
    media: &impl Storage,
    pubkey: &str,
    delete: bool,
    inventory: &mut Inventory,
) -> Result<Value> {
    let (mut selected, mut kept, mut deleted_versions) = (0usize, 0usize, 0usize);
    let mut after = String::new();
    loop {
        let page = inventory.candidates(&after).await?;
        if page.is_empty() {
            break;
        }
        for (hash, shared) in page {
            selected += 1;
            crate::print_json(&json!({"file_hash": hash, "has_other_uploader": shared,
                "decision": if shared { "keep" } else { "delete" },
                "mode": if delete { "delete" } else { "read-only" }}))?;
            if shared {
                kept += 1;
            }
            let mut handled = 0usize;
            if !shared {
                let mut cursor = (-1, 0);
                loop {
                    let versions = inventory.versions(&hash, cursor).await?;
                    if versions.is_empty() {
                        break;
                    }
                    for (priority, id, version) in versions {
                        crate::print_json(&json!({"file_hash": hash, "version": version,
                            "mode": if delete { "delete" } else { "read-only" }}))?;
                        if delete {
                            media.delete(&version).await?;
                            handled += 1;
                        }
                        cursor = (priority, id);
                    }
                }
            }
            if delete {
                let removed = db.remove(pubkey, &hash).await?;
                crate::print_json(&json!({"file_hash": hash, "deleted_versions": handled,
                    "removed_uploader_rows": removed}))?;
            }
            deleted_versions += handled;
            after = hash;
        }
    }
    Ok(json!({"mode": if delete { "delete" } else { "read-only" },
        "selected_files": selected, "kept_files": kept, "deleted_versions": deleted_versions,
        "account_erasure_complete": false}))
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

    use std::cell::RefCell;
    use std::collections::BTreeMap;

    const ALICE: &str = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";
    const BOB: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";

    #[derive(Default)]
    struct FakeDb {
        candidates: Vec<String>,
        shared: Vec<String>,
        checked: RefCell<Vec<String>>,
        removed: RefCell<Vec<String>>,
    }
    impl Database for FakeDb {
        async fn candidates(&self, _: &str, after: &str) -> Result<Vec<String>> {
            Ok(self
                .candidates
                .iter()
                .filter(|hash| hash.as_str() > after)
                .take(usize::from(PAGE_SIZE))
                .cloned()
                .collect())
        }
        async fn shared(&self, _: &str, hashes: &[String]) -> Result<Vec<String>> {
            self.checked.borrow_mut().extend_from_slice(hashes);
            Ok(hashes
                .iter()
                .filter(|hash| self.shared.contains(hash))
                .cloned()
                .collect())
        }
        async fn remove(&self, _: &str, hash: &str) -> Result<u64> {
            self.removed.borrow_mut().push(hash.into());
            Ok(1)
        }
    }
    #[derive(Default)]
    struct FakeStorage {
        entries: RefCell<Vec<ObjectVersionEntry>>,
        bodies: BTreeMap<String, Vec<u8>>,
        deleted: RefCell<Vec<ObjectVersionEntry>>,
        fail_delete: RefCell<Option<String>>,
        fail_second_pass: bool,
    }
    impl FakeStorage {
        fn object(&mut self, key: String, kind: ObjectVersionKind) {
            let version_id = format!("version-{}", self.entries.borrow().len());
            self.entries.borrow_mut().push(ObjectVersionEntry {
                key,
                kind,
                version_id,
                size: 1,
            });
        }
        fn upload(&mut self, hash: &str, owner: &str, community: u128) {
            let community = uuid::Uuid::from_u128(community);
            let event = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
            let key = format!("_uploads/{community}/{hash}/{event}.json");
            self.bodies.insert(
                key.clone(),
                serde_json::to_vec(&json!({
                    "version": UPLOAD_RECORD_VERSION, "event_id": event, "sha256": hash,
                    "ext": "png", "mime_type": "image/png", "size": 1, "uploaded_at": 1,
                    "community_id": community.to_string(), "community_host": "media.example",
                    "uploader_id": owner, "uploader_npub": "unused"
                }))
                .unwrap(),
            );
            self.object(key, ObjectVersionKind::Object);
        }
    }
    impl Storage for FakeStorage {
        async fn list(
            &self,
            prefix: &str,
            key: Option<String>,
            version: Option<String>,
        ) -> Result<ObjectVersionsPage> {
            if prefix.is_empty() && self.fail_second_pass {
                bail!("injected listing failure");
            }
            let mut entries: Vec<_> = self
                .entries
                .borrow()
                .iter()
                .filter(|entry| entry.key.starts_with(prefix))
                .cloned()
                .collect();
            entries.sort_by(|a, b| (&a.key, &a.version_id).cmp(&(&b.key, &b.version_id)));
            let start = match (key, version) {
                (Some(key), Some(version)) => {
                    entries
                        .iter()
                        .position(|e| e.key == key && e.version_id == version)
                        .unwrap()
                        + 1
                }
                _ => 0,
            };
            let page: Vec<_> = entries.iter().skip(start).take(2).cloned().collect();
            let truncated = start + page.len() < entries.len();
            Ok(ObjectVersionsPage {
                next_key_marker: if truncated {
                    page.last().map(|e| e.key.clone())
                } else {
                    None
                },
                next_version_id_marker: if truncated {
                    page.last().map(|e| e.version_id.clone())
                } else {
                    None
                },
                is_truncated: truncated,
                entries: page,
            })
        }
        async fn read(&self, entry: &ObjectVersionEntry) -> Result<Vec<u8>> {
            Ok(self
                .bodies
                .get(&entry.key)
                .context("missing fake upload record")?
                .clone())
        }
        async fn delete(&self, entry: &ObjectVersionEntry) -> Result<()> {
            if self.fail_delete.borrow().as_deref() == Some(entry.key.as_str()) {
                bail!("injected deletion failure");
            }
            self.deleted.borrow_mut().push(entry.clone());
            self.entries
                .borrow_mut()
                .retain(|e| e.key != entry.key || e.version_id != entry.version_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn two_passes_join_storage_only_candidates_with_both_sharing_sources() {
        let a = "a".repeat(64); // Exclusive; DB and storage both identify Alice.
        let b = "b".repeat(64); // Storage-only Alice; Bob's storage record comes first.
        let c = "c".repeat(64); // Storage-only Alice; Bob appears only in the DB.
        let d = "d".repeat(64); // Unrelated Bob file.
        let db = FakeDb {
            candidates: vec![a.clone()],
            shared: vec![c.clone()],
            ..Default::default()
        };
        let mut media = FakeStorage::default();
        media.upload(&b, BOB, 1);
        media.upload(&d, BOB, 1);
        for hash in [&a, &b, &c] {
            media.upload(hash, ALICE, 2);
        }
        for hash in [&a, &b, &c, &d] {
            media.object(format!("{hash}.png"), ObjectVersionKind::Object);
        }
        media.object(format!("{a}.png"), ObjectVersionKind::Object); // Historical version too.
        media.object(format!("{a}.png"), ObjectVersionKind::DeleteMarker);
        media.object(
            format!("_meta/{}/{a}.json", uuid::Uuid::from_u128(2)),
            ObjectVersionKind::Object,
        );
        let preview = run_handler(&db, &media, ALICE, false).await.unwrap();
        assert_eq!(preview["selected_files"], 3);
        assert_eq!(preview["kept_files"], 2);
        assert!(db.removed.borrow().is_empty());
        assert!(media.deleted.borrow().is_empty());
        assert!(db.checked.borrow().contains(&c));
        assert!(!db.checked.borrow().contains(&d));
        let result = run_handler(&db, &media, ALICE, true).await.unwrap();
        assert_eq!(result["selected_files"], preview["selected_files"]);
        assert_eq!(result["kept_files"], preview["kept_files"]);
        assert_eq!(result["deleted_versions"], 5);
        assert!(media.deleted.borrow().iter().all(|v| v.key.contains(&a)));
        assert!(media
            .deleted
            .borrow()
            .last()
            .unwrap()
            .key
            .starts_with("_uploads/"));
        assert_eq!(*db.removed.borrow(), vec![a, b, c]);
    }

    #[tokio::test]
    async fn failed_deletion_retains_storage_only_attribution_for_a_fresh_scan() {
        let hash = "a".repeat(64);
        let db = FakeDb::default();
        let mut media = FakeStorage::default();
        media.upload(&hash, ALICE, 1);
        let claim = media.entries.borrow()[0].key.clone();
        media.object(claim.clone(), ObjectVersionKind::DeleteMarker);
        media.object(format!("{hash}.png"), ObjectVersionKind::Object);
        let thumb = format!("{hash}.thumb.webp");
        media.object(thumb.clone(), ObjectVersionKind::Object);
        *media.fail_delete.borrow_mut() = Some(thumb);
        assert!(run_handler(&db, &media, ALICE, true).await.is_err());
        assert!(db.removed.borrow().is_empty());
        assert!(media
            .entries
            .borrow()
            .iter()
            .any(|v| v.key == claim && v.kind == ObjectVersionKind::Object));
        *media.fail_delete.borrow_mut() = None;
        let result = run_handler(&db, &media, ALICE, true).await.unwrap();
        assert_eq!(result["selected_files"], 1);
        assert!(media.entries.borrow().is_empty());
        assert_eq!(*db.removed.borrow(), vec![hash]);
        let deletes = media.deleted.borrow();
        assert_eq!(
            deletes[deletes.len() - 2].kind,
            ObjectVersionKind::DeleteMarker
        );
        assert_eq!(deletes.last().unwrap().key, claim);
    }

    #[tokio::test]
    async fn incomplete_evidence_and_unversioned_targets_never_start_deletion() {
        let hash = "a".repeat(64);
        for failure in ["listing", "record", "version"] {
            let db = FakeDb {
                candidates: vec![hash.clone()],
                ..Default::default()
            };
            let mut media = FakeStorage::default();
            media.upload(&hash, ALICE, 1);
            media.object(format!("{hash}.png"), ObjectVersionKind::Object);
            match failure {
                "listing" => media.fail_second_pass = true,
                "record" => {
                    for body in media.bodies.values_mut() {
                        *body = b"{}".to_vec();
                    }
                }
                _ => media.entries.borrow_mut().last_mut().unwrap().version_id = "null".into(),
            }
            assert!(run_handler(&db, &media, ALICE, true).await.is_err());
            assert!(media.deleted.borrow().is_empty());
            assert!(db.removed.borrow().is_empty());
        }
    }
}
