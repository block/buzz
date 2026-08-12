use std::{path::Path, process::Command};

use tauri::AppHandle;

use crate::managed_agents::{FilesystemIsolationRun, ManagedAgentRecord};

pub(super) fn prepared_command(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    resolved_acp_command: &Path,
) -> Result<(Command, Option<FilesystemIsolationRun>), String> {
    // The boundary wraps the outer buzz-acp process, not the inner runtime.
    // That is the only shared spawn seam inherited by the harness, ACP
    // runtime, MCP/tool servers, shells, and background descendants.
    match &record.filesystem_isolation {
        Some(profile) => {
            let (command, run) = crate::managed_agents::consume_prepared_isolated_agent_command(
                profile,
                &record.pubkey,
                &super::current_instance_id(app),
                resolved_acp_command,
            )?;
            Ok((command, Some(run)))
        }
        None => {
            let mut command = Command::new(resolved_acp_command);
            if let Some(home) = crate::managed_agents::default_agent_workdir() {
                command.current_dir(home);
            }
            Ok((command, None))
        }
    }
}
