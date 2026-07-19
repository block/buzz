use std::collections::HashMap;

use tauri::AppHandle;

use super::{
    append_log_marker, current_instance_id, now_iso, process_belongs_to_us,
    process_has_buzz_marker, process_is_running, terminate_process, ManagedAgentPairRuntime,
    ManagedAgentRecord, ManagedAgentRuntimeKey,
};

fn managed_agent_runtime_keys<T>(
    runtimes: &HashMap<ManagedAgentRuntimeKey, T>,
    pubkey: &str,
) -> Vec<ManagedAgentRuntimeKey> {
    runtimes
        .keys()
        .filter(|key| key.pubkey.eq_ignore_ascii_case(pubkey))
        .cloned()
        .collect()
}

pub fn stop_managed_agent_process(
    app: &AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
) -> Result<(), String> {
    let keys = managed_agent_runtime_keys(runtimes, &record.pubkey);
    if keys.is_empty() {
        // Legacy PID cleanup only; pair receipts are restored separately.
        if let Some(pid) = record.runtime_pid.take() {
            if process_is_running(pid)
                && process_belongs_to_us(pid)
                && process_has_buzz_marker(pid, &current_instance_id(app))
            {
                terminate_process(pid)?;
            }
            record.updated_at = now_iso();
        }
        super::super::remove_agent_pid_file(app, &record.pubkey);
        return Ok(());
    }

    let mut errors = Vec::new();
    for key in keys {
        let Some(mut runtime) = runtimes.remove(&key) else {
            continue;
        };
        let result = (|| -> Result<(), String> {
            #[cfg(unix)]
            terminate_process(runtime.child.id())?;
            #[cfg(windows)]
            match runtime.job.take() {
                Some(job) => drop(job),
                None => runtime
                    .child
                    .kill()
                    .map_err(|error| format!("failed to kill agent process: {error}"))?,
            }
            #[cfg(not(any(unix, windows)))]
            runtime
                .child
                .kill()
                .map_err(|error| format!("failed to kill agent process: {error}"))?;
            let status = runtime
                .child
                .wait()
                .map_err(|error| format!("failed to wait for agent shutdown: {error}"))?;
            record.last_exit_code = status.code();
            super::super::remove_agent_runtime_receipt(app, &key);
            append_log_marker(
                &runtime.log_path,
                &format!(
                    "=== stopped {} ({}) at {} ===",
                    record.name,
                    record.pubkey,
                    now_iso()
                ),
            )
        })();
        if let Err(error) = result {
            errors.push(format!("{}: {error}", key.relay_url));
            // Keep failed teardown visible/manageable instead of orphaning it.
            runtimes.insert(key, runtime);
        }
    }

    let now = now_iso();
    record.runtime_pid = None;
    record.updated_at = now.clone();
    record.last_stopped_at = Some(now);
    record.last_error = None;
    record.last_error_code = None;
    super::super::remove_agent_pid_file(app, &record.pubkey);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "failed to stop one or more managed-agent runtimes: {}",
            errors.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_wide_selection_drains_every_pair_only_for_that_agent() {
        let agent = "aa".repeat(32);
        let other = "bb".repeat(32);
        let first = ManagedAgentRuntimeKey::new(&agent, "wss://one.example").unwrap();
        let second = ManagedAgentRuntimeKey::new(&agent, "wss://two.example").unwrap();
        let unrelated = ManagedAgentRuntimeKey::new(other, "wss://one.example").unwrap();
        let runtimes = HashMap::from([(first.clone(), ()), (second.clone(), ()), (unrelated, ())]);

        let mut selected = managed_agent_runtime_keys(&runtimes, &agent);
        selected.sort_by(|left, right| left.relay_url.cmp(&right.relay_url));
        assert_eq!(selected, vec![first, second]);
    }
}
