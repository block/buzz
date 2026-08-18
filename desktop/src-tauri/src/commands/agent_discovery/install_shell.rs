/// Resolve the shell binary for install commands.
///
/// Unix prefers the historical `/bin` locations, then falls back to PATH for
/// non-FHS distributions. Windows uses Git Bash because install commands use
/// bash-only `-l -c` syntax.
pub(super) fn resolve_install_shell() -> Result<std::path::PathBuf, String> {
    #[cfg(not(windows))]
    {
        for shell in ["/bin/zsh", "/bin/bash"] {
            if std::path::Path::new(shell).exists() {
                return Ok(std::path::PathBuf::from(shell));
            }
        }
        crate::managed_agents::login_shell_candidates()
            .into_iter()
            .next()
            .ok_or_else(|| "No zsh or bash executable was found".to_string())
    }

    #[cfg(windows)]
    {
        install_shell_from(crate::managed_agents::git_bash::resolve_bash_path())
    }
}

/// Map a resolved Git Bash path to the install-shell result.
#[cfg(windows)]
pub(crate) fn install_shell_from(
    resolved: Option<std::path::PathBuf>,
) -> Result<std::path::PathBuf, String> {
    resolved.ok_or_else(|| crate::managed_agents::git_bash::GIT_BASH_INSTALL_HINT.to_string())
}
