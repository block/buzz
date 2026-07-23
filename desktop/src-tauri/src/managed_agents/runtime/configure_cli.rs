//! Configure the `CLAUDE_CODE_EXECUTABLE` environment variable for Claude
//! agent spawns.
//!
//! On Windows, `CLAUDE_CODE_EXECUTABLE` must point to an actual executable,
//! not a `.cmd` or `.bat` shim. Passing a shim path causes `CreateProcess`
//! to fail with `EINVAL` because the kernel cannot directly execute a batch
//! script. When the resolved CLI path has a `.cmd` / `.bat` extension we skip
//! setting the variable so the agent's own PATH lookup finds the real binary.

use crate::managed_agents::{resolve_command, KnownAcpRuntime};

/// Set `CLAUDE_CODE_EXECUTABLE` on `command` when the resolved CLI path is
/// safe to pass directly to `CreateProcess` / `execve`.
///
/// Skips the env-var on Windows when the resolved path ends in `.cmd` or
/// `.bat` (case-insensitive) — those are batch shims that spawn a new
/// `cmd.exe` process and cannot be exec'd directly, causing `EINVAL`.
pub(crate) fn configure_runtime_cli(
    command: &mut std::process::Command,
    runtime: Option<&KnownAcpRuntime>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    if runtime.id != "claude" {
        return;
    }
    if let Some(cli_path) = runtime.underlying_cli.and_then(resolve_command) {
        #[cfg(windows)]
        {
            if let Some(ext) = cli_path.extension() {
                let ext_lower = ext.to_string_lossy().to_lowercase();
                if ext_lower == "cmd" || ext_lower == "bat" {
                    // Batch shim — skip CLAUDE_CODE_EXECUTABLE to avoid
                    // CreateProcess EINVAL; the agent will find the real
                    // binary via PATH.
                    return;
                }
            }
        }
        command.env("CLAUDE_CODE_EXECUTABLE", cli_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::known_acp_runtime;

    /// On Windows, a `.cmd` shim must NOT set CLAUDE_CODE_EXECUTABLE because
    /// `CreateProcess` cannot exec batch scripts directly (EINVAL).
    #[cfg(windows)]
    #[test]
    fn cmd_shim_does_not_set_claude_code_executable() {
        let _guard = crate::managed_agents::lock_path_mutex();
        let temp = tempfile::tempdir().expect("temp dir");

        // Write a .cmd shim — no execute bit needed on Windows.
        let shim = temp.path().join("claude.cmd");
        std::fs::write(&shim, "@echo off\r\necho shim\r\n").expect("write shim");

        let original_path = std::env::var_os("PATH");
        std::env::set_var("PATH", temp.path());

        let mut command = std::process::Command::new("buzz-acp");
        configure_runtime_cli(&mut command, known_acp_runtime("claude-agent-acp"));

        if let Some(path) = original_path {
            std::env::set_var("PATH", path);
        } else {
            std::env::remove_var("PATH");
        }

        assert!(
            !command
                .get_envs()
                .any(|(key, _)| key == "CLAUDE_CODE_EXECUTABLE"),
            ".cmd shim must not set CLAUDE_CODE_EXECUTABLE"
        );
    }
}
