use std::{
    io::Write,
    path::{Path, PathBuf},
};

use ed25519_dalek::{
    pkcs8::{DecodePublicKey, EncodePublicKey},
    VerifyingKey,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use tauri::{AppHandle, Manager};

use crate::managed_agents::storage::atomic_write_json_restricted;

const NXTLINQ_AUDIENCE: &str = "nxtlinq-authorization-gateway";
const NXTLINQ_STRUCTURED_SCOPE: &str = "demo:structured-capabilities";
const NXTLINQ_ATTEST_PACKAGE: &str = "@nxtlinq/attest";
const NXTLINQ_ATTEST_VERSION: &str = "3.0.0";
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const POLICY_FIELDS: &[&str] = &["name", "version", "scope", "aud", "capabilities", "exp"];

mod initialization;
mod policy;

use initialization::*;
use policy::validate_policy;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NxtlinqManifestPolicyDraft {
    pub name: String,
    pub version: String,
    pub scope: Vec<String>,
    pub aud: Vec<String>,
    pub capabilities: Vec<Map<String, Value>>,
    #[serde(default)]
    pub exp: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NxtlinqManifestPreview {
    pub manifest_path: String,
    pub current_manifest: String,
    pub proposed_manifest: String,
    pub unified_diff: String,
    pub current_sha256: String,
    pub changed: bool,
    pub requires_signature: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NxtlinqManifestSignResult {
    pub cancelled: bool,
    pub signer_key_id: Option<String>,
    pub manifest_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NxtlinqAttestInitializationResult {
    pub cancelled: bool,
    pub signer_key_id: Option<String>,
    pub public_key_fingerprint: Option<String>,
    pub private_key_storage: Option<String>,
    pub trust_store_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NxtlinqAttestInitializationState {
    Missing,
    Initialized,
    WorkspacePrivateKey,
    Invalid,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NxtlinqAttestInitializationStatus {
    pub status: NxtlinqAttestInitializationState,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NxtlinqAuthorizationConfig {
    #[serde(default)]
    pub trust_store: Option<String>,
    pub receipt_root: String,
}

fn config_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("nxtlinq"))
        .map_err(|error| format!("could not locate Nxtlinq configuration storage: {error}"))
}

fn default_config(app: &AppHandle) -> Result<NxtlinqAuthorizationConfig, String> {
    Ok(NxtlinqAuthorizationConfig {
        trust_store: None,
        receipt_root: config_root(app)?.join("receipts").display().to_string(),
    })
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config_root(app)?.join("authorization.json"))
}

fn load_authorization_config(app: &AppHandle) -> Result<NxtlinqAuthorizationConfig, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return default_config(app);
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("read Nxtlinq configuration {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse Nxtlinq configuration {}: {error}", path.display()))
}

fn existing_trust_store(path: Option<String>) -> Option<String> {
    path.map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty() && Path::new(path).is_file())
}

fn validate_absolute_path(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path"));
    }
    if path
        .components()
        .any(|component| component.as_os_str().is_empty())
    {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

pub(crate) fn prepare_receipt_directory(path: &Path) -> Result<(), String> {
    validate_absolute_path(path, "receipt root")?;
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("inspect receipt root: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("receipt root must be a real directory, not a symlink".to_string());
        }
    } else {
        std::fs::create_dir_all(path)
            .map_err(|error| format!("create receipt root {}: {error}", path.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("restrict receipt root {}: {error}", path.display()))?;
    }
    Ok(())
}

fn validate_config(config: &NxtlinqAuthorizationConfig) -> Result<(), String> {
    let trust_store = config
        .trust_store
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or("select a trusted-signers.json file")?;
    let trust_path = Path::new(trust_store);
    validate_absolute_path(trust_path, "trust store")?;
    if !trust_path.is_file() {
        return Err(format!(
            "trust store does not exist or is not a file: {}",
            trust_path.display()
        ));
    }
    let bytes = std::fs::read(trust_path)
        .map_err(|error| format!("read trust store {}: {error}", trust_path.display()))?;
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| format!("trust store is not valid JSON: {error}"))?;
    prepare_receipt_directory(Path::new(config.receipt_root.trim()))
}

#[tauri::command]
pub fn get_nxtlinq_authorization_config(
    app: AppHandle,
) -> Result<NxtlinqAuthorizationConfig, String> {
    let mut config = load_authorization_config(&app)?;
    // A trust-store path is operator-owned external state and can disappear
    // independently of Buzz. Do not present a stale saved path as the default
    // for a newly initialized project; require the owner to select an existing
    // trust store and explicitly save that enrollment instead.
    config.trust_store = existing_trust_store(config.trust_store);
    Ok(config)
}

#[tauri::command]
pub fn set_nxtlinq_authorization_config(
    app: AppHandle,
    config: NxtlinqAuthorizationConfig,
) -> Result<NxtlinqAuthorizationConfig, String> {
    save_authorization_config(&app, config)
}

fn save_authorization_config(
    app: &AppHandle,
    mut config: NxtlinqAuthorizationConfig,
) -> Result<NxtlinqAuthorizationConfig, String> {
    config.trust_store = config
        .trust_store
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty());
    config.receipt_root = config.receipt_root.trim().to_string();
    validate_config(&config)?;
    let root = config_root(app)?;
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("create Nxtlinq configuration directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("restrict Nxtlinq configuration directory: {error}"))?;
    }
    let payload = serde_json::to_vec_pretty(&config)
        .map_err(|error| format!("serialize Nxtlinq configuration: {error}"))?;
    atomic_write_json_restricted(&config_path(app)?, &payload)?;
    Ok(config)
}

#[tauri::command]
pub async fn pick_nxtlinq_trust_store(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Nxtlinq trust store", &["json"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = receiver
        .await
        .map_err(|_| "trust-store picker closed unexpectedly".to_string())?
    else {
        return Ok(None);
    };
    path.as_path()
        .map(|path| Some(path.display().to_string()))
        .ok_or("selected trust-store path is invalid".to_string())
}

#[tauri::command]
pub async fn pick_nxtlinq_directory(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = sender.send(path);
    });
    let Some(path) = receiver
        .await
        .map_err(|_| "directory picker closed unexpectedly".to_string())?
    else {
        return Ok(None);
    };
    path.as_path()
        .map(|path| Some(path.display().to_string()))
        .ok_or("selected directory path is invalid".to_string())
}

fn manifest_path(project_root: &str) -> Result<PathBuf, String> {
    let requested = Path::new(project_root.trim());
    validate_absolute_path(requested, "project root")?;
    let project = std::fs::canonicalize(requested)
        .map_err(|error| format!("resolve project root {}: {error}", requested.display()))?;
    if !project.is_dir() {
        return Err(format!(
            "project root is not a directory: {}",
            project.display()
        ));
    }
    let nxtlinq = project.join("nxtlinq");
    let nxtlinq_metadata = std::fs::symlink_metadata(&nxtlinq).map_err(|_| {
        "run nxtlinq-attest init in the project before asking an Agent to configure its policy"
            .to_string()
    })?;
    if nxtlinq_metadata.file_type().is_symlink() || !nxtlinq_metadata.is_dir() {
        return Err("project nxtlinq directory must be a real directory".to_string());
    }
    if nxtlinq.join("private.key").exists() {
        return Err(
            "move nxtlinq/private.key to owner-controlled storage outside the Agent workspace before starting conversational setup"
                .to_string(),
        );
    }
    let manifest = nxtlinq.join("agent.manifest.json");
    let metadata = std::fs::symlink_metadata(&manifest).map_err(|_| {
        "run nxtlinq-attest init to create nxtlinq/agent.manifest.json first".to_string()
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Nxtlinq manifest must be a real file, not a symlink".to_string());
    }
    if metadata.len() as usize > MAX_MANIFEST_BYTES {
        return Err("Nxtlinq manifest is too large to review in Buzz".to_string());
    }
    Ok(manifest)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn managed_attest_cli() -> Result<(PathBuf, PathBuf), String> {
    let prefix = crate::managed_agents::buzz_managed_npm_prefix()
        .ok_or("Buzz managed npm prefix is unavailable")?;
    let canonical_prefix = std::fs::canonicalize(&prefix)
        .map_err(|error| format!("resolve Buzz managed npm prefix: {error}"))?;
    #[cfg(windows)]
    let gateway_root = prefix
        .join("node_modules")
        .join("@nxtlinq")
        .join("authorization-gateway");
    #[cfg(not(windows))]
    let gateway_root = prefix
        .join("lib")
        .join("node_modules")
        .join("@nxtlinq")
        .join("authorization-gateway");
    #[cfg(windows)]
    let direct_root = prefix.join("node_modules").join("@nxtlinq").join("attest");
    #[cfg(not(windows))]
    let direct_root = prefix
        .join("lib")
        .join("node_modules")
        .join("@nxtlinq")
        .join("attest");
    let candidates = [
        direct_root,
        gateway_root.join("node_modules/@nxtlinq/attest"),
    ];
    let mut version_error = None;
    for candidate in candidates {
        let Ok(root) = std::fs::canonicalize(&candidate) else {
            continue;
        };
        if !root.starts_with(&canonical_prefix) {
            continue;
        }
        let manifest_path = root.join("package.json");
        let Ok(bytes) = std::fs::read(&manifest_path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if manifest.get("name").and_then(Value::as_str) != Some(NXTLINQ_ATTEST_PACKAGE) {
            continue;
        }
        let version = manifest
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if version != NXTLINQ_ATTEST_VERSION {
            version_error = Some(format!(
                "Nxtlinq Attest version mismatch: expected {NXTLINQ_ATTEST_VERSION}, found {version}"
            ));
            continue;
        }
        let cli = std::fs::canonicalize(root.join("bin/nxtlinq-attest.mjs"))
            .map_err(|error| format!("resolve managed Nxtlinq Attest CLI: {error}"))?;
        if !cli.starts_with(&root) || !cli.is_file() {
            return Err("managed Nxtlinq Attest CLI is outside its reviewed package".to_string());
        }
        let node = crate::managed_agents::buzz_managed_node_bin_path()
            .ok_or("Buzz managed Node runtime is unavailable")?;
        let node = std::fs::canonicalize(&node)
            .map_err(|error| format!("resolve Buzz managed Node runtime: {error}"))?;
        if !node.is_file() {
            return Err("Buzz managed Node runtime is not installed".to_string());
        }
        return Ok((node, cli));
    }
    Err(version_error.unwrap_or_else(|| {
        "Nxtlinq Attest is missing from the reviewed Gateway installation; reinstall the Gateway"
            .to_string()
    }))
}

fn validate_signer_key_id(key_id: &str) -> Result<&str, String> {
    let key_id = key_id.trim();
    if key_id.is_empty() || key_id.len() > 128 {
        return Err("signer key ID must contain between 1 and 128 characters".to_string());
    }
    if !key_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-._:/".contains(character))
    {
        return Err(
            "signer key ID may contain only letters, numbers, dash, dot, underscore, colon, or slash"
                .to_string(),
        );
    }
    Ok(key_id)
}

fn command_error_without_paths(
    label: &str,
    output: &std::process::Output,
    hidden_paths: &[&Path],
) -> String {
    let mut detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    for path in hidden_paths {
        detail = detail.replace(&path.display().to_string(), "<owner-controlled-path>");
    }
    if detail.is_empty() {
        format!("{label} failed with status {}", output.status)
    } else {
        format!("{label} failed: {detail}")
    }
}

fn validate_external_private_key(project: &Path, selected: &Path) -> Result<PathBuf, String> {
    if !selected.is_absolute() {
        return Err("selected signing key must have an absolute path".to_string());
    }
    let metadata = std::fs::symlink_metadata(selected)
        .map_err(|error| format!("inspect selected signing key: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("selected signing key must be a real file, not a symlink".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(
                "selected signing key must not be readable or writable by group/other users"
                    .to_string(),
            );
        }
    }
    let selected = std::fs::canonicalize(selected)
        .map_err(|error| format!("resolve selected signing key: {error}"))?;
    let project = std::fs::canonicalize(project)
        .map_err(|error| format!("resolve Agent workspace: {error}"))?;
    if selected.starts_with(&project) {
        return Err("select a signing key stored outside the Agent workspace".to_string());
    }
    Ok(selected)
}

fn command_failure(label: &str, output: &std::process::Output, secret_path: &Path) -> String {
    let detail = String::from_utf8_lossy(&output.stderr)
        .trim()
        .replace(&secret_path.display().to_string(), "<selected-private-key>");
    if detail.is_empty() {
        format!("{label} failed with status {}", output.status)
    } else {
        format!("{label} failed: {detail}")
    }
}

fn assert_manifest_policy(
    manifest: &[u8],
    policy: &NxtlinqManifestPolicyDraft,
) -> Result<Map<String, Value>, String> {
    validate_policy(policy)?;
    let manifest: Map<String, Value> = serde_json::from_slice(manifest)
        .map_err(|error| format!("parse Nxtlinq manifest before signing: {error}"))?;
    let expected = [
        ("name", Value::String(policy.name.trim().to_string())),
        ("version", Value::String(policy.version.trim().to_string())),
        ("scope", serde_json::json!(policy.scope)),
        ("aud", serde_json::json!(policy.aud)),
        ("capabilities", serde_json::json!(policy.capabilities)),
    ];
    for (field, expected) in expected {
        if manifest.get(field) != Some(&expected) {
            return Err(format!(
                "Nxtlinq manifest {field} no longer matches the approved proposal; review it again"
            ));
        }
    }
    match policy.exp {
        Some(exp) if manifest.get("exp") == Some(&Value::Number(exp.into())) => {}
        Some(_) => {
            return Err(
                "Nxtlinq manifest exp no longer matches the approved proposal; review it again"
                    .to_string(),
            )
        }
        None if !manifest.contains_key("exp") => {}
        None => {
            return Err(
                "Nxtlinq manifest exp no longer matches the approved proposal; review it again"
                    .to_string(),
            )
        }
    }
    Ok(manifest)
}

fn validate_trust_store_for_signing(path: &Path, project: &Path) -> Result<PathBuf, String> {
    validate_absolute_path(path, "trust store")?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect trust store {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("trust store must be a real file, not a symlink".to_string());
    }
    let path =
        std::fs::canonicalize(path).map_err(|error| format!("resolve trust store: {error}"))?;
    let project = std::fs::canonicalize(project)
        .map_err(|error| format!("resolve Agent workspace: {error}"))?;
    if path.starts_with(project) {
        return Err("trust store must be outside the Agent workspace".to_string());
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("read trust store {}: {error}", path.display()))?;
    serde_json::from_slice::<Value>(&bytes)
        .map_err(|error| format!("trust store is not valid JSON: {error}"))?;
    Ok(path)
}

fn validate_signature_output(manifest: &Path) -> Result<(), String> {
    let signature = manifest.with_file_name("agent.manifest.sig");
    if !signature.exists() {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(&signature)
        .map_err(|error| format!("inspect Nxtlinq signature output: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Nxtlinq signature output must be a real file, not a symlink".to_string());
    }
    Ok(())
}

fn sign_manifest(
    project_root: &str,
    policy: &NxtlinqManifestPolicyDraft,
    expected_sha256: &str,
    trust_store: &str,
    selected_private_key: &Path,
) -> Result<NxtlinqManifestSignResult, String> {
    let manifest = manifest_path(project_root)?;
    let project = manifest
        .parent()
        .and_then(Path::parent)
        .ok_or("could not resolve Nxtlinq project root")?;
    let current = std::fs::read(&manifest)
        .map_err(|error| format!("read Nxtlinq manifest {}: {error}", manifest.display()))?;
    if sha256_hex(&current) != expected_sha256 {
        return Err(
            "Nxtlinq manifest changed after approval; review the refreshed diff before signing"
                .to_string(),
        );
    }
    assert_manifest_policy(&current, policy)?;

    let private_key = validate_external_private_key(project, selected_private_key)?;
    let trust_store = validate_trust_store_for_signing(Path::new(trust_store.trim()), project)?;
    validate_signature_output(&manifest)?;
    let (node, cli) = managed_attest_cli()?;

    let sign_output = std::process::Command::new(&node)
        .arg(&cli)
        .arg("sign")
        .arg("--private-key")
        .arg(&private_key)
        .current_dir(project)
        .output()
        .map_err(|error| format!("launch managed Nxtlinq Attest signer: {error}"))?;
    if !sign_output.status.success() {
        return Err(command_failure(
            "Nxtlinq manifest signing",
            &sign_output,
            &private_key,
        ));
    }
    validate_signature_output(&manifest)?;

    let signed = std::fs::read(&manifest)
        .map_err(|error| format!("read signed Nxtlinq manifest: {error}"))?;
    let signed_manifest = assert_manifest_policy(&signed, policy)?;
    let signer_key_id = signed_manifest
        .get("signerKeyId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("signed Nxtlinq manifest has no signerKeyId")?
        .to_string();

    let verify_output = std::process::Command::new(&node)
        .arg(&cli)
        .arg("verify")
        .arg("--trust-store")
        .arg(&trust_store)
        .arg("--audience")
        .arg(NXTLINQ_AUDIENCE)
        .current_dir(project)
        .output()
        .map_err(|error| format!("launch managed Nxtlinq Attest verifier: {error}"))?;
    if !verify_output.status.success() {
        return Err(command_failure(
            "Nxtlinq signature trust verification",
            &verify_output,
            &private_key,
        ));
    }

    Ok(NxtlinqManifestSignResult {
        cancelled: false,
        signer_key_id: Some(signer_key_id),
        manifest_sha256: Some(sha256_hex(&signed)),
    })
}

fn sign_manifest_for_stopped_agent(
    app: &AppHandle,
    agent_pubkey: &str,
    project_root: &str,
    policy: &NxtlinqManifestPolicyDraft,
    expected_sha256: &str,
    trust_store: &str,
    selected_private_key: &Path,
) -> Result<NxtlinqManifestSignResult, String> {
    let state = app.state::<crate::app_state::AppState>();
    let _transition_guard = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = crate::managed_agents::load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let (sync_changed, exited_pubkeys) = crate::managed_agents::sync_managed_agent_processes(
        &mut records,
        &mut runtimes,
        &crate::managed_agents::current_instance_id(app),
    );
    if sync_changed {
        crate::managed_agents::save_managed_agents(app, &records)?;
    }
    for pubkey in &exited_pubkeys {
        state.clear_agent_session_caches(pubkey);
    }

    let requested_project = std::fs::canonicalize(project_root)
        .map_err(|error| format!("resolve requested Agent workspace: {error}"))?;
    {
        let record = crate::managed_agents::find_managed_agent_mut(&mut records, agent_pubkey)?;
        if record.backend != crate::managed_agents::BackendKind::Local {
            return Err("one-click signing requires a local managed Agent".to_string());
        }
        let configured_project = record
            .working_directory
            .as_deref()
            .ok_or("the requesting Agent has no configured workspace")?;
        let configured_project = std::fs::canonicalize(configured_project)
            .map_err(|error| format!("resolve configured Agent workspace: {error}"))?;
        if configured_project != requested_project {
            return Err(
                "the signing project does not match the requesting Agent workspace".to_string(),
            );
        }
        crate::managed_agents::stop_managed_agent_process(app, record, &mut runtimes)?;
    }
    crate::managed_agents::save_managed_agents(app, &records)?;
    state.clear_agent_session_caches(agent_pubkey);

    // Keep the runtime-transition, store, and process locks until signing and
    // verification finish so no pair can be restored or started concurrently.
    sign_manifest(
        project_root,
        policy,
        expected_sha256,
        trust_store,
        selected_private_key,
    )
}

fn initialize_attest_for_stopped_agent(
    app: &AppHandle,
    agent_pubkey: &str,
    project_root: &str,
    key_id: &str,
) -> Result<NxtlinqAttestInitializationResult, String> {
    let state = app.state::<crate::app_state::AppState>();
    let _transition_guard = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = crate::managed_agents::load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;
    let (sync_changed, exited_pubkeys) = crate::managed_agents::sync_managed_agent_processes(
        &mut records,
        &mut runtimes,
        &crate::managed_agents::current_instance_id(app),
    );
    if sync_changed {
        crate::managed_agents::save_managed_agents(app, &records)?;
    }
    for pubkey in &exited_pubkeys {
        state.clear_agent_session_caches(pubkey);
    }

    let requested_project = resolve_real_project_root(project_root)?;
    {
        let record = crate::managed_agents::find_managed_agent_mut(&mut records, agent_pubkey)?;
        if record.backend != crate::managed_agents::BackendKind::Local {
            return Err("Nxtlinq initialization requires a local managed Agent".to_string());
        }
        let configured_project = record
            .working_directory
            .as_deref()
            .ok_or("the requesting Agent has no configured workspace")?;
        let configured_project = resolve_real_project_root(
            configured_project
                .to_str()
                .ok_or("the configured Agent workspace path is invalid")?,
        )
        .map_err(|_| "the configured Agent workspace is invalid".to_string())?;
        if configured_project != requested_project {
            return Err(
                "the initialization project does not match the requesting Agent workspace"
                    .to_string(),
            );
        }
        if initialization_status(project_root).status != NxtlinqAttestInitializationState::Missing {
            return Err(
                "the project is no longer uninitialized; inspect it before continuing".to_string(),
            );
        }
        crate::managed_agents::stop_managed_agent_process(app, record, &mut runtimes)?;
    }
    crate::managed_agents::save_managed_agents(app, &records)?;
    state.clear_agent_session_caches(agent_pubkey);

    // Keep the runtime-transition, store, and process locks until Attest
    // finishes writing so no pair can be restored or started concurrently.
    initialize_attest_with_generated_identity(app, project_root, key_id)
}

fn atomic_write_manifest(path: &Path, payload: &[u8]) -> Result<(), String> {
    use atomic_write_file::AtomicWriteFile;

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("inspect Nxtlinq manifest {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Nxtlinq manifest must remain a real file, not a symlink".to_string());
    }
    let resolved = std::fs::canonicalize(path)
        .map_err(|error| format!("resolve Nxtlinq manifest {}: {error}", path.display()))?;
    if resolved != path {
        return Err("Nxtlinq manifest path changed after review; review it again".to_string());
    }
    let mut file = AtomicWriteFile::open(path)
        .map_err(|error| format!("open {} for atomic write: {error}", path.display()))?;
    file.write_all(payload)
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    file.commit()
        .map_err(|error| format!("commit {}: {error}", path.display()))
}

fn proposed_manifest(
    current: &[u8],
    policy: &NxtlinqManifestPolicyDraft,
) -> Result<(String, String), String> {
    validate_policy(policy)?;
    let current_text = std::str::from_utf8(current)
        .map_err(|_| "Nxtlinq manifest must be UTF-8 JSON".to_string())?;
    let mut manifest: Map<String, Value> = serde_json::from_slice(current)
        .map_err(|error| format!("parse current Nxtlinq manifest: {error}"))?;
    for field in POLICY_FIELDS {
        manifest.remove(*field);
    }
    manifest.insert("name".into(), Value::String(policy.name.trim().to_string()));
    manifest.insert(
        "version".into(),
        Value::String(policy.version.trim().to_string()),
    );
    manifest.insert("scope".into(), serde_json::json!(policy.scope));
    manifest.insert("aud".into(), serde_json::json!(policy.aud));
    manifest.insert(
        "capabilities".into(),
        serde_json::json!(policy.capabilities),
    );
    if let Some(exp) = policy.exp {
        manifest.insert("exp".into(), Value::Number(exp.into()));
    }
    let proposed = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("serialize proposed Nxtlinq manifest: {error}"))?
        + "\n";
    Ok((current_text.to_string(), proposed))
}

fn full_unified_diff(old: &str, new: &str, path: &Path) -> String {
    if old == new {
        return String::new();
    }
    let label = path.display();
    let diff = TextDiff::from_lines(old, new);
    let mut output = format!("--- a/{label}\n+++ b/{label}\n");
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        output.push_str(&hunk.to_string());
    }
    output
}

fn preview_manifest_policy(
    project_root: &str,
    policy: &NxtlinqManifestPolicyDraft,
) -> Result<NxtlinqManifestPreview, String> {
    let path = manifest_path(project_root)?;
    let current = std::fs::read(&path)
        .map_err(|error| format!("read Nxtlinq manifest {}: {error}", path.display()))?;
    let digest = sha256_hex(&current);
    let (current_manifest, proposed_manifest) = proposed_manifest(&current, policy)?;
    let changed = current_manifest != proposed_manifest;
    Ok(NxtlinqManifestPreview {
        manifest_path: path.display().to_string(),
        unified_diff: full_unified_diff(&current_manifest, &proposed_manifest, &path),
        current_manifest,
        proposed_manifest,
        current_sha256: digest,
        changed,
        requires_signature: changed,
    })
}

fn apply_manifest_policy(
    project_root: &str,
    policy: &NxtlinqManifestPolicyDraft,
    expected_sha256: &str,
) -> Result<NxtlinqManifestPreview, String> {
    let preview = preview_manifest_policy(project_root, policy)?;
    if preview.current_sha256 != expected_sha256 {
        return Err(
            "Nxtlinq manifest changed after preview; review the refreshed diff before applying"
                .to_string(),
        );
    }
    if preview.changed {
        atomic_write_manifest(
            Path::new(&preview.manifest_path),
            preview.proposed_manifest.as_bytes(),
        )?;
    }
    preview_manifest_policy(project_root, policy)
}

#[tauri::command]
pub async fn inspect_nxtlinq_attest_initialization(
    project_root: String,
) -> Result<NxtlinqAttestInitializationStatus, String> {
    tauri::async_runtime::spawn_blocking(move || initialization_status(&project_root))
        .await
        .map_err(|error| format!("Nxtlinq initialization inspection task failed: {error}"))
}

#[tauri::command]
pub async fn initialize_nxtlinq_attest(
    app: AppHandle,
    agent_pubkey: String,
    project_root: String,
    key_id: String,
) -> Result<NxtlinqAttestInitializationResult, String> {
    if initialization_status(&project_root).status != NxtlinqAttestInitializationState::Missing {
        return Err("the project is not ready for Nxtlinq initialization".to_string());
    }
    validate_signer_key_id(&key_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        initialize_attest_for_stopped_agent(&app, &agent_pubkey, &project_root, &key_id)
    })
    .await
    .map_err(|error| format!("Nxtlinq initialization task failed: {error}"))?
}

#[tauri::command]
pub async fn preview_nxtlinq_manifest_policy(
    project_root: String,
    policy: NxtlinqManifestPolicyDraft,
) -> Result<NxtlinqManifestPreview, String> {
    tauri::async_runtime::spawn_blocking(move || preview_manifest_policy(&project_root, &policy))
        .await
        .map_err(|error| format!("Nxtlinq manifest preview task failed: {error}"))?
}

#[tauri::command]
pub async fn apply_nxtlinq_manifest_policy(
    project_root: String,
    policy: NxtlinqManifestPolicyDraft,
    expected_sha256: String,
) -> Result<NxtlinqManifestPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        apply_manifest_policy(&project_root, &policy, &expected_sha256)
    })
    .await
    .map_err(|error| format!("Nxtlinq manifest apply task failed: {error}"))?
}

#[tauri::command]
pub async fn sign_nxtlinq_manifest(
    app: AppHandle,
    agent_pubkey: String,
    project_root: String,
    policy: NxtlinqManifestPolicyDraft,
    expected_sha256: String,
    trust_store: String,
) -> Result<NxtlinqManifestSignResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let managed_key = load_managed_private_key(&app, &project_root)?;
        let temporary_key = tempfile::Builder::new()
            .prefix("nxtlinq-policy-signing-")
            .tempdir()
            .map_err(|_| "create protected signing directory".to_string())?;
        let selected = temporary_key.path().join("private.pem");
        write_new_private_file(&selected, managed_key.as_bytes())?;
        sign_manifest_for_stopped_agent(
            &app,
            &agent_pubkey,
            &project_root,
            &policy,
            &expected_sha256,
            &trust_store,
            &selected,
        )
    })
    .await
    .map_err(|error| format!("Nxtlinq manifest signing task failed: {error}"))?
}

#[cfg(test)]
mod signing_tests;

#[cfg(test)]
mod tests;
