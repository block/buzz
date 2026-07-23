//! PATH augmentation for launched managed-agent child processes.

use std::ffi::OsString;
use std::path::PathBuf;

/// Assemble the augmented `PATH` for a launched managed-agent child process.
///
/// Concatenates, in priority order:
///   1. `<home>/.local/bin` — bundled CLI symlink
///   2. Buzz-managed npm prefix bin dir — app-private ACP adapter shims
///   3. Buzz-managed Node.js bin dir — app-private Node/npm runtime
///   4. `nvm_bin` — nvm's default Node.js bin dir (if the user uses nvm)
///   5. exe parent dir — DMG sidecars under `Contents/MacOS/`
///   6. user's login-shell `PATH` — runtimes like node/python from other managers
///      (on Windows, where login-shell PATH is intentionally unavailable, the
///      process `PATH` is appended instead so system Node remains findable)
///
/// `shell_path` is the raw colon-delimited string from a login shell, so it is
/// split into individual entries before joining. Pushing it as a single segment
/// would make `join_paths` reject it (a segment containing the separator is an
/// error), collapsing the entire augmented `PATH` to `None` — the bug this
/// guards against, which left managed agents unable to find `buzz`. Returns
/// `None` only when no entries exist.
pub(in crate::managed_agents) fn build_augmented_path(
    home: Option<PathBuf>,
    exe_parent: Option<PathBuf>,
    shell_path: Option<String>,
    nvm_bin: Option<PathBuf>,
) -> Option<String> {
    #[cfg(windows)]
    let process_path = std::env::var_os("PATH");
    #[cfg(not(windows))]
    let process_path = None;
    build_augmented_path_inner(home, exe_parent, shell_path, nvm_bin, process_path)
}

fn build_augmented_path_inner(
    home: Option<PathBuf>,
    exe_parent: Option<PathBuf>,
    shell_path: Option<String>,
    nvm_bin: Option<PathBuf>,
    process_path: Option<OsString>,
) -> Option<String> {
    let mut parts: Vec<PathBuf> = Vec::new();
    let home_added = home.is_some();
    if let Some(home) = home {
        parts.push(home.join(".local").join("bin"));
    }
    // Only add managed runtime dirs when a home or executable context exists.
    // This keeps tests/utility callers that intentionally pass no local context
    // from manufacturing a PATH out of ambient platform dirs alone.
    if home_added || exe_parent.is_some() {
        if let Some(managed_npm_bin) = crate::managed_agents::buzz_managed_npm_bin_dir() {
            parts.push(managed_npm_bin);
        }
        if let Some(managed_node_bin) = crate::managed_agents::buzz_managed_node_bin_dir() {
            parts.push(managed_node_bin);
        }
    }
    if let Some(nvm_bin) = nvm_bin {
        parts.push(nvm_bin);
    }
    if let Some(parent) = exe_parent {
        parts.push(parent);
    }
    if let Some(shell_path) = shell_path {
        parts.extend(std::env::split_paths(&shell_path));
    } else if let Some(process_path) = process_path {
        // Windows: login_shell_path() returns None so native children inherit
        // the real Windows PATH. When we still build an augmented PATH (e.g.
        // ~/.local/bin + exe parent), append the process PATH or system Node
        // disappears under the override.
        parts.extend(std::env::split_paths(&process_path));
    }
    if parts.is_empty() {
        return None;
    }
    // join_paths uses the platform separator (':' on Unix, ';' on Windows).
    std::env::join_paths(parts)
        .ok()
        .map(|s| s.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::{build_augmented_path, build_augmented_path_inner};
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[cfg(unix)]
    #[test]
    fn splits_colon_delimited_shell_path() {
        // Regression: the shell PATH arrives as one colon-delimited string. It
        // must be split into segments before join_paths, or join_paths rejects
        // it and the whole augmented PATH collapses to None (managed agents then
        // lose `buzz`).
        let result = build_augmented_path(
            Some(PathBuf::from("/home/agent")),
            Some(PathBuf::from("/Applications/Buzz.app/Contents/MacOS")),
            Some("/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin".to_string()),
            None,
        );
        let result = result.expect("path");
        assert!(result.starts_with("/home/agent/.local/bin:"), "{result}");
        assert!(
            result.contains(":/Applications/Buzz.app/Contents/MacOS:"),
            "{result}"
        );
        assert!(
            result.ends_with(":/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin"),
            "{result}"
        );
    }

    #[test]
    fn none_when_no_inputs() {
        assert_eq!(build_augmented_path(None, None, None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn shell_path_only() {
        let result = build_augmented_path(None, None, Some("/usr/bin:/bin".to_string()), None);
        assert_eq!(result.as_deref(), Some("/usr/bin:/bin"));
    }

    #[cfg(unix)]
    #[test]
    fn nvm_bin_inserted_after_local_bin_before_exe_parent() {
        let result = build_augmented_path(
            Some(PathBuf::from("/home/user")),
            Some(PathBuf::from("/Applications/Buzz.app/Contents/MacOS")),
            Some("/usr/bin:/bin".to_string()),
            Some(PathBuf::from("/home/user/.nvm/versions/node/v20.0.0/bin")),
        );
        let result = result.expect("path");
        let local = result.find("/home/user/.local/bin").unwrap();
        let nvm = result
            .find("/home/user/.nvm/versions/node/v20.0.0/bin")
            .unwrap();
        let exe = result
            .find("/Applications/Buzz.app/Contents/MacOS")
            .unwrap();
        assert!(local < nvm && nvm < exe, "{result}");
        assert!(result.ends_with(":/usr/bin:/bin"), "{result}");
    }

    #[cfg(unix)]
    #[test]
    fn nvm_bin_none_does_not_add_segment() {
        let result = build_augmented_path(
            Some(PathBuf::from("/home/user")),
            Some(PathBuf::from("/usr/local/bin")),
            None,
            None,
        );
        let result = result.expect("path");
        assert!(result.starts_with("/home/user/.local/bin:"), "{result}");
        assert!(result.ends_with(":/usr/local/bin"), "{result}");
    }

    #[test]
    fn process_path_appended_when_shell_path_missing() {
        // Windows regression (#2327): with no login-shell PATH, managed dirs
        // alone must not replace the process PATH — system Node lives there.
        let process_path = std::env::join_paths([
            PathBuf::from("/opt/nodejs"),
            PathBuf::from("/usr/bin"),
        ])
        .expect("join process path");
        let result = build_augmented_path_inner(
            Some(PathBuf::from("/home/agent")),
            Some(PathBuf::from("/opt/Buzz")),
            None,
            None,
            Some(process_path),
        );
        let result = result.expect("path");
        assert!(
            result.contains(
                &PathBuf::from("/home/agent")
                    .join(".local")
                    .join("bin")
                    .to_string_lossy()
                    .into_owned()
            ),
            "{result}"
        );
        assert!(
            result.contains(
                PathBuf::from("/opt/nodejs")
                    .to_string_lossy()
                    .as_ref()
            ),
            "process PATH must be retained: {result}"
        );
        let nodejs = PathBuf::from("/opt/nodejs").to_string_lossy().into_owned();
        let usr_bin = PathBuf::from("/usr/bin").to_string_lossy().into_owned();
        let node_pos = result.find(&nodejs).expect("nodejs");
        let usr_pos = result.find(&usr_bin).expect("usr_bin");
        assert!(node_pos < usr_pos, "{result}");
    }

    #[test]
    fn shell_path_preferred_over_process_path() {
        let shell = std::env::join_paths([
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
        ])
        .expect("join shell")
        .to_string_lossy()
        .into_owned();
        let result = build_augmented_path_inner(
            None,
            None,
            Some(shell.clone()),
            None,
            Some(OsString::from("/should/not/appear")),
        );
        assert_eq!(result.as_deref(), Some(shell.as_str()));
    }
}
