//! Tauri commands for global agent configuration defaults.
//!
//! `get_global_agent_config` / `set_global_agent_config` — simple load/save
//! around the `global_config` module with the standard save-time validation.
//!
//! `set_global_agent_config` additionally auto-respawns any local agent that
//! was previously in setup-listener mode (i.e. readiness was `NotReady`) but
//! would now satisfy `agent_readiness` with the new global config.  This is
//! the only honest way to deliver new env vars to a running process — the env
//! is baked at spawn time and cannot be mutated in place.

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        agent_readiness, effective_agent_command, known_acp_runtime, load_global_agent_config,
        load_managed_agents, load_personas, process_is_running, resolve_effective_agent_env,
        save_global_agent_config, save_managed_agents, start_managed_agent_process,
        stop_managed_agent_process, validate_global_config, AgentReadiness, BackendKind,
        GlobalAgentConfig,
    },
};

/// Read the current global agent configuration.
///
/// Returns the default (empty) config if `global-agent-config.json` has not
/// been written yet.
#[tauri::command]
pub fn get_global_agent_config(app: AppHandle) -> Result<GlobalAgentConfig, String> {
    load_global_agent_config(&app)
}

/// Validate and persist a new global agent configuration, then auto-respawn
/// any setup-listener agents whose readiness flips to `Ready` under the new
/// config.
///
/// Strips empty env values before writing (empty = "inherit" semantics), then
/// applies standard validation: POSIX key shape, reserved-key reject,
/// derived-provider-model-key reject, NUL/size caps.
///
/// Respawn is best-effort: per-agent errors are logged to stderr but do not
/// fail the command.  The returned value is the round-tripped config from disk.
#[tauri::command]
pub async fn set_global_agent_config(
    config: GlobalAgentConfig,
    app: AppHandle,
) -> Result<GlobalAgentConfig, String> {
    use tauri::Manager;

    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();

        validate_global_config(&config)?;

        // Snapshot the old global before overwriting so we can compare
        // readiness deltas below.
        let old_global = load_global_agent_config(&app).unwrap_or_default();

        save_global_agent_config(&app, &config)?;

        // Re-read from disk so the returned value reflects the strip-on-write pass.
        let new_global = load_global_agent_config(&app)?;

        // ── Auto-respawn setup-listener agents ────────────────────────────
        //
        // A "setup-listener" agent is one that was spawned while `agent_readiness`
        // returned `NotReady`.  Because env is baked at spawn time, it will only
        // transition to normal mode after a restart.  We detect the transition by
        // comparing readiness under the old vs new global config: agents that go
        // from NotReady → Ready get a stop-then-start.
        //
        // Errors are non-fatal and logged; the caller receives the saved config
        // regardless.
        respawn_newly_ready_agents(&app, &state, &old_global, &new_global);

        Ok(new_global)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Stop-then-start every local agent whose readiness transitions from
/// `NotReady` (under `old_global`) to `Ready` (under `new_global`).
///
/// Only agents that currently have a live process are candidates — an agent
/// that is already stopped will pick up the new config naturally on its next
/// manual start.
fn respawn_newly_ready_agents(
    app: &AppHandle,
    state: &AppState,
    old_global: &GlobalAgentConfig,
    new_global: &GlobalAgentConfig,
) {
    let records = match load_managed_agents(app) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to load agents for respawn: {e}"
            );
            return;
        }
    };
    let all_personas = match load_personas(app) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to load personas for respawn: {e}"
            );
            return;
        }
    };

    // Collect the pubkeys of agents to respawn before taking any locks.
    let to_respawn: Vec<String> = records
        .iter()
        .filter(|record| {
            // Only local agents are managed here; remote agents are cloud-hosted.
            if record.backend != BackendKind::Local {
                return false;
            }
            // Only respawn if the process is currently live.
            let Some(pid) = record.runtime_pid else {
                return false;
            };
            if !process_is_running(pid) {
                return false;
            }
            // Look up the runtime catalog entry for accurate model/provider env vars.
            let effective_cmd = effective_agent_command(
                record.persona_id.as_deref(),
                &all_personas,
                record.agent_command_override.as_deref(),
            );
            let runtime_meta = known_acp_runtime(&effective_cmd);
            let old_effective =
                resolve_effective_agent_env(record, &all_personas, runtime_meta, old_global);
            let new_effective =
                resolve_effective_agent_env(record, &all_personas, runtime_meta, new_global);
            matches!(
                agent_readiness(&old_effective),
                AgentReadiness::NotReady { .. }
            ) && matches!(agent_readiness(&new_effective), AgentReadiness::Ready)
        })
        .map(|r| r.pubkey.clone())
        .collect();

    if to_respawn.is_empty() {
        return;
    }

    // Take the store lock once for the batch.
    let _store_guard = match state.managed_agents_store_lock.lock() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("buzz-desktop: set_global_agent_config: failed to acquire store lock for respawn: {e}");
            return;
        }
    };
    let mut records = match load_managed_agents(app) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to reload agents under lock: {e}"
            );
            return;
        }
    };
    let mut runtimes = match state.managed_agent_processes.lock() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to acquire runtimes lock: {e}"
            );
            return;
        }
    };

    for pubkey in &to_respawn {
        let Some(record) = records.iter_mut().find(|r| &r.pubkey == pubkey) else {
            continue;
        };
        if let Err(e) = stop_managed_agent_process(app, record, &mut runtimes) {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to stop {pubkey} for respawn: {e}"
            );
            continue;
        }
        if let Err(e) = start_managed_agent_process(app, record, &mut runtimes, None) {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to start {pubkey} after respawn: {e}"
            );
        } else {
            eprintln!(
                "buzz-desktop: set_global_agent_config: respawned setup-listener agent {pubkey}"
            );
        }
    }

    if let Err(e) = save_managed_agents(app, &records) {
        eprintln!(
            "buzz-desktop: set_global_agent_config: failed to save agents after respawn: {e}"
        );
    }
}
