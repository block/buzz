//! Phase-1 underlying CLI install / repair for ACP runtime setup.

use crate::managed_agents::{
    readiness::cli_probe, InstallRuntimeResult, InstallStepResult, KnownAcpRuntime,
};

use super::run_install_command_with_retry;

/// Install a missing underlying CLI, or repair an outdated Claude Code build.
///
/// Returns `Err(result)` when a step fails (caller should return that result
/// immediately). Returns `Ok(steps)` with zero or more completed install steps.
pub(super) fn install_or_repair_underlying_cli(
    runtime_id: &str,
    runtime: &KnownAcpRuntime,
) -> Result<Vec<InstallStepResult>, InstallRuntimeResult> {
    let Some(cli) = runtime.underlying_cli else {
        return Ok(Vec::new());
    };

    let mut steps = Vec::new();
    match crate::managed_agents::resolve_command(cli) {
        None => {
            for cmd in runtime.cli_install_commands_for_os() {
                let result = run_install_command_with_retry("cli", cmd);
                let success = result.success;
                steps.push(result);
                if !success {
                    return Err(InstallRuntimeResult {
                        success: false,
                        steps,
                        restarted_count: 0,
                        failed_restart_count: 0,
                    });
                }
            }
        }
        // Claude Code may already be present but too old to expose
        // `claude auth status` (older builds treat those args as a prompt).
        // Repair with `claude update` before adapter install so onboarding
        // auth probes can succeed.
        Some(cli_path) if runtime_id == "claude" => {
            let augmented = cli_probe::augmented_path();
            if cli_probe::claude_auth_status_needs_upgrade(&cli_path, augmented.as_deref()) {
                let result = run_install_command_with_retry("cli", "claude update");
                let success = result.success;
                steps.push(result);
                if !success {
                    return Err(InstallRuntimeResult {
                        success: false,
                        steps,
                        restarted_count: 0,
                        failed_restart_count: 0,
                    });
                }
                crate::managed_agents::clear_resolve_cache();
            }
        }
        Some(_) => {}
    }

    Ok(steps)
}
