use super::*;

/// Kill stale agent processes from a previous session whose PID is still alive
/// but not tracked in the current `runtimes` map. Updates the record fields and
/// returns `true` if any records were modified.
pub fn kill_stale_tracked_processes(
    records: &mut [ManagedAgentRecord],
    runtimes: &HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    instance_id: &str,
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
        if !runtimes.keys().any(|key| key.pubkey == record.pubkey) {
            if process_belongs_to_us(pid) && process_has_buzz_marker(pid, instance_id) {
                let _ = terminate_process(pid);
            }
            record.runtime_pid = None;
            record.last_stopped_at = Some(crate::util::now_iso());
            record.updated_at = crate::util::now_iso();
            changed = true;
        }
    }
    changed
}

pub fn sync_managed_agent_processes(
    records: &mut [ManagedAgentRecord],
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    _instance_id: &str,
) -> (bool, Vec<String>) {
    let mut changed = false;
    let mut exited = Vec::new();

    for (key, runtime) in runtimes.iter_mut() {
        let status = match runtime.child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                if let Some(record) = records
                    .iter_mut()
                    .find(|record| record.pubkey == key.pubkey)
                {
                    record.updated_at = now_iso();
                    record.last_error = Some(format!("failed to inspect process state: {error}"));
                    record.last_error_code = None;
                }
                changed = true;
                exited.push(key.clone());
                continue;
            }
        };

        let Some(status) = status else {
            continue;
        };

        if let Some(record) = records
            .iter_mut()
            .find(|record| record.pubkey == key.pubkey)
        {
            record.updated_at = now_iso();
            record.last_stopped_at = Some(now_iso());
            record.last_exit_code = status.code();
            let log_err = if status.success() {
                None
            } else {
                Some(
                    super::super::meaningful_agent_error_from_log(&runtime.log_path)
                        .unwrap_or_else(|| super::super::storage::AgentLogError {
                            message: format!("harness exited with status {status}"),
                            code: None,
                        }),
                )
            };
            record.last_error = log_err.as_ref().map(|e| e.message.clone());
            record.last_error_code = log_err.as_ref().and_then(|e| e.code);
        }

        changed = true;
        exited.push(key.clone());
    }

    let exited_pubkeys: Vec<String> = exited.iter().map(|key| key.pubkey.clone()).collect();
    for key in exited {
        runtimes.remove(&key);
    }

    // `runtime_pid` is legacy bookkeeping. Pair runtimes and receipts are the
    // authoritative lifecycle source; migration cleanup is handled separately.
    for record in records.iter_mut() {
        if record.runtime_pid.take().is_some() {
            record.updated_at = now_iso();
            changed = true;
        }
    }

    (changed, exited_pubkeys)
}
