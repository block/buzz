use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use uuid::Uuid;

use super::{
    atomic_write_json_restricted, managed_agents_base_dir, BackendKind, CreateManagedAgentRequest,
    ManagedAgentRecord,
};

const STORE_VERSION: u32 = 3;
const MAX_TASKS: usize = 250;
const MODEL_SCAN_BYTES: u64 = 1024 * 1024;
const EXCLUSIVE_TASK_LOCKED_ERROR: &str = "This Codex task still has an exclusive writer lock. A task can be idle and still open in Codex Desktop. Close or leave it in every Codex client, then Retry in Buzz. To use both apps at once, both must connect to the same shared app-server.";

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
    let parsed = url::Url::parse(value)
        .map_err(|error| format!("invalid Codex app-server URL: {error}"))?;
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

pub fn prepare_codex_task_binding(
    input: &CreateManagedAgentRequest,
) -> Result<Option<CodexTaskBinding>, String> {
    let app_server_url = normalize_app_server_url(input.codex_app_server_url.as_deref())?;
    let mut binding = input
        .codex_task_id
        .as_deref()
        .map(binding_for_task_id)
        .transpose()?;
    if let Some(binding) = binding.as_mut() {
        binding.app_server_url = app_server_url;
        if input.backend != BackendKind::Local {
            return Err("Codex tasks can only be bound to local agents".to_string());
        }
        if input
            .parallelism
            .is_some_and(|parallelism| parallelism != 1)
        {
            return Err("Codex task-bound agents require parallelism 1".to_string());
        }
    } else if app_server_url.is_some() {
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
        if binding.app_server_url.is_none() {
            ensure_codex_task_available(&binding.task_id)?;
        }
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
    if let Some(url) = binding.and_then(|binding| binding.app_server_url.as_deref()) {
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

pub fn ensure_codex_task_available(task_id: &str) -> Result<(), String> {
    let codex_home = codex_home_dir()?;
    let lock_path = codex_home
        .join("thread-writer-locks")
        .join(format!("{task_id}.lock"));
    if !lock_path.exists() {
        return Ok(());
    }

    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|_| EXCLUSIVE_TASK_LOCKED_ERROR.to_string())?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| EXCLUSIVE_TASK_LOCKED_ERROR.to_string())?;
    let _ = fs2::FileExt::unlock(&file);
    Ok(())
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
    fn exclusive_lock_error_explains_idle_and_shared_modes() {
        assert!(EXCLUSIVE_TASK_LOCKED_ERROR.contains("idle and still open"));
        assert!(EXCLUSIVE_TASK_LOCKED_ERROR.contains("Retry in Buzz"));
        assert!(EXCLUSIVE_TASK_LOCKED_ERROR.contains("same shared app-server"));
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
    fn exclusive_task_keeps_inherited_codex_path() {
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
