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
mod backend;
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
pub(crate) mod snapshot_avatar;
pub(crate) mod spawn_snapshot;
pub(crate) mod storage;
pub(crate) mod team_events;
mod team_repair;
pub(crate) use team_repair::team_persona_key;
mod teams;
mod types;

// Shared guard for tests that mutate or read process-global PATH.
#[cfg(test)]
static PATH_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn lock_path_mutex() -> std::sync::MutexGuard<'static, ()> {
    PATH_MUTEX.lock().unwrap_or_else(|e| e.into_inner())
}

/// Swap `PATH` for the duration of a test, holding [`lock_path_mutex`] and
/// keeping the `resolve_command` cache consistent with it.
///
/// Taking the mutex is not sufficient on its own. `resolve_command` caches
/// results keyed on the **command name alone** — not on `PATH` — for the life
/// of the process, so a test that changes `PATH` without clearing that cache
/// leaks in both directions: a resolution made under the real `PATH` is
/// returned for a lookup that should hit the test's temp directory, and the
/// test's own temp-directory resolution outlives its `TempDir` and is handed to
/// every later test.
///
/// That is not hypothetical. `claude_spawn_uses_the_probed_cli_executable`
/// asserted that its fake CLI was picked up, but read a real `claude` cached by
/// an earlier test instead — and only on machines where Claude Code is actually
/// installed, so it failed for developers and never once in CI.
///
/// Restoration happens in `Drop`, so a panicking assertion no longer leaves the
/// process with a temp-directory `PATH`.
#[cfg(test)]
pub(crate) struct PathOverride {
    _guard: std::sync::MutexGuard<'static, ()>,
    original: Option<std::ffi::OsString>,
}

#[cfg(test)]
impl PathOverride {
    /// Replace `PATH` entirely with `path`.
    pub(crate) fn set(path: impl AsRef<std::ffi::OsStr>) -> Self {
        let guard = lock_path_mutex();
        let original = std::env::var_os("PATH");
        std::env::set_var("PATH", path);
        discovery::clear_resolve_cache();
        Self {
            _guard: guard,
            original,
        }
    }
}

#[cfg(test)]
impl Drop for PathOverride {
    fn drop(&mut self) {
        match self.original.take() {
            Some(path) => std::env::set_var("PATH", path),
            None => std::env::remove_var("PATH"),
        }
        discovery::clear_resolve_cache();
    }
}

pub use backend::*;
pub(crate) use definition_validation::{
    validate_agent_definition_text, validate_managed_agent_definition_text,
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
