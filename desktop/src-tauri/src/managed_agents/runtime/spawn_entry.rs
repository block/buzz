use crate::managed_agents::ManagedAgentRecord;
use tauri::AppHandle;

/// Spawn an agent process without holding any locks on records or runtimes.
/// Returns the child process and log path on success. The caller is responsible
/// for updating `ManagedAgentRecord` fields and inserting into the runtimes map.
///
/// `owner_hex`: the workspace owner's pubkey, used as a fallback for legacy
/// records that have no NIP-OA `auth_tag`. See `build_respond_to_env`.
pub fn spawn_agent_child(
    app: &AppHandle,
    record: &ManagedAgentRecord,
    relay_url: &str,
    lazy: bool,
    owner_hex: Option<&str>,
) -> Result<crate::managed_agents::ManagedAgentProcess, String> {
    // Durable Stop fences config-driven and legacy starts too. Hold the OS lock
    // through spawn so a concurrent executor cannot persist Stop between check
    // and process creation.
    let _fence =
        crate::managed_agents::execution::legacy_spawn_guard(app, record, relay_url, owner_hex)?;
    super::spawn_agent_child_for_run(app, record, relay_url, lazy, owner_hex, None)
}

pub(super) fn child_rust_log_filter() -> String {
    match std::env::var("RUST_LOG") {
        Ok(existing) if existing.contains("buzz_acp") => existing,
        Ok(existing) if !existing.trim().is_empty() => format!("{existing},buzz_acp=info"),
        _ => "buzz_acp=info".to_string(),
    }
}

pub(super) fn launcher_generation(generation: Option<(&str, &str)>) -> Result<String, String> {
    let run = generation
        .map(|(run, _)| run.to_owned())
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
    if !buzz_core_pkg::host_execution::hex_id(&run, 32) {
        return Err("invalid launcher generation".into());
    }
    Ok(run)
}
