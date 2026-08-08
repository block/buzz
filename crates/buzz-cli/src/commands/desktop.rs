//! Local, owner-reviewed Buzz Desktop control commands.

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{error::CliError, DesktopAgentsCmd, DesktopCmd};

const API_VERSION: u32 = 1;
const PROD_BUNDLE_IDENTIFIER: &str = "xyz.block.buzz.app";
const SOCKET_FILE_NAME: &str = "desktop-control-v1.sock";
const MAX_TEAM_SNAPSHOT_BYTES: u64 = 5 * 1024 * 1024;
#[cfg(unix)]
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

#[derive(Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
enum DesktopControlRequest<'a> {
    Status {
        #[serde(rename = "apiVersion")]
        api_version: u32,
    },
    ImportTeamDraft {
        #[serde(rename = "apiVersion")]
        api_version: u32,
        #[serde(rename = "idempotencyKey")]
        idempotency_key: &'a str,
        #[serde(rename = "fileName")]
        file_name: &'a str,
        #[serde(rename = "fileBase64")]
        file_base64: &'a str,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DesktopControlResponse {
    ok: bool,
    api_version: u32,
    state: String,
    #[serde(default)]
    message: Option<String>,
}

pub async fn dispatch(command: &DesktopCmd) -> Result<(), CliError> {
    match command {
        DesktopCmd::Status { socket } => {
            let response = send(
                &resolve_socket_path(socket.as_deref())?,
                &DesktopControlRequest::Status {
                    api_version: API_VERSION,
                },
            )
            .await?;
            print_response(response)
        }
        DesktopCmd::Agents(command) => match command {
            DesktopAgentsCmd::ImportTeamDraft {
                path,
                socket,
                idempotency_key,
            } => {
                let file_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| CliError::Usage("snapshot path has no UTF-8 basename".into()))?;
                if !file_name.ends_with(".team.json") {
                    return Err(CliError::Usage(
                        "team snapshot filename must end in .team.json".into(),
                    ));
                }
                let metadata = std::fs::metadata(path).map_err(|error| {
                    CliError::Other(format!("could not read {}: {error}", path.display()))
                })?;
                if metadata.len() > MAX_TEAM_SNAPSHOT_BYTES {
                    return Err(CliError::Usage(
                        "team snapshot exceeds the 5 MiB local-control limit".into(),
                    ));
                }
                let file_bytes = std::fs::read(path).map_err(|error| {
                    CliError::Other(format!("could not read {}: {error}", path.display()))
                })?;
                let generated_key = format!("sha256:{}", hex::encode(Sha256::digest(&file_bytes)));
                let idempotency_key = idempotency_key.as_deref().unwrap_or(&generated_key);
                let file_base64 = STANDARD.encode(&file_bytes);
                let response = send(
                    &resolve_socket_path(socket.as_deref())?,
                    &DesktopControlRequest::ImportTeamDraft {
                        api_version: API_VERSION,
                        idempotency_key,
                        file_name,
                        file_base64: &file_base64,
                    },
                )
                .await?;
                print_response(response)
            }
        },
    }
}

fn resolve_socket_path(override_path: Option<&Path>) -> Result<PathBuf, CliError> {
    if let Some(path) = override_path {
        return Ok(path.to_path_buf());
    }
    let data_dir = dirs::data_dir().ok_or_else(|| {
        CliError::Other("could not resolve platform app-data directory".to_string())
    })?;
    Ok(data_dir.join(PROD_BUNDLE_IDENTIFIER).join(SOCKET_FILE_NAME))
}

fn print_response(raw: String) -> Result<(), CliError> {
    let parsed: DesktopControlResponse = serde_json::from_str(&raw)
        .map_err(|error| CliError::Other(format!("invalid Desktop control response: {error}")))?;
    if parsed.api_version != API_VERSION {
        return Err(CliError::Other(format!(
            "unsupported Desktop control response version {} (expected {API_VERSION})",
            parsed.api_version
        )));
    }
    if !parsed.ok {
        let message = parsed.message.unwrap_or(parsed.state);
        return Err(CliError::Other(format!(
            "Desktop control rejected request: {message}"
        )));
    }
    println!("{raw}");
    Ok(())
}

#[cfg(unix)]
async fn send(path: &Path, request: &DesktopControlRequest<'_>) -> Result<String, CliError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        send_within_deadline(path, request),
    )
    .await
    .map_err(|_| CliError::Other("Desktop control request timed out".to_string()))?
}

#[cfg(unix)]
async fn send_within_deadline(
    path: &Path,
    request: &DesktopControlRequest<'_>,
) -> Result<String, CliError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|error| {
            CliError::Other(format!(
                "could not connect to Buzz Desktop at {}: {error}",
                path.display()
            ))
        })?;
    let encoded = serde_json::to_vec(request)
        .map_err(|error| CliError::Other(format!("could not encode request: {error}")))?;
    stream
        .write_all(&encoded)
        .await
        .map_err(|error| CliError::Other(format!("could not send request: {error}")))?;
    stream
        .shutdown()
        .await
        .map_err(|error| CliError::Other(format!("could not finish request: {error}")))?;
    let mut response = String::new();
    let response_bytes = (&mut stream)
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_string(&mut response)
        .await
        .map_err(|error| CliError::Other(format!("could not read response: {error}")))?;
    if response_bytes as u64 > MAX_RESPONSE_BYTES {
        return Err(CliError::Other(
            "Desktop control response exceeds 64 KiB".to_string(),
        ));
    }
    Ok(response.trim().to_string())
}

#[cfg(not(unix))]
async fn send(_path: &Path, _request: &DesktopControlRequest<'_>) -> Result<String, CliError> {
    Err(CliError::Usage(
        "Buzz Desktop local control is not yet available on this platform".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_override_wins() {
        let path = resolve_socket_path(Some(Path::new("/custom/buzz.sock"))).unwrap();
        assert_eq!(path, PathBuf::from("/custom/buzz.sock"));
    }

    #[test]
    fn default_socket_uses_production_bundle_data_dir() {
        let path = resolve_socket_path(None).unwrap();
        assert!(path.ends_with("xyz.block.buzz.app/desktop-control-v1.sock"));
    }

    #[test]
    fn rejects_response_from_an_unsupported_api_version() {
        let response = r#"{"ok":true,"apiVersion":2,"state":"ready"}"#.to_string();
        assert!(print_response(response).is_err());
    }
}
