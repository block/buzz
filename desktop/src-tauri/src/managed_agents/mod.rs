pub(crate) mod access_policy;
mod agent_env;
pub(crate) mod agent_events;
pub(crate) mod agent_snapshot;
pub(crate) mod agent_snapshot_envelope;
pub(crate) mod team_snapshot;
pub(crate) use access_policy::{owner_only, owner_only_access_build, projected_access_with_policy};
pub(crate) use agent_env::{
    baked_build_env, build_buzz_agent_provider_defaults, discovery_env_with_baked_floor,
};
mod agent_description;
pub(crate) use agent_description::{effective_agent_description, record_effective_description};
mod backend;
pub(crate) mod bestie_assignment;
pub(crate) mod claude_config;
pub(crate) mod config_bridge;
pub(crate) mod custom_harnesses;
mod definition_validation;
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
mod session_policy;
pub(crate) mod snapshot_avatar;
pub(crate) mod spawn_snapshot;
pub(crate) mod storage;
pub(crate) mod team_catalog;
pub(crate) mod team_events;
mod team_repair;
pub(crate) use team_repair::team_persona_key;
mod teams;
mod types;

// Shared lock for tests that call `lock_path_mutex` or `lock_env_mutex`.
// Both helpers delegate here so any two tests using either helper are mutually
// exclusive with each other. Tests in other modules that maintain their own
// independent locks (app_state_tests, agent_config_tests, reader_tests) are
// NOT in this domain and are not covered by this mutex.
#[cfg(test)]
static PROCESS_ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

// Acquires the shared process-env lock. Call from any test in this module that
// reads, writes, or removes a process-global environment variable (including PATH).
#[cfg(test)]
pub(crate) fn lock_path_mutex() -> std::sync::MutexGuard<'static, ()> {
    PROCESS_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
}

// Delegates to the same lock as `lock_path_mutex`. Tests using either helper
// are mutually exclusive with each other; PATH and env-key mutations that go
// through these helpers cannot race.
#[cfg(test)]
pub(crate) fn lock_env_mutex() -> std::sync::MutexGuard<'static, ()> {
    PROCESS_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
}

pub use backend::*;
pub(crate) use definition_validation::{
    validate_agent_definition_text, validate_agent_description_text,
    validate_managed_agent_definition_text, validate_visible_text,
};
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
pub(crate) use session_policy::{
    acp_session_policy, apply_app_acp_session_policy_env, insert_acp_session_policy_env,
    AcpSessionPolicy, ManagedAgentExperimentState, ACP_SESSION_POLICY_ENV_VAR,
};
pub use storage::*;
pub use teams::*;
pub use types::*;

#[cfg(test)]
pub(crate) use teams::delete_catalog_team_at;

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

/// Returns `true` if `path` is a real directory (not a symlink).
fn is_real_dir(path: &std::path::Path) -> bool {
    path.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false)
}
