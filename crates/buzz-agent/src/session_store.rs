//! Where buzz-agent's conversations are persisted.
//!
//! goose's [`SessionManager`] is the conversation store the loop reads and
//! writes each round. Its `instance()` singleton resolves to
//! `Paths::data_dir()` — for a normal install, the *user's own* goose data
//! directory. A buzz-agent using it would write its turns into the same
//! `sessions.db` as the person's `goose` CLI history: not a correctness bug
//! (SQLite handles concurrent writers), but wrong on two counts. Agent
//! conversations get interleaved with a human's personal sessions, and
//! buzz-agent grows an ever-larger file it does not own and cannot prune.
//!
//! So buzz-agent picks its own directory instead. `BUZZ_AGENT_SESSION_DIR`
//! (absolute) is the explicit knob; otherwise we derive a per-user default
//! under the OS data directory. Only if neither is available do we fall back
//! to goose's singleton, because a degraded shared store still beats refusing
//! to start.
//!
//! Note this is deliberately *not* `GOOSE_PATH_ROOT`: that would relocate
//! goose's config and keyring lookups too, and buzz-agent wants goose's
//! provider configuration to keep resolving exactly where goose puts it.

use std::path::PathBuf;
use std::sync::Arc;

use goose::session::session_manager::SessionManager;

/// Absolute path to a directory buzz-agent should keep its session store in.
pub const SESSION_DIR_ENV: &str = "BUZZ_AGENT_SESSION_DIR";

/// Resolve the session-store directory, or `None` to use goose's default.
///
/// Split out from [`session_manager`] so the precedence is testable without
/// touching the filesystem or building a SQLite pool.
fn resolve_dir(env_value: Option<&str>, data_dir: Option<PathBuf>) -> Option<PathBuf> {
    // An explicit relative path is a misconfiguration we should not silently
    // resolve against an arbitrary CWD -- the agent's CWD is the user's
    // workspace, and a stray sessions.db there would be surprising.
    if let Some(path) = env_value.map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(path);
    }
    data_dir.map(|dir| dir.join("buzz-agent"))
}

/// The session store this process should use.
///
/// Resolved once and shared: `SessionManager::new` builds its own SQLite pool,
/// so calling it per session or per round would open a new pool each time
/// against the same file. Every call site must get this one.
///
/// Falls back to goose's shared singleton only when no private directory can
/// be determined at all.
pub fn session_manager() -> Arc<SessionManager> {
    static STORE: std::sync::OnceLock<Arc<SessionManager>> = std::sync::OnceLock::new();
    STORE
        .get_or_init(|| {
            Arc::new(
                match resolve_dir(
                    std::env::var(SESSION_DIR_ENV).ok().as_deref(),
                    dirs::data_dir(),
                ) {
                    Some(dir) => {
                        tracing::debug!(dir = %dir.display(), "session store");
                        SessionManager::new(dir)
                    }
                    None => {
                        tracing::warn!(
                            "no data dir available; falling back to goose's shared session store"
                        );
                        SessionManager::instance()
                    }
                },
            )
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_env_path_wins() {
        let dir = resolve_dir(Some("/tmp/buzz-sessions"), Some(PathBuf::from("/data")));
        assert_eq!(dir, Some(PathBuf::from("/tmp/buzz-sessions")));
    }

    #[test]
    fn a_relative_env_path_is_ignored() {
        // Resolving this against the agent's CWD would drop a sessions.db in
        // whatever workspace the user happened to open.
        let dir = resolve_dir(Some("relative/path"), Some(PathBuf::from("/data")));
        assert_eq!(dir, Some(PathBuf::from("/data/buzz-agent")));
    }

    #[test]
    fn the_default_is_private_to_buzz_agent() {
        // The point of the whole module: never the bare goose data dir, which
        // is where the user's own CLI history lives.
        let dir = resolve_dir(None, Some(PathBuf::from("/data")));
        assert_eq!(dir, Some(PathBuf::from("/data/buzz-agent")));
    }

    #[test]
    fn without_a_data_dir_we_defer_to_goose() {
        assert_eq!(resolve_dir(None, None), None);
    }
}
