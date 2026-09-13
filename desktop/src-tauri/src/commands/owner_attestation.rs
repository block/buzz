use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use nostr::{
    hashes::{sha256::Hash as Sha256Hash, Hash},
    Keys, PublicKey,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

#[cfg(unix)]
use rustix::{
    fs::{AtFlags, Mode, OFlags},
    io::Errno,
};

use crate::app_state::AppState;

const REQUEST_SCHEMA: &str = "buzz.nip-oa-owner-attestation-request.v1";
const REQUEST_FILE_NAME: &str = "OWNER_ATTESTATION_REQUEST.json";
const TARGET_FILE_NAME: &str = "BUZZ_AUTH_TAG";
const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const MAX_CONFIRMATION_CONDITIONS_BYTES: usize = 512;
const MAX_CONFIRMATION_PATH_BYTES: usize = 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnerAttestationRequest {
    schema: String,
    agent_pubkey: String,
    conditions: String,
    signing_preimage: String,
    signing_hash_algorithm: String,
    signature_algorithm: String,
    result_tag_shape: [String; 4],
    private_key_in_request: bool,
    signed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerAttestationPreview {
    preview_id: String,
    agent_pubkey: String,
    owner_pubkey: String,
    conditions: String,
    result_path: String,
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

    fn same_stable_file_identity(&self, other: &Self) -> bool {
        self.dev == other.dev
            && self.ino == other.ino
            && self.uid == other.uid
            && self.gid == other.gid
            && self.mode == other.mode
            && self.links == other.links
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
}

#[cfg(unix)]
pub(crate) struct PreparedOwnerAttestation {
    preview_id: String,
    validated: ValidatedRequest,
    owner_pubkey: PublicKey,
}

#[cfg(unix)]
impl PreparedOwnerAttestation {
    fn preview(&self) -> OwnerAttestationPreview {
        OwnerAttestationPreview {
            preview_id: self.preview_id.clone(),
            agent_pubkey: self.validated.request.agent_pubkey.clone(),
            owner_pubkey: self.owner_pubkey.to_hex(),
            conditions: self.validated.request.conditions.clone(),
            result_path: self.validated.target_path.display().to_string(),
        }
    }
}

#[cfg(unix)]
#[derive(Default)]
pub(crate) struct OwnerAttestationPreviewStore {
    current: Option<PreparedOwnerAttestation>,
}

#[cfg(unix)]
impl OwnerAttestationPreviewStore {
    fn clear(&mut self) {
        self.current = None;
    }

    fn replace(&mut self, prepared: PreparedOwnerAttestation) -> OwnerAttestationPreview {
        let preview = prepared.preview();
        self.current = Some(prepared);
        preview
    }

    fn take(&mut self, preview_id: &str) -> Result<PreparedOwnerAttestation, String> {
        let Some(current) = self.current.as_ref() else {
            return Err("owner attestation preview is missing or already consumed; select the request again".into());
        };
        if current.preview_id != preview_id {
            return Err("owner attestation preview is stale; select the request again".into());
        }
        self.current
            .take()
            .ok_or_else(|| "owner attestation preview was already consumed".to_string())
    }
}

#[cfg(not(unix))]
#[derive(Default)]
pub(crate) struct OwnerAttestationPreviewStore;

#[cfg(unix)]
struct PinnedDirectory {
    root: File,
    directory: File,
    path_components: Vec<OsString>,
    path: PathBuf,
    identity: FileIdentity,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TempLinkState {
    NotCreated,
    Preparing,
    Linked,
    CleanupAmbiguous,
    TempUnlinked,
    Committed,
}

/// Open and validate an owner-attestation request selected by the user.
///
/// This command is inspection-only. It does not sign, write, publish, create an
/// agent, or return any secret material.
#[tauri::command]
pub async fn select_owner_attestation_request(
    app_handle: AppHandle,
) -> Result<Option<OwnerAttestationPreview>, String> {
    #[cfg(not(unix))]
    {
        let _ = app_handle;
        return Err("owner attestation requires Unix owner and mode enforcement".to_string());
    }

    #[cfg(unix)]
    {
        app_handle
            .state::<AppState>()
            .owner_attestation_previews
            .lock()
            .map_err(|error| error.to_string())?
            .clear();
        let (tx, rx) = tokio::sync::oneshot::channel();
        app_handle
            .dialog()
            .file()
            .add_filter("Owner attestation request", &["json"])
            .pick_file(move |path| {
                let _ = tx.send(path);
            });

        let selected = rx
            .await
            .map_err(|_| "request dialog cancelled".to_string())?;
        let Some(file_path) = selected else {
            return Ok(None);
        };
        let request_path = file_path
            .as_path()
            .ok_or_else(|| "request dialog returned an invalid path".to_string())?
            .to_path_buf();

        let owner_pubkey = app_handle.state::<AppState>().signing_keys()?.public_key();
        let prepared =
            tokio::task::spawn_blocking(move || prepare_request(&request_path, &owner_pubkey))
                .await
                .map_err(|error| format!("request inspection task failed: {error}"))??;
        let preview = app_handle
            .state::<AppState>()
            .owner_attestation_previews
            .lock()
            .map_err(|error| error.to_string())?
            .replace(prepared);
        Ok(Some(preview))
    }
}

/// Sign one previously inspected request and atomically create its protected
/// result file. The auth tag and signature never cross the Rust IPC boundary.
#[tauri::command]
pub async fn sign_owner_attestation_request(
    preview_id: String,
    app_handle: AppHandle,
) -> Result<(), String> {
    #[cfg(not(unix))]
    {
        let _ = (preview_id, app_handle);
        return Err("owner attestation requires Unix owner and mode enforcement".to_string());
    }

    #[cfg(unix)]
    {
        tokio::task::spawn_blocking(move || {
            let state = app_handle.state::<AppState>();
            let prepared = state
                .owner_attestation_previews
                .lock()
                .map_err(|error| error.to_string())?
                .take(&preview_id)?;
            let confirmation = confirmation_message(&prepared)?;
            confirm_and_execute_prepared(
                &prepared,
                |_| {
                    app_handle
                        .dialog()
                        .message(confirmation)
                        .title("Sign this exact owner attestation?")
                        .kind(MessageDialogKind::Warning)
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            "Sign once".to_string(),
                            "Cancel".to_string(),
                        ))
                        .blocking_show()
                },
                |prepared| {
                    // Never hold the identity mutation lock while waiting for
                    // an unbounded native-dialog interaction. Re-acquire it
                    // only after confirmation and revalidate under the lock.
                    let _identity_guard =
                        state.identity_mutation.lock().map_err(|e| e.to_string())?;
                    let owner_keys = state.signing_keys()?;
                    sign_prepared_request(prepared, &owner_keys)
                },
            )
        })
        .await
        .map_err(|error| format!("owner attestation task failed: {error}"))?
    }
}

#[cfg(not(unix))]
fn inspect_request(
    _request_path: &Path,
    _owner_pubkey: &PublicKey,
) -> Result<OwnerAttestationPreview, String> {
    Err("owner attestation requires Unix owner and mode enforcement".to_string())
}

#[cfg(unix)]
fn inspect_request(
    request_path: &Path,
    owner_pubkey: &PublicKey,
) -> Result<OwnerAttestationPreview, String> {
    prepare_request(request_path, owner_pubkey).map(|prepared| prepared.preview())
}

#[cfg(unix)]
fn prepare_request(
    request_path: &Path,
    owner_pubkey: &PublicKey,
) -> Result<PreparedOwnerAttestation, String> {
    let validated = load_and_validate_request(request_path)?;
    if *owner_pubkey == validated.agent_pubkey {
        return Err("owner and agent pubkeys must differ".to_string());
    }

    let prepared = PreparedOwnerAttestation {
        preview_id: uuid::Uuid::new_v4().to_string(),
        validated,
        owner_pubkey: *owner_pubkey,
    };
    // Fail inspection if the exact confirmation cannot be rendered as a
    // bounded, unambiguous, printf-safe ASCII representation.
    confirmation_message(&prepared)?;
    Ok(prepared)
}

#[cfg(unix)]
fn confirmation_message(prepared: &PreparedOwnerAttestation) -> Result<String, String> {
    let conditions = escape_confirmation_field(
        &prepared.validated.request.conditions,
        "conditions",
        MAX_CONFIRMATION_CONDITIONS_BYTES,
    )?;
    let target_path = prepared
        .validated
        .target_path
        .to_str()
        .ok_or_else(|| "authorized result path must be valid UTF-8".to_string())?;
    let target_path = escape_confirmation_field(
        target_path,
        "authorized result path",
        MAX_CONFIRMATION_PATH_BYTES,
    )?;
    Ok(format!(
        "Buzz Desktop will sign exactly this authorization and create BUZZ_AUTH_TAG once.\n\nAgent public key:\n{}\n\nDesktop owner public key:\n{}\n\nConditions (escaped ASCII):\n{}\n\nAuthorized result path (escaped ASCII):\n{}\n\nThe private key and signature stay inside Desktop. Nothing is published and no agent is created.",
        prepared.validated.request.agent_pubkey,
        prepared.owner_pubkey.to_hex(),
        conditions,
        target_path,
    ))
}

#[cfg(unix)]
fn escape_confirmation_field(value: &str, label: &str, max_bytes: usize) -> Result<String, String> {
    if value.len() > max_bytes {
        return Err(format!(
            "{label} is too long for native confirmation (maximum {max_bytes} UTF-8 bytes)"
        ));
    }

    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric()
            || matches!(
                character,
                '/' | '.' | '_' | '-' | ':' | '=' | '<' | '>' | '&' | ' '
            )
        {
            escaped.push(character);
        } else {
            escaped.push_str(&format!(r"\u{{{:X}}}", character as u32));
        }
    }
    debug_assert!(!escaped.contains('%') && !escaped.contains('\0'));
    Ok(escaped)
}

#[cfg(unix)]
fn confirm_and_execute_prepared<C, E>(
    prepared: &PreparedOwnerAttestation,
    confirm: C,
    execute: E,
) -> Result<(), String>
where
    C: FnOnce(&PreparedOwnerAttestation) -> bool,
    E: FnOnce(&PreparedOwnerAttestation) -> Result<(), String>,
{
    validate_prepared_filesystem(prepared)?;
    if !confirm(prepared) {
        return Err(
            "owner cancelled the native signing confirmation; select the request again".into(),
        );
    }
    execute(prepared)
}

#[cfg(unix)]
fn validate_prepared_request(
    prepared: &PreparedOwnerAttestation,
    owner_keys: &Keys,
) -> Result<(), String> {
    let owner_pubkey = owner_keys.public_key();
    if owner_pubkey != prepared.owner_pubkey {
        return Err(
            "Desktop owner identity changed after inspection; select the request again".into(),
        );
    }
    validate_prepared_filesystem(prepared)
}

#[cfg(unix)]
fn validate_prepared_filesystem(prepared: &PreparedOwnerAttestation) -> Result<(), String> {
    let custody = open_pinned_directory(&prepared.validated.request_path)?;
    let current = load_and_validate_request_in(&custody, &prepared.validated.request_path)?;
    if current.request_sha256 != prepared.validated.request_sha256
        || current.request_identity != prepared.validated.request_identity
        || current.parent_identity != prepared.validated.parent_identity
        || current.request != prepared.validated.request
        || current.request_path != prepared.validated.request_path
        || current.target_path != prepared.validated.target_path
    {
        return Err(
            "request or custody directory no longer matches the inspected preview; select it again"
                .into(),
        );
    }
    if prepared.owner_pubkey == current.agent_pubkey {
        return Err("owner and agent pubkeys must differ".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn sign_prepared_request(
    prepared: &PreparedOwnerAttestation,
    owner_keys: &Keys,
) -> Result<(), String> {
    validate_prepared_request(prepared, owner_keys)?;
    let custody = open_pinned_directory(&prepared.validated.request_path)?;
    let first = load_and_validate_request_in(&custody, &prepared.validated.request_path)?;
    if first.request_sha256 != prepared.validated.request_sha256
        || first.request_identity != prepared.validated.request_identity
        || first.parent_identity != prepared.validated.parent_identity
        || first.request != prepared.validated.request
    {
        return Err(
            "request or custody directory no longer matches the inspected preview; select it again"
                .into(),
        );
    }
    let owner_pubkey = owner_keys.public_key();

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
    // target absence must all remain bound to the preview.
    let second = load_and_validate_request_in(&custody, &prepared.validated.request_path)?;
    if second.request_sha256 != first.request_sha256
        || second.request_identity != first.request_identity
        || second.parent_identity != first.parent_identity
        || second.request != first.request
    {
        return Err(
            "request or custody directory changed before commit; nothing was written".into(),
        );
    }
    if owner_keys.public_key() != owner_pubkey {
        return Err("Desktop owner identity changed before commit; nothing was written".into());
    }

    atomic_create_secret(&custody, auth_tag.as_bytes())?;
    Ok(())
}

#[cfg(unix)]
fn load_and_validate_request(request_path: &Path) -> Result<ValidatedRequest, String> {
    let custody = open_pinned_directory(request_path)?;
    load_and_validate_request_in(&custody, request_path)
}

#[cfg(unix)]
fn open_pinned_directory(request_path: &Path) -> Result<PinnedDirectory, String> {
    validate_normal_absolute_path(request_path, REQUEST_FILE_NAME)?;
    let parent = request_path
        .parent()
        .ok_or_else(|| "request path has no parent directory".to_string())?;

    let mut path_components = Vec::new();
    for component in parent.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) => path_components.push(value.to_os_string()),
            _ => return Err("custody path contains a non-normal component".into()),
        }
    }

    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let root = File::from(
        rustix::fs::open("/", flags, Mode::empty())
            .map_err(|error| format!("open root directory for custody resolution: {error}"))?,
    );
    let directory = open_directory_components(&root, &path_components)?;
    let metadata = directory
        .metadata()
        .map_err(|error| format!("inspect pinned custody directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("request directory must be a regular non-symlink directory".into());
    }
    let identity = FileIdentity::from_metadata(&metadata);
    if identity.mode != 0o700 {
        return Err(format!(
            "request directory mode must be 0700, got {:04o}",
            identity.mode
        ));
    }

    Ok(PinnedDirectory {
        root,
        directory,
        path_components,
        path: parent.to_path_buf(),
        identity,
    })
}

#[cfg(unix)]
fn open_directory_components(root: &File, components: &[OsString]) -> Result<File, String> {
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut current = File::from(
        rustix::fs::openat(root, OsStr::new("."), flags, Mode::empty())
            .map_err(|error| format!("pin root custody directory: {error}"))?,
    );
    for component in components {
        current = File::from(
            rustix::fs::openat(&current, component.as_os_str(), flags, Mode::empty()).map_err(
                |error| {
                    format!(
                        "open non-symlink custody path component {}: {error}",
                        component.to_string_lossy()
                    )
                },
            )?,
        );
    }
    Ok(current)
}

#[cfg(unix)]
fn verify_current_path_binding(custody: &PinnedDirectory) -> Result<(), String> {
    let current = open_directory_components(&custody.root, &custody.path_components)?;
    let current_identity = FileIdentity::from_metadata(
        &current
            .metadata()
            .map_err(|error| format!("reinspect custody path binding: {error}"))?,
    );
    if !current_identity.same_directory(&custody.identity) {
        return Err("custody directory path was renamed or replaced".into());
    }
    Ok(())
}

#[cfg(unix)]
fn load_and_validate_request_in(
    custody: &PinnedDirectory,
    request_path: &Path,
) -> Result<ValidatedRequest, String> {
    validate_normal_absolute_path(request_path, REQUEST_FILE_NAME)?;
    if request_path.parent() != Some(custody.path.as_path()) {
        return Err("request path diverges from the pinned custody directory".into());
    }
    verify_current_path_binding(custody)?;

    let file = File::from(
        rustix::fs::openat(
            &custody.directory,
            REQUEST_FILE_NAME,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| format!("open regular non-symlink owner attestation request: {error}"))?,
    );
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
    if request_identity.uid != custody.identity.uid || request_identity.gid != custody.identity.gid
    {
        return Err("request owner and group must match its custody directory".into());
    }
    if request_identity.len == 0 || request_identity.len > MAX_REQUEST_BYTES {
        return Err(format!(
            "request size must be between 1 and {MAX_REQUEST_BYTES} bytes"
        ));
    }

    let raw = read_exact_file(
        file,
        request_identity.len,
        "owner attestation request",
        false,
    )?;
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
    buzz_sdk_pkg::nip_oa::validate_conditions(&request.conditions)
        .map_err(|error| format!("invalid owner attestation conditions: {error}"))?;
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
    let expected_preimage = format!(
        "nostr:agent-auth:{}:{}",
        request.agent_pubkey, request.conditions
    );
    if request.signing_preimage.as_bytes() != expected_preimage.as_bytes() {
        return Err(
            "signing_preimage does not byte-exactly bind agent_pubkey and conditions".into(),
        );
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

    let target_path = custody.path.join(TARGET_FILE_NAME);
    ensure_target_absent(custody)?;

    Ok(ValidatedRequest {
        request,
        request_path: request_path.to_path_buf(),
        target_path,
        request_sha256,
        request_identity,
        parent_identity: custody.identity,
        agent_pubkey,
    })
}

fn validate_normal_absolute_path(path: &Path, expected_name: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("path must be absolute".into());
    }
    if path.to_str().is_none() {
        return Err(
            "path must be valid UTF-8 so native confirmation can display it exactly".into(),
        );
    }
    if path.file_name().and_then(|value| value.to_str()) != Some(expected_name) {
        return Err(format!("path must end in {expected_name}"));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err("path must not contain '.' or '..' components".into());
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_target_absent(custody: &PinnedDirectory) -> Result<(), String> {
    match rustix::fs::statat(
        &custody.directory,
        TARGET_FILE_NAME,
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Ok(_) => Err(
            "BUZZ_AUTH_TAG target already exists or is a symlink; refusing to replace it".into(),
        ),
        Err(Errno::NOENT) => Ok(()),
        Err(error) => Err(format!("inspect BUZZ_AUTH_TAG target: {error}")),
    }
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
trait AtomicFileOps {
    fn after_temp_sync(&self, _custody: &PinnedDirectory, _temp_name: &str) -> Result<(), Errno> {
        Ok(())
    }

    fn link_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno>;
    fn unlink_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno>;
    fn sync_directory(&self, custody: &PinnedDirectory) -> Result<(), Errno>;
}

#[cfg(unix)]
struct RealAtomicFileOps;

#[cfg(unix)]
impl AtomicFileOps for RealAtomicFileOps {
    fn link_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        rustix::fs::linkat(
            &custody.directory,
            temp_name,
            &custody.directory,
            TARGET_FILE_NAME,
            AtFlags::empty(),
        )
    }

    fn unlink_temp(&self, custody: &PinnedDirectory, temp_name: &str) -> Result<(), Errno> {
        rustix::fs::unlinkat(&custody.directory, temp_name, AtFlags::empty())
    }

    fn sync_directory(&self, custody: &PinnedDirectory) -> Result<(), Errno> {
        rustix::fs::fsync(&custody.directory)
    }
}

#[cfg(unix)]
fn atomic_create_secret(custody: &PinnedDirectory, bytes: &[u8]) -> Result<(), String> {
    atomic_create_secret_with_ops(custody, bytes, &RealAtomicFileOps)
}

#[cfg(unix)]
fn atomic_create_secret_with_ops<O: AtomicFileOps>(
    custody: &PinnedDirectory,
    bytes: &[u8],
    ops: &O,
) -> Result<(), String> {
    verify_current_path_binding(custody)?;
    ensure_target_absent(custody)?;

    let temp_name = format!(".{TARGET_FILE_NAME}.{}.tmp", uuid::Uuid::new_v4());
    let mut state = TempLinkState::NotCreated;
    let operation = (|| {
        let mut temp = File::from(
            rustix::fs::openat(
                &custody.directory,
                temp_name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|error| format!("create protected temporary attestation file: {error}"))?,
        );
        state = TempLinkState::Preparing;

        let temp_identity =
            FileIdentity::from_metadata(&temp.metadata().map_err(|error| {
                format!("inspect protected temporary attestation file: {error}")
            })?);
        if temp_identity.mode != 0o600
            || temp_identity.uid != custody.identity.uid
            || temp_identity.gid != custody.identity.gid
            || temp_identity.links != 1
        {
            return Err(
                "protected temporary attestation file failed owner/mode/link checks".into(),
            );
        }

        temp.write_all(bytes)
            .map_err(|error| format!("write protected temporary attestation file: {error}"))?;
        temp.sync_all()
            .map_err(|error| format!("sync protected temporary attestation file: {error}"))?;
        ops.after_temp_sync(custody, &temp_name)
            .map_err(|error| format!("post-sync temporary-file check failed: {error}"))?;

        let persisted_file = open_relative_file(custody, &temp_name, "temporary attestation file")?;
        let persisted_identity =
            FileIdentity::from_metadata(&persisted_file.metadata().map_err(|error| {
                format!("reinspect protected temporary attestation file: {error}")
            })?);
        if !persisted_identity.same_stable_file_identity(&temp_identity) {
            return Err(
                "protected temporary attestation file identity changed before commit".into(),
            );
        }
        if persisted_identity.len != bytes.len() as u64 {
            return Err(
                "protected temporary attestation file length mismatched before commit".into(),
            );
        }
        let persisted = read_exact_file(
            persisted_file,
            bytes.len() as u64,
            "temporary attestation file",
            true,
        )?;
        if persisted.as_slice() != bytes {
            return Err("protected temporary attestation file reread mismatch".into());
        }

        verify_current_path_binding(custody)?;
        ensure_target_absent(custody)?;
        ops.link_temp(custody, &temp_name).map_err(|error| {
            if error == Errno::EXIST {
                "BUZZ_AUTH_TAG target appeared before commit; refusing to replace it".to_string()
            } else {
                format!("atomically commit BUZZ_AUTH_TAG without replacement: {error}")
            }
        })?;
        state = TempLinkState::Linked;

        if let Err(error) = ops.unlink_temp(custody, &temp_name) {
            state = TempLinkState::CleanupAmbiguous;
            return Err(format!(
                "BUZZ_AUTH_TAG was linked but temporary-link cleanup failed: {error}; STOP and do not retry"
            ));
        }
        state = TempLinkState::TempUnlinked;
        drop(temp);

        ops.sync_directory(custody).map_err(|error| {
            format!(
                "BUZZ_AUTH_TAG was committed but custody-directory sync failed: {error}; STOP and do not retry"
            )
        })?;
        verify_current_path_binding(custody).map_err(|error| {
            format!(
                "BUZZ_AUTH_TAG was committed but custody path verification failed: {error}; STOP and do not retry"
            )
        })?;

        let target = open_relative_file(custody, TARGET_FILE_NAME, "BUZZ_AUTH_TAG")?;
        let target_metadata = target.metadata().map_err(|error| {
            format!(
                "BUZZ_AUTH_TAG was committed but metadata verification failed: {error}; STOP and do not retry"
            )
        })?;
        let target_identity = FileIdentity::from_metadata(&target_metadata);
        if !target_metadata.is_file()
            || target_metadata.file_type().is_symlink()
            || target_identity.mode != 0o600
            || target_identity.uid != custody.identity.uid
            || target_identity.gid != custody.identity.gid
            || target_identity.links != 1
        {
            return Err(
                "BUZZ_AUTH_TAG was committed but failed regular-file/owner/mode/link verification; STOP and do not retry"
                    .into(),
            );
        }
        let target_bytes = read_exact_file(target, bytes.len() as u64, "BUZZ_AUTH_TAG", true)?;
        if target_bytes.as_slice() != bytes {
            return Err(
                "BUZZ_AUTH_TAG was committed but reread mismatched; STOP and do not retry".into(),
            );
        }
        state = TempLinkState::Committed;
        Ok(())
    })();

    if state == TempLinkState::Preparing {
        if let Err(operation_error) = &operation {
            if let Err(cleanup_error) = ops.unlink_temp(custody, &temp_name) {
                return Err(format!(
                    "{operation_error}; pre-commit temporary-file cleanup failed: {cleanup_error}; STOP and do not retry"
                ));
            }
            state = TempLinkState::NotCreated;
        }
    }

    debug_assert!(matches!(
        state,
        TempLinkState::NotCreated
            | TempLinkState::CleanupAmbiguous
            | TempLinkState::TempUnlinked
            | TempLinkState::Committed
    ));
    operation
}

#[cfg(unix)]
fn open_relative_file(custody: &PinnedDirectory, name: &str, label: &str) -> Result<File, String> {
    rustix::fs::openat(
        &custody.directory,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|error| format!("open protected {label}: {error}; STOP and do not retry"))
}

#[cfg(unix)]
fn read_exact_file(
    mut file: File,
    expected_len: u64,
    label: &str,
    stop_on_error: bool,
) -> Result<Vec<u8>, String> {
    let stop = if stop_on_error {
        "; STOP and do not retry"
    } else {
        ""
    };
    let expected_len = usize::try_from(expected_len)
        .map_err(|_| format!("{label} length does not fit memory limits{stop}"))?;
    let mut bytes = vec![0; expected_len];
    file.read_exact(&mut bytes)
        .map_err(|error| format!("read {label} at its inspected length: {error}{stop}"))?;
    let mut extra = [0_u8; 1];
    match file.read(&mut extra) {
        Ok(0) => Ok(bytes),
        Ok(_) => Err(format!("{label} grew while it was read{stop}")),
        Err(error) => Err(format!("check {label} length after read: {error}{stop}")),
    }
}

#[cfg(all(test, unix))]
#[path = "owner_attestation_tests.rs"]
mod tests;
