use super::*;

/// Kill stale agent processes from a previous session whose PID is still alive
/// but not tracked in the current `runtimes` map. Updates the record fields and
/// returns `true` if any records were modified.
#[cfg(windows)]
pub fn kill_stale_tracked_processes(
    records: &mut [ManagedAgentRecord],
    runtimes: &HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    _instance_id: &str,
) -> bool {
    use crate::managed_agents::BackendKind;

    let mut changed = false;
    for record in records.iter_mut() {
        let Some(pid) = record.runtime_pid else {
            continue;
        };
        if record.backend != BackendKind::Local
            || runtimes.keys().any(|key| key.pubkey == record.pubkey)
            || super::super::has_unverified_job_reap(record)
        {
            continue;
        }
        let message = format!(
            "cannot safely reconcile persisted Windows PID {pid} without owned Child/Job authority; preserving recovery identity"
        );
        if record.last_error.as_deref() != Some(message.as_str()) {
            record.updated_at = crate::util::now_iso();
            record.last_error = Some(message);
            record.last_error_code = None;
            changed = true;
        }
    }
    changed
}

#[cfg(not(windows))]
pub fn kill_stale_tracked_processes(
    records: &mut [ManagedAgentRecord],
    runtimes: &HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    instance_id: &str,
) -> bool {
    kill_stale_tracked_processes_with(
        records,
        runtimes,
        |pid| process_has_buzz_marker(pid, instance_id),
        terminate_process,
    )
}

/// Injectable version of `kill_stale_tracked_processes` for testing.
/// `has_marker(pid)` returns true when the process carries this instance's
/// `BUZZ_MANAGED_AGENT` marker; `kill(pid)` performs the termination.
pub(crate) fn kill_stale_tracked_processes_with(
    records: &mut [ManagedAgentRecord],
    runtimes: &HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    has_marker: impl Fn(u32) -> bool,
    mut kill: impl FnMut(u32) -> Result<(), String>,
) -> bool {
    use crate::managed_agents::BackendKind;

    let mut changed = false;
    for record in records.iter_mut() {
        if record.backend != BackendKind::Local {
            continue;
        }
        let Some(pid) = record.runtime_pid else {
            continue;
        };
        if super::super::has_unverified_job_reap(record) {
            // Post-Job launcher reap remains unverified. Preserve durable
            // PID/receipt recovery identity; Windows marker probes cannot
            // prove this process absent.
            continue;
        }
        if !runtimes.keys().any(|key| key.pubkey == record.pubkey) {
            // Name-gate is omitted intentionally: custom harnesses use arbitrary
            // binary names not in KNOWN_AGENT_BINARIES. BUZZ_MANAGED_AGENT is the
            // authoritative ownership proof; terminate only if it matches.
            if has_marker(pid) {
                if let Err(error) = kill(pid) {
                    // A failed termination must retain the persisted PID so a
                    // later reconciliation can retry with the same ownership
                    // marker instead of publishing a false stopped state.
                    record.updated_at = crate::util::now_iso();
                    record.last_error = Some(format!(
                        "failed to terminate stale managed-agent process {pid}: {error}"
                    ));
                    record.last_error_code = None;
                    changed = true;
                    continue;
                }
                record.last_stopped_at = Some(crate::util::now_iso());
                record.last_error = None;
            }
            record.runtime_pid = None;
            record.updated_at = crate::util::now_iso();
            changed = true;
        }
    }
    changed
}

pub fn sync_managed_agent_processes(
    app: &tauri::AppHandle,
    records: &mut [ManagedAgentRecord],
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
) -> (bool, Vec<ManagedAgentRuntimeKey>) {
    sync_managed_agent_processes_with(Some(app), records, runtimes, |key, runtime| {
        let status = runtime.child.try_wait()?;
        #[cfg(windows)]
        if status.is_some() {
            return super::super::process_lifecycle::finalize_tracked_runtime(app, key, runtime)
                .map(Some)
                .map_err(std::io::Error::other);
        }
        #[cfg(not(windows))]
        if status.is_some() {
            super::super::remove_agent_runtime_receipt(app, key).map_err(std::io::Error::other)?;
        }
        Ok(status)
    })
}

pub(crate) fn sync_managed_agent_processes_with(
    app: Option<&tauri::AppHandle>,
    records: &mut [ManagedAgentRecord],
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    inspect: impl FnMut(
        &ManagedAgentRuntimeKey,
        &mut ManagedAgentPairRuntime,
    ) -> std::io::Result<Option<std::process::ExitStatus>>,
) -> (bool, Vec<ManagedAgentRuntimeKey>) {
    sync_managed_agent_processes_with_recovery(app, records, runtimes, inspect, |record, key| {
        if let Some(app) = app {
            super::super::clear_pair_recovery_with_terminal_proof(app, record, key)
        } else {
            Ok(false)
        }
    })
}

pub(crate) fn sync_managed_agent_processes_with_recovery(
    app: Option<&tauri::AppHandle>,
    records: &mut [ManagedAgentRecord],
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    inspect: impl FnMut(
        &ManagedAgentRuntimeKey,
        &mut ManagedAgentPairRuntime,
    ) -> std::io::Result<Option<std::process::ExitStatus>>,
    clear_recovery: impl FnMut(&mut ManagedAgentRecord, &ManagedAgentRuntimeKey) -> Result<bool, String>,
) -> (bool, Vec<ManagedAgentRuntimeKey>) {
    sync_managed_agent_processes_with_recovery_and_persist(
        app,
        records,
        runtimes,
        inspect,
        clear_recovery,
        |records| match app {
            Some(app) => super::super::save_managed_agents(app, records),
            None => Ok(()),
        },
    )
}

pub(crate) fn sync_managed_agent_processes_with_recovery_and_persist(
    app: Option<&tauri::AppHandle>,
    records: &mut [ManagedAgentRecord],
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    mut inspect: impl FnMut(
        &ManagedAgentRuntimeKey,
        &mut ManagedAgentPairRuntime,
    ) -> std::io::Result<Option<std::process::ExitStatus>>,
    mut clear_recovery: impl FnMut(
        &mut ManagedAgentRecord,
        &ManagedAgentRuntimeKey,
    ) -> Result<bool, String>,
    persist: impl FnOnce(&[ManagedAgentRecord]) -> Result<(), String>,
) -> (bool, Vec<ManagedAgentRuntimeKey>) {
    let mut changed = false;
    let mut exited = Vec::new();
    let mut terminal_summaries = std::collections::HashMap::<
        String,
        Vec<(
            String,
            Option<i32>,
            Option<super::super::storage::AgentLogError>,
        )>,
    >::new();
    let mut runtime_keys = runtimes.keys().cloned().collect::<Vec<_>>();
    runtime_keys.sort_by_key(ManagedAgentRuntimeKey::runtime_id);

    for key in runtime_keys {
        let Some(runtime) = runtimes.get_mut(&key) else {
            continue;
        };
        let status = match inspect(&key, runtime) {
            Ok(status) => status,
            Err(error) => {
                if let Some(record) = records
                    .iter_mut()
                    .find(|record| record.pubkey == key.pubkey)
                {
                    #[cfg(windows)]
                    super::receipt_failure::mark_unverified_runtime(
                        app,
                        record,
                        &key,
                        runtime.child.id(),
                        now_iso(),
                        format!("failed to inspect or finalize process state: {error}"),
                    );
                    #[cfg(not(windows))]
                    {
                        record.updated_at = now_iso();
                        record.last_error =
                            Some(format!("failed to inspect process state: {error}"));
                        record.last_error_code = None;
                    }
                }
                changed = true;
                // Inspection failure is not proof of exit. Preserve the exact
                // Child/Job authority so callers can retry Stop instead of
                // converting an uncertain live tree into an invisible orphan.
                continue;
            }
        };

        let Some(status) = status else {
            continue;
        };

        let log_err = if status.success() {
            None
        } else {
            Some(
                super::super::meaningful_agent_error_from_log(&runtime.log_path).unwrap_or_else(
                    || super::super::storage::AgentLogError {
                        message: format!("harness exited with status {status}"),
                        code: None,
                    },
                ),
            )
        };

        if let Some(record) = records
            .iter_mut()
            .find(|record| record.pubkey == key.pubkey)
        {
            record.updated_at = now_iso();
            if !status.success() || super::super::pending_pair_failures(record).is_empty() {
                record.last_exit_code = status.code();
            }
            if let Err(error) = clear_recovery(record, &key) {
                super::super::record_terminal_proof_pending_recovery_clear(
                    record,
                    &key,
                    runtime.child.id(),
                    &error,
                );
                if let Some(log_error) = &log_err {
                    super::super::record_pending_pair_failure(
                        record,
                        &key,
                        status.code(),
                        log_error,
                    );
                }
                changed = true;
                // Keep the finalized Child/Job token in memory until either
                // the canonical clear or its exact-pair retry marker commits.
                continue;
            }
            if let Some(error) = &log_err {
                super::super::record_pending_pair_failure(record, &key, status.code(), error);
            }
            terminal_summaries
                .entry(key.pubkey.clone())
                .or_default()
                .push((key.runtime_id(), status.code(), log_err));
        }

        changed = true;
        exited.push(key.clone());
    }

    for record in records.iter_mut() {
        let Some(mut summaries) = terminal_summaries.remove(&record.pubkey) else {
            continue;
        };
        if runtimes
            .keys()
            .any(|key| key.pubkey == record.pubkey && !exited.iter().any(|exited| exited == key))
            || super::super::has_unverified_job_reap(record)
        {
            continue;
        }
        summaries.sort_by(|left, right| left.0.cmp(&right.0));
        let mut failures = super::super::pending_pair_failures(record);
        for (runtime_id, _, error) in &summaries {
            if let Some(error) = error {
                failures.insert(runtime_id.clone(), error.message.clone());
            }
        }
        let current_failure_code = summaries
            .iter()
            .find_map(|(_, _, error)| error.as_ref().and_then(|error| error.code));
        record.last_stopped_at = Some(now_iso());
        if failures.is_empty() {
            record.last_exit_code = summaries.first().and_then(|summary| summary.1);
            record.last_error = None;
            record.last_error_code = None;
        } else {
            record.last_exit_code = summaries
                .iter()
                .find(|summary| summary.2.is_some())
                .and_then(|summary| summary.1)
                .or(record.last_exit_code);
            record.last_error = Some(
                failures
                    .iter()
                    .map(|(runtime_id, message)| format!("{runtime_id}: {message}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
            record.last_error_code = current_failure_code.or(record.last_error_code);
        }
    }

    // `runtime_pid` is legacy bookkeeping. Pair runtimes and receipts are the
    // authoritative lifecycle source; migration cleanup is handled separately.
    for record in records.iter_mut() {
        #[cfg(windows)]
        if record.runtime_pid.is_some() {
            // A persisted Windows PID without its owned Child/Job handle is
            // unresolved recovery evidence. Generic polling cannot prove it
            // absent because the Windows liveness probe is intentionally not
            // PID-authoritative; admission/migration must classify it first.
            continue;
        }
        if super::super::has_unverified_job_reap(record) {
            continue;
        }
        if record.runtime_pid.take().is_some() {
            record.updated_at = now_iso();
            changed = true;
        }
    }

    if !exited.is_empty() {
        if persist(records).is_ok() {
            for key in &exited {
                runtimes.remove(key);
            }
        } else {
            // The Child/Job token remains authoritative until the terminal
            // record update has crossed its durable commit barrier.
            exited.clear();
        }
    }

    (changed, exited)
}
