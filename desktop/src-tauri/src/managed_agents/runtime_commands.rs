use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager};

use super::{
    agent_readiness, append_log_marker, current_instance_id, find_managed_agent_mut,
    load_global_agent_config, load_managed_agents, load_personas, managed_agent_runtime_log_path,
    process_is_running, record_agent_command, resolve_effective_agent_env, save_managed_agents,
    spawn_agent_child, terminate_process, terminate_untracked_pair_runtime,
    write_agent_runtime_receipt, AgentReadiness, BackendKind, ManagedAgentPairRuntime,
    ManagedAgentRuntimeKey, ManagedAgentRuntimeLifecycle, ManagedAgentRuntimeReceipt,
    ManagedAgentRuntimeStatus,
};
use crate::app_state::AppState;
use crate::managed_agents::global_config::load_global_agent_config_at;
use crate::managed_agents::personas::load_personas_at;
use crate::managed_agents::storage::{load_managed_agents_at, save_managed_agents_at};

const STATUS_EVENT: &str = "managed-agent-runtime-status";

fn status_for<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    record: &super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    runtime: Option<&ManagedAgentPairRuntime>,
    requested_relay_url: Option<String>,
) -> ManagedAgentRuntimeStatus {
    let personas = load_personas(app).unwrap_or_default();
    let global = load_global_agent_config(app).unwrap_or_default();
    status_for_with(
        app,
        record,
        key,
        runtime,
        requested_relay_url,
        StatusInputs {
            personas: &personas,
            global: &global,
        },
    )
}

/// Preloaded per-call-site inputs for [`status_for_with`], so multi-row
/// callers (list, reconcile) hit disk once instead of once per row.
struct StatusInputs<'a> {
    personas: &'a [super::AgentDefinition],
    global: &'a super::GlobalAgentConfig,
}

fn status_for_with<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    record: &super::ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    runtime: Option<&ManagedAgentPairRuntime>,
    requested_relay_url: Option<String>,
    inputs: StatusInputs<'_>,
) -> ManagedAgentRuntimeStatus {
    let StatusInputs { personas, global } = inputs;
    let command = record_agent_command(record, personas);
    let metadata = super::known_acp_runtime(&command);
    let effective = resolve_effective_agent_env(record, personas, metadata, global);
    let local_setup = matches!(agent_readiness(&effective), AgentReadiness::Ready);
    ManagedAgentRuntimeStatus {
        pubkey: key.pubkey.clone(),
        relay_url: key.relay_url.clone(),
        requested_relay_url,
        local_setup,
        lifecycle: runtime
            .map(|runtime| runtime.lifecycle.clone())
            .unwrap_or(ManagedAgentRuntimeLifecycle::Stopped),
        pid: runtime.map(|runtime| runtime.child.id()),
        error: runtime.and_then(|runtime| runtime.error.clone()),
        log_path: managed_agent_runtime_log_path(app, key)
            .ok()
            .map(|path| path.display().to_string()),
    }
}

fn emit_status<R: tauri::Runtime>(app: &tauri::AppHandle<R>, status: &ManagedAgentRuntimeStatus) {
    let _ = app.emit(STATUS_EVENT, status);
}

// Observer-authored runtime-lifecycle capability command (§3.3a): extracted to
// keep this file under the 1000-line size ratchet.
#[path = "runtime_commands_observer.rs"]
mod observer;
pub use observer::put_managed_agent_runtime_lifecycle;
// Re-exported for the capability and unit test suites only; production code
// reaches the core through the `#[tauri::command]` wrapper inside `observer`.
#[cfg(test)]
use observer::observer_lifecycle_key;
#[cfg(test)]
pub(crate) use observer::put_managed_agent_runtime_lifecycle_for;

#[tauri::command]
pub fn list_managed_agent_runtimes(
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    // Capture scope at function entry — all reads in this function (personas,
    // global config, managed agents) must use the same scope so a concurrent
    // workspace switch cannot assemble mixed-scope inputs.
    let state = app.state::<AppState>();
    let scope = state
        .capture_active_scope()
        .ok_or_else(|| "list_managed_agent_runtimes: no active workspace scope".to_string())?;
    let definitions_dir = scope.definitions_dir.clone();

    // This command is polled whenever the members sidebar opens and refetched
    // on every status event — load the per-row status inputs once, outside
    // the locks, instead of hitting disk per row while holding them.
    // Both loads use the captured scope so they are consistent with the
    // load_managed_agents below.
    let personas = load_personas_at(&definitions_dir).unwrap_or_default();
    let global = load_global_agent_config_at(&definitions_dir).unwrap_or_default();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents_at(&definitions_dir)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let exited_keys: Vec<_> = runtimes
        .iter_mut()
        .filter_map(|(key, runtime)| match runtime.child.try_wait() {
            Ok(Some(_)) | Err(_) => Some(key.clone()),
            Ok(None) => None,
        })
        .collect();
    let records_changed = !exited_keys.is_empty();
    let mut statuses = Vec::new();
    for key in exited_keys {
        runtimes.remove(&key);
        super::remove_agent_runtime_receipt(&app, &key);
        if let Some(record) = records
            .iter_mut()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))
        {
            record.updated_at = crate::util::now_iso();
            record.last_stopped_at = Some(record.updated_at.clone());
            let status = status_for_with(
                &app,
                record,
                &key,
                None,
                None,
                StatusInputs {
                    personas: &personas,
                    global: &global,
                },
            );
            emit_status(&app, &status);
            statuses.push(status);
        }
    }
    statuses.extend(runtimes.iter().filter_map(|(key, runtime)| {
        let record = records
            .iter()
            .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))?;
        Some(status_for_with(
            &app,
            record,
            key,
            Some(runtime),
            None,
            StatusInputs {
                personas: &personas,
                global: &global,
            },
        ))
    }));
    drop(runtimes);
    // Records are only mutated above when a runtime exited — skip the store
    // rewrite on the common nothing-changed poll.
    if records_changed {
        save_managed_agents_at(&definitions_dir, &records)?;
    }
    Ok(statuses)
}

pub(crate) fn start_managed_agent_runtime_pair_lazy(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair_lazy_for(pubkey, relay_url, app)
}

/// Generic start-pair-lazy seam shared by the production adapter and tests.
///
/// Acquires `managed_agent_runtime_transition` as its first action — the same
/// lock that `stop`, `restart`, and `drain` operations hold, serialising all
/// runtime mutations. Tests that need a mock-runtime contender call this
/// function directly instead of the non-generic production adapter.
pub(crate) fn start_pair_lazy_for<R: tauri::Runtime>(
    pubkey: String,
    relay_url: String,
    app: tauri::AppHandle<R>,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair_lazy_for_with_hook(pubkey, relay_url, app, || {}, |_| {})
}

// Start-pair hook seams: extracted to stay under the file-size ratchet.
#[path = "runtime_commands_seams.rs"]
mod seams;
pub(crate) use seams::{start_pair_for_with_hook, start_pair_lazy_for_with_hook};

#[tauri::command]
pub fn start_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_managed_agent_runtime_pair_lazy(pubkey, relay_url, app)
}

fn start_pair(
    pubkey: String,
    relay_url: String,
    lazy: bool,
    expected_updated_at: Option<&str>,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair_for(pubkey, relay_url, lazy, expected_updated_at, app)
}

fn start_pair_for<R: tauri::Runtime>(
    pubkey: String,
    relay_url: String,
    lazy: bool,
    expected_updated_at: Option<&str>,
    app: tauri::AppHandle<R>,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair_for_with_hook(
        pubkey,
        relay_url,
        lazy,
        expected_updated_at,
        app,
        || {},
        |_| {},
    )
}

/// The spawn-and-register body of `start_pair`, called with the
/// `managed_agent_runtime_transition` and `managed_agents_store_lock` already
/// held by the caller.
///
/// Used by two callers:
/// 1. `start_pair` — normal start path; locks are acquired immediately above.
/// 2. `compensate_drain` — compensation path; locks are re-acquired by the
///    compensation primitive before calling this function, so compensation never
///    yields the epoch between journal entries and concurrent starts cannot
///    interleave.
///
/// The caller is responsible for saving `records` to disk after the call (or
/// for saving inside a batch loop if called for multiple entries).
fn start_pair_under_held_locks<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    state: &AppState,
    pubkey: String,
    relay_url: String,
    lazy: bool,
    expected_updated_at: Option<&str>,
    records: &mut [super::ManagedAgentRecord],
) -> Result<ManagedAgentRuntimeStatus, String> {
    let record = find_managed_agent_mut(records, &pubkey)?;
    if record.backend != BackendKind::Local {
        return Err("managed runtime pairs require a local agent".into());
    }
    if expected_updated_at.is_some_and(|expected| record.updated_at != expected) {
        return Err("managed agent changed while runtime reconciliation was in flight".into());
    }
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    if runtimes
        .get_mut(&key)
        .is_some_and(|runtime| runtime.child.try_wait().ok().flatten().is_none())
    {
        let status = status_for(app, record, &key, runtimes.get(&key), None);
        return Ok(status);
    }
    runtimes.remove(&key);
    terminate_untracked_pair_runtime(app, &key)?;

    let owner = state.current_pubkey().ok().map(|pk| pk.to_hex());
    let scope_id = state
        .capture_active_scope()
        .map(|scope| scope.scope_id.clone());
    let mut process = spawn_agent_child(app, record, &key.relay_url, lazy, owner.as_deref())?;
    let now = crate::util::now_iso();
    let receipt = ManagedAgentRuntimeReceipt {
        key: key.clone(),
        pid: process.child.id(),
        desktop_instance_id: current_instance_id(app),
        started_at: now.clone(),
    };
    if let Err(error) = write_agent_runtime_receipt(app, &receipt) {
        let _ = terminate_process(process.child.id());
        let _ = process.child.wait();
        return Err(error);
    }
    record.runtime_pid = None;
    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_error = None;
    runtimes.insert(
        key.clone(),
        ManagedAgentPairRuntime::starting(process, scope_id),
    );
    let status = status_for(app, record, &key, runtimes.get(&key), None);
    drop(runtimes);
    save_managed_agents(app, records)?;
    emit_status(app, &status);
    Ok(status)
}

#[tauri::command]
pub fn stop_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let state = app.state::<AppState>();
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let record = find_managed_agent_mut(&mut records, &pubkey)?;
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    if let Some(mut runtime) = runtimes.remove(&key) {
        let stop_result = if process_is_running(runtime.child.id()) {
            terminate_process(runtime.child.id())
        } else {
            Ok(())
        }
        .and_then(|()| runtime.child.wait().map_err(|e| e.to_string()));
        match stop_result {
            Ok(status) => {
                record.last_exit_code = status.code();
                let _ = append_log_marker(&runtime.log_path, "=== stopped pair runtime ===");
            }
            Err(error) => {
                // Keep failed teardown visible/manageable instead of
                // orphaning it: the child stays tracked and the receipt
                // stays on disk until a stop actually succeeds.
                runtimes.insert(key, runtime);
                return Err(error);
            }
        }
    } else {
        // No runtime is tracked at this key, but a valid prior-session
        // receipt may still point at a live child (e.g. the crash-recovery
        // window for a non-auto-start agent). Terminate that orphan before
        // erasing its receipt — otherwise this "stop" leaves the harness
        // running yet deletes the one artifact sweeps and
        // terminate_untracked_pair_runtime use to find it, and a follow-up
        // start would spawn a duplicate harness for the same pair. On
        // failure the receipt stays on disk (terminate_untracked_pair_runtime
        // only removes it after the child exits), mirroring the tracked
        // path's keep-until-success invariant.
        terminate_untracked_pair_runtime(&app, &key)?;
    }
    super::remove_agent_runtime_receipt(&app, &key);
    record.runtime_pid = None;
    record.updated_at = crate::util::now_iso();
    record.last_stopped_at = Some(record.updated_at.clone());
    let status = status_for(&app, record, &key, None, None);
    drop(runtimes);
    save_managed_agents(&app, &records)?;
    emit_status(&app, &status);
    Ok(status)
}

#[tauri::command]
pub fn restart_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    stop_managed_agent_runtime(pubkey.clone(), relay_url.clone(), app.clone())?;
    start_pair(pubkey, relay_url, true, None, app)
}

/// Probe whether this agent can operate on `requested_relay_url`.
///
/// Runs a bounded authenticated query with the agent's own keys (NIP-42 +
/// NIP-OA auth tag). Auth success is the spawn-eligibility signal: NIP-29
/// membership (kind 39002) cannot exist before the agent's harness first
/// connects to a relay, so gating on membership *presence* could never
/// bootstrap a pair on a newly configured community — it only rediscovered
/// pairs that had already run. A rejected or timed-out probe surfaces as a
/// Failed status row instead of a silent skip.
async fn probe_agent_relay_access(
    state: &AppState,
    record: super::ManagedAgentRecord,
    requested_relay_url: String,
) -> Result<(super::ManagedAgentRecord, ManagedAgentRuntimeKey, String), String> {
    let key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), &requested_relay_url)?;
    let keys = nostr::Keys::parse(record.private_key_nsec.trim())
        .map_err(|error| format!("invalid managed-agent key: {error}"))?;
    let api_base = crate::relay::relay_http_base_url(&key.relay_url);
    // Managed-agent egress construction site (P29-C1 closed-world sink). Admit
    // the interim keyed-egress lease before the probe query.
    let lease = crate::owner_identity_egress::EgressLease::ManagedAgentKeyed(
        crate::owner_identity_egress::admit_managed_agent_egress().await?,
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::relay::query_relay_at_with_keys(
            state,
            &api_base,
            &[serde_json::json!({"kinds": [39002], "#p": [record.pubkey]})],
            &keys,
            record.auth_tag.as_deref(),
            &lease,
        ),
    )
    .await
    .map_err(|_| "relay access probe timed out".to_string())??;
    Ok((record, key, requested_relay_url))
}

/// Build the `Failed` status row for a probe failure whose requested relay URL
/// cannot even form a pair key (so there is no canonical `relay_url` to key on).
/// The raw requested URL stands in for both the identity and the requested
/// field so the batch still degrades this one community to a visible row
/// instead of aborting every other community's row.
fn unkeyable_failed_status(
    record: &super::ManagedAgentRecord,
    requested: String,
    error: String,
    personas: &[super::AgentDefinition],
    global: &super::GlobalAgentConfig,
) -> ManagedAgentRuntimeStatus {
    let command = record_agent_command(record, personas);
    let metadata = super::known_acp_runtime(&command);
    let effective = resolve_effective_agent_env(record, personas, metadata, global);
    ManagedAgentRuntimeStatus {
        pubkey: record.pubkey.clone(),
        relay_url: requested.clone(),
        requested_relay_url: Some(requested),
        local_setup: matches!(agent_readiness(&effective), AgentReadiness::Ready),
        lifecycle: ManagedAgentRuntimeLifecycle::Failed,
        pid: None,
        error: Some(error),
        log_path: None,
    }
}

/// Spawn a lazy harness pair for every auto-start local agent in the active
/// workspace scope.
///
/// The target relay is derived from the captured active scope — the
/// `communities` fan-out parameter has been removed. Under the active-scope-only
/// runtime policy, reconcile targets exactly one relay: the relay the current
/// workspace is bound to. Cross-scope fan-out is no longer representable at the
/// API level.
///
/// Eligibility is gated on `start_on_app_launch`: auto-start is the proactive
/// fan-out policy — agents not set to auto-start are left alone until something
/// explicitly asks for them.
#[tauri::command]
pub async fn reconcile_managed_agent_runtimes(
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    use futures_util::{stream, StreamExt};

    let state = app.state::<AppState>();
    let scope = state
        .capture_active_scope()
        .ok_or_else(|| "reconcile_managed_agent_runtimes: no active workspace scope".to_string())?;
    let relay_url = scope.relay_url.clone();

    let records = load_managed_agents(&app)?;
    let mut jobs = Vec::new();
    for record in records
        .iter()
        .filter(|record| record.start_on_app_launch && record.backend == BackendKind::Local)
    {
        jobs.push((record.clone(), relay_url.clone()));
    }
    let probes: Vec<_> = stream::iter(jobs)
        .map(|(record, requested)| {
            let state = app.state::<AppState>();
            async move {
                let fallback_record = record.clone();
                let fallback_requested = requested.clone();
                probe_agent_relay_access(&state, record, requested)
                    .await
                    .map_err(|error| (fallback_record, fallback_requested, error))
            }
        })
        .buffer_unordered(6)
        .collect()
        .await;

    // start_pair does blocking work (std mutexes, process spawn, receipt
    // writes, and up-to-2s exit polling in terminate_untracked_pair_runtime),
    // so run the post-probe start loop off the async workers, matching the
    // restart flows.
    tokio::task::spawn_blocking(move || {
        let personas = load_personas(&app).unwrap_or_default();
        let global = load_global_agent_config(&app).unwrap_or_default();
        let mut rows = Vec::new();
        for probe in probes {
            match probe {
                Ok((record, key, requested)) => {
                    match start_pair(
                        record.pubkey.clone(),
                        key.relay_url.clone(),
                        true,
                        Some(&record.updated_at),
                        app.clone(),
                    ) {
                        Ok(mut status) => {
                            status.requested_relay_url = Some(requested);
                            rows.push(status);
                        }
                        Err(error) => {
                            let mut status = status_for_with(
                                &app,
                                &record,
                                &key,
                                None,
                                Some(requested),
                                StatusInputs {
                                    personas: &personas,
                                    global: &global,
                                },
                            );
                            status.lifecycle = ManagedAgentRuntimeLifecycle::Failed;
                            status.error = Some(error);
                            rows.push(status);
                        }
                    }
                }
                Err((record, requested, error)) => {
                    // Per-community degradation: a relay URL that cannot even
                    // form a pair key gets a Failed row (with the raw
                    // requested URL) like any other probe failure, instead of
                    // aborting every other community's row.
                    let status =
                        match ManagedAgentRuntimeKey::new(record.pubkey.clone(), &requested) {
                            Ok(key) => {
                                let mut status = status_for_with(
                                    &app,
                                    &record,
                                    &key,
                                    None,
                                    Some(requested),
                                    StatusInputs {
                                        personas: &personas,
                                        global: &global,
                                    },
                                );
                                status.lifecycle = ManagedAgentRuntimeLifecycle::Failed;
                                status.error = Some(error);
                                status
                            }
                            Err(_) => unkeyable_failed_status(
                                &record, requested, error, &personas, &global,
                            ),
                        };
                    rows.push(status);
                }
            }
        }
        rows
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// A single entry in the drain journal: enough to restart the process if
/// compensation is needed after a partial drain failure.
#[derive(Debug, Clone)]
pub(crate) struct DrainJournalEntry {
    pub key: ManagedAgentRuntimeKey,
    /// Whether the agent would auto-start on app launch (used to determine
    /// whether compensation should restart it as auto-start or lazy).
    pub start_on_app_launch: bool,
}

/// Execute a drain journal against the runtime map.
///
/// Pure inner function: takes the map directly so callers (including tests)
/// can drive it without an `AppHandle`. The `cleanup_fn` is called for each
/// successfully stopped entry to remove its receipt and clear the session
/// cache; the closure is a no-op in tests.
///
/// Returns `(stopped, remaining, first_stop_error)`:
/// - `stopped` — entries successfully killed (compensation restores these).
/// - `remaining` — entries NOT attempted due to an earlier stop failure.
/// - error — the first stop failure, if any; `None` on full success.
pub(crate) fn execute_drain_journal(
    journal: &[DrainJournalEntry],
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    cleanup_fn: impl FnMut(&ManagedAgentRuntimeKey),
) -> (
    Vec<DrainJournalEntry>,
    Vec<DrainJournalEntry>,
    Option<String>,
) {
    drain_journal_with_stop(journal, runtimes, cleanup_fn, |key, runtime| {
        let kill_result = if super::process_is_running(runtime.child.id()) {
            super::terminate_process(runtime.child.id())
        } else {
            Ok(())
        }
        .and_then(|()| runtime.child.wait().map_err(|e| e.to_string()));
        let _ = key; // key available for logging; unused in production path
        kill_result.map(|_| ())
    })
}

/// Inner implementation of drain journal execution, parameterized by a stop
/// function for testability.
///
/// The `stop_fn` receives the journal key and a mutable reference to the
/// runtime being stopped. It returns `Ok(())` on success or `Err(String)` on
/// failure. In production it sends SIGTERM/SIGKILL + wait; in tests it can
/// inject controlled failures per-key.
fn drain_journal_with_stop(
    journal: &[DrainJournalEntry],
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    mut cleanup_fn: impl FnMut(&ManagedAgentRuntimeKey),
    mut stop_fn: impl FnMut(&ManagedAgentRuntimeKey, &mut ManagedAgentPairRuntime) -> Result<(), String>,
) -> (
    Vec<DrainJournalEntry>,
    Vec<DrainJournalEntry>,
    Option<String>,
) {
    let mut stopped: Vec<DrainJournalEntry> = Vec::new();
    let mut first_error: Option<String> = None;

    for (idx, entry) in journal.iter().enumerate() {
        let key = &entry.key;
        let stop_result = if let Some(mut runtime) = runtimes.remove(key) {
            match stop_fn(key, &mut runtime) {
                Ok(()) => {
                    cleanup_fn(key);
                    Ok(())
                }
                Err(e) => {
                    // Put it back so the map is consistent.
                    runtimes.insert(key.clone(), runtime);
                    Err(e)
                }
            }
        } else {
            // Nothing live at this key — treat as already stopped.
            Ok(())
        };

        match stop_result {
            Ok(()) => stopped.push(entry.clone()),
            Err(e) => {
                let msg = format!("failed to stop agent {}@{}: {e}", key.pubkey, key.relay_url);
                first_error.get_or_insert(msg);
                // Return the un-attempted tail (idx+1 onward) as remaining.
                return (stopped, journal[idx + 1..].to_vec(), first_error);
            }
        }
    }

    (stopped, vec![], first_error)
}

/// Test-only variant of `execute_drain_journal` with an injectable stop
/// function so partial-failure scenarios can be exercised without relying on
/// OS-specific process-wait behavior.
///
/// The `stop_fn` receives the `ManagedAgentRuntimeKey` being stopped and
/// returns `Ok(())` for simulated success or `Err(String)` for simulated
/// failure. Entries absent from the runtime map are still treated as stopped
/// (matching the production path).
#[cfg(test)]
pub(crate) fn execute_drain_journal_with_stop_fn(
    journal: &[DrainJournalEntry],
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    cleanup_fn: impl FnMut(&ManagedAgentRuntimeKey),
    mut stop_fn: impl FnMut(&ManagedAgentRuntimeKey) -> Result<(), String>,
) -> (
    Vec<DrainJournalEntry>,
    Vec<DrainJournalEntry>,
    Option<String>,
) {
    drain_journal_with_stop(journal, runtimes, cleanup_fn, |key, _runtime| stop_fn(key))
}

/// Drain all live runtimes from the runtime map and return a drain journal
/// (keys + restart recipes) for use by compensation.
///
/// This runs under the `managed_agent_runtime_transition` lock (Layer 2
/// synchronous epoch — no `.await`). Callers are responsible for acquiring
/// that lock before calling this function.
///
/// Returns `(stopped, remaining, first_stop_error)`. `stopped` contains the
/// entries that were successfully killed (compensation restores these).
/// `remaining` contains entries that were NOT attempted (due to early-exit on
/// first failure). On success `remaining` is empty.
pub(crate) fn drain_scope_runtimes<R: tauri::Runtime>(
    app: &AppHandle<R>,
    state: &AppState,
) -> (
    Vec<DrainJournalEntry>,
    Vec<DrainJournalEntry>,
    Option<String>,
) {
    // Snapshot the journal from the live runtime map before any stops.
    let journal: Vec<DrainJournalEntry> = {
        let runtimes = match state.managed_agent_processes.lock() {
            Ok(r) => r,
            Err(e) => {
                return (
                    vec![],
                    vec![],
                    Some(format!("runtime map lock poisoned: {e}")),
                )
            }
        };
        runtimes
            .keys()
            .map(|key| {
                // Look up start_on_app_launch from the current store; if we
                // can't read it, assume true (safer for compensation — we'd
                // rather restart too many than too few).
                let start_on_app_launch = load_managed_agents(app)
                    .ok()
                    .and_then(|records| {
                        records
                            .iter()
                            .find(|r| r.pubkey == key.pubkey)
                            .map(|r| r.start_on_app_launch)
                    })
                    .unwrap_or(true);
                DrainJournalEntry {
                    key: key.clone(),
                    start_on_app_launch,
                }
            })
            .collect()
    };

    let mut runtimes = match state.managed_agent_processes.lock() {
        Ok(r) => r,
        Err(e) => {
            return (
                vec![],
                journal,
                Some(format!("runtime map lock poisoned during drain: {e}")),
            )
        }
    };

    execute_drain_journal(&journal, &mut runtimes, |key| {
        super::remove_agent_runtime_receipt(app, key);
    })
}

/// Compensate a partial drain by restarting the entries that were successfully
/// stopped before the failure.
///
/// `stopped` is the slice of journal entries that were actually stopped (i.e.,
/// the prefix of the journal up to the first failure). We restart them so the
/// old workspace is as intact as possible.
///
/// `captured_scope` is the workspace scope that was active when the drain began.
///
/// `_rt_transition_held` is the caller's already-held
/// `managed_agent_runtime_transition` guard. The caller must NOT drop it before
/// calling this function — passing ownership here ensures the transition lock
/// is held continuously from drain through all journal restarts, closing the
/// drop-then-reacquire interleave window that a concurrent start could exploit.
///
/// Returns a degradation message describing what could not be restarted.
///
/// The journal-restore loop is implemented in [`compensate_drain_for`] with an
/// injected start function, allowing the iteration contract to be unit-tested
/// without an `AppHandle`.
// ────────────────────────────────────────────────────────────────────────────
/// Lock-free testable core of the journal-restore loop.
///
/// **Preconditions (enforced by the [`compensate_drain`] adapter before calling):**
/// - `managed_agent_runtime_transition` is held by the caller (passed by value
///   to `compensate_drain`).
/// - `managed_agents_store_lock` is acquired by the adapter BEFORE this call.
/// - `records` is loaded by the adapter AFTER acquiring the store lock.
///
/// `compensate_drain` passes a `start_fn` that invokes
/// [`start_pair_under_held_locks`]; tests inject a closure that records calls
/// and returns synthetic results without spawning processes or touching disk.
///
/// The mechanism serializing writers: the adapter holds `managed_agents_store_lock`
/// continuously across validate→load→restore→save, so any writer that takes only
/// the store lock is serialized here — not on the transition guard.
///
/// Returns a degradation message when one or more restarts fail, `None` on full
/// success.
pub(crate) fn compensate_drain_for<F>(
    stopped: &[DrainJournalEntry],
    records: &mut [super::ManagedAgentRecord],
    mut start_fn: F,
) -> Option<String>
where
    F: FnMut(&DrainJournalEntry, &mut [super::ManagedAgentRecord]) -> Result<(), String>,
{
    debug_assert!(
        !stopped.is_empty(),
        "compensate_drain_for called with empty stopped list"
    );

    let mut failed_restarts = Vec::new();
    for entry in stopped {
        if let Err(e) = start_fn(entry, records) {
            failed_restarts.push(format!("{}@{}: {e}", entry.key.pubkey, entry.key.relay_url));
        }
    }

    if failed_restarts.is_empty() {
        None
    } else {
        Some(format!(
            "workspace drain compensation failed for: {}",
            failed_restarts.join(", ")
        ))
    }
}

/// Production adapter for [`compensate_drain_for`].
///
/// Delegates to [`compensate_drain_with_hook`] with no-op hooks.
pub(crate) fn compensate_drain<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    stopped: &[DrainJournalEntry],
    captured_scope: &crate::managed_agents::scope::WorkspaceAgentScope,
    _rt_transition_held: std::sync::MutexGuard<'_, ()>,
) -> Option<String> {
    compensate_drain_with_hook(
        app,
        stopped,
        captured_scope,
        _rt_transition_held,
        |_| {},
        |_, _| {},
    )
}

/// Inner implementation of [`compensate_drain`] with injectable hooks.
///
/// Lock order: transition guard held by caller → acquire store lock → validate generation
/// → load records → `on_records_loaded` → delegate to `compensate_drain_for` →
/// `on_after_restore(&mut records, &_store)` (borrows the actual store guard; production
/// passes `|_, _| {}`). Store lock held across the entire sequence. With no-op callbacks,
/// production behavior is byte-for-byte equivalent to the pre-hook path.
pub(crate) fn compensate_drain_with_hook<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    stopped: &[DrainJournalEntry],
    captured_scope: &crate::managed_agents::scope::WorkspaceAgentScope,
    _rt_transition_held: std::sync::MutexGuard<'_, ()>,
    on_records_loaded: impl FnOnce(&mut Vec<super::ManagedAgentRecord>),
    on_after_restore: impl FnOnce(&mut Vec<super::ManagedAgentRecord>, &std::sync::MutexGuard<'_, ()>),
) -> Option<String> {
    if stopped.is_empty() {
        drop(_rt_transition_held);
        return None;
    }

    let state = app.state::<AppState>();

    let _store = match state.managed_agents_store_lock.lock() {
        Ok(g) => g,
        Err(e) => {
            return Some(format!(
                "compensation failed: could not acquire store lock: {e}"
            ));
        }
    };

    // 3. Validate the captured scope under the store lock.
    if let Err(stale_msg) = crate::managed_agents::scope::validate_scope_generation(captured_scope)
    {
        return Some(format!(
            "compensation skipped: {stale_msg}; new scope will restore its own agents"
        ));
    }

    // 4. Load records under the held store lock.
    let mut records = match load_managed_agents_at(&captured_scope.definitions_dir) {
        Ok(r) => r,
        Err(e) => {
            return Some(format!(
                "compensation failed: could not load agent records: {e}"
            ));
        }
    };

    // 5. Pre-restore hook — no-op in production.
    //    Tests inject a sentinel mutation here and synchronise with a writer thread.
    on_records_loaded(&mut records);

    // 6. Delegate — store guard held through every restore by start_pair_under_held_locks.
    let result = compensate_drain_for(stopped, &mut records, |entry, recs| {
        start_pair_under_held_locks(
            app,
            &state,
            entry.key.pubkey.clone(),
            entry.key.relay_url.clone(),
            entry.start_on_app_launch,
            None,
            recs,
        )
        .map(|_| ())
    });

    // 7. Post-restore hook — no-op in production. Tests add `COMP_SENTINEL` and save
    //    while borrowing the actual store guard, proving the lock is held through restore
    //    and this callback.
    on_after_restore(&mut records, &_store);

    result
}

#[cfg(test)]
#[path = "runtime_commands_tests.rs"]
mod tests;
