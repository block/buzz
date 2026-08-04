//! Runtime-only managed-agent process state.

use std::{path::PathBuf, process::Child};

use super::AcpAvailabilityStatus;

#[derive(Debug)]
pub struct ManagedAgentProcess {
    pub child: Child,
    pub log_path: PathBuf,
    /// Credential-bearing structured MCP file for this launch. `buzz-acp`
    /// deletes it immediately after reading; retained as a cleanup backstop.
    pub project_mcp_config_path: Option<PathBuf>,
    /// Digest of the effective spawn config at launch.
    pub spawn_config_hash: u64,
    /// Whether this process was spawned in setup-listener mode.
    pub setup_mode: bool,
    /// Adapter availability status stamped at spawn time.
    pub adapter_availability: Option<AcpAvailabilityStatus>,
    /// Unpredictable identity shared only with this harness generation.
    pub start_nonce: String,
    /// Win32 Job Object owning the harness and its process tree.
    #[cfg(windows)]
    pub job: Option<crate::managed_agents::JobHandle>,
}

impl Drop for ManagedAgentProcess {
    fn drop(&mut self) {
        if let Some(path) = self.project_mcp_config_path.as_deref() {
            let _ =
                crate::managed_agents::project_connections::remove_agent_project_connection_config(
                    path,
                );
        }
    }
}
