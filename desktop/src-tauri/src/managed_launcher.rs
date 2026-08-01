//! Windows managed-agent launcher entry point and contained-process boundary.

use std::process::{Command, ExitStatus};

/// Run launcher mode before Tauri starts.
///
/// Returns `false` for normal desktop invocations; launcher mode exits after
/// forwarding the contained target's real exit code.
pub fn run_if_requested() -> bool {
    crate::managed_agents::run_managed_agent_launcher_if_requested()
}

/// Opaque handle used by the native boundary proof.
///
/// Construction and finalization deliberately delegate to the same private
/// production functions used by managed-agent lifecycle commands. Keeping the
/// owned Child and Job inaccessible prevents callers from bypassing checked
/// teardown.
#[doc(hidden)]
pub struct ContainedManagedProcess(crate::managed_agents::ManagedAgentProcess);

impl ContainedManagedProcess {
    /// PID of the exact desktop launcher child retained for observation/reaping.
    pub fn launcher_pid(&self) -> u32 {
        self.0.child.id()
    }

    /// Current members of the owned Windows Job.
    pub fn active_process_count(&self) -> Result<usize, String> {
        let job = self
            .0
            .job
            .as_ref()
            .ok_or_else(|| "managed process no longer owns a Windows Job".to_string())?;
        job.members()
            .map(|members| members.len())
            .map_err(|error| format!("failed to query managed Windows Job: {error}"))
    }

    /// Terminate the Job, prove zero membership, reap the exact launcher, and
    /// only then release the Job authority.
    pub fn terminate_checked(&mut self) -> Result<(ExitStatus, usize), String> {
        let status = crate::managed_agents::terminate_managed_agent_process(&mut self.0)?;
        let remaining = self.active_process_count()?;
        if remaining != 0 {
            return Err(format!(
                "managed Windows Job still has {remaining} members after checked termination"
            ));
        }
        crate::managed_agents::release_finalized_managed_agent_process(&mut self.0)?;
        Ok((status, remaining))
    }
}

/// Spawn an original target command through the production launcher envelope
/// and safe `processkit` Job boundary.
///
/// This narrow diagnostic API exists so the native integration test can prove
/// the complete original-command → wrapper → Job → target chain rather than
/// reconstructing private launcher variables itself.
#[doc(hidden)]
pub fn spawn_contained(
    command: Command,
    launcher_exe: &std::path::Path,
    log_path: &std::path::Path,
    environment_cleared: bool,
) -> Result<ContainedManagedProcess, String> {
    crate::managed_agents::spawn_managed_agent_process_with_launcher(
        command,
        launcher_exe,
        log_path.to_path_buf(),
        environment_cleared,
    )
    .map(ContainedManagedProcess)
}
