use std::collections::HashMap;

use super::ManagedAgentPairRuntime;
use crate::managed_agents::{
    ManagedAgentProcess, ManagedAgentRecord, ManagedAgentRuntimeKey, UNVERIFIED_JOB_REAP_PREFIX,
};

pub(crate) fn mark_unverified_runtime(
    app: Option<&tauri::AppHandle>,
    record: &mut ManagedAgentRecord,
    key: &ManagedAgentRuntimeKey,
    recovery_pid: u32,
    now: String,
    detail: String,
) -> String {
    let message = app
        .map(|app| {
            crate::managed_agents::mark_pair_recovery_uncertain(
                app,
                record,
                key,
                recovery_pid,
                detail.clone(),
            )
        })
        .transpose();
    if let Err(error) = message {
        let pair = serde_json::to_string(key).unwrap_or_else(|_| key.runtime_id());
        let projected_pair_is_preserved = record.last_error.as_deref().is_some_and(|current| {
            current.starts_with(UNVERIFIED_JOB_REAP_PREFIX)
                && current.contains(&format!("pair={pair} pid={recovery_pid}"))
        });
        if !projected_pair_is_preserved {
            record.runtime_pid = Some(recovery_pid);
            record.last_stopped_at = None;
            record.last_error = Some(format!(
                "{UNVERIFIED_JOB_REAP_PREFIX} pair={pair} pid={recovery_pid}; recovery sidecar persistence failed closed: {error}; {detail}"
            ));
            record.last_error_code = None;
        }
    } else if app.is_none() {
        record.runtime_pid = Some(recovery_pid);
        record.last_stopped_at = None;
        record.last_error = Some(format!(
            "{UNVERIFIED_JOB_REAP_PREFIX} pair={} pid={recovery_pid}; {detail}",
            serde_json::to_string(key).unwrap_or_else(|_| key.runtime_id())
        ));
        record.last_error_code = None;
    }
    record.updated_at = now;
    record.last_error.clone().unwrap_or_else(|| {
        format!(
            "{UNVERIFIED_JOB_REAP_PREFIX} exact pair recovery remains uncertain for pid {recovery_pid}"
        )
    })
}

pub(crate) fn clear_error_after_verified_start(record: &mut ManagedAgentRecord) {
    if !crate::managed_agents::has_unverified_job_reap(record) {
        record.last_error = None;
        record.last_error_code = None;
    }
}

#[cfg(windows)]
struct ReceiptFailureContext {
    key: ManagedAgentRuntimeKey,
    now: String,
    error: String,
}

#[cfg(windows)]
fn cleanup_failed_receipt_with(
    app: Option<&tauri::AppHandle>,
    mut process: ManagedAgentProcess,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    context: ReceiptFailureContext,
    terminate: impl FnOnce(&mut ManagedAgentProcess) -> Result<std::process::ExitStatus, String>,
) -> Result<(), String> {
    let ReceiptFailureContext { key, now, error } = context;
    match terminate(&mut process) {
        Ok(_) => Err(error),
        Err(cleanup_error) => {
            let recovery_pid = process.child.id();
            let message = mark_unverified_runtime(
                app,
                record,
                &key,
                recovery_pid,
                now,
                format!(
                    "{error}; cleanup is incomplete and exact pair authority remains tracked for retry: {cleanup_error}"
                ),
            );
            runtimes.insert(key, ManagedAgentPairRuntime::starting(process));
            Err(message)
        }
    }
}

pub(crate) fn cleanup(
    app: &tauri::AppHandle,
    process: ManagedAgentProcess,
    record: &mut ManagedAgentRecord,
    runtimes: &mut HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    key: ManagedAgentRuntimeKey,
    now: String,
    error: String,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        cleanup_failed_receipt_with(
            Some(app),
            process,
            record,
            runtimes,
            ReceiptFailureContext { key, now, error },
            crate::managed_agents::process_lifecycle::terminate_managed_agent_process,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        let _ = super::terminate_process(process.child.id());
        let _ = process.child.wait();
        Err(error)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[cfg(windows)]
    #[tokio::test]
    async fn receipt_and_cleanup_failure_retains_exact_runtime_key_and_record_error() {
        let mut record: ManagedAgentRecord = serde_json::from_str(
            r#"{
                "pubkey": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "name": "receipt-test",
                "private_key_nsec": "nsec1fake",
                "relay_url": "",
                "acp_command": "buzz-acp",
                "agent_command": "buzz-agent",
                "agent_args": [],
                "mcp_command": "",
                "turn_timeout_seconds": 320,
                "system_prompt": null,
                "model": null,
                "provider": null,
                "env_vars": {},
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            }"#,
        )
        .expect("receipt failure fixture");
        let key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), "wss://relay.example")
            .expect("receipt failure runtime key");
        let mut command = Command::new("ping.exe");
        command
            .args(["-t", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let process = crate::managed_agents::process_lifecycle::spawn_managed_agent_process(
            command,
            std::path::PathBuf::new(),
            0,
            false,
            None,
            "receipt-generation".into(),
        )
        .expect("spawn receipt failure child in Job Object");
        let child_id = process.child.id();
        let mut runtimes = HashMap::new();

        let error = cleanup_failed_receipt_with(
            None,
            process,
            &mut record,
            &mut runtimes,
            ReceiptFailureContext {
                key: key.clone(),
                now: "2026-01-02T00:00:00Z".into(),
                error: "receipt write failed".into(),
            },
            |_| Err("cleanup denied".into()),
        )
        .expect_err("receipt plus cleanup failure must propagate");

        assert!(error.contains("receipt write failed"));
        assert!(error.contains("cleanup denied"));
        assert!(runtimes.contains_key(&key));
        assert!(record
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("cleanup denied")));
        let persisted = serde_json::to_string(&record).expect("serialize mutated recovery record");
        let reloaded: ManagedAgentRecord =
            serde_json::from_str(&persisted).expect("reload mutated recovery record");
        assert!(reloaded
            .last_error
            .as_deref()
            .is_some_and(|message| message.starts_with(UNVERIFIED_JOB_REAP_PREFIX)));
        assert_eq!(reloaded.runtime_pid, Some(child_id));
        let durable_error = reloaded
            .last_error
            .as_deref()
            .expect("durable recovery error");
        assert!(durable_error.contains(&key.pubkey));
        assert!(durable_error.contains(&key.relay_url));
        let mut runtime = runtimes.remove(&key).expect("retained exact runtime");
        crate::managed_agents::process_lifecycle::terminate_managed_agent_process(
            &mut runtime.process,
        )
        .expect("cleanup retained Job authority");
        assert!(runtime.process.job.is_some());
        crate::managed_agents::process_lifecycle::release_finalized_managed_agent_process(
            &mut runtime.process,
        )
        .expect("release retained Job after no receipt was committed");
        assert!(runtime.process.job.is_none());
    }
}
