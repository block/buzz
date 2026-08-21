use std::{
    collections::HashSet,
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::codex_tasks::codex_shared_app_server_url;
use super::{atomic_write_json_restricted, managed_agents_base_dir};

const SHARED_RUNTIME_CONFIG_VERSION: u32 = 1;
const SHARED_RUNTIME_COMMAND_ENV: &str = "BUZZ_CODEX_APP_SERVER_COMMAND";
const SHARED_RUNTIME_ERROR_TAIL_BYTES: u64 = 4096;
const CODEX_CODE_MODE_HOST_FLAG: &str = "features.code_mode_host=true";
#[cfg(windows)]
const WINDOWS_CODEX_SHARED_RUNTIME_LAUNCHER_SCRIPT: &str = r#"
param([Parameter(Mandatory=$true)][string]$ConfigPath)
$ErrorActionPreference='Stop'
$config=Get-Content -LiteralPath $ConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json
try {
  Start-Process `
    -FilePath ([string]$config.executable) `
    -ArgumentList @('-c','features.code_mode_host=true','app-server','--listen',([string]$config.url)) `
    -WindowStyle Hidden `
    -RedirectStandardOutput ([string]$config.stdout_log) `
    -RedirectStandardError ([string]$config.stderr_log) | Out-Null
} catch {
  $message='buzz shared runtime launcher failed: ' + $_.Exception.Message + [Environment]::NewLine
  [IO.File]::AppendAllText([string]$config.stderr_log,$message,[Text.Encoding]::UTF8)
  exit 1
}
"#;
#[cfg(windows)]
const WINDOWS_CODEX_SHARED_RUNTIME_WMI_SCRIPT: &str = r#"
$ErrorActionPreference='Stop'
$startup=New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{ShowWindow=[uint16]0}
$result=Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
  CommandLine=$env:BUZZ_CODEX_SHARED_RUNTIME_COMMAND_LINE
  ProcessStartupInformation=$startup
}
if ($result.ReturnValue -ne 0) {
  throw "Win32_Process.Create returned $($result.ReturnValue)"
}
$result.ProcessId
"#;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CodexSharedRuntimeConfig {
    version: u32,
    enabled: bool,
}

impl Default for CodexSharedRuntimeConfig {
    fn default() -> Self {
        Self {
            version: SHARED_RUNTIME_CONFIG_VERSION,
            enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexSharedRuntimeState {
    SetupRequired,
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodexSharedRuntimeStatus {
    pub enabled: bool,
    pub state: CodexSharedRuntimeState,
    pub url: String,
    pub detail: Option<String>,
    pub desktop_process_ids: Vec<u32>,
    pub private_app_server_process_ids: Vec<u32>,
    pub desktop_detection_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct WindowsProcessInfo {
    process_id: u32,
    parent_process_id: u32,
    executable_path: String,
    command_line: String,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
struct WindowsProcessSnapshot {
    #[serde(default)]
    desktop_executable_paths: Vec<String>,
    #[serde(default)]
    private_app_server_executable_paths: Vec<String>,
    #[serde(default)]
    processes: Vec<WindowsProcessInfo>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CodexDesktopProcessSnapshot {
    desktop_processes: Vec<WindowsProcessInfo>,
    private_app_server_processes: Vec<WindowsProcessInfo>,
}

const WINDOWS_CODEX_PROCESS_SNAPSHOT_SCRIPT: &str = r#"
$ErrorActionPreference='Stop'
$desktopPaths=@()
$backendPaths=@()
$packages=@(Get-AppxPackage | Where-Object { $_.Name -in @('OpenAI.Codex','OpenAI.CodexBeta') })
foreach ($package in $packages) {
  $manifest=Get-AppxPackageManifest -Package $package
  foreach ($application in @($manifest.Package.Applications.Application)) {
    $relative=[string]$application.Executable
    if (-not [string]::IsNullOrWhiteSpace($relative)) {
      $desktopPaths += [IO.Path]::GetFullPath((Join-Path $package.InstallLocation $relative))
    }
  }
  $backendPaths += [IO.Path]::GetFullPath((Join-Path $package.InstallLocation 'app\resources\codex.exe'))
}
$processes=@(Get-CimInstance Win32_Process | ForEach-Object {
  [pscustomobject]@{
    process_id=[uint32]$_.ProcessId
    parent_process_id=[uint32]$_.ParentProcessId
    executable_path=if ($_.ExecutablePath) { [string]$_.ExecutablePath } else { '' }
    command_line=if ($_.CommandLine) { [string]$_.CommandLine } else { '' }
  }
})
[pscustomobject]@{
  desktop_executable_paths=@($desktopPaths)
  private_app_server_executable_paths=@($backendPaths)
  processes=@($processes)
} | ConvertTo-Json -Depth 4 -Compress
"#;

const WINDOWS_TERMINATE_VERIFIED_PROCESS_SCRIPT: &str = r#"
$ErrorActionPreference='Stop'
$pidValue=[uint32]$env:BUZZ_CODEX_TARGET_PID
$expected=[IO.Path]::GetFullPath($env:BUZZ_CODEX_TARGET_EXE).TrimEnd('\').ToLowerInvariant()
$process=Get-CimInstance Win32_Process -Filter "ProcessId = $pidValue"
if (-not $process) { exit 0 }
$actual=if ($process.ExecutablePath) { [IO.Path]::GetFullPath([string]$process.ExecutablePath).TrimEnd('\').ToLowerInvariant() } else { '' }
if ($actual -ne $expected) { throw "PID $pidValue no longer matches the verified Codex package path" }
$arguments=@('/PID',[string]$pidValue,'/F')
if ($env:BUZZ_CODEX_TARGET_TREE -eq '1') { $arguments += '/T' }
& taskkill.exe @arguments | Out-Null
if ($LASTEXITCODE -ne 0) { throw "taskkill exited with $LASTEXITCODE" }
"#;

fn shared_runtime_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("codex-shared-runtime.json"))
}

fn load_shared_runtime_config(app: &AppHandle) -> Result<CodexSharedRuntimeConfig, String> {
    let path = shared_runtime_config_path(app)?;
    if !path.exists() {
        return Ok(CodexSharedRuntimeConfig::default());
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn save_shared_runtime_config(
    app: &AppHandle,
    config: &CodexSharedRuntimeConfig,
) -> Result<(), String> {
    let path = shared_runtime_config_path(app)?;
    let payload = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("failed to serialize Codex shared runtime: {error}"))?;
    atomic_write_json_restricted(&path, &payload)
}

fn normalize_windows_executable_path(path: &str) -> String {
    path.trim()
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn command_has_argument(command_line: &str, expected: &str) -> bool {
    command_line
        .split_whitespace()
        .map(|argument| argument.trim_matches('"'))
        .any(|argument| argument.eq_ignore_ascii_case(expected))
}

fn command_listens_on(command_line: &str, expected_url: &str) -> bool {
    let mut arguments = command_line
        .split_whitespace()
        .map(|argument| argument.trim_matches('"'));
    while let Some(argument) = arguments.next() {
        if argument.eq_ignore_ascii_case("--listen") {
            return arguments
                .next()
                .is_some_and(|url| url.eq_ignore_ascii_case(expected_url));
        }
        if let Some(url) = argument.strip_prefix("--listen=") {
            return url.eq_ignore_ascii_case(expected_url);
        }
    }
    false
}

fn classify_windows_process_snapshot(
    snapshot: WindowsProcessSnapshot,
    shared_url: &str,
) -> CodexDesktopProcessSnapshot {
    let desktop_paths = snapshot
        .desktop_executable_paths
        .iter()
        .map(|path| normalize_windows_executable_path(path))
        .collect::<HashSet<_>>();
    let backend_paths = snapshot
        .private_app_server_executable_paths
        .iter()
        .map(|path| normalize_windows_executable_path(path))
        .collect::<HashSet<_>>();

    let mut classified = CodexDesktopProcessSnapshot::default();
    for process in snapshot.processes {
        let path = normalize_windows_executable_path(&process.executable_path);
        if desktop_paths.contains(&path) {
            classified.desktop_processes.push(process.clone());
        }
        if backend_paths.contains(&path)
            && command_has_argument(&process.command_line, "app-server")
            && !command_listens_on(&process.command_line, shared_url)
        {
            classified.private_app_server_processes.push(process);
        }
    }
    classified
        .desktop_processes
        .sort_by_key(|process| process.process_id);
    classified
        .private_app_server_processes
        .sort_by_key(|process| process.process_id);
    classified
}

fn parse_windows_process_snapshot(
    output: &str,
    shared_url: &str,
) -> Result<CodexDesktopProcessSnapshot, String> {
    let raw = serde_json::from_str::<WindowsProcessSnapshot>(output.trim())
        .map_err(|error| format!("failed to parse the Codex Desktop process snapshot: {error}"))?;
    Ok(classify_windows_process_snapshot(raw, shared_url))
}

fn desktop_process_tree_roots(snapshot: &CodexDesktopProcessSnapshot) -> Vec<WindowsProcessInfo> {
    let desktop_ids = snapshot
        .desktop_processes
        .iter()
        .map(|process| process.process_id)
        .collect::<HashSet<_>>();
    snapshot
        .desktop_processes
        .iter()
        .filter(|process| !desktop_ids.contains(&process.parent_process_id))
        .cloned()
        .collect()
}

fn ensure_ordinary_desktop_launch_allowed(
    snapshot: &CodexDesktopProcessSnapshot,
) -> Result<(), String> {
    if snapshot.private_app_server_processes.is_empty() {
        return Ok(());
    }
    Err(
        "Codex Desktop is still running outside the shared runtime. Use Take over Codex Desktop to review the interruption warning and reconnect it safely."
            .to_string(),
    )
}

fn require_takeover_confirmation(confirmed: bool) -> Result<(), String> {
    if confirmed {
        Ok(())
    } else {
        Err("Codex Desktop takeover requires explicit confirmation".to_string())
    }
}

fn ensure_post_launch_snapshot(snapshot: &CodexDesktopProcessSnapshot) -> Result<(), String> {
    if snapshot.private_app_server_processes.is_empty() {
        Ok(())
    } else {
        Err(
            "Codex Desktop started another private app-server. It was closed to protect the shared task runtime; fully quit Desktop and try again."
                .to_string(),
        )
    }
}

#[cfg(windows)]
fn snapshot_codex_desktop_processes(
    shared_url: &str,
) -> Result<CodexDesktopProcessSnapshot, String> {
    use std::os::windows::process::CommandExt;

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            WINDOWS_CODEX_PROCESS_SNAPSHOT_SCRIPT,
        ])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|error| format!("failed to inspect Codex Desktop processes: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "Windows could not inspect Codex Desktop processes".to_string()
        } else {
            detail
        });
    }
    parse_windows_process_snapshot(&String::from_utf8_lossy(&output.stdout), shared_url)
}

#[cfg(not(windows))]
fn snapshot_codex_desktop_processes(
    _shared_url: &str,
) -> Result<CodexDesktopProcessSnapshot, String> {
    Ok(CodexDesktopProcessSnapshot::default())
}

async fn snapshot_codex_desktop_processes_async(
    shared_url: &str,
) -> Result<CodexDesktopProcessSnapshot, String> {
    let shared_url = shared_url.to_string();
    tokio::task::spawn_blocking(move || snapshot_codex_desktop_processes(&shared_url))
        .await
        .map_err(|error| format!("Codex Desktop process inspection failed: {error}"))?
}

async fn attach_desktop_process_status(
    mut status: CodexSharedRuntimeStatus,
) -> CodexSharedRuntimeStatus {
    match snapshot_codex_desktop_processes_async(&status.url).await {
        Ok(snapshot) => {
            status.desktop_process_ids = snapshot
                .desktop_processes
                .iter()
                .map(|process| process.process_id)
                .collect();
            status.private_app_server_process_ids = snapshot
                .private_app_server_processes
                .iter()
                .map(|process| process.process_id)
                .collect();
        }
        Err(error) => status.desktop_detection_error = Some(error),
    }
    status
}

async fn probe_codex_shared_runtime(url: &str) -> Result<(), String> {
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(2), connect_async(url))
        .await
        .map_err(|_| format!("timed out connecting to {url}"))?
        .map_err(|error| format!("could not connect to {url}: {error}"))?;
    let initialize = serde_json::json!({
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "buzz_shared_runtime_probe",
                "title": "Buzz shared runtime probe",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": { "experimentalApi": true }
        }
    });
    socket
        .send(Message::Text(initialize.to_string().into()))
        .await
        .map_err(|error| format!("failed to initialize {url}: {error}"))?;

    let initialized = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(message) = socket.next().await {
            let message = message.map_err(|error| error.to_string())?;
            let Message::Text(text) = message else {
                continue;
            };
            let payload: serde_json::Value =
                serde_json::from_str(text.as_str()).map_err(|error| error.to_string())?;
            if payload.get("id").and_then(serde_json::Value::as_u64) == Some(1) {
                if let Some(error) = payload.get("error") {
                    return Err(format!("initialize was rejected: {error}"));
                }
                return payload
                    .get("result")
                    .is_some()
                    .then_some(())
                    .ok_or_else(|| "initialize response had no result".to_string());
            }
        }
        Err("connection closed before initialize completed".to_string())
    })
    .await
    .map_err(|_| format!("timed out initializing {url}"))??;
    let _ = socket.close(None).await;
    Ok(initialized)
}

fn read_shared_runtime_log_tail(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let start = length.saturating_sub(SHARED_RUNTIME_ERROR_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    if start > 0 {
        if let Some(first_newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=first_newline);
        }
    }
    let tail = String::from_utf8_lossy(&bytes).trim().to_string();
    (!tail.is_empty()).then_some(tail)
}

fn append_shared_runtime_log_detail(error: String, log_path: &Path) -> String {
    let Some(tail) = read_shared_runtime_log_tail(log_path) else {
        return error;
    };
    format!(
        "{error}\n\nCodex runtime log ({}):\n{tail}",
        log_path.display()
    )
}

fn shared_runtime_failure_detail(app: &AppHandle, error: String) -> String {
    let Ok(base_dir) = managed_agents_base_dir(app) else {
        return error;
    };
    append_shared_runtime_log_detail(
        error,
        &base_dir
            .join("logs")
            .join("codex-shared-runtime.stderr.log"),
    )
}

pub async fn codex_shared_runtime_status(
    app: &AppHandle,
) -> Result<CodexSharedRuntimeStatus, String> {
    let config = load_shared_runtime_config(app)?;
    let url = codex_shared_app_server_url()?;
    if !config.enabled {
        return Ok(attach_desktop_process_status(CodexSharedRuntimeStatus {
            enabled: false,
            state: CodexSharedRuntimeState::SetupRequired,
            url,
            detail: None,
            desktop_process_ids: Vec::new(),
            private_app_server_process_ids: Vec::new(),
            desktop_detection_error: None,
        })
        .await);
    }
    let status = match probe_codex_shared_runtime(&url).await {
        Ok(()) => CodexSharedRuntimeStatus {
            enabled: true,
            state: CodexSharedRuntimeState::Ready,
            url,
            detail: None,
            desktop_process_ids: Vec::new(),
            private_app_server_process_ids: Vec::new(),
            desktop_detection_error: None,
        },
        Err(error) => CodexSharedRuntimeStatus {
            enabled: true,
            state: CodexSharedRuntimeState::Unavailable,
            url,
            detail: Some(shared_runtime_failure_detail(app, error)),
            desktop_process_ids: Vec::new(),
            private_app_server_process_ids: Vec::new(),
            desktop_detection_error: None,
        },
    };
    Ok(attach_desktop_process_status(status).await)
}

#[cfg(windows)]
fn is_usable_codex_app_server_executable(path: &Path) -> bool {
    path.is_file()
        && path
            .parent()
            .map(|parent| parent.join("codex-code-mode-host.exe").is_file())
            .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_usable_codex_app_server_executable(path: &Path) -> bool {
    path.is_file()
        && path
            .parent()
            .map(|parent| parent.join("codex-code-mode-host").is_file())
            .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn macos_codex_app_server_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.push(PathBuf::from(
        "/Applications/ChatGPT.app/Contents/Resources/codex",
    ));
    candidates.push(PathBuf::from(
        "/Applications/Codex.app/Contents/Resources/codex",
    ));
    if let Some(home) = dirs::home_dir() {
        candidates.push(
            home.join("Applications")
                .join("ChatGPT.app")
                .join("Contents")
                .join("Resources")
                .join("codex"),
        );
        candidates.push(
            home.join("Applications")
                .join("Codex.app")
                .join("Contents")
                .join("Resources")
                .join("codex"),
        );
        candidates.push(home.join(".cargo").join("bin").join("codex"));
        candidates.push(home.join(".local").join("bin").join("codex"));
        candidates.push(home.join(".codex").join("bin").join("codex"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/codex"));
    candidates.push(PathBuf::from("/usr/local/bin/codex"));
    candidates
}

fn path_codex_app_server_candidates(executable: &str) -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|directory| directory.join(executable))
                .collect()
        })
        .unwrap_or_default()
}

fn codex_app_server_candidates(executable: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    #[cfg(target_os = "macos")]
    candidates.extend(macos_codex_app_server_candidates());

    candidates.extend(path_codex_app_server_candidates(executable));
    candidates
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

fn find_codex_app_server_executable() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os(SHARED_RUNTIME_COMMAND_ENV) {
        let path = PathBuf::from(path);
        if is_usable_codex_app_server_executable(&path) {
            return Ok(path);
        }
        return Err(format!(
            "{SHARED_RUNTIME_COMMAND_ENV} does not point to a complete Codex runtime: {}",
            path.display()
        ));
    }

    #[cfg(windows)]
    {
        // Codex Desktop materializes an executable runtime bundle here. Requiring
        // the matching sidecar avoids selecting a partial update while it is
        // still being installed.
        if let Some(local_data) = dirs::data_local_dir() {
            let bin_dir = local_data.join("OpenAI").join("Codex").join("bin");
            let mut candidates = fs::read_dir(&bin_dir)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path().join("codex.exe"))
                .filter(|path| is_usable_codex_app_server_executable(path))
                .collect::<Vec<_>>();
            candidates.sort_by_key(|path| {
                path.metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
            });
            if let Some(path) = candidates.pop() {
                return Ok(path);
            }
        }
    }

    let executable = if cfg!(windows) { "codex.exe" } else { "codex" };
    if let Some(path) = codex_app_server_candidates(executable)
        .into_iter()
        .find(|candidate| is_usable_codex_app_server_executable(candidate))
    {
        return Ok(path);
    }

    Err(
        "A complete Codex runtime was not found. Open Codex Desktop normally once to finish runtime setup, then retry."
            .to_string(),
    )
}

fn codex_shared_runtime_args(url: &str) -> Vec<String> {
    vec![
        "-c".to_string(),
        CODEX_CODE_MODE_HOST_FLAG.to_string(),
        "app-server".to_string(),
        "--listen".to_string(),
        url.to_string(),
    ]
}

fn spawn_codex_shared_runtime(app: &AppHandle, url: &str) -> Result<(), String> {
    let executable = find_codex_app_server_executable()?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let base_dir = managed_agents_base_dir(app)?;
        let logs_dir = base_dir.join("logs");
        fs::create_dir_all(&logs_dir)
            .map_err(|error| format!("failed to create {}: {error}", logs_dir.display()))?;
        let launcher_path = base_dir.join("codex-shared-runtime-launcher.ps1");
        fs::write(&launcher_path, WINDOWS_CODEX_SHARED_RUNTIME_LAUNCHER_SCRIPT)
            .map_err(|error| format!("failed to write {}: {error}", launcher_path.display()))?;
        let stdout_log = logs_dir.join("codex-shared-runtime.stdout.log");
        let stderr_log = logs_dir.join("codex-shared-runtime.stderr.log");
        fs::write(&stdout_log, [])
            .map_err(|error| format!("failed to reset {}: {error}", stdout_log.display()))?;
        fs::write(&stderr_log, [])
            .map_err(|error| format!("failed to reset {}: {error}", stderr_log.display()))?;
        let launcher_config_path = base_dir.join("codex-shared-runtime-launcher.json");
        let launcher_config = serde_json::to_vec_pretty(&serde_json::json!({
            "executable": executable,
            "url": url,
            "stdout_log": stdout_log,
            "stderr_log": stderr_log,
        }))
        .map_err(|error| format!("failed to serialize Codex runtime launcher: {error}"))?;
        fs::write(&launcher_config_path, launcher_config).map_err(|error| {
            format!(
                "failed to write {}: {error}",
                launcher_config_path.display()
            )
        })?;

        // WMI owns the transient launcher, so the shared backend survives Buzz
        // updates. Both the launcher and Codex run without console windows.
        let command_line = format!(
            "powershell.exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\" -ConfigPath \"{}\"",
            launcher_path.display(),
            launcher_config_path.display()
        );
        let output = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                WINDOWS_CODEX_SHARED_RUNTIME_WMI_SCRIPT,
            ])
            .env("BUZZ_CODEX_SHARED_RUNTIME_COMMAND_LINE", command_line)
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|error| format!("failed to request Codex runtime start: {error}"))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if detail.is_empty() {
                "Windows could not start the Codex shared runtime".to_string()
            } else {
                detail
            });
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        use std::process::Stdio;

        let logs_dir = managed_agents_base_dir(app)?.join("logs");
        fs::create_dir_all(&logs_dir)
            .map_err(|error| format!("failed to create {}: {error}", logs_dir.display()))?;
        let stdout = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs_dir.join("codex-shared-runtime.stdout.log"))
            .map_err(|error| format!("failed to open Codex runtime log: {error}"))?;
        let stderr = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs_dir.join("codex-shared-runtime.stderr.log"))
            .map_err(|error| format!("failed to open Codex runtime error log: {error}"))?;
        let mut command = Command::new(&executable);
        command
            .args(codex_shared_runtime_args(url))
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        command
            .spawn()
            .map_err(|error| format!("failed to start {}: {error}", executable.display()))?;
        Ok(())
    }
}

pub async fn enable_codex_shared_runtime(
    app: &AppHandle,
) -> Result<CodexSharedRuntimeStatus, String> {
    static START_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let _guard = START_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    save_shared_runtime_config(
        app,
        &CodexSharedRuntimeConfig {
            version: SHARED_RUNTIME_CONFIG_VERSION,
            enabled: true,
        },
    )?;
    let url = codex_shared_app_server_url()?;
    if probe_codex_shared_runtime(&url).await.is_err() {
        spawn_codex_shared_runtime(app, &url)?;
        let mut last_error = None;
        for _ in 0..50 {
            match probe_codex_shared_runtime(&url).await {
                Ok(()) => {
                    last_error = None;
                    break;
                }
                Err(error) => last_error = Some(error),
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if let Some(error) = last_error {
            return Ok(attach_desktop_process_status(CodexSharedRuntimeStatus {
                enabled: true,
                state: CodexSharedRuntimeState::Unavailable,
                url,
                detail: Some(shared_runtime_failure_detail(app, error)),
                desktop_process_ids: Vec::new(),
                private_app_server_process_ids: Vec::new(),
                desktop_detection_error: None,
            })
            .await);
        }
    }
    Ok(attach_desktop_process_status(CodexSharedRuntimeStatus {
        enabled: true,
        state: CodexSharedRuntimeState::Ready,
        url,
        detail: None,
        desktop_process_ids: Vec::new(),
        private_app_server_process_ids: Vec::new(),
        desktop_detection_error: None,
    })
    .await)
}

pub async fn restore_codex_runtime(app: AppHandle) {
    if load_shared_runtime_config(&app)
        .map(|config| config.enabled)
        .unwrap_or(false)
    {
        if let Err(error) = enable_codex_shared_runtime(&app).await {
            eprintln!("buzz-desktop: failed to restore Codex shared runtime: {error}");
        }
    }
}

#[cfg(windows)]
fn launch_codex_desktop_shared_unchecked(url: &str) -> Result<WindowsProcessInfo, String> {
    use std::os::windows::process::CommandExt;

    const SCRIPT: &str = r#"
$ErrorActionPreference='Stop'
$env:CODEX_APP_SERVER_WS_URL=$env:BUZZ_CODEX_DESKTOP_SHARED_URL
$package=Get-AppxPackage | Where-Object { $_.Name -in @('OpenAI.Codex','OpenAI.CodexBeta') } | Sort-Object @{Expression={if ($_.Name -eq 'OpenAI.Codex') {0} else {1}};Ascending=$true},@{Expression={$_.Version};Descending=$true} | Select-Object -First 1
if (-not $package) { throw 'Codex Desktop is not installed' }
$application=@((Get-AppxPackageManifest -Package $package).Package.Applications.Application)[0]
$exe=[IO.Path]::GetFullPath((Join-Path $package.InstallLocation ([string]$application.Executable)))
$process=Start-Process -FilePath $exe -PassThru
[pscustomobject]@{
  process_id=[uint32]$process.Id
  parent_process_id=0
  executable_path=$exe
  command_line=''
} | ConvertTo-Json -Compress
"#;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("BUZZ_CODEX_DESKTOP_SHARED_URL", url)
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|error| format!("failed to launch Codex Desktop: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "Windows could not launch Codex Desktop".to_string()
        } else {
            detail
        });
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("could not read the launched Codex Desktop process: {error}"))
}

#[cfg(windows)]
fn terminate_verified_windows_process(
    process: &WindowsProcessInfo,
    include_tree: bool,
) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            WINDOWS_TERMINATE_VERIFIED_PROCESS_SCRIPT,
        ])
        .env("BUZZ_CODEX_TARGET_PID", process.process_id.to_string())
        .env("BUZZ_CODEX_TARGET_EXE", &process.executable_path)
        .env(
            "BUZZ_CODEX_TARGET_TREE",
            if include_tree { "1" } else { "0" },
        )
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|error| format!("failed to close Codex Desktop: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if detail.is_empty() {
            format!(
                "Windows could not close Codex Desktop PID {}",
                process.process_id
            )
        } else {
            detail
        })
    }
}

#[cfg(windows)]
pub fn launch_codex_desktop_shared() -> Result<(), String> {
    let url = codex_shared_app_server_url()?;
    let snapshot = snapshot_codex_desktop_processes(&url)?;
    ensure_ordinary_desktop_launch_allowed(&snapshot)?;
    launch_codex_desktop_shared_unchecked(&url).map(|_| ())
}

#[cfg(not(windows))]
pub fn launch_codex_desktop_shared() -> Result<(), String> {
    Err("Automatic Codex Desktop relaunch is currently available on Windows only.".to_string())
}

/// Close a conflicting packaged Codex Desktop runtime and reconnect Desktop to
/// Buzz's long-lived shared app-server after explicit user confirmation.
pub async fn take_over_codex_desktop_shared(
    app: &AppHandle,
    confirmed: bool,
) -> Result<CodexSharedRuntimeStatus, String> {
    require_takeover_confirmation(confirmed)?;

    #[cfg(not(windows))]
    {
        let _ = app;
        return Err(
            "Automatic Codex Desktop takeover is currently available on Windows only.".to_string(),
        );
    }

    #[cfg(windows)]
    {
        let url = codex_shared_app_server_url()?;
        probe_codex_shared_runtime(&url).await.map_err(|error| {
            format!(
                "The shared Codex runtime is not ready at {url}: {error}. Start it before taking over Desktop."
            )
        })?;
        let initial = snapshot_codex_desktop_processes_async(&url).await?;
        if initial.private_app_server_processes.is_empty() {
            return codex_shared_runtime_status(app).await;
        }

        let roots = desktop_process_tree_roots(&initial);
        tokio::task::spawn_blocking(move || {
            for process in &roots {
                terminate_verified_windows_process(process, true)?;
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|error| format!("Codex Desktop close task failed: {error}"))??;

        let remaining = snapshot_codex_desktop_processes_async(&url).await?;
        let orphan_backends = remaining.private_app_server_processes.clone();
        tokio::task::spawn_blocking(move || {
            for process in &orphan_backends {
                terminate_verified_windows_process(process, false)?;
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|error| format!("Codex private backend close task failed: {error}"))??;

        let original_target_ids = initial
            .desktop_processes
            .iter()
            .chain(initial.private_app_server_processes.iter())
            .map(|process| process.process_id)
            .collect::<HashSet<_>>();
        let mut targets_still_running = true;
        for _ in 0..50 {
            let snapshot = snapshot_codex_desktop_processes_async(&url).await?;
            targets_still_running = snapshot
                .desktop_processes
                .iter()
                .chain(snapshot.private_app_server_processes.iter())
                .any(|process| original_target_ids.contains(&process.process_id));
            if !targets_still_running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if targets_still_running {
            return Err(
                "Codex Desktop did not fully exit within 10 seconds. Close it manually, then try again."
                    .to_string(),
            );
        }

        probe_codex_shared_runtime(&url).await.map_err(|error| {
            format!(
                "The shared Codex runtime was lost while Desktop closed: {error}. Start it again before reconnecting Desktop."
            )
        })?;

        let launch_url = url.clone();
        let launched =
            tokio::task::spawn_blocking(move || launch_codex_desktop_shared_unchecked(&launch_url))
                .await
                .map_err(|error| format!("Codex Desktop launch task failed: {error}"))??;

        let mut stable_desktop_checks = 0u8;
        for _ in 0..50 {
            let snapshot = match snapshot_codex_desktop_processes_async(&url).await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    let cleanup = launched.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        terminate_verified_windows_process(&cleanup, true)
                    })
                    .await;
                    return Err(format!(
                        "Codex Desktop reopened, but Buzz could not verify its runtime: {error}"
                    ));
                }
            };
            if let Err(error) = ensure_post_launch_snapshot(&snapshot) {
                let roots = desktop_process_tree_roots(&snapshot);
                let private_backends = snapshot.private_app_server_processes.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    for process in &roots {
                        let _ = terminate_verified_windows_process(process, true);
                    }
                    for process in &private_backends {
                        let _ = terminate_verified_windows_process(process, false);
                    }
                })
                .await;
                return Err(error);
            }
            if snapshot.desktop_processes.is_empty() {
                stable_desktop_checks = 0;
            } else {
                stable_desktop_checks += 1;
                // A private backend is normally spawned shortly after the
                // Electron process. Observe three clean seconds before
                // claiming that Desktop stayed on the shared runtime.
                if stable_desktop_checks >= 15 {
                    return codex_shared_runtime_status(app).await;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        let cleanup = launched;
        let _ =
            tokio::task::spawn_blocking(move || terminate_verified_windows_process(&cleanup, true))
                .await;
        Err(
            "Codex Desktop did not remain open after reconnecting. Buzz closed the launch attempt; try again after checking the Desktop installation."
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::codex_tasks::DEFAULT_CODEX_SHARED_APP_SERVER_URL;
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_shared_runtime_requires_matching_code_mode_host() {
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("codex.exe");
        fs::write(&codex, []).unwrap();

        assert!(!is_usable_codex_app_server_executable(&codex));

        fs::write(dir.path().join("codex-code-mode-host.exe"), []).unwrap();
        assert!(is_usable_codex_app_server_executable(&codex));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_shared_runtime_requires_matching_code_mode_host() {
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("codex");
        fs::write(&codex, []).unwrap();

        assert!(!is_usable_codex_app_server_executable(&codex));

        fs::write(dir.path().join("codex-code-mode-host"), []).unwrap();
        assert!(is_usable_codex_app_server_executable(&codex));
    }

    #[test]
    fn shared_runtime_launch_args_enable_code_mode_host() {
        assert_eq!(
            codex_shared_runtime_args(DEFAULT_CODEX_SHARED_APP_SERVER_URL),
            vec![
                "-c",
                CODEX_CODE_MODE_HOST_FLAG,
                "app-server",
                "--listen",
                DEFAULT_CODEX_SHARED_APP_SERVER_URL,
            ]
        );
        #[cfg(windows)]
        assert!(WINDOWS_CODEX_SHARED_RUNTIME_LAUNCHER_SCRIPT.contains(CODEX_CODE_MODE_HOST_FLAG));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_candidates_include_desktop_and_homebrew_runtime_locations() {
        let candidates = macos_codex_app_server_candidates()
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(candidates
            .iter()
            .any(|path| path == "/Applications/ChatGPT.app/Contents/Resources/codex"));
        assert!(candidates
            .iter()
            .any(|path| path == "/opt/homebrew/bin/codex"));
        assert!(candidates.iter().any(|path| path == "/usr/local/bin/codex"));
    }

    #[test]
    fn parses_zero_one_and_multiple_windows_processes() {
        let empty = r#"{
            "desktop_executable_paths": [],
            "private_app_server_executable_paths": [],
            "processes": []
        }"#;
        assert_eq!(
            parse_windows_process_snapshot(empty, DEFAULT_CODEX_SHARED_APP_SERVER_URL).unwrap(),
            CodexDesktopProcessSnapshot::default()
        );

        let populated = r#"{
            "desktop_executable_paths": ["C:\\Program Files\\WindowsApps\\OpenAI.Codex_1\\app\\ChatGPT.exe"],
            "private_app_server_executable_paths": ["C:\\Program Files\\WindowsApps\\OpenAI.Codex_1\\app\\resources\\codex.exe"],
            "processes": [
                {"process_id":10,"parent_process_id":1,"executable_path":"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1\\app\\ChatGPT.exe","command_line":"ChatGPT.exe"},
                {"process_id":11,"parent_process_id":10,"executable_path":"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1\\app\\ChatGPT.exe","command_line":"ChatGPT.exe --type=renderer"},
                {"process_id":12,"parent_process_id":10,"executable_path":"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1\\app\\resources\\codex.exe","command_line":"codex.exe app-server"}
            ]
        }"#;
        let snapshot =
            parse_windows_process_snapshot(populated, DEFAULT_CODEX_SHARED_APP_SERVER_URL).unwrap();
        assert_eq!(
            snapshot
                .desktop_processes
                .iter()
                .map(|process| process.process_id)
                .collect::<Vec<_>>(),
            vec![10, 11]
        );
        assert_eq!(
            snapshot
                .private_app_server_processes
                .iter()
                .map(|process| process.process_id)
                .collect::<Vec<_>>(),
            vec![12]
        );
        assert_eq!(
            desktop_process_tree_roots(&snapshot)
                .iter()
                .map(|process| process.process_id)
                .collect::<Vec<_>>(),
            vec![10]
        );
    }

    #[test]
    fn distinguishes_local_shared_runtime_from_packaged_private_backend() {
        let raw = WindowsProcessSnapshot {
            desktop_executable_paths: vec![
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1\app\ChatGPT.exe".to_string(),
            ],
            private_app_server_executable_paths: vec![
                r"C:\Program Files\WindowsApps\OpenAI.Codex_1\app\resources\codex.exe".to_string(),
            ],
            processes: vec![
                WindowsProcessInfo {
                    process_id: 20,
                    parent_process_id: 1,
                    executable_path:
                        r"C:\Users\tester\AppData\Local\OpenAI\Codex\bin\abc\codex.exe".to_string(),
                    command_line: format!(
                        "codex.exe app-server --listen {}",
                        DEFAULT_CODEX_SHARED_APP_SERVER_URL
                    ),
                },
                WindowsProcessInfo {
                    process_id: 21,
                    parent_process_id: 30,
                    executable_path:
                        r"C:\Program Files\WindowsApps\OpenAI.Codex_1\app\resources\codex.exe"
                            .to_string(),
                    command_line: "codex.exe app-server --analytics-default-enabled".to_string(),
                },
                WindowsProcessInfo {
                    process_id: 22,
                    parent_process_id: 30,
                    executable_path:
                        r"C:\Program Files\WindowsApps\OpenAI.Codex_1\app\resources\codex.exe"
                            .to_string(),
                    command_line: format!(
                        "codex.exe app-server --listen \"{}\"",
                        DEFAULT_CODEX_SHARED_APP_SERVER_URL
                    ),
                },
            ],
        };
        let snapshot = classify_windows_process_snapshot(raw, DEFAULT_CODEX_SHARED_APP_SERVER_URL);
        assert_eq!(
            snapshot
                .private_app_server_processes
                .iter()
                .map(|process| process.process_id)
                .collect::<Vec<_>>(),
            vec![21]
        );
        assert!(!snapshot
            .private_app_server_processes
            .iter()
            .any(|process| process.process_id == 20));
    }

    #[test]
    fn ordinary_launch_and_post_launch_verification_refuse_private_backends() {
        let conflict = CodexDesktopProcessSnapshot {
            desktop_processes: Vec::new(),
            private_app_server_processes: vec![WindowsProcessInfo {
                process_id: 42,
                parent_process_id: 1,
                executable_path:
                    r"C:\Program Files\WindowsApps\OpenAI.Codex_1\app\resources\codex.exe"
                        .to_string(),
                command_line: "codex.exe app-server".to_string(),
            }],
        };
        assert!(ensure_ordinary_desktop_launch_allowed(&conflict).is_err());
        assert!(ensure_post_launch_snapshot(&conflict).is_err());
        assert!(
            ensure_ordinary_desktop_launch_allowed(&CodexDesktopProcessSnapshot::default()).is_ok()
        );
        assert!(ensure_post_launch_snapshot(&CodexDesktopProcessSnapshot::default()).is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn windows_shared_runtime_launch_is_hidden_and_logged() {
        assert!(WINDOWS_CODEX_SHARED_RUNTIME_WMI_SCRIPT.contains("ShowWindow"));
        assert!(WINDOWS_CODEX_SHARED_RUNTIME_LAUNCHER_SCRIPT.contains("-WindowStyle Hidden"));
        assert!(WINDOWS_CODEX_SHARED_RUNTIME_LAUNCHER_SCRIPT.contains("-RedirectStandardOutput"));
        assert!(WINDOWS_CODEX_SHARED_RUNTIME_LAUNCHER_SCRIPT.contains("-RedirectStandardError"));
    }

    #[test]
    fn unavailable_status_includes_a_bounded_runtime_log_tail() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("codex-shared-runtime.stderr.log");
        fs::write(
            &log_path,
            format!(
                "{}\ncurrent startup failure",
                "old diagnostics ".repeat(400)
            ),
        )
        .unwrap();

        let detail = append_shared_runtime_log_detail("runtime unavailable".to_string(), &log_path);

        assert!(detail.contains("runtime unavailable"));
        assert!(detail.contains("current startup failure"));
        assert!(!detail.contains("old diagnostics"));
        assert!(detail.contains(log_path.to_string_lossy().as_ref()));
    }

    #[test]
    fn takeover_requires_confirmation_and_termination_rechecks_exact_paths() {
        assert!(require_takeover_confirmation(false).is_err());
        assert!(require_takeover_confirmation(true).is_ok());
        assert!(WINDOWS_TERMINATE_VERIFIED_PROCESS_SCRIPT.contains("ExecutablePath"));
        assert!(WINDOWS_TERMINATE_VERIFIED_PROCESS_SCRIPT.contains("BUZZ_CODEX_TARGET_EXE"));
        assert!(WINDOWS_TERMINATE_VERIFIED_PROCESS_SCRIPT.contains("/PID"));
        assert!(!WINDOWS_TERMINATE_VERIFIED_PROCESS_SCRIPT.contains("$_.Name"));
        assert!(!WINDOWS_TERMINATE_VERIFIED_PROCESS_SCRIPT.contains("51919"));
    }
}
