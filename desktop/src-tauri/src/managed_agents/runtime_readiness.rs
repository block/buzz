use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};

use super::{
    discovery::{KnownAcpRuntime, RuntimeReadinessPolicy},
    readiness::EffectiveHarnessDescriptor,
    AcpAvailabilityStatus, AcpRuntimeCatalogEntry, AgentModelInfo, AgentModelsResponse,
    GlobalAgentConfig, RuntimeReadinessStatus,
};

const MODEL_PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const READINESS_CACHE_TTL: Duration = Duration::from_secs(30);
const MAX_PROBE_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RuntimeReadinessCacheKey {
    command: String,
    args: Vec<String>,
    env_digest: [u8; 32],
}

#[derive(Debug, Clone, Copy)]
struct CachedReadiness {
    status: RuntimeReadinessStatus,
    observed_at: Instant,
}

static READINESS_CACHE: LazyLock<Mutex<HashMap<RuntimeReadinessCacheKey, CachedReadiness>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn canonical_command(command: &str) -> String {
    super::resolve_command(command)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| command.to_string())
}

fn readiness_cache_key(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> RuntimeReadinessCacheKey {
    let mut hasher = Sha256::new();
    for (key, value) in env {
        hasher.update((key.len() as u64).to_le_bytes());
        hasher.update(key.as_bytes());
        hasher.update((value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
    RuntimeReadinessCacheKey {
        command: canonical_command(command),
        args: args.to_vec(),
        env_digest: hasher.finalize().into(),
    }
}

fn put_cached(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    status: RuntimeReadinessStatus,
) {
    let key = readiness_cache_key(command, args, env);
    let mut cache = READINESS_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    cache.insert(
        key,
        CachedReadiness {
            status,
            observed_at: Instant::now(),
        },
    );
}

pub(crate) fn invalidate_runtime_readiness() {
    READINESS_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

pub(crate) fn cached_runtime_readiness(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> RuntimeReadinessStatus {
    let key = readiness_cache_key(command, args, env);
    let mut cache = READINESS_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    match cache.get(&key).copied() {
        Some(value) if value.observed_at.elapsed() <= READINESS_CACHE_TTL => value.status,
        Some(_) => {
            cache.remove(&key);
            RuntimeReadinessStatus::Unknown
        }
        None => RuntimeReadinessStatus::Unknown,
    }
}

fn resolved_buzz_acp() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .map(|path| path.with_file_name(format!("buzz-acp{}", std::env::consts::EXE_SUFFIX)))
        .filter(|path| path.exists())
        .or_else(|| super::resolve_command("buzz-acp"))
}

fn kill_probe_tree(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = super::terminate_process(pid);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

fn read_bounded(file: &mut std::fs::File) -> Result<Vec<u8>, String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to rewind model probe output: {error}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_PROBE_OUTPUT_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read model probe output: {error}"))?;
    Ok(bytes)
}

/// Execute the generic `buzz-acp models --json` path with bounded lifetime and
/// output. The child receives only the sanitized effective environment plus
/// the two probe-owned launch variables.
pub(crate) fn run_agent_models_process(
    resolved_acp: &Path,
    agent_command: &str,
    agent_args: &[String],
    merged_env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<serde_json::Value, String> {
    let mut stdout = tempfile::NamedTempFile::new()
        .map_err(|error| format!("failed to allocate model probe stdout: {error}"))?;
    let mut stderr = tempfile::NamedTempFile::new()
        .map_err(|error| format!("failed to allocate model probe stderr: {error}"))?;
    let stdout_child = stdout
        .reopen()
        .map_err(|error| format!("failed to open model probe stdout: {error}"))?;
    let stderr_child = stderr
        .reopen()
        .map_err(|error| format!("failed to open model probe stderr: {error}"))?;

    let resolved_agent_command = canonical_command(agent_command);
    let mut command = Command::new(resolved_acp);
    if let Some(home) = super::default_agent_workdir() {
        command.current_dir(home);
    }
    if let Some(path) = super::readiness::cli_probe::augmented_path() {
        command.env("PATH", path);
    }
    command
        .arg("models")
        .arg("--json")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_child))
        .stderr(Stdio::from(stderr_child));

    for key in super::env_vars::RESERVED_ENV_KEYS {
        command.env_remove(key);
    }
    command
        .env("BUZZ_ACP_AGENT_COMMAND", &resolved_agent_command)
        .env("BUZZ_ACP_AGENT_ARGS", agent_args.join(","));

    if let Some(runtime) = super::known_acp_runtime(&resolved_agent_command) {
        for (key, value) in runtime.default_env {
            command.env(key, value);
        }
    }
    super::build_buzz_agent_provider_defaults(&mut command);
    for (key, value) in merged_env {
        if !super::env_vars::is_reserved_env_key(key) {
            command.env(key, value);
        }
    }
    super::configure_runtime_cli(
        &mut command,
        super::known_acp_runtime(&resolved_agent_command),
    );
    crate::util::configure_no_window(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn buzz-acp models: {error}"))?;
    let pid = child.id();
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                kill_probe_tree(pid);
                let _ = child.wait();
                return Err(format!(
                    "buzz-acp models timed out after {} seconds",
                    timeout.as_secs()
                ));
            }
            Err(error) => {
                kill_probe_tree(pid);
                let _ = child.wait();
                return Err(format!("failed to wait for buzz-acp models: {error}"));
            }
        }
    };
    // A successful helper should have shut its ACP child down. Kill the process
    // group unconditionally so a misbehaving descendant cannot survive a probe.
    kill_probe_tree(pid);

    let stdout_bytes = read_bounded(stdout.as_file_mut())?;
    let stderr_bytes = read_bounded(stderr.as_file_mut())?;
    if !status.success() {
        let stderr_text = String::from_utf8_lossy(&stderr_bytes);
        let redacted = super::redact_env_values_in(stderr_text.as_ref(), merged_env);
        return Err(format!(
            "buzz-acp models failed (exit {}): {redacted}",
            status.code().unwrap_or(-1)
        ));
    }

    serde_json::from_slice(&stdout_bytes)
        .map_err(|error| format!("failed to parse model JSON: {error}"))
}

/// Normalize the shared `buzz-acp models --json` response. Both the interactive
/// model picker and runtime-readiness probe use this exact parser.
pub(crate) fn normalize_agent_models(
    raw: &serde_json::Value,
    persisted_model: Option<String>,
) -> AgentModelsResponse {
    let agent_name = raw["agent"]["name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let agent_version = raw["agent"]["version"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let mut models: Vec<AgentModelInfo> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    if let Some(config_options) = raw["stable"]["configOptions"].as_array() {
        for option in config_options {
            if option.get("category").and_then(|value| value.as_str()) != Some("model") {
                continue;
            }
            if let Some(options) = option.get("options").and_then(|value| value.as_array()) {
                for value in options {
                    if let Some(id) = value.get("value").and_then(|value| value.as_str()) {
                        if seen_ids.insert(id.to_string()) {
                            models.push(AgentModelInfo {
                                id: id.to_string(),
                                name: value
                                    .get("displayName")
                                    .and_then(|value| value.as_str())
                                    .map(str::to_string),
                                description: None,
                            });
                        }
                    }
                }
            }
        }
    }

    let mut agent_default_model = None;
    if let Some(unstable) = raw.get("unstable") {
        agent_default_model = unstable["currentModelId"].as_str().map(str::to_string);
        if let Some(available) = unstable["availableModels"].as_array() {
            for model in available {
                if let Some(id) = model.get("modelId").and_then(|value| value.as_str()) {
                    if seen_ids.insert(id.to_string()) {
                        models.push(AgentModelInfo {
                            id: id.to_string(),
                            name: model
                                .get("name")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            description: model
                                .get("description")
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                        });
                    }
                }
            }
        }
    }

    AgentModelsResponse {
        agent_name,
        agent_version,
        supports_switching: !models.is_empty(),
        models,
        agent_default_model,
        selected_model: persisted_model,
    }
}

pub(crate) fn cache_agent_models_result(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    response: &AgentModelsResponse,
) {
    let status = if response.models.is_empty() {
        RuntimeReadinessStatus::ModelUnavailable
    } else {
        RuntimeReadinessStatus::Ready
    };
    put_cached(command, args, env, status);
}

fn runtime_for_descriptor(
    descriptor: &EffectiveHarnessDescriptor,
) -> Option<&'static KnownAcpRuntime> {
    descriptor
        .runtime_id
        .as_deref()
        .and_then(super::known_acp_runtime_exact)
        .or_else(|| super::known_acp_runtime(&descriptor.command))
}

pub(crate) fn descriptor_cached_readiness(
    descriptor: &EffectiveHarnessDescriptor,
) -> RuntimeReadinessStatus {
    let Some(runtime) = runtime_for_descriptor(descriptor) else {
        return RuntimeReadinessStatus::Ready;
    };
    match runtime.readiness_policy {
        RuntimeReadinessPolicy::AvailabilityOnly => RuntimeReadinessStatus::Ready,
        RuntimeReadinessPolicy::Configuration => RuntimeReadinessStatus::ConfigurationRequired,
        RuntimeReadinessPolicy::Authentication => RuntimeReadinessStatus::Unknown,
        RuntimeReadinessPolicy::AcpModelCatalog => {
            cached_runtime_readiness(&descriptor.command, &descriptor.args, &descriptor.env)
        }
    }
}

pub(crate) fn force_descriptor_readiness(
    descriptor: &EffectiveHarnessDescriptor,
    resolved_acp: Option<&Path>,
) -> RuntimeReadinessStatus {
    let Some(runtime) = runtime_for_descriptor(descriptor) else {
        return RuntimeReadinessStatus::Ready;
    };
    if runtime.readiness_policy != RuntimeReadinessPolicy::AcpModelCatalog {
        return descriptor_cached_readiness(descriptor);
    }

    let result = resolved_acp
        .map(Path::to_path_buf)
        .or_else(resolved_buzz_acp)
        .ok_or_else(|| "buzz-acp helper not found".to_string())
        .and_then(|acp| {
            run_agent_models_process(
                &acp,
                &descriptor.command,
                &descriptor.args,
                &descriptor.env,
                MODEL_PROBE_TIMEOUT,
            )
        })
        .map(|raw| normalize_agent_models(&raw, None));

    let status = match result {
        Ok(response) if response.models.is_empty() => RuntimeReadinessStatus::ModelUnavailable,
        Ok(_) => RuntimeReadinessStatus::Ready,
        Err(error) => {
            eprintln!(
                "buzz-desktop: ACP model readiness probe failed for {}: {error}",
                runtime.id
            );
            RuntimeReadinessStatus::Unknown
        }
    };
    put_cached(
        &descriptor.command,
        &descriptor.args,
        &descriptor.env,
        status,
    );
    status
}

pub(crate) fn refresh_catalog_runtime_readiness(
    entries: &mut [AcpRuntimeCatalogEntry],
    global: &GlobalAgentConfig,
) {
    for entry in entries {
        if entry.availability != AcpAvailabilityStatus::Available {
            continue;
        }
        let Some(runtime) = super::known_acp_runtime_exact(&entry.id) else {
            continue;
        };
        if runtime.readiness_policy != RuntimeReadinessPolicy::AcpModelCatalog {
            continue;
        }
        let Ok(spec) = super::resolve_catalog_harness_by_id(&entry.id) else {
            entry.runtime_readiness = RuntimeReadinessStatus::Unknown;
            continue;
        };
        let mut env = super::baked_build_env();
        for (key, value) in spec.definition_env() {
            if !super::env_vars::is_reserved_env_key(&key) {
                env.insert(key, value);
            }
        }
        env.extend(super::merged_user_env(&BTreeMap::new(), &global.env_vars));
        let descriptor = EffectiveHarnessDescriptor {
            runtime_id: Some(spec.runtime_id),
            command: spec.command,
            args: spec.default_args,
            env,
        };
        entry.runtime_readiness = force_descriptor_readiness(&descriptor, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_catalog_classification_preserves_full_ids() {
        let raw = serde_json::json!({
            "agent": {"name": "Oh My PK", "version": "test"},
            "stable": {"configOptions": [{
                "category": "model",
                "options": [
                    {"value": "anthropic/claude-sonnet-4", "displayName": "Claude"},
                    {"value": "openai-codex/gpt-5.3", "displayName": "Codex"}
                ]
            }]}
        });
        let response = normalize_agent_models(&raw, None);
        assert_eq!(
            response
                .models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["anthropic/claude-sonnet-4", "openai-codex/gpt-5.3"]
        );
        assert!(response.supports_switching);
    }

    #[test]
    fn cache_key_digests_environment_without_retaining_secret() {
        let args = vec!["acp".to_string()];
        let first = BTreeMap::from([("ANTHROPIC_API_KEY".to_string(), "secret-one".to_string())]);
        let second = BTreeMap::from([("ANTHROPIC_API_KEY".to_string(), "secret-two".to_string())]);
        let first_key = readiness_cache_key("ompk", &args, &first);
        let second_key = readiness_cache_key("ompk", &args, &second);
        assert_ne!(first_key, second_key);
        let debug = format!("{first_key:?}");
        assert!(!debug.contains("secret-one"));
        assert!(!debug.contains("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn cache_only_read_never_executes_a_probe() {
        invalidate_runtime_readiness();
        let args = vec!["acp".to_string()];
        let env = BTreeMap::new();
        assert_eq!(
            cached_runtime_readiness("definitely-missing-ompk", &args, &env),
            RuntimeReadinessStatus::Unknown
        );
    }
}
