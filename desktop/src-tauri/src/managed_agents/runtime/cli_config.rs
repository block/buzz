use std::process::Command;

use crate::managed_agents::{resolve_command, KnownAcpRuntime};

use super::should_skip_claude_executable;

pub(crate) fn configure_runtime_cli(command: &mut Command, runtime: Option<&KnownAcpRuntime>) {
    let Some(runtime) = runtime else {
        return;
    };
    if runtime.id != "claude" {
        return;
    }
    if let Some(cli_path) = runtime.underlying_cli.and_then(resolve_command) {
        // Windows batch shims cannot be passed directly to CreateProcess.
        // Let the adapter use PATH when the resolved CLI is a shim.
        if should_skip_claude_executable(&cli_path, cfg!(windows)) {
            return;
        }
        command.env("CLAUDE_CODE_EXECUTABLE", cli_path);
    }
}
