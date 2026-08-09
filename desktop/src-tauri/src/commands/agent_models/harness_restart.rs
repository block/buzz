use tauri::AppHandle;

use crate::managed_agents::ManagedAgentRuntimeKey;

pub(super) fn should_restart_after_harness_edit(
    previous_command: &str,
    current_command: &str,
    auto_restart_enabled: bool,
    live_pair_count: usize,
) -> bool {
    auto_restart_enabled && live_pair_count > 0 && previous_command != current_command
}

pub(super) fn restart_live_pairs(
    app: &AppHandle,
    pubkey: &str,
    restart_keys: Vec<ManagedAgentRuntimeKey>,
) {
    for key in restart_keys {
        if let Err(error) = crate::managed_agents::restart_managed_agent_runtime(
            pubkey.to_string(),
            key.relay_url,
            app.clone(),
        ) {
            eprintln!(
                "buzz-desktop: saved harness change for {pubkey} but failed to restart its live pair: {error}"
            );
        }
    }
}
