//! Persona deletion and its managed-agent cascade.
//!
//! Deleting a persona deletes every managed-agent instance built from it. The
//! interesting part is the provider-backed subset of that cascade: those
//! instances are deployed units this app can forget but not stop, because the
//! provider protocol is deploy-only. `preflight_remote_deployed_cascade` is
//! where that asymmetry is made explicit rather than assumed.

use tauri::AppHandle;

use crate::{
    app_state::AppState,
    managed_agents::{
        current_instance_id, delete_agent_key, load_managed_agents, load_personas, load_teams,
        save_managed_agents, save_personas, stop_managed_agent_process,
        sync_managed_agent_processes, try_regenerate_nest, validate_persona_deletion,
        ManagedAgentRecord,
    },
};

use super::tombstone_persona_pending;

/// Return pubkeys of every managed agent whose definition is the given persona.
///
/// Pure helper used by `delete_persona` to determine which agent records to
/// cascade-delete. Extracted so the filtering logic can be unit-tested without
/// a full Tauri `AppHandle`.
fn collect_cascade_pubkeys(agents: &[ManagedAgentRecord], persona_id: &str) -> Vec<String> {
    agents
        .iter()
        .filter(|a| a.persona_id.as_deref() == Some(persona_id))
        .map(|a| a.pubkey.clone())
        .collect()
}

/// Names of cascade agents that are provider-deployed: non-local backend with
/// a live `backend_agent_id`.
///
/// Pure helper behind `delete_persona`'s pre-flight. Deleting one of these
/// records is not the same act as deleting a local one: the provider protocol
/// has no undeploy (see `agents.rs`, deploy-only v1), so the remote unit named
/// by `backend_agent_id` keeps running after its record is gone.
fn collect_remote_deployed(
    agents: &[ManagedAgentRecord],
    cascade: &std::collections::HashSet<String>,
) -> Vec<String> {
    agents
        .iter()
        .filter(|a| {
            cascade.contains(&a.pubkey)
                && a.backend != crate::managed_agents::BackendKind::Local
                && a.backend_agent_id.is_some()
        })
        .map(|a| a.name.clone())
        .collect()
}

/// Pre-flight gate for a cascade that contains provider-deployed instances.
///
/// Two-step contract, mirroring `delete_managed_agent`'s `force_remote_delete`
/// parameter: a caller that has not acknowledged the consequence is refused,
/// and a caller that has is allowed through. The consequence is specific — the
/// provider protocol has no undeploy, so cascade-deleting these records leaves
/// their remote units running — which is why the acknowledgement is explicit
/// rather than implied by the delete itself.
///
/// Without the opt-in the error is unchanged from before the opt-in existed,
/// so an IPC caller that never learned about the flag sees identical behavior.
fn preflight_remote_deployed_cascade(
    agents: &[ManagedAgentRecord],
    cascade: &std::collections::HashSet<String>,
    persona_id: &str,
    force_remote_delete: bool,
) -> Result<(), String> {
    if force_remote_delete {
        return Ok(());
    }
    let remote_deployed = collect_remote_deployed(agents, cascade);
    if remote_deployed.is_empty() {
        return Ok(());
    }
    Err(format!(
        "persona {persona_id} has provider-deployed agent instances ({}); delete those agent instances first",
        remote_deployed.join(", ")
    ))
}

/// Remove cascade agents from `agents` and persist via the injectable `save`.
///
/// Extracted from `delete_persona` so unit tests can inject a failing save and
/// verify retry-safety without a full `AppHandle` mock: if `save` returns `Err`,
/// this function propagates it before the keyring deletions and tombstones that
/// appear after the `?` in the call site — nothing is destroyed and the command
/// is safe to retry.
fn commit_cascade_agents(
    agents: &mut Vec<ManagedAgentRecord>,
    cascade: &std::collections::HashSet<String>,
    save: impl FnOnce(&[ManagedAgentRecord]) -> Result<(), String>,
) -> Result<(), String> {
    agents.retain(|a| !cascade.contains(&a.pubkey));
    save(agents)
}

/// Delete a persona and cascade-delete every managed-agent instance built from it.
///
/// `force_remote_delete` acknowledges the one consequence a cascade cannot undo:
/// provider-backed instances are deployed units this app can only forget, never
/// tear down (the provider protocol is deploy-only), so their remote processes
/// keep running after the cascade. Without the flag such a cascade is refused;
/// with it, the instances are deleted along with the persona. Local-only
/// cascades never consult the flag.
#[tauri::command]
pub async fn delete_persona(
    id: String,
    force_remote_delete: Option<bool>,
    app: AppHandle,
) -> Result<(), String> {
    use tauri::Manager;
    tokio::task::spawn_blocking(move || {
        let state = app.state::<AppState>();

        {
            // Store lock held across all three phases.
            // Lock ordering: store lock (acquired here) → process lock (per-agent in Phase 2).
            let _store_guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;

            // Load and validate the persona before any destructive work.
            let mut personas = load_personas(&app)?;
            let persona = personas
                .iter()
                .find(|record| record.id == id)
                .ok_or_else(|| format!("persona {id} not found"))?;
            let referenced_by_team = load_teams(&app)?.iter().any(|team| {
                team.persona_ids
                    .iter()
                    .any(|persona_id| persona_id == id.as_str())
            });
            validate_persona_deletion(persona, referenced_by_team)?;
            // Capture the coordinate before the record might leave the list. Only
            // reached for non-builtin, non-team personas (both rejected above),
            // so every deleted persona here is one this owner published.
            let d_tag = crate::managed_agents::persona_events::persona_d_tag(persona);

            // ── Phase 1: Stage ─────────────────────────────────────────────
            //
            // Load agents, sync process state, and build the cascade set. Lock
            // ordering: store lock (held) → process lock (acquired for sync,
            // then released before Phase 2 stops). Every fallible read/lock is
            // here; an error leaves all state intact and the command is retryable.
            let mut agents = load_managed_agents(&app)?;
            {
                let mut runtimes = state
                    .managed_agent_processes
                    .lock()
                    .map_err(|error| error.to_string())?;
                let (sync_changed, exited_pubkeys) = sync_managed_agent_processes(
                    &mut agents,
                    &mut runtimes,
                    &current_instance_id(&app),
                );
                if sync_changed {
                    save_managed_agents(&app, &agents)?;
                }
                for pk in &exited_pubkeys {
                    state.clear_agent_session_caches(pk);
                }
                // runtimes drops here (process lock released before Phase 2).
            }

            // Build the cascade set. HashSet for O(1) membership in Phase 3.
            let cascade: std::collections::HashSet<String> =
                collect_cascade_pubkeys(&agents, &id).into_iter().collect();

            // Remote-agent pre-flight: run before any destructive work, so an
            // unacknowledged cascade fails with everything still on disk.
            // Nothing in create_managed_agent forbids a persona-linked provider
            // agent, so this is a runtime guard, not an assumed invariant.
            preflight_remote_deployed_cascade(
                &agents,
                &cascade,
                &id,
                force_remote_delete.unwrap_or(false),
            )?;

            // ── Phase 2: Stop ───────────────────────────────────────────────
            //
            // Best-effort stop each running cascade instance. Lock ordering:
            // store lock (held) → process lock acquired per-agent and released
            // between stops so the process lock is not held across the full poll
            // cycle (stop_managed_agent_process polls 100ms×10 before SIGKILL).
            //
            // Per-agent stop errors are swallowed — these records are deleted in
            // Phase 3 regardless. Intentional difference from delete_managed_agent
            // (single-agent, fatal on stop failure); here the cascade is multi-agent
            // and deletion must proceed even if one instance cannot be stopped.
            for pk in &cascade {
                if let Some(rec) = agents.iter_mut().find(|a| a.pubkey == *pk) {
                    let mut runtimes = state
                        .managed_agent_processes
                        .lock()
                        .map_err(|error| error.to_string())?;
                    if let Err(e) = stop_managed_agent_process(&app, rec, &mut runtimes) {
                        eprintln!("buzz-desktop: delete_persona: failed to stop agent {pk}: {e}");
                    }
                    // runtimes drops here (per-agent, process lock not held across stops).
                }
            }

            // ── Phase 3: Commit ─────────────────────────────────────────────
            //
            // Disk-authoritative writes first, side effects strictly after.
            // commit_cascade_agents is an injectable seam so unit tests can
            // verify retry-safety: a failing save propagates before any keyring
            // deletion or tombstone occurs.
            //
            // Failure semantics:
            //   agent save fails   → nothing destroyed; full cascade retries cleanly
            //   persona save fails → cascade agents gone, persona survives; a retry
            //                        finds an empty cascade and proceeds cleanly
            // Keys and tombstones are enqueued only after their records leave disk.
            if !cascade.is_empty() {
                commit_cascade_agents(&mut agents, &cascade, |recs| {
                    save_managed_agents(&app, recs)
                })?;
            }

            let original_len = personas.len();
            personas.retain(|record| record.id != id);
            if personas.len() == original_len {
                return Err(format!("persona {id} not found"));
            }
            save_personas(&app, &personas)?;

            // Side effects — strictly after records leave disk.
            for pk in &cascade {
                state.clear_agent_session_caches(pk);
                // Remove nsec from keyring after the record is gone.
                delete_agent_key(pk);
                crate::commands::agents::tombstone_managed_agent_pending(&app, &state, pk);
                crate::commands::agents::archive_managed_agent_pending(&app, &state, pk);
            }
            tombstone_persona_pending(&app, &state, &d_tag);

            // _store_guard drops here, before try_regenerate_nest.
        }

        try_regenerate_nest(&app);

        Ok(())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
#[path = "delete_cascade_tests.rs"]
mod delete_cascade_tests;
