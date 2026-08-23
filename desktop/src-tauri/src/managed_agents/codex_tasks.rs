use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::Duration,
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use uuid::Uuid;

use super::{
    atomic_write_json_restricted, managed_agents_base_dir, BackendKind, CreateManagedAgentRequest,
    ManagedAgentRecord,
};

const STORE_VERSION: u32 = 4;
const MAX_TASKS: usize = 250;
const MODEL_SCAN_BYTES: u64 = 1024 * 1024;
const MODEL_SCAN_CHUNK_BYTES: u64 = 64 * 1024;
const MAX_HISTORY_MESSAGES: usize = 200;
const MAX_HISTORY_MESSAGE_CHARS: usize = 20_000;
pub const DEFAULT_CODEX_SHARED_APP_SERVER_URL: &str = "ws://127.0.0.1:51919";
const SHARED_RUNTIME_URL_ENV: &str = "BUZZ_CODEX_SHARED_APP_SERVER_URL";

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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodexTaskHistoryMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodexTaskHistory {
    pub task_id: String,
    pub thread_name: String,
    pub messages: Vec<CodexTaskHistoryMessage>,
    pub truncated: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CodexTaskBindingStore {
    version: u32,
    bindings: HashMap<String, CodexTaskBinding>,
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

#[derive(Clone)]
struct CachedTaskModel {
    len: u64,
    modified: Option<SystemTime>,
    model: Option<String>,
}

fn task_model_cache() -> &'static Mutex<HashMap<PathBuf, CachedTaskModel>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedTaskModel>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
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
            Some((
                CodexTaskSummary {
                    id: normalized,
                    thread_name: entry.thread_name,
                    workspace: location.workspace.clone(),
                    updated_at: entry.updated_at,
                    archived: location.archived,
                    model: None,
                },
                location.path.clone(),
            ))
        })
        .collect::<Vec<_>>();
    tasks.sort_by(|(left, _), (right, _)| right.updated_at.cmp(&left.updated_at));
    tasks.truncate(MAX_TASKS);
    Ok(tasks
        .into_iter()
        .map(|(mut task, path)| {
            task.model = read_latest_codex_model(&path);
            task
        })
        .collect())
}

pub fn get_codex_task_history(
    app: &AppHandle,
    agent_pubkey: &str,
) -> Result<CodexTaskHistory, String> {
    let binding = load_codex_task_binding(app, agent_pubkey)?
        .ok_or_else(|| "This agent is not bound to a Codex task".to_string())?;
    let codex_home = codex_home_dir()?;
    let mut locations = HashMap::new();
    collect_session_locations(&codex_home.join("sessions"), false, &mut locations);
    collect_session_locations(&codex_home.join("archived_sessions"), true, &mut locations);
    let location = locations.get(&binding.task_id).ok_or_else(|| {
        format!(
            "Codex task {} was not found on this computer",
            binding.task_id
        )
    })?;
    let (messages, truncated) = read_codex_task_history(&location.path)?;
    Ok(CodexTaskHistory {
        task_id: binding.task_id,
        thread_name: binding.thread_name,
        messages,
        truncated,
    })
}

fn read_codex_task_history(path: &Path) -> Result<(Vec<CodexTaskHistoryMessage>, bool), String> {
    let file =
        File::open(path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut messages = VecDeque::with_capacity(MAX_HISTORY_MESSAGES);
    let mut truncated = false;
    for (line_index, line) in BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .enumerate()
    {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(message) = parse_codex_history_message(&value, line_index) else {
            continue;
        };
        if messages.len() == MAX_HISTORY_MESSAGES {
            messages.pop_front();
            truncated = true;
        }
        messages.push_back(message);
    }
    Ok((messages.into_iter().collect(), truncated))
}

fn parse_codex_history_message(
    value: &serde_json::Value,
    line_index: usize,
) -> Option<CodexTaskHistoryMessage> {
    let timestamp = value
        .get("timestamp")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let payload = value.get("payload")?;
    let (role, content) = match (
        value.get("type").and_then(serde_json::Value::as_str),
        payload.get("type").and_then(serde_json::Value::as_str),
    ) {
        (Some("event_msg"), Some("user_message")) => {
            ("user", payload.get("message")?.as_str()?.to_string())
        }
        (Some("response_item"), Some("message"))
            if payload.get("role").and_then(serde_json::Value::as_str) == Some("assistant")
                && payload.get("phase").and_then(serde_json::Value::as_str)
                    == Some("final_answer") =>
        {
            let content = payload
                .get("content")?
                .as_array()?
                .iter()
                .filter(|item| {
                    item.get("type").and_then(serde_json::Value::as_str) == Some("output_text")
                })
                .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            ("assistant", content)
        }
        _ => return None,
    };
    let content = content.trim();
    if content.is_empty() {
        return None;
    }
    Some(CodexTaskHistoryMessage {
        id: payload
            .get("id")
            .or_else(|| payload.get("client_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("line-{line_index}")),
        role: role.to_string(),
        content: truncate_history_content(content),
        timestamp,
    })
}

fn truncate_history_content(content: &str) -> String {
    if content.chars().count() <= MAX_HISTORY_MESSAGE_CHARS {
        return content.to_string();
    }
    let mut truncated = content
        .chars()
        .take(MAX_HISTORY_MESSAGE_CHARS)
        .collect::<String>();
    truncated.push_str("\n\n... [message truncated]");
    truncated
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
    let metadata = fs::metadata(path).ok()?;
    let len = metadata.len();
    let modified = metadata.modified().ok();
    if let Ok(cache) = task_model_cache().lock() {
        if let Some(cached) = cache.get(path) {
            if cached.len == len && cached.modified == modified {
                return cached.model.clone();
            }
        }
    }

    let mut file = File::open(path).ok()?;
    let model = scan_latest_codex_model(&mut file, len);

    if let Ok(mut cache) = task_model_cache().lock() {
        cache.insert(
            path.to_path_buf(),
            CachedTaskModel {
                len,
                modified,
                model: model.clone(),
            },
        );
    }
    model
}

fn scan_latest_codex_model(file: &mut File, len: u64) -> Option<String> {
    let min_offset = len.saturating_sub(MODEL_SCAN_BYTES);
    let mut end = len;
    let mut leading_fragment = Vec::new();
    while end > min_offset {
        let start = end.saturating_sub(MODEL_SCAN_CHUNK_BYTES).max(min_offset);
        let mut bytes = vec![0; (end - start) as usize];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut bytes).ok()?;
        bytes.extend_from_slice(&leading_fragment);

        if let Some(first_newline) = bytes.iter().position(|byte| *byte == b'\n') {
            if let Some(model) = bytes[first_newline + 1..]
                .split(|byte| *byte == b'\n')
                .rev()
                .find_map(parse_codex_model_line)
            {
                return Some(model);
            }
            leading_fragment.clear();
            leading_fragment.extend_from_slice(&bytes[..first_newline]);
        } else {
            leading_fragment = bytes;
        }
        end = start;
    }
    parse_codex_model_line(&leading_fragment)
}

fn parse_codex_model_line(line: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<serde_json::Value>(line).ok()?;
    if value.get("type").and_then(serde_json::Value::as_str) != Some("turn_context") {
        return None;
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
    Some(match effort {
        Some(effort) if !(model.contains('[') && model.ends_with(']')) => {
            format!("{model}[{effort}]")
        }
        _ => model.to_string(),
    })
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
    use std::fs::OpenOptions;
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
    fn scans_additional_chunks_when_latest_context_is_outside_initial_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let mut file = File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"type":"turn_context","payload":{{"model":"gpt-5.5","effort":"high"}}}}"#
        )
        .unwrap();
        file.write_all(&vec![b'x'; MODEL_SCAN_CHUNK_BYTES as usize + 1])
            .unwrap();
        drop(file);

        assert_eq!(
            read_latest_codex_model(&path).as_deref(),
            Some("gpt-5.5[high]")
        );
    }

    #[test]
    fn invalidates_cached_model_when_session_grows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        fs::write(
            &path,
            r#"{"type":"turn_context","payload":{"model":"gpt-5","effort":"medium"}}
"#,
        )
        .unwrap();
        assert_eq!(
            read_latest_codex_model(&path).as_deref(),
            Some("gpt-5[medium]")
        );

        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            file,
            r#"{{"type":"turn_context","payload":{{"model":"gpt-5.5","effort":"xhigh"}}}}"#
        )
        .unwrap();
        drop(file);

        assert_eq!(
            read_latest_codex_model(&path).as_deref(),
            Some("gpt-5.5[xhigh]")
        );
    }

    #[test]
    fn reads_only_user_messages_and_final_answers_from_codex_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let mut file = File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2026-08-19T01:00:00Z","type":"event_msg","payload":{{"type":"user_message","message":"Hello"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2026-08-19T01:00:01Z","type":"event_msg","payload":{{"type":"agent_reasoning","text":"hidden"}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2026-08-19T01:00:02Z","type":"response_item","payload":{{"type":"message","role":"assistant","phase":"commentary","content":[{{"type":"output_text","text":"working"}}]}}}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2026-08-19T01:00:03Z","type":"response_item","payload":{{"type":"message","role":"assistant","phase":"final_answer","content":[{{"type":"output_text","text":"Done"}}]}}}}"#
        )
        .unwrap();

        let (messages, truncated) = read_codex_task_history(&path).unwrap();
        assert!(!truncated);
        assert_eq!(
            messages,
            vec![
                CodexTaskHistoryMessage {
                    id: "line-0".to_string(),
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                    timestamp: Some("2026-08-19T01:00:00Z".to_string()),
                },
                CodexTaskHistoryMessage {
                    id: "line-3".to_string(),
                    role: "assistant".to_string(),
                    content: "Done".to_string(),
                    timestamp: Some("2026-08-19T01:00:03Z".to_string()),
                },
            ]
        );
    }

    #[test]
    fn codex_history_keeps_only_the_latest_message_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollout.jsonl");
        let mut file = File::create(&path).unwrap();
        for index in 0..=MAX_HISTORY_MESSAGES {
            writeln!(
                file,
                r#"{{"type":"event_msg","payload":{{"type":"user_message","message":"message-{index}"}}}}"#
            )
            .unwrap();
        }

        let (messages, truncated) = read_codex_task_history(&path).unwrap();
        assert!(truncated);
        assert_eq!(messages.len(), MAX_HISTORY_MESSAGES);
        assert_eq!(
            messages.first().map(|message| message.content.as_str()),
            Some("message-1")
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
