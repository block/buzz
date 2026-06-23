//! PATH augmentation for launched managed-agent child processes.

use std::path::PathBuf;

/// Assemble the augmented `PATH` for a launched managed-agent child process.
///
/// Concatenates, in priority order: `<home>/.local/bin` (the bundled CLI
/// symlink), the running executable's parent dir (DMG sidecars under
/// `Contents/MacOS/`), and the user's login-shell `PATH` (runtimes like
/// node/python).
///
/// `shell_path` is the raw colon-delimited string from a login shell, so it is
/// split into individual entries before joining. Pushing it as a single segment
/// would make `join_paths` reject it (a segment containing the separator is an
/// error), collapsing the entire augmented `PATH` to `None` — the bug this
/// guards against, which left managed agents unable to find `buzz`. Returns
/// `None` only when no entries exist.
pub(super) fn build_augmented_path(
    home: Option<PathBuf>,
    exe_parent: Option<PathBuf>,
    shell_path: Option<String>,
) -> Option<String> {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(home) = home {
        parts.push(home.join(".local").join("bin"));
    }
    if let Some(parent) = exe_parent {
        parts.push(parent);
    }
    if let Some(shell_path) = shell_path {
        parts.extend(std::env::split_paths(&shell_path));
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
    use super::build_augmented_path;
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
        );
        assert_eq!(
            result.as_deref(),
            Some(
                "/home/agent/.local/bin:/Applications/Buzz.app/Contents/MacOS:\
/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin"
            ),
        );
    }

    #[test]
    fn none_when_no_inputs() {
        assert_eq!(build_augmented_path(None, None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn shell_path_only() {
        let result = build_augmented_path(None, None, Some("/usr/bin:/bin".to_string()));
        assert_eq!(result.as_deref(), Some("/usr/bin:/bin"));
    }
}
