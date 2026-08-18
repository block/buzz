use std::path::PathBuf;

/// Collect executable login-shell candidates for the current platform.
///
/// Unix checks the historical `/bin` locations before PATH-backed non-FHS
/// locations. Windows uses Git Bash for callers that require `-l -c`.
pub(crate) fn login_shell_candidates() -> Vec<PathBuf> {
    #[cfg(not(windows))]
    {
        let mut candidates = vec![PathBuf::from("/bin/zsh"), PathBuf::from("/bin/bash")];
        candidates.extend(super::path_candidates_from_env("zsh"));
        candidates.extend(super::path_candidates_from_env("bash"));
        candidates.retain(|candidate| super::is_executable_file(candidate));
        candidates.dedup();
        candidates
    }
    #[cfg(windows)]
    {
        crate::managed_agents::git_bash::resolve_bash_path()
            .into_iter()
            .collect()
    }
}
