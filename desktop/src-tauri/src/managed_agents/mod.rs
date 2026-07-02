mod agent_env;
pub(crate) mod agent_events;
pub(crate) use agent_env::{build_buzz_agent_provider_defaults, discovery_env_with_baked_floor};
mod backend;
pub(crate) mod config_bridge;
mod discovery;
mod env_vars;
mod nest;
mod persona_avatars;
mod persona_card;
pub(crate) mod persona_events;
mod personas;
#[cfg(windows)]
mod process_lifecycle;
pub(crate) mod readiness;
#[cfg(feature = "mesh-llm")]
mod relay_mesh;
mod repos;
mod restore;
pub mod retention;
mod runtime;
mod storage;
pub(crate) mod team_events;
mod team_repair;
mod teams;
mod types;

pub use backend::*;
pub use discovery::*;
pub use env_vars::*;
pub use nest::*;
pub use persona_card::*;
pub use personas::*;
#[cfg(windows)]
pub use process_lifecycle::*;
pub(crate) use readiness::{
    agent_readiness, resolve_effective_agent_env, AgentReadiness, Requirement,
};
#[cfg(feature = "mesh-llm")]
pub use relay_mesh::*;
pub use repos::{
    effective_repos_dir, ensure_repos_symlink, resolve_repos_at_boot, validate_repos_dir,
    write_persisted_repos_dir,
};
pub use restore::*;
pub use runtime::*;
pub use storage::*;
pub use team_repair::{sync_team_personas, team_persona_key};
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
