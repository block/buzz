use std::sync::atomic::Ordering;

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

const STATUS_EVENT: &str = "managed-agent-runtime-status";

fn status_for(
    app: &AppHandle,
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

fn status_for_with(
    app: &AppHandle,
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

fn emit_status(app: &AppHandle, status: &ManagedAgentRuntimeStatus) {
    let _ = app.emit(STATUS_EVENT, status);
}

fn observer_lifecycle_key(
    outer_pubkey: &str,
    payload: &super::ManagedAgentRuntimeLifecycleObserverPayload,
) -> Result<ManagedAgentRuntimeKey, String> {
    if !outer_pubkey.eq_ignore_ascii_case(&payload.pubkey) {
        return Err("observer signer does not match lifecycle payload pubkey".into());
    }
    if matches!(
        payload.lifecycle,
        ManagedAgentRuntimeLifecycle::Starting | ManagedAgentRuntimeLifecycle::Stopped
    ) {
        return Err("observer cannot author starting or stopped lifecycle".into());
    }
    if payload.lifecycle == ManagedAgentRuntimeLifecycle::Failed && payload.error.is_none() {
        return Err("failed lifecycle requires an error".into());
    }
    if payload.lifecycle != ManagedAgentRuntimeLifecycle::Failed && payload.error.is_some() {
        return Err("lifecycle error is only valid for failed".into());
    }
    ManagedAgentRuntimeKey::new(payload.pubkey.clone(), &payload.relay_url)
}

#[tauri::command]
pub fn put_managed_agent_runtime_lifecycle(
    outer_pubkey: String,
    payload: super::ManagedAgentRuntimeLifecycleObserverPayload,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let key = observer_lifecycle_key(&outer_pubkey, &payload)?;
    let state = app.state::<AppState>();
    let records = load_managed_agents(&app)?;
    let record = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(&key.pubkey))
        .ok_or_else(|| format!("agent {} not found", key.pubkey))?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    let runtime = runtimes
        .get_mut(&key)
        .ok_or_else(|| "lifecycle frame does not match a tracked runtime pair".to_string())?;
    if runtime.start_nonce != payload.start_nonce {
        return Err("lifecycle frame does not match the current harness generation".into());
    }
    if runtime
        .child
        .try_wait()
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("lifecycle frame arrived after process exit".into());
    }
    runtime.lifecycle = payload.lifecycle;
    runtime.error = payload.error;
    let status = status_for(&app, record, &key, Some(runtime), None);
    emit_status(&app, &status);
    Ok(status)
}

// Keep disk, process, and mutex work off the main thread so opening members cannot stall the UI.
#[tauri::command]
pub async fn list_managed_agent_runtimes(
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    tokio::task::spawn_blocking(move || {
        // This command is polled whenever the members sidebar opens and refetched
        // on every status event — load the per-row status inputs once, outside
        // the locks, instead of hitting disk per row while holding them.
        let personas = load_personas(&app).unwrap_or_default();
        let global = load_global_agent_config(&app).unwrap_or_default();
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
            state.clear_agent_session_cache(&key);
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
            save_managed_agents(&app, &records)?;
        }
        Ok(statuses)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

pub(crate) fn start_managed_agent_runtime_pair_lazy(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_pair(pubkey, relay_url, true, None, app)
}

#[tauri::command]
pub fn start_managed_agent_runtime(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    start_managed_agent_runtime_pair_lazy(pubkey, relay_url, app)
}

/// Fold the relay-resolved record into the exact bytes a pair spawn executes.
///
/// Pure over `(resolved, personas)` so the final-use boundary is testable
/// without a live `AppHandle` (same seam strategy as
/// `finalize_restore_candidate` in `restore.rs`):
/// - a RESOLVED non-local backend is refused by name — pair runtimes are a
///   local-process concept, and a relay head that migrated the agent to a
///   provider backend must not spawn a local child from leftover disk bytes;
/// - the linked persona snapshot is re-applied LAST so the definition quad
///   (prompt/model/provider/runtime) keeps its established precedence over
///   both disk and relay bytes;
/// - an orphaned instance (persona_id with no live persona) passes through:
///   `spawn_agent_child` owns that refusal via `resolve_effective_config`.
fn resolve_pair_spawn_record(
    mut resolved: super::ManagedAgentRecord,
    personas: &[super::AgentDefinition],
) -> Result<super::ManagedAgentRecord, String> {
    if resolved.backend != BackendKind::Local {
        return Err("managed runtime pairs require a local agent".into());
    }
    if let Some(persona_id) = resolved.persona_id.clone() {
        if let Some(persona) = personas.iter().find(|persona| persona.id == persona_id) {
            super::persona_events::apply_persona_snapshot(&mut resolved, persona);
            resolved.updated_at = crate::util::now_iso();
        }
    }
    Ok(resolved)
}

fn start_pair(
    pubkey: String,
    relay_url: String,
    lazy: bool,
    expected_updated_at: Option<&str>,
    app: AppHandle,
) -> Result<ManagedAgentRuntimeStatus, String> {
    let state = app.state::<AppState>();
    // A relay-only record (hydrated overlay head with no disk row) needs a
    // durable device-local lifecycle anchor before the disk lookup below —
    // exactly like the interactive start path. Without this, pair
    // Start/Restart for a relay-only card fails "agent not found" instead of
    // materializing it; a relay-only PROVIDER card is refused here by name.
    // Takes and releases its own locks, so it must run before ours.
    super::private_config_overlay::materialize_relay_only_agent(&app, &state, &pubkey)?;
    let _transition = state
        .managed_agent_runtime_transition
        .lock()
        .map_err(|e| e.to_string())?;
    if state.shutdown_started.load(Ordering::Acquire) {
        return Err("desktop shutdown has started".into());
    }
    let _store = state
        .managed_agents_store_lock
        .lock()
        .map_err(|e| e.to_string())?;
    let mut records = load_managed_agents(&app)?;
    let record = find_managed_agent_mut(&mut records, &pubkey)?;
    if expected_updated_at.is_some_and(|expected| record.updated_at != expected) {
        return Err("managed agent changed while runtime reconciliation was in flight".into());
    }
    // Final-use boundary: the spawn below executes the relay-primary resolve
    // of the disk row (persona snapshot re-applied last), never raw disk
    // bytes — otherwise a follower device showing relay config B would
    // execute stale disk config A. Lifecycle mutations further down still
    // land on the DISK `record`; relay-owned configuration is never written
    // back to the device-local store.
    let spawn_record = resolve_pair_spawn_record(
        super::private_config_overlay::resolved_local_record(&state, record)?,
        &load_personas(&app).unwrap_or_default(),
    )?;
    let key = ManagedAgentRuntimeKey::new(pubkey, &relay_url)?;
    let mut runtimes = state
        .managed_agent_processes
        .lock()
        .map_err(|e| e.to_string())?;
    if runtimes
        .get_mut(&key)
        .is_some_and(|runtime| runtime.child.try_wait().ok().flatten().is_none())
    {
        let status = status_for(&app, &spawn_record, &key, runtimes.get(&key), None);
        return Ok(status);
    }
    runtimes.remove(&key);
    terminate_untracked_pair_runtime(&app, &key)?;

    let owner = state
        .keys
        .lock()
        .ok()
        .map(|keys| keys.public_key().to_hex());
    let mut process = spawn_agent_child(
        &app,
        &spawn_record,
        &key.relay_url,
        lazy,
        owner.as_deref(),
        None,
    )?;
    let now = crate::util::now_iso();
    let receipt = ManagedAgentRuntimeReceipt {
        key: key.clone(),
        pid: process.child.id(),
        desktop_instance_id: current_instance_id(&app),
        started_at: now.clone(),
    };
    if let Err(error) = write_agent_runtime_receipt(&app, &receipt) {
        let _ = terminate_process(process.child.id());
        let _ = process.child.wait();
        return Err(error);
    }
    record.runtime_pid = None;
    record.updated_at = now.clone();
    record.last_started_at = Some(now);
    record.last_stopped_at = None;
    record.last_error = None;
    runtimes.insert(key.clone(), ManagedAgentPairRuntime::starting(process));
    let status = status_for(&app, &spawn_record, &key, runtimes.get(&key), None);
    drop(runtimes);
    save_managed_agents(&app, &records)?;
    emit_status(&app, &status);
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
    state.clear_agent_session_cache(&key);
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
    // Materialize a relay-only record before the stop half, which fails
    // "agent not found" on a pubkey with no disk row. `start_pair` would
    // materialize anyway, but only after stop has already failed the restart.
    {
        let state = app.state::<AppState>();
        super::private_config_overlay::materialize_relay_only_agent(&app, &state, &pubkey)?;
    }
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
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        crate::relay::query_relay_at_with_keys(
            state,
            &api_base,
            &[serde_json::json!({"kinds": [39002], "#p": [record.pubkey]})],
            &keys,
            record.auth_tag.as_deref(),
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

/// Spawn a lazy harness pair for every eligible (agent, community) pair.
///
/// Eligibility is deliberately gated on `start_on_app_launch`: auto-start is
/// the *proactive fan-out* policy — "keep this agent warm in every community" —
/// not a correctness prerequisite. A manual-start agent still works on demand
/// everywhere: attaching it to a channel ensures its pair, an @mention wakes a
/// pair, the members sidebar and Settings controls start pairs, and restore
/// preserves running pairs across relaunch. Fanning out warm-socket pairs for
/// agents the user chose *not* to auto-start would contradict that choice, so
/// reconcile leaves them alone until something explicitly asks for them.
#[tauri::command]
pub async fn reconcile_managed_agent_runtimes(
    communities: Vec<super::ManagedAgentCommunityTarget>,
    app: AppHandle,
) -> Result<Vec<ManagedAgentRuntimeStatus>, String> {
    use futures_util::{stream, StreamExt};

    let records = load_managed_agents(&app)?;
    // Fan-out candidates resolve through the relay-primary overlay BEFORE the
    // probe: the probe authenticates as the agent (nsec + auth tag), and a
    // follower device may hold a rotated identity only in the overlay.
    // `start_on_app_launch` is device-local (never patched), so the
    // auto-start choice itself still reads disk; the backend gate reads the
    // RESOLVED record so a relay head that migrated an agent off the local
    // backend is skipped instead of spawned from leftover disk bytes. The
    // raw disk `updated_at` is carried alongside as the in-flight guard
    // `start_pair` re-checks against the disk row it reloads.
    let candidates = {
        let state = app.state::<AppState>();
        let mut candidates = Vec::new();
        for record in records.iter().filter(|record| record.start_on_app_launch) {
            let resolved = super::private_config_overlay::resolved_local_record(&state, record)?;
            if resolved.backend != BackendKind::Local {
                continue;
            }
            candidates.push((resolved, record.updated_at.clone()));
        }
        candidates
    };
    let mut jobs = Vec::new();
    for community in communities {
        for (record, disk_updated_at) in &candidates
        // The legacy per-record relay pin is deliberately ignored here — see
        // `effective_agent_relay_url`. Every local auto-start agent fans out
        // to every configured community.
        {
            jobs.push((
                record.clone(),
                disk_updated_at.clone(),
                community.relay_url.clone(),
            ));
        }
    }
    let probes: Vec<_> = stream::iter(jobs)
        .map(|(record, disk_updated_at, requested)| {
            let state = app.state::<AppState>();
            async move {
                let fallback_record = record.clone();
                let fallback_requested = requested.clone();
                probe_agent_relay_access(&state, record, requested)
                    .await
                    .map(|(record, key, requested)| (record, key, requested, disk_updated_at))
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
                Ok((record, key, requested, disk_updated_at)) => {
                    match start_pair(
                        record.pubkey.clone(),
                        key.relay_url.clone(),
                        true,
                        Some(&disk_updated_at),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_managed_agent_runtimes_returns_a_future() {
        fn assert_async_command<F, Fut>(_command: F)
        where
            F: Fn(AppHandle) -> Fut,
            Fut: std::future::Future<Output = Result<Vec<ManagedAgentRuntimeStatus>, String>>,
        {
        }

        assert_async_command(list_managed_agent_runtimes);
    }

    fn payload(
        relay_url: &str,
        lifecycle: ManagedAgentRuntimeLifecycle,
        error: Option<&str>,
    ) -> super::super::ManagedAgentRuntimeLifecycleObserverPayload {
        super::super::ManagedAgentRuntimeLifecycleObserverPayload {
            pubkey: "aa".repeat(32),
            relay_url: relay_url.into(),
            start_nonce: "test-generation".into(),
            lifecycle,
            error: error.map(str::to_owned),
        }
    }

    fn record_with_relay(relay_url: &str) -> super::super::ManagedAgentRecord {
        serde_json::from_str(&format!(
            r#"{{
                "pubkey": "{}",
                "name": "pin-test",
                "relay_url": "{relay_url}",
                "acp_command": "buzz-acp",
                "agent_command": "goose",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": "",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }}"#,
            "aa".repeat(32)
        ))
        .unwrap()
    }

    #[test]
    fn legacy_relay_pin_is_ignored_for_fan_out() {
        // Zero-touch cutover (#2122): a record carrying a creation-era
        // `relay_url` pin must fan out exactly like an unpinned one — the
        // stored field is parsed but never consulted. See
        // `effective_agent_relay_url`.
        let unpinned = record_with_relay("");
        let pinned = record_with_relay("wss://one.example");
        for record in [&unpinned, &pinned] {
            assert_eq!(
                crate::relay::effective_agent_relay_url(&record.relay_url, "wss://two.example"),
                "wss://two.example"
            );
        }
    }

    #[test]
    fn unkeyable_relay_degrades_to_failed_row() {
        // A requested URL that cannot form a pair key must still yield a
        // Failed row keyed by the raw requested string, so one bad community
        // never aborts the rest of the reconcile batch.
        let record = record_with_relay("");
        let status = unkeyable_failed_status(
            &record,
            "not a url".to_string(),
            "relay access probe timed out".to_string(),
            &[],
            &super::super::GlobalAgentConfig::default(),
        );
        assert!(matches!(
            status.lifecycle,
            ManagedAgentRuntimeLifecycle::Failed
        ));
        assert_eq!(status.relay_url, "not a url");
        assert_eq!(status.requested_relay_url.as_deref(), Some("not a url"));
        assert_eq!(status.pubkey, record.pubkey);
        assert_eq!(
            status.error.as_deref(),
            Some("relay access probe timed out")
        );
        assert!(status.pid.is_none());
    }

    #[test]
    fn runtime_key_rejects_non_hex_pubkeys() {
        assert!(ManagedAgentRuntimeKey::new("../not-a-key", "wss://relay.example").is_err());
        assert!(ManagedAgentRuntimeKey::new("gg".repeat(32), "wss://relay.example").is_err());
    }

    #[test]
    fn runtime_key_canonicalizes_hex_pubkeys() {
        let key = ManagedAgentRuntimeKey::new("AA".repeat(32), "wss://relay.example").unwrap();
        assert_eq!(key.pubkey, "aa".repeat(32));
    }

    #[test]
    fn observer_lifecycle_key_preserves_exact_canonical_pair() {
        let first = payload(
            "WSS://Relay.Example:443/",
            ManagedAgentRuntimeLifecycle::Ready,
            None,
        );
        let key = observer_lifecycle_key(&first.pubkey, &first).unwrap();
        assert_eq!(key.pubkey, first.pubkey);
        assert_eq!(key.relay_url, "wss://relay.example");

        let other = payload(
            "wss://other.example",
            ManagedAgentRuntimeLifecycle::Ready,
            None,
        );
        assert_ne!(key, observer_lifecycle_key(&other.pubkey, &other).unwrap());
    }

    #[test]
    fn observer_lifecycle_rejects_cross_agent_and_desktop_states() {
        let ready = payload(
            "wss://relay.example",
            ManagedAgentRuntimeLifecycle::Ready,
            None,
        );
        assert!(observer_lifecycle_key(&"bb".repeat(32), &ready).is_err());

        let stopped = payload(
            "wss://relay.example",
            ManagedAgentRuntimeLifecycle::Stopped,
            None,
        );
        assert!(observer_lifecycle_key(&stopped.pubkey, &stopped).is_err());
    }

    #[test]
    fn observer_lifecycle_enforces_failed_error_contract() {
        let failed = payload(
            "wss://relay.example",
            ManagedAgentRuntimeLifecycle::Failed,
            None,
        );
        assert!(observer_lifecycle_key(&failed.pubkey, &failed).is_err());

        let ready_with_error = payload(
            "wss://relay.example",
            ManagedAgentRuntimeLifecycle::Ready,
            Some("unexpected"),
        );
        assert!(observer_lifecycle_key(&ready_with_error.pubkey, &ready_with_error).is_err());
    }
}

// ── Pair-spawn fold: relay-overlay resolve at the final-use boundary ─────────
//
// The production wiring (`start_pair` resolving the disk row through
// `resolved_local_record` before `spawn_agent_child`) needs a live
// `AppHandle`, so its presence is pinned by `write_site_resolve_guard` in
// `private_config_overlay.rs`. These tests prove the fold itself at the same
// overlay + finalize seam the production path composes — the pair-start
// variant of `restore_fold_tests`.
#[cfg(test)]
mod pair_spawn_resolve_tests {
    use super::resolve_pair_spawn_record;
    use crate::managed_agents::private_config_overlay::{test_relay_payload, PrivateConfigOverlay};
    use crate::managed_agents::{AgentDefinition, BackendKind, ManagedAgentRecord};
    use std::collections::BTreeMap;

    /// A stale disk row as `start_pair` reloads it under its store lock.
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

    /// Carl round-9 P1 regression (stale-disk A / overlay B at pair start):
    /// the record handed to `spawn_agent_child` must carry the relay head for
    /// everything relay-owned, while device-local lifecycle state
    /// (`start_on_app_launch`) survives — previously a follower device
    /// showing relay config B pair-started stale disk config A.
    #[test]
    fn pair_start_spawns_relay_config_not_stale_disk() {
        let pubkey = "aa".repeat(32);
        let mut overlay = PrivateConfigOverlay::default();
        overlay.insert(test_relay_payload(&pubkey)).unwrap();

        let disk = stale_disk_record(&pubkey);
        let spawn = resolve_pair_spawn_record(overlay.resolve_local_record(&disk), &[])
            .expect("local resolved record must survive the fold");

        assert_eq!(spawn.name, "relay name");
        assert_eq!(spawn.system_prompt.as_deref(), Some("relay prompt"));
        assert_eq!(spawn.model.as_deref(), Some("relay-model"));
        assert_eq!(spawn.private_key_nsec, "nsec-relay");
        assert_eq!(spawn.parallelism, 4);
        assert!(
            spawn.start_on_app_launch,
            "device-local lifecycle flag must survive the overlay resolve"
        );

        // NEGATIVE CONTROL: an empty overlay leaves the disk record as-is —
        // the assertions above prove the patch, not the fixture.
        let untouched = resolve_pair_spawn_record(
            PrivateConfigOverlay::default().resolve_local_record(&disk),
            &[],
        )
        .unwrap();
        assert_eq!(untouched.name, "stale disk name");
        assert_eq!(untouched.private_key_nsec, "nsec-stale-disk");
    }

    /// The linked persona keeps its definition-authoritative precedence over
    /// BOTH disk and relay bytes — the snapshot is re-applied after the
    /// overlay patch, mirroring the interactive start and restore paths.
    #[test]
    fn persona_snapshot_reapplies_on_top_of_overlay_patch() {
        let pubkey = "bb".repeat(32);
        let mut payload = test_relay_payload(&pubkey);
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
        let spawn = resolve_pair_spawn_record(resolved, std::slice::from_ref(&persona)).unwrap();
        assert_eq!(spawn.system_prompt.as_deref(), Some("definition prompt"));
        assert_eq!(spawn.model.as_deref(), Some("definition-model"));
        // Relay still owns what the definition does not.
        assert_eq!(spawn.name, "relay name");
        assert_eq!(spawn.private_key_nsec, "nsec-relay");
    }

    /// A relay head that migrated the agent off the local backend must be
    /// refused by name — never spawned as a local child from leftover disk
    /// bytes.
    #[test]
    fn resolved_non_local_backend_is_refused() {
        let pubkey = "cc".repeat(32);
        let mut record = stale_disk_record(&pubkey);
        record.backend = BackendKind::Provider {
            id: "cloud".into(),
            config: serde_json::json!({}),
        };
        let error = resolve_pair_spawn_record(record, &[]).unwrap_err();
        assert_eq!(error, "managed runtime pairs require a local agent");
    }

    /// Carl round-9 P1, relay-only arm: a hydrated overlay head with no disk
    /// row materializes into a spawnable record on the pair route (previously
    /// "agent not found"); a relay-only PROVIDER head is refused by name at
    /// the same fold.
    #[test]
    fn relay_only_record_materializes_for_pair_start() {
        let pubkey = "dd".repeat(32);
        let mut overlay = PrivateConfigOverlay::default();
        overlay.insert(test_relay_payload(&pubkey)).unwrap();

        let materialized = overlay
            .materialize_relay_only_record(&pubkey, &[])
            .expect("relay-only head must materialize");
        let spawn = resolve_pair_spawn_record(materialized, &[]).unwrap();
        assert_eq!(spawn.name, "relay name");
        assert_eq!(spawn.private_key_nsec, "nsec-relay");
        assert_eq!(spawn.backend, BackendKind::Local);

        let mut provider = test_relay_payload(&"ee".repeat(32));
        provider.config.backend = serde_json::json!({"type":"provider","id":"cloud","config":{}});
        let mut overlay = PrivateConfigOverlay::default();
        overlay.insert(provider).unwrap();
        let materialized = overlay
            .materialize_relay_only_record(&"ee".repeat(32), &[])
            .unwrap();
        assert!(resolve_pair_spawn_record(materialized, &[]).is_err());
    }
}
