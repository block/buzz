mod backend;
mod discovery;
mod env_vars;
mod nest;
mod persona_avatars;
mod persona_card;
mod personas;
#[cfg(feature = "mesh-llm")]
mod relay_mesh;
mod restore;
mod runtime;
mod storage;
mod teams;
mod types;

pub use backend::*;
pub use discovery::*;
pub use env_vars::*;
pub use nest::*;
pub use persona_card::*;
pub use personas::*;
#[cfg(feature = "mesh-llm")]
pub use relay_mesh::*;
pub use restore::*;
pub use runtime::*;
pub use storage::*;
pub use teams::*;
pub use types::*;

/// Returns the Sprout nest directory (`~/.sprout`) if it exists as a real
/// directory (not a symlink), falling back to the user's home directory.
///
/// Used as the default working directory for spawned agent processes.
/// `ensure_nest()` must be called during app setup before this is first
/// invoked, so that `~/.sprout` exists and gets cached.
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
            // Prefer ~/.sprout if it exists (created by ensure_nest()).
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
