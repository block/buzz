//! PATH augmentation for launched managed-agent child processes.

use std::path::PathBuf;

/// Return `true` when `path` is a Windows batch shim (`.cmd` or `.bat`,
/// case-insensitive) that cannot be passed directly to `CreateProcess`.
///
/// Extracted as a pure function so it can be unit-tested on any host without
/// touching the global PATH or `resolve_command` cache (issue #2397).
pub(crate) fn is_batch_shim(path: &std::path::Path) -> bool {
    path.extension()
        .map(|ext| {
            let lower = ext.to_string_lossy().to_lowercase();
            lower == "cmd" || lower == "bat"
        })
        .unwrap_or(false)
}

/// Return `true` when the resolved CLI path should be skipped for
/// `CLAUDE_CODE_EXECUTABLE` assignment.
///
/// On Windows, `.cmd`/`.bat` batch shims cannot be passed directly to
/// `CreateProcess` (EINVAL, issue #2397). On non-Windows those extensions are
/// valid executables and must not be suppressed — the `is_windows` flag keeps
/// this decision testable cross-host on macOS CI.
pub(crate) fn should_skip_claude_executable(path: &std::path::Path, is_windows: bool) -> bool {
    is_windows && is_batch_shim(path)
}

/// Decide whether the inherited process PATH should be appended to the
/// composed PATH.
///
/// On Windows, `login_shell_path()` always returns `None` because Git Bash
/// returns POSIX colon-delimited paths that poison native children.
/// `Command::env("PATH", …)` replaces rather than extends, so without the
/// inherited PATH every child loses node/npm/git.
///
/// This pure function takes an explicit `is_windows` flag so it can be
/// unit-tested cross-host (macOS CI can pass `true` to exercise the Windows
/// policy without needing the `cfg!(windows)` target).
///
/// Rules:
/// - Only append when `is_windows` — on Unix the login-shell PATH always covers
///   the needed runtimes.
/// - Suppress when `had_shell_path` is `true` — if a login-shell PATH was
///   supplied it already carries the user's native entries; appending the
///   process PATH would double them.
/// - Suppress when `has_local_context` is `false` — callers that pass no home
///   or exe-parent context must not receive a PATH manufactured from ambient
///   process state alone.
pub(crate) fn should_use_inherited(
    had_shell_path: bool,
    has_local_context: bool,
    is_windows: bool,
) -> bool {
    is_windows && !had_shell_path && has_local_context
}

/// Pure PATH composition kernel shared by the install shell and the runtime/probe paths.
///
/// Merges already-split PATH entries in precedence order:
///   1. `managed` — Buzz-controlled dirs (highest precedence, e.g. managed Node/npm bins)
///   2. `login`   — login-shell PATH entries (split before calling)
///   3. `inherited` — current-process PATH entries (split before calling), appended
///      only when `use_inherited` is `true`
///
/// Callers are responsible for splitting raw PATH strings and for prepending any
/// additional prefix entries (e.g. `home/.local/bin`, `nvm`, `exe_parent`) before
/// passing them in `managed`.  `split_paths`/`join_paths` are kept at the wrapper
/// boundaries so this function remains fully pure and testable on any host.
pub(crate) fn compose_path_entries(
    managed: Vec<PathBuf>,
    login: Vec<PathBuf>,
    inherited: Vec<PathBuf>,
    use_inherited: bool,
) -> Vec<PathBuf> {
    let mut parts = managed;
    parts.extend(login);
    if use_inherited {
        parts.extend(inherited);
    }
    parts
}

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
    let home_added = home.is_some();
    let exe_added = exe_parent.is_some();
    let has_local_context = home_added || exe_added;

    // Build the managed/prefix entries (everything before login-shell PATH).
    let mut managed: Vec<PathBuf> = Vec::new();
    if let Some(home) = home {
        managed.push(home.join(".local").join("bin"));
    }
    // Only add managed runtime dirs when a home or executable context exists.
    // This keeps tests/utility callers that intentionally pass no local context
    // from manufacturing a PATH out of ambient platform dirs alone.
    if has_local_context {
        if let Some(managed_npm_bin) = crate::managed_agents::buzz_managed_npm_bin_dir() {
            managed.push(managed_npm_bin);
        }
        if let Some(managed_node_bin) = crate::managed_agents::buzz_managed_node_bin_dir() {
            managed.push(managed_node_bin);
        }
    }
    if let Some(nvm_bin) = nvm_bin {
        managed.push(nvm_bin);
    }
    if let Some(parent) = exe_parent {
        managed.push(parent);
    }

    // Split the login-shell PATH into individual entries.
    let had_shell_path = shell_path.is_some();
    let login: Vec<PathBuf> = shell_path
        .as_deref()
        .map(|s| std::env::split_paths(s).collect())
        .unwrap_or_default();

    let inherited: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let use_inherited = should_use_inherited(had_shell_path, has_local_context, cfg!(windows));

    let parts = compose_path_entries(managed, login, inherited, use_inherited);
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
    /// fix.
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
        assert!(
            result.ends_with(":/usr/local/bin:/usr/bin:/bin"),
            "Unix output must not append process PATH: {result}"
        );
    }

    /// On Windows: when no login-shell PATH is available, `build_augmented_path`
    /// must append the inherited process PATH so node/npm remain visible.
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

    /// On Windows: when a login-shell PATH IS supplied, the process PATH must
    /// NOT also be appended.
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

    /// On Windows: when no local context is provided, the function must return
    /// None even if the process PATH is set.
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

// ── Pure policy and composition tests — run on every host ────────────────────
//
// These test `should_use_inherited` and `compose_path_entries` with explicit
// inputs, so they run on macOS/Linux CI and validate the Windows policy
// behavior without touching process state or requiring a Windows target.
#[cfg(test)]
mod compose_tests {
    use super::{compose_path_entries, is_batch_shim, should_use_inherited};
    use std::path::{Path, PathBuf};

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    // ── should_use_inherited policy matrix ────────────────────────────────────

    /// Windows + no shell path + has local context → must use inherited.
    #[test]
    fn policy_windows_no_shell_with_context_uses_inherited() {
        assert!(
            should_use_inherited(false, true, true),
            "Windows, no shell path, has context → must append inherited"
        );
    }

    /// Windows + shell path present → must NOT use inherited (login path covers it).
    #[test]
    fn policy_windows_shell_path_present_suppresses_inherited() {
        assert!(
            !should_use_inherited(true, true, true),
            "Windows, shell path present → must not append inherited"
        );
    }

    /// Windows + no local context → must NOT use inherited (no ambient state).
    #[test]
    fn policy_windows_no_local_context_suppresses_inherited() {
        assert!(
            !should_use_inherited(false, false, true),
            "Windows, no local context → must not append inherited"
        );
    }

    /// Non-Windows → never use inherited, regardless of other flags.
    #[test]
    fn policy_non_windows_never_uses_inherited() {
        assert!(
            !should_use_inherited(false, true, false),
            "non-Windows must never append inherited PATH"
        );
        assert!(
            !should_use_inherited(false, false, false),
            "non-Windows + no context must never append inherited PATH"
        );
    }

    // ── compose_path_entries ordering ─────────────────────────────────────────

    #[test]
    fn managed_entries_appear_first() {
        let managed = vec![p("/buzz/node/bin"), p("/buzz/npm/bin")];
        let login = vec![p("/usr/local/bin"), p("/usr/bin")];
        let result = compose_path_entries(managed, login, vec![], false);
        assert_eq!(result[0], p("/buzz/node/bin"), "managed[0] must be first");
        assert_eq!(result[1], p("/buzz/npm/bin"), "managed[1] must be second");
        assert_eq!(
            result[2],
            p("/usr/local/bin"),
            "login[0] must follow managed"
        );
    }

    #[test]
    fn login_path_suppresses_inherited_when_use_inherited_false() {
        let login = vec![p("/usr/local/bin")];
        let inherited = vec![p("/should/not/appear")];
        let result = compose_path_entries(vec![], login, inherited, false);
        assert!(
            !result.contains(&p("/should/not/appear")),
            "inherited must not appear when use_inherited=false"
        );
    }

    #[test]
    fn inherited_appended_last_when_use_inherited_true() {
        let managed = vec![p("/buzz/npm/bin")];
        let inherited = vec![p("C:/windows/node"), p("C:/windows/npm")];
        let result = compose_path_entries(managed, vec![], inherited.clone(), true);
        assert_eq!(result[0], p("/buzz/npm/bin"), "managed must be first");
        assert_eq!(
            &result[1..],
            &inherited[..],
            "inherited entries must be appended last"
        );
    }

    /// Windows policy ON + empty inherited PATH — should produce just managed
    /// entries, not None and not a phantom segment.
    #[test]
    fn windows_policy_on_empty_inherited_produces_managed_only() {
        let managed = vec![p("/buzz/npm/bin")];
        let result = compose_path_entries(managed.clone(), vec![], vec![], true);
        assert_eq!(
            result, managed,
            "empty inherited must not add phantom entries"
        );
    }

    /// Windows policy ON + unset/absent inherited (empty vec from var_os None) —
    /// same result as above; no crash, no phantom.
    #[test]
    fn windows_policy_on_unset_inherited_path_produces_managed_only() {
        // Simulates std::env::var_os("PATH") returning None → empty vec.
        let managed = vec![p("/buzz/npm/bin")];
        let inherited: Vec<PathBuf> = vec![]; // empty, as if PATH is unset
        let result = compose_path_entries(managed.clone(), vec![], inherited, true);
        assert_eq!(result, managed);
    }

    /// No local context + Windows policy ON — compose_path_entries itself still
    /// works (no crash), and the caller is responsible for not calling it.
    /// Specifically: all-empty inputs with use_inherited=true still returns empty.
    #[test]
    fn all_empty_with_use_inherited_true_returns_empty() {
        let result = compose_path_entries(vec![], vec![], vec![], true);
        assert!(
            result.is_empty(),
            "all-empty inputs must produce empty output"
        );
    }

    #[test]
    fn empty_all_inputs_use_inherited_false_returns_empty() {
        let result = compose_path_entries(vec![], vec![], vec![], false);
        assert!(
            result.is_empty(),
            "all-empty inputs must produce empty output"
        );
    }

    /// Non-Windows behavior: `use_inherited=false` must produce byte-identical
    /// output to before this fix. Inherited entries are collected but dropped.
    #[cfg(unix)]
    #[test]
    fn unix_use_inherited_false_output_unchanged() {
        let managed = vec![p("/buzz/npm/bin")];
        let login = vec![p("/usr/local/bin"), p("/usr/bin"), p("/bin")];
        let inherited = vec![p("/proc/ambient/PATH")]; // would be real proc PATH on Unix
        let result = compose_path_entries(managed, login, inherited, false);
        assert_eq!(
            result,
            vec![
                p("/buzz/npm/bin"),
                p("/usr/local/bin"),
                p("/usr/bin"),
                p("/bin")
            ],
            "Unix output must not include inherited entries when use_inherited=false"
        );
    }

    // ── Structural wrapper-alignment test ──────────────────────────────────────
    //
    // Verifies that both `build_augmented_path` and `install_shell_command`
    // compute the same `should_use_inherited` decision for equivalent inputs.
    // Tests the policy function directly to confirm the wrappers can't drift.

    /// Exhaustive truth-table for `should_use_inherited` — all four input
    /// combinations that affect real callers. Confirms the policy is correct
    /// before either wrapper binds to it.
    #[test]
    fn should_use_inherited_policy_truth_table() {
        // (had_shell, has_context, is_windows) → expected
        let cases = [
            (false, true, true, true),   // Windows, no shell, context → USE
            (true, true, true, false),   // Windows, shell present → NO
            (false, false, true, false), // Windows, no context → NO
            (false, true, false, false), // non-Windows → NO
        ];
        for (had_shell, has_ctx, is_win, expected) in cases {
            let result = should_use_inherited(had_shell, has_ctx, is_win);
            assert_eq!(
                result, expected,
                "policy mismatch: had_shell={had_shell} has_ctx={has_ctx} is_win={is_win}"
            );
        }
    }

    // ── is_batch_shim extension tests ─────────────────────────────────────────

    #[test]
    fn batch_shim_cmd_lower() {
        assert!(is_batch_shim(Path::new("claude.cmd")));
    }

    #[test]
    fn batch_shim_cmd_upper() {
        assert!(is_batch_shim(Path::new("claude.CMD")));
    }

    #[test]
    fn batch_shim_bat_lower() {
        assert!(is_batch_shim(Path::new("claude.bat")));
    }

    #[test]
    fn batch_shim_bat_upper() {
        assert!(is_batch_shim(Path::new("claude.BAT")));
    }

    #[test]
    fn batch_shim_exe_not_shim() {
        assert!(!is_batch_shim(Path::new("claude.exe")));
    }

    #[test]
    fn batch_shim_no_extension_not_shim() {
        assert!(!is_batch_shim(Path::new("claude")));
    }

    // ── should_skip_claude_executable policy tests ────────────────────────────
    //
    // Cross-host policy: shim + Windows → skip; shim + non-Windows → assign;
    // non-shim either OS → assign. Mirrors the `should_use_inherited` pattern.

    #[test]
    fn skip_claude_executable_shim_windows_returns_true() {
        assert!(
            super::should_skip_claude_executable(Path::new("claude.cmd"), true),
            "shim + windows=true must skip"
        );
        assert!(
            super::should_skip_claude_executable(Path::new("claude.BAT"), true),
            "shim + windows=true must skip"
        );
    }

    #[test]
    fn skip_claude_executable_shim_non_windows_returns_false() {
        assert!(
            !super::should_skip_claude_executable(Path::new("claude.cmd"), false),
            "shim + windows=false must NOT skip (valid executable on non-Windows)"
        );
        assert!(
            !super::should_skip_claude_executable(Path::new("claude.bat"), false),
            "shim + windows=false must NOT skip"
        );
    }

    #[test]
    fn skip_claude_executable_exe_both_platforms_returns_false() {
        assert!(
            !super::should_skip_claude_executable(Path::new("claude.exe"), true),
            "non-shim + windows=true must NOT skip"
        );
        assert!(
            !super::should_skip_claude_executable(Path::new("claude.exe"), false),
            "non-shim + windows=false must NOT skip"
        );
    }

    #[test]
    fn skip_claude_executable_no_ext_both_platforms_returns_false() {
        assert!(
            !super::should_skip_claude_executable(Path::new("claude"), true),
            "no-ext + windows=true must NOT skip"
        );
        assert!(
            !super::should_skip_claude_executable(Path::new("claude"), false),
            "no-ext + windows=false must NOT skip"
        );
    }
}
