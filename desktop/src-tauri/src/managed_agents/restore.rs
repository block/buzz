#[cfg(feature = "mesh-llm")]
use super::relay_mesh_model_id;
use super::{
    find_managed_agent_mut, kill_stale_tracked_processes, load_managed_agents, load_personas,
    save_managed_agents, spawn_agent_child, sync_managed_agent_processes, BackendKind,
    ManagedAgentProcess,
};
use crate::app_state::AppState;
use crate::util;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

type SpawnResult = Result<ManagedAgentProcess, String>;
type AgentSpawnResult = (String, SpawnResult);

/// Backfill the pinned persona snapshot for pre-existing agents created before
/// the record became the spawn source of truth. Runs once at launch, before
/// `restore_managed_agents_on_launch` spawns anything, so no agent boots from an
/// empty snapshot.
///
/// Only records with a `persona_id` but no `persona_source_version` are touched.
/// If the linked persona is gone, we log loudly and leave the snapshot empty —
/// the record's own `system_prompt`/`model` (possibly empty for persona-created
/// agents) is then all the config that remains, which is the same fallback an
/// orphaned agent already gets.
pub fn backfill_persona_snapshots(app: &tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;

    let mut records = load_managed_agents(app)?;
    let needs_backfill = records
        .iter()
        .any(|r| r.persona_id.is_some() && r.persona_source_version.is_none());
    if !needs_backfill {
        return Ok(());
    }

    let personas = load_personas(app)?;
    let mut changed = false;
    for record in records.iter_mut() {
        let Some(persona_id) = record.persona_id.clone() else {
            continue;
        };
        if record.persona_source_version.is_some() {
            continue;
        }
        let Some(persona) = personas.iter().find(|p| p.id == persona_id) else {
            eprintln!(
                "buzz-desktop: persona-snapshot backfill: agent {} links persona {persona_id} which no longer exists; leaving snapshot empty — it will spawn from its record fields",
                record.pubkey
            );
            continue;
        };
        // Layer the agent's own env overrides over persona env, matching
        // create-time precedence (persona env < agent env).
        let snapshot = super::persona_events::persona_snapshot(persona, &record.env_vars);
        if let Some(prompt) = snapshot.system_prompt {
            record.system_prompt = Some(prompt);
        }
        record.model = snapshot.model;
        record.provider = snapshot.provider;
        record.env_vars = snapshot.env_vars;
        record.persona_source_version = Some(snapshot.source_version);
        record.updated_at = util::now_iso();
        changed = true;
    }

    if changed {
        save_managed_agents(app, &records)?;
    }
    Ok(())
}

/// Restore managed agents that were running before the app was closed.
///
/// Split into three phases to minimise lock contention with the frontend:
///   A (under lock): sync process state, cleanup, collect agents to start
///   B (no locks):   resolve commands and spawn processes in parallel
///   C (re-lock):    write back PIDs and status to records on disk
pub async fn restore_managed_agents_on_launch(
    app: &tauri::AppHandle,
    shutdown_started: &AtomicBool,
) -> Result<(), String> {
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }

    let state = app.state::<AppState>();

    // ── Phase A (under lock): housekeeping + collect agents to restore ──
    let mut agents_to_start: Vec<super::ManagedAgentRecord>;
    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;

        if shutdown_started.load(Ordering::SeqCst) {
            return Ok(());
        }

        let mut records = load_managed_agents(app)?;
        let mut runtimes = state
            .managed_agent_processes
            .lock()
            .map_err(|error| error.to_string())?;
        let (mut changed, _exited) = sync_managed_agent_processes(
            &mut records,
            &mut runtimes,
            &super::current_instance_id(app),
        );
        changed |=
            kill_stale_tracked_processes(&mut records, &runtimes, &super::current_instance_id(app));

        let tracked_pids: Vec<u32> = records
            .iter()
            .filter_map(|r| r.runtime_pid)
            .chain(runtimes.values().map(|rt| rt.child.id()))
            .collect();
        super::sweep_orphaned_agent_processes(app, &tracked_pids);

        // System-wide sweep: enumerate all user processes and kill any known
        // agent binaries not tracked by this session. Catches orphans whose
        // PID files were already cleaned up (e.g. agent workers in their own
        // process group whose parent harness exited).
        super::sweep_system_agent_processes(&super::current_instance_id(app), &tracked_pids);

        // Dead-instance reaping: find agents belonging to Buzz instances
        // whose desktop process is no longer running and reap them.
        super::reap_dead_instance_agents(&super::current_instance_id(app), &tracked_pids);

        let candidates: Vec<String> = records
            .iter()
            .filter(|record| record.start_on_app_launch && record.backend == BackendKind::Local)
            .map(|record| record.pubkey.clone())
            .collect();

        let mut to_start = Vec::new();
        for pubkey in &candidates {
            if let Some(runtime) = runtimes.get_mut(pubkey) {
                if runtime.child.try_wait().ok().flatten().is_none() {
                    continue;
                }
            }
            if let Some(record) = records.iter().find(|r| r.pubkey == *pubkey) {
                if let Some(pid) = record.runtime_pid {
                    if super::process_is_running(pid) {
                        continue;
                    }
                }
                to_start.push(record.clone());
            }
        }
        agents_to_start = to_start;

        // Re-snapshot persona config for agents about to be restored, matching
        // the interactive spawn path so auto-start agents also pick up the
        // current persona on app launch.
        let personas_for_snapshot = super::load_personas(app).unwrap_or_default();
        for record in records.iter_mut() {
            if !agents_to_start.iter().any(|r| r.pubkey == record.pubkey) {
                continue;
            }
            let Some(persona_id) = record.persona_id.clone() else {
                continue;
            };
            let Some(persona) = personas_for_snapshot.iter().find(|p| p.id == persona_id) else {
                continue;
            };
            let snapshot = super::persona_events::persona_snapshot(persona, &record.env_vars);
            if let Some(prompt) = snapshot.system_prompt {
                record.system_prompt = Some(prompt);
            }
            record.model = snapshot.model;
            record.provider = snapshot.provider;
            record.env_vars = snapshot.env_vars;
            record.persona_source_version = Some(snapshot.source_version);
            record.updated_at = util::now_iso();
            changed = true;
        }
        // Re-collect to_start from the updated records so Phase B spawns the refreshed config.
        agents_to_start = records
            .iter()
            .filter(|r| agents_to_start.iter().any(|s| s.pubkey == r.pubkey))
            .cloned()
            .collect();

        if changed {
            save_managed_agents(app, &records)?;
        }
    }

    if agents_to_start.is_empty() {
        return Ok(());
    }

    // Snapshot the workspace owner once and refresh each restored agent's
    // NIP-OA auth tag when the app identity changed since the record was
    // created. Without this, owner-only agents can start successfully but ignore
    // the current user's messages because the harness resolves an old owner.
    let (owner_hex, auth_tag_updates): (Option<String>, Vec<(String, Option<String>, String)>) = {
        let owner_keys = state.keys.lock().map_err(|e| e.to_string())?;
        let owner_hex = owner_keys.public_key().to_hex();
        let mut updates = Vec::new();
        for record in &mut agents_to_start {
            if super::auth_tag_matches_owner(
                record.auth_tag.as_deref(),
                &record.pubkey,
                Some(&owner_hex),
            ) {
                continue;
            }
            let refreshed_auth_tag =
                super::managed_agent_auth_tag_for_owner(&owner_keys, &record.pubkey)?;
            let updated_at = util::now_iso();
            record.auth_tag = refreshed_auth_tag.clone();
            record.updated_at = updated_at.clone();
            updates.push((record.pubkey.clone(), refreshed_auth_tag, updated_at));
        }
        (Some(owner_hex), updates)
    };

    if !auth_tag_updates.is_empty() {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let mut records = load_managed_agents(app)?;
        for (pubkey, auth_tag, updated_at) in &auth_tag_updates {
            if let Ok(record) = find_managed_agent_mut(&mut records, pubkey) {
                record.auth_tag = auth_tag.clone();
                record.updated_at = updated_at.clone();
            }
        }
        save_managed_agents(app, &records)?;
    }

    #[cfg(feature = "mesh-llm")]
    let agents_to_start = {
        let mut mesh_preflight_failures = std::collections::HashSet::new();
        for record in &agents_to_start {
            if relay_mesh_model_id(record).is_none() {
                continue;
            }
            // Auto-start after relaunch: re-resolve a live bootstrap target and
            // dial it. Skip (with an actionable error) only when no live target
            // serves this model right now.
            if let Err(error) =
                crate::commands::ensure_relay_mesh_for_record(app, record, false).await
            {
                persist_restore_error(app, &state, &record.pubkey, error)?;
                mesh_preflight_failures.insert(record.pubkey.clone());
            }
        }
        agents_to_start
            .into_iter()
            .filter(|record| !mesh_preflight_failures.contains(&record.pubkey))
            .collect::<Vec<_>>()
    };
    if agents_to_start.is_empty() {
        return Ok(());
    }

    // ── Phase B (no locks): resolve commands and spawn processes in parallel ──
    let spawn_results: Vec<AgentSpawnResult> = std::thread::scope(|scope| {
        let owner_hex_ref = owner_hex.as_deref();
        let handles: Vec<_> = agents_to_start
            .iter()
            .filter(|_| !shutdown_started.load(Ordering::SeqCst))
            .map(|record| {
                let pubkey = record.pubkey.clone();
                let handle = scope.spawn(move || {
                    let result = spawn_agent_child(app, record, owner_hex_ref);
                    (pubkey, result)
                });
                handle
            })
            .collect();

        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    if spawn_results.is_empty() {
        return Ok(());
    }

    // ── Phase C (re-acquire lock): write back PIDs and status to records ──
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;

    let mut successfully_spawned: Vec<String> = Vec::new();

    for (pubkey, result) in spawn_results {
        let record = match find_managed_agent_mut(&mut records, &pubkey) {
            Ok(r) => r,
            Err(_) => continue,
        };
        match result {
            Ok(process) => {
                let now = util::now_iso();
                record.updated_at = now.clone();
                record.runtime_pid = Some(process.child.id());
                record.last_started_at = Some(now);
                record.last_stopped_at = None;
                record.last_exit_code = None;
                record.last_error = None;
                runtimes.insert(pubkey.clone(), process);
                successfully_spawned.push(pubkey);
            }
            Err(error) => {
                record.updated_at = util::now_iso();
                record.last_error = Some(error);
            }
        }
    }

    // Collect profile reconciliation data for successfully spawned agents before
    // releasing the lock. This mirrors the fire-and-forget pattern in
    // start_managed_agent — ensuring boot-restored agents get the same profile
    // self-healing as UI-started agents.
    let reconcile_personas = super::load_personas(app).unwrap_or_default();
    let reconcile_items: Vec<(String, crate::commands::ProfileReconcileData)> =
        successfully_spawned
            .iter()
            .filter_map(|pubkey| {
                let record = records.iter().find(|r| r.pubkey == *pubkey)?;
                // Resolve the effective harness for the avatar-fallback
                // derivation (the snapshot may be empty/stale for an inherited
                // harness). Mirrors the UI start path.
                let effective_command = crate::managed_agents::effective_agent_command(
                    record.persona_id.as_deref(),
                    &reconcile_personas,
                    record.agent_command_override.as_deref(),
                    Some(&record.agent_command),
                );
                Some((
                    pubkey.clone(),
                    crate::commands::ProfileReconcileData {
                        private_key_nsec: record.private_key_nsec.clone(),
                        name: record.name.clone(),
                        relay_url: record.relay_url.clone(),
                        avatar_url: record.avatar_url.clone(),
                        auth_tag: record.auth_tag.clone(),
                        pubkey: record.pubkey.clone(),
                        agent_command: effective_command,
                        persona_id: record.persona_id.clone(),
                    },
                ))
            })
            .collect();

    save_managed_agents(app, &records)?;

    // ── Profile reconciliation (fire-and-forget) ────────────────────────────
    // Spawn background tasks to ensure each restored agent's kind:0 profile is
    // published on the relay. Same pattern as the UI start path.
    for (pubkey, data) in reconcile_items {
        let reconcile_app = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = reconcile_app.state::<AppState>();
            if let Err(e) =
                crate::commands::reconcile_agent_profile(&state, &reconcile_app, &pubkey, &data)
                    .await
            {
                eprintln!("buzz-desktop: profile reconciliation failed for agent {pubkey}: {e}");
            }
        });
    }

    Ok(())
}

#[cfg(feature = "mesh-llm")]
fn persist_restore_error(
    app: &tauri::AppHandle,
    state: &AppState,
    pubkey: &str,
    error: String,
) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let mut records = load_managed_agents(app)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.updated_at = util::now_iso();
    record.last_error = Some(error);
    save_managed_agents(app, &records)
}
