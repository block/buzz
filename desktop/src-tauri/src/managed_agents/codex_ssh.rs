use std::{
    collections::HashMap,
    io::Read,
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::codex_tasks::CodexTaskSummary;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSshConnectRequest {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub username: String,
    pub identity_file: PathBuf,
    #[serde(default = "default_remote_app_server_port")]
    pub remote_app_server_port: u16,
    #[serde(default)]
    pub remote_shell: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSshRuntimeStatus {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub local_port: u16,
    pub app_server_url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSshTaskQueryRequest {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub username: String,
    pub identity_file: PathBuf,
    #[serde(default)]
    pub remote_shell: String,
}

#[derive(Debug, Deserialize)]
struct RemoteSessionIndexEntry {
    id: String,
    thread_name: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct RemoteSessionMetaLine {
    payload: RemoteSessionMetaPayload,
}

#[derive(Debug, Deserialize)]
struct RemoteSessionMetaPayload {
    id: String,
    cwd: String,
}

fn default_ssh_port() -> u16 {
    22
}
fn default_remote_app_server_port() -> u16 {
    51919
}

fn is_powershell(shell: &str) -> bool {
    matches!(
        shell.trim().to_ascii_lowercase().as_str(),
        "powershell" | "pwsh" | "windows"
    )
}

fn tunnels() -> &'static Mutex<HashMap<String, (CodexSshRuntimeStatus, Child)>> {
    static TUNNELS: OnceLock<Mutex<HashMap<String, (CodexSshRuntimeStatus, Child)>>> =
        OnceLock::new();
    TUNNELS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn validate(request: &CodexSshConnectRequest) -> Result<(), String> {
    if request.host.trim().is_empty() || request.username.trim().is_empty() {
        return Err("SSH host and username are required".to_string());
    }
    if !request.identity_file.is_file() {
        return Err(format!(
            "SSH identity file does not exist: {}",
            request.identity_file.display()
        ));
    }
    Ok(())
}

fn ssh_capture(request: &CodexSshTaskQueryRequest, remote_command: &str) -> Result<String, String> {
    if request.host.trim().is_empty() || request.username.trim().is_empty() {
        return Err("SSH host and username are required".to_string());
    }
    if !request.identity_file.is_file() {
        return Err(format!(
            "SSH identity file does not exist: {}",
            request.identity_file.display()
        ));
    }
    let output = Command::new(if cfg!(windows) { "ssh.exe" } else { "ssh" })
        .args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"])
        .arg("-p")
        .arg(request.port.to_string())
        .arg("-i")
        .arg(&request.identity_file)
        .arg(format!(
            "{}@{}",
            request.username.trim(),
            request.host.trim()
        ))
        .arg(remote_command)
        .output()
        .map_err(|error| format!("failed to run ssh: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "remote command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("remote command returned invalid UTF-8: {error}"))
}

pub fn list_codex_ssh_tasks(
    request: CodexSshTaskQueryRequest,
) -> Result<Vec<CodexTaskSummary>, String> {
    let index_command = if is_powershell(&request.remote_shell) {
        "Get-Content \"$HOME/.codex/session_index.jsonl\""
    } else {
        "cat ~/.codex/session_index.jsonl"
    };
    let output = ssh_capture(&request, index_command)?;
    let metadata = ssh_capture(
        &request,
        if is_powershell(&request.remote_shell) {
            "Get-ChildItem \"$HOME/.codex/sessions\",\"$HOME/.codex/archived_sessions\" -Recurse -Filter *.jsonl -File | ForEach-Object { Select-String -Path $_.FullName -Pattern 'session_meta' | ForEach-Object Line }"
        } else {
            "find ~/.codex/sessions ~/.codex/archived_sessions -type f -name '*.jsonl' -print0 | xargs -0 grep -h 'session_meta' 2>/dev/null"
        },
    )
    .unwrap_or_default();
    let workspaces = metadata
        .lines()
        .filter_map(|line| serde_json::from_str::<RemoteSessionMetaLine>(line).ok())
        .filter_map(|line| {
            Uuid::parse_str(&line.payload.id)
                .ok()
                .map(|id| (id.to_string(), line.payload.cwd))
        })
        .collect::<HashMap<_, _>>();
    let mut tasks = Vec::new();
    for entry in output
        .lines()
        .filter_map(|line| serde_json::from_str::<RemoteSessionIndexEntry>(line).ok())
    {
        let Ok(id) = Uuid::parse_str(&entry.id) else {
            continue;
        };
        tasks.push(CodexTaskSummary {
            id: id.to_string(),
            thread_name: entry.thread_name,
            workspace: workspaces
                .get(&id.to_string())
                .cloned()
                .unwrap_or_else(|| "Remote workspace".to_string()),
            updated_at: entry.updated_at,
            archived: false,
            model: None,
        });
    }
    tasks.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    tasks.dedup_by(|left, right| left.id == right.id);
    tasks.truncate(250);
    Ok(tasks)
}

pub fn read_codex_ssh_task_history(
    request: CodexSshTaskQueryRequest,
    task_id: &str,
) -> Result<String, String> {
    let id =
        Uuid::parse_str(task_id.trim()).map_err(|_| "Codex task ID must be a UUID".to_string())?;
    let command = if is_powershell(&request.remote_shell) {
        format!("$file = Get-ChildItem \"$HOME/.codex/sessions\",\"$HOME/.codex/archived_sessions\" -Recurse -Filter *.jsonl -File | Where-Object {{ Select-String -Path $_.FullName -Pattern 'session_meta.*{id}' -Quiet }} | Select-Object -First 1; if ($file) {{ Get-Content $file.FullName }}")
    } else {
        format!("file=$(find ~/.codex/sessions ~/.codex/archived_sessions -type f -name '*.jsonl' -print0 | xargs -0 grep -l 'session_meta.*{id}' 2>/dev/null | head -n 1); test -n \"$file\" && cat \"$file\"")
    };
    ssh_capture(&request, &command)
}

pub fn connect(request: CodexSshConnectRequest) -> Result<CodexSshRuntimeStatus, String> {
    validate(&request)?;
    let local_port = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("could not allocate a local tunnel port: {error}"))?
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let key = format!(
        "{}@{}:{}",
        request.username.trim(),
        request.host.trim(),
        request.port,
    );
    if let Ok(mut active) = tunnels().lock() {
        if let Some((status, child)) = active.get_mut(&key) {
            if child
                .try_wait()
                .map_err(|error| format!("failed to inspect existing SSH tunnel: {error}"))?
                .is_none()
            {
                return Ok(status.clone());
            }
            active.remove(&key);
        }
    }
    let target = format!(
        "127.0.0.1:{}:127.0.0.1:{}",
        local_port, request.remote_app_server_port
    );
    let mut command = Command::new(if cfg!(windows) { "ssh.exe" } else { "ssh" });
    command
        .args([
            "-T",
            "-o",
            "ExitOnForwardFailure=yes",
            "BatchMode=yes",
            "-o",
            "ServerAliveInterval=30",
            "-o",
            "ServerAliveCountMax=3",
        ])
        .arg("-p")
        .arg(request.port.to_string())
        .arg("-i")
        .arg(&request.identity_file)
        .arg("-L")
        .arg(target)
        .arg(format!(
            "{}@{}",
            request.username.trim(),
            request.host.trim()
        ))
        .arg(if is_powershell(&request.remote_shell) {
            format!(
                "powershell -NoProfile -Command \"codex app-server --listen ws://127.0.0.1:{}\"",
                request.remote_app_server_port
            )
        } else {
            format!(
                "codex app-server --listen ws://127.0.0.1:{}",
                request.remote_app_server_port
            )
        })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start ssh: {error}"))?;
    thread::sleep(Duration::from_millis(700));
    if let Some(status) = child
        .try_wait()
        .map_err(|error| format!("failed to inspect ssh: {error}"))?
    {
        let mut detail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut detail);
        }
        return Err(format!(
            "SSH remote Codex app-server exited with {status}: {}",
            detail.trim()
        ));
    }
    let tunnel_addr = format!("127.0.0.1:{local_port}")
        .parse()
        .map_err(|error| format!("invalid tunnel address: {error}"))?;
    let mut tunnel_ready = false;
    for _ in 0..20 {
        if TcpStream::connect_timeout(&tunnel_addr, Duration::from_millis(250)).is_ok() {
            tunnel_ready = true;
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    if !tunnel_ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err(
            "SSH tunnel started but the remote Codex app-server did not become reachable"
                .to_string(),
        );
    }
    let status = CodexSshRuntimeStatus {
        host: request.host,
        port: request.port,
        username: request.username,
        local_port,
        app_server_url: format!("ws://127.0.0.1:{local_port}"),
    };
    tunnels()
        .lock()
        .map_err(|_| "SSH tunnel state is unavailable".to_string())?
        .insert(key, (status.clone(), child));
    Ok(status)
}

pub fn stop(key: &str) -> Result<(), String> {
    if let Some((_, mut child)) = tunnels()
        .lock()
        .map_err(|_| "SSH tunnel state is unavailable".to_string())?
        .remove(key)
    {
        let _ = child.kill();
        let _ = child.wait();
    }
    Ok(())
}

pub fn stop_all() {
    if let Ok(mut active) = tunnels().lock() {
        for (_, (_, mut child)) in active.drain() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
