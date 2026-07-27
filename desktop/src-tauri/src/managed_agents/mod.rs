mod agent_env;
pub(crate) mod agent_events;
pub(crate) mod agent_snapshot;
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
pub(crate) use managed_node_paths::*;
pub use nest::*;
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

/// Returns a safe Buzz nest directory (`~/.buzz`) if it exists as a real
/// directory (not a symlink), falling back to the user's home directory.
///
/// Used as the default working directory for spawned agent processes.
/// `ensure_nest()` must be called during app setup before this is first
/// invoked, so that `~/.buzz` exists and gets cached.
///
/// Cached for the process lifetime via `OnceLock`.
/// Returns `None` in sandboxed/containerized environments where `$HOME` is
/// unset, points to a non-existent path, or resolves to a filesystem root.
/// Agent spawn callers must reject that state rather than inheriting the
/// desktop process's CWD, which can be `/` for macOS app launches.
pub fn default_agent_workdir() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static WORKDIR: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    WORKDIR
        .get_or_init(|| select_agent_workdir(nest_dir(), dirs::home_dir()))
        .clone()
}

fn select_agent_workdir(
    nest: Option<std::path::PathBuf>,
    home: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    nest.filter(|path| is_safe_agent_workdir(path))
        .or_else(|| home.filter(|path| is_safe_agent_workdir(path)))
}

/// A filesystem root is never a suitable coding-agent workspace: running an
/// agent there makes common project discovery commands scan the whole machine.
fn is_safe_agent_workdir(path: &std::path::Path) -> bool {
    path.parent().is_some() && is_real_dir(path)
}

/// Returns `true` if `path` is a real directory (not a symlink).
fn is_real_dir(path: &std::path::Path) -> bool {
    path.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false)
}

#[cfg(test)]
mod workdir_tests {
    use super::select_agent_workdir;

    #[test]
    fn rejects_filesystem_root_as_agent_workdir() {
        assert_eq!(
            select_agent_workdir(Some("/".into()), Some("/".into())),
            None
        );
    }

    #[test]
    fn prefers_a_real_nest_and_falls_back_to_home() {
        let nest = tempfile::tempdir().expect("create nest");
        let home = tempfile::tempdir().expect("create home");

        assert_eq!(
            select_agent_workdir(
                Some(nest.path().to_path_buf()),
                Some(home.path().to_path_buf())
            ),
            Some(nest.path().to_path_buf())
        );

        assert_eq!(
            select_agent_workdir(
                Some(nest.path().join("missing")),
                Some(home.path().to_path_buf())
            ),
            Some(home.path().to_path_buf())
        );
    }
}
