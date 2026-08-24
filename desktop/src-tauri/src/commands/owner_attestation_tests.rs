use super::*;
use std::{
    cell::Cell,
    os::unix::fs::{MetadataExt, PermissionsExt},
};

struct Fixture {
    _dir: tempfile::TempDir,
    request_path: PathBuf,
    target_path: PathBuf,
    owner_keys: Keys,
    agent_keys: Keys,
    conditions: String,
}

impl Fixture {
    fn new() -> Self {
        let test_root = std::env::current_dir().expect("test working directory");
        let dir = tempfile::Builder::new()
            .prefix("owner-attestation-test-")
            .tempdir_in(test_root)
            .expect("temp dir");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("restrict temp dir");
        let request_path = dir.path().join(REQUEST_FILE_NAME);
        let target_path = dir.path().join(TARGET_FILE_NAME);
        let owner_keys = Keys::generate();
        let agent_keys = Keys::generate();
        let conditions = "kind=9&created_at>1&created_at<4294967295".to_string();
        let fixture = Self {
            _dir: dir,
            request_path,
            target_path,
            owner_keys,
            agent_keys,
            conditions,
        };
        fixture.write_request(None);
        fixture
    }

    fn request(&self) -> OwnerAttestationRequest {
        let agent_pubkey = self.agent_keys.public_key().to_hex();
        OwnerAttestationRequest {
            schema: REQUEST_SCHEMA.to_string(),
            agent_pubkey: agent_pubkey.clone(),
            conditions: self.conditions.clone(),
            signing_preimage: format!("nostr:agent-auth:{agent_pubkey}:{}", self.conditions),
            signing_hash_algorithm: "SHA256".to_string(),
            signature_algorithm: "BIP340_Schnorr_secp256k1".to_string(),
            result_tag_shape: [
                "auth".to_string(),
                "OWNER_PUBLIC_KEY_HEX".to_string(),
                self.conditions.clone(),
                "OWNER_SIGNATURE_HEX".to_string(),
            ],
            private_key_in_request: false,
            signed: false,
        }
    }

    fn write_request(&self, request: Option<OwnerAttestationRequest>) {
        let request = request.unwrap_or_else(|| self.request());
        let bytes = serde_json::to_vec_pretty(&request).expect("request JSON");
        std::fs::write(&self.request_path, bytes).expect("write request");
        std::fs::set_permissions(&self.request_path, std::fs::Permissions::from_mode(0o644))
            .expect("set request mode");
    }

    fn preview(&self) -> OwnerAttestationPreview {
        inspect_request(&self.request_path, &self.owner_keys.public_key()).expect("inspect request")
    }

    fn sign(&self, preview: &OwnerAttestationPreview) -> Result<(), String> {
        sign_request(
            &self.request_path,
            &preview.request_sha256,
            &preview.owner_pubkey,
            &self.owner_keys,
        )
    }
}

#[test]
fn nonempty_conditions_sign_and_verify_with_atomic_owner_only_custody() {
    let fixture = Fixture::new();
    let request_before = std::fs::read(&fixture.request_path).expect("request bytes");
    let request_meta_before = std::fs::metadata(&fixture.request_path).expect("request metadata");
    let preview = fixture.preview();

    fixture.sign(&preview).expect("sign request");
    assert_ne!(
        fixture.owner_keys.public_key(),
        fixture.agent_keys.public_key()
    );
    assert_eq!(preview.conditions, fixture.conditions);
    assert_eq!(
        std::fs::read(&fixture.request_path).unwrap(),
        request_before
    );
    let request_meta_after = std::fs::metadata(&fixture.request_path).unwrap();
    assert_eq!(request_meta_after.ino(), request_meta_before.ino());
    assert_eq!(request_meta_after.mtime(), request_meta_before.mtime());
    assert_eq!(
        request_meta_after.mtime_nsec(),
        request_meta_before.mtime_nsec()
    );

    let tag_json = std::fs::read_to_string(&fixture.target_path).expect("protected tag");
    let recovered =
        buzz_sdk_pkg::nip_oa::verify_auth_tag(&tag_json, &fixture.agent_keys.public_key())
            .expect("BIP340 verify");
    assert_eq!(recovered, fixture.owner_keys.public_key());
    let parts: Vec<String> = serde_json::from_str(&tag_json).expect("tag JSON");
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[0], "auth");
    assert_eq!(parts[2].as_bytes(), fixture.conditions.as_bytes());
    assert_eq!(parts[3].len(), 128);

    let target_meta = std::fs::symlink_metadata(&fixture.target_path).unwrap();
    let parent_meta = std::fs::metadata(fixture.target_path.parent().unwrap()).unwrap();
    assert!(target_meta.is_file());
    assert!(!target_meta.file_type().is_symlink());
    assert_eq!(target_meta.permissions().mode() & 0o7777, 0o600);
    assert_eq!(target_meta.uid(), parent_meta.uid());
    assert_eq!(target_meta.gid(), parent_meta.gid());
    assert_eq!(target_meta.nlink(), 1);
    let entries = std::fs::read_dir(fixture.target_path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 2, "no temporary or unrelated side effects");
}

#[test]
fn existing_target_is_rejected_without_modification() {
    let fixture = Fixture::new();
    let preview = fixture.preview();
    std::fs::write(&fixture.target_path, b"preserve-me").unwrap();
    std::fs::set_permissions(&fixture.target_path, std::fs::Permissions::from_mode(0o600)).unwrap();

    let error = fixture
        .sign(&preview)
        .expect_err("existing target must fail");

    assert!(error.contains("already exists"));
    assert_eq!(std::fs::read(&fixture.target_path).unwrap(), b"preserve-me");
}

#[test]
fn symlink_target_is_rejected_without_following_it() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let outside = fixture.target_path.parent().unwrap().join("outside");
    std::fs::write(&outside, b"outside-preserved").unwrap();
    symlink(&outside, &fixture.target_path).unwrap();

    let error = inspect_request(&fixture.request_path, &fixture.owner_keys.public_key())
        .expect_err("symlink target must fail");

    assert!(error.contains("already exists or is a symlink"));
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside-preserved");
    assert!(std::fs::symlink_metadata(&fixture.target_path)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn request_symlink_mode_and_link_count_are_rejected_without_output() {
    use std::os::unix::fs::symlink;

    let symlink_fixture = Fixture::new();
    let real_request = symlink_fixture
        .request_path
        .parent()
        .unwrap()
        .join("request-real.json");
    std::fs::rename(&symlink_fixture.request_path, &real_request).unwrap();
    symlink(&real_request, &symlink_fixture.request_path).unwrap();
    assert!(inspect_request(
        &symlink_fixture.request_path,
        &symlink_fixture.owner_keys.public_key()
    )
    .unwrap_err()
    .contains("symlink"));
    assert!(!symlink_fixture.target_path.exists());

    let mode_fixture = Fixture::new();
    std::fs::set_permissions(
        &mode_fixture.request_path,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(inspect_request(
        &mode_fixture.request_path,
        &mode_fixture.owner_keys.public_key()
    )
    .unwrap_err()
    .contains("0644"));
    assert!(!mode_fixture.target_path.exists());

    let link_fixture = Fixture::new();
    let second_link = link_fixture
        .request_path
        .parent()
        .unwrap()
        .join("request-second-link.json");
    std::fs::hard_link(&link_fixture.request_path, &second_link).unwrap();
    assert!(inspect_request(
        &link_fixture.request_path,
        &link_fixture.owner_keys.public_key()
    )
    .unwrap_err()
    .contains("exactly one hard link"));
    assert!(!link_fixture.target_path.exists());
}

#[test]
fn invalid_conditions_and_owner_fail_without_output() {
    let fixture = Fixture::new();
    let mut request = fixture.request();
    request.conditions.clear();
    request.signing_preimage = format!("nostr:agent-auth:{}:", request.agent_pubkey);
    request.result_tag_shape[2].clear();
    fixture.write_request(Some(request));
    assert!(
        inspect_request(&fixture.request_path, &fixture.owner_keys.public_key())
            .unwrap_err()
            .contains("non-empty")
    );
    assert!(!fixture.target_path.exists());

    let mut request = fixture.request();
    request.agent_pubkey = fixture.owner_keys.public_key().to_hex();
    request.signing_preimage = format!(
        "nostr:agent-auth:{}:{}",
        request.agent_pubkey, request.conditions
    );
    fixture.write_request(Some(request));
    assert!(
        inspect_request(&fixture.request_path, &fixture.owner_keys.public_key())
            .unwrap_err()
            .contains("must differ")
    );
    assert!(!fixture.target_path.exists());
}

#[test]
fn stale_preview_fails_without_output() {
    let fixture = Fixture::new();
    let preview = fixture.preview();
    let mut request = fixture.request();
    request.result_tag_shape[1] = "OWNER_PUBLIC_KEY_HEX_CHANGED".to_string();
    fixture.write_request(Some(request));
    let error = fixture.sign(&preview).expect_err("stale preview must fail");
    assert!(error.contains("changed after inspection") || error.contains("result_tag_shape"));
    assert!(!fixture.target_path.exists());
}

#[test]
fn existing_request_shape_is_accepted_and_derives_sibling_target() {
    let fixture = Fixture::new();
    let existing = serde_json::json!({
        "schema": REQUEST_SCHEMA,
        "agent_pubkey": fixture.agent_keys.public_key().to_hex(),
        "conditions": fixture.conditions.clone(),
        "signing_preimage": format!("nostr:agent-auth:{}:{}", fixture.agent_keys.public_key().to_hex(), fixture.conditions),
        "signing_hash_algorithm": "SHA256",
        "signature_algorithm": "BIP340_Schnorr_secp256k1",
        "result_tag_shape": ["auth", "OWNER_PUBLIC_KEY_HEX", fixture.conditions, "OWNER_SIGNATURE_HEX"],
        "private_key_in_request": false,
        "signed": false
    });
    std::fs::write(
        &fixture.request_path,
        serde_json::to_vec_pretty(&existing).unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(
        &fixture.request_path,
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let preview = inspect_request(&fixture.request_path, &fixture.owner_keys.public_key())
        .expect("existing request must be accepted");
    assert_eq!(
        preview.result_path,
        fixture.target_path.display().to_string()
    );
    assert!(!fixture.target_path.exists());
}

struct CountingSuccessOps {
    link_calls: Cell<usize>,
    unlink_calls: Cell<usize>,
    sync_calls: Cell<usize>,
}

impl AtomicFileOps for CountingSuccessOps {
    fn link_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        self.link_calls.set(self.link_calls.get() + 1);
        RealAtomicFileOps.link_temp(custody, temp_name)
    }

    fn unlink_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        self.unlink_calls.set(self.unlink_calls.get() + 1);
        RealAtomicFileOps.unlink_temp(custody, temp_name)
    }

    fn sync_directory(&self, custody: &PinnedDirectory) -> Result<(), Errno> {
        self.sync_calls.set(self.sync_calls.get() + 1);
        RealAtomicFileOps.sync_directory(custody)
    }
}

struct StableIdentityMutationOps {
    link_calls: Cell<usize>,
    unlink_calls: Cell<usize>,
}

impl AtomicFileOps for StableIdentityMutationOps {
    fn after_temp_sync(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        std::fs::set_permissions(
            custody.path.join(temp_name),
            std::fs::Permissions::from_mode(0o400),
        )
        .expect("mutate stable temp mode");
        Ok(())
    }

    fn link_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        self.link_calls.set(self.link_calls.get() + 1);
        RealAtomicFileOps.link_temp(custody, temp_name)
    }

    fn unlink_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        self.unlink_calls.set(self.unlink_calls.get() + 1);
        RealAtomicFileOps.unlink_temp(custody, temp_name)
    }

    fn sync_directory(&self, custody: &PinnedDirectory) -> Result<(), Errno> {
        RealAtomicFileOps.sync_directory(custody)
    }
}

struct UnlinkFailureOps {
    unlink_calls: Cell<usize>,
    sync_calls: Cell<usize>,
}

impl AtomicFileOps for UnlinkFailureOps {
    fn link_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        RealAtomicFileOps.link_temp(custody, temp_name)
    }

    fn unlink_temp(&self, _custody: &PinnedDirectory, _temp_name: &str) -> Result<(), Errno> {
        self.unlink_calls.set(self.unlink_calls.get() + 1);
        Err(Errno::IO)
    }

    fn sync_directory(&self, _custody: &PinnedDirectory) -> Result<(), Errno> {
        self.sync_calls.set(self.sync_calls.get() + 1);
        Ok(())
    }
}

struct SyncFailureOps {
    unlink_calls: Cell<usize>,
    sync_calls: Cell<usize>,
}

struct TargetAppearsDuringLinkOps {
    link_calls: Cell<usize>,
    unlink_calls: Cell<usize>,
}

impl AtomicFileOps for TargetAppearsDuringLinkOps {
    fn link_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        self.link_calls.set(self.link_calls.get() + 1);
        let mut target = File::from(rustix::fs::openat(
            &custody.directory,
            TARGET_FILE_NAME,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW,
            Mode::RUSR | Mode::WUSR,
        )?);
        target.write_all(b"preserve-race-winner").unwrap();
        target.sync_all().unwrap();
        RealAtomicFileOps.link_temp(custody, temp_name)
    }

    fn unlink_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        self.unlink_calls.set(self.unlink_calls.get() + 1);
        RealAtomicFileOps.unlink_temp(custody, temp_name)
    }

    fn sync_directory(&self, custody: &PinnedDirectory) -> Result<(), Errno> {
        RealAtomicFileOps.sync_directory(custody)
    }
}

impl AtomicFileOps for SyncFailureOps {
    fn link_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        RealAtomicFileOps.link_temp(custody, temp_name)
    }

    fn unlink_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        self.unlink_calls.set(self.unlink_calls.get() + 1);
        RealAtomicFileOps.unlink_temp(custody, temp_name)
    }

    fn sync_directory(&self, _custody: &PinnedDirectory) -> Result<(), Errno> {
        self.sync_calls.set(self.sync_calls.get() + 1);
        Err(Errno::IO)
    }
}

struct RenameDuringLinkOps {
    original: PathBuf,
    moved: PathBuf,
    link_calls: Cell<usize>,
}

impl AtomicFileOps for RenameDuringLinkOps {
    fn link_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        self.link_calls.set(self.link_calls.get() + 1);
        std::fs::rename(&self.original, &self.moved).expect("rename custody during link");
        std::fs::create_dir(&self.original).expect("create replacement custody during link");
        std::fs::set_permissions(&self.original, std::fs::Permissions::from_mode(0o700))
            .expect("restrict replacement custody during link");
        RealAtomicFileOps.link_temp(custody, temp_name)
    }

    fn unlink_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        RealAtomicFileOps.unlink_temp(custody, temp_name)
    }

    fn sync_directory(&self, custody: &PinnedDirectory) -> Result<(), Errno> {
        RealAtomicFileOps.sync_directory(custody)
    }
}

fn restore_replaced_custody(original: &Path, moved: &Path) {
    std::fs::remove_dir(original).expect("remove replacement custody directory");
    std::fs::rename(moved, original).expect("restore original custody directory");
}

#[test]
fn valid_post_write_identity_and_length_reach_linkat_success() {
    let fixture = Fixture::new();
    let custody = open_pinned_directory(&fixture.request_path).expect("pin custody directory");
    let ops = CountingSuccessOps {
        link_calls: Cell::new(0),
        unlink_calls: Cell::new(0),
        sync_calls: Cell::new(0),
    };

    atomic_create_secret_with_ops(&custody, b"test-auth-tag", &ops)
        .expect("valid write must reach linkat");

    assert_eq!(ops.link_calls.get(), 1);
    assert_eq!(ops.unlink_calls.get(), 1);
    assert_eq!(ops.sync_calls.get(), 1);
    assert_eq!(
        std::fs::read(&fixture.target_path).unwrap(),
        b"test-auth-tag"
    );
}

#[test]
fn stable_temp_identity_mutation_fails_before_linkat() {
    let fixture = Fixture::new();
    let custody = open_pinned_directory(&fixture.request_path).expect("pin custody directory");
    let ops = StableIdentityMutationOps {
        link_calls: Cell::new(0),
        unlink_calls: Cell::new(0),
    };

    let error = atomic_create_secret_with_ops(&custody, b"test-auth-tag", &ops)
        .expect_err("stable identity mutation must fail");

    assert!(error.contains("identity changed before commit"));
    assert_eq!(ops.link_calls.get(), 0);
    assert_eq!(ops.unlink_calls.get(), 1, "pre-commit cleanup runs once");
    assert!(!fixture.target_path.exists());
    assert_eq!(
        std::fs::read_dir(fixture.request_path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".BUZZ_AUTH_TAG."))
            .count(),
        0
    );
}

#[test]
fn directory_replacement_before_commit_is_rejected_by_pinned_identity() {
    let fixture = Fixture::new();
    let custody = open_pinned_directory(&fixture.request_path).expect("pin custody directory");
    let original = fixture.request_path.parent().unwrap().to_path_buf();
    let moved = original.with_extension("moved-before-commit");
    std::fs::rename(&original, &moved).unwrap();
    std::fs::create_dir(&original).unwrap();
    std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o700)).unwrap();

    let error = atomic_create_secret(&custody, b"test-auth-tag")
        .expect_err("replaced custody directory must fail");

    assert!(error.contains("renamed or replaced"));
    assert!(!moved.join(TARGET_FILE_NAME).exists());
    assert!(!original.join(TARGET_FILE_NAME).exists());
    restore_replaced_custody(&original, &moved);
}

#[test]
fn directory_rename_during_link_is_detected_after_descriptor_relative_commit() {
    let fixture = Fixture::new();
    let custody = open_pinned_directory(&fixture.request_path).expect("pin custody directory");
    let original = fixture.request_path.parent().unwrap().to_path_buf();
    let moved = original.with_extension("moved-during-link");
    let ops = RenameDuringLinkOps {
        original: original.clone(),
        moved: moved.clone(),
        link_calls: Cell::new(0),
    };

    let error = atomic_create_secret_with_ops(&custody, b"test-auth-tag", &ops)
        .expect_err("rename during commit must be detected");

    assert_eq!(ops.link_calls.get(), 1);
    assert!(error.contains("custody path verification failed"));
    assert!(error.contains("STOP and do not retry"));
    assert_eq!(
        std::fs::read(moved.join(TARGET_FILE_NAME)).unwrap(),
        b"test-auth-tag"
    );
    assert!(!original.join(TARGET_FILE_NAME).exists());
    restore_replaced_custody(&original, &moved);
}

#[test]
fn target_appearing_at_link_commit_wins_without_replacement() {
    let fixture = Fixture::new();
    let custody = open_pinned_directory(&fixture.request_path).expect("pin custody directory");
    let ops = TargetAppearsDuringLinkOps {
        link_calls: Cell::new(0),
        unlink_calls: Cell::new(0),
    };

    let error = atomic_create_secret_with_ops(&custody, b"test-auth-tag", &ops)
        .expect_err("concurrent target must win");

    assert!(error.contains("target appeared before commit"));
    assert_eq!(ops.link_calls.get(), 1);
    assert_eq!(
        ops.unlink_calls.get(),
        1,
        "pre-commit temp cleanup runs once"
    );
    assert_eq!(
        std::fs::read(&fixture.target_path).unwrap(),
        b"preserve-race-winner"
    );
    assert_eq!(
        std::fs::read_dir(fixture.request_path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".BUZZ_AUTH_TAG."))
            .count(),
        0
    );
}

#[test]
fn unlink_fault_is_visible_and_never_retried_by_cleanup() {
    let fixture = Fixture::new();
    let custody = open_pinned_directory(&fixture.request_path).expect("pin custody directory");
    let ops = UnlinkFailureOps {
        unlink_calls: Cell::new(0),
        sync_calls: Cell::new(0),
    };

    let error = atomic_create_secret_with_ops(&custody, b"test-auth-tag", &ops)
        .expect_err("unlink fault must stop");

    assert!(error.contains("temporary-link cleanup failed"));
    assert!(error.contains("STOP and do not retry"));
    assert_eq!(ops.unlink_calls.get(), 1, "cleanup must not retry in Drop");
    assert_eq!(
        ops.sync_calls.get(),
        0,
        "no operation follows ambiguous cleanup"
    );
    assert_eq!(
        std::fs::read(&fixture.target_path).unwrap(),
        b"test-auth-tag"
    );
    assert_eq!(
        std::fs::metadata(&fixture.target_path).unwrap().nlink(),
        2,
        "failed cleanup leaves the two-link state visible"
    );
    let temp_files = std::fs::read_dir(fixture.request_path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".BUZZ_AUTH_TAG.")
        })
        .collect::<Vec<_>>();
    assert_eq!(temp_files.len(), 1, "ambiguous temp link is left untouched");
}

#[test]
fn directory_fsync_fault_is_visible_after_single_cleanup() {
    let fixture = Fixture::new();
    let custody = open_pinned_directory(&fixture.request_path).expect("pin custody directory");
    let ops = SyncFailureOps {
        unlink_calls: Cell::new(0),
        sync_calls: Cell::new(0),
    };

    let error = atomic_create_secret_with_ops(&custody, b"test-auth-tag", &ops)
        .expect_err("directory fsync fault must stop");

    assert!(error.contains("custody-directory sync failed"));
    assert!(error.contains("STOP and do not retry"));
    assert_eq!(ops.unlink_calls.get(), 1);
    assert_eq!(ops.sync_calls.get(), 1);
    assert_eq!(
        std::fs::read(&fixture.target_path).unwrap(),
        b"test-auth-tag"
    );
    assert_eq!(
        std::fs::read_dir(fixture.request_path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".BUZZ_AUTH_TAG."))
            .count(),
        0
    );
}
