use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

struct Fixture {
    _dir: tempfile::TempDir,
    request_path: PathBuf,
    target_path: PathBuf,
    owner_keys: Keys,
    agent_keys: Keys,
    conditions: String,
    now: u64,
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
        let now = 2_000_000_000;
        let conditions = format!("created_at>{}&created_at<{}", now - 10, now + 3_600);
        let fixture = Self {
            _dir: dir,
            request_path,
            target_path,
            owner_keys,
            agent_keys,
            conditions,
            now,
        };
        fixture.write_request(None);
        fixture
    }

    fn request(&self, result_path: &Path) -> OwnerAttestationRequest {
        let agent_pubkey = self.agent_keys.public_key().to_hex();
        let mut compressed = vec![0x03];
        compressed.extend_from_slice(&hex::decode(&agent_pubkey).expect("agent hex"));
        OwnerAttestationRequest {
            schema: REQUEST_SCHEMA.to_string(),
            agent_pubkey: agent_pubkey.clone(),
            agent_public_fingerprint_sha256: Sha256Hash::hash(&compressed).to_string(),
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
            result_path: result_path.display().to_string(),
            private_key_in_request: false,
            signed: false,
        }
    }

    fn write_request(&self, request: Option<OwnerAttestationRequest>) {
        let request = request.unwrap_or_else(|| self.request(&self.target_path));
        let bytes = serde_json::to_vec_pretty(&request).expect("request JSON");
        std::fs::write(&self.request_path, bytes).expect("write request");
        std::fs::set_permissions(
            &self.request_path,
            std::fs::Permissions::from_mode(0o644),
        )
        .expect("set request mode");
    }

    fn preview(&self) -> OwnerAttestationPreview {
        inspect_request(&self.request_path, &self.owner_keys.public_key(), self.now)
            .expect("inspect request")
    }

    fn sign(
        &self,
        preview: &OwnerAttestationPreview,
    ) -> Result<OwnerAttestationWriteReceipt, String> {
        sign_request(
            &self.request_path,
            &preview.request_sha256,
            &preview.owner_pubkey,
            &self.owner_keys,
            self.now,
        )
    }
}

#[test]
fn nonempty_conditions_sign_and_verify_with_atomic_owner_only_custody() {
    let fixture = Fixture::new();
    let request_before = std::fs::read(&fixture.request_path).expect("request bytes");
    let request_meta_before = std::fs::metadata(&fixture.request_path).expect("request metadata");
    let preview = fixture.preview();

    let receipt = fixture.sign(&preview).expect("sign request");

    assert!(receipt.written);
    assert_eq!(receipt.owner_pubkey, fixture.owner_keys.public_key().to_hex());
    assert_ne!(fixture.owner_keys.public_key(), fixture.agent_keys.public_key());
    assert_eq!(preview.conditions, fixture.conditions);
    assert_eq!(preview.validity_seconds, 3_610);
    assert_eq!(std::fs::read(&fixture.request_path).unwrap(), request_before);
    let request_meta_after = std::fs::metadata(&fixture.request_path).unwrap();
    assert_eq!(request_meta_after.ino(), request_meta_before.ino());
    assert_eq!(request_meta_after.mtime(), request_meta_before.mtime());
    assert_eq!(
        request_meta_after.mtime_nsec(),
        request_meta_before.mtime_nsec()
    );

    let tag_json = std::fs::read_to_string(&fixture.target_path).expect("protected tag");
    let recovered = buzz_sdk_pkg::nip_oa::verify_auth_tag(
        &tag_json,
        &fixture.agent_keys.public_key(),
    )
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
    std::fs::set_permissions(
        &fixture.target_path,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();

    let error = fixture.sign(&preview).expect_err("existing target must fail");

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

    let error = inspect_request(
        &fixture.request_path,
        &fixture.owner_keys.public_key(),
        fixture.now,
    )
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
        &symlink_fixture.owner_keys.public_key(),
        symlink_fixture.now
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
        &mode_fixture.owner_keys.public_key(),
        mode_fixture.now
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
        &link_fixture.owner_keys.public_key(),
        link_fixture.now
    )
    .unwrap_err()
    .contains("exactly one hard link"));
    assert!(!link_fixture.target_path.exists());
}

#[test]
fn invalid_conditions_owner_and_path_fail_without_output() {
    let fixture = Fixture::new();
    let mut request = fixture.request(&fixture.target_path);
    request.conditions.clear();
    request.signing_preimage = format!("nostr:agent-auth:{}:", request.agent_pubkey);
    request.result_tag_shape[2].clear();
    fixture.write_request(Some(request));
    assert!(inspect_request(
        &fixture.request_path,
        &fixture.owner_keys.public_key(),
        fixture.now
    )
    .unwrap_err()
    .contains("non-empty"));
    assert!(!fixture.target_path.exists());

    let mut request = fixture.request(&fixture.target_path);
    request.agent_pubkey = fixture.owner_keys.public_key().to_hex();
    let mut compressed = vec![0x02];
    compressed.extend_from_slice(&hex::decode(&request.agent_pubkey).unwrap());
    request.agent_public_fingerprint_sha256 = Sha256Hash::hash(&compressed).to_string();
    request.signing_preimage = format!(
        "nostr:agent-auth:{}:{}",
        request.agent_pubkey, request.conditions
    );
    fixture.write_request(Some(request));
    assert!(inspect_request(
        &fixture.request_path,
        &fixture.owner_keys.public_key(),
        fixture.now
    )
    .unwrap_err()
    .contains("must differ"));
    assert!(!fixture.target_path.exists());

    let outside = fixture
        .target_path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(TARGET_FILE_NAME);
    fixture.write_request(Some(fixture.request(&outside)));
    assert!(inspect_request(
        &fixture.request_path,
        &fixture.owner_keys.public_key(),
        fixture.now
    )
    .unwrap_err()
    .contains("exact BUZZ_AUTH_TAG sibling"));
    assert!(!fixture.target_path.exists());
    assert!(!outside.exists());
}

#[test]
fn stale_preview_and_noncurrent_validity_fail_without_output() {
    let fixture = Fixture::new();
    let preview = fixture.preview();
    let mut request = fixture.request(&fixture.target_path);
    request.result_tag_shape[1] = "OWNER_PUBLIC_KEY_HEX_CHANGED".to_string();
    fixture.write_request(Some(request));
    let error = fixture.sign(&preview).expect_err("stale preview must fail");
    assert!(error.contains("changed after inspection") || error.contains("result_tag_shape"));
    assert!(!fixture.target_path.exists());

    let mut request = fixture.request(&fixture.target_path);
    request.conditions = "created_at>1&created_at<2".to_string();
    request.signing_preimage = format!(
        "nostr:agent-auth:{}:{}",
        request.agent_pubkey, request.conditions
    );
    request.result_tag_shape[2] = request.conditions.clone();
    fixture.write_request(Some(request));
    assert!(inspect_request(
        &fixture.request_path,
        &fixture.owner_keys.public_key(),
        fixture.now
    )
    .unwrap_err()
    .contains("not current"));
    assert!(!fixture.target_path.exists());
}

#[test]
fn validity_over_ninety_days_is_rejected_without_output() {
    let fixture = Fixture::new();
    let mut request = fixture.request(&fixture.target_path);
    request.conditions = format!(
        "created_at>{}&created_at<{}",
        fixture.now - 10,
        fixture.now + MAX_VALIDITY_SECONDS + 10
    );
    request.signing_preimage = format!(
        "nostr:agent-auth:{}:{}",
        request.agent_pubkey, request.conditions
    );
    request.result_tag_shape[2] = request.conditions.clone();
    fixture.write_request(Some(request));

    let error = inspect_request(
        &fixture.request_path,
        &fixture.owner_keys.public_key(),
        fixture.now,
    )
    .expect_err("overlong validity must fail");

    assert!(error.contains("at most"));
    assert!(!fixture.target_path.exists());
}

#[test]
fn request_requires_explicit_fingerprint_and_result_path() {
    let fixture = Fixture::new();
    let incomplete = serde_json::json!({
        "schema": REQUEST_SCHEMA,
        "agent_pubkey": fixture.agent_keys.public_key().to_hex(),
        "conditions": fixture.conditions.clone(),
        "signing_preimage": "not-reached",
        "signing_hash_algorithm": "SHA256",
        "signature_algorithm": "BIP340_Schnorr_secp256k1",
        "result_tag_shape": ["auth", "OWNER_PUBLIC_KEY_HEX", "not-reached", "OWNER_SIGNATURE_HEX"],
        "private_key_in_request": false,
        "signed": false
    });
    std::fs::write(
        &fixture.request_path,
        serde_json::to_vec_pretty(&incomplete).unwrap(),
    )
    .unwrap();
    std::fs::set_permissions(
        &fixture.request_path,
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let error = inspect_request(
        &fixture.request_path,
        &fixture.owner_keys.public_key(),
        fixture.now,
    )
    .expect_err("implicit custody bindings must fail");

    assert!(error.contains("agent_public_fingerprint_sha256"));
    assert!(!fixture.target_path.exists());
}
