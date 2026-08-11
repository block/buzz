use super::{
    find_managed_agent_mut, kill_stale_tracked_processes, load_managed_agents, load_personas,
    save_managed_agents, spawn_agent_child, sync_managed_agent_processes, BackendKind,
    ManagedAgentProcess,
};
use crate::app_state::AppState;
use crate::util;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// Workspace boundary captured after authoritative launch state is hydrated.
/// Restore may do preparatory work without holding the runtime-transition lock,
/// but it must still own this exact boundary when it crosses into spawning.
pub struct ManagedAgentRestoreScope {
    pub owner_pubkey: String,
    pub relay_url: String,
    pub db_path: PathBuf,
}

/// A restore remains pending until the production spawn/persistence seam says
/// it completed. Dropping an attempt after any error deliberately leaves the
/// flag set so a later authoritative bootstrap can retry.
struct PendingRestoreAttempt<'a> {
    pending: &'a AtomicBool,
    failed_agents: HashSet<String>,
}

impl<'a> PendingRestoreAttempt<'a> {
    fn begin(pending: &'a AtomicBool) -> Option<Self> {
        pending.load(Ordering::Acquire).then_some(Self {
            pending,
            failed_agents: HashSet::new(),
        })
    }

    fn record_failure(&mut self, pubkey: &str) {
        self.failed_agents.insert(pubkey.to_string());
    }

    fn finish(self) -> Result<(), String> {
        if self.failed_agents.is_empty() {
            self.pending.store(false, Ordering::Release);
            Ok(())
        } else {
            let agent_label = if self.failed_agents.len() == 1 {
                "agent"
            } else {
                "agents"
            };
            Err(format!(
                "managed-agent restore incomplete for {} {agent_label}; reconnect to retry",
                self.failed_agents.len(),
            ))
        }
    }
}

fn restore_scope_matches(
    expected: &ManagedAgentRestoreScope,
    active: &super::retention::RetentionScope,
) -> bool {
    active.db_path == expected.db_path
        && active
            .owner_keys
            .public_key()
            .to_hex()
            .eq_ignore_ascii_case(&expected.owner_pubkey)
}

/// Outcome of a Phase B spawn attempt for one restore candidate.
///
/// `Skipped` covers the case where a concurrently-running startup reconcile
/// already spawned and tracked this exact pair during the Phase A window (the
/// transition lock is only held from Phase B onward). Restore must then leave
/// that live child alone rather than terminate-and-respawn it — mirroring the
/// live-child guard in `start_pair` (`runtime_commands.rs`). Without this,
/// restore would kill reconcile's lazy child by its receipt and replace it with
/// an eager one, flipping the pair's laziness on a startup race.
struct PendingSpawnedProcess(Option<Box<ManagedAgentProcess>>);

impl PendingSpawnedProcess {
    fn new(process: ManagedAgentProcess) -> Self {
        Self(Some(Box::new(process)))
    }

    fn process_mut(&mut self) -> Option<&mut ManagedAgentProcess> {
        self.0.as_deref_mut()
    }

    fn adopt(mut self) -> Option<Box<ManagedAgentProcess>> {
        self.0.take()
    }
}

impl Drop for PendingSpawnedProcess {
    fn drop(&mut self) {
        let Some(mut process) = self.0.take() else {
            return;
        };
        let _ = super::terminate_process(process.child.id());
        let _ = process.child.wait();
    }
}

enum SpawnOutcome {
    /// Boxed: the spawned process carries its full spawn-config snapshot, so an
    /// inline variant would make every `Skipped`/`Failed` outcome pay for it.
    Spawned(super::ManagedAgentRuntimeKey, PendingSpawnedProcess),
    Skipped,
    Failed(String),
}
type AgentSpawnResult = (String, SpawnOutcome);

/// Resolve the exact records Phase B may hand to `spawn_agent_child`.
///
/// Candidate selection remains device-local (`start_on_app_launch` and process
/// lifecycle), while every portable/private launch field comes from the
/// authoritative overlay. The current device-local definition is applied last
/// because its prompt/model/provider/runtime remain template-owned.
fn resolve_restore_spawn_records(
    records: &[super::ManagedAgentRecord],
    candidate_pubkeys: &HashSet<&str>,
    overlay: &super::private_config_overlay::PrivateConfigOverlay,
    personas: &[super::AgentDefinition],
    updated_at: &str,
) -> Vec<super::ManagedAgentRecord> {
    records
        .iter()
        .filter(|record| candidate_pubkeys.contains(record.pubkey.as_str()))
        .filter_map(|record| {
            let mut resolved = overlay.resolve_local_record(record);
            if resolved.backend != BackendKind::Local {
                return None;
            }
            if let Some(persona_id) = resolved.persona_id.clone() {
                if let Some(persona) = personas.iter().find(|persona| persona.id == persona_id) {
                    super::persona_events::apply_persona_snapshot(&mut resolved, persona);
                    resolved.updated_at = updated_at.to_string();
                }
            }
            Some(resolved)
        })
        .collect()
}

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
///   B (no locks):   resolve commands and spawn processes in parallel
///   C (re-lock):    write back PIDs and status to records on disk
pub async fn restore_managed_agents_on_launch(
    app: &tauri::AppHandle,
    shutdown_started: &AtomicBool,
    expected_scope: &ManagedAgentRestoreScope,
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

        let candidates: Vec<String> = records
            .iter()
            .filter(|record| record.start_on_app_launch)
            .map(|record| record.pubkey.clone())
            .collect();

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
                // Orphaned: no current persona to re-snapshot from. Leave the
                // record as-is — `spawn_agent_child` (Phase B below) refuses to
                // spawn it and Phase C persists the refusal to `last_error`.
                continue;
            };
            super::persona_events::apply_persona_snapshot(record, persona);
            record.updated_at = util::now_iso();
            changed = true;
        }
        // Build the actual spawn records from the authoritative private-config
        // overlay, then re-apply the device-local definition. The disk records
        // above remain lifecycle/migration state and must never become the
        // launch source merely because restore runs at boot.
        let candidate_pubkeys: HashSet<_> = agents_to_start
            .iter()
            .map(|record| record.pubkey.as_str())
            .collect();
        let overlay = state
            .private_managed_agent_overlay
            .lock()
            .map_err(|error| error.to_string())?;
        agents_to_start = resolve_restore_spawn_records(
            &records,
            &candidate_pubkeys,
            &overlay,
            &personas_for_snapshot,
            &util::now_iso(),
        );
        drop(overlay);

        if changed {
            save_managed_agents(app, &records)?;
        }
    }

    // The owner and relay used by every child come from the same boundary that
    // completed backfill, never from mutable workspace state during restore.
    let owner_hex = Some(expected_scope.owner_pubkey.clone());

    #[cfg(feature = "mesh-llm")]
    let (agents_to_start, mesh_preflight_failures) = {
        // Preflight against the same resolution spawn uses — `resolve_effective_config`
        // (definition → global fallback). A linked instance's own `provider`/`model`/
        // `relay_mesh` bytes never contribute. See `start_local_agent_with_preflight`
        // in `commands/agents.rs` for the identical rationale on the interactive path.
        let personas = load_personas(app).unwrap_or_default();
        let global = super::load_global_agent_config(app).unwrap_or_default();
        let mut mesh_preflight_failures = std::collections::HashSet::new();
        for record in &agents_to_start {
            let mesh_model_id = super::effective_config::resolve_effective_relay_mesh_model_id(
                record, &personas, &global,
            );
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
                persist_restore_error(app, &state, &record.pubkey, error)?;
                mesh_preflight_failures.insert(record.pubkey.clone());
            }
        }
        (
            agents_to_start
                .into_iter()
                .filter(|record| !mesh_preflight_failures.contains(&record.pubkey))
                .collect::<Vec<_>>(),
            mesh_preflight_failures,
        )
    };
    #[cfg(not(feature = "mesh-llm"))]
    let mesh_preflight_failures = HashSet::<String>::new();
    // Serialize spawning and runtime registration with shutdown cleanup. The
    // same lock also serializes workspace mutation. Revalidate the captured
    // scope after taking it: whichever transition wins determines whether this
    // restore runs or becomes a harmless stale completion.
    let restore_transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|error| error.to_string())?;
    if shutdown_started.load(Ordering::SeqCst) {
        return Ok(());
    }
    let Some(active_scope) =
        super::retention::arrival_retention_scope(app, &state, &expected_scope.relay_url)?
    else {
        return Ok(());
    };
    if !restore_scope_matches(expected_scope, &active_scope) {
        return Ok(());
    }
    // Another completion may have waited on this transition while the first
    // completed. Recheck inside the serialized boundary so it cannot launch a
    // duplicate restore.
    let Some(mut restore_attempt) =
        PendingRestoreAttempt::begin(&state.managed_agent_restore_pending)
    else {
        return Ok(());
    };
    for pubkey in mesh_preflight_failures {
        restore_attempt.record_failure(&pubkey);
    }
    if agents_to_start.is_empty() {
        return restore_attempt.finish();
    }

    // ── Phase B (transition lock held): resolve commands and spawn in parallel ──
    let spawn_results: Vec<AgentSpawnResult> = std::thread::scope(|scope| {
        let owner_hex_ref = owner_hex.as_deref();
        let handles: Vec<_> = agents_to_start
            .iter()
            .filter(|_| !shutdown_started.load(Ordering::SeqCst))
            .map(|record| {
                let handle = scope.spawn(move || {
                    let relay_url = crate::relay::effective_agent_relay_url(
                        &record.relay_url,
                        &expected_scope.relay_url,
                    );
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
                                            )
                                        }) {
                                        Ok(process) => SpawnOutcome::Spawned(
                                            key,
                                            PendingSpawnedProcess::new(process),
                                        ),
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
        return restore_attempt.finish();
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

    for (pubkey, outcome) in spawn_results {
        match outcome {
            // Skipped means a concurrent reconcile already owns a live child for
            // this pair; leave its runtime and record state untouched.
            SpawnOutcome::Skipped => continue,
            SpawnOutcome::Spawned(key, mut pending_process) => {
                let Ok(record) = find_managed_agent_mut(&mut records, &pubkey) else {
                    continue;
                };
                let Some(process) = pending_process.process_mut() else {
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
                    restore_attempt.record_failure(&pubkey);
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
                let Some(process) = pending_process.adopt() else {
                    continue;
                };
                runtimes.insert(key, super::ManagedAgentPairRuntime::starting(*process));
                successfully_spawned.push(pubkey);
            }
            SpawnOutcome::Failed(error) => {
                restore_attempt.record_failure(&pubkey);
                let Ok(record) = find_managed_agent_mut(&mut records, &pubkey) else {
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
                let effective_command =
                    crate::managed_agents::record_agent_command(record, &reconcile_personas);
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
    // A restore request is consumed only when every requested agent crossed the
    // spawn, receipt, adoption, and persistence boundary. Successful agents are
    // already tracked, so a retry skips them and retries only the failed agents.
    let restore_result = restore_attempt.finish();
    drop(runtimes);
    drop(_store_guard);
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

    restore_result
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

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core_pkg::private_managed_agent::{
        Payload, PrivateConfig, PrivateIdentity, FORMAT, VERSION,
    };
    use serde_json::{json, Map};
    use std::collections::BTreeMap;

    #[test]
    fn failed_restore_attempt_remains_pending_for_retry() {
        let pending = AtomicBool::new(true);

        {
            let _first_attempt = PendingRestoreAttempt::begin(&pending).unwrap();
            // Leaving this scope models the production restore returning an error.
        }
        assert!(pending.load(Ordering::Acquire));

        PendingRestoreAttempt::begin(&pending)
            .unwrap()
            .finish()
            .unwrap();
        assert!(!pending.load(Ordering::Acquire));
        assert!(PendingRestoreAttempt::begin(&pending).is_none());
    }

    #[test]
    fn partial_restore_failure_stays_pending_until_retry_succeeds() {
        let pending = AtomicBool::new(true);
        let mut first_attempt = PendingRestoreAttempt::begin(&pending).unwrap();

        // One agent crossed the production boundary; another did not. Only
        // failures are recorded, so successful agents remain live and Phase A
        // will skip them when the pending attempt is retried.
        first_attempt.record_failure(&"bb".repeat(32));
        let error = first_attempt.finish().unwrap_err();
        assert!(error.contains("1 agent"));
        assert!(pending.load(Ordering::Acquire));

        PendingRestoreAttempt::begin(&pending)
            .unwrap()
            .finish()
            .unwrap();
        assert!(!pending.load(Ordering::Acquire));
    }

    #[test]
    fn restore_scope_rejects_a_workspace_that_changed_before_spawn() {
        let owner_a = nostr::Keys::generate();
        let owner_b = nostr::Keys::generate();
        let expected = ManagedAgentRestoreScope {
            owner_pubkey: owner_a.public_key().to_hex(),
            relay_url: "wss://community-a.example".into(),
            db_path: PathBuf::from("scope-a.db"),
        };
        let matching = super::super::retention::RetentionScope {
            db_path: PathBuf::from("scope-a.db"),
            relay_url: "wss://community-a.example".into(),
            owner_keys: owner_a,
        };
        let switched = super::super::retention::RetentionScope {
            db_path: PathBuf::from("scope-b.db"),
            relay_url: "wss://community-b.example".into(),
            owner_keys: owner_b,
        };

        assert!(restore_scope_matches(&expected, &matching));
        assert!(!restore_scope_matches(&expected, &switched));
    }

    #[cfg(unix)]
    #[test]
    fn spawn_guard_terminates_child_when_phase_c_cannot_adopt() {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "sleep 30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.process_group(0);
        let child = command.spawn().expect("spawn external restore child");
        let pid = child.id();
        let process = ManagedAgentProcess {
            child,
            log_path: PathBuf::new(),
            spawn_config: super::super::spawn_snapshot::prospective_spawn_config_snapshot(
                &disk_record(&"aa".repeat(32)),
                &[],
                &[],
                "wss://relay.example",
                &Default::default(),
            ),
            setup_mode: false,
            adapter_availability: None,
            start_nonce: "restore-test".into(),
        };

        drop(PendingSpawnedProcess::new(process));

        assert!(!super::super::process_is_running(pid));
    }

    fn disk_record(pubkey: &str) -> super::super::ManagedAgentRecord {
        serde_json::from_value(json!({
            "pubkey": pubkey,
            "name": "Disk agent",
            "private_key_nsec": "nsec-disk",
            "auth_tag": "disk-auth",
            "relay_url": "wss://relay.example",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "parallelism": 1,
            "system_prompt": "STALE disk prompt",
            "model": "disk-model",
            "env_vars": {"API_TOKEN": "disk-token"},
            "start_on_app_launch": true,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }

    #[test]
    fn restore_spawn_record_uses_relay_config_after_delayed_backfill() {
        let pubkey = "aa".repeat(32);
        let disk = disk_record(&pubkey);
        let mut overlay = super::super::private_config_overlay::PrivateConfigOverlay::default();
        overlay
            .insert(Payload {
                format: FORMAT.into(),
                version: VERSION,
                agent_pubkey: pubkey.clone(),
                owner_pubkey: "bb".repeat(32),
                generation: 2,
                previous_event_id: Some("cc".repeat(32)),
                updated_at: "2026-08-07T00:00:00Z".into(),
                identity: PrivateIdentity {
                    private_key_nsec: "nsec-relay".into(),
                    auth_tag: Some("relay-auth".into()),
                },
                config: PrivateConfig {
                    relay_url: "wss://relay.example".into(),
                    name: "Relay agent".into(),
                    persona_id: None,
                    runtime: Some("goose".into()),
                    model: Some("relay-model".into()),
                    provider: None,
                    system_prompt: Some("FRESH relay prompt".into()),
                    parallelism: Some(7),
                    respond_to: None,
                    respond_to_allowlist: vec![],
                    agent_command_override: None,
                    agent_args: vec![],
                    idle_timeout_seconds: None,
                    max_turn_duration_seconds: None,
                    env_vars: BTreeMap::from([("API_TOKEN".into(), "relay-token".into())]),
                    backend: json!({"type":"local"}),
                    backend_agent_id: None,
                    team_id: None,
                    persona_name_in_team: None,
                    relay_mesh: None,
                    extra: Map::new(),
                },
                extensions: BTreeMap::new(),
                extra: Map::new(),
            })
            .unwrap();
        let candidates = HashSet::from([pubkey.as_str()]);

        let resolved = resolve_restore_spawn_records(
            &[disk],
            &candidates,
            &overlay,
            &[],
            "2026-08-07T00:00:01Z",
        );

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "Relay agent");
        assert_eq!(resolved[0].private_key_nsec, "nsec-relay");
        assert_eq!(resolved[0].auth_tag.as_deref(), Some("relay-auth"));
        assert_eq!(
            resolved[0].system_prompt.as_deref(),
            Some("FRESH relay prompt")
        );
        assert_eq!(resolved[0].model.as_deref(), Some("relay-model"));
        assert_eq!(resolved[0].parallelism, 7);
        assert_eq!(resolved[0].env_vars["API_TOKEN"], "relay-token");

        let spawn_snapshot = super::super::spawn_snapshot::prospective_spawn_config_snapshot(
            &resolved[0],
            &[],
            &[],
            "wss://relay.example",
            &Default::default(),
        )
        .canonical();
        assert_eq!(spawn_snapshot["system_prompt"], "FRESH relay prompt");
        assert_eq!(spawn_snapshot["model"], "relay-model");
        assert_eq!(spawn_snapshot["env"]["API_TOKEN"], "relay-token");
        assert_eq!(spawn_snapshot["parallelism"], 7);
    }
}
