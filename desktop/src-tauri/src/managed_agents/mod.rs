mod agent_env;
pub(crate) mod agent_events;
pub(crate) mod agent_snapshot;
pub(crate) mod team_snapshot;
pub(crate) use agent_env::{
    baked_build_env, build_buzz_agent_provider_defaults, discovery_env_with_baked_floor,
};
mod backend;
pub(crate) mod config_bridge;
mod discovery;
mod env_vars;
pub(crate) mod git_bash;
pub(crate) mod global_config;
mod goose_env;
mod managed_node_paths;
mod nest;
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
pub(crate) mod spawn_hash;
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
pub(crate) use goose_env::apply_goose_runtime_env;
pub(crate) use managed_node_paths::*;
pub use nest::*;
pub use personas::*;
#[cfg(windows)]
pub use process_lifecycle::*;
pub(crate) use readiness::{
    agent_readiness, resolve_effective_agent_env, AgentReadiness, Requirement,
};
pub use relay_mesh::*;
pub use repos::{
    effective_repos_dir, ensure_repos_symlink, resolve_repos_at_boot, validate_repos_dir,
    write_persisted_repos_dir,
};
pub use restore::*;
pub use runtime::*;
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

/// Returns `true` if `path` is a real directory (not a symlink).
fn is_real_dir(path: &std::path::Path) -> bool {
    path.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false)
}
