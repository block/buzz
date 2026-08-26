use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use super::codex_tasks::CodexTaskSummary;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSshConfigHost {
    pub alias: String,
    pub hostname: String,
    pub username: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSshConnectRequest {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub identity_file: Option<PathBuf>,
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
    #[serde(default)]
    pub identity_file: Option<PathBuf>,
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

pub fn list_config_hosts() -> Result<Vec<CodexSshConfigHost>, String> {
    let home =
        dirs::home_dir().ok_or_else(|| "could not find the user home directory".to_string())?;
    let config_path = home.join(".ssh").join("config");
    let config = match std::fs::read_to_string(&config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("failed to read {}: {error}", config_path.display())),
    };
    let mut aliases = Vec::new();
    for raw_line in config.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        let mut parts = line.split_whitespace();
        if !parts
            .next()
            .is_some_and(|key| key.eq_ignore_ascii_case("host"))
        {
            continue;
        }
        for alias in parts {
            if !alias.contains(['*', '!', '?']) && !aliases.iter().any(|value| value == alias) {
                aliases.push(alias.to_string());
            }
        }
    }
    let ssh = if cfg!(windows) { "ssh.exe" } else { "ssh" };
    let mut hosts = Vec::new();
    for alias in aliases {
        let mut command = Command::new(ssh);
        hide_console_window(&mut command);
        let output = command
            .args(["-G", &alias])
            .output()
            .map_err(|error| format!("failed to inspect SSH config: {error}"))?;
        if !output.status.success() {
            continue;
        }
        let resolved = String::from_utf8_lossy(&output.stdout);
        let value = |name: &str| {
            resolved.lines().find_map(|line| {
                let (key, value) = line.split_once(' ')?;
                key.eq_ignore_ascii_case(name)
                    .then(|| value.trim().to_string())
            })
        };
        hosts.push(CodexSshConfigHost {
            hostname: value("hostname").unwrap_or_else(|| alias.clone()),
            username: value("user").unwrap_or_default(),
            port: value("port")
                .and_then(|port| port.parse().ok())
                .unwrap_or(22),
            alias,
        });
    }
    Ok(hosts)
}

fn is_powershell(shell: &str) -> bool {
    matches!(
        shell.trim().to_ascii_lowercase().as_str(),
        "powershell" | "pwsh" | "windows"
    )
}

fn hide_console_window(command: &mut Command) {
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
}

fn usable_identity_file(path: Option<&PathBuf>) -> Option<&PathBuf> {
    path.filter(|value| !value.as_os_str().is_empty())
}

fn app_server_ready(local_port: u16) -> bool {
    let address = format!("127.0.0.1:{local_port}");
    let Ok(address) = address.parse() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(300)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    if stream
        .write_all(b"GET /readyz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = [0_u8; 256];
    let Ok(read) = stream.read(&mut response) else {
        return false;
    };
    let response = String::from_utf8_lossy(&response[..read]);
    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

const SSH_STDERR_LIMIT: usize = 16 * 1024;

fn capture_stderr(stderr: std::process::ChildStderr) -> Arc<Mutex<VecDeque<u8>>> {
    let captured = Arc::new(Mutex::new(VecDeque::with_capacity(SSH_STDERR_LIMIT)));
    let writer = Arc::clone(&captured);
    thread::spawn(move || {
        let mut stderr = stderr;
        let mut chunk = [0_u8; 1024];
        loop {
            let Ok(read) = stderr.read(&mut chunk) else {
                break;
            };
            if read == 0 {
                break;
            }
            let Ok(mut buffer) = writer.lock() else {
                break;
            };
            buffer.extend(&chunk[..read]);
            while buffer.len() > SSH_STDERR_LIMIT {
                buffer.pop_front();
            }
        }
    });
    captured
}

fn captured_stderr_text(captured: Option<&Arc<Mutex<VecDeque<u8>>>>) -> String {
    let Some(captured) = captured else {
        return String::new();
    };
    let Ok(mut buffer) = captured.lock() else {
        return String::new();
    };
    String::from_utf8_lossy(buffer.make_contiguous())
        .trim()
        .to_string()
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
    if let Some(identity_file) = usable_identity_file(request.identity_file.as_ref()) {
        if !identity_file.is_file() {
            return Err(format!(
                "SSH identity file does not exist: {}",
                identity_file.display()
            ));
        }
    }
    Ok(())
}

fn ssh_capture(request: &CodexSshTaskQueryRequest, remote_command: &str) -> Result<String, String> {
    if request.host.trim().is_empty() || request.username.trim().is_empty() {
        return Err("SSH host and username are required".to_string());
    }
    if let Some(identity_file) = usable_identity_file(request.identity_file.as_ref()) {
        if !identity_file.is_file() {
            return Err(format!(
                "SSH identity file does not exist: {}",
                identity_file.display()
            ));
        }
    }
    let mut command = Command::new(if cfg!(windows) { "ssh.exe" } else { "ssh" });
    hide_console_window(&mut command);
    command
        .args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=10"])
        .arg("-p")
        .arg(request.port.to_string());
    if let Some(identity_file) = usable_identity_file(request.identity_file.as_ref()) {
        command.arg("-i").arg(identity_file);
    }
    let output = command
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
        "{}@{}:{}->{}",
        request.username.trim(),
        request.host.trim(),
        request.port,
        request.remote_app_server_port,
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
    hide_console_window(&mut command);
    command
        .args([
            "-T",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "BatchMode=yes",
            "-o",
            "ServerAliveInterval=30",
            "-o",
            "ServerAliveCountMax=3",
        ])
        .arg("-p")
        .arg(request.port.to_string());
    if let Some(identity_file) = usable_identity_file(request.identity_file.as_ref()) {
        command.arg("-i").arg(identity_file);
    }
    command
        .arg("-L")
        .arg(target)
        .arg(format!(
            "{}@{}",
            request.username.trim(),
            request.host.trim()
        ))
        .arg(if is_powershell(&request.remote_shell) {
            format!(
                "powershell -NoProfile -Command \"$c=Get-Command codex -ErrorAction SilentlyContinue; if (-not $c) {{ Write-Error 'codex is not on the remote PowerShell PATH'; exit 127 }}; & $c.Source app-server --listen ws://127.0.0.1:{}\"",
                request.remote_app_server_port
            )
        } else {
            format!(
                "bash -lic 'source ~/.profile >/dev/null 2>&1 || true; source ~/.bashrc >/dev/null 2>&1 || true; command -v codex >/dev/null 2>&1 || {{ echo \"codex is not on the remote login PATH; install Codex or add it to ~/.profile or ~/.bashrc\" >&2; exit 127; }}; exec codex app-server --listen ws://127.0.0.1:{}'",
                request.remote_app_server_port
            )
        })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start ssh: {error}"))?;
    let stderr = child.stderr.take().map(capture_stderr);
    thread::sleep(Duration::from_millis(700));
    if let Some(status) = child
        .try_wait()
        .map_err(|error| format!("failed to inspect ssh: {error}"))?
    {
        let detail = captured_stderr_text(stderr.as_ref());
        return Err(format!(
            "SSH remote Codex app-server exited with {status}: {}",
            detail.trim()
        ));
    }
    let mut tunnel_ready = false;
    for _ in 0..40 {
        if app_server_ready(local_port) {
            tunnel_ready = true;
            break;
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to inspect ssh: {error}"))?
        {
            thread::sleep(Duration::from_millis(20));
            let detail = captured_stderr_text(stderr.as_ref());
            return Err(format!(
                "SSH remote Codex app-server exited with {status}: {detail}"
            ));
        }
        thread::sleep(Duration::from_millis(250));
    }
    if !tunnel_ready {
        let _ = child.kill();
        let _ = child.wait();
        let detail = captured_stderr_text(stderr.as_ref());
        let suffix = (!detail.is_empty())
            .then(|| format!(": {detail}"))
            .unwrap_or_default();
        return Err(format!(
            "SSH tunnel started but the remote Codex app-server did not become reachable{suffix}"
        ));
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
    let mut active = tunnels()
        .lock()
        .map_err(|_| "SSH tunnel state is unavailable".to_string())?;
    let keys = active
        .keys()
        .filter(|candidate| *candidate == key || candidate.starts_with(&format!("{key}->")))
        .cloned()
        .collect::<Vec<_>>();
    for key in keys {
        if let Some((_, mut child)) = active.remove(&key) {
            let _ = child.kill();
            let _ = child.wait();
        }
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
