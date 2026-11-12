//! Offline user media cleanup. Both modes use the same upload-only inventory.
use anyhow::{bail, Context, Result};
use buzz_db::{Db, DbConfig};
use buzz_media::{
    classify_key, KeyClass, MediaStorage, ObjectVersionEntry, ObjectVersionKind, ObjectVersionRef,
    ObjectVersionsPage, UploadRecord, UPLOAD_RECORD_VERSION,
};
use serde_json::Value;
use std::collections::HashSet;
use std::io::{BufWriter, Write};
#[path = "media_inventory.rs"]
mod inventory;
#[path = "media_report.rs"]
mod report;
use inventory::{Inventory, PAGE_SIZE};

const MAX_STORAGE_PAGES: usize = 100_000;

/// Direct media-handler arguments; no workflow request is needed.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Human public key (hex or npub). Includes owned agents across all communities.
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
    handle(&db, &args.pubkey, args.delete).await?;
    Ok(0)
}

/// Collect upload claims, select files, then print or delete the same inventory.
/// Writers must remain stopped for delete mode. No URL references are inspected.
/// Unknown historical ownership cannot be recovered from a bare stored blob.
pub async fn handle(db: &Db, pubkey: &str, delete: bool) -> Result<Value> {
    let pubkey = buzz_db::user_deletion::normalize_user_deletion_pubkey(pubkey)?;
    let media = configured_storage()?;
    let mut output = BufWriter::new(std::io::stdout());
    run_handler(db, &media, &pubkey, delete, &mut output).await
}

// Fakes replace only external DB/storage boundaries; the tests run the same
// candidate collection, two passes, disk inventory and deletion loop.
trait Database {
    async fn targets(&self, pubkey: &str) -> Result<Vec<String>>;
    async fn candidates(&self, pubkeys: &[String], after: &str) -> Result<Vec<String>>;
    async fn shared(&self, pubkeys: &[String], hashes: &[String]) -> Result<Vec<String>>;
    async fn owners(&self, hash: &str, after: &str) -> Result<Vec<String>>;
    async fn remove(&self, pubkeys: &[String], hash: &str) -> Result<u64>;
}
impl Database for Db {
    async fn targets(&self, pubkey: &str) -> Result<Vec<String>> {
        Ok(self.user_media_targets(pubkey).await?)
    }
    async fn candidates(&self, pubkeys: &[String], after: &str) -> Result<Vec<String>> {
        Ok(self
            .user_media_candidates(pubkeys, after, PAGE_SIZE)
            .await?)
    }
    async fn shared(&self, pubkeys: &[String], hashes: &[String]) -> Result<Vec<String>> {
        Ok(self.user_media_shared_hashes(pubkeys, hashes).await?)
    }
    async fn owners(&self, hash: &str, after: &str) -> Result<Vec<String>> {
        Ok(self.user_media_owners(hash, after).await?)
    }
    async fn remove(&self, pubkeys: &[String], hash: &str) -> Result<u64> {
        Ok(self.remove_user_media_uploader(pubkeys, hash).await?)
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
    output: &mut impl Write,
) -> Result<Value> {
    let keys = db.targets(pubkey).await?;
    let keys_set: HashSet<_> = keys.iter().cloned().collect();
    let mut inventory = Inventory::new().await?;
    let mut after = String::new();
    loop {
        let page = db.candidates(&keys, &after).await?;
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
    scan_storage(media, &keys_set, &mut inventory, true).await?;
    after.clear();
    loop {
        let page = inventory.candidates(&after).await?;
        let Some((last, _)) = page.last() else {
            break;
        };
        after = last.clone();
        let hashes: Vec<_> = page.into_iter().map(|(hash, _)| hash).collect();
        inventory
            .mark_shared(&db.shared(&keys, &hashes).await?)
            .await?;
        for hash in hashes {
            let mut owner_cursor = String::new();
            loop {
                let owners = db.owners(&hash, &owner_cursor).await?;
                if owners.is_empty() {
                    break;
                }
                if owners.len() > usize::from(PAGE_SIZE) || owners[0] <= owner_cursor {
                    bail!("invalid database owner page");
                }
                for owner in owners {
                    inventory.add_owner(&hash, &owner).await?;
                    owner_cursor = owner;
                }
            }
        }
    }
    // Pass two can now recognize another uploader seen before Alice in pass one.
    // It also spools exact candidate versions, without retaining the whole bucket.
    scan_storage(media, &keys_set, &mut inventory, false).await?;
    inventory.require_immutable_versions().await?;
    let result = report::execute(db, media, &keys, delete, &mut inventory, output).await?;
    inventory.close().await?;
    Ok(result)
}

async fn scan_storage(
    media: &impl Storage,
    keys_set: &HashSet<String>,
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
                    if candidates_only && keys_set.contains(&owner) {
                        candidates.push(hash.clone());
                    }
                    if !candidates_only {
                        inventory.add_owner(hash, &owner).await?;
                        if !keys_set.contains(&owner) {
                            shared.push(hash.clone());
                        }
                    }
                }
            }
            if !candidates_only {
                if matches!(class, KeyClass::Blob { .. }) {
                    inventory.add_filename(hash, &entry.key).await?;
                }
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
    mod s3;
    use super::*;
    use clap::Parser;
    use serde_json::json;

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
        targets: Vec<String>,
        target_calls: RefCell<usize>,
        candidates: Vec<String>,
        shared: Vec<String>,
        checked: RefCell<Vec<String>>,
        removed: RefCell<Vec<String>>,
    }
    impl Database for FakeDb {
        async fn targets(&self, pubkey: &str) -> Result<Vec<String>> {
            *self.target_calls.borrow_mut() += 1;
            Ok(if self.targets.is_empty() {
                vec![pubkey.to_owned()]
            } else {
                self.targets.clone()
            })
        }
        async fn candidates(&self, keys: &[String], after: &str) -> Result<Vec<String>> {
            if !self.targets.is_empty() {
                assert_eq!(keys, self.targets);
            }
            Ok(self
                .candidates
                .iter()
                .filter(|hash| hash.as_str() > after)
                .take(usize::from(PAGE_SIZE))
                .cloned()
                .collect())
        }
        async fn shared(&self, keys: &[String], hashes: &[String]) -> Result<Vec<String>> {
            if !self.targets.is_empty() {
                assert_eq!(keys, self.targets);
            }
            self.checked.borrow_mut().extend_from_slice(hashes);
            Ok(hashes
                .iter()
                .filter(|hash| self.shared.contains(hash))
                .cloned()
                .collect())
        }
        async fn owners(&self, hash: &str, after: &str) -> Result<Vec<String>> {
            let mut owners = Vec::new();
            if self.candidates.iter().any(|candidate| candidate == hash) {
                owners.push(ALICE.to_owned());
            }
            if self.shared.iter().any(|candidate| candidate == hash) {
                owners.push(BOB.to_owned());
            }
            owners.sort();
            Ok(owners
                .into_iter()
                .filter(|owner| owner.as_str() > after)
                .collect())
        }
        async fn remove(&self, keys: &[String], hash: &str) -> Result<u64> {
            if !self.targets.is_empty() {
                assert_eq!(keys, self.targets);
            }
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
        passes: RefCell<Vec<String>>,
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
            if key.is_none() {
                self.passes.borrow_mut().push(prefix.to_owned());
            }
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
        let preview = run_handler(&db, &media, ALICE, false, &mut Vec::new())
            .await
            .unwrap();
        assert_eq!(preview["selected_files"], 3);
        assert_eq!(preview["kept_files"], 2);
        assert!(db.removed.borrow().is_empty());
        assert!(media.deleted.borrow().is_empty());
        assert!(db.checked.borrow().contains(&c));
        assert!(!db.checked.borrow().contains(&d));
        let result = run_handler(&db, &media, ALICE, true, &mut Vec::new())
            .await
            .unwrap();
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
        assert!(run_handler(&db, &media, ALICE, true, &mut Vec::new())
            .await
            .is_err());
        assert!(db.removed.borrow().is_empty());
        assert!(media
            .entries
            .borrow()
            .iter()
            .any(|v| v.key == claim && v.kind == ObjectVersionKind::Object));
        *media.fail_delete.borrow_mut() = None;
        let result = run_handler(&db, &media, ALICE, true, &mut Vec::new())
            .await
            .unwrap();
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
            assert!(run_handler(&db, &media, ALICE, true, &mut Vec::new())
                .await
                .is_err());
            assert!(media.deleted.borrow().is_empty());
            assert!(db.removed.borrow().is_empty());
        }
    }

    const AGENT: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

    #[tokio::test]
    async fn combined_targets_share_one_inventory_and_preserve_outside_uploaders() {
        let hashes: Vec<_> = (1..=5).map(|n| format!("{n:064x}")).collect();
        let db = FakeDb {
            targets: vec![ALICE.into(), AGENT.into()],
            candidates: vec![hashes[0].clone()],
            shared: vec![hashes[3].clone()], // Outside uploader found only in DB.
            ..Default::default()
        };
        let mut media = FakeStorage::default();
        media.upload(&hashes[0], ALICE, 1);
        media.upload(&hashes[0], AGENT, 2); // Internal sharing must not protect it.
        media.upload(&hashes[1], AGENT, 1); // Agent-only, storage-only candidate.
        media.upload(&hashes[2], BOB, 1); // Outside claim sorts before target claim.
        media.upload(&hashes[2], AGENT, 2);
        media.upload(&hashes[3], AGENT, 1);
        media.upload(&hashes[4], BOB, 1); // Unrelated file is never selected.
        for hash in &hashes {
            media.object(format!("{hash}.png"), ObjectVersionKind::Object);
        }
        let mut output = Vec::new();
        let preview = run_handler(&db, &media, ALICE, false, &mut output)
            .await
            .unwrap();
        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["target_pubkeys"], json!([ALICE, AGENT]));
        assert_eq!(report["summary"], preview);
        let files = report["files"].as_array().unwrap();
        assert_eq!(files.len(), 4);
        let mut internal = vec![ALICE, AGENT];
        internal.sort();
        let mut external = vec![BOB, AGENT];
        external.sort();
        assert_eq!(files[0]["owners"], json!(internal));
        assert_eq!(files[1]["owners"], json!([AGENT]));
        assert_eq!(files[2]["owners"], json!(external));
        assert_eq!(files[3]["owners"], json!(external)); // Includes DB-only outside evidence.
        let mut reported_versions = Vec::new();
        for (index, file) in files.iter().enumerate() {
            assert_eq!(file["filename"], format!("{}.png", hashes[index]));
            assert_eq!(file["original_filename"], Value::Null);
            assert_eq!(file["decision"], if index < 2 { "delete" } else { "keep" });
            assert_eq!(file["deleted_versions"], 0);
            assert_eq!(file["removed_uploader_rows"], 0);
            reported_versions.extend(file["versions"].as_array().unwrap().iter().cloned());
        }
        // Kept files have the same complete structure, including their versions.
        let mut expected_versions: Vec<Value> = media
            .entries
            .borrow()
            .iter()
            .filter(|entry| !entry.key.contains(&hashes[4]))
            .map(|entry| serde_json::to_value(entry).unwrap())
            .collect();
        reported_versions.sort_by_key(Value::to_string);
        expected_versions.sort_by_key(Value::to_string);
        assert_eq!(reported_versions, expected_versions);
        assert_eq!(preview["selected_files"], 4);
        assert_eq!(preview["kept_files"], 2);
        assert_eq!(preview["deleted_versions"], 0);
        assert_eq!(*db.target_calls.borrow(), 1);
        assert_eq!(*media.passes.borrow(), vec!["_uploads/", ""]);
        assert!(db.removed.borrow().is_empty());
        assert!(media.deleted.borrow().is_empty());

        output.clear();
        let result = run_handler(&db, &media, ALICE, true, &mut output)
            .await
            .unwrap();
        let deleted_report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(deleted_report["summary"], result);
        assert_eq!(deleted_report["files"][0]["deleted_versions"], 3);
        assert_eq!(deleted_report["files"][1]["deleted_versions"], 2);
        assert_eq!(deleted_report["files"][2]["deleted_versions"], 0);
        assert_eq!(result["selected_files"], 4);
        assert_eq!(result["kept_files"], 2);
        assert_eq!(result["deleted_versions"], 5);
        assert_eq!(*db.target_calls.borrow(), 2);
        assert_eq!(
            *media.passes.borrow(),
            vec!["_uploads/", "", "_uploads/", ""]
        );
        assert_eq!(*db.removed.borrow(), hashes[..4]);
        assert!(media
            .deleted
            .borrow()
            .iter()
            .all(|entry| entry.key.contains(&hashes[0]) || entry.key.contains(&hashes[1])));
        assert_eq!(media.entries.borrow().len(), 7);
    }
    #[tokio::test]
    async fn grouped_report_streams_large_files_and_represents_missing_storage() {
        let mut output = Vec::new();
        let empty = FakeDb::default();
        let mut media = FakeStorage::default();
        run_handler(&empty, &media, ALICE, false, &mut output)
            .await
            .unwrap();
        let report: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(report["files"], json!([]));
        let hashes = vec!["a".repeat(64), "b".repeat(64)];
        let db = FakeDb {
            candidates: hashes.clone(),
            ..Default::default()
        };
        for _ in 0..501 {
            media.object(format!("{}.png", hashes[0]), ObjectVersionKind::Object);
        }
        output.clear();
        run_handler(&db, &media, ALICE, false, &mut output)
            .await
            .unwrap();
        let report: Value = serde_json::from_slice(&output).unwrap();
        let files = report["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["versions"].as_array().unwrap().len(), 501);
        assert_eq!(files[1]["filename"], Value::Null);
        assert_eq!(files[1]["owners"], json!([ALICE]));
        assert_eq!(files[1]["versions"], json!([]));
        assert_eq!(
            files[0].as_object().unwrap().keys().collect::<Vec<_>>(),
            files[1].as_object().unwrap().keys().collect::<Vec<_>>()
        );
    }
}
