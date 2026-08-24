use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use nostr::{
    hashes::{sha256::Hash as Sha256Hash, Hash},
    Keys, PublicKey,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::app_state::AppState;

const REQUEST_SCHEMA: &str = "buzz.nip-oa-owner-attestation-request.v1";
const REQUEST_FILE_NAME: &str = "OWNER_ATTESTATION_REQUEST.json";
const TARGET_FILE_NAME: &str = "BUZZ_AUTH_TAG";
const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_VALIDITY_SECONDS: u64 = 90 * 24 * 60 * 60;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnerAttestationRequest {
    schema: String,
    agent_pubkey: String,
    agent_public_fingerprint_sha256: String,
    conditions: String,
    signing_preimage: String,
    signing_hash_algorithm: String,
    signature_algorithm: String,
    result_tag_shape: [String; 4],
    result_path: String,
    private_key_in_request: bool,
    signed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerAttestationPreview {
    request_path: String,
    request_sha256: String,
    schema: String,
    agent_pubkey: String,
    agent_public_fingerprint_sha256: String,
    owner_pubkey: String,
    conditions: String,
    signing_preimage: String,
    signing_hash_algorithm: String,
    signature_algorithm: String,
    result_tag_shape: [String; 4],
    result_path: String,
    valid_from: u64,
    expires_at: u64,
    validity_seconds: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerAttestationWriteReceipt {
    request_path: String,
    request_sha256: String,
    owner_pubkey: String,
    result_path: String,
    written: bool,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    dev: u64,
    ino: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    links: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
}

#[cfg(unix)]
impl FileIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;

        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode() & 0o7777,
            links: metadata.nlink(),
            len: metadata.len(),
            mtime: metadata.mtime(),
            mtime_nsec: metadata.mtime_nsec(),
        }
    }

    fn same_directory(&self, other: &Self) -> bool {
        self.dev == other.dev
            && self.ino == other.ino
            && self.uid == other.uid
            && self.gid == other.gid
            && self.mode == other.mode
    }
}

#[cfg(unix)]
struct ValidatedRequest {
    request: OwnerAttestationRequest,
    request_path: PathBuf,
    target_path: PathBuf,
    request_sha256: String,
    request_identity: FileIdentity,
    parent_identity: FileIdentity,
    agent_pubkey: PublicKey,
    valid_from: u64,
    expires_at: u64,
}

/// Open and validate an owner-attestation request selected by the user.
///
/// This command is inspection-only. It does not sign, write, publish, create an
/// agent, or return any secret material.
#[tauri::command]
pub async fn select_owner_attestation_request(
    app_handle: AppHandle,
) -> Result<Option<OwnerAttestationPreview>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app_handle
        .dialog()
        .file()
        .add_filter("Owner attestation request", &["json"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });

    let selected = rx.await.map_err(|_| "request dialog cancelled".to_string())?;
    let Some(file_path) = selected else {
        return Ok(None);
    };
    let request_path = file_path
        .as_path()
        .ok_or_else(|| "request dialog returned an invalid path".to_string())?
        .to_path_buf();

    let owner_pubkey = app_handle
        .state::<AppState>()
        .signing_keys()?
        .public_key();
    let now = unix_time_now()?;
    tokio::task::spawn_blocking(move || inspect_request(&request_path, &owner_pubkey, now))
        .await
        .map_err(|error| format!("request inspection task failed: {error}"))?
        .map(Some)
}

/// Sign one previously inspected request and atomically create its protected
/// result file. The auth tag and signature never cross the Rust IPC boundary.
#[tauri::command]
pub async fn sign_owner_attestation_request(
    request_path: String,
    expected_request_sha256: String,
    expected_owner_pubkey: String,
    app_handle: AppHandle,
) -> Result<OwnerAttestationWriteReceipt, String> {
    tokio::task::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        let _identity_guard = state.identity_mutation.lock().map_err(|e| e.to_string())?;
        let owner_keys = state.signing_keys()?;
        sign_request(
            Path::new(&request_path),
            &expected_request_sha256,
            &expected_owner_pubkey,
            &owner_keys,
            unix_time_now()?,
        )
    })
    .await
    .map_err(|error| format!("owner attestation task failed: {error}"))?
}

fn unix_time_now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))
        .map(|duration| duration.as_secs())
}

#[cfg(not(unix))]
fn inspect_request(
    _request_path: &Path,
    _owner_pubkey: &PublicKey,
    _now: u64,
) -> Result<OwnerAttestationPreview, String> {
    Err("owner attestation requires Unix owner and mode enforcement".to_string())
}

#[cfg(unix)]
fn inspect_request(
    request_path: &Path,
    owner_pubkey: &PublicKey,
    now: u64,
) -> Result<OwnerAttestationPreview, String> {
    let validated = load_and_validate_request(request_path, now)?;
    if *owner_pubkey == validated.agent_pubkey {
        return Err("owner and agent pubkeys must differ".to_string());
    }

    let validity_seconds = validated
        .expires_at
        .checked_sub(validated.valid_from)
        .ok_or_else(|| "attestation validity bounds are reversed".to_string())?;
    Ok(OwnerAttestationPreview {
        request_path: validated.request_path.display().to_string(),
        request_sha256: validated.request_sha256,
        schema: validated.request.schema,
        agent_pubkey: validated.request.agent_pubkey,
        agent_public_fingerprint_sha256: validated.request.agent_public_fingerprint_sha256,
        owner_pubkey: owner_pubkey.to_hex(),
        conditions: validated.request.conditions,
        signing_preimage: validated.request.signing_preimage,
        signing_hash_algorithm: validated.request.signing_hash_algorithm,
        signature_algorithm: validated.request.signature_algorithm,
        result_tag_shape: validated.request.result_tag_shape,
        result_path: validated.target_path.display().to_string(),
        valid_from: validated.valid_from,
        expires_at: validated.expires_at,
        validity_seconds,
    })
}

#[cfg(not(unix))]
fn sign_request(
    _request_path: &Path,
    _expected_request_sha256: &str,
    _expected_owner_pubkey: &str,
    _owner_keys: &Keys,
    _now: u64,
) -> Result<OwnerAttestationWriteReceipt, String> {
    Err("owner attestation requires Unix owner and mode enforcement".to_string())
}

#[cfg(unix)]
fn sign_request(
    request_path: &Path,
    expected_request_sha256: &str,
    expected_owner_pubkey: &str,
    owner_keys: &Keys,
    now: u64,
) -> Result<OwnerAttestationWriteReceipt, String> {
    let first = load_and_validate_request(request_path, now)?;
    if first.request_sha256 != expected_request_sha256 {
        return Err("request bytes changed after inspection; select it again".to_string());
    }

    let owner_pubkey = owner_keys.public_key();
    if owner_pubkey.to_hex() != expected_owner_pubkey {
        return Err(
            "Desktop owner identity changed after inspection; select the request again".into(),
        );
    }
    if owner_pubkey == first.agent_pubkey {
        return Err("owner and agent pubkeys must differ".to_string());
    }

    // Reuse the canonical NIP-OA primitive. The request conditions are passed
    // verbatim; the primitive hashes the exact specified preimage and produces
    // a BIP-340 Schnorr signature.
    let auth_tag = buzz_sdk_pkg::nip_oa::compute_auth_tag(
        owner_keys,
        &first.agent_pubkey,
        &first.request.conditions,
    )
    .map_err(|error| format!("owner attestation validation failed: {error}"))?;

    verify_computed_tag(&auth_tag, &first, &owner_pubkey)?;

    // Re-open and re-validate immediately before the only external effect.
    // Exact bytes, inode metadata, parent directory identity, owner identity,
    // validity, and target absence must all remain bound to the preview.
    let second = load_and_validate_request(request_path, now)?;
    if second.request_sha256 != first.request_sha256
        || second.request_identity != first.request_identity
        || second.parent_identity != first.parent_identity
        || second.request != first.request
    {
        return Err("request or custody directory changed before commit; nothing was written".into());
    }
    if owner_keys.public_key() != owner_pubkey {
        return Err("Desktop owner identity changed before commit; nothing was written".into());
    }

    atomic_create_secret(
        &second.target_path,
        auth_tag.as_bytes(),
        second.parent_identity,
    )?;

    Ok(OwnerAttestationWriteReceipt {
        request_path: second.request_path.display().to_string(),
        request_sha256: second.request_sha256,
        owner_pubkey: owner_pubkey.to_hex(),
        result_path: second.target_path.display().to_string(),
        written: true,
    })
}

#[cfg(unix)]
fn load_and_validate_request(request_path: &Path, now: u64) -> Result<ValidatedRequest, String> {
    use std::os::unix::fs::OpenOptionsExt;

    validate_normal_absolute_path(request_path, REQUEST_FILE_NAME)?;
    ensure_no_symlink_components(request_path)?;

    let parent = request_path
        .parent()
        .ok_or_else(|| "request path has no parent directory".to_string())?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("inspect request directory: {error}"))?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err("request directory must be a regular non-symlink directory".into());
    }
    let parent_identity = FileIdentity::from_metadata(&parent_metadata);
    if parent_identity.mode != 0o700 {
        return Err(format!(
            "request directory mode must be 0700, got {:04o}",
            parent_identity.mode
        ));
    }

    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options
        .open(request_path)
        .map_err(|error| format!("open owner attestation request: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspect owner attestation request: {error}"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("request must be a regular non-symlink file".into());
    }
    let request_identity = FileIdentity::from_metadata(&metadata);
    if request_identity.links != 1 {
        return Err("request must have exactly one hard link".into());
    }
    if request_identity.mode != 0o644 {
        return Err(format!(
            "request mode must be 0644, got {:04o}",
            request_identity.mode
        ));
    }
    if request_identity.uid != parent_identity.uid || request_identity.gid != parent_identity.gid {
        return Err("request owner and group must match its custody directory".into());
    }
    if request_identity.len == 0 || request_identity.len > MAX_REQUEST_BYTES {
        return Err(format!(
            "request size must be between 1 and {MAX_REQUEST_BYTES} bytes"
        ));
    }

    let mut raw = Vec::with_capacity(request_identity.len as usize);
    file.read_to_end(&mut raw)
        .map_err(|error| format!("read owner attestation request: {error}"))?;
    if raw.len() as u64 != request_identity.len {
        return Err("request length changed while it was read".into());
    }
    let request_sha256 = Sha256Hash::hash(&raw).to_string();
    let request: OwnerAttestationRequest = serde_json::from_slice(&raw)
        .map_err(|error| format!("invalid owner attestation request JSON: {error}"))?;

    if request.schema != REQUEST_SCHEMA {
        return Err(format!(
            "unsupported owner attestation request schema: {}",
            request.schema
        ));
    }
    if request.private_key_in_request {
        return Err("request must declare private_key_in_request=false".into());
    }
    if request.signed {
        return Err("request must be unsigned".into());
    }
    if request.conditions.is_empty() {
        return Err("owner attestation conditions must be non-empty".into());
    }
    if request.signing_hash_algorithm != "SHA256" {
        return Err("signing_hash_algorithm must be SHA256".into());
    }
    if request.signature_algorithm != "BIP340_Schnorr_secp256k1" {
        return Err("signature_algorithm must be BIP340_Schnorr_secp256k1".into());
    }
    if request.agent_pubkey.len() != 64 || !is_lowercase_hex(&request.agent_pubkey) {
        return Err("agent_pubkey must be exactly 64 lowercase hex characters".into());
    }
    let agent_pubkey = PublicKey::from_hex(&request.agent_pubkey)
        .map_err(|error| format!("invalid agent_pubkey: {error}"))?;
    validate_agent_fingerprint(
        &request.agent_pubkey,
        &request.agent_public_fingerprint_sha256,
    )?;

    let expected_preimage = format!(
        "nostr:agent-auth:{}:{}",
        request.agent_pubkey, request.conditions
    );
    if request.signing_preimage.as_bytes() != expected_preimage.as_bytes() {
        return Err("signing_preimage does not byte-exactly bind agent_pubkey and conditions".into());
    }
    let expected_shape = [
        "auth".to_string(),
        "OWNER_PUBLIC_KEY_HEX".to_string(),
        request.conditions.clone(),
        "OWNER_SIGNATURE_HEX".to_string(),
    ];
    if request.result_tag_shape != expected_shape {
        return Err("result_tag_shape does not byte-exactly bind conditions".into());
    }

    let (valid_from, expires_at) = validate_validity(&request.conditions, now)?;
    let target_path = PathBuf::from(&request.result_path);
    validate_normal_absolute_path(&target_path, TARGET_FILE_NAME)?;
    if target_path.parent() != Some(parent) {
        return Err("result_path must be the exact BUZZ_AUTH_TAG sibling of the request".into());
    }
    ensure_no_symlink_components(parent)?;
    ensure_target_absent(&target_path)?;

    Ok(ValidatedRequest {
        request,
        request_path: request_path.to_path_buf(),
        target_path,
        request_sha256,
        request_identity,
        parent_identity,
        agent_pubkey,
        valid_from,
        expires_at,
    })
}

fn validate_normal_absolute_path(path: &Path, expected_name: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("path must be absolute".into());
    }
    if path.file_name().and_then(|value| value.to_str()) != Some(expected_name) {
        return Err(format!("path must end in {expected_name}"));
    }
    if path.components().any(|component| {
        matches!(component, Component::CurDir | Component::ParentDir)
    }) {
        return Err("path must not contain '.' or '..' components".into());
    }
    Ok(())
}

fn ensure_no_symlink_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|error| format!("inspect path component {}: {error}", current.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("path component must not be a symlink: {}", current.display()));
        }
    }
    Ok(())
}

fn ensure_target_absent(target_path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(target_path) {
        Ok(_) => Err(
            "BUZZ_AUTH_TAG target already exists or is a symlink; refusing to replace it".into(),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("inspect BUZZ_AUTH_TAG target: {error}")),
    }
}

fn validate_agent_fingerprint(agent_pubkey: &str, fingerprint: &str) -> Result<(), String> {
    if fingerprint.len() != 64 || !is_lowercase_hex(fingerprint) {
        return Err("agent_public_fingerprint_sha256 must be 64 lowercase hex characters".into());
    }
    let xonly = hex::decode(agent_pubkey)
        .map_err(|error| format!("decode agent_pubkey for fingerprint: {error}"))?;
    let mut even = Vec::with_capacity(33);
    even.push(0x02);
    even.extend_from_slice(&xonly);
    let mut odd = even.clone();
    odd[0] = 0x03;
    let even_fingerprint = Sha256Hash::hash(&even).to_string();
    let odd_fingerprint = Sha256Hash::hash(&odd).to_string();
    if fingerprint != even_fingerprint && fingerprint != odd_fingerprint {
        return Err("agent public fingerprint does not bind the compressed agent pubkey".into());
    }
    Ok(())
}

fn validate_validity(conditions: &str, now: u64) -> Result<(u64, u64), String> {
    let mut valid_from = None;
    let mut expires_at = None;
    for clause in conditions.split('&') {
        if let Some(value) = clause.strip_prefix("created_at>") {
            if valid_from.is_some() {
                return Err("conditions contain duplicate created_at> bounds".into());
            }
            valid_from = Some(parse_canonical_u32(value, "created_at>")?);
        } else if let Some(value) = clause.strip_prefix("created_at<") {
            if expires_at.is_some() {
                return Err("conditions contain duplicate created_at< bounds".into());
            }
            expires_at = Some(parse_canonical_u32(value, "created_at<")?);
        } else if let Some(value) = clause.strip_prefix("kind=") {
            let kind = parse_canonical_u32(value, "kind")?;
            if kind > 65_535 {
                return Err("kind condition is out of range".into());
            }
        } else {
            return Err("conditions contain an unsupported or empty clause".into());
        }
    }
    let valid_from =
        valid_from.ok_or_else(|| "conditions require one created_at> bound".to_string())?;
    let expires_at =
        expires_at.ok_or_else(|| "conditions require one created_at< bound".to_string())?;
    let validity = expires_at
        .checked_sub(valid_from)
        .ok_or_else(|| "attestation validity bounds are reversed".to_string())?;
    if validity == 0 || validity > MAX_VALIDITY_SECONDS {
        return Err(format!(
            "attestation validity must be positive and at most {MAX_VALIDITY_SECONDS} seconds"
        ));
    }
    if now <= valid_from || now >= expires_at {
        return Err("attestation validity window is not current".into());
    }
    Ok((valid_from, expires_at))
}

fn parse_canonical_u32(value: &str, label: &str) -> Result<u64, String> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(format!("{label} must use canonical unsigned decimal encoding"));
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|error| format!("{label} is out of range: {error}"))?;
    if parsed > u32::MAX as u64 {
        return Err(format!("{label} is out of range"));
    }
    Ok(parsed)
}

fn is_lowercase_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(unix)]
fn verify_computed_tag(
    auth_tag: &str,
    request: &ValidatedRequest,
    owner_pubkey: &PublicKey,
) -> Result<(), String> {
    let recovered = buzz_sdk_pkg::nip_oa::verify_auth_tag(auth_tag, &request.agent_pubkey)
        .map_err(|error| format!("computed owner attestation did not verify: {error}"))?;
    if recovered != *owner_pubkey {
        return Err("computed owner attestation owner binding failed".into());
    }
    let parts: Vec<String> = serde_json::from_str(auth_tag)
        .map_err(|error| format!("computed owner attestation shape failed: {error}"))?;
    if parts.len() != 4
        || parts[0] != "auth"
        || parts[1] != owner_pubkey.to_hex()
        || parts[2].as_bytes() != request.request.conditions.as_bytes()
        || parts[3].len() != 128
        || !is_lowercase_hex(&parts[3])
    {
        return Err("computed owner attestation has an invalid protected tag shape".into());
    }
    Ok(())
}

#[cfg(unix)]
fn atomic_create_secret(
    target_path: &Path,
    bytes: &[u8],
    expected_parent: FileIdentity,
) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;

    let parent = target_path
        .parent()
        .ok_or_else(|| "BUZZ_AUTH_TAG target has no parent directory".to_string())?;
    let current_parent = FileIdentity::from_metadata(
        &std::fs::symlink_metadata(parent)
            .map_err(|error| format!("reinspect custody directory: {error}"))?,
    );
    if !current_parent.same_directory(&expected_parent) {
        return Err("custody directory changed before write; nothing was written".into());
    }
    ensure_target_absent(target_path)?;

    let temp_path = parent.join(format!(
        ".{TARGET_FILE_NAME}.{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let mut cleanup = TempFileCleanup(Some(temp_path.clone()));
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut temp = options
        .open(&temp_path)
        .map_err(|error| format!("create protected temporary attestation file: {error}"))?;
    let temp_identity = FileIdentity::from_metadata(
        &temp
            .metadata()
            .map_err(|error| format!("inspect protected temporary attestation file: {error}"))?,
    );
    if temp_identity.mode != 0o600
        || temp_identity.uid != expected_parent.uid
        || temp_identity.gid != expected_parent.gid
        || temp_identity.links != 1
    {
        return Err("protected temporary attestation file failed owner/mode/link checks".into());
    }

    temp.write_all(bytes)
        .map_err(|error| format!("write protected temporary attestation file: {error}"))?;
    temp.sync_all()
        .map_err(|error| format!("sync protected temporary attestation file: {error}"))?;
    drop(temp);
    let persisted = std::fs::read(&temp_path)
        .map_err(|error| format!("verify protected temporary attestation file: {error}"))?;
    if persisted.as_slice() != bytes {
        return Err("protected temporary attestation file reread mismatch".into());
    }

    let current_parent = FileIdentity::from_metadata(
        &std::fs::symlink_metadata(parent)
            .map_err(|error| format!("reinspect custody directory before commit: {error}"))?,
    );
    if !current_parent.same_directory(&expected_parent) {
        return Err("custody directory changed before commit; nothing was written".into());
    }
    ensure_target_absent(target_path)?;

    // Linking a fully written, synced same-directory inode is an atomic,
    // no-replace commit: EEXIST preserves any concurrently created target.
    std::fs::hard_link(&temp_path, target_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            "BUZZ_AUTH_TAG target appeared before commit; refusing to replace it".to_string()
        } else {
            format!("atomically commit BUZZ_AUTH_TAG without replacement: {error}")
        }
    })?;

    if let Err(error) = std::fs::remove_file(&temp_path) {
        return Err(format!(
            "BUZZ_AUTH_TAG may have been committed but temporary-link cleanup failed: {error}; STOP and do not retry"
        ));
    }
    cleanup.0 = None;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| {
            format!(
                "BUZZ_AUTH_TAG was committed but custody-directory sync failed: {error}; STOP and do not retry"
            )
        })?;

    let target_metadata = std::fs::symlink_metadata(target_path).map_err(|error| {
        format!(
            "BUZZ_AUTH_TAG was committed but metadata verification failed: {error}; STOP and do not retry"
        )
    })?;
    let target_identity = FileIdentity::from_metadata(&target_metadata);
    if !target_metadata.is_file()
        || target_metadata.file_type().is_symlink()
        || target_identity.mode != 0o600
        || target_identity.uid != expected_parent.uid
        || target_identity.gid != expected_parent.gid
        || target_identity.links != 1
    {
        return Err(
            "BUZZ_AUTH_TAG was committed but failed regular-file/owner/mode/link verification; STOP and do not retry"
                .into(),
        );
    }
    let target_bytes = std::fs::read(target_path).map_err(|error| {
        format!(
            "BUZZ_AUTH_TAG was committed but reread failed: {error}; STOP and do not retry"
        )
    })?;
    if target_bytes.as_slice() != bytes {
        return Err("BUZZ_AUTH_TAG was committed but reread mismatched; STOP and do not retry".into());
    }
    Ok(())
}

struct TempFileCleanup(Option<PathBuf>);

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(all(test, unix))]
#[path = "owner_attestation_tests.rs"]
mod tests;
