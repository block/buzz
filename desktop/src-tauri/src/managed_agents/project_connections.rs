//! Project-owned MCP connection metadata and credentials.
//!
//! Connection metadata is partitioned by the active Buzz community and
//! identity. Secret values are never written to metadata or returned to the
//! webview after a write.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager as _};
use uuid::Uuid;

use super::{atomic_write_json_restricted, managed_agents_base_dir};
use crate::util::now_iso;
#[cfg(feature = "system-keyring")]
use crate::{app_state::keyring_service, secret_store::SecretStore};

const CONNECTION_STORE_VERSION: u32 = 1;
const MAX_CONNECTIONS: usize = 128;
const MAX_NAME_BYTES: usize = 128;
const MAX_PROVIDER_BYTES: usize = 64;
const MAX_COMMAND_BYTES: usize = 1024;
const MAX_ARGS: usize = 128;
const MAX_ARG_BYTES: usize = 4096;
const MAX_ENV_KEYS: usize = 128;
const MAX_SECRET_BYTES: usize = 64 * 1024;
const HEALTH_STALE_AFTER_SECONDS: i64 = 24 * 60 * 60;

static PROJECT_CONNECTIONS_LOCK: Mutex<()> = Mutex::new(());

mod approval;
mod credential_journal;
mod transactions;
#[cfg(test)]
use approval::executable_sha256;
use approval::{approved_execution_sha256, canonical_connection_command};
use transactions::{commit_delete, commit_update, UpdateTransaction};

pub(super) fn lock_project_connections() -> MutexGuard<'static, ()> {
    PROJECT_CONNECTIONS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConnectionScope {
    pub relay_url: String,
    pub operator_pubkey: String,
    /// Canonical NIP-MP Project coordinate (`30621:<owner>:<d-tag>`).
    ///
    /// Legacy one-repository Projects use their NIP-34 repository coordinate
    /// (`30617:<owner>:<d-tag>`).
    #[serde(alias = "repoAddress")]
    pub project_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectConnectionHealthStatus {
    Ready,
    NotTested,
    CheckNeeded,
    ApprovalRequired,
    SignInRequired,
    MissingAccess,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConnectionHealth {
    pub status: ProjectConnectionHealthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Default for ProjectConnectionHealth {
    fn default() -> Self {
        Self {
            status: ProjectConnectionHealthStatus::NotTested,
            last_verified_at: None,
            detail: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConnection {
    pub id: String,
    pub project_scope: ProjectConnectionScope,
    pub name: String,
    pub provider: String,
    pub capability_ids: Vec<String>,
    pub command: String,
    pub args: Vec<String>,
    /// Names only. Values are never returned by a Tauri command.
    pub env_keys: Vec<String>,
    pub discovered_tools: Vec<String>,
    pub health: ProjectConnectionHealth,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct StoredProjectConnection {
    id: String,
    project_scope: ProjectConnectionScope,
    name: String,
    provider: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    capability_ids: Vec<String>,
    command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    env_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    discovered_tools: Vec<String>,
    #[serde(default)]
    health: ProjectConnectionHealth,
    executable_sha256: String,
    generation: String,
    credential_generation: String,
    created_at: String,
    updated_at: String,
}

impl From<StoredProjectConnection> for ProjectConnection {
    fn from(connection: StoredProjectConnection) -> Self {
        Self {
            id: connection.id,
            project_scope: connection.project_scope,
            name: connection.name,
            provider: connection.provider,
            capability_ids: connection.capability_ids,
            command: connection.command,
            args: connection.args,
            env_keys: connection.env_keys,
            discovered_tools: connection.discovered_tools,
            health: connection.health,
            created_at: connection.created_at,
            updated_at: connection.updated_at,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectConnectionRequest {
    pub project_scope: ProjectConnectionScope,
    pub name: String,
    pub provider: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Secret environment values. They are write-only at this boundary.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub execution_acknowledged: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectConnectionRequest {
    pub id: String,
    pub project_scope: ProjectConnectionScope,
    pub name: String,
    pub provider: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Changed or added values. Omitted keys retain their saved value.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub remove_env_keys: Vec<String>,
    #[serde(default)]
    pub execution_acknowledged: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectConnectionStore {
    version: u32,
    connections: Vec<StoredProjectConnection>,
}

impl Default for ProjectConnectionStore {
    fn default() -> Self {
        Self {
            version: CONNECTION_STORE_VERSION,
            connections: Vec::new(),
        }
    }
}

fn next_generation() -> String {
    Uuid::new_v4().simple().to_string()
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn validate_stored_connection(connection: &StoredProjectConnection) -> Result<(), String> {
    if !is_lower_hex(&connection.id, 32)
        || !is_lower_hex(&connection.generation, 32)
        || !is_lower_hex(&connection.credential_generation, 32)
        || !is_lower_hex(&connection.executable_sha256, 64)
        || canonical_project_scope(&connection.project_scope)? != connection.project_scope
    {
        return Err("Project connection metadata is invalid.".to_string());
    }
    Ok(())
}

fn valid_stable_id(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

pub(super) fn canonical_project_scope(
    scope: &ProjectConnectionScope,
) -> Result<ProjectConnectionScope, String> {
    let relay_url = buzz_core_pkg::relay::normalize_relay_url(&scope.relay_url)
        .map_err(|_| "Choose a valid Buzz community before continuing.".to_string())?;
    if scope.operator_pubkey.len() != 64
        || !scope
            .operator_pubkey
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Buzz could not verify who owns these connections.".to_string());
    }
    let mut parts = scope.project_address.splitn(3, ':');
    let kind = parts.next();
    let owner = parts.next();
    let d_tag = parts.next();
    if !matches!(kind, Some("30617") | Some("30621"))
        || !owner.is_some_and(|value| {
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        || !d_tag.is_some_and(|value| {
            !value.is_empty()
                && value.len() <= 256
                && !value.chars().any(char::is_control)
                && !value.contains(':')
        })
    {
        return Err("Choose a valid Buzz Project before continuing.".to_string());
    }
    Ok(ProjectConnectionScope {
        relay_url,
        operator_pubkey: scope.operator_pubkey.to_ascii_lowercase(),
        project_address: format!(
            "{}:{}:{}",
            kind.unwrap_or_default(),
            owner.unwrap_or_default().to_ascii_lowercase(),
            d_tag.unwrap_or_default()
        ),
    })
}

fn validate_project_scope_for_app(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
) -> Result<ProjectConnectionScope, String> {
    let canonical = canonical_project_scope(scope)?;
    let state = app.state::<crate::app_state::AppState>();
    let active_relay = buzz_core_pkg::relay::normalize_relay_url(
        &crate::relay::relay_ws_url_with_override(&state),
    )
    .map_err(|_| "Buzz could not verify the active community.".to_string())?;
    if canonical.relay_url != active_relay {
        return Err("This Project belongs to another Buzz community.".to_string());
    }
    let active_operator = state
        .keys
        .lock()
        .map_err(|_| "Buzz could not verify the active identity.".to_string())?
        .public_key()
        .to_hex();
    if !canonical
        .operator_pubkey
        .eq_ignore_ascii_case(&active_operator)
    {
        return Err("These connections belong to another Buzz identity.".to_string());
    }
    Ok(canonical)
}

fn workspace_scope_id(scope: &ProjectConnectionScope) -> String {
    let mut hasher = Sha256::new();
    hasher.update(scope.operator_pubkey.as_bytes());
    hasher.update(b"\0");
    hasher.update(scope.relay_url.as_bytes());
    hex::encode(hasher.finalize())
}

fn ensure_owner_only_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("Buzz refused an unsafe Project connection directory.".to_string());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| {
                format!(
                    "failed to create Project connection directory {}: {error}",
                    path.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect Project connection directory {}: {error}",
                path.display()
            ));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!(
                "failed to protect Project connection directory {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

fn workspace_connection_dir(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
) -> Result<PathBuf, String> {
    let root = managed_agents_base_dir(app)?.join("project-connections");
    ensure_owner_only_directory(&root)?;
    let scoped = root.join(workspace_scope_id(scope));
    ensure_owner_only_directory(&scoped)?;
    Ok(scoped)
}

fn connection_store_path(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
) -> Result<PathBuf, String> {
    Ok(workspace_connection_dir(app, scope)?.join("connections.json"))
}

fn reject_unsafe_owner_file(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to inspect Project connection file {}: {error}",
                path.display()
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Buzz refused an unsafe Project connection file.".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(
                "Project connection data is not owner-only. Fix its permissions before continuing."
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn load_store_unlocked(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
) -> Result<ProjectConnectionStore, String> {
    let path = connection_store_path(app, scope)?;
    reject_unsafe_owner_file(&path)?;
    let store = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("failed to parse Project connections: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ProjectConnectionStore::default()
        }
        Err(error) => {
            return Err(format!(
                "failed to read Project connections from {}: {error}",
                path.display()
            ));
        }
    };
    if store.version != CONNECTION_STORE_VERSION {
        return Err(format!(
            "unsupported Project connection store version {}",
            store.version
        ));
    }
    if store.connections.len() > MAX_CONNECTIONS {
        return Err("Project connection store exceeds its connection limit".to_string());
    }
    for connection in &store.connections {
        validate_stored_connection(connection)?;
    }
    credential_journal::reconcile(app, scope, &store)?;
    Ok(store)
}

fn save_store_unlocked(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
    store: &ProjectConnectionStore,
) -> Result<(), String> {
    let path = connection_store_path(app, scope)?;
    reject_unsafe_owner_file(&path)?;
    let bytes = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("failed to serialize Project connections: {error}"))?;
    atomic_write_json_restricted(&path, &bytes)
}

#[cfg(feature = "system-keyring")]
fn connection_secret_key(
    scope: &ProjectConnectionScope,
    id: &str,
    credential_generation: &str,
) -> String {
    format!(
        "project-connection:{}:{id}:{credential_generation}",
        workspace_scope_id(scope)
    )
}

#[cfg(not(feature = "system-keyring"))]
fn connection_secret_path(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
    id: &str,
    credential_generation: &str,
) -> Result<PathBuf, String> {
    let dir = workspace_connection_dir(app, scope)?.join("secrets");
    ensure_owner_only_directory(&dir)?;
    let digest = Sha256::digest(format!("{id}\0{credential_generation}").as_bytes());
    Ok(dir.join(format!("{}.json", hex::encode(digest))))
}

fn serialize_secrets(env: &BTreeMap<String, String>) -> Result<Vec<u8>, String> {
    serde_json::to_vec(env)
        .map_err(|error| format!("failed to prepare connection credentials: {error}"))
}

#[cfg(any(test, feature = "system-keyring"))]
fn store_verified_secret<Write, Verify, Delete>(
    mut write: Write,
    mut verify: Verify,
    mut delete: Delete,
) -> Result<(), String>
where
    Write: FnMut() -> Result<(), String>,
    Verify: FnMut() -> Result<bool, String>,
    Delete: FnMut() -> Result<(), String>,
{
    let failure = match write() {
        Ok(()) => match verify() {
            Ok(true) => return Ok(()),
            Ok(false) | Err(_) => "Buzz could not verify the saved credentials.",
        },
        Err(_) => "Buzz could not save these credentials in the system keyring.",
    };

    if delete().is_err() {
        return Err(format!(
            "{failure} Buzz also could not remove the unverified credentials."
        ));
    }
    Err(failure.to_string())
}

fn store_secrets(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
    id: &str,
    credential_generation: &str,
    env: &BTreeMap<String, String>,
) -> Result<(), String> {
    if env.is_empty() {
        return delete_secrets(app, scope, id, credential_generation);
    }
    let serialized = serialize_secrets(env)?;
    #[cfg(feature = "system-keyring")]
    {
        let raw = String::from_utf8(serialized)
            .map_err(|_| "Buzz could not prepare these credentials.".to_string())?;
        let key = connection_secret_key(scope, id, credential_generation);
        let store = SecretStore::shared(keyring_service());
        store_verified_secret(
            || store.store(&key, &raw),
            || store.verify_stored_raw(&key, &raw),
            || store.delete(&key),
        )?;
    }
    #[cfg(not(feature = "system-keyring"))]
    {
        let path = connection_secret_path(app, scope, id, credential_generation)?;
        reject_unsafe_owner_file(&path)?;
        atomic_write_json_restricted(&path, &serialized)?;
    }
    Ok(())
}

fn load_secrets(
    app: &AppHandle,
    connection: &StoredProjectConnection,
) -> Result<BTreeMap<String, String>, String> {
    if connection.env_keys.is_empty() {
        return Ok(BTreeMap::new());
    }
    #[cfg(feature = "system-keyring")]
    let _ = app;
    #[cfg(feature = "system-keyring")]
    let raw = SecretStore::shared(keyring_service())
        .load(&connection_secret_key(
            &connection.project_scope,
            &connection.id,
            &connection.credential_generation,
        ))
        .map_err(|_| {
            format!(
                "Sign in again to '{}'. Buzz could not read its saved credentials.",
                connection.name
            )
        })?
        .ok_or_else(|| {
            format!(
                "Sign in again to '{}'. Its saved credentials are missing.",
                connection.name
            )
        })?
        .into_bytes();
    #[cfg(not(feature = "system-keyring"))]
    let raw = {
        let path = connection_secret_path(
            app,
            &connection.project_scope,
            &connection.id,
            &connection.credential_generation,
        )?;
        reject_unsafe_owner_file(&path)?;
        fs::read(path).map_err(|_| {
            format!(
                "Sign in again to '{}'. Its saved credentials are missing.",
                connection.name
            )
        })?
    };
    let env: BTreeMap<String, String> = serde_json::from_slice(&raw).map_err(|_| {
        format!(
            "Sign in again to '{}'. Its credentials are invalid.",
            connection.name
        )
    })?;
    let actual: BTreeSet<&str> = env.keys().map(String::as_str).collect();
    let expected: BTreeSet<&str> = connection.env_keys.iter().map(String::as_str).collect();
    if actual != expected {
        return Err(format!(
            "Sign in again to '{}'. Its saved credentials are incomplete.",
            connection.name
        ));
    }
    Ok(env)
}

fn delete_secrets(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
    id: &str,
    credential_generation: &str,
) -> Result<(), String> {
    #[cfg(feature = "system-keyring")]
    {
        let _ = app;
        SecretStore::shared(keyring_service()).delete(&connection_secret_key(
            scope,
            id,
            credential_generation,
        ))
    }
    #[cfg(not(feature = "system-keyring"))]
    {
        let path = connection_secret_path(app, scope, id, credential_generation)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("failed to remove saved credentials: {error}")),
        }
    }
}

fn validate_connection_input(
    name: &str,
    provider: &str,
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > MAX_NAME_BYTES {
        return Err("Give this connection a short name.".to_string());
    }
    if provider.trim().is_empty() || provider.len() > MAX_PROVIDER_BYTES {
        return Err("Name the service this connection uses.".to_string());
    }
    if command.trim().is_empty()
        || command.len() > MAX_COMMAND_BYTES
        || command.contains('\0')
        || command.contains('\n')
    {
        return Err("Enter a valid MCP server executable path.".to_string());
    }
    if args.len() > MAX_ARGS
        || args
            .iter()
            .any(|arg| arg.len() > MAX_ARG_BYTES || arg.contains('\0'))
    {
        return Err("The MCP server arguments exceed Buzz's safety limits.".to_string());
    }
    if env.len() > MAX_ENV_KEYS {
        return Err("This connection has too many secret values.".to_string());
    }
    let mut total = 0usize;
    for (key, value) in env {
        if !super::is_well_formed_env_key(key) || super::is_reserved_env_key(key) {
            return Err(format!(
                "'{key}' cannot be used as a connection secret name."
            ));
        }
        if value.is_empty() {
            return Err(format!("Enter a value for '{key}' or remove it."));
        }
        if value.contains('\0') {
            return Err(format!(
                "The value for '{key}' contains an invalid character."
            ));
        }
        total = total.saturating_add(key.len()).saturating_add(value.len());
    }
    if total > MAX_SECRET_BYTES {
        return Err("The connection secret values exceed Buzz's size limit.".to_string());
    }
    Ok(())
}

fn health_for_display(mut connection: StoredProjectConnection) -> StoredProjectConnection {
    if connection.health.status == ProjectConnectionHealthStatus::Ready {
        let stale = connection
            .health
            .last_verified_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .is_none_or(|verified| {
                chrono::Utc::now().signed_duration_since(verified.with_timezone(&chrono::Utc))
                    > chrono::Duration::seconds(HEALTH_STALE_AFTER_SECONDS)
            });
        if stale {
            connection.health.status = ProjectConnectionHealthStatus::CheckNeeded;
        }
    }
    connection
}

fn find_connection<'a>(
    store: &'a ProjectConnectionStore,
    project_scope: &ProjectConnectionScope,
    connection_id: &str,
) -> Result<&'a StoredProjectConnection, String> {
    find_connection_index(store, project_scope, connection_id)
        .map(|index| &store.connections[index])
        .ok_or_else(|| "This connection no longer exists in this Project.".to_string())
}

fn find_connection_index(
    store: &ProjectConnectionStore,
    project_scope: &ProjectConnectionScope,
    connection_id: &str,
) -> Option<usize> {
    store.connections.iter().position(|connection| {
        connection.id == connection_id && connection.project_scope == *project_scope
    })
}

fn project_connection_count(
    store: &ProjectConnectionStore,
    project_scope: &ProjectConnectionScope,
) -> usize {
    store
        .connections
        .iter()
        .filter(|connection| connection.project_scope == *project_scope)
        .count()
}

pub fn list_project_connections(
    app: &AppHandle,
    project_scope: &ProjectConnectionScope,
) -> Result<Vec<ProjectConnection>, String> {
    let project_scope = validate_project_scope_for_app(app, project_scope)?;
    let _guard = lock_project_connections();
    let mut connections: Vec<_> = load_store_unlocked(app, &project_scope)?
        .connections
        .into_iter()
        .filter(|connection| connection.project_scope == project_scope)
        .map(health_for_display)
        .map(ProjectConnection::from)
        .collect();
    connections.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(connections)
}

pub fn create_project_connection(
    app: &AppHandle,
    mut input: CreateProjectConnectionRequest,
) -> Result<ProjectConnection, String> {
    input.project_scope = validate_project_scope_for_app(app, &input.project_scope)?;
    validate_connection_input(
        &input.name,
        &input.provider,
        &input.command,
        &input.args,
        &input.env,
    )?;
    if !input.execution_acknowledged {
        return Err("Review and acknowledge this local program before saving.".to_string());
    }
    let (command, executable_sha256) = canonical_connection_command(&input.command)?;
    let executable_sha256 = approved_execution_sha256(&executable_sha256, &input.args)?;
    let _guard = lock_project_connections();
    let mut store = load_store_unlocked(app, &input.project_scope)?;
    if project_connection_count(&store, &input.project_scope) >= MAX_CONNECTIONS {
        return Err("Buzz has reached the Project connection limit.".to_string());
    }
    let id = Uuid::new_v4().simple().to_string();
    let now = now_iso();
    let credential_generation = next_generation();
    let connection = StoredProjectConnection {
        id: id.clone(),
        project_scope: input.project_scope.clone(),
        name: input.name.trim().to_string(),
        provider: input.provider.trim().to_string(),
        capability_ids: Vec::new(),
        command,
        args: input.args,
        env_keys: input.env.keys().cloned().collect(),
        discovered_tools: Vec::new(),
        health: ProjectConnectionHealth::default(),
        executable_sha256,
        generation: next_generation(),
        credential_generation: credential_generation.clone(),
        created_at: now.clone(),
        updated_at: now,
    };
    if !input.env.is_empty() {
        credential_journal::begin(
            app,
            &input.project_scope,
            &id,
            vec![credential_generation.clone()],
        )?;
        store_secrets(
            app,
            &input.project_scope,
            &id,
            &credential_generation,
            &input.env,
        )?;
    }
    store.connections.push(connection.clone());
    if let Err(error) = save_store_unlocked(app, &input.project_scope, &store) {
        if !input.env.is_empty() {
            if let Err(cleanup_error) =
                delete_secrets(app, &input.project_scope, &id, &credential_generation)
            {
                return Err(format!(
                    "{error} Buzz also could not remove the unreferenced credentials: {cleanup_error}"
                ));
            }
        }
        return Err(error);
    }
    if !input.env.is_empty() {
        if let Err(error) = credential_journal::complete(app, &input.project_scope) {
            eprintln!(
                "buzz-desktop: Project connection credential recovery marker remains after create: {error}"
            );
        }
    }
    Ok(connection.into())
}

pub fn update_project_connection(
    app: &AppHandle,
    mut input: UpdateProjectConnectionRequest,
) -> Result<ProjectConnection, String> {
    input.project_scope = validate_project_scope_for_app(app, &input.project_scope)?;
    for key in &input.remove_env_keys {
        if !super::is_well_formed_env_key(key) {
            return Err("A secret name is invalid.".to_string());
        }
    }
    let (command, executable_sha256) = canonical_connection_command(&input.command)?;
    let executable_sha256 = approved_execution_sha256(&executable_sha256, &input.args)?;
    probe::with_project_connection_probe_excluded(|| {
        let _guard = lock_project_connections();
        let mut store = load_store_unlocked(app, &input.project_scope)?;
        let previous = find_connection(&store, &input.project_scope, &input.id)?.clone();
        let previous_secrets = load_secrets(app, &previous)?;
        let mut next_secrets = previous_secrets.clone();
        for key in &input.remove_env_keys {
            next_secrets.remove(key);
        }
        next_secrets.extend(input.env);
        validate_connection_input(
            &input.name,
            &input.provider,
            &command,
            &input.args,
            &next_secrets,
        )?;
        let execution_changed = previous.command != command
            || previous.executable_sha256 != executable_sha256
            || previous.args != input.args
            || previous_secrets != next_secrets;
        if execution_changed && !input.execution_acknowledged {
            return Err(
                "Review and acknowledge the changed program, arguments, and credentials before saving."
                    .to_string(),
            );
        }
        let index = find_connection_index(&store, &input.project_scope, &input.id)
            .ok_or_else(|| "This connection no longer exists.".to_string())?;
        let mut updated = previous.clone();
        updated.name = input.name.trim().to_string();
        updated.provider = input.provider.trim().to_string();
        updated.command = command;
        updated.executable_sha256 = executable_sha256;
        updated.args = input.args;
        updated.env_keys = next_secrets.keys().cloned().collect();
        updated.generation = next_generation();
        updated.updated_at = now_iso();
        if execution_changed {
            updated.capability_ids.clear();
            updated.discovered_tools.clear();
            updated.health = ProjectConnectionHealth::default();
        }

        let secrets_changed = previous_secrets != next_secrets;
        if secrets_changed {
            updated.credential_generation = next_generation();
            credential_journal::begin(
                app,
                &input.project_scope,
                &input.id,
                vec![
                    previous.credential_generation.clone(),
                    updated.credential_generation.clone(),
                ],
            )?;
        }
        let result = commit_update(
            &mut store,
            UpdateTransaction {
                index,
                previous: &previous,
                updated: &updated,
                secrets_changed,
            },
            || {
                store_secrets(
                    app,
                    &input.project_scope,
                    &input.id,
                    &updated.credential_generation,
                    &next_secrets,
                )
            },
            |candidate| save_store_unlocked(app, &input.project_scope, candidate),
            |generation| delete_secrets(app, &input.project_scope, &input.id, generation),
        );
        if result.is_ok() && secrets_changed {
            if let Err(error) = credential_journal::complete(app, &input.project_scope) {
                eprintln!(
                    "buzz-desktop: Project connection credential recovery marker remains after update: {error}"
                );
            }
        }
        result?;
        Ok(updated.into())
    })
}

pub fn delete_project_connection(
    app: &AppHandle,
    project_scope: &ProjectConnectionScope,
    connection_id: &str,
) -> Result<(), String> {
    let project_scope = validate_project_scope_for_app(app, project_scope)?;
    probe::with_project_connection_probe_excluded(|| {
        let _guard = lock_project_connections();
        let mut store = load_store_unlocked(app, &project_scope)?;
        let index = find_connection_index(&store, &project_scope, connection_id)
            .ok_or_else(|| "This connection no longer exists in this Project.".to_string())?;
        let removed = store.connections[index].clone();
        if !removed.env_keys.is_empty() {
            credential_journal::begin(
                app,
                &project_scope,
                connection_id,
                vec![removed.credential_generation.clone()],
            )?;
        }
        let result = commit_delete(
            &mut store,
            index,
            |candidate| save_store_unlocked(app, &project_scope, candidate),
            |generation| delete_secrets(app, &project_scope, connection_id, generation),
        );
        if result.is_ok() && !removed.env_keys.is_empty() {
            if let Err(error) = credential_journal::complete(app, &project_scope) {
                eprintln!(
                    "buzz-desktop: Project connection credential recovery marker remains after delete: {error}"
                );
            }
        }
        result
    })
}

mod probe;
pub use probe::test_project_connection;

#[cfg(test)]
mod tests;
