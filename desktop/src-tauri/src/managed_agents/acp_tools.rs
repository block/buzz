//! Bundled ACP bridge tool resolution.
//!
//! Buzz ships pinned ACP bridge CLIs (`claude-agent-acp`, `codex-acp`) as
//! Tauri application resources (see `desktop/acp-tools.lock.json` and
//! `desktop/scripts/prepare-acp-tools-resource.sh`). This module resolves the
//! staged bin directory once at app setup so the command resolution sweep and
//! the spawn-time PATH augmentation both prefer the bundled bridges over
//! user-installed copies, while everything else on the user's PATH (including
//! installed harness CLIs and their auth state) stays discoverable.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use tauri::path::BaseDirectory;
use tauri::Manager;

use super::discovery::{command_looks_like_path, executable_basename, is_executable_file};

/// Dev-mode override exported by `just dev` / `just staging`, pointing at the
/// freshly staged `src-tauri/resources/acp/bin` in the working tree.
pub const ACP_TOOLS_DIR_ENV: &str = "BUZZ_ACP_TOOLS_DIR";
/// Bundled resource path, relative to the Tauri resource dir (mirrors the
/// `resources/acp` entry in `tauri.conf.json`).
const ACP_TOOLS_RESOURCE_DIR: &str = "resources/acp/bin";
/// Node runtime manifest staged by `desktop/scripts/prepare-acp-tools-resource.sh`
/// next to the bundled bin dir.
const NODE_RUNTIME_MANIFEST_FILE: &str = "node-runtime.json";

static BUNDLED_ACP_TOOLS_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Resolve and register the bundled ACP tools bin dir for the app lifetime:
/// the dev env override wins, then the Tauri resource dir for packaged apps.
/// Called once during app setup, before anything can resolve agent commands.
pub fn register_bundled_acp_tools_dir(app_handle: &tauri::AppHandle) {
    let resolved = bundled_acp_tools_dir_from_parts(
        std::env::var_os(ACP_TOOLS_DIR_ENV).as_deref(),
        app_handle
            .path()
            .resolve(ACP_TOOLS_RESOURCE_DIR, BaseDirectory::Resource)
            .ok()
            .as_deref(),
    );
    let _ = BUNDLED_ACP_TOOLS_DIR.set(resolved);
}

/// The registered bundled ACP tools bin dir, if the app ships one. `None`
/// until [`register_bundled_acp_tools_dir`] runs (e.g. in unit tests), so
/// every consumer degrades to the pre-bundling resolution order.
pub(crate) fn bundled_acp_tools_dir() -> Option<PathBuf> {
    BUNDLED_ACP_TOOLS_DIR.get().cloned().flatten()
}

/// Resolve `command` inside the bundled tools dir. Bare command names only —
/// a path-like command (absolute or multi-component) names a specific binary
/// the user picked and must never be redirected into the bundle.
pub(in crate::managed_agents) fn command_in_bundled_dir(command: &str) -> Option<PathBuf> {
    command_in_dir(&bundled_acp_tools_dir()?, command)
}

/// Path of the Node runtime manifest staged by
/// `desktop/scripts/prepare-acp-tools-resource.sh`: it lives next to the
/// tools bin dir (`acp/node-runtime.json` beside `acp/bin`), so it resolves
/// for both the bundled resource dir and a `BUZZ_ACP_TOOLS_DIR` dev override.
pub(in crate::managed_agents) fn node_runtime_manifest_path(bin_dir: &Path) -> Option<PathBuf> {
    bin_dir
        .parent()
        .map(|dir| dir.join(NODE_RUNTIME_MANIFEST_FILE))
}

fn command_in_dir(dir: &Path, command: &str) -> Option<PathBuf> {
    if command_looks_like_path(command) {
        return None;
    }
    let candidate = dir.join(executable_basename(command));
    is_executable_file(&candidate).then_some(candidate)
}

fn bundled_acp_tools_dir_from_parts(
    env_override: Option<&OsStr>,
    resource_dir: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(value) = env_override {
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    resource_dir.map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::{bundled_acp_tools_dir_from_parts, command_in_dir};
    use std::ffi::OsStr;
    use std::path::Path;

    #[test]
    fn env_override_wins_over_resource_dir() {
        assert_eq!(
            bundled_acp_tools_dir_from_parts(
                Some(OsStr::new("/dev/acp/bin")),
                Some(Path::new("/bundle/resources/acp/bin")),
            )
            .as_deref(),
            Some(Path::new("/dev/acp/bin")),
        );
    }

    #[test]
    fn empty_env_override_falls_back_to_resource_dir() {
        assert_eq!(
            bundled_acp_tools_dir_from_parts(
                Some(OsStr::new("")),
                Some(Path::new("/bundle/resources/acp/bin")),
            )
            .as_deref(),
            Some(Path::new("/bundle/resources/acp/bin")),
        );
    }

    #[test]
    fn missing_inputs_resolve_to_none() {
        assert!(bundled_acp_tools_dir_from_parts(None, None).is_none());
    }

    #[test]
    fn node_runtime_manifest_sits_beside_bin_dir() {
        assert_eq!(
            super::node_runtime_manifest_path(Path::new("/bundle/resources/acp/bin")).as_deref(),
            Some(Path::new("/bundle/resources/acp/node-runtime.json")),
        );
        assert!(super::node_runtime_manifest_path(Path::new("/")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn command_in_dir_finds_executable_by_bare_name() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let tool = temp.path().join("claude-agent-acp");
        fs::write(&tool, "#!/bin/sh\n").expect("write tool");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).expect("chmod tool");

        assert_eq!(
            command_in_dir(temp.path(), "claude-agent-acp").as_deref(),
            Some(tool.as_path()),
        );
        assert!(command_in_dir(temp.path(), "codex-acp").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn command_in_dir_rejects_path_like_commands() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let tool = temp.path().join("codex-acp");
        fs::write(&tool, "#!/bin/sh\n").expect("write tool");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).expect("chmod tool");

        // An absolute path joined onto the bundled dir would *replace* it
        // (Path::join semantics) — path-like commands must pass through to
        // the regular resolution order untouched.
        assert!(command_in_dir(temp.path(), tool.to_str().expect("utf8")).is_none());
        assert!(command_in_dir(temp.path(), "custom/codex-acp").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn command_in_dir_skips_non_executable_files() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let tool = temp.path().join("claude-agent-acp");
        fs::write(&tool, "not executable").expect("write tool");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o644)).expect("chmod tool");

        assert!(command_in_dir(temp.path(), "claude-agent-acp").is_none());
    }
}
