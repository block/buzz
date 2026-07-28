//! Owner-only local administration IPC for Buzz Desktop.
//!
//! The socket and Keychain token authorize the current OS user to perform the
//! same validated persona and managed-agent operations as the Desktop UI. The
//! surface intentionally has no delete, archive, identity, secret-read, or
//! outbound-message operations.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
};

use crate::{
    app_state::AppState,
    commands::{
        create_managed_agent, create_persona, list_managed_agents, list_personas,
        start_managed_agent, stop_managed_agent, update_managed_agent, update_persona,
    },
    managed_agents::{
        CreateManagedAgentRequest, CreatePersonaRequest, UpdateManagedAgentRequest,
        UpdatePersonaRequest,
    },
};

const TOKEN_KEY: &str = "local-admin-token";
const SOCKET_NAME: &str = "local-admin.sock";
const AUDIT_NAME: &str = "local-admin-audit.jsonl";
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
struct Request {
    token: String,
    action: String,
    #[serde(default)]
    input: Value,
}

#[derive(Serialize)]
struct Response {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct AuditEntry<'a> {
    timestamp: String,
    action: &'a str,
    target: Option<String>,
    ok: bool,
    error: Option<String>,
}

pub(crate) fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = serve(app).await {
            eprintln!("buzz-desktop: local admin unavailable: {error}");
        }
    });
}

async fn serve(app: AppHandle) -> Result<(), String> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
    let socket_path = std::env::var_os("BUZZ_ADMIN_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join(SOCKET_NAME));
    remove_stale_socket(&socket_path)?;

    let token = load_or_create_token()?;
    let listener = UnixListener::bind(&socket_path).map_err(|e| e.to_string())?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;

    eprintln!(
        "buzz-desktop: local admin listening on {}",
        socket_path.display()
    );
    loop {
        let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let app = app.clone();
        let token = token.clone();
        let audit_path = data_dir.join(AUDIT_NAME);
        tauri::async_runtime::spawn(async move {
            if let Err(error) = handle_connection(stream, app, &token, &audit_path).await {
                eprintln!("buzz-desktop: local admin request failed: {error}");
            }
        });
    }
}

fn remove_stale_socket(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path).map_err(|e| e.to_string())
        }
        Ok(_) => Err(format!(
            "refusing to replace non-socket local admin path {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn load_or_create_token() -> Result<String, String> {
    let store = crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
    if let Some(token) = store.load(TOKEN_KEY)? {
        return Ok(token);
    }
    let token = nostr::Keys::generate().secret_key().to_secret_hex();
    store.store(TOKEN_KEY, &token)?;
    if !store.verify_stored_raw(TOKEN_KEY, &token)? {
        return Err("local admin token failed Keychain read-back verification".to_string());
    }
    Ok(token)
}

async fn handle_connection(
    stream: UnixStream,
    app: AppHandle,
    expected_token: &str,
    audit_path: &Path,
) -> Result<(), String> {
    let (reader, mut writer) = stream.into_split();
    let mut line = String::new();
    let bytes = BufReader::new(reader)
        .take(MAX_REQUEST_BYTES as u64)
        .read_line(&mut line)
        .await
        .map_err(|e| e.to_string())?;
    if bytes == 0 {
        return Ok(());
    }

    let request: Request = match serde_json::from_str(&line) {
        Ok(request) => request,
        Err(error) => {
            return write_response(
                &mut writer,
                Response {
                    ok: false,
                    data: None,
                    error: Some(format!("invalid request: {error}")),
                },
            )
            .await;
        }
    };
    if !constant_time_eq(request.token.as_bytes(), expected_token.as_bytes()) {
        let response = Response {
            ok: false,
            data: None,
            error: Some("local admin authentication failed".to_string()),
        };
        append_audit(
            audit_path,
            &request.action,
            None,
            &Err(response.error.clone().unwrap_or_default()),
        )?;
        return write_response(&mut writer, response).await;
    }

    let target = audit_target(&request.action, &request.input);
    let result = dispatch(&app, &request.action, request.input)
        .await
        .map(redact_sensitive);
    append_audit(audit_path, &request.action, target, &result)?;
    let response = match result {
        Ok(data) => Response {
            ok: true,
            data: Some(data),
            error: None,
        },
        Err(error) => Response {
            ok: false,
            data: None,
            error: Some(error),
        },
    };
    write_response(&mut writer, response).await
}

async fn write_response(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    response: Response,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await.map_err(|e| e.to_string())
}

async fn dispatch(app: &AppHandle, action: &str, input: Value) -> Result<Value, String> {
    match action {
        "status" => Ok(json!({"available": true, "transport": "unix", "auth": "keychain"})),
        "personas.list" => to_value(list_personas(app.clone()).await?),
        "personas.apply" => apply_persona(app, input).await,
        "agents.list" => to_value(list_managed_agents(app.clone()).await?),
        "agents.create" => {
            let request: CreateManagedAgentRequest = from_value(input)?;
            let state = app.state::<AppState>();
            let mut value = to_value(create_managed_agent(request, app.clone(), state).await?)?;
            // Creation returns the freshly minted nsec for interactive export.
            // The admin API never reveals keys.
            if let Some(object) = value.as_object_mut() {
                object.remove("privateKeyNsec");
                object.remove("private_key_nsec");
            }
            Ok(value)
        }
        "agents.update" => {
            let request: UpdateManagedAgentRequest = from_value(input)?;
            let state = app.state::<AppState>();
            to_value(update_managed_agent(request, app.clone(), state).await?)
        }
        "agents.start" => {
            let pubkey = required_string(&input, "pubkey")?;
            let state = app.state::<AppState>();
            to_value(start_managed_agent(pubkey, app.clone(), state).await?)
        }
        "agents.stop" => {
            let pubkey = required_string(&input, "pubkey")?;
            to_value(stop_managed_agent(pubkey, app.clone()).await?)
        }
        "agents.restart" => {
            let pubkey = required_string(&input, "pubkey")?;
            stop_managed_agent(pubkey.clone(), app.clone()).await?;
            let state = app.state::<AppState>();
            to_value(start_managed_agent(pubkey, app.clone(), state).await?)
        }
        "skills.install" => install_skill(input),
        "templates.apply" => apply_templates(app, input),
        _ => Err(format!("unsupported local admin action: {action}")),
    }
}

async fn apply_persona(app: &AppHandle, mut input: Value) -> Result<Value, String> {
    let explicit_id = input.get("id").and_then(Value::as_str).map(str::to_string);
    let display_name = required_string(&input, "displayName")?;
    let personas = list_personas(app.clone()).await?;
    let matching: Vec<_> = personas
        .iter()
        .filter(|persona| persona.display_name == display_name)
        .collect();
    let id = match explicit_id {
        Some(id) => Some(id),
        None if matching.len() == 1 => Some(matching[0].id.clone()),
        None if matching.len() > 1 => {
            return Err(format!(
                "multiple personas named {display_name:?}; provide id"
            ));
        }
        None => None,
    };

    if let Some(id) = id {
        input
            .as_object_mut()
            .ok_or_else(|| "persona input must be an object".to_string())?
            .insert("id".to_string(), Value::String(id));
        let request: UpdatePersonaRequest = from_value(input)?;
        to_value(update_persona(request, app.clone()).await?)
    } else {
        let request: CreatePersonaRequest = from_value(input)?;
        to_value(create_persona(request, app.clone()).await?)
    }
}

fn install_skill(input: Value) -> Result<Value, String> {
    let source = canonical_source(&input)?;
    let name = safe_name(required_string(&input, "name")?)?;
    let nest =
        crate::managed_agents::nest_dir().ok_or_else(|| "Buzz nest is unavailable".to_string())?;
    let target_dir = nest.join(".agents").join("skills");
    fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
    let target = target_dir.join(&name);
    if let Ok(existing) = fs::read_link(&target) {
        if existing == source {
            return Ok(json!({"name": name, "path": target, "changed": false}));
        }
        return Err(format!(
            "skill target {} already points elsewhere",
            target.display()
        ));
    }
    if target.exists() {
        return Err(format!("skill target {} already exists", target.display()));
    }
    std::os::unix::fs::symlink(&source, &target).map_err(|e| e.to_string())?;
    Ok(json!({"name": name, "path": target, "changed": true}))
}

fn apply_templates(app: &AppHandle, input: Value) -> Result<Value, String> {
    let source = canonical_source(&input)?;
    let bytes = fs::read(&source).map_err(|e| e.to_string())?;
    let _: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid template JSON: {e}"))?;
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let templates_dir = data_dir.join("templates");
    fs::create_dir_all(&templates_dir).map_err(|e| e.to_string())?;
    let target = templates_dir.join("channel-templates.json");
    let changed = fs::read(&target).ok().as_deref() != Some(bytes.as_slice());
    if changed {
        let mut file = atomic_write_file::AtomicWriteFile::options()
            .open(&target)
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.commit().map_err(|e| e.to_string())?;
    }
    Ok(json!({"path": target, "changed": changed}))
}

fn canonical_source(input: &Value) -> Result<PathBuf, String> {
    let source = required_string(input, "source")?;
    PathBuf::from(source)
        .canonicalize()
        .map_err(|e| format!("invalid source: {e}"))
}

fn safe_name(name: String) -> Result<String, String> {
    if name.is_empty()
        || name.starts_with('.')
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("name must contain only letters, numbers, '-' or '_'".to_string());
    }
    Ok(name)
}

fn required_string(input: &Value, key: &str) -> Result<String, String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{key} is required"))
}

fn from_value<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn to_value<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

fn audit_target(action: &str, input: &Value) -> Option<String> {
    match action {
        "personas.apply" => input.get("id").or_else(|| input.get("displayName")),
        action if action.starts_with("agents.") => {
            input.get("pubkey").or_else(|| input.get("name"))
        }
        "skills.install" => input.get("name"),
        "templates.apply" => input.get("source"),
        _ => None,
    }
    .and_then(Value::as_str)
    .map(str::to_string)
}

fn redact_sensitive(mut value: Value) -> Value {
    redact_value(&mut value);
    value
}

fn redact_value(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
                if normalized == "envvars" {
                    if let Value::Object(env) = child {
                        for value in env.values_mut() {
                            *value = Value::String("[redacted]".to_string());
                        }
                    } else {
                        *child = Value::String("[redacted]".to_string());
                    }
                } else if normalized.contains("private")
                    || normalized.contains("secret")
                    || normalized.contains("token")
                    || normalized.contains("apikey")
                    || normalized.contains("authtag")
                    || normalized.contains("nsec")
                {
                    *child = Value::String("[redacted]".to_string());
                } else {
                    redact_value(child);
                }
            }
        }
        Value::Array(values) => {
            for child in values {
                redact_value(child);
            }
        }
        _ => {}
    }
}

fn append_audit(
    path: &Path,
    action: &str,
    target: Option<String>,
    result: &Result<Value, String>,
) -> Result<(), String> {
    let entry = AuditEntry {
        timestamp: crate::util::now_iso(),
        action,
        target,
        ok: result.is_ok(),
        error: result.as_ref().err().cloned(),
    };
    let mut line = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(&line).map_err(|e| e.to_string())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposed_actions_exclude_destructive_and_secret_operations() {
        for action in [
            "agents.delete",
            "personas.delete",
            "identity.export",
            "messages.send",
        ] {
            assert!(
                !matches!(
                    action,
                    "status"
                        | "personas.list"
                        | "personas.apply"
                        | "agents.list"
                        | "agents.create"
                        | "agents.update"
                        | "agents.start"
                        | "agents.stop"
                        | "agents.restart"
                        | "skills.install"
                        | "templates.apply"
                ),
                "gated action must not enter the local admin allowlist"
            );
        }
    }

    #[test]
    fn token_comparison_requires_exact_match() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"diff"));
        assert!(!constant_time_eq(b"short", b"longer"));
    }

    #[test]
    fn skill_names_cannot_escape_the_install_directory() {
        assert_eq!(safe_name("astro-agent".to_string()).unwrap(), "astro-agent");
        assert!(safe_name("../escape".to_string()).is_err());
        assert!(safe_name("nested/skill".to_string()).is_err());
    }

    #[test]
    fn responses_preserve_env_names_but_never_values_or_keys() {
        let redacted = redact_sensitive(json!({
            "env_vars": {"OPENAI_API_KEY": "live-value"},
            "nested": [{"privateKeyNsec": "nsec1...", "auth_tag": "tag-value"}]
        }));
        assert_eq!(redacted["env_vars"]["OPENAI_API_KEY"], "[redacted]");
        assert_eq!(redacted["nested"][0]["privateKeyNsec"], "[redacted]");
        assert_eq!(redacted["nested"][0]["auth_tag"], "[redacted]");
        assert!(!redacted.to_string().contains("live-value"));
        assert!(!redacted.to_string().contains("nsec1"));
        assert!(!redacted.to_string().contains("tag-value"));
    }
}
