use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use super::{
    atomic_write_json_restricted, managed_agents_base_dir, BackendKind, CreateManagedAgentRequest,
    ManagedAgentRecord,
};

const STORE_VERSION: u32 = 4;
const SHARED_RUNTIME_CONFIG_VERSION: u32 = 1;
const MAX_TASKS: usize = 250;
const MODEL_SCAN_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_CODEX_SHARED_APP_SERVER_URL: &str = "ws://127.0.0.1:51919";
const SHARED_RUNTIME_URL_ENV: &str = "BUZZ_CODEX_SHARED_APP_SERVER_URL";
const SHARED_RUNTIME_COMMAND_ENV: &str = "BUZZ_CODEX_APP_SERVER_COMMAND";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexTaskBinding {
    pub task_id: String,
    pub thread_name: String,
    pub workspace: String,
    pub updated_at: String,
    #[serde(default)]
    pub model: Option<String>,
    /// When set, codex-acp connects to this long-lived app-server instead of
    /// spawning a private Codex process for the Buzz agent.
    #[serde(default)]
    pub app_server_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodexTaskSummary {
    pub id: String,
    pub thread_name: String,
    pub workspace: String,
    pub updated_at: String,
    pub archived: bool,
    pub model: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CodexTaskBindingStore {
    version: u32,
    bindings: HashMap<String, CodexTaskBinding>,
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
}

#[derive(Debug, Deserialize)]
struct SessionIndexEntry {
    id: String,
    thread_name: String,
    updated_at: String,
}

#[derive(Debug)]
struct SessionLocation {
    workspace: String,
    archived: bool,
    path: PathBuf,
}

fn codex_home_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEX_HOME") {
        let path = PathBuf::from(path);
        if path.is_dir() {
            return Ok(path);
        }
    }

    dirs::home_dir()
        .map(|home| home.join(".codex"))
        .filter(|path| path.is_dir())
        .ok_or_else(|| "Codex home directory was not found".to_string())
}

fn binding_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("codex-task-bindings.json"))
}

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

pub fn codex_shared_app_server_url() -> Result<String, String> {
    let configured = std::env::var(SHARED_RUNTIME_URL_ENV).ok();
    normalize_app_server_url(
        configured
            .as_deref()
            .or(Some(DEFAULT_CODEX_SHARED_APP_SERVER_URL)),
    )?
    .ok_or_else(|| "Codex shared app-server URL is not configured".to_string())
}

fn load_binding_store(app: &AppHandle) -> Result<CodexTaskBindingStore, String> {
    let path = binding_store_path(app)?;
    if !path.exists() {
        return Ok(CodexTaskBindingStore {
            version: STORE_VERSION,
            bindings: HashMap::new(),
        });
    }

    let bytes =
        fs::read(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut store: CodexTaskBindingStore = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    if store.version < STORE_VERSION {
        let shared_url = codex_shared_app_server_url()?;
        if let Ok(tasks) = list_codex_tasks() {
            let models = tasks
                .into_iter()
                .map(|task| (task.id, task.model))
                .collect::<HashMap<_, _>>();
            for binding in store.bindings.values_mut() {
                if binding.model.is_none() {
                    binding.model = models.get(&binding.task_id).cloned().flatten();
                }
            }
        }
        for binding in store.bindings.values_mut() {
            binding.app_server_url = Some(shared_url.clone());
        }
        store.version = STORE_VERSION;
        save_binding_store(app, &store)?;
    }
    Ok(store)
}

fn save_binding_store(app: &AppHandle, store: &CodexTaskBindingStore) -> Result<(), String> {
    let path = binding_store_path(app)?;
    let payload = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("failed to serialize Codex task bindings: {error}"))?;
    atomic_write_json_restricted(&path, &payload)
}

pub fn load_codex_task_binding(
    app: &AppHandle,
    agent_pubkey: &str,
) -> Result<Option<CodexTaskBinding>, String> {
    Ok(load_binding_store(app)?.bindings.get(agent_pubkey).cloned())
}

pub fn save_codex_task_binding(
    app: &AppHandle,
    agent_pubkey: &str,
    binding: CodexTaskBinding,
) -> Result<(), String> {
    let mut store = load_binding_store(app)?;
    let active_agent_pubkeys = super::load_managed_agents(app)?
        .into_iter()
        .map(|record| record.pubkey)
        .collect::<HashSet<_>>();
    prune_stale_codex_task_bindings(&mut store, &active_agent_pubkeys);
    if let Some((existing_pubkey, _)) = store
        .bindings
        .iter()
        .find(|(pubkey, existing)| *pubkey != agent_pubkey && existing.task_id == binding.task_id)
    {
        return Err(format!(
            "Codex task {} is already bound to agent {}",
            binding.task_id, existing_pubkey
        ));
    }
    store.version = STORE_VERSION;
    store.bindings.insert(agent_pubkey.to_string(), binding);
    save_binding_store(app, &store)
}

fn prune_stale_codex_task_bindings(
    store: &mut CodexTaskBindingStore,
    active_agent_pubkeys: &HashSet<String>,
) -> bool {
    let original_len = store.bindings.len();
    store
        .bindings
        .retain(|pubkey, _| active_agent_pubkeys.contains(pubkey));
    store.bindings.len() != original_len
}

pub fn remove_codex_task_binding(app: &AppHandle, agent_pubkey: &str) -> Result<(), String> {
    let mut store = load_binding_store(app)?;
    if store.bindings.remove(agent_pubkey).is_some() {
        save_binding_store(app, &store)?;
    }
    Ok(())
}

pub fn binding_for_task_id(task_id: &str) -> Result<CodexTaskBinding, String> {
    let normalized = Uuid::parse_str(task_id.trim())
        .map_err(|_| "Codex task ID must be a UUID".to_string())?
        .to_string();
    let task = list_codex_tasks()?
        .into_iter()
        .find(|task| task.id == normalized)
        .ok_or_else(|| format!("Codex task {normalized} was not found on this computer"))?;
    let workspace = PathBuf::from(&task.workspace);
    if !workspace.is_dir() {
        return Err(format!(
            "Codex task workspace no longer exists: {}",
            workspace.display()
        ));
    }

    Ok(CodexTaskBinding {
        task_id: task.id,
        thread_name: task.thread_name,
        workspace: task.workspace,
        updated_at: task.updated_at,
        model: task.model,
        app_server_url: None,
    })
}

fn normalize_app_server_url(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let parsed =
        url::Url::parse(value).map_err(|error| format!("invalid Codex app-server URL: {error}"))?;
    if !matches!(parsed.scheme(), "ws" | "wss") {
        return Err("Codex app-server URL must use ws:// or wss://".to_string());
    }
    if parsed.host_str().is_none() {
        return Err("Codex app-server URL must include a host".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Codex app-server URL cannot include credentials".to_string());
    }
    Ok(Some(parsed.to_string().trim_end_matches('/').to_string()))
}

fn resolve_codex_task_app_server_url(requested: Option<&str>) -> Result<String, String> {
    let requested = normalize_app_server_url(requested)?;
    let shared_url = codex_shared_app_server_url()?;
    if requested.as_deref().is_some_and(|url| url != shared_url) {
        return Err(format!(
            "Codex task agents use the computer shared runtime at {shared_url}; per-agent app-server URLs are not supported"
        ));
    }
    Ok(shared_url)
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

pub async fn codex_shared_runtime_status(
    app: &AppHandle,
) -> Result<CodexSharedRuntimeStatus, String> {
    let config = load_shared_runtime_config(app)?;
    let url = codex_shared_app_server_url()?;
    if !config.enabled {
        return Ok(CodexSharedRuntimeStatus {
            enabled: false,
            state: CodexSharedRuntimeState::SetupRequired,
            url,
            detail: None,
        });
    }
    match probe_codex_shared_runtime(&url).await {
        Ok(()) => Ok(CodexSharedRuntimeStatus {
            enabled: true,
            state: CodexSharedRuntimeState::Ready,
            url,
            detail: None,
        }),
        Err(error) => Ok(CodexSharedRuntimeStatus {
            enabled: true,
            state: CodexSharedRuntimeState::Unavailable,
            url,
            detail: Some(error),
        }),
    }
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
    if let Some(path) = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(executable))
            .find(|candidate| is_usable_codex_app_server_executable(candidate))
    }) {
        return Ok(path);
    }

    Err(
        "A complete Codex runtime was not found. Open Codex Desktop normally once to finish runtime setup, then retry."
            .to_string(),
    )
}

fn spawn_codex_shared_runtime(app: &AppHandle, url: &str) -> Result<(), String> {
    let executable = find_codex_app_server_executable()?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // Create the long-lived server through WMI so closing or updating Buzz
        // does not tear down the backend currently shared with Codex Desktop.
        let command_line = format!(
            "\"{}\" -c features.code_mode_host=true app-server --listen \"{url}\"",
            executable.display()
        );
        let script = "$result=Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{CommandLine=$env:BUZZ_CODEX_SHARED_RUNTIME_COMMAND_LINE}; if ($result.ReturnValue -ne 0) { throw \"Win32_Process.Create returned $($result.ReturnValue)\" }; $result.ProcessId";
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
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
        let _ = app;
        return Ok(());
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
            .args(["app-server", "--listen", url])
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
            return Ok(CodexSharedRuntimeStatus {
                enabled: true,
                state: CodexSharedRuntimeState::Unavailable,
                url,
                detail: Some(error),
            });
        }
    }
    Ok(CodexSharedRuntimeStatus {
        enabled: true,
        state: CodexSharedRuntimeState::Ready,
        url,
        detail: None,
    })
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
pub fn launch_codex_desktop_shared() -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    let url = codex_shared_app_server_url()?
        .replace('`', "``")
        .replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $env:CODEX_APP_SERVER_WS_URL='{url}'; \
         $package=Get-AppxPackage | Where-Object {{ $_.Name -in @('OpenAI.Codex','OpenAI.CodexBeta') }} | Sort-Object @{{Expression={{if ($_.Name -eq 'OpenAI.Codex') {{0}} else {{1}}}};Ascending=$true}},@{{Expression={{$_.Version}};Descending=$true}} | Select-Object -First 1; \
         if (-not $package) {{ throw 'Codex Desktop is not installed' }}; \
         $app=@((Get-AppxPackageManifest -Package $package).Package.Applications.Application)[0]; \
         $exe=Join-Path $package.InstallLocation ([string]$app.Executable); \
         Start-Process -FilePath $exe"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|error| format!("failed to launch Codex Desktop: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn launch_codex_desktop_shared() -> Result<(), String> {
    Err("Automatic Codex Desktop relaunch is currently available on Windows only.".to_string())
}

pub fn prepare_codex_task_binding(
    input: &CreateManagedAgentRequest,
) -> Result<Option<CodexTaskBinding>, String> {
    let requested_url = normalize_app_server_url(input.codex_app_server_url.as_deref())?;
    let mut binding = input
        .codex_task_id
        .as_deref()
        .map(binding_for_task_id)
        .transpose()?;
    if let Some(binding) = binding.as_mut() {
        binding.app_server_url = Some(resolve_codex_task_app_server_url(
            input.codex_app_server_url.as_deref(),
        )?);
        if input.backend != BackendKind::Local {
            return Err("Codex tasks can only be bound to local agents".to_string());
        }
        if input
            .parallelism
            .is_some_and(|parallelism| parallelism != 1)
        {
            return Err("Codex task-bound agents require parallelism 1".to_string());
        }
    } else if requested_url.is_some() {
        return Err("A shared Codex app-server requires a Codex task binding".to_string());
    }
    Ok(binding)
}

pub fn save_agents_with_codex_task_binding(
    app: &AppHandle,
    records: &[ManagedAgentRecord],
    agent_pubkey: &str,
    binding: Option<CodexTaskBinding>,
) -> Result<(), String> {
    if let Some(binding) = binding {
        save_codex_task_binding(app, agent_pubkey, binding)?;
    }
    if let Err(error) = super::save_managed_agents(app, records) {
        let _ = remove_codex_task_binding(app, agent_pubkey);
        return Err(error);
    }
    Ok(())
}

pub fn delete_codex_task_identity_state(app: &AppHandle, agent_pubkey: &str) -> Result<(), String> {
    remove_codex_task_binding(app, agent_pubkey)?;
    super::delete_agent_key(agent_pubkey);
    Ok(())
}

pub fn task_binding_for_spawn(
    app: &AppHandle,
    record: &ManagedAgentRecord,
) -> Result<Option<CodexTaskBinding>, String> {
    let binding = load_codex_task_binding(app, &record.pubkey)?;
    if let Some(binding) = &binding {
        if record.backend != BackendKind::Local {
            return Err("Codex task-bound agents can only run on this computer".to_string());
        }
        if !Path::new(&binding.workspace).is_dir() {
            return Err(format!(
                "Codex task workspace no longer exists: {}",
                binding.workspace
            ));
        }
        let url = binding.app_server_url.as_deref().ok_or_else(|| {
            "This Codex task binding predates shared runtime setup. Reopen Buzz to migrate it."
                .to_string()
        })?;
        ensure_codex_shared_runtime_reachable(url)?;
    }
    Ok(binding)
}

pub fn configure_task_bound_command(
    command: &mut Command,
    binding: Option<&CodexTaskBinding>,
    lazy: bool,
) {
    if let Some(binding) = binding {
        command.current_dir(&binding.workspace);
        command.env("BUZZ_ACP_CODEX_TASK_ID", &binding.task_id);
        command.env("BUZZ_ACP_CODEX_TASK_WORKSPACE", &binding.workspace);
    } else {
        if let Some(home) = super::default_agent_workdir() {
            command.current_dir(home);
        }
        command.env_remove("BUZZ_ACP_CODEX_TASK_ID");
        command.env_remove("BUZZ_ACP_CODEX_TASK_WORKSPACE");
    }
    command.env(
        "BUZZ_ACP_LAZY_POOL",
        if lazy && binding.is_none() {
            "true"
        } else {
            "false"
        },
    );
}

pub fn configure_shared_app_server(
    command: &mut Command,
    binding: Option<&CodexTaskBinding>,
    proxy_executable: &Path,
) {
    if let Some(binding) = binding {
        let url = binding
            .app_server_url
            .clone()
            .or_else(|| codex_shared_app_server_url().ok())
            .unwrap_or_else(|| DEFAULT_CODEX_SHARED_APP_SERVER_URL.to_string());
        command.env("CODEX_PATH", proxy_executable);
        command.env("CODEX_SHARED_APP_SERVER_URL", url);
    } else {
        command.env_remove("CODEX_SHARED_APP_SERVER_URL");
    }
}

pub fn task_bound_worker_count(
    effective_command: &str,
    parallelism: u32,
    binding: Option<&CodexTaskBinding>,
) -> String {
    if binding.is_some() {
        "1".to_string()
    } else {
        super::acp_agents_value(effective_command, parallelism)
    }
}

fn ensure_codex_shared_runtime_reachable(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url)
        .map_err(|error| format!("invalid Codex shared runtime URL: {error}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "Codex shared runtime URL has no host".to_string())?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "Codex shared runtime URL has no port".to_string())?;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("could not resolve Codex shared runtime: {error}"))?;
    for address in addresses {
        if TcpStream::connect_timeout(&address, Duration::from_millis(750)).is_ok() {
            return Ok(());
        }
    }
    Err(format!(
        "Codex shared runtime is unavailable at {url}. Open Agent settings and start the shared runtime, then retry."
    ))
}

pub fn list_codex_tasks() -> Result<Vec<CodexTaskSummary>, String> {
    let codex_home = codex_home_dir()?;
    let index_path = codex_home.join("session_index.jsonl");
    let index_file = File::open(&index_path)
        .map_err(|error| format!("failed to read {}: {error}", index_path.display()))?;
    // Renames append another entry for the same task. Keep the last one so the
    // picker cannot show duplicate identities with stale titles.
    let mut entries_by_id = HashMap::new();
    for entry in BufReader::new(index_file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<SessionIndexEntry>(&line).ok())
    {
        let Ok(id) = Uuid::parse_str(&entry.id) else {
            continue;
        };
        entries_by_id.insert(id.to_string(), entry);
    }

    let mut locations = HashMap::new();
    collect_session_locations(&codex_home.join("sessions"), false, &mut locations);
    collect_session_locations(&codex_home.join("archived_sessions"), true, &mut locations);

    let mut tasks = entries_by_id
        .into_iter()
        .filter_map(|(normalized, entry)| {
            let location = locations.get(&normalized)?;
            Some(CodexTaskSummary {
                id: normalized,
                thread_name: entry.thread_name,
                workspace: location.workspace.clone(),
                updated_at: entry.updated_at,
                archived: location.archived,
                model: read_latest_codex_model(&location.path),
            })
        })
        .collect::<Vec<_>>();
    tasks.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    tasks.truncate(MAX_TASKS);
    Ok(tasks)
}

fn collect_session_locations(
    root: &Path,
    archived: bool,
    locations: &mut HashMap<String, SessionLocation>,
) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_session_locations(&path, archived, locations);
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let Some((task_id, workspace)) = read_session_meta(&path) else {
            continue;
        };
        locations.insert(
            task_id,
            SessionLocation {
                workspace,
                archived,
                path,
            },
        );
    }
}

fn read_latest_codex_model(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len > MODEL_SCAN_BYTES {
        file.seek(SeekFrom::End(-(MODEL_SCAN_BYTES as i64))).ok()?;
    }

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let tail = String::from_utf8_lossy(&bytes);
    for line in tail.lines().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("type").and_then(serde_json::Value::as_str) != Some("turn_context") {
            continue;
        }
        let payload = value.get("payload")?;
        let model = payload
            .get("model")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        let effort = payload
            .get("effort")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                payload
                    .pointer("/collaboration_mode/settings/reasoning_effort")
                    .and_then(serde_json::Value::as_str)
            })
            .map(str::trim)
            .filter(|value| !value.is_empty());

        return Some(match effort {
            Some(effort) if !(model.contains('[') && model.ends_with(']')) => {
                format!("{model}[{effort}]")
            }
            _ => model.to_string(),
        });
    }
    None
}

fn read_session_meta(path: &Path) -> Option<(String, String)> {
    let file = File::open(path).ok()?;
    let mut lines = BufReader::new(file).lines();
    let line = lines.next()?.ok()?;
    let value: serde_json::Value = serde_json::from_str(&line).ok()?;
    if value.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let payload = value.get("payload")?;
    let task_id = Uuid::parse_str(payload.get("id")?.as_str()?)
        .ok()?
        .to_string();
    let workspace = payload.get("cwd")?.as_str()?.to_string();
    Some((task_id, workspace))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn reads_codex_session_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let mut file = File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"type":"session_meta","payload":{{"id":"019eca9a-beb9-7902-8ce6-527b2ba56020","cwd":"C:\\repo"}}}}"#
        )
        .unwrap();

        assert_eq!(
            read_session_meta(&path),
            Some((
                "019eca9a-beb9-7902-8ce6-527b2ba56020".to_string(),
                r"C:\repo".to_string(),
            ))
        );
    }

    #[test]
    fn reads_latest_model_and_reasoning_effort() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let mut file = File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"type":"turn_context","payload":{{"model":"gpt-5","effort":"high"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"type":"turn_context","payload":{{"model":"gpt-5.5","collaboration_mode":{{"settings":{{"reasoning_effort":"xhigh"}}}}}}}}"#
        )
        .unwrap();

        assert_eq!(
            read_latest_codex_model(&path).as_deref(),
            Some("gpt-5.5[xhigh]")
        );
    }

    #[test]
    fn validates_shared_app_server_urls() {
        assert_eq!(
            normalize_app_server_url(Some(" ws://127.0.0.1:51919/ ")).unwrap(),
            Some("ws://127.0.0.1:51919".to_string())
        );
        assert!(normalize_app_server_url(Some("http://127.0.0.1:51919")).is_err());
        assert!(normalize_app_server_url(Some("ws://user@127.0.0.1:51919")).is_err());
    }

    #[test]
    fn shared_runtime_has_one_computer_level_default() {
        assert_eq!(
            normalize_app_server_url(Some(DEFAULT_CODEX_SHARED_APP_SERVER_URL)).unwrap(),
            Some(DEFAULT_CODEX_SHARED_APP_SERVER_URL.to_string())
        );
        assert_eq!(
            resolve_codex_task_app_server_url(None).unwrap(),
            DEFAULT_CODEX_SHARED_APP_SERVER_URL
        );
        assert!(resolve_codex_task_app_server_url(Some("ws://127.0.0.1:59999")).is_err());
    }

    #[test]
    fn stale_agent_bindings_are_pruned_before_rebinding() {
        let binding = CodexTaskBinding {
            task_id: "019febeb-ae12-71d3-88c4-25c04a461042".to_string(),
            thread_name: "Deleted task agent".to_string(),
            workspace: r"C:\repo".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            model: None,
            app_server_url: Some(DEFAULT_CODEX_SHARED_APP_SERVER_URL.to_string()),
        };
        let mut store = CodexTaskBindingStore {
            version: STORE_VERSION,
            bindings: HashMap::from([
                ("active-agent".to_string(), binding.clone()),
                ("deleted-agent".to_string(), binding),
            ]),
        };
        let active = HashSet::from(["active-agent".to_string()]);

        assert!(prune_stale_codex_task_bindings(&mut store, &active));
        assert!(store.bindings.contains_key("active-agent"));
        assert!(!store.bindings.contains_key("deleted-agent"));
        assert!(!prune_stale_codex_task_bindings(&mut store, &active));
    }

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

    #[test]
    fn configures_shared_app_server_proxy_environment() {
        let binding = CodexTaskBinding {
            task_id: "019eca9a-beb9-7902-8ce6-527b2ba56020".to_string(),
            thread_name: "Shared task".to_string(),
            workspace: r"C:\repo".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            model: Some("gpt-5.5[xhigh]".to_string()),
            app_server_url: Some("ws://127.0.0.1:51919".to_string()),
        };
        let mut command = Command::new("buzz-acp");

        configure_shared_app_server(
            &mut command,
            Some(&binding),
            Path::new(r"C:\Buzz\buzz-acp.exe"),
        );

        let env = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<HashMap<_, _>>();
        assert_eq!(
            env.get("CODEX_SHARED_APP_SERVER_URL"),
            Some(&Some("ws://127.0.0.1:51919".to_string()))
        );
        assert_eq!(
            env.get("CODEX_PATH"),
            Some(&Some(r"C:\Buzz\buzz-acp.exe".to_string()))
        );
    }

    #[test]
    fn ordinary_agent_keeps_inherited_codex_path() {
        let mut command = Command::new("buzz-acp");

        configure_shared_app_server(&mut command, None, Path::new(r"C:\Buzz\buzz-acp.exe"));

        let env = command
            .get_envs()
            .map(|(key, value)| (key.to_string_lossy().into_owned(), value))
            .collect::<HashMap<_, _>>();
        assert!(!env.contains_key("CODEX_PATH"));
        assert_eq!(env.get("CODEX_SHARED_APP_SERVER_URL"), Some(&None));
    }
}
