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
        agent_readiness, current_instance_id, find_managed_agent_mut, known_acp_runtime,
        load_global_agent_config, load_managed_agents, load_personas, process_is_running,
        record_agent_command, resolve_effective_agent_env, save_global_agent_config,
        save_managed_agents, stop_managed_agent_process, sync_managed_agent_processes,
        validate_global_config, AgentReadiness, BackendKind, GlobalAgentConfig,
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
/// Respawn is best-effort: per-agent errors are logged to stderr and persisted
/// to `last_error` but do not fail the command.  The returned value is the
/// round-tripped config from disk.
#[tauri::command]
pub async fn set_global_agent_config(
    config: GlobalAgentConfig,
    app: AppHandle,
) -> Result<GlobalAgentConfig, String> {
    use tauri::Manager;

    // ── Phase 1: disk write (sync, spawn_blocking) ────────────────────────
    //
    // Validate, snapshot old config, write new config, collect pre-filter
    // candidate pubkeys (local backend + recorded PID + old NotReady + new
    // Ready).  The candidate list is a hint — eligibility is re-checked under
    // lock in Phase 2 after sync_managed_agent_processes.
    let app_for_write = app.clone();
    let (new_global, old_global, candidates) = tokio::task::spawn_blocking(move || {
        validate_global_config(&config)?;

        let old_global = load_global_agent_config(&app_for_write).unwrap_or_default();

        save_global_agent_config(&app_for_write, &config)?;

        // Re-read from disk so the returned value reflects the strip-on-write pass.
        let new_global = load_global_agent_config(&app_for_write)?;

        // Pre-filter: identify agents that look eligible before taking any locks.
        // This is a hint only; definitive eligibility check happens under lock
        // in Phase 2.
        let candidates = collect_respawn_candidates(&app_for_write, &old_global, &new_global);

        Ok::<_, String>((new_global, old_global, candidates))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    // ── Phase 2: async respawn (outside spawn_blocking) ───────────────────
    //
    // For each candidate: stop under the lock (re-verifying eligibility after
    // sync_managed_agent_processes), then start via start_local_agent_with_preflight
    // — the same path as a manual restart.  This ensures owner_hex is computed
    // and passed (NIP-OA auth_tag fallback), the persona is re-snapshotted, and
    // last_error is persisted on failure.
    //
    // Errors are non-fatal; the caller always receives the saved config.
    if !candidates.is_empty() {
        let state = app.state::<AppState>();
        let owner_hex = match super::agents::workspace_owner_hex(&state) {
            Ok(h) => h,
            Err(e) => {
                eprintln!(
                    "buzz-desktop: set_global_agent_config: failed to compute owner_hex for respawn: {e}"
                );
                return Ok(new_global);
            }
        };

        for pubkey in &candidates {
            restart_setup_listener_agent(&app, pubkey, &owner_hex, &old_global, &new_global).await;
        }
    }

    Ok(new_global)
}

/// Collect pubkeys of agents whose readiness transitions NotReady → Ready
/// under the new global config.  Pre-lock hint used by Phase 1 of
/// `set_global_agent_config`.  Eligibility is re-verified under lock in Phase 2.
fn collect_respawn_candidates(
    app: &AppHandle,
    old_global: &GlobalAgentConfig,
    new_global: &GlobalAgentConfig,
) -> Vec<String> {
    let records = match load_managed_agents(app) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to load agents for respawn scan: {e}"
            );
            return Vec::new();
        }
    };
    let all_personas = match load_personas(app) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to load personas for respawn scan: {e}"
            );
            return Vec::new();
        }
    };

    records
        .iter()
        .filter(|record| {
            if record.backend != BackendKind::Local {
                return false;
            }
            // Quick pre-check: must have a recorded PID (may still be alive).
            if record.runtime_pid.is_none() {
                return false;
            }
            let effective_cmd = record_agent_command(record, &all_personas);
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
        .collect()
}

/// Stop-then-start a single setup-listener agent as a normal agent.
///
/// This is the per-agent respawn step in Phase 2 of `set_global_agent_config`.
/// It mirrors the semantics of a manual agent restart:
///
/// 1. **Stop under lock** — acquires the store lock, calls
///    `sync_managed_agent_processes`, re-verifies eligibility (local backend,
///    live process, old-global readiness NotReady, new-global readiness Ready),
///    then stops the process and saves the record.  The lock is released before
///    the start so `start_local_agent_with_preflight` can re-acquire it cleanly.
///
/// 2. **Start via the normal preflight path** — calls
///    `start_local_agent_with_preflight`, which computes and passes `owner_hex`
///    (NIP-OA fallback for legacy records without `auth_tag`), re-snapshots the
///    persona (agent starts with current persona config), saves the updated
///    record, and retains the event for relay sync.  On failure, `last_error` is
///    persisted under lock so the UI surfaces a diagnosable stopped state.
///
/// All errors are logged to stderr and swallowed; the caller always proceeds.
async fn restart_setup_listener_agent(
    app: &AppHandle,
    pubkey: &str,
    owner_hex: &str,
    old_global: &GlobalAgentConfig,
    new_global: &GlobalAgentConfig,
) {
    // ── Step 1: stop under lock, re-verifying eligibility ─────────────────
    let app_for_stop = app.clone();
    let pubkey_owned = pubkey.to_string();
    let old_global_clone = old_global.clone();
    let new_global_clone = new_global.clone();

    let stop_result = tokio::task::spawn_blocking(move || {
        use tauri::Manager;
        let state = app_for_stop.state::<AppState>();

        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|e| format!("failed to acquire store lock: {e}"))?;

        let mut records = load_managed_agents(&app_for_stop)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|e| format!("failed to acquire runtimes lock: {e}"))?;

        // Sync process state so PID liveness reflects current reality.
        let (sync_changed, _) = sync_managed_agent_processes(
            &mut records,
            &mut runtimes,
            &current_instance_id(&app_for_stop),
        );
        if sync_changed {
            save_managed_agents(&app_for_stop, &records)?;
        }

        // Re-check eligibility under lock with current record state.
        let record = records
            .iter()
            .find(|r| r.pubkey == pubkey_owned)
            .ok_or_else(|| format!("agent {pubkey_owned} not found"))?;

        if record.backend != BackendKind::Local {
            return Err(format!("agent {pubkey_owned} is no longer a local agent"));
        }
        let Some(pid) = record.runtime_pid else {
            return Err(format!(
                "agent {pubkey_owned} no longer has a live process after sync"
            ));
        };
        if !process_is_running(pid) {
            return Err(format!(
                "agent {pubkey_owned} process {pid} is no longer running"
            ));
        }

        // Re-check the NotReady → Ready transition under lock.
        let all_personas = load_personas(&app_for_stop).unwrap_or_default();
        let effective_cmd = record_agent_command(record, &all_personas);
        let runtime_meta = known_acp_runtime(&effective_cmd);
        let old_effective =
            resolve_effective_agent_env(record, &all_personas, runtime_meta, &old_global_clone);
        let new_effective =
            resolve_effective_agent_env(record, &all_personas, runtime_meta, &new_global_clone);
        if !matches!(
            agent_readiness(&old_effective),
            AgentReadiness::NotReady { .. }
        ) || !matches!(agent_readiness(&new_effective), AgentReadiness::Ready)
        {
            return Err(format!(
                "agent {pubkey_owned} readiness transition no longer valid under lock"
            ));
        }

        // Stop the setup-listener process.
        let record_mut = find_managed_agent_mut(&mut records, &pubkey_owned)?;
        stop_managed_agent_process(&app_for_stop, record_mut, &mut runtimes)?;
        save_managed_agents(&app_for_stop, &records)?;

        Ok(())
    })
    .await;

    let stopped = match stop_result {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            eprintln!("buzz-desktop: set_global_agent_config: skipping respawn of {pubkey}: {e}");
            false
        }
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: spawn_blocking failed for stop of {pubkey}: {e}"
            );
            false
        }
    };

    if !stopped {
        return;
    }

    // ── Step 2: start via the normal preflight path ────────────────────────
    //
    // start_local_agent_with_preflight handles: re-acquiring the store lock,
    // persona re-snapshot (agent starts with current persona config), passing
    // owner_hex (NIP-OA auth_tag fallback for legacy records), saving the
    // updated record, and retaining the event for relay sync.
    {
        use tauri::Manager;
        let state = app.state::<AppState>();
        match super::agents::start_local_agent_with_preflight(app, &state, pubkey, owner_hex, false)
            .await
        {
            Ok(_) => {
                eprintln!(
                    "buzz-desktop: set_global_agent_config: respawned setup-listener agent {pubkey}"
                );
            }
            Err(e) => {
                eprintln!(
                    "buzz-desktop: set_global_agent_config: failed to start {pubkey} after respawn: {e}"
                );
                // Persist last_error so the UI surfaces a diagnosable stopped state.
                if let Err(save_err) = persist_last_error(app, pubkey, &e) {
                    eprintln!(
                        "buzz-desktop: set_global_agent_config: failed to persist last_error for {pubkey}: {save_err}"
                    );
                }
            }
        }
    }
}

/// Persist a `last_error` on the agent record under the store lock.
///
/// Best-effort: called only after a failed respawn start to leave the record
/// in a diagnosable state rather than a silent "stopped with no error" state.
fn persist_last_error(app: &AppHandle, pubkey: &str, error: &str) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| format!("failed to acquire store lock: {e}"))?;
    let mut records = load_managed_agents(app)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.last_error = Some(error.to_string());
    record.updated_at = crate::util::now_iso();
    save_managed_agents(app, &records)
}
