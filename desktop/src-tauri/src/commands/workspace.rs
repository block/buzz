use nostr::Keys;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app_state::AppState;
use crate::managed_agents::{
    effective_repos_dir, ensure_repos_symlink, nest_dir, restore_managed_agents_on_launch,
    write_persisted_repos_dir,
};
use crate::relay;

const WORKSPACE_APPLY_SUPERSEDED: &str = "workspace apply superseded by a newer request";

fn next_apply_generation(generation: &std::sync::atomic::AtomicU64) -> u64 {
    generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
}

fn assert_current_apply_generation(
    generation: &std::sync::atomic::AtomicU64,
    ticket: u64,
) -> Result<(), String> {
    if generation.load(Ordering::Acquire) == ticket {
        Ok(())
    } else {
        Err(WORKSPACE_APPLY_SUPERSEDED.to_string())
    }
}

async fn begin_workspace_apply(
    lock: std::sync::Arc<tokio::sync::Mutex<()>>,
    generation: &std::sync::atomic::AtomicU64,
) -> (tokio::sync::OwnedMutexGuard<()>, u64) {
    let guard = lock.lock_owned().await;
    let ticket = next_apply_generation(generation);
    (guard, ticket)
}

#[derive(Deserialize)]
struct RelayInfoIcon {
    #[serde(default)]
    icon: Option<String>,
}

/// Fetch a relay's workspace icon from its NIP-11 relay information document.
///
/// Works for any workspace (active or not) with a plain unauthenticated HTTP
/// GET — no WebSocket session needed. Returns `None` when the relay has no
/// icon set, is unreachable, or serves a malformed document: the rail falls
/// back to initials in all three cases.
#[tauri::command]
pub async fn fetch_workspace_icon(
    relay_url: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let http_url = relay::relay_http_base_url(&relay_url);
    let Ok(response) = state
        .http_client
        .get(&http_url)
        .header("Accept", "application/nostr+json")
        .send()
        .await
    else {
        return Ok(None);
    };
    if !response.status().is_success() {
        return Ok(None);
    }
    let doc = response
        .json::<RelayInfoIcon>()
        .await
        .unwrap_or(RelayInfoIcon { icon: None });
    Ok(doc.icon.filter(|icon| !icon.is_empty()))
}

#[derive(Serialize)]
pub struct ActiveWorkspaceInfo {
    relay_url: String,
    pubkey: String,
}

/// Returns the current active workspace info (relay URL + pubkey).
#[tauri::command]
pub fn get_active_workspace(state: State<'_, AppState>) -> Result<ActiveWorkspaceInfo, String> {
    let pubkey = state.current_pubkey()?;
    let relay_url = relay::relay_ws_url_with_override(&state);
    Ok(ActiveWorkspaceInfo {
        relay_url,
        pubkey: pubkey.to_hex(),
    })
}

/// Validate a candidate `repos_dir` without mutating the filesystem.
///
/// The Add/Edit workspace dialogs call this on submit to block Save on a bad
/// path, so a typo never reaches `apply_workspace`. Reuses the same
/// `validate_repos_dir` the boot/apply path uses — one source of truth for
/// "what's a valid repos dir". An empty/whitespace value clears the override
/// and is valid. `Err` carries the human-readable reason for inline display.
#[tauri::command]
pub async fn validate_repos_dir(dir: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let trimmed = dir.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        let nest = nest_dir().ok_or("cannot resolve home directory for nest")?;
        crate::managed_agents::validate_repos_dir(&nest, trimmed).map(|_| ())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Apply a workspace's configuration to the backend session.
///
/// Called by the frontend on app init (after reload) to configure the
/// Tauri backend with the selected workspace's relay URL, keys, and repos
/// directory.
///
/// Returns `WorkspaceApplyResult`:
/// - `applied: true`, `blocked: None` → new scope committed; post-commit
///   failures surface as `degraded` entries (informational — workspace IS
///   active).
/// - `applied: true`, `blocked: Some(reason)` → scope committed, but
///   post-commit provider-access reconciliation failed hard; dependent
///   post-commit steps were skipped (fail-closed). Workspace IS active but
///   caller must park on the loading gate — retry by re-applying.
/// - `applied: false` → drain failed; old scope still active; `degraded`
///   names what could not be stopped or restored by compensation.
///
/// A bad `repos_dir` is non-fatal: relay/keys always apply (the relay is the
/// active workspace's own choice — orthogonal to the filesystem repos dir),
/// the bad value is NOT persisted (so the next boot starts clean), the
/// `REPOS` symlink is skipped (REPOS stays a real dir), a `repos-dir-error`
/// event surfaces the reason. The dialogs already block a bad path at Save
/// (`validate_repos_dir`); this fallback only catches a value that went bad
/// after save (deleted dir, unmounted volume).
#[tauri::command]
pub async fn apply_workspace(
    relay_url: String,
    nsec: Option<String>,
    repos_dir: Option<String>,
    agent_managed_profiles: Option<bool>,
    app: AppHandle,
) -> Result<crate::managed_agents::scope::WorkspaceApplyResult, String> {
    // ── Layer 1: async serialization lock + Mesh preflight ──────────────────
    // workspace_transition serializes apply_workspace and live identity import
    // so scope transitions are never concurrent.
    //
    // When the `mesh-llm` feature is active, `with_workspace_transition_preflight`
    // acquires the lock AND runs `fail_if_client_mesh_active` under a single guard,
    // with the guard held across the entire async body.  When the feature is off,
    // we acquire the lock inline (no preflight needed).
    #[cfg(feature = "mesh-llm")]
    {
        let app_for_preflight = app.clone();
        return crate::commands::mesh_llm::scope_impl::with_workspace_transition_preflight(
            &app_for_preflight,
            move || {
                Box::pin(apply_workspace_body(
                    relay_url,
                    nsec,
                    repos_dir,
                    agent_managed_profiles,
                    app,
                ))
            },
        )
        .await;
    }
    #[cfg(not(feature = "mesh-llm"))]
    {
        let lock_app = app.clone();
        let lock_state = lock_app.state::<AppState>();
        let _transition_guard = lock_state.workspace_transition.lock().await;
        apply_workspace_body(relay_url, nsec, repos_dir, agent_managed_profiles, app).await
    }
}
async fn apply_workspace_body(
    relay_url: String,
    nsec: Option<String>,
    repos_dir: Option<String>,
    agent_managed_profiles: Option<bool>,
    app: AppHandle,
) -> Result<crate::managed_agents::scope::WorkspaceApplyResult, String> {
    use crate::managed_agents::scope::WorkspaceApplyResult;

    let restore_app = app.clone();
    // #6003: take the apply epoch under the durable apply lock. The owned guard
    // transfers into the fire-and-forget restore spawn below, so a queued apply
    // cannot mutate relay/identity until this apply's restore has finished every
    // mutable workspace read — it outlives the command return that
    // `workspace_transition` bounds. The generation lets each post-`await` phase
    // detect it was superseded by a newer apply and abort.
    let (apply_guard, apply_generation) = {
        let lock_state = restore_app.state::<AppState>();
        begin_workspace_apply(
            lock_state.workspace_apply_lock.clone(),
            &lock_state.workspace_apply_generation,
        )
        .await
    };
    // Capture the caller's relay before the blocking apply. Reading shared
    // state afterward could pick up a newer concurrent community switch.
    let profile_reconcile_relay = relay_url.clone();
    let blocking_result: Result<WorkspaceApplyResult, String> =
        tokio::task::spawn_blocking(move || {
            let state = app.state::<AppState>();

            // ── Validate before mutating ──────────────────────────────────────
            let parsed_keys = match nsec.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(nsec_trimmed) => {
                    Some(Keys::parse(nsec_trimmed).map_err(|e| format!("invalid nsec: {e}"))?)
                }
                None => None,
            };

            // Decide the effective repos_dir from the candidate. A bad path does NOT
            // reject — it is treated as if no override were set: relay/keys still
            // apply, the bad value is not persisted, and a `repos-dir-error` surfaces
            // the reason.
            let nest = nest_dir();
            let effective_repos_dir = match nest.as_deref() {
                Some(nest) => match effective_repos_dir(nest, repos_dir.as_deref()) {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = app.emit("repos-dir-error", error);
                        None
                    }
                },
                None => None,
            };

            // ── Prepare: derive target scope and run staged initialization ────
            // Reversible prepare stage: the old scope remains active throughout.
            let base_dir = crate::managed_agents::managed_agents_base_dir(&app).unwrap_or_default();
            let effective_owner_pubkey = match &parsed_keys {
                Some(keys) => keys.public_key().to_hex(),
                None => state.current_pubkey()?.to_hex(),
            };
            let target_scope_id =
                crate::managed_agents::scope::derive_scope_id(&relay_url, &effective_owner_pubkey);
            let scope_dir =
                crate::managed_agents::scope::scoped_definitions_dir(&base_dir, &target_scope_id);
            crate::managed_agents::scope_init::ensure_scope_ready(
                &target_scope_id,
                &scope_dir,
                &base_dir,
                &effective_owner_pubkey,
            )?;

            // ── Layer 2: drain + commit under one continuous lock ─────────────
            // #6003 defense in depth: this transaction still owns the apply
            // generation before its first mutation (the drain below). A queued
            // apply cannot advance it — it is blocked on `workspace_apply_lock`,
            // which this transaction holds via `apply_guard` — so this assert
            // only ever trips if the invariant is violated.
            assert_current_apply_generation(&state.workspace_apply_generation, apply_generation)?;
            // `managed_agent_runtime_transition` is held from journal creation
            // through the end of the commit swap so no start/reconcile can insert
            // a new runtime into the gap between drain and scope publication.
            //
            // `managed_agents_store_lock` is acquired immediately after
            // `managed_agent_runtime_transition` and held through commit so that
            // a concurrent save_managed_agents (e.g., a runtime status flush)
            // cannot interleave with the drain or the scope swap.
            //
            // All fallible guards (relay_url_override, keys, active_agent_scope)
            // are acquired BEFORE any field is mutated so a poison or other lock
            // failure cannot leave us half-committed with old processes drained.
            let rt_transition = state
                .managed_agent_runtime_transition
                .lock()
                .map_err(|e| e.to_string())?;

            let _store = state
                .managed_agents_store_lock
                .lock()
                .map_err(|e| e.to_string())?;

            // Capture the current (pre-switch) scope so compensation can
            // validate generation before restarting journal entries.
            let pre_switch_scope = state.capture_active_scope();

            // Build the journal and drain under the held transition lock.
            let (stopped_entries, _remaining, drain_error) =
                crate::managed_agents::drain_scope_runtimes(&app, &state);

            if let Some(drain_err) = drain_error {
                // Drain failed — compensate by restarting what we stopped.
                // Drop the store lock BEFORE calling compensate_drain (it
                // re-acquires the store internally), but keep rt_transition
                // held: passing it to compensate_drain closes the interleave
                // window where a concurrent start could slip in between drop
                // and reacquire.
                drop(_store);
                let comp_err = if let Some(scope) = pre_switch_scope.as_ref() {
                    crate::managed_agents::compensate_drain(
                        &app,
                        &stopped_entries,
                        scope,
                        rt_transition,
                    )
                } else {
                    drop(rt_transition);
                    None
                };
                let degraded_msg = match comp_err {
                    Some(comp) => {
                        format!("drain failed ({drain_err}); compensation also failed: {comp}")
                    }
                    None => format!("drain failed ({drain_err}); old runtimes restored"),
                };
                return Ok(WorkspaceApplyResult::drain_failed(degraded_msg));
            }

            // Acquire all fallible commit guards BEFORE mutating any field.
            // If any guard fails, compensation runs and no field has changed.
            // For each failure: drop only _store before compensate_drain (which
            // re-acquires it), but keep rt_transition held through the call.
            let mut override_guard = match state.relay_url_override.lock() {
                Ok(g) => g,
                Err(e) => {
                    drop(_store);
                    let comp_err = if let Some(scope) = pre_switch_scope.as_ref() {
                        crate::managed_agents::compensate_drain(
                            &app,
                            &stopped_entries,
                            scope,
                            rt_transition,
                        )
                    } else {
                        drop(rt_transition);
                        None
                    };
                    let msg = format!(
                        "commit failed (relay lock poisoned: {e}){}",
                        comp_err
                            .map_or_else(String::new, |c| format!("; compensation failed: {c}"))
                    );
                    return Ok(WorkspaceApplyResult::drain_failed(msg));
                }
            };
            let mut keys_guard = match state.identity_lifecycle_keys_guard() {
                Ok(g) => g,
                Err(e) => {
                    drop(override_guard);
                    drop(_store);
                    let comp_err = if let Some(scope) = pre_switch_scope.as_ref() {
                        crate::managed_agents::compensate_drain(
                            &app,
                            &stopped_entries,
                            scope,
                            rt_transition,
                        )
                    } else {
                        drop(rt_transition);
                        None
                    };
                    let msg = format!(
                        "commit failed (keys lock poisoned: {e}){}",
                        comp_err
                            .map_or_else(String::new, |c| format!("; compensation failed: {c}"))
                    );
                    return Ok(WorkspaceApplyResult::drain_failed(msg));
                }
            };
            let mut scope_guard = match state.active_agent_scope.lock() {
                Ok(g) => g,
                Err(e) => {
                    drop(keys_guard);
                    drop(override_guard);
                    drop(_store);
                    let comp_err = if let Some(scope) = pre_switch_scope.as_ref() {
                        crate::managed_agents::compensate_drain(
                            &app,
                            &stopped_entries,
                            scope,
                            rt_transition,
                        )
                    } else {
                        drop(rt_transition);
                        None
                    };
                    let msg = format!(
                        "commit failed (scope lock poisoned: {e}){}",
                        comp_err
                            .map_or_else(String::new, |c| format!("; compensation failed: {c}"))
                    );
                    return Ok(WorkspaceApplyResult::drain_failed(msg));
                }
            };

            // ── Infallible commit: all guards held, no .await, no I/O ─────────
            *override_guard = Some(relay_url.clone());
            drop(override_guard);
            crate::relay_admission::reset_gate_for_workspace_change();

            if let Some(new_keys) = parsed_keys {
                *keys_guard = new_keys;
            }
            let owner_pubkey = keys_guard.public_key().to_hex();
            drop(keys_guard);

            state
                .managed_agent_profile_reconcile_enabled
                .store(!agent_managed_profiles.unwrap_or(false), Ordering::Release);

            let generation = crate::managed_agents::scope::next_scope_generation();
            let scope = crate::managed_agents::scope::WorkspaceAgentScope::new(
                relay_url,
                owner_pubkey,
                &base_dir,
                generation,
            );
            *scope_guard = Some(scope);
            drop(scope_guard);
            drop(rt_transition);

            // ── Filesystem side-effects (non-fatal) ───────────────────────────
            if let Some(nest) = nest.as_deref() {
                if let Err(error) = write_persisted_repos_dir(nest, effective_repos_dir.as_deref())
                {
                    eprintln!("buzz-desktop: persist repos dir failed: {error}");
                }
                if let Err(error) = ensure_repos_symlink(nest, effective_repos_dir.as_deref()) {
                    eprintln!("buzz-desktop: repos dir setup failed: {error}");
                    let _ = app.emit("repos-dir-error", error);
                }
            }

            Ok::<WorkspaceApplyResult, String>(WorkspaceApplyResult::success())
        })
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))?;

    // If blocking returned a drain-failed result, surface it now.
    let apply_result = blocking_result?;
    if !apply_result.applied {
        return Ok(apply_result);
    }

    // ── Post-commit (non-rollback) ──────────────────────────────────────
    // The workspace HAS switched. Post-commit failures surface as degradation
    // on the applied result — we never pretend the old scope survived.
    // #6003: re-assert the epoch before the awaited post-commit phases. We hold
    // `apply_guard` across the whole body, so a queued apply is still blocked on
    // `workspace_apply_lock` and cannot have advanced the generation; this is
    // defense in depth against a future refactor that releases the guard early.
    {
        let state = restore_app.state::<AppState>();
        assert_current_apply_generation(&state.workspace_apply_generation, apply_generation)?;
    }
    let mut degraded: Vec<String> = Vec::new();

    // Nest context reflects the active scope's agents.md — regenerate now that
    // the scope is committed. Awaited (not fire-and-forget) so a regeneration
    // failure surfaces as applied-but-degraded rather than vanishing on a
    // spawned task. Agents still run fine against a stale AGENTS.md.
    if let Err(error) = crate::managed_agents::regenerate_nest_now(&restore_app).await {
        degraded.push(format!("nest context regeneration failed: {error}"));
    }

    let state = restore_app.state::<AppState>();
    if let Err(reason) =
        super::agents::provider_access::reconcile_on_workspace_apply(&restore_app, &state).await
    {
        // Provider-access reconciliation failed after the scope committed.
        // Preserves #4053's fail-closed intent: no agents spawn against a
        // workspace whose provider deployment may not have accepted
        // owner-only access. Return applied-but-blocked so the frontend
        // can park on the loading gate with a truthful error rather than
        // falsely treating the workspace as unapplied.
        return Ok(
            crate::managed_agents::scope::WorkspaceApplyResult::applied_but_blocked(
                reason, degraded,
            ),
        );
    }

    // The Bumble→Pollen migration may have renamed stopped agents. Reconcile
    // their relay profiles independently of runtime restore; successful writes
    // record this relay while retaining the agent for other communities, and
    // failures retry on the next workspace apply. The loader is scope-resolved
    // (scoped-store queue path), so this runs after the scope has committed.
    crate::managed_agents::spawn_pending_profile_reconciliations(
        &restore_app,
        &profile_reconcile_relay,
    );

    // Backfill this exact relay+owner scope only after the workspace has been
    // applied. Running at process boot would target the fallback relay and
    // collapse every community into one pending-event store.
    match crate::managed_agents::retention::active_retention_scope(&restore_app, &state) {
        Ok(scope) => {
            if let Some(agent_scope) = state.capture_active_scope() {
                // Per-apply team-membership repair. Main runs the repair on
                // every boot (`repair_then_detach_teams`); its drift source #2
                // — adding a persona to a team not backfilling running
                // instances' `team_id` — is ongoing, not one-shot, so a
                // once-per-scope-lifetime repair in scope-init would be a silent
                // downgrade. Repair-only (no detach; detach stays one-shot in
                // scope-init) and upstream of the superseding-head write below
                // so the fatal team leg retains the corrected roster.
                // Best-effort: a failure is logged and does not block the apply;
                // the corrected roster is re-derived on the next apply.
                if let Err(error) =
                    crate::migration::repair_team_membership_in_dir(&agent_scope.definitions_dir)
                {
                    eprintln!("buzz-desktop: per-apply team-membership repair failed: {error}");
                }
                // Legacy global-retention adoption (main's per-apply
                // `migrate_legacy_retention_into`) is dropped as subsumed:
                // scope-init's pre-Ready family already runs
                // `migrate_legacy_retention_db` (Step A) into the identical
                // scoped `db_path` this scope resolves, and the READY_MARKER v2
                // bump re-runs that step for scopes marked Ready by the v1
                // pipeline. The adoption is one-shot per scope by design, so the
                // scope-init call fully covers it.
                //
                // Await the reconcile to completion — do NOT spawn it — and
                // propagate its failure. The boot migration may have repaired
                // team membership on disk; the frontend starts inbound history
                // replay the moment `useCommunityInit` observes the applied
                // workspace, and an old relay team head could otherwise win that
                // race and overwrite the repaired `persona_ids`. The team leg is
                // fatal (see `run_event_sync`): only its success durably retains
                // the corrected head with a superseding `monotonic_created_at`,
                // so `retain_inbound_event`'s equal/older guard rejects the
                // stale head. On failure we return `Err` — the command reports
                // failure, `useCommunityInit` never exposes the community, and
                // inbound replay never starts against an un-superseded disk
                // state.
                crate::event_sync::run_event_sync_blocking(
                    restore_app.clone(),
                    scope.owner_keys,
                    scope.db_path,
                    agent_scope.definitions_dir,
                )
                .await?;
            } else {
                degraded.push(
                    "active agent scope unavailable after workspace apply — event sync skipped"
                        .to_string(),
                );
            }
        }
        Err(error) => {
            // Scope resolution is a prerequisite for establishing the
            // superseding head, so its failure is fatal for the same reason:
            // without a scope we cannot retain the repaired roster ahead of an
            // inbound replay. Fail the command rather than silently opening the
            // inbound lane.
            return Err(format!(
                "scoped event-sync unavailable after workspace apply: {error}"
            ));
        }
    }

    // Per-transition restore: always restore the new scope's auto-start agents
    // (replaces the launch-only `managed_agent_restore_pending.swap` one-shot).
    // Fire-and-forget spawn so the command returns promptly; restore failures
    // are surfaced as a structured `workspace-degraded` event consumed by the UI.
    #[cfg(feature = "mesh-llm")]
    {
        let app = restore_app.clone();
        // #6003: transfer the apply guard into the restore task so a queued
        // apply stays blocked on `workspace_apply_lock` until restore's every
        // mutable workspace read completes — the guard outlives this return.
        let restore_lock = apply_guard;
        tauri::async_runtime::spawn(async move {
            let _restore_lock = restore_lock;
            let state = app.state::<AppState>();
            // Restore mesh sharing first so a slow stopped-status request cannot
            // overwrite a newly restored serving status.
            if let Err(error) = crate::commands::mesh_llm::restore_mesh_sharing(&app, &state).await
            {
                eprintln!("buzz-desktop: failed to restore Share Compute: {error}");
            }
            crate::mesh_llm::publish_current_status_once(&app, "workspace apply").await;
            if let Err(error) =
                restore_managed_agents_on_launch(&app, &state.shutdown_started).await
            {
                let msg = format!("agent restore failed: {error}");
                eprintln!("buzz-desktop: {msg}");
                let _ = app.emit("workspace-degraded", &msg);
            }
        });
    }

    #[cfg(not(feature = "mesh-llm"))]
    {
        let app = restore_app.clone();
        // #6003: transfer the apply guard into the restore task (see mesh-llm
        // branch) so a queued apply cannot mutate relay/identity until restore
        // finishes.
        let restore_lock = apply_guard;
        tauri::async_runtime::spawn(async move {
            let _restore_lock = restore_lock;
            let state = app.state::<AppState>();
            if let Err(error) =
                restore_managed_agents_on_launch(&app, &state.shutdown_started).await
            {
                let msg = format!("agent restore failed: {error}");
                eprintln!("buzz-desktop: {msg}");
                let _ = app.emit("workspace-degraded", &msg);
            }
        });
    }

    if degraded.is_empty() {
        Ok(WorkspaceApplyResult::success())
    } else {
        Ok(degraded
            .into_iter()
            .fold(WorkspaceApplyResult::success(), |r, msg| {
                r.with_degradation(msg)
            }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };

    use super::{assert_current_apply_generation, begin_workspace_apply, next_apply_generation};

    #[test]
    fn explicit_newer_generation_supersedes_older_ticket() {
        let generation = AtomicU64::new(0);
        let older = next_apply_generation(&generation);
        let newer = next_apply_generation(&generation);

        let error = assert_current_apply_generation(&generation, older).unwrap_err();
        assert!(error.contains("superseded"), "{error}");
        assert_current_apply_generation(&generation, newer).unwrap();
    }

    #[tokio::test]
    async fn queued_apply_cannot_supersede_running_transaction_or_restore_phase() {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let generation = Arc::new(AtomicU64::new(0));
        let (running_guard, running_ticket) =
            begin_workspace_apply(Arc::clone(&lock), &generation).await;

        let queued_lock = Arc::clone(&lock);
        let queued_generation = Arc::clone(&generation);
        let queued = tokio::spawn(async move {
            let (_guard, ticket) = begin_workspace_apply(queued_lock, &queued_generation).await;
            ticket
        });
        tokio::task::yield_now().await;

        // A queued workspace has not advanced the generation, so every awaited
        // phase of the running transaction, including one-shot launch restore,
        // remains authoritative while it holds the lock.
        assert_eq!(generation.load(Ordering::Acquire), running_ticket);
        assert_current_apply_generation(&generation, running_ticket).unwrap();
        assert!(!queued.is_finished());

        drop(running_guard);
        let queued_ticket = queued.await.unwrap();
        assert!(queued_ticket > running_ticket);
        assert_current_apply_generation(&generation, queued_ticket).unwrap();
    }
}
