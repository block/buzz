use std::{
    collections::HashSet,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
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
const WINDOWS_CODEX_RUNTIME_COMPANIONS: [&str; 3] = [
    "codex-code-mode-host.exe",
    "codex-command-runner.exe",
    "codex-windows-sandbox-setup.exe",
];
#[cfg(windows)]
const WINDOWS_CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const WINDOWS_CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[cfg(windows)]
fn windows_shared_runtime_creation_flags() -> u32 {
    WINDOWS_CREATE_NO_WINDOW | WINDOWS_CREATE_NEW_PROCESS_GROUP
}

fn detach_shared_runtime(child: Child) -> u32 {
    let process_id = child.id();
    drop(child);
    process_id
}

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
}

#[cfg(not(windows))]
fn is_usable_codex_app_server_executable(path: &Path) -> bool {
    path.is_file()
}

fn codex_code_mode_host_available(path: &Path) -> bool {
    path.parent()
        .map(|parent| {
            #[cfg(windows)]
            {
                parent.join("codex-code-mode-host.exe").is_file()
            }
            #[cfg(not(windows))]
            {
                parent.join("codex-code-mode-host").is_file()
            }
        })
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

#[cfg(windows)]
fn windows_appx_codex_candidates() -> Vec<PathBuf> {
    // Codex installed from the Microsoft Store keeps its runtime under the
    // package install directory rather than %LOCALAPPDATA%\\OpenAI\\Codex\\bin.
    // Query AppX instead of guessing the versioned WindowsApps directory.
    let script = r#"
$ErrorActionPreference='SilentlyContinue'
Get-AppxPackage -Name OpenAI.Codex,OpenAI.CodexBeta |
  ForEach-Object { Join-Path $_.InstallLocation 'app\resources\codex.exe' }
"#;
    Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .output()
        .ok()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

fn codex_app_server_candidates(executable: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    #[cfg(windows)]
    candidates.extend(windows_appx_codex_candidates());

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
                (
                    codex_code_mode_host_available(path),
                    path.metadata()
                        .and_then(|metadata| metadata.modified())
                        .ok(),
                )
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

#[cfg(windows)]
fn codex_runtime_bundle_files(executable: &Path) -> Result<Vec<(PathBuf, &'static str)>, String> {
    let parent = executable.parent().ok_or_else(|| {
        format!(
            "Codex runtime executable has no parent directory: {}",
            executable.display()
        )
    })?;
    let mut files = vec![(executable.to_path_buf(), "codex.exe")];
    files.extend(
        WINDOWS_CODEX_RUNTIME_COMPANIONS
            .into_iter()
            .filter_map(|name| {
                let path = parent.join(name);
                path.is_file().then_some((path, name))
            }),
    );
    Ok(files)
}

#[cfg(windows)]
fn codex_runtime_bundle_key(executable: &Path) -> Result<String, String> {
    let mut hasher = DefaultHasher::new();
    for (path, destination_name) in codex_runtime_bundle_files(executable)? {
        let metadata = path
            .metadata()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        destination_name.hash(&mut hasher);
        metadata.len().hash(&mut hasher);
        metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .hash(&mut hasher);
    }
    Ok(format!("{:016x}", hasher.finish()))
}

#[cfg(windows)]
fn codex_runtime_copy_matches(source_executable: &Path, destination: &Path) -> bool {
    let Ok(files) = codex_runtime_bundle_files(source_executable) else {
        return false;
    };
    files.into_iter().all(|(source, destination_name)| {
        let Ok(source_metadata) = source.metadata() else {
            return false;
        };
        destination
            .join(destination_name)
            .metadata()
            .is_ok_and(|metadata| metadata.len() == source_metadata.len())
    })
}

#[cfg(windows)]
fn prepare_managed_codex_runtime(
    source_executable: &Path,
    cache_root: &Path,
) -> Result<PathBuf, String> {
    let key = codex_runtime_bundle_key(source_executable)?;
    fs::create_dir_all(cache_root)
        .map_err(|error| format!("failed to create {}: {error}", cache_root.display()))?;
    let destination = cache_root.join(&key);
    let managed_executable = destination.join("codex.exe");
    if codex_runtime_copy_matches(source_executable, &destination) {
        return Ok(managed_executable);
    }
    if destination.exists() {
        return Err(format!(
            "Buzz's cached Codex runtime is incomplete: {}. Remove that directory while the shared runtime is stopped, then retry.",
            destination.display()
        ));
    }

    let staging = cache_root.join(format!(
        ".{key}.{}.{}.staging",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir(&staging)
        .map_err(|error| format!("failed to create {}: {error}", staging.display()))?;
    let copy_result = (|| {
        for (source, destination_name) in codex_runtime_bundle_files(source_executable)? {
            let target = staging.join(destination_name);
            fs::copy(&source, &target).map_err(|error| {
                format!(
                    "failed to copy {} to {}: {error}",
                    source.display(),
                    target.display()
                )
            })?;
        }
        if !codex_runtime_copy_matches(source_executable, &staging) {
            return Err("Buzz's copied Codex runtime failed verification".to_string());
        }
        fs::rename(&staging, &destination).map_err(|error| {
            format!(
                "failed to activate Codex runtime {}: {error}",
                destination.display()
            )
        })?;
        Ok(managed_executable)
    })();
    if copy_result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    copy_result
}

#[cfg(windows)]
fn managed_codex_app_server_executable(
    app: &AppHandle,
    source_executable: &Path,
) -> Result<PathBuf, String> {
    prepare_managed_codex_runtime(
        source_executable,
        &managed_agents_base_dir(app)?.join("codex-runtime"),
    )
}

fn codex_shared_runtime_args(url: &str, code_mode_host_available: bool) -> Vec<String> {
    let mut args = Vec::with_capacity(if code_mode_host_available { 5 } else { 3 });
    if code_mode_host_available {
        args.extend(["-c".to_string(), CODEX_CODE_MODE_HOST_FLAG.to_string()]);
    }
    args.extend([
        "app-server".to_string(),
        "--listen".to_string(),
        url.to_string(),
    ]);
    args
}

fn append_shared_runtime_launch_log(
    log_path: &Path,
    executable: &Path,
    url: &str,
    code_mode_host_available: bool,
    process_id: Option<u32>,
) -> Result<(), String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| format!("failed to open {}: {error}", log_path.display()))?;
    writeln!(
        log,
        "buzz shared runtime launch: timestamp={timestamp} method=direct pid={} executable={} code_mode_host={} url={url}",
        process_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "pending".to_string()),
        executable.display(),
        if code_mode_host_available { "enabled" } else { "unavailable" },
    )
    .map_err(|error| format!("failed to write {}: {error}", log_path.display()))
}

fn spawn_codex_shared_runtime(app: &AppHandle, url: &str) -> Result<u32, String> {
    let source_executable = find_codex_app_server_executable()?;
    #[cfg(windows)]
    let executable = managed_codex_app_server_executable(app, &source_executable)?;
    #[cfg(not(windows))]
    let executable = source_executable;
    let code_mode_host_available = codex_code_mode_host_available(&executable);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let logs_dir = managed_agents_base_dir(app)?.join("logs");
        fs::create_dir_all(&logs_dir)
            .map_err(|error| format!("failed to create {}: {error}", logs_dir.display()))?;
        let stdout_log = logs_dir.join("codex-shared-runtime.stdout.log");
        let stderr_log = logs_dir.join("codex-shared-runtime.stderr.log");
        let working_directory = executable
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        append_shared_runtime_launch_log(
            &stderr_log,
            &executable,
            url,
            code_mode_host_available,
            None,
        )?;
        let stdout = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stdout_log)
            .map_err(|error| format!("failed to open {}: {error}", stdout_log.display()))?;
        let stderr = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&stderr_log)
            .map_err(|error| format!("failed to open {}: {error}", stderr_log.display()))?;
        let child = Command::new(&executable)
            .args(codex_shared_runtime_args(url, code_mode_host_available))
            .current_dir(working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .creation_flags(windows_shared_runtime_creation_flags())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                let mut log = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&stderr_log)
                    .map_err(|log_error| {
                        format!(
                            "failed to start {}: {error}; failed to open {}: {log_error}",
                            executable.display(),
                            stderr_log.display()
                        )
                    })?;
                writeln!(
                    log,
                    "buzz shared runtime launch failed: executable={} error={error}",
                    executable.display()
                )
                .map_err(|log_error| {
                    format!(
                        "failed to start {}: {error}; failed to write {}: {log_error}",
                        executable.display(),
                        stderr_log.display()
                    )
                })?;
                return Err(format!("failed to start {}: {error}", executable.display()));
            }
        };
        let process_id = child.id();
        if let Err(error) = append_shared_runtime_launch_log(
            &stderr_log,
            &executable,
            url,
            code_mode_host_available,
            Some(process_id),
        ) {
            let _ = super::terminate_process(process_id);
            let _ = child.wait();
            return Err(error);
        }
        // The shared app-server is a computer service used by both Buzz and
        // Codex Desktop. Dropping Child intentionally detaches it from the Buzz
        // window lifecycle; the versioned copy above prevents it from locking
        // the source runtime while Codex updates.
        Ok(detach_shared_runtime(child))
    }

    #[cfg(not(windows))]
    {
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
            .args(codex_shared_runtime_args(
                url,
                codex_code_mode_host_available(&executable),
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        let child = command
            .spawn()
            .map_err(|error| format!("failed to start {}: {error}", executable.display()))?;
        Ok(detach_shared_runtime(child))
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
        let spawned_pid = spawn_codex_shared_runtime(app, &url)?;
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
            let _ = super::terminate_process(spawned_pid);
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
        // Codex Desktop may still be updating/materializing its runtime bundle
        // during Buzz startup. Retry briefly so a transient missing executable
        // does not permanently leave the shared runtime unavailable until the
        // user manually opens the setup panel.
        let mut last_error = None;
        for attempt in 0..15 {
            match enable_codex_shared_runtime(&app).await {
                Ok(status) if status.state == CodexSharedRuntimeState::Ready => return,
                Ok(status) => {
                    last_error = status.detail;
                }
                Err(error) => last_error = Some(error),
            }
            if attempt < 14 {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        if let Some(error) = last_error {
            eprintln!(
                "buzz-desktop: failed to restore Codex shared runtime after retries: {error}"
            );
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
    fn windows_shared_runtime_accepts_codex_without_code_mode_host() {
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("codex.exe");
        fs::write(&codex, []).unwrap();

        assert!(is_usable_codex_app_server_executable(&codex));
        assert!(!codex_code_mode_host_available(&codex));

        fs::write(dir.path().join("codex-code-mode-host.exe"), []).unwrap();
        assert!(is_usable_codex_app_server_executable(&codex));
        assert!(codex_code_mode_host_available(&codex));
    }

    #[cfg(windows)]
    #[test]
    fn windows_shared_runtime_uses_an_immutable_versioned_copy() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let cache = dir.path().join("cache");
        fs::create_dir(&source).unwrap();
        let codex = source.join("codex.exe");
        fs::write(&codex, b"codex-v1").unwrap();
        fs::write(source.join("codex-code-mode-host.exe"), b"host-v1").unwrap();
        fs::write(source.join("codex-command-runner.exe"), b"runner-v1").unwrap();
        fs::write(
            source.join("codex-windows-sandbox-setup.exe"),
            b"sandbox-v1",
        )
        .unwrap();

        let first = prepare_managed_codex_runtime(&codex, &cache).unwrap();
        let reused = prepare_managed_codex_runtime(&codex, &cache).unwrap();
        assert_eq!(reused, first);
        assert_eq!(fs::read(&first).unwrap(), b"codex-v1");
        assert_eq!(
            fs::read(first.parent().unwrap().join("codex-code-mode-host.exe")).unwrap(),
            b"host-v1"
        );

        fs::write(&codex, b"codex-v2-with-a-new-size").unwrap();
        let second = prepare_managed_codex_runtime(&codex, &cache).unwrap();
        assert_ne!(second, first);
        assert_eq!(fs::read(&first).unwrap(), b"codex-v1");
        assert_eq!(fs::read(&second).unwrap(), b"codex-v2-with-a-new-size");
    }

    #[cfg(windows)]
    #[test]
    fn detached_shared_runtime_survives_dropping_the_buzz_child_handle() {
        use std::{thread, time::Duration};

        use std::os::windows::process::CommandExt;

        let child = Command::new("cmd.exe")
            .args(["/d", "/c", "ping -n 30 127.0.0.1 >NUL"])
            .creation_flags(WINDOWS_CREATE_NO_WINDOW | WINDOWS_CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .unwrap();
        let process_id = detach_shared_runtime(child);
        thread::sleep(Duration::from_millis(100));

        let survived = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "if (Get-Process -Id $env:BUZZ_TEST_PID -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }",
            ])
            .env("BUZZ_TEST_PID", process_id.to_string())
            .creation_flags(WINDOWS_CREATE_NO_WINDOW)
            .status()
            .is_ok_and(|status| status.success());
        let _ = super::super::terminate_process(process_id);
        assert!(survived, "shared runtime exited when Buzz dropped Child");
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_shared_runtime_accepts_codex_without_code_mode_host() {
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("codex");
        fs::write(&codex, []).unwrap();

        assert!(is_usable_codex_app_server_executable(&codex));
        assert!(!codex_code_mode_host_available(&codex));

        fs::write(dir.path().join("codex-code-mode-host"), []).unwrap();
        assert!(is_usable_codex_app_server_executable(&codex));
        assert!(codex_code_mode_host_available(&codex));
    }

    #[test]
    fn shared_runtime_launch_args_enable_code_mode_host() {
        assert_eq!(
            codex_shared_runtime_args(DEFAULT_CODEX_SHARED_APP_SERVER_URL, true),
            vec![
                "-c",
                CODEX_CODE_MODE_HOST_FLAG,
                "app-server",
                "--listen",
                DEFAULT_CODEX_SHARED_APP_SERVER_URL,
            ]
        );
        assert_eq!(
            codex_shared_runtime_args(DEFAULT_CODEX_SHARED_APP_SERVER_URL, false),
            vec![
                "app-server",
                "--listen",
                DEFAULT_CODEX_SHARED_APP_SERVER_URL,
            ]
        );
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
        let flags = windows_shared_runtime_creation_flags();
        assert_ne!(flags & WINDOWS_CREATE_NO_WINDOW, 0);
        assert_ne!(flags & WINDOWS_CREATE_NEW_PROCESS_GROUP, 0);
    }

    #[test]
    fn shared_runtime_launch_log_records_method_executable_and_pid() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("codex-shared-runtime.stderr.log");
        let executable = dir
            .path()
            .join(if cfg!(windows) { "codex.exe" } else { "codex" });

        append_shared_runtime_launch_log(
            &log_path,
            &executable,
            DEFAULT_CODEX_SHARED_APP_SERVER_URL,
            true,
            Some(1234),
        )
        .unwrap();
        let log = fs::read_to_string(log_path).unwrap();

        assert!(log.contains("method=direct"));
        assert!(log.contains("pid=1234"));
        assert!(log.contains(executable.to_string_lossy().as_ref()));
        assert!(log.contains(DEFAULT_CODEX_SHARED_APP_SERVER_URL));
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
