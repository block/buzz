//! Owner-reviewed local control transport for Buzz Desktop.
//!
//! The v1 surface is deliberately narrow: a same-user Unix socket may enqueue
//! a validated team snapshot for the existing Desktop preview/confirm flow.
//! It never returns identity material and never applies a mutation itself.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Mutex,
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

const API_VERSION: u32 = 1;
#[cfg(unix)]
const SOCKET_FILE_NAME: &str = "desktop-control-v1.sock";
const IDEMPOTENCY_FILE_NAME: &str = "desktop-control-idempotency.json";
const PENDING_FILE_NAME: &str = "desktop-control-pending.json";
const REQUEST_AVAILABLE_EVENT: &str = "desktop-control-request-available";
#[cfg(unix)]
const MAX_REQUEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TEAM_SNAPSHOT_BYTES: usize = 5 * 1024 * 1024;
const MAX_ACCEPTED_IDEMPOTENCY_KEYS: usize = 128;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDesktopControlImport {
    pub request_id: String,
    pub file_name: String,
    pub file_bytes: Vec<u8>,
}

#[derive(Default)]
struct PendingState {
    pending: Option<PendingDesktopControlImport>,
    delivered: bool,
    applied_idempotency_keys: VecDeque<String>,
}

#[derive(Default)]
pub struct DesktopControlState {
    inner: Mutex<PendingState>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
enum DesktopControlRequest {
    Status {
        #[serde(rename = "apiVersion")]
        api_version: u32,
    },
    ImportTeamDraft {
        #[serde(rename = "apiVersion")]
        api_version: u32,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: String,
        #[serde(rename = "fileName")]
        file_name: String,
        #[serde(rename = "fileBase64")]
        file_base64: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopControlResponse {
    ok: bool,
    api_version: u32,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    capabilities: Vec<&'static str>,
}

impl DesktopControlResponse {
    fn status() -> Self {
        Self {
            ok: true,
            api_version: API_VERSION,
            state: "ready",
            request_id: None,
            message: None,
            capabilities: vec!["agents.import-team-draft"],
        }
    }

    fn error(state: &'static str, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            api_version: API_VERSION,
            state,
            request_id: None,
            message: Some(message.into()),
            capabilities: Vec::new(),
        }
    }
}

fn validate_api_version(api_version: u32) -> Result<(), String> {
    if api_version != API_VERSION {
        return Err(format!(
            "unsupported Desktop control API version {api_version} (expected {API_VERSION})"
        ));
    }
    Ok(())
}

fn validate_idempotency_key(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 200 || value.chars().any(char::is_control) {
        return Err("idempotencyKey must contain 1-200 non-control characters".to_string());
    }
    Ok(value.to_string())
}

fn validate_file_name(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 240
        || value.chars().any(char::is_control)
        || Path::new(value).file_name().and_then(|name| name.to_str()) != Some(value)
        || !value.ends_with(".team.json")
    {
        return Err("fileName must be a basename ending in .team.json".to_string());
    }
    Ok(value.to_string())
}

fn decode_file_base64(value: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
    let bytes = STANDARD
        .decode(value)
        .map_err(|error| format!("fileBase64 is invalid: {error}"))?;
    if bytes.len() > max_bytes {
        return Err(format!(
            "decoded team snapshot exceeds the {} byte limit",
            max_bytes
        ));
    }
    Ok(bytes)
}

fn enqueue_team_import(
    app: &AppHandle,
    state: &DesktopControlState,
    idempotency_key: String,
    file_name: String,
    file_bytes: Vec<u8>,
) -> DesktopControlResponse {
    if let Err(error) = crate::commands::decode_team_snapshot_from_bytes(&file_bytes) {
        return DesktopControlResponse::error("invalid_request", error);
    }

    let mut inner = match state.inner.lock() {
        Ok(inner) => inner,
        Err(error) => {
            return DesktopControlResponse::error(
                "internal_error",
                format!("Desktop control state is unavailable: {error}"),
            )
        }
    };
    if inner
        .applied_idempotency_keys
        .iter()
        .any(|key| key == &idempotency_key)
    {
        return DesktopControlResponse {
            ok: true,
            api_version: API_VERSION,
            state: "already_applied",
            request_id: Some(idempotency_key),
            message: None,
            capabilities: Vec::new(),
        };
    }
    if let Some(pending) = &inner.pending {
        if pending.request_id == idempotency_key {
            inner.delivered = false;
            drop(inner);
            let _ = app.emit(REQUEST_AVAILABLE_EVENT, ());
            return DesktopControlResponse {
                ok: true,
                api_version: API_VERSION,
                state: "already_queued",
                request_id: Some(idempotency_key),
                message: None,
                capabilities: Vec::new(),
            };
        }
        return DesktopControlResponse::error(
            "conflict",
            "another Desktop control import is waiting for owner review",
        );
    }

    let pending = PendingDesktopControlImport {
        request_id: idempotency_key.clone(),
        file_name,
        file_bytes,
    };
    let encoded = match serde_json::to_vec(&pending) {
        Ok(value) => value,
        Err(error) => {
            return DesktopControlResponse::error(
                "internal_error",
                format!("could not encode pending Desktop control import: {error}"),
            )
        }
    };
    let pending_path = match app.path().app_data_dir() {
        Ok(path) => path.join(PENDING_FILE_NAME),
        Err(error) => {
            return DesktopControlResponse::error(
                "internal_error",
                format!("could not resolve Desktop app-data directory: {error}"),
            )
        }
    };
    if let Err(error) =
        crate::managed_agents::storage::atomic_write_json_restricted(&pending_path, &encoded)
    {
        return DesktopControlResponse::error(
            "internal_error",
            format!("could not persist pending Desktop control import: {error}"),
        );
    }

    inner.pending = Some(pending);
    inner.delivered = false;
    drop(inner);

    if let Err(error) = app.emit(REQUEST_AVAILABLE_EVENT, ()) {
        // The frontend also drains retained requests when it mounts, so an
        // event-delivery failure does not make this accepted request uncertain.
        eprintln!("buzz-desktop: desktop-control notification failed: {error}");
    }
    DesktopControlResponse {
        ok: true,
        api_version: API_VERSION,
        state: "queued_for_owner_review",
        request_id: Some(idempotency_key),
        message: Some(
            "Desktop queued the team import preview; no agents exist until the owner confirms it"
                .to_string(),
        ),
        capabilities: Vec::new(),
    }
}

fn handle_request(
    app: &AppHandle,
    state: &DesktopControlState,
    request: DesktopControlRequest,
) -> DesktopControlResponse {
    match request {
        DesktopControlRequest::Status { api_version } => {
            if let Err(error) = validate_api_version(api_version) {
                DesktopControlResponse::error("unsupported_version", error)
            } else {
                DesktopControlResponse::status()
            }
        }
        DesktopControlRequest::ImportTeamDraft {
            api_version,
            idempotency_key,
            file_name,
            file_base64,
        } => {
            if let Err(error) = validate_api_version(api_version) {
                return DesktopControlResponse::error("unsupported_version", error);
            }
            let idempotency_key = match validate_idempotency_key(&idempotency_key) {
                Ok(value) => value,
                Err(error) => return DesktopControlResponse::error("invalid_request", error),
            };
            let file_name = match validate_file_name(&file_name) {
                Ok(value) => value,
                Err(error) => return DesktopControlResponse::error("invalid_request", error),
            };
            let file_bytes = match decode_file_base64(&file_base64, MAX_TEAM_SNAPSHOT_BYTES) {
                Ok(value) => value,
                Err(error) => return DesktopControlResponse::error("invalid_request", error),
            };
            enqueue_team_import(app, state, idempotency_key, file_name, file_bytes)
        }
    }
}

#[tauri::command]
pub fn take_pending_desktop_control_import(
    state: State<'_, DesktopControlState>,
) -> Result<Option<PendingDesktopControlImport>, String> {
    let mut inner = state.inner.lock().map_err(|error| error.to_string())?;
    if inner.delivered {
        return Ok(None);
    }
    let pending = inner.pending.clone();
    inner.delivered = pending.is_some();
    Ok(pending)
}

#[tauri::command]
pub fn cancel_pending_desktop_control_import(
    request_id: String,
    app: AppHandle,
    state: State<'_, DesktopControlState>,
) -> Result<bool, String> {
    let mut inner = state.inner.lock().map_err(|error| error.to_string())?;
    if inner
        .pending
        .as_ref()
        .map(|pending| pending.request_id.as_str())
        != Some(request_id.as_str())
    {
        return Ok(false);
    }
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("could not resolve Desktop app-data directory: {error}"))?
        .join(PENDING_FILE_NAME);
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("could not remove pending import: {error}")),
    }
    inner.pending = None;
    inner.delivered = false;
    Ok(true)
}

pub(crate) fn mark_import_applied(
    app: &AppHandle,
    state: &DesktopControlState,
    request_id: &str,
) -> Result<(), String> {
    let mut inner = state.inner.lock().map_err(|error| error.to_string())?;
    if inner
        .pending
        .as_ref()
        .map(|pending| pending.request_id.as_str())
        != Some(request_id)
    {
        return Err("Desktop control request is no longer pending".to_string());
    }
    let mut keys = inner.applied_idempotency_keys.clone();
    if !keys.iter().any(|key| key == request_id) {
        keys.push_back(request_id.to_string());
    }
    while keys.len() > MAX_ACCEPTED_IDEMPOTENCY_KEYS {
        keys.pop_front();
    }
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("could not resolve Desktop app-data directory: {error}"))?;
    let encoded = serde_json::to_vec(&keys)
        .map_err(|error| format!("could not encode idempotency state: {error}"))?;
    crate::managed_agents::storage::atomic_write_json_restricted(
        &app_data.join(IDEMPOTENCY_FILE_NAME),
        &encoded,
    )?;
    // The persisted idempotency marker is authoritative after restart. Update
    // memory before removing the stale pending file so a cleanup failure cannot
    // make an already-applied request queueable again in this process.
    inner.applied_idempotency_keys = keys;
    inner.pending = None;
    inner.delivered = false;
    match std::fs::remove_file(app_data.join(PENDING_FILE_NAME)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("could not remove applied pending import: {error}")),
    }
    Ok(())
}

pub(crate) fn validate_pending_import(
    state: &DesktopControlState,
    request_id: &str,
    file_bytes: &[u8],
) -> Result<(), String> {
    let inner = state.inner.lock().map_err(|error| error.to_string())?;
    let pending = inner
        .pending
        .as_ref()
        .ok_or_else(|| "Desktop control request is no longer pending".to_string())?;
    if pending.request_id != request_id || pending.file_bytes != file_bytes {
        return Err("Desktop control request does not match the pending team snapshot".to_string());
    }
    Ok(())
}

/// Start the same-user control socket only when Desktop can safely access the
/// owner identity. Requests may enqueue a preview but never apply mutations.
pub fn start_unless_recovery(app: &AppHandle, recovery_mode: bool) {
    if recovery_mode {
        return;
    }
    match start(app) {
        Ok(path) => eprintln!("buzz-desktop: local control ready at {}", path.display()),
        Err(error) => eprintln!("buzz-desktop: local control unavailable: {error}"),
    }
}

fn load_idempotency_state(app: &AppHandle, state: &DesktopControlState) -> Result<(), String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("could not resolve Desktop app-data directory: {error}"))?
        .join(IDEMPOTENCY_FILE_NAME);
    let mut keys: VecDeque<String> = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid Desktop control idempotency state: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => VecDeque::new(),
        Err(error) => {
            return Err(format!(
                "could not read Desktop control idempotency state: {error}"
            ))
        }
    };
    if keys
        .iter()
        .any(|key| validate_idempotency_key(key).is_err())
    {
        return Err("invalid idempotency key in Desktop control state".to_string());
    }
    while keys.len() > MAX_ACCEPTED_IDEMPOTENCY_KEYS {
        keys.pop_front();
    }
    let mut inner = state.inner.lock().map_err(|error| error.to_string())?;
    inner.applied_idempotency_keys = keys;
    let pending_path = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("could not resolve Desktop app-data directory: {error}"))?
        .join(PENDING_FILE_NAME);
    inner.pending = match std::fs::read(&pending_path) {
        Ok(bytes) => Some(
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid pending Desktop control import: {error}"))?,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(format!(
                "could not read pending Desktop control import: {error}"
            ))
        }
    };
    if inner.pending.as_ref().is_some_and(|pending| {
        inner
            .applied_idempotency_keys
            .iter()
            .any(|key| key == &pending.request_id)
    }) {
        let _ = std::fs::remove_file(&pending_path);
        inner.pending = None;
    }
    inner.delivered = false;
    Ok(())
}

#[cfg(unix)]
pub fn start(app: &AppHandle) -> Result<PathBuf, String> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let state = app.state::<DesktopControlState>();
    load_idempotency_state(app, &state)?;
    let socket_path = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("could not resolve Desktop app-data directory: {error}"))?
        .join(SOCKET_FILE_NAME);
    if let Ok(metadata) = std::fs::symlink_metadata(&socket_path) {
        if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
            return Err(format!(
                "refusing to replace non-socket Desktop control path {}",
                socket_path.display()
            ));
        }
        std::fs::remove_file(&socket_path)
            .map_err(|error| format!("could not remove stale Desktop control socket: {error}"))?;
    }

    let listener = std::os::unix::net::UnixListener::bind(&socket_path)
        .map_err(|error| format!("could not bind Desktop control socket: {error}"))?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not restrict Desktop control socket: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("could not configure Desktop control socket: {error}"))?;
    let listener = tokio::net::UnixListener::from_std(listener)
        .map_err(|error| format!("could not activate Desktop control socket: {error}"))?;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!("buzz-desktop: desktop-control accept failed: {error}");
                    continue;
                }
            };
            let connection_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                let mut bytes = Vec::new();
                let read_result = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    (&mut stream)
                        .take(MAX_REQUEST_BYTES + 1)
                        .read_to_end(&mut bytes),
                )
                .await;
                let response = match read_result {
                    Err(_) => DesktopControlResponse::error(
                        "transport_error",
                        "timed out reading Desktop control request",
                    ),
                    Ok(Ok(_)) if bytes.len() as u64 > MAX_REQUEST_BYTES => {
                        DesktopControlResponse::error("invalid_request", "request exceeds 8 MiB")
                    }
                    Ok(Ok(_)) => match serde_json::from_slice::<DesktopControlRequest>(&bytes) {
                        Ok(request) => {
                            let state = connection_handle.state::<DesktopControlState>();
                            handle_request(&connection_handle, &state, request)
                        }
                        Err(error) => DesktopControlResponse::error(
                            "invalid_request",
                            format!("invalid Desktop control request: {error}"),
                        ),
                    },
                    Ok(Err(error)) => DesktopControlResponse::error(
                        "transport_error",
                        format!("could not read Desktop control request: {error}"),
                    ),
                };
                if let Ok(mut encoded) = serde_json::to_vec(&response) {
                    encoded.push(b'\n');
                    let _ = stream.write_all(&encoded).await;
                    let _ = stream.shutdown().await;
                }
            });
        }
    });
    Ok(socket_path)
}

#[cfg(not(unix))]
pub fn start(_app: &AppHandle) -> Result<PathBuf, String> {
    Err("Desktop control is not yet available on this platform".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_and_non_team_names() {
        assert!(validate_file_name("../agents.team.json").is_err());
        assert!(validate_file_name("agents.json").is_err());
        assert_eq!(
            validate_file_name("ai-devops.team.json").unwrap(),
            "ai-devops.team.json"
        );
    }

    #[test]
    fn validates_idempotency_keys_without_normalizing_content() {
        assert!(validate_idempotency_key("").is_err());
        assert!(validate_idempotency_key("bad\nkey").is_err());
        assert_eq!(
            validate_idempotency_key("sha256:abc").unwrap(),
            "sha256:abc"
        );
    }

    #[test]
    fn rejects_decoded_files_over_the_limit() {
        let encoded = STANDARD.encode([0_u8; 4]);
        assert!(decode_file_base64(&encoded, 3).is_err());
        assert_eq!(decode_file_base64(&encoded, 4).unwrap(), vec![0_u8; 4]);
    }
}
