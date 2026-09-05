mod authority;

use super::{
    bestie_assignment::recover_pending_assignment_cleanup, find_managed_agent_mut,
    kill_stale_tracked_processes, load_managed_agents, load_personas, managed_agents_base_dir,
    save_managed_agents, spawn_agent_child, sync_managed_agent_processes, BackendKind,
    ManagedAgentProcess,
};
use crate::app_state::AppState;
use crate::util;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// Outcome of a Phase B spawn attempt for one restore candidate.
///
/// `Skipped` covers the case where a concurrently-running startup reconcile
/// already spawned and tracked this exact pair during the Phase A window (the
/// transition lock is only held from Phase B onward). Restore must then leave
/// that live child alone rather than terminate-and-respawn it — mirroring the
/// live-child guard in `start_pair` (`runtime_commands.rs`). Without this,
/// restore would kill reconcile's lazy child by its receipt and replace it with
/// an eager one, flipping the pair's laziness on a startup race.
enum SpawnOutcome {
    /// Boxed: the spawned process carries its full spawn-config snapshot, so an
    /// inline variant would make every `Skipped`/`Failed` outcome pay for it.
    Spawned(super::ManagedAgentRuntimeKey, Box<ManagedAgentProcess>),
    Skipped,
    Failed(String),
}
type AgentSpawnResult = (String, SpawnOutcome);

/// Backfill the pinned persona snapshot for pre-existing agents created before
/// the record became the spawn source of truth. Runs once at launch, before
/// `restore_managed_agents_on_launch` spawns anything, so no agent boots from an
/// empty snapshot.
///
/// Only records with a `persona_id` but no `persona_source_version` are touched.
/// Records that already have a `persona_source_version` — including those whose
/// `model`/`provider` were clobbered by the old unconditional snapshot code before
/// this fix — are skipped here; they self-heal on the next manual start via the
/// start-path re-snapshot in `start_local_agent_with_preflight`.
/// If the linked persona is gone, we log loudly and leave the record untouched —
/// it stays orphaned and `spawn_agent_child` refuses to start it (see
/// `effective_config::resolve_effective_config`'s `OrphanedInstance` arm).
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
                "buzz-desktop: persona-snapshot backfill: agent {} links persona {persona_id} which no longer exists; leaving it orphaned — spawn will refuse it",
                record.pubkey
            );
            continue;
        };
        // Layer precedence at read time: persona env < agent env. When the
        // persona leaves model/provider blank, the record's own configured
        // values are preserved — a blank persona must not clobber a
        // user-configured agent. See `apply_persona_snapshot`.
        super::persona_events::apply_persona_snapshot(record, persona);
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
///   B (transition → store): reload authority, then spawn in parallel
///   C (same locks): register children and persist lifecycle receipts
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
    let restore_scope;
    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;

        if shutdown_started.load(Ordering::SeqCst) {
            return Ok(());
        }

        super::private_config_overlay::require_authority_ready(&state)?;
        restore_scope = super::retention::active_retention_scope(app, &state)?.db_path;
        let mut records = load_managed_agents(app)?;
        recover_pending_assignment_cleanup(&managed_agents_base_dir(app)?, |pending_pubkey| {
            records
                .iter()
                .any(|record| record.pubkey.eq_ignore_ascii_case(pending_pubkey))
        })?;
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

        let tracked_pids: Vec<u32> = runtimes
            .values()
            .map(|runtime| runtime.child.id())
            .chain(
                super::read_all_agent_runtime_receipts(app)
                    .into_iter()
                    .filter_map(|(path, receipt)| {
                        super::valid_agent_runtime_receipt(
                            &path,
                            &receipt,
                            &super::current_instance_id(app),
                        )
                        .then_some(receipt.pid)
                    }),
            )
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

        // Exact-path sweep: kill any buzz-acp process whose executable path
        // matches this bundle's harness binary but is not in the tracked set.
        // Complements the env-var sweep above — catches orphans that predate
        // BUZZ_MANAGED_AGENT injection or lost their PID-file receipt.
        //
        // TODO: the three sweeps above each walk the PID table independently.
        // A future consolidation should collect a single shared process snapshot
        // at the top of this block and thread it through all sweep functions,
        // replacing the three separate kernel enumerations.
        super::sweep_untracked_bundle_harnesses(&tracked_pids);

        let candidates = authority::restore_candidate_pubkeys(&records);

        let mut to_start = Vec::new();
        for pubkey in &candidates {
            if let Some(runtime) = runtimes
                .iter_mut()
                .find(|(key, _)| key.pubkey == *pubkey)
                .map(|(_, runtime)| runtime)
            {
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

        // Resolve before backend selection: a Provider disk seed may now be
        // Local on the relay. Persona snapshots remain in memory, never written
        // back over the migration seed during housekeeping.
        let personas_for_snapshot = super::load_personas(app)?;
        let mut resolved_to_start = Vec::with_capacity(agents_to_start.len());
        for record in records
            .iter()
            .filter(|r| agents_to_start.iter().any(|s| s.pubkey == r.pubkey))
        {
            let resolved = crate::managed_agents::private_config_overlay::resolved_local_record(
                &state, record,
            )?;
            if let Some(spawn_record) = finalize_restore_candidate(resolved, &personas_for_snapshot)
            {
                resolved_to_start.push(spawn_record);
            }
        }
        agents_to_start = resolved_to_start;

        if changed {
            save_managed_agents(app, &records)?;
        }
    }

    if agents_to_start.is_empty() {
        return Ok(());
    }

    #[cfg(feature = "mesh-llm")]
    let (agents_to_start, preflight_models) = {
        // Preflight against the same resolution spawn uses — `resolve_effective_config`
        // (definition → global fallback). A linked instance's own `provider`/`model`/
        // `relay_mesh` bytes never contribute. See `start_local_agent_with_preflight`
        // in `commands/agents.rs` for the identical rationale on the interactive path.
        let personas = load_personas(app).unwrap_or_default();
        let global = super::load_global_agent_config(app).unwrap_or_default();
        let mut mesh_preflight_failures = std::collections::HashSet::new();
        let mut preflight_models = std::collections::HashMap::new();
        for record in &agents_to_start {
            let mesh_model_id = super::effective_config::resolve_effective_relay_mesh_model_id(
                record, &personas, &global,
            );
            preflight_models.insert(record.pubkey.clone(), mesh_model_id.clone());
            if mesh_model_id.is_none() {
                continue;
            }
            // Auto-start after relaunch: re-resolve a live bootstrap target and
            // dial it. Skip (with an actionable error) only when no live target
            // serves this model right now.
            if let Err(error) =
                crate::commands::ensure_relay_mesh_for_record(app, mesh_model_id.as_deref(), false)
                    .await
            {
                persist_restore_error(app, &state, &restore_scope, &record.pubkey, error)?;
                mesh_preflight_failures.insert(record.pubkey.clone());
            }
        }
        (
            agents_to_start
                .into_iter()
                .filter(|record| !mesh_preflight_failures.contains(&record.pubkey))
                .collect::<Vec<_>>(),
            preflight_models,
        )
    };
    if agents_to_start.is_empty() {
        return Ok(());
    }

    // Serialize spawning and runtime registration with shutdown cleanup. The
    // shutdown flag is rechecked after taking the lock so shutdown either
    // prevents this transition or waits until every child is tracked and can
    // be terminated.
    let restore_transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }

    // Rebuild immediately before use, after all awaits and transition waiting.
    // Keep this guard through registration so delete/config/scope writers cannot
    // invalidate authority between resolution, spawn, and receipt persistence.
    let mut authority =
        authority::lock_restore_authority(app, &state, &restore_scope, &agents_to_start)?;
    let agents_to_start = std::mem::take(&mut authority.candidates);
    #[cfg(feature = "mesh-llm")]
    {
        let personas = load_personas(app)?;
        let global = super::load_global_agent_config(app)?;
        for record in &agents_to_start {
            let current_model = super::effective_config::resolve_effective_relay_mesh_model_id(
                record, &personas, &global,
            );
            if preflight_models.get(&record.pubkey) != Some(&current_model) {
                return Err(
                    "agent mesh configuration changed during restore; retry initialization".into(),
                );
            }
        }
    }
    let owner_hex = Some(authority.owner_hex.clone());
    let workspace_relay = &authority.workspace_relay;

    // ── Phase B (transition lock held): resolve commands and spawn in parallel ──
    let spawn_results: Vec<AgentSpawnResult> = std::thread::scope(|scope| {
        let owner_hex_ref = owner_hex.as_deref();
        let handles: Vec<_> = agents_to_start
            .iter()
            .filter(|_| !shutdown_started.load(Ordering::SeqCst))
            .map(|record| {
                let handle = scope.spawn(move || {
                    let relay_url =
                        crate::relay::effective_agent_relay_url(&record.relay_url, workspace_relay);
                    let outcome =
                        match super::ManagedAgentRuntimeKey::new(record.pubkey.clone(), &relay_url)
                        {
                            Ok(key) => {
                                // F2: if a concurrent startup reconcile already
                                // tracked a live child for this exact pair during
                                // the Phase A window, leave it alone. Mirrors the
                                // live-child guard in `start_pair`.
                                let already_live = app
                                    .state::<AppState>()
                                    .managed_agent_processes
                                    .lock()
                                    .ok()
                                    .and_then(|mut runtimes| {
                                        runtimes.get_mut(&key).map(|runtime| {
                                            runtime.child.try_wait().ok().flatten().is_none()
                                        })
                                    })
                                    .unwrap_or(false);
                                if already_live {
                                    SpawnOutcome::Skipped
                                } else {
                                    match super::terminate_untracked_pair_runtime(app, &key)
                                        .and_then(|()| {
                                            // F1: restore spawns lazy, matching
                                            // reconcile and manual start. Eager on
                                            // restore buys nothing — a crashed
                                            // mid-turn session is not resumed by an
                                            // eager child — and silently reintroduces
                                            // N idle brains on every launch.
                                            spawn_agent_child(
                                                app,
                                                record,
                                                &key.relay_url,
                                                true,
                                                owner_hex_ref,
                                                None,
                                            )
                                        }) {
                                        Ok(process) => {
                                            SpawnOutcome::Spawned(key, Box::new(process))
                                        }
                                        Err(error) => SpawnOutcome::Failed(error),
                                    }
                                }
                            }
                            Err(error) => SpawnOutcome::Failed(error),
                        };
                    (record.pubkey.clone(), outcome)
                });
                handle
            })
            .collect();

        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    if spawn_results.is_empty() {
        return Ok(());
    }

    // ── Phase C (same authority guard): register and persist lifecycle only ──
    let records = &mut authority.records;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|error| error.to_string())?;

    let mut successfully_spawned: Vec<(String, String)> = Vec::new();

    for (pubkey, outcome) in spawn_results {
        match outcome {
            // Skipped means a concurrent reconcile already owns a live child for
            // this pair; leave its runtime and record state untouched.
            SpawnOutcome::Skipped => continue,
            SpawnOutcome::Spawned(key, mut process) => {
                let Ok(record) = find_managed_agent_mut(records, &pubkey) else {
                    let _ = super::terminate_process(process.child.id());
                    let _ = process.child.wait();
                    continue;
                };
                let now = util::now_iso();
                let receipt = super::ManagedAgentRuntimeReceipt {
                    key: key.clone(),
                    pid: process.child.id(),
                    desktop_instance_id: super::current_instance_id(app),
                    started_at: now.clone(),
                };
                if let Err(error) = super::write_agent_runtime_receipt(app, &receipt) {
                    let _ = super::terminate_process(process.child.id());
                    let _ = process.child.wait();
                    record.updated_at = now;
                    record.last_error = Some(error);
                    continue;
                }
                record.updated_at = now.clone();
                record.runtime_pid = None;
                record.last_started_at = Some(now);
                record.last_stopped_at = None;
                record.last_exit_code = None;
                record.last_error = None;
                runtimes.insert(
                    key.clone(),
                    super::ManagedAgentPairRuntime::starting(*process),
                );
                // Carry the spawn key's relay into profile reconciliation so
                // the background task queries/publishes on the relay this
                // spawn was actually keyed to — not whatever workspace is
                // active when the task eventually executes.
                successfully_spawned.push((pubkey, key.relay_url.clone()));
            }
            SpawnOutcome::Failed(error) => {
                let Ok(record) = find_managed_agent_mut(records, &pubkey) else {
                    continue;
                };
                record.updated_at = util::now_iso();
                record.last_error = Some(error);
            }
        }
    }

    // Collect profile reconciliation data for successfully spawned agents before
    // releasing the lock. This mirrors the fire-and-forget pattern in
    // start_managed_agent — ensuring boot-restored agents get the same profile
    // self-healing as UI-started agents. Read from `agents_to_start` (the
    // overlay-resolved spawn records), not the raw disk `records`, so the
    // reconciled profile matches the config the spawn actually used — the
    // interactive path builds its reconcile data from the resolved record too.
    let reconcile_personas = super::load_personas(app).unwrap_or_default();
    let reconcile_items: Vec<(String, crate::commands::ProfileReconcileData)> =
        successfully_spawned
            .iter()
            .filter_map(|(pubkey, spawn_relay)| {
                let record = agents_to_start.iter().find(|r| r.pubkey == *pubkey)?;
                // Resolve the effective harness for the avatar-fallback
                // derivation (the snapshot may be empty/stale for an inherited
                // harness). Mirrors the UI start path.
                let effective_command =
                    crate::managed_agents::record_agent_command(record, &reconcile_personas);
                Some((
                    pubkey.clone(),
                    crate::commands::ProfileReconcileData {
                        private_key_nsec: record.private_key_nsec.clone(),
                        name: record.name.clone(),
                        relay_url: record.relay_url.clone(),
                        // Pin the relay this spawn was keyed to (see the
                        // successfully_spawned push above) so the deferred
                        // task cannot resolve a post-switch workspace.
                        target_relay_url: Some(spawn_relay.clone()),
                        avatar_url: record.avatar_url.clone(),
                        auth_tag: record.auth_tag.clone(),
                        pubkey: record.pubkey.clone(),
                        agent_command: effective_command,
                        persona_id: record.persona_id.clone(),
                        about: crate::managed_agents::record_effective_description(
                            record,
                            &reconcile_personas,
                        ),
                    },
                ))
            })
            .collect();

    save_managed_agents(app, records)?;
    drop(runtimes);
    drop(authority);
    drop(restore_transition);

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

fn profile_reconcile_completed(outcome: crate::commands::ProfileReconcileOutcome) -> bool {
    outcome == crate::commands::ProfileReconcileOutcome::Reconciled
}

/// Final shaping of a spawn candidate AFTER the relay-overlay
/// resolve, mirroring `start_local_agent_with_preflight` exactly:
///
/// - a resolved backend that is no longer local cannot be spawned by this
///   device — the candidate is dropped (`None`);
/// - the linked persona snapshot is re-applied ON TOP of the overlay patch,
///   so the definition-authoritative quad keeps its established precedence
///   over both disk and relay bytes;
/// - an orphaned instance (persona_id with no live persona) is passed through
///   untouched: `spawn_agent_child` refuses it via
///   `resolve_effective_config`'s `OrphanedInstance` arm and Phase C persists
///   the refusal to `last_error` — restore must not silently drop it.
///
/// Pure over (record, personas) so the restore fold is testable without an
/// `AppHandle`.
fn finalize_restore_candidate(
    mut resolved: super::ManagedAgentRecord,
    personas: &[super::AgentDefinition],
) -> Option<super::ManagedAgentRecord> {
    if resolved.backend != BackendKind::Local {
        return None;
    }
    if let Some(persona_id) = resolved.persona_id.clone() {
        if let Some(persona) = personas.iter().find(|p| p.id == persona_id) {
            super::persona_events::apply_persona_snapshot(&mut resolved, persona);
            resolved.updated_at = util::now_iso();
        }
    }
    Some(resolved)
}

pub(crate) fn spawn_pending_profile_reconciliations(app: &tauri::AppHandle, workspace_relay: &str) {
    let state = app.state::<AppState>();
    if !state
        .managed_agent_profile_reconcile_enabled()
        .load(Ordering::Acquire)
    {
        return;
    }
    let items = match crate::commands::load_pending_profile_reconciliations(app, workspace_relay) {
        Ok(items) => items,
        Err(error) => {
            eprintln!("buzz-desktop: failed to load pending profile reconciliations: {error}");
            return;
        }
    };

    for (pubkey, data) in items {
        let reconcile_app = app.clone();
        let relay_url = data
            .target_relay_url
            .clone()
            .unwrap_or_else(|| data.relay_url.clone());
        tauri::async_runtime::spawn(async move {
            let state = reconcile_app.state::<AppState>();
            match crate::commands::reconcile_agent_profile(&state, &reconcile_app, &pubkey, &data)
                .await
            {
                Ok(outcome) if profile_reconcile_completed(outcome) => {
                    if let Err(error) = crate::commands::mark_profile_reconciled(
                        &reconcile_app,
                        &pubkey,
                        &relay_url,
                    ) {
                        eprintln!(
                            "buzz-desktop: failed to record profile reconciliation for agent {pubkey}: {error}"
                        );
                    }
                }
                Ok(_) => {}
                Err(error) => eprintln!(
                    "buzz-desktop: profile reconciliation failed for agent {pubkey}: {error}"
                ),
            }
        });
    }
}

#[cfg(test)]
mod profile_reconcile_tests {
    use super::profile_reconcile_completed;
    use crate::commands::ProfileReconcileOutcome;

    #[test]
    fn skipped_reconciliation_never_retires_pending_work() {
        assert!(profile_reconcile_completed(
            ProfileReconcileOutcome::Reconciled
        ));
        assert!(!profile_reconcile_completed(
            ProfileReconcileOutcome::SkippedDisabled
        ));
    }
}

// ── Restore fold: relay-overlay resolve before spawn ────────────────────────
//
// The production wiring (Phase A resolving every candidate through
// `resolved_local_record` before Phase B spawns) needs a live `AppHandle`, so
// its presence is pinned by `write_site_resolve_guard` in
// `private_config_overlay.rs`. These tests prove the fold itself: what a
// resolved candidate looks like, at the same overlay + finalize seam the
// production path composes.
#[cfg(test)]
mod restore_fold_tests {
    use super::finalize_restore_candidate;
    use crate::managed_agents::private_config_overlay::PrivateConfigOverlay;
    use crate::managed_agents::{AgentDefinition, BackendKind, ManagedAgentRecord};
    use buzz_core_pkg::private_managed_agent::{
        Payload, PrivateConfig, PrivateIdentity, FORMAT, VERSION,
    };
    use std::collections::BTreeMap;

    fn relay_payload(pubkey: &str) -> Payload {
        Payload {
            format: FORMAT.into(),
            version: VERSION,
            agent_pubkey: pubkey.into(),
            owner_pubkey: "11".repeat(32),
            generation: 2,
            previous_event_id: None,
            updated_at: "2026-08-20T00:00:00Z".into(),
            identity: PrivateIdentity {
                private_key_nsec: "nsec-relay".into(),
                auth_tag: None,
            },
            config: PrivateConfig {
                relay_url: "wss://relay.example".into(),
                name: "relay name".into(),
                persona_id: None,
                runtime: Some("goose".into()),
                model: Some("relay-model".into()),
                provider: None,
                system_prompt: Some("relay prompt".into()),
                parallelism: Some(4),
                respond_to: None,
                respond_to_allowlist: vec![],
                agent_command_override: None,
                agent_args: vec![],
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
                env_vars: BTreeMap::new(),
                backend: serde_json::json!({"type":"local"}),
                backend_agent_id: None,
                team_id: None,
                persona_name_in_team: None,
                relay_mesh: None,
                effort_level: None,
                extra: serde_json::Map::new(),
            },
            extensions: BTreeMap::new(),
            extra: serde_json::Map::new(),
        }
    }

    /// A stale disk record flagged for auto-start, as Phase A collects it.
    fn stale_disk_record(pubkey: &str) -> ManagedAgentRecord {
        serde_json::from_value(serde_json::json!({
            "pubkey": pubkey,
            "name": "stale disk name",
            "private_key_nsec": "nsec-stale-disk",
            "relay_url": "wss://old.example",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "stale disk prompt",
            "model": "stale-model",
            "provider": null,
            "env_vars": {},
            "start_on_app_launch": true,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }))
        .expect("stale_disk_record fixture")
    }

    /// Issue-1 regression (stale-disk / newer-retained-head / start-on-launch):
    /// a hydrated overlay head must win over the disk record for everything
    /// relay-owned, while disk-only lifecycle state (`start_on_app_launch`)
    /// survives untouched — the pair that previously spawned "config A shown
    /// as config B".
    #[test]
    fn restore_candidate_spawns_relay_config_not_stale_disk() {
        let pubkey = "aa".repeat(32);
        let mut overlay = PrivateConfigOverlay::default();
        overlay.insert(relay_payload(&pubkey)).unwrap();

        let disk = stale_disk_record(&pubkey);
        let resolved = overlay.resolve_local_record(&disk);
        let spawn = finalize_restore_candidate(resolved, &[])
            .expect("local candidate must survive the fold");

        assert_eq!(spawn.name, "relay name");
        assert_eq!(spawn.system_prompt.as_deref(), Some("relay prompt"));
        assert_eq!(spawn.model.as_deref(), Some("relay-model"));
        assert_eq!(spawn.private_key_nsec, "nsec-relay");
        assert_eq!(spawn.relay_url, "wss://relay.example");
        assert_eq!(spawn.parallelism, 4);
        assert!(
            spawn.start_on_app_launch,
            "device-local lifecycle flag must survive the overlay resolve"
        );

        // NEGATIVE CONTROL: an empty overlay leaves the disk record as-is —
        // the assertions above are proving the patch, not the fixture.
        let untouched = PrivateConfigOverlay::default().resolve_local_record(&disk);
        assert_eq!(untouched.name, "stale disk name");
        assert_eq!(untouched.private_key_nsec, "nsec-stale-disk");
    }

    /// The linked persona keeps its definition-authoritative precedence over
    /// BOTH disk and relay bytes — mirroring the interactive start path, which
    /// re-applies the snapshot after the overlay resolve.
    #[test]
    fn persona_snapshot_reapplies_on_top_of_overlay_patch() {
        let pubkey = "bb".repeat(32);
        let mut payload = relay_payload(&pubkey);
        payload.config.persona_id = Some("def-1".into());
        let mut overlay = PrivateConfigOverlay::default();
        overlay.insert(payload).unwrap();

        let persona = AgentDefinition {
            id: "def-1".into(),
            display_name: "Definition".into(),
            avatar_url: None,
            description: None,
            system_prompt: "definition prompt".into(),
            runtime: Some("goose".into()),
            model: Some("definition-model".into()),
            provider: None,
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            team_catalog_source: None,
            env_vars: BTreeMap::new(),
            respond_to: None,
            respond_to_allowlist: vec![],
            parallelism: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        };

        let resolved = overlay.resolve_local_record(&stale_disk_record(&pubkey));
        let spawn = finalize_restore_candidate(resolved, std::slice::from_ref(&persona)).unwrap();
        assert_eq!(spawn.system_prompt.as_deref(), Some("definition prompt"));
        assert_eq!(spawn.model.as_deref(), Some("definition-model"));
        // Relay still owns what the definition does not.
        assert_eq!(spawn.name, "relay name");
        assert_eq!(spawn.private_key_nsec, "nsec-relay");
    }

    /// A candidate whose RESOLVED backend is no longer local must not spawn on
    /// this device — same refusal as `start_local_agent_with_preflight`.
    #[test]
    fn non_local_resolved_backend_is_dropped() {
        let pubkey = "cc".repeat(32);
        let mut record = stale_disk_record(&pubkey);
        record.backend = BackendKind::Provider {
            id: "cloud".into(),
            config: serde_json::json!({}),
        };
        assert!(
            finalize_restore_candidate(record, &[]).is_none(),
            "a non-local resolved backend must be refused at the fold"
        );
    }

    /// An orphaned instance (persona_id with no live persona) passes through
    /// unchanged: `spawn_agent_child` owns the refusal so Phase C persists it
    /// to `last_error` — the fold must not silently drop the record.
    #[test]
    fn orphaned_instance_passes_through_for_spawn_refusal() {
        let pubkey = "dd".repeat(32);
        let mut record = stale_disk_record(&pubkey);
        record.persona_id = Some("gone".into());
        let spawn = finalize_restore_candidate(record, &[]).unwrap();
        assert_eq!(spawn.persona_id.as_deref(), Some("gone"));
        assert_eq!(
            spawn.system_prompt.as_deref(),
            Some("stale disk prompt"),
            "no persona to re-apply: record passes through untouched"
        );
    }
}

#[cfg(feature = "mesh-llm")]
fn persist_restore_error(
    app: &tauri::AppHandle,
    state: &AppState,
    expected_scope: &std::path::Path,
    pubkey: &str,
    error: String,
) -> Result<(), String> {
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    if super::retention::active_retention_scope(app, state)?.db_path != expected_scope {
        return Err("workspace changed during managed-agent restore; retry initialization".into());
    }
    super::private_config_overlay::require_authority_ready(state)?;
    let mut records = load_managed_agents(app)?;
    let record = find_managed_agent_mut(&mut records, pubkey)?;
    record.updated_at = util::now_iso();
    record.last_error = Some(error);
    save_managed_agents(app, &records)
}
