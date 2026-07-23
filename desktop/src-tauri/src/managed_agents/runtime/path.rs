//! PATH augmentation for launched managed-agent child processes.

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
///   7. Windows only: the current process `PATH` (appended when no login-shell
///      PATH exists, because callers use `Command::env("PATH", …)` which
///      *replaces* the child's PATH — without this, the child loses node/npm/git
///      and every npm `.cmd` shim fails with `'node' is not recognized`)
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
    let mut parts: Vec<PathBuf> = Vec::new();
    let home_added = home.is_some();
    if let Some(home) = home {
        parts.push(home.join(".local").join("bin"));
    }
    // Only add managed runtime dirs when a home or executable context exists.
    // This keeps tests/utility callers that intentionally pass no local context
    // from manufacturing a PATH out of ambient platform dirs alone.
    let exe_added = exe_parent.is_some();
    if home_added || exe_added {
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
    // Track whether a login-shell PATH was provided; used by the Windows
    // fallback below to avoid appending the process PATH when a shell PATH
    // was already supplied (cfg-gated so the variable is not unused on Unix).
    #[cfg(windows)]
    let had_shell_path = shell_path.is_some();
    if let Some(shell_path) = shell_path {
        parts.extend(std::env::split_paths(&shell_path));
    }

    // On Windows, `login_shell_path()` always returns `None` because Git Bash
    // reports POSIX colon-delimited paths that poison native children.  Nothing
    // above contributes the user's real Windows PATH, and `Command::env("PATH",
    // …)` replaces rather than extends, so every child loses node/npm/git.
    // Append the inherited process PATH here — after the Buzz-managed dirs so
    // those still win — but only when there is local context (home or exe_parent
    // was supplied) to prevent manufacturing a PATH from ambient state alone.
    #[cfg(windows)]
    if !had_shell_path && (home_added || exe_added) {
        if let Some(proc_path) = std::env::var_os("PATH") {
            parts.extend(std::env::split_paths(&proc_path));
        }
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

    /// On Unix, supplying a `shell_path` must NOT trigger the Windows process-PATH
    /// fallback — the output must be byte-identical to what it was before this
    /// fix.  (The `#[cfg(windows)]` block is dead on this platform, but the
    /// `had_shell_path` variable introduced alongside it must not affect non-Windows
    /// output.)
    #[cfg(unix)]
    #[test]
    fn unix_shell_path_output_unchanged_by_windows_fallback_logic() {
        let result = build_augmented_path(
            Some(PathBuf::from("/home/user")),
            None,
            Some("/usr/local/bin:/usr/bin:/bin".to_string()),
            None,
        );
        let result = result.expect("path");
        // Must end exactly with the login-shell PATH — no ambient process PATH
        // appended even though shell_path is set.
        assert!(
            result.ends_with(":/usr/local/bin:/usr/bin:/bin"),
            "Unix output must not append process PATH: {result}"
        );
    }

    /// On Windows: when no login-shell PATH is available, `build_augmented_path`
    /// must append the inherited process PATH so node/npm remain visible.
    ///
    /// This test manipulates `std::env::var_os("PATH")` directly — it must hold
    /// the `lock_path_mutex` to avoid racing with other tests.
    #[cfg(windows)]
    #[test]
    fn windows_appends_process_path_when_no_shell_path() {
        let _guard = crate::managed_agents::lock_path_mutex();
        let previous = std::env::var_os("PATH");
        std::env::set_var("PATH", r"C:\Program Files\nodejs");

        let result = build_augmented_path(Some(PathBuf::from(r"C:\Users\agent")), None, None, None);

        match previous {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }

        let result = result.expect("path must not be None with a home dir");
        assert!(
            result.starts_with(r"C:\Users\agent\.local\bin;"),
            "home/.local/bin must be first: {result}"
        );
        assert!(
            result.ends_with(r";C:\Program Files\nodejs"),
            "process PATH must be last: {result}"
        );
    }

    /// On Windows: when a login-shell PATH IS supplied (hypothetically), the
    /// process PATH must NOT also be appended — that would double the PATH.
    #[cfg(windows)]
    #[test]
    fn windows_does_not_append_process_path_when_shell_path_present() {
        let _guard = crate::managed_agents::lock_path_mutex();
        let previous = std::env::var_os("PATH");
        std::env::set_var("PATH", r"C:\ShouldNotAppear");

        let result = build_augmented_path(
            Some(PathBuf::from(r"C:\Users\agent")),
            None,
            Some(r"C:\Program Files\nodejs".to_string()),
            None,
        );

        match previous {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }

        let result = result.expect("path");
        assert!(
            !result.contains("ShouldNotAppear"),
            "process PATH must not be appended when shell_path is present: {result}"
        );
    }

    /// On Windows: when no local context is provided (home=None, exe_parent=None),
    /// the function must return None even if the process PATH is set — callers
    /// that pass no context must not get a PATH manufactured from ambient state.
    #[cfg(windows)]
    #[test]
    fn windows_no_process_path_without_local_context() {
        let _guard = crate::managed_agents::lock_path_mutex();
        let previous = std::env::var_os("PATH");
        std::env::set_var("PATH", r"C:\Windows\System32");

        let result = build_augmented_path(None, None, None, None);

        match previous {
            Some(value) => std::env::set_var("PATH", value),
            None => std::env::remove_var("PATH"),
        }

        assert_eq!(
            result, None,
            "must return None when no local context and no shell_path"
        );
    }
}
