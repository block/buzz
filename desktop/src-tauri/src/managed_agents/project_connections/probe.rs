use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write as _},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::Duration,
};

use super::*;

const TEST_TIMEOUT: Duration = Duration::from_secs(8);
const CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_DISCOVERED_TOOLS: usize = 256;
const PROBE_BUSY_ERROR: &str =
    "Another Project connection is being tested. Try again when it finishes.";
const EXECUTABLE_CHANGED_ERROR: &str =
    "This executable changed after it was approved. Edit the connection and review it again.";

static PROJECT_CONNECTION_PROBE_LOCK: Mutex<()> = Mutex::new(());

enum ReaderMessage {
    Line(Vec<u8>),
    Oversized,
    Closed,
}

fn inherited_test_env() -> BTreeMap<String, String> {
    [
        "PATH",
        "HOME",
        "USER",
        "TMPDIR",
        "TEMP",
        "TMP",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
    ]
    .into_iter()
    .filter_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| (key.to_string(), value))
    })
    .collect()
}

fn recv_json_response(
    rx: &std::sync::mpsc::Receiver<ReaderMessage>,
    expected_id: u64,
) -> Result<serde_json::Value, String> {
    let deadline = std::time::Instant::now() + TEST_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let message = rx
            .recv_timeout(remaining)
            .map_err(|_| "The MCP server did not respond in time.".to_string())?;
        let line = match message {
            ReaderMessage::Line(line) => line,
            ReaderMessage::Oversized => {
                return Err("The MCP server returned an oversized response.".to_string());
            }
            ReaderMessage::Closed => {
                return Err("The MCP server closed before responding.".to_string());
            }
        };
        let value: serde_json::Value = serde_json::from_slice(&line)
            .map_err(|_| "The MCP server returned an invalid response.".to_string())?;
        if value.get("id").and_then(serde_json::Value::as_u64) == Some(expected_id) {
            if value.get("error").is_some() {
                return Err("The MCP server rejected the request.".to_string());
            }
            return value
                .get("result")
                .cloned()
                .ok_or_else(|| "The MCP server returned no result.".to_string());
        }
    }
}

fn read_bounded_line(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, ()> {
    let mut line = Vec::new();
    let count = reader
        .take((MAX_RESPONSE_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)
        .map_err(|_| ())?;
    if count == 0 {
        return Ok(None);
    }
    if line.len() > MAX_RESPONSE_BYTES {
        return Err(());
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    Ok(Some(line))
}

fn stop_child(child: &mut Child, pid: u32) -> Result<(), String> {
    let termination = super::super::runtime::terminate_process(pid);
    let deadline = std::time::Instant::now() + CLEANUP_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return termination.map_err(|_| {
                    "Buzz stopped the MCP server, but could not verify process-group cleanup."
                        .to_string()
                })
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => break,
            Err(_) => return Err("Buzz could not verify that the MCP server stopped.".to_string()),
        }
    }
    let _ = child.kill();
    let kill_deadline = std::time::Instant::now() + CLEANUP_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return Err("Buzz had to force-stop this MCP server after the test.".to_string());
            }
            Ok(None) if std::time::Instant::now() < kill_deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                return Err("Buzz could not stop this MCP server after the test.".to_string());
            }
        }
    }
}

fn verify_saved_executable(connection: &StoredProjectConnection) -> Result<(), String> {
    let (canonical, fingerprint) = canonical_connection_command(&connection.command)?;
    if canonical != connection.command || fingerprint != connection.executable_sha256 {
        return Err(EXECUTABLE_CHANGED_ERROR.to_string());
    }
    Ok(())
}

fn probe_mcp_connection(
    connection: &StoredProjectConnection,
    secrets: &BTreeMap<String, String>,
) -> Result<Vec<String>, String> {
    let _probe_guard = PROJECT_CONNECTION_PROBE_LOCK
        .try_lock()
        .map_err(|_| PROBE_BUSY_ERROR.to_string())?;
    let mut command = Command::new(&connection.command);
    command
        .args(&connection.args)
        .env_clear()
        .envs(inherited_test_env())
        .envs(secrets)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|_| "Buzz could not start this MCP server.".to_string())?;
    let pid = child.id();
    let Some(stdout) = child.stdout.take() else {
        let _ = stop_child(&mut child, pid);
        return Err("Buzz could not read from this MCP server.".to_string());
    };
    let Some(mut stdin) = child.stdin.take() else {
        let _ = stop_child(&mut child, pid);
        return Err("Buzz could not write to this MCP server.".to_string());
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let message = match read_bounded_line(&mut reader) {
                Ok(Some(line)) => ReaderMessage::Line(line),
                Ok(None) => ReaderMessage::Closed,
                Err(()) => ReaderMessage::Oversized,
            };
            let terminal = matches!(message, ReaderMessage::Closed | ReaderMessage::Oversized);
            if tx.send(message).is_err() || terminal {
                return;
            }
        }
    });

    let result = (|| {
        let initialize = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {
                    "name": "buzz-desktop",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });
        writeln!(stdin, "{initialize}")
            .and_then(|_| stdin.flush())
            .map_err(|_| "Buzz could not initialize this MCP server.".to_string())?;
        let initialized = recv_json_response(&rx, 1)?;
        if initialized.get("protocolVersion").is_none() {
            return Err("The MCP server did not complete initialization.".to_string());
        }
        writeln!(
            stdin,
            "{}",
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized",
                "params": {}
            })
        )
        .and_then(|_| {
            writeln!(
                stdin,
                "{}",
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/list",
                    "params": {}
                })
            )
        })
        .and_then(|_| stdin.flush())
        .map_err(|_| "Buzz could not inspect this MCP server.".to_string())?;
        let tools_result = recv_json_response(&rx, 2)?;
        let tools = tools_result
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "The MCP server did not return a tool list.".to_string())?;
        if tools.len() > MAX_DISCOVERED_TOOLS {
            return Err("The MCP server returned too many tools.".to_string());
        }
        let mut names = Vec::with_capacity(tools.len());
        for tool in tools {
            let name = tool
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| valid_stable_id(name, 128))
                .ok_or_else(|| "The MCP server returned an invalid tool name.".to_string())?;
            names.push(name.to_string());
        }
        names.sort();
        names.dedup();
        if names.is_empty() {
            return Err("The MCP server did not expose any tools.".to_string());
        }
        Ok(names)
    })();

    drop(stdin);
    drop(rx);
    match stop_child(&mut child, pid) {
        Ok(()) => result,
        Err(cleanup_error) => Err(cleanup_error),
    }
}

fn safe_health_detail(error: &str) -> String {
    match error {
        "The MCP server did not respond in time." => error.to_string(),
        "Buzz could not start this MCP server." => error.to_string(),
        PROBE_BUSY_ERROR | EXECUTABLE_CHANGED_ERROR => error.to_string(),
        _ => "Buzz could not verify this MCP server.".to_string(),
    }
}

pub fn test_project_connection(
    app: &AppHandle,
    project_scope: &ProjectConnectionScope,
    connection_id: &str,
) -> Result<ProjectConnection, String> {
    let project_scope = validate_project_scope_for_app(app, project_scope)?;
    let connection = {
        let _guard = lock_project_connections();
        let store = load_store_unlocked(app, &project_scope)?;
        find_connection(&store, &project_scope, connection_id)?.clone()
    };
    if let Err(error) = verify_saved_executable(&connection) {
        let _guard = lock_project_connections();
        let mut store = load_store_unlocked(app, &project_scope)?;
        if let Some(current) = store.connections.iter_mut().find(|candidate| {
            candidate.id == connection.id
                && candidate.project_scope == project_scope
                && candidate.generation == connection.generation
        }) {
            current.updated_at = now_iso();
            current.health = ProjectConnectionHealth {
                status: ProjectConnectionHealthStatus::CheckNeeded,
                last_verified_at: None,
                detail: Some("Executable approval is out of date.".to_string()),
            };
            save_store_unlocked(app, &project_scope, &store)?;
        }
        return Err(error);
    }
    let secrets = match load_secrets(app, &connection) {
        Ok(secrets) => secrets,
        Err(error) => {
            let _guard = lock_project_connections();
            let mut store = load_store_unlocked(app, &project_scope)?;
            if let Some(current) = store.connections.iter_mut().find(|candidate| {
                candidate.id == connection.id
                    && candidate.project_scope == project_scope
                    && candidate.generation == connection.generation
            }) {
                current.updated_at = now_iso();
                current.health = ProjectConnectionHealth {
                    status: ProjectConnectionHealthStatus::SignInRequired,
                    last_verified_at: None,
                    detail: Some("Saved credentials are unavailable.".to_string()),
                };
                save_store_unlocked(app, &project_scope, &store)?;
            }
            return Err(error);
        }
    };
    let result = probe_mcp_connection(&connection, &secrets);
    if matches!(&result, Err(error) if error == PROBE_BUSY_ERROR) {
        return Err(PROBE_BUSY_ERROR.to_string());
    }
    let _guard = lock_project_connections();
    let mut store = load_store_unlocked(app, &project_scope)?;
    let index = store
        .connections
        .iter()
        .position(|candidate| {
            candidate.id == connection_id && candidate.project_scope == project_scope
        })
        .ok_or_else(|| "This connection was removed while Buzz tested it.".to_string())?;
    if store.connections[index].generation != connection.generation {
        return Err("This connection changed while Buzz tested it. Test it again.".to_string());
    }
    let connection = &mut store.connections[index];
    connection.updated_at = now_iso();
    match result {
        Ok(tools) => {
            connection.discovered_tools = tools.clone();
            connection.capability_ids = tools
                .iter()
                .map(|tool| format!("mcp.tool.{tool}"))
                .collect();
            connection.health = ProjectConnectionHealth {
                status: ProjectConnectionHealthStatus::Ready,
                last_verified_at: Some(now_iso()),
                detail: None,
            };
            let updated = connection.clone();
            save_store_unlocked(app, &project_scope, &store)?;
            Ok(updated.into())
        }
        Err(error) => {
            connection.health = ProjectConnectionHealth {
                status: ProjectConnectionHealthStatus::Unavailable,
                last_verified_at: None,
                detail: Some(safe_health_detail(&error)),
            };
            save_store_unlocked(app, &project_scope, &store)?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::Path;

    #[test]
    fn synthetic_server_proves_initialize_and_tool_discovery() {
        let node = super::super::super::resolve_command("node")
            .expect("Hermit must provide Node for desktop tests");
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/synthetic-project-connection-mcp.mjs");
        assert!(script.is_file(), "missing fixture {}", script.display());
        let executable_sha256 = executable_sha256(&node).unwrap();
        let connection = StoredProjectConnection {
            id: "synthetic-project-connection".to_string(),
            project_scope: ProjectConnectionScope {
                relay_url: "ws://127.0.0.1:3000".to_string(),
                operator_pubkey: "a".repeat(64),
                repo_address: format!("30617:{}:portable-agents", "a".repeat(64)),
            },
            name: "Synthetic analytics".to_string(),
            provider: "Buzz test fixture".to_string(),
            capability_ids: Vec::new(),
            command: node.to_string_lossy().to_string(),
            args: vec![script.to_string_lossy().to_string()],
            env_keys: vec!["PROJECT_CONNECTION_CANARY".to_string()],
            discovered_tools: Vec::new(),
            health: ProjectConnectionHealth::default(),
            executable_sha256,
            generation: next_generation(),
            credential_generation: next_generation(),
            created_at: now_iso(),
            updated_at: now_iso(),
        };
        let secrets = BTreeMap::from([(
            "PROJECT_CONNECTION_CANARY".to_string(),
            "test-only".to_string(),
        )]);

        assert_eq!(
            probe_mcp_connection(&connection, &secrets).unwrap(),
            ["analytics.weekly_summary"]
        );
    }

    #[test]
    fn bounded_reader_rejects_a_response_without_a_newline() {
        let mut input = Cursor::new(vec![b'x'; MAX_RESPONSE_BYTES + 1]);
        assert!(read_bounded_line(&mut input).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn executable_replacement_invalidates_approval() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let executable = dir.path().join("server");
        fs::write(&executable, b"first").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let (command, executable_sha256) =
            canonical_connection_command(executable.to_str().unwrap()).unwrap();
        let connection = StoredProjectConnection {
            id: "c".repeat(32),
            project_scope: ProjectConnectionScope {
                relay_url: "ws://127.0.0.1:3000".to_string(),
                operator_pubkey: "a".repeat(64),
                repo_address: format!("30617:{}:portable-agents", "a".repeat(64)),
            },
            name: "Test".to_string(),
            provider: "Fixture".to_string(),
            capability_ids: Vec::new(),
            command,
            args: Vec::new(),
            env_keys: Vec::new(),
            discovered_tools: Vec::new(),
            health: ProjectConnectionHealth::default(),
            executable_sha256,
            generation: next_generation(),
            credential_generation: next_generation(),
            created_at: now_iso(),
            updated_at: now_iso(),
        };

        assert!(verify_saved_executable(&connection).is_ok());
        fs::write(&executable, b"second").unwrap();
        assert_eq!(
            verify_saved_executable(&connection).unwrap_err(),
            EXECUTABLE_CHANGED_ERROR
        );
    }
}
