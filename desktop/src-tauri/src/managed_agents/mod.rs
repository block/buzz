mod agent_env;
pub(crate) mod agent_events;
pub(crate) mod agent_snapshot;
pub(crate) mod agent_snapshot_envelope;
pub(crate) mod team_snapshot;
pub(crate) use agent_env::{
    baked_build_env, build_buzz_agent_provider_defaults, discovery_env_with_baked_floor,
};
mod backend;
pub(crate) mod config_bridge;
pub(crate) mod custom_harnesses;
mod discovery;
pub(crate) mod effective_config;
mod env_vars;
pub(crate) mod git_bash;
pub(crate) mod global_config;
mod managed_node_paths;
mod nest;
pub(crate) mod parallelism;
mod persona_avatars;
pub(crate) mod persona_events;
mod personas;
#[cfg(windows)]
mod process_lifecycle;
pub(crate) mod readiness;
pub(crate) mod reconcile;
mod relay_mesh;
mod repos;
mod restore;
pub mod retention;
mod runtime;
mod runtime_commands;
mod runtime_types;
pub(crate) mod snapshot_avatar;
pub(crate) mod spawn_snapshot;
pub(crate) mod storage;
pub(crate) mod team_events;
mod team_repair;
mod teams;
mod types;

// Shared guard for tests that mutate or read process-global PATH.
#[cfg(test)]
static PATH_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_path_mutex() -> std::sync::MutexGuard<'static, ()> {
    PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
}

pub use backend::*;
pub use discovery::*;
pub use env_vars::*;
#[cfg(windows)]
pub(crate) use git_bash::git_bash_available;
pub(crate) use git_bash::{discover_git_bash, GitBashPrerequisite};
pub(crate) use global_config::{
    load_global_agent_config, resolve_effective_model_provider, save_global_agent_config,
    validate_global_config, GlobalAgentConfig,
};
pub(crate) use managed_node_paths::*;
pub use nest::*;
pub use parallelism::{acp_agents_value, effective_parallelism, harness_max_parallelism};
pub use personas::*;
#[cfg(windows)]
pub use process_lifecycle::*;
pub(crate) use readiness::{
    agent_readiness, resolve_effective_agent_env, resolve_effective_harness_descriptor,
    AgentReadiness, Requirement,
};
pub use relay_mesh::*;
pub use repos::{
    effective_repos_dir, ensure_repos_symlink, resolve_repos_at_boot, validate_repos_dir,
    write_persisted_repos_dir,
};
pub use restore::*;
pub use runtime::*;
pub use runtime_commands::*;
pub use runtime_types::*;
pub use storage::*;
pub use teams::*;
pub use types::*;

/// Returns the Buzz nest directory (`~/.buzz`) if it exists as a real
/// directory (not a symlink), falling back to the user's home directory.
///
/// Used as the default working directory for spawned agent processes.
/// `ensure_nest()` must be called during app setup before this is first
/// invoked, so that `~/.buzz` exists and gets cached.
///
/// Cached for the process lifetime via `OnceLock`.
/// Returns `None` in sandboxed/containerized environments where `$HOME` is
/// unset or points to a non-existent path; callers fall back to inheriting
/// the parent's CWD.
pub fn default_agent_workdir() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static WORKDIR: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    WORKDIR
        .get_or_init(|| {
            // Prefer ~/.buzz if it exists (created by ensure_nest()).
            // Reject symlinks to prevent redirect attacks — is_dir()
            // follows symlinks, so check symlink_metadata() first.
            // Fall back to $HOME for resilience.
            nest_dir()
                .filter(|p| is_real_dir(p))
                .or_else(|| dirs::home_dir().filter(|p| p.is_dir()))
        })
        .clone()
}

/// Validate and normalize a configured per-agent working directory.
///
/// Configured paths are machine-local guardrails, not sandbox boundaries. They
/// must resolve to an existing absolute directory and may not resolve to a
/// filesystem root. Canonicalizing before persistence prevents later launches
/// from interpreting `..` components or a configured symlink differently.
pub(crate) fn normalize_agent_workdir(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("working directory cannot be empty; clear it to use the default".to_string());
    }
    let candidate = std::path::Path::new(trimmed);
    if !candidate.is_absolute() {
        return Err("working directory must be an absolute path".to_string());
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("working directory is not accessible: {error}"))?;
    if !canonical.is_dir() {
        return Err("working directory must be an existing directory".to_string());
    }
    if canonical.parent().is_none() {
        return Err("working directory cannot be a filesystem root".to_string());
    }
    canonical
        .into_os_string()
        .into_string()
        .map_err(|_| "working directory must be valid UTF-8".to_string())
}

/// Resolve the directory an agent harness will launch from.
pub(crate) fn effective_agent_workdir(
    record: &ManagedAgentRecord,
) -> Result<Option<std::path::PathBuf>, String> {
    match record.working_directory.as_deref() {
        Some(path) => normalize_agent_workdir(path)
            .map(std::path::PathBuf::from)
            .map(Some),
        None => Ok(default_agent_workdir()),
    }
}

/// Apply the effective per-agent working directory to a child command.
pub(crate) fn configure_agent_workdir(
    command: &mut std::process::Command,
    record: &ManagedAgentRecord,
) -> Result<Option<std::path::PathBuf>, String> {
    let working_directory = effective_agent_workdir(record)?;
    if let Some(path) = &working_directory {
        command.current_dir(path);
        tracing::info!(
            agent_pubkey = %record.pubkey,
            working_directory = %path.display(),
            configured = record.working_directory.is_some(),
            "resolved managed agent working directory"
        );
    }
    Ok(working_directory)
}

/// Returns `true` if `path` is a real directory (not a symlink).
fn is_real_dir(path: &std::path::Path) -> bool {
    path.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false)
}

#[cfg(test)]
mod workdir_tests {
    use super::{configure_agent_workdir, normalize_agent_workdir};

    #[test]
    fn configured_workdir_requires_an_absolute_existing_non_root_directory() {
        assert!(normalize_agent_workdir("relative/path").is_err());
        assert!(normalize_agent_workdir("").is_err());

        let current = std::env::current_dir().unwrap();
        let root = current.ancestors().last().unwrap();
        assert!(normalize_agent_workdir(&root.to_string_lossy()).is_err());

        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            normalize_agent_workdir(&dir.path().to_string_lossy()).unwrap(),
            dir.path().canonicalize().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn configured_workdir_rejects_files_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "test").unwrap();
        assert!(normalize_agent_workdir(&file.to_string_lossy()).is_err());
        assert!(normalize_agent_workdir(&dir.path().join("missing").to_string_lossy()).is_err());
    }

    #[test]
    fn configured_workdir_is_applied_to_the_spawn_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut record: crate::managed_agents::ManagedAgentRecord =
            serde_json::from_value(serde_json::json!({
                "pubkey": "aa",
                "name": "test",
                "relay_url": "",
                "working_directory": dir.path(),
                "acp_command": "buzz-acp",
                "agent_command": "buzz-agent",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 0,
                "parallelism": 1,
                "system_prompt": null,
                "start_on_app_launch": false,
                "runtime_pid": null,
                "created_at": "now",
                "updated_at": "now"
            }))
            .unwrap();
        record.working_directory = Some(dir.path().to_string_lossy().into_owned());

        let mut command = std::process::Command::new("unused");
        configure_agent_workdir(&mut command, &record).unwrap();
        assert_eq!(
            command.get_current_dir(),
            Some(dir.path().canonicalize().unwrap().as_path())
        );
    }
}
