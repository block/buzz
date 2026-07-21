//! Install-shell selection and PATH composition for runtime install commands.

/// Build a login-shell `Command` for `command` with hermit env vars stripped,
/// Buzz-managed npm locations set, and the user's PATH set. This is the
/// single source of truth for the shell selection and environment cleanup
/// shared by `run_install_command` and managed npm install path — keeping them
/// in sync so the hermit-strip list can't drift between command execution paths.
///
/// On Windows, resolves Git Bash via `resolve_bash_path` (skips `BUZZ_SHELL`
/// since install commands require bash syntax). Returns `Err` when no shell
/// can be found.
pub(super) fn install_shell_command(command: &str) -> Result<std::process::Command, String> {
    let shell: std::path::PathBuf = resolve_install_shell()?;

    let mut cmd = std::process::Command::new(&shell);
    cmd.args(["-l", "-c", command]);

    // Strip hermit env vars so npm/node use the user's normal registry rather
    // than the project-local hermit-managed paths, then give npm defaults for
    // Buzz-owned app data. Adapter install commands also pass --prefix
    // explicitly; these env vars keep subprocesses/cache/corepack aligned.
    cmd.env_remove("NPM_CONFIG_PREFIX");
    cmd.env_remove("NPM_CONFIG_CACHE");
    cmd.env_remove("COREPACK_HOME");

    if let Some(prefix) = crate::managed_agents::buzz_managed_npm_prefix() {
        cmd.env("NPM_CONFIG_PREFIX", &prefix);
        cmd.env("npm_config_prefix", &prefix);
        cmd.env("COREPACK_HOME", prefix.join("corepack"));
        cmd.env("NPM_CONFIG_CACHE", prefix.join("cache"));
        cmd.env("npm_config_cache", prefix.join("cache"));
    }

    #[cfg(windows)]
    let well_known = crate::managed_agents::windows_existing_well_known_path_dirs();
    #[cfg(not(windows))]
    let well_known: Vec<std::path::PathBuf> = Vec::new();

    let path_parts = install_shell_path_parts(
        crate::managed_agents::buzz_managed_node_bin_dir(),
        crate::managed_agents::buzz_managed_npm_bin_dir(),
        well_known,
        crate::managed_agents::login_shell_path().as_deref(),
        std::env::var_os("PATH"),
    );
    if !path_parts.is_empty() {
        if let Ok(path) = std::env::join_paths(path_parts) {
            cmd.env("PATH", path);
        }
    }

    // Detach from the controlling terminal so install scripts that read from
    // /dev/tty (e.g. Codex's "Start Codex now? [y/N]") fall back to stdin
    // (which is /dev/null) instead of blocking forever.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

    crate::windows_console::hide_console(&mut cmd);

    Ok(cmd)
}

/// Compose PATH for install shell children.
///
/// Order:
/// 1. managed Node bin
/// 2. managed npm bin
/// 3. well-known Windows Node/npm/goose dirs that exist on disk
/// 4. login-shell PATH (Unix) **or** process PATH when login-shell PATH is
///    unavailable (Windows)
///
/// Windows must not drop the process PATH: `login_shell_path()` is intentionally
/// `None` there (POSIX PATH poison), and without this fallback npm-backed
/// adapter installs only see the empty managed prefix (#2238). Well-known dirs
/// cover official `%ProgramFiles%\nodejs`, nvm-windows `NVM_SYMLINK` /
/// `NVM_HOME`, and similar when the GUI process PATH is thin.
pub(super) fn install_shell_path_parts(
    managed_node_bin: Option<std::path::PathBuf>,
    managed_npm_bin: Option<std::path::PathBuf>,
    well_known_dirs: impl IntoIterator<Item = std::path::PathBuf>,
    login_shell_path: Option<&str>,
    process_path: Option<std::ffi::OsString>,
) -> Vec<std::path::PathBuf> {
    let mut path_parts = Vec::new();
    if let Some(managed_node_bin) = managed_node_bin {
        path_parts.push(managed_node_bin);
    }
    if let Some(managed_npm_bin) = managed_npm_bin {
        path_parts.push(managed_npm_bin);
    }
    path_parts.extend(well_known_dirs);
    if let Some(path) = login_shell_path {
        path_parts.extend(std::env::split_paths(path));
    } else if let Some(path) = process_path {
        path_parts.extend(std::env::split_paths(&path));
    }
    // Dedup while preserving order — well-known dirs often already appear on PATH.
    let mut seen = std::collections::HashSet::new();
    path_parts
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

/// Resolve the shell binary for install commands.
///
/// Unix: `/bin/zsh` if present, else `/bin/bash`.
/// Windows: Git Bash via `resolve_bash_path` — skips `BUZZ_SHELL` because install
/// commands use bash-only `-l -c` syntax. A `BUZZ_SHELL=pwsh` user gets a green
/// Doctor prereq (their agents work) but installs use the Git Bash fallback chain.
pub(super) fn resolve_install_shell() -> Result<std::path::PathBuf, String> {
    #[cfg(not(windows))]
    {
        if std::path::Path::new("/bin/zsh").exists() {
            return Ok(std::path::PathBuf::from("/bin/zsh"));
        }
        Ok(std::path::PathBuf::from("/bin/bash"))
    }

    #[cfg(windows)]
    {
        install_shell_from(crate::managed_agents::git_bash::resolve_bash_path())
    }
}

/// Pure mapping from a resolved bash path to the install-shell result.
/// `None` → `Err(GIT_BASH_INSTALL_HINT)`, `Some(path)` → `Ok(path)`.
#[cfg(windows)]
pub(crate) fn install_shell_from(
    resolved: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, String> {
    resolved.ok_or_else(|| crate::managed_agents::git_bash::GIT_BASH_INSTALL_HINT.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_install_shell_path_falls_back_to_process_path_without_login_shell() {
        let managed = std::path::PathBuf::from("/tmp/buzz-node-tools");
        // Platform-native paths: Windows drive letters contain `:`, which
        // join_paths rejects on Unix (Codex P1 / Desktop Core CI failure).
        let process = std::env::join_paths([
            std::path::Path::new("/opt/nodejs/bin"),
            std::path::Path::new("/usr/bin"),
        ])
        .expect("join process PATH");

        let parts = install_shell_path_parts(
            None,
            Some(managed.clone()),
            std::iter::empty(),
            None, // Windows: login_shell_path() is None
            Some(process),
        );

        assert_eq!(parts.first(), Some(&managed), "managed npm bin first");
        assert!(
            parts
                .iter()
                .any(|p| p == std::path::Path::new("/opt/nodejs/bin")),
            "process PATH entries must remain visible for system npm; got {parts:?}"
        );
    }

    #[test]
    fn test_install_shell_path_prepends_well_known_dirs_before_process_path() {
        let well_known = vec![std::path::PathBuf::from("/opt/nodejs/bin")];
        let process = std::env::join_paths([std::path::Path::new("/usr/bin")]).expect("join PATH");
        let parts = install_shell_path_parts(None, None, well_known, None, Some(process));
        assert_eq!(
            parts.first().map(|p| p.as_path()),
            Some(std::path::Path::new("/opt/nodejs/bin")),
            "well-known Node dir should precede process PATH; got {parts:?}"
        );
        assert!(
            parts.iter().any(|p| p == std::path::Path::new("/usr/bin")),
            "process PATH must still be included; got {parts:?}"
        );
    }

    #[test]
    fn test_install_shell_path_dedups_well_known_already_on_process_path() {
        let well_known = vec![std::path::PathBuf::from("/opt/nodejs/bin")];
        let process = std::env::join_paths([
            std::path::Path::new("/opt/nodejs/bin"),
            std::path::Path::new("/usr/bin"),
        ])
        .expect("join process PATH");
        let parts = install_shell_path_parts(None, None, well_known, None, Some(process));
        let node_count = parts
            .iter()
            .filter(|p| p.as_path() == std::path::Path::new("/opt/nodejs/bin"))
            .count();
        assert_eq!(
            node_count, 1,
            "duplicate PATH entries must be collapsed; got {parts:?}"
        );
    }

    #[test]
    fn test_install_shell_path_prefers_login_shell_over_process_path() {
        // Build platform-native PATH strings so split_paths works on Windows (;) and Unix (:).
        let login = std::env::join_paths([
            std::path::Path::new("/usr/local/bin"),
            std::path::Path::new("/usr/bin"),
        ])
        .expect("join login PATH");
        let login = login.to_string_lossy();
        let process = std::env::join_paths([std::path::Path::new("/should/not/appear")])
            .expect("join process PATH");
        let parts = install_shell_path_parts(
            None,
            None,
            std::iter::empty(),
            Some(login.as_ref()),
            Some(process),
        );
        assert!(
            parts
                .iter()
                .any(|p| p == std::path::Path::new("/usr/local/bin")),
            "login-shell PATH should win when present; got {parts:?}"
        );
        assert!(
            !parts
                .iter()
                .any(|p| p == std::path::Path::new("/should/not/appear")),
            "process PATH must not be mixed in when login-shell PATH is present"
        );
    }

    /// On Unix, resolve_install_shell always succeeds (returns zsh or bash).
    #[cfg(unix)]
    #[test]
    fn test_resolve_install_shell_succeeds_on_unix() {
        let result = resolve_install_shell();
        assert!(result.is_ok(), "Unix must always resolve a shell");
        let shell = result.unwrap();
        assert!(
            shell == std::path::Path::new("/bin/zsh") || shell == std::path::Path::new("/bin/bash"),
            "expected /bin/zsh or /bin/bash, got {shell:?}"
        );
    }

    /// install_shell_command returns a valid Command on Unix.
    #[cfg(unix)]
    #[test]
    fn test_install_shell_command_returns_ok_on_unix() {
        let result = install_shell_command("echo test");
        assert!(result.is_ok(), "install_shell_command must succeed on Unix");
    }

    /// On Windows (CI runner has Git pre-installed), resolve_install_shell succeeds.
    #[cfg(windows)]
    #[test]
    fn test_resolve_install_shell_succeeds_on_windows_with_git() {
        let result = resolve_install_shell();
        assert!(
            result.is_ok(),
            "Windows CI runner has Git — resolve_install_shell must succeed; got: {:?}",
            result.err()
        );
        let shell = result.unwrap();
        let fname = shell.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert!(
            fname.eq_ignore_ascii_case("bash.exe"),
            "Windows install shell must be bash.exe, got: {shell:?}"
        );
    }

    /// On Windows, when no Git Bash is found, the error carries the Doctor hint.
    #[cfg(windows)]
    #[test]
    fn test_resolve_install_shell_error_contains_doctor_hint() {
        let hint = crate::managed_agents::git_bash::GIT_BASH_INSTALL_HINT;
        assert!(
            hint.contains("Git for Windows"),
            "GIT_BASH_INSTALL_HINT must mention Git for Windows; got: {hint}"
        );
        assert!(
            hint.contains("PATH"),
            "GIT_BASH_INSTALL_HINT must mention PATH option; got: {hint}"
        );
    }

    /// install_shell_command returns a valid Command on Windows.
    #[cfg(windows)]
    #[test]
    fn test_install_shell_command_returns_ok_on_windows() {
        let result = install_shell_command("echo test");
        assert!(
            result.is_ok(),
            "install_shell_command must succeed on Windows with Git; got: {:?}",
            result.err()
        );
    }

    /// When `resolve_bash_path` returns `None` (no Git Bash found),
    /// `install_shell_from` maps it to `Err(GIT_BASH_INSTALL_HINT)`.
    #[cfg(windows)]
    #[test]
    fn test_install_shell_from_none_returns_hint() {
        use crate::managed_agents::git_bash;

        let result = install_shell_from(None);
        assert_eq!(
            result,
            Err(git_bash::GIT_BASH_INSTALL_HINT.to_string()),
            "install_shell_from(None) must return the Git Bash install hint"
        );
    }

    /// When `resolve_bash_path` returns `Some(path)`, `install_shell_from`
    /// passes it through as `Ok`.
    #[cfg(windows)]
    #[test]
    fn test_install_shell_from_some_returns_path() {
        let path = std::path::PathBuf::from(r"C:\Git\bin\bash.exe");
        let result = install_shell_from(Some(path.clone()));
        assert_eq!(
            result,
            Ok(path),
            "install_shell_from(Some) must return the path as Ok"
        );
    }
}
