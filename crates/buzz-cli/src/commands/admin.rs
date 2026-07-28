//! `buzz admin` — authenticated, owner-local Buzz Desktop administration.

use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::CliError;
use crate::{AdminAgentsCmd, AdminCmd, AdminPersonasCmd, AdminSkillsCmd, AdminTemplatesCmd};

const TOKEN_KEY: &str = "local-admin-token";
const KEYRING_BLOB_KEY: &str = "secrets";
const PROD_BUNDLE_IDENTIFIER: &str = "xyz.block.buzz.app";

#[derive(Serialize)]
struct Request {
    token: String,
    action: String,
    input: Value,
}

#[derive(Deserialize)]
struct Response {
    ok: bool,
    data: Option<Value>,
    error: Option<String>,
}

pub async fn dispatch(command: AdminCmd) -> Result<(), CliError> {
    let (action, input) = match command {
        AdminCmd::Status => ("status", json!({})),
        AdminCmd::Personas { command } => match command {
            AdminPersonasCmd::List => ("personas.list", json!({})),
            AdminPersonasCmd::Apply { file } => ("personas.apply", read_json(&file)?),
        },
        AdminCmd::Agents { command } => match command {
            AdminAgentsCmd::List => ("agents.list", json!({})),
            AdminAgentsCmd::Create { file } => ("agents.create", read_json(&file)?),
            AdminAgentsCmd::Update { file } => ("agents.update", read_json(&file)?),
            AdminAgentsCmd::Start { pubkey } => ("agents.start", json!({"pubkey": pubkey})),
            AdminAgentsCmd::Stop { pubkey } => ("agents.stop", json!({"pubkey": pubkey})),
            AdminAgentsCmd::Restart { pubkey } => ("agents.restart", json!({"pubkey": pubkey})),
        },
        AdminCmd::Skills { command } => match command {
            AdminSkillsCmd::Install { name, source } => {
                ("skills.install", json!({"name": name, "source": source}))
            }
        },
        AdminCmd::Templates { command } => match command {
            AdminTemplatesCmd::Apply { file } => ("templates.apply", json!({"source": file})),
        },
    };

    let response = request(action, input).await?;
    if !response.ok {
        return Err(CliError::Other(
            response
                .error
                .unwrap_or_else(|| "local admin request failed".to_string()),
        ));
    }
    println!(
        "{}",
        serde_json::to_string(&response.data.unwrap_or(Value::Null))
            .map_err(|e| CliError::Other(e.to_string()))?
    );
    Ok(())
}

fn read_json(path: &str) -> Result<Value, CliError> {
    let content = if path == "-" {
        std::io::read_to_string(std::io::stdin())
            .map_err(|e| CliError::Usage(format!("failed to read stdin: {e}")))?
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| CliError::Usage(format!("failed to read {path}: {e}")))?
    };
    serde_json::from_str(&content).map_err(|e| CliError::Usage(format!("invalid JSON input: {e}")))
}

#[cfg(unix)]
async fn request(action: &str, input: Value) -> Result<Response, CliError> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let token = load_token()?;
    let socket_path = socket_path()?;
    let mut stream = UnixStream::connect(&socket_path).await.map_err(|e| {
        CliError::Other(format!(
            "Buzz Desktop local admin is unavailable at {}: {e}",
            socket_path.display()
        ))
    })?;
    let request = Request {
        token,
        action: action.to_string(),
        input,
    };
    let mut bytes = serde_json::to_vec(&request).map_err(|e| CliError::Other(e.to_string()))?;
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .await
        .map_err(|e| CliError::Other(format!("local admin write failed: {e}")))?;

    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .await
        .map_err(|e| CliError::Other(format!("local admin read failed: {e}")))?;
    serde_json::from_str(&line)
        .map_err(|e| CliError::Other(format!("invalid local admin response: {e}")))
}

#[cfg(not(unix))]
async fn request(_action: &str, _input: Value) -> Result<Response, CliError> {
    Err(CliError::Other(
        "Buzz local administration is currently available on macOS and Linux".to_string(),
    ))
}

fn socket_path() -> Result<PathBuf, CliError> {
    if let Some(path) = std::env::var_os("BUZZ_ADMIN_SOCKET") {
        return Ok(PathBuf::from(path));
    }
    dirs::data_dir()
        .map(|path| path.join(PROD_BUNDLE_IDENTIFIER).join("local-admin.sock"))
        .ok_or_else(|| CliError::Other("cannot resolve platform data directory".to_string()))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn load_token() -> Result<String, CliError> {
    let service =
        std::env::var("BUZZ_ADMIN_KEYRING_SERVICE").unwrap_or_else(|_| "buzz-desktop".to_string());
    let entry = keyring::Entry::new(&service, KEYRING_BLOB_KEY)
        .map_err(|e| CliError::Auth(format!("cannot access Buzz Keychain entry: {e}")))?;
    let raw = entry
        .get_password()
        .map_err(|e| CliError::Auth(format!("cannot read Buzz local admin credential: {e}")))?;
    let secrets: HashMap<String, String> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Auth(format!("Buzz Keychain entry is invalid: {e}")))?;
    secrets.get(TOKEN_KEY).cloned().ok_or_else(|| {
        CliError::Auth("Buzz Desktop has not initialized local administration yet".to_string())
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn load_token() -> Result<String, CliError> {
    Err(CliError::Auth(
        "Buzz local administration credential access is unsupported on this platform".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_override_supports_isolated_desktop_instances() {
        let key = "BUZZ_ADMIN_SOCKET";
        let previous = std::env::var_os(key);
        std::env::set_var(key, "/tmp/buzz-test-admin.sock");
        assert_eq!(
            socket_path().unwrap(),
            PathBuf::from("/tmp/buzz-test-admin.sock")
        );
        if let Some(value) = previous {
            std::env::set_var(key, value);
        } else {
            std::env::remove_var(key);
        }
    }
}
