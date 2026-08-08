//! Tauri commands for global agent configuration defaults.
//!
//! `get_global_agent_config` / `set_global_agent_config` — simple load/save
//! around the `global_config` module with the standard save-time validation.
//!
//! `set_global_agent_config` additionally auto-restarts any running local agent
//! whose effective env changes under the new global config — including agents
//! that were in setup-listener mode (`NotReady`) but become `Ready`, and agents
//! already running whose provider/model/env vars change.  This is the only
//! honest way to deliver new env vars to a running process — the env is baked
//! at spawn time and cannot be mutated in place.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        agent_readiness, current_instance_id, find_managed_agent_mut, known_acp_runtime,
        load_global_agent_config, record_agent_command, resolve_effective_agent_env,
        stop_managed_agent_process, sync_managed_agent_processes, validate_global_config,
        AgentDefinition, AgentReadiness, BackendKind, GlobalAgentConfig, TeamRecord,
    },
};

/// Result returned by `set_global_agent_config`.
///
/// Carries the canonical saved config together with restart counts. Use
/// `restarted_count` for "Restarted N agent(s)." feedback and
/// `failed_restart_count` to surface partial failures ("M failed to restart").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalAgentConfigSaveResult {
    /// The persisted global config (after strip-on-write).
    pub config: GlobalAgentConfig,
    /// Number of local agents successfully stopped and restarted.
    pub restarted_count: u32,
    /// Number of agents whose stop succeeded but respawn failed.
    pub failed_restart_count: u32,
}

/// Read the current global agent configuration.
///
/// Returns the default (empty) config if `global-agent-config.json` has not
/// been written yet.
#[tauri::command]
pub fn get_global_agent_config(app: AppHandle) -> Result<GlobalAgentConfig, String> {
    load_global_agent_config(&app)
}

/// Validate and persist a new global agent configuration, then auto-restart
/// any running local agent whose effective env changes under the new config
/// (including setup-listener agents whose readiness flips to `Ready`).
///
/// Strips empty env values before writing (empty = "inherit" semantics), then
/// applies standard validation: POSIX key shape, reserved-key reject,
/// derived-provider-model-key reject, NUL/size caps.
///
/// Restart is best-effort: per-agent errors are logged to stderr and persisted
/// to `last_error` but do not fail the command.  Returns the saved config and
/// the count of agents successfully restarted.
#[tauri::command]
pub async fn set_global_agent_config(
    config: GlobalAgentConfig,
    app: AppHandle,
) -> Result<GlobalAgentConfigSaveResult, String> {
    use tauri::Manager;

    // Capture the active scope at command entry. All definition I/O targets
    // the captured scope's definitions_dir throughout both phases so a concurrent
    // workspace switch cannot split the config write (Phase 1) from the agent
    // restart (Phase 2) across two different scopes.
    let captured_scope = {
        let state = app.state::<AppState>();
        state
            .capture_active_scope()
            .ok_or("set_global_agent_config: no active workspace scope")?
    };
    let definitions_dir = captured_scope.definitions_dir.clone();

    // ── Phase 1: disk write (sync, spawn_blocking) ────────────────────────
    //
    // Validate, snapshot old config, write new config, collect pre-filter
    // candidate pubkeys (local backend + recorded PID + old NotReady + new
    // Ready).  The candidate list is a hint — eligibility is re-checked under
    // lock in Phase 2 after sync_managed_agent_processes.
    let app_for_write = app.clone();
    let definitions_dir_for_phase1 = definitions_dir.clone();
    let captured_scope_for_phase1 = captured_scope.clone();
    let phase1 = tokio::task::spawn_blocking(move || {
        validate_global_config(&config)?;

        let old_global = crate::managed_agents::global_config::load_global_agent_config_at(
            &definitions_dir_for_phase1,
        )
        .unwrap_or_default();

        // Validate generation before writing so a concurrent switch after the
        // command was dispatched doesn't clobber a newly activated scope's config.
        {
            use tauri::Manager;
            let state = app_for_write.state::<AppState>();
            let _store = state
                .managed_agents_store_lock
                .lock()
                .map_err(|e| e.to_string())?;
            crate::managed_agents::scope::validate_scope_generation(&captured_scope_for_phase1)
                .map_err(|e| format!("set_global_agent_config: {e}"))?;
            crate::managed_agents::global_config::save_global_agent_config_at(
                &definitions_dir_for_phase1,
                &config,
            )?;
        }

        // Re-read from disk so the returned value reflects the strip-on-write pass.
        let new_global = crate::managed_agents::global_config::load_global_agent_config_at(
            &definitions_dir_for_phase1,
        )?;

        // Pre-filter: identify agents that look eligible before taking any locks.
        // This is a hint only; definitive eligibility check happens under lock
        // in Phase 2.
        let (candidates, personas_snapshot) = collect_restart_candidates_at(
            &app_for_write,
            &definitions_dir_for_phase1,
            &old_global,
            &new_global,
        );

        Ok::<_, String>((new_global, old_global, candidates, personas_snapshot))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))??;
    let (new_global, old_global, candidates, personas_snapshot) = phase1;

    // ── Phase 2: async restart (outside spawn_blocking) ──────────────────
    //
    // For each candidate: stop under the lock (re-verifying eligibility after
    // sync_managed_agent_processes), then start via start_local_agent_with_preflight
    // — the same path as a manual restart.  This ensures owner_hex is computed
    // and passed (NIP-OA auth_tag fallback), the persona is re-snapshotted, and
    // last_error is persisted on failure.
    //
    // Uses the same captured `definitions_dir` as Phase 1 so a concurrent
    // workspace switch cannot split config-write from agent-restart across scopes.
    // Generation is re-validated under lock before each stop.
    //
    // Errors are non-fatal; the caller always receives the saved config.
    // failed_restart_count surfaces stops that succeeded but respawn failed.
    let mut restarted_count: u32 = 0;
    let mut failed_restart_count: u32 = 0;
    if !candidates.is_empty() {
        for pubkey in &candidates {
            let outcome = restart_local_agent_on_config_change(
                &app,
                pubkey,
                &old_global,
                &new_global,
                &personas_snapshot,
                &captured_scope,
                &definitions_dir,
            )
            .await;
            match outcome {
                RestartOutcome::Restarted => restarted_count += 1,
                RestartOutcome::FailedAfterStop => failed_restart_count += 1,
                RestartOutcome::Skipped => {}
            }
        }
    }

    Ok(GlobalAgentConfigSaveResult {
        config: new_global,
        restarted_count,
        failed_restart_count,
    })
}

/// Outcome of a single per-agent restart attempt in Phase 2.
#[derive(Debug)]
pub(crate) enum RestartOutcome {
    /// Stop succeeded and the agent re-launched with the new config.
    Restarted,
    /// Stop succeeded but the subsequent spawn failed.
    FailedAfterStop,
    /// Eligibility check failed under lock — agent skipped without touching it.
    Skipped,
}

/// Error returned by [`restart_under_captured_epoch_for`].
#[derive(Debug)]
pub(crate) enum EpochError {
    /// Eligibility check failed before the stop (or stop failed); the agent
    /// was not touched, or the stop failed before any irreversible transition.
    Skipped(String),
    /// Stop succeeded but the subsequent spawn failed.
    FailedAfterStop(String),
}

/// Immutable captured context prepared fallibly BEFORE any stop in the async
/// pre-stop phase of [`restart_local_agent_on_config_change_for`].
///
/// All fields derive from `captured_scope.definitions_dir` — never from live
/// state. Owner keys are verified against `captured_scope.owner_pubkey` before
/// construction; `owner_hex` is derived from the verified keys, not the scope
/// string. The context is frozen once built; subsequent workspace switches or
/// agent edits are detected by generation revalidation inside the epoch.
#[derive(Clone, Debug)]
pub(crate) struct CapturedRestartContext {
    pub scope: crate::managed_agents::scope::WorkspaceAgentScope,
    pub personas: Vec<AgentDefinition>,
    pub teams: Vec<TeamRecord>,
    pub global: GlobalAgentConfig,
    /// Owner pubkey hex derived from verified signing keys, not the scope string.
    pub owner_hex: String,
    /// Effective Relay-Mesh model ID resolved from the candidate record at
    /// preparation time. Used for pre-stop Mesh preflight and in-epoch
    /// re-resolution mismatch guard.
    pub mesh_model_id: Option<String>,
}

/// Collect pubkeys of local agents that should be restarted after a global
/// config change, together with the personas snapshot used for the scan.
///
/// Scoped variant used by Phase 1 of `set_global_agent_config`: reads from the
/// captured `definitions_dir` rather than the live active scope so a concurrent
/// workspace switch cannot redirect the scan to a different scope's records.
///
/// Pre-lock hint — eligibility is re-verified under lock in Phase 2. The personas
/// snapshot is threaded to `restart_local_agent_on_config_change` so it is not
/// reloaded per agent.
///
/// An agent is a candidate when it is a local backend with a recorded PID, and
/// either:
/// - its readiness transitions `NotReady → Ready` (was blocked on missing
///   provider/model key, now unblocked), OR
/// - it was already `Ready`, its process is currently alive, and its effective
///   env changed (provider, model, or env var update that needs a restart to
///   take effect, since env is baked at spawn time).
fn collect_restart_candidates_at(
    app: &AppHandle,
    definitions_dir: &std::path::Path,
    old_global: &GlobalAgentConfig,
    new_global: &GlobalAgentConfig,
) -> (Vec<String>, Vec<crate::managed_agents::AgentDefinition>) {
    let records = match crate::managed_agents::storage::load_managed_agents_at(definitions_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to load agents for restart scan: {e}"
            );
            return (Vec::new(), Vec::new());
        }
    };
    let all_personas = match crate::managed_agents::load_personas_at(definitions_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to load personas for restart scan: {e}"
            );
            return (Vec::new(), Vec::new());
        }
    };
    use tauri::Manager;
    let state = app.state::<AppState>();
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let candidates = records
        .iter()
        .filter(|record| {
            if record.backend != BackendKind::Local {
                return false;
            }
            let has_live_runtime = runtimes.iter_mut().any(|(key, runtime)| {
                key.pubkey.eq_ignore_ascii_case(&record.pubkey)
                    && runtime.child.try_wait().ok().flatten().is_none()
            });
            if !has_live_runtime {
                return false;
            }
            let effective_cmd = record_agent_command(record, &all_personas);
            let runtime_meta = known_acp_runtime(&effective_cmd);
            let old_effective =
                resolve_effective_agent_env(record, &all_personas, runtime_meta, old_global);
            let new_effective =
                resolve_effective_agent_env(record, &all_personas, runtime_meta, new_global);
            let old_ready = matches!(agent_readiness(&old_effective), AgentReadiness::Ready);
            let new_ready = matches!(agent_readiness(&new_effective), AgentReadiness::Ready);
            // For a Ready+running agent: the process must be alive now and the
            // process-env map must differ.  The alive check avoids queuing a
            // restart for a process that already exited between the pre-filter
            // scan and Phase 2.  NotReady→Ready bypasses the alive check
            // because Phase 2 will stop-then-start unconditionally.
            let env_changed = old_ready && old_effective.env != new_effective.env;

            should_restart_on_config_change(old_ready, new_ready, env_changed)
        })
        .map(|r| r.pubkey.clone())
        .collect();

    (candidates, all_personas)
}

/// Restart a local agent whose effective env changed under the new global config.
///
/// Async driver. Prepares [`CapturedRestartContext`] fallibly BEFORE any stop,
/// including the relay-Mesh preflight — failure here leaves the old process
/// running. Only after the context is fully prepared does the atomic
/// stop→spawn epoch run inside `spawn_blocking`.
///
/// Returns [`RestartOutcome::FailedAfterStop`] when stop succeeded but spawn
/// failed; [`RestartOutcome::Skipped`] when any pre-stop check fails or the
/// epoch generation guard aborts before the stop.
async fn restart_local_agent_on_config_change(
    app: &AppHandle,
    pubkey: &str,
    old_global: &GlobalAgentConfig,
    new_global: &GlobalAgentConfig,
    personas_snapshot: &[crate::managed_agents::AgentDefinition],
    captured_scope: &crate::managed_agents::scope::WorkspaceAgentScope,
    definitions_dir: &std::path::Path,
) -> RestartOutcome {
    restart_local_agent_on_config_change_for(
        app,
        pubkey,
        old_global,
        new_global,
        personas_snapshot,
        captured_scope,
        definitions_dir,
        // Production mesh preflight — async, runs before spawn_blocking.
        |app_ref, model_id| {
            let app_clone = app_ref.clone();
            let model = model_id.map(str::to_string);
            Box::pin(async move {
                #[cfg(feature = "mesh-llm")]
                {
                    crate::commands::ensure_relay_mesh_for_record(
                        &app_clone,
                        model.as_deref(),
                        false,
                    )
                    .await
                }
                #[cfg(not(feature = "mesh-llm"))]
                {
                    let _ = (app_clone, model);
                    Ok(())
                }
            })
        },
        // Production stop function.
        stop_managed_agent_process,
        // Production spawn function.
        |app_ref, rec, relay, owner, personas, global, teams| {
            crate::managed_agents::spawn_agent_child_at(
                app_ref, rec, relay, true, owner, personas, global, teams,
            )
        },
        // Production receipt function.
        crate::managed_agents::write_agent_runtime_receipt,
    )
    .await
}

/// Injected-seam async driver for restarting a local agent on config change.
///
/// Accepts injected `mesh_fn`, `stop_fn`, `spawn_fn`, and `write_receipt_fn`
/// so the full driver can be exercised in tests without spawning real processes,
/// hitting the macOS keychain, or calling a real Mesh relay.
///
/// The function signature uses generic parameters (stable Rust) rather than
/// `AsyncFn` (nightly) or `dyn` (requires boxing closures). The production
/// adapter `restart_local_agent_on_config_change` closes over the concrete
/// function pointers.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn restart_local_agent_on_config_change_for<
    R,
    MeshFn,
    StopFn,
    SpawnFn,
    ReceiptFn,
>(
    app: &tauri::AppHandle<R>,
    pubkey: &str,
    old_global: &GlobalAgentConfig,
    new_global: &GlobalAgentConfig,
    personas_snapshot: &[crate::managed_agents::AgentDefinition],
    captured_scope: &crate::managed_agents::scope::WorkspaceAgentScope,
    definitions_dir: &std::path::Path,
    mesh_fn: MeshFn,
    stop_fn: StopFn,
    spawn_fn: SpawnFn,
    write_receipt_fn: ReceiptFn,
) -> RestartOutcome
where
    R: tauri::Runtime,
    MeshFn: for<'a> Fn(
        &'a tauri::AppHandle<R>,
        Option<&'a str>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>,
    >,
    StopFn: Fn(
            &tauri::AppHandle<R>,
            &mut crate::managed_agents::ManagedAgentRecord,
            &mut std::collections::HashMap<
                crate::managed_agents::ManagedAgentRuntimeKey,
                crate::managed_agents::ManagedAgentPairRuntime,
            >,
        ) -> Result<(), String>
        + Send
        + 'static,
    SpawnFn: Fn(
            &tauri::AppHandle<R>,
            &crate::managed_agents::ManagedAgentRecord,
            &str,
            Option<&str>,
            &[AgentDefinition],
            &GlobalAgentConfig,
            &[TeamRecord],
        ) -> Result<crate::managed_agents::ManagedAgentProcess, String>
        + Send
        + 'static,
    ReceiptFn: Fn(
            &tauri::AppHandle<R>,
            &crate::managed_agents::ManagedAgentRuntimeReceipt,
        ) -> Result<(), String>
        + Send
        + 'static,
{
    // ── Pre-stop phase: prepare CapturedRestartContext fallibly ─────────────
    // Any failure here leaves the old process running (RestartOutcome::Skipped).

    let personas_at = match crate::managed_agents::load_personas_at(definitions_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "buzz-desktop: restart_local_agent_on_config_change_for: failed to load personas for {pubkey}: {e}"
            );
            return RestartOutcome::Skipped;
        }
    };

    let teams_at = {
        let teams_path = crate::managed_agents::teams_store_path_at(definitions_dir);
        match crate::managed_agents::load_teams_readonly(&teams_path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "buzz-desktop: restart_local_agent_on_config_change_for: failed to load teams for {pubkey}: {e}"
                );
                return RestartOutcome::Skipped;
            }
        }
    };

    let global_at = match crate::managed_agents::global_config::load_global_agent_config_at(
        definitions_dir,
    ) {
        Ok(g) => g,
        Err(e) => {
            eprintln!(
                    "buzz-desktop: restart_local_agent_on_config_change_for: failed to load global config for {pubkey}: {e}"
                );
            return RestartOutcome::Skipped;
        }
    };

    // Verify owner keys still match the captured scope; derive owner_hex from keys.
    let owner_hex = {
        use tauri::Manager;
        let state = app.state::<AppState>();
        match state.signing_keys() {
            Ok(keys) => {
                let hex = keys.public_key().to_hex();
                if !hex.eq_ignore_ascii_case(&captured_scope.owner_pubkey) {
                    eprintln!(
                        "buzz-desktop: restart_local_agent_on_config_change_for: owner key mismatch for {pubkey}"
                    );
                    return RestartOutcome::Skipped;
                }
                hex
            }
            Err(e) => {
                eprintln!(
                    "buzz-desktop: restart_local_agent_on_config_change_for: signing keys unavailable for {pubkey}: {e}"
                );
                return RestartOutcome::Skipped;
            }
        }
    };

    // Load candidate record and resolve its effective Mesh model ID.
    let records_for_preflight = match crate::managed_agents::storage::load_managed_agents_at(
        definitions_dir,
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                    "buzz-desktop: restart_local_agent_on_config_change_for: failed to load records for preflight for {pubkey}: {e}"
                );
            return RestartOutcome::Skipped;
        }
    };
    let candidate_record = match records_for_preflight.iter().find(|r| r.pubkey == pubkey) {
        Some(r) => r.clone(),
        None => {
            eprintln!(
                "buzz-desktop: restart_local_agent_on_config_change_for: agent {pubkey} not found during preflight"
            );
            return RestartOutcome::Skipped;
        }
    };
    let mesh_model_id =
        crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
            &candidate_record,
            &personas_at,
            &global_at,
        );

    // Mesh preflight — async, before spawn_blocking, before any stop.
    if let Err(e) = mesh_fn(app, mesh_model_id.as_deref()).await {
        eprintln!(
            "buzz-desktop: restart_local_agent_on_config_change_for: mesh preflight failed for {pubkey}: {e}"
        );
        return RestartOutcome::Skipped;
    }

    let context = CapturedRestartContext {
        scope: captured_scope.clone(),
        personas: personas_at,
        teams: teams_at,
        global: global_at,
        owner_hex,
        mesh_model_id,
    };

    // ── Atomic stop→spawn epoch in spawn_blocking ───────────────────────────
    let app_owned = app.clone();
    let pubkey_owned = pubkey.to_string();
    let old_global_owned = old_global.clone();
    let new_global_owned = new_global.clone();
    let personas_owned = personas_snapshot.to_vec();
    let context_owned = context;

    let result = tokio::task::spawn_blocking(move || {
        restart_under_captured_epoch_for(
            &app_owned,
            &pubkey_owned,
            &old_global_owned,
            &new_global_owned,
            &personas_owned,
            &context_owned,
            stop_fn,
            spawn_fn,
            write_receipt_fn,
        )
    })
    .await;

    let captured_scope_for_err = captured_scope.clone();
    match result {
        Ok(Ok(())) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: restarted agent {pubkey} with updated config"
            );
            RestartOutcome::Restarted
        }
        Ok(Err(EpochError::Skipped(e))) => {
            eprintln!("buzz-desktop: set_global_agent_config: skipping restart of {pubkey}: {e}");
            RestartOutcome::Skipped
        }
        Ok(Err(EpochError::FailedAfterStop(e))) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: failed to start {pubkey} after stop: {e}"
            );
            if let Err(save_err) = persist_last_error(app, pubkey, &e, &captured_scope_for_err) {
                eprintln!(
                    "buzz-desktop: set_global_agent_config: failed to persist last_error for {pubkey}: {save_err}"
                );
            }
            RestartOutcome::FailedAfterStop
        }
        Err(e) => {
            eprintln!(
                "buzz-desktop: set_global_agent_config: spawn_blocking panicked for {pubkey}: {e}"
            );
            RestartOutcome::Skipped
        }
    }
}

/// Testable epoch core — acquires locks, validates generation, re-resolves
/// the Mesh model (non-workspace TOCTOU guard), stops the process, spawns
/// from pre-built captured context, writes receipt, registers runtime, saves.
///
/// All three operations (stop, spawn, receipt write) are injected so the core
/// can be exercised without spawning real child processes or writing receipts
/// to the filesystem. The production adapter passes the real implementations.
///
/// **Stop failure is `Skipped`** — if `stop_fn` fails the runtime is
/// reinserted and no irreversible transition occurred. `FailedAfterStop` is
/// reserved for failures AFTER a successful stop.
///
/// `spawn_fn` and `write_receipt_fn` are `FnMut` to support records with
/// multiple relay pairs. The core itself owns key/receipt construction,
/// `runtimes` insertion, captured-dir saves, and retention.
///
/// INVARIANT: `managed_agent_runtime_transition` must be held by the caller
/// through the entire epoch — no workspace switch can occur during this call,
/// so `spawn_agent_child_at` receives the captured scope's teams (passed
/// explicitly via `context.teams`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn restart_under_captured_epoch_for<R, StopFn, SpawnFn, ReceiptFn>(
    app: &tauri::AppHandle<R>,
    pubkey: &str,
    old_global: &GlobalAgentConfig,
    new_global: &GlobalAgentConfig,
    personas_snapshot: &[AgentDefinition],
    context: &CapturedRestartContext,
    mut stop_fn: StopFn,
    mut spawn_fn: SpawnFn,
    mut write_receipt_fn: ReceiptFn,
) -> Result<(), EpochError>
where
    R: tauri::Runtime,
    StopFn: FnMut(
        &tauri::AppHandle<R>,
        &mut crate::managed_agents::ManagedAgentRecord,
        &mut std::collections::HashMap<
            crate::managed_agents::ManagedAgentRuntimeKey,
            crate::managed_agents::ManagedAgentPairRuntime,
        >,
    ) -> Result<(), String>,
    SpawnFn: FnMut(
        &tauri::AppHandle<R>,
        &crate::managed_agents::ManagedAgentRecord,
        &str,
        Option<&str>,
        &[AgentDefinition],
        &GlobalAgentConfig,
        &[TeamRecord],
    ) -> Result<crate::managed_agents::ManagedAgentProcess, String>,
    ReceiptFn: FnMut(
        &tauri::AppHandle<R>,
        &crate::managed_agents::ManagedAgentRuntimeReceipt,
    ) -> Result<(), String>,
{
    use crate::managed_agents::{
        managed_agent_runtime_keys,
        storage::{load_managed_agents_at, save_managed_agents_at},
        ManagedAgentPairRuntime, ManagedAgentRuntimeKey,
    };
    use tauri::Manager;

    let state = app.state::<AppState>();
    let definitions_dir = &context.scope.definitions_dir;

    // Hold transition from stop through spawn — no concurrent start can enter.
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| EpochError::Skipped(format!("transition lock poisoned: {e}")))?;

    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| EpochError::Skipped(format!("store lock poisoned: {e}")))?;

    // Validate captured generation before touching any state.
    crate::managed_agents::scope::validate_scope_generation(&context.scope)
        .map_err(|e| EpochError::Skipped(format!("stale scope: {e}")))?;

    let mut records = load_managed_agents_at(definitions_dir)
        .map_err(|e| EpochError::Skipped(format!("load records: {e}")))?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| EpochError::Skipped(format!("runtimes lock poisoned: {e}")))?;

    let (sync_changed, _) =
        sync_managed_agent_processes(&mut records, &mut runtimes, &current_instance_id(app));
    if sync_changed {
        save_managed_agents_at(definitions_dir, &records)
            .map_err(|e| EpochError::Skipped(format!("save after sync: {e}")))?;
    }

    // Re-check eligibility under both locks.
    let record = records
        .iter()
        .find(|r| r.pubkey == pubkey)
        .ok_or_else(|| EpochError::Skipped(format!("agent {pubkey} not found")))?;
    if record.backend != BackendKind::Local {
        return Err(EpochError::Skipped(format!(
            "agent {pubkey} is not a local agent"
        )));
    }
    let runtime_keys = managed_agent_runtime_keys(&runtimes, pubkey);
    if runtime_keys.is_empty() {
        return Err(EpochError::Skipped(format!(
            "agent {pubkey} has no live pair runtime after sync"
        )));
    }
    let relay_urls: Vec<String> = runtime_keys.iter().map(|k| k.relay_url.clone()).collect();

    let effective_cmd = record_agent_command(record, personas_snapshot);
    let runtime_meta = known_acp_runtime(&effective_cmd);
    let old_effective =
        resolve_effective_agent_env(record, personas_snapshot, runtime_meta, old_global);
    let new_effective =
        resolve_effective_agent_env(record, personas_snapshot, runtime_meta, new_global);
    let old_ready = matches!(agent_readiness(&old_effective), AgentReadiness::Ready);
    let new_ready = matches!(agent_readiness(&new_effective), AgentReadiness::Ready);
    let env_changed = old_ready && old_effective.env != new_effective.env;
    if !should_restart_on_config_change(old_ready, new_ready, env_changed) {
        return Err(EpochError::Skipped(format!(
            "agent {pubkey} restart condition no longer valid under lock"
        )));
    }

    // Non-workspace TOCTOU guard: re-load the record and re-resolve its Mesh
    // model against the captured config. An agent edit (definition change)
    // can occur between context preparation and epoch entry without advancing
    // the workspace generation. If the model ID differs from what was
    // preflighted, abort before stop — the preflight covered a model that may
    // no longer be in play.
    let re_resolved_mesh =
        crate::managed_agents::effective_config::resolve_effective_relay_mesh_model_id(
            record,
            &context.personas,
            &context.global,
        );
    if re_resolved_mesh != context.mesh_model_id {
        return Err(EpochError::Skipped(format!(
            "agent {pubkey} relay-mesh model changed between preflight and epoch \
             (was {:?}, now {:?}); aborting before stop",
            context.mesh_model_id, re_resolved_mesh
        )));
    }

    // Stop under the held locks. Stop failure → runtime reinserted → Skipped
    // (no irreversible transition occurred).
    let record_mut = find_managed_agent_mut(&mut records, pubkey)
        .map_err(|e| EpochError::Skipped(format!("find record: {e}")))?;
    if let Err(e) = stop_fn(app, record_mut, &mut runtimes) {
        return Err(EpochError::Skipped(format!(
            "stop failed before irreversible transition: {e}"
        )));
    }
    save_managed_agents_at(definitions_dir, &records)
        .map_err(|e| EpochError::FailedAfterStop(format!("save after stop: {e}")))?;

    // Reload records with the updated last_stopped_at.
    let mut records = load_managed_agents_at(definitions_dir)
        .map_err(|e| EpochError::FailedAfterStop(format!("reload records after stop: {e}")))?;

    let owner_hex = &context.owner_hex;
    let scope_id = context.scope.scope_id.clone();
    let mut spawn_errors: Vec<String> = Vec::new();

    for relay_url in &relay_urls {
        let key = match ManagedAgentRuntimeKey::new(pubkey, relay_url) {
            Ok(k) => k,
            Err(e) => {
                spawn_errors.push(format!("{relay_url}: key error: {e}"));
                continue;
            }
        };

        // Apply persona snapshot before spawning (same as interactive start path).
        if let Ok(record_mut) = find_managed_agent_mut(&mut records, pubkey) {
            if let Some(persona_id) = record_mut.persona_id.clone() {
                if let Some(persona) = context.personas.iter().find(|p| p.id == persona_id) {
                    crate::managed_agents::persona_events::apply_persona_snapshot(
                        record_mut, persona,
                    );
                    record_mut.updated_at = crate::util::now_iso();
                }
            }
        }

        let spawn_record = match records.iter().find(|r| r.pubkey == pubkey).cloned() {
            Some(r) => r,
            None => {
                spawn_errors.push(format!("{relay_url}: record disappeared before spawn"));
                continue;
            }
        };

        // Spawn using captured personas/global/teams and captured owner.
        // Teams are passed explicitly from the captured context — no live disk I/O.
        let spawn_result = spawn_fn(
            app,
            &spawn_record,
            relay_url,
            Some(owner_hex.as_str()),
            &context.personas,
            &context.global,
            &context.teams,
        );
        let mut process = match spawn_result {
            Ok(p) => p,
            Err(e) => {
                spawn_errors.push(format!("{relay_url}: spawn: {e}"));
                continue;
            }
        };

        let now = crate::util::now_iso();
        let receipt = crate::managed_agents::ManagedAgentRuntimeReceipt {
            key: key.clone(),
            pid: process.child.id(),
            desktop_instance_id: current_instance_id(app),
            started_at: now.clone(),
        };
        if let Err(e) = write_receipt_fn(app, &receipt) {
            let _ = crate::managed_agents::terminate_process(process.child.id());
            let _ = process.child.wait();
            spawn_errors.push(format!("{relay_url}: receipt: {e}"));
            continue;
        }

        if let Ok(record_mut) = find_managed_agent_mut(&mut records, pubkey) {
            record_mut.runtime_pid = None;
            record_mut.updated_at = now.clone();
            record_mut.last_started_at = Some(now);
            record_mut.last_stopped_at = None;
            record_mut.last_error = None;
        }
        // Register runtime with the captured scope_id.
        runtimes.insert(
            key.clone(),
            ManagedAgentPairRuntime::starting(process, Some(scope_id.clone())),
        );
    }

    save_managed_agents_at(definitions_dir, &records)
        .map_err(|e| EpochError::FailedAfterStop(format!("save after spawn: {e}")))?;

    // Drop runtimes lock before retention (retention uses its own DB mutex).
    drop(runtimes);

    // Retain the agent event under the captured retention scope.
    if let Some(saved_record) = records.iter().find(|r| r.pubkey == pubkey) {
        let owner_keys_result = state.signing_keys();
        let scope_result = owner_keys_result
            .ok()
            .filter(|k| {
                k.public_key()
                    .to_hex()
                    .eq_ignore_ascii_case(&context.scope.owner_pubkey)
            })
            .map(|keys| {
                crate::managed_agents::retention::retention_scope_from_captured(
                    &context.scope,
                    keys,
                )
            });
        match scope_result {
            Some(Ok(scope)) => {
                use crate::managed_agents::{
                    reconcile::retain_agent_record, retention::open_retention_db,
                };
                if let Ok(conn) = open_retention_db(&scope.db_path) {
                    let _ = retain_agent_record(&conn, &scope.owner_keys, saved_record);
                }
            }
            Some(Err(e)) => {
                eprintln!(
                    "buzz-desktop: set_global_agent_config: retention scope error for {pubkey}: {e}"
                );
            }
            None => {}
        }
    }

    if spawn_errors.is_empty() {
        Ok(())
    } else {
        Err(EpochError::FailedAfterStop(spawn_errors.join("; ")))
    }
}

/// Persist a `last_error` on the agent record under a freshly acquired store lock.
///
/// Best-effort: called only after a failed restart to surface a diagnosable
/// stopped state in the UI.  Takes `captured_scope`, validates generation under
/// the acquired lock so a stale write doesn't silently target the old scope.
///
/// MUST NOT be called while `managed_agents_store_lock` is already held — this
/// function acquires the lock itself and fails closed on poison.
fn persist_last_error<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    pubkey: &str,
    error: &str,
    captured_scope: &crate::managed_agents::scope::WorkspaceAgentScope,
) -> Result<(), String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| format!("persist_last_error: store lock poisoned — fail closed: {e}"))?;
    crate::managed_agents::scope::validate_scope_generation(captured_scope)
        .map_err(|e| format!("persist_last_error: stale scope, skipping write: {e}"))?;
    let definitions_dir = &captured_scope.definitions_dir;
    let mut records = crate::managed_agents::storage::load_managed_agents_at(definitions_dir)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.last_error = Some(error.to_string());
    record.updated_at = crate::util::now_iso();
    crate::managed_agents::storage::save_managed_agents_at(definitions_dir, &records)
}

/// Pure predicate: should an agent be restarted given resolved readiness and
/// effective-env snapshots?
///
/// Extracted so the restart decision logic can be unit-tested without an
/// `AppHandle` or `EffectiveAgentEnv`.  Both `collect_restart_candidates` and
/// the under-lock eligibility check in `restart_local_agent_on_config_change`
/// delegate to this predicate.
///
/// Conditions:
/// - `NotReady → Ready`: blocked on missing key, now unblocked.
/// - `Ready + env changed`: running with stale env; env is baked at spawn time.
///   Also covers `Ready → NotReady` when the env changed (key removed).
///
/// **Readiness invariant (T,F,F):** For `buzz-agent` and `goose`, readiness is
/// derived purely from `EffectiveAgentEnv` — it cannot flip without an env delta.
/// For `claude`/`codex`, `cli_login_requirements` queries runtime auth state
/// (e.g. `claude auth status`), so readiness CAN flip Ready→NotReady without
/// an env change. In that case combo (T,F,F) evaluates to `false` — the running
/// agent is NOT restarted. This is intentional: the env is unchanged, and a
/// restart would not repair the missing auth token. If the binary disappears,
/// the process would already be dead and the PID alive-check in the candidate
/// scan would have excluded it.
fn should_restart_on_config_change(old_ready: bool, new_ready: bool, env_changed: bool) -> bool {
    (!old_ready && new_ready) || (old_ready && env_changed)
}

#[cfg(test)]
#[path = "global_agent_config_tests.rs"]
mod tests;
