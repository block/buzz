use super::*;

#[test]
fn kill_stale_failure_retains_pid_and_records_error() {
    let mut record = minimal_record("pubkey-retry");
    record.runtime_pid = Some(9004);
    let original_stopped_at = record.last_stopped_at.clone();
    let mut records = vec![record];
    let runtimes = std::collections::HashMap::new();

    let changed = super::super::kill_stale_tracked_processes_with(
        &mut records,
        &runtimes,
        |_pid| true,
        |_pid| Err("taskkill denied".to_string()),
    );

    assert!(changed);
    assert_eq!(records[0].runtime_pid, Some(9004));
    assert_eq!(records[0].last_stopped_at, original_stopped_at);
    assert!(records[0]
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("taskkill denied")));
}

#[cfg(windows)]
#[test]
fn windows_stale_pid_refusal_never_terminates_the_numeric_pid() {
    use std::process::{Command, Stdio};

    let mut child = Command::new("ping.exe")
        .args(["-t", "127.0.0.1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("stale PID sentinel");
    let pid = child.id();
    let mut record = minimal_record(&"da".repeat(32));
    record.runtime_pid = Some(pid);
    let mut records = vec![record];
    let runtimes = std::collections::HashMap::new();

    let changed = super::super::kill_stale_tracked_processes(
        &mut records,
        &runtimes,
        "different-desktop-instance",
    );

    assert!(changed);
    assert_eq!(records[0].runtime_pid, Some(pid));
    assert!(records[0].last_error.as_deref().is_some_and(|error| {
        error.contains("reconcile persisted Windows PID")
            && error.contains("without owned Child/Job authority")
            && error.contains("preserving recovery identity")
    }));
    assert!(
        child
            .try_wait()
            .expect("inspect stale PID sentinel")
            .is_none(),
        "Windows stale reconciliation must not kill by persisted numeric PID"
    );
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn unverified_job_reap_is_not_laundered_by_stale_cleanup() {
    let mut record = minimal_record(&"ce".repeat(32));
    record.runtime_pid = Some(4141);
    record.last_error = Some(format!(
        "{} synthetic reap uncertainty",
        crate::managed_agents::UNVERIFIED_JOB_REAP_PREFIX
    ));
    let mut records = vec![record];
    let runtimes = std::collections::HashMap::new();

    let changed = super::super::kill_stale_tracked_processes_with(
        &mut records,
        &runtimes,
        |_| true,
        |_| panic!("unverified Job reap must not fall through to PID kill"),
    );

    assert!(!changed);
    assert_eq!(records[0].runtime_pid, Some(4141));
    assert!(records[0]
        .last_error
        .as_deref()
        .is_some_and(|error| error.starts_with(crate::managed_agents::UNVERIFIED_JOB_REAP_PREFIX)));
}

#[test]
fn unverified_job_reap_pid_survives_runtime_sync() {
    let mut record = minimal_record(&"cf".repeat(32));
    record.runtime_pid = Some(5151);
    record.last_error = Some(format!(
        "{} synthetic reap uncertainty",
        crate::managed_agents::UNVERIFIED_JOB_REAP_PREFIX
    ));
    let mut records = vec![record];
    let mut runtimes = std::collections::HashMap::new();

    let (changed, exited) = super::super::lifecycle::sync_managed_agent_processes_with(
        None,
        &mut records,
        &mut runtimes,
        |_key, _runtime| Ok(None),
    );

    assert!(!changed);
    assert!(exited.is_empty());
    assert_eq!(records[0].runtime_pid, Some(5151));
    assert!(crate::managed_agents::has_unverified_job_reap(&records[0]));
}

#[cfg(windows)]
#[test]
fn legacy_windows_pid_without_marker_survives_runtime_sync() {
    let mut record = minimal_record(&"cd".repeat(32));
    record.runtime_pid = Some(5252);
    record.last_error = None;
    let original_stopped_at = record.last_stopped_at.clone();
    let mut records = vec![record];
    let mut runtimes = std::collections::HashMap::new();

    let (changed, exited) = super::super::lifecycle::sync_managed_agent_processes_with(
        None,
        &mut records,
        &mut runtimes,
        |_key, _runtime| Ok(None),
    );

    assert!(!changed);
    assert!(exited.is_empty());
    assert_eq!(records[0].runtime_pid, Some(5252));
    assert_eq!(records[0].last_stopped_at, original_stopped_at);
    assert_eq!(records[0].last_error, None);
}

#[test]
fn sync_inspection_error_preserves_runtime_authority() {
    use std::collections::HashMap;

    let pubkey = "ee".repeat(32);
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://relay.example")
            .expect("runtime key fixture");
    let mut records = vec![minimal_record(&pubkey)];
    let mut runtimes = HashMap::from([(key.clone(), make_pair_runtime_placeholder())]);

    let (changed, exited) = super::super::lifecycle::sync_managed_agent_processes_with(
        None,
        &mut records,
        &mut runtimes,
        |_key, _runtime| Err(std::io::Error::other("synthetic inspection failure")),
    );

    assert!(changed);
    assert!(exited.is_empty(), "inspection error is not proof of exit");
    assert!(
        runtimes.contains_key(&key),
        "Child/Job authority must remain tracked for retry"
    );
    assert!(records[0]
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("synthetic inspection failure")));
}

#[cfg(windows)]
#[test]
fn sync_two_terminal_siblings_records_a_final_stopped_state() {
    use std::collections::HashMap;
    use std::os::windows::process::ExitStatusExt;

    let pubkey = "ef".repeat(32);
    let pair_a =
        crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://one.example")
            .expect("pair A key");
    let pair_b =
        crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://two.example")
            .expect("pair B key");
    let mut record = minimal_record(&pubkey);
    record.last_stopped_at = None;
    let mut records = vec![record];
    let mut runtimes = HashMap::from([
        (pair_a, make_pair_runtime_placeholder()),
        (pair_b, make_pair_runtime_placeholder()),
    ]);

    let (_, exited) = super::super::lifecycle::sync_managed_agent_processes_with(
        None,
        &mut records,
        &mut runtimes,
        |_, _| Ok(Some(std::process::ExitStatus::from_raw(0))),
    );

    assert_eq!(exited.len(), 2);
    assert!(runtimes.is_empty());
    assert!(records[0].last_stopped_at.is_some());
    assert_eq!(records[0].last_exit_code, Some(0));
}

#[cfg(windows)]
#[test]
fn staggered_terminal_sibling_success_preserves_earlier_failure() {
    use std::collections::HashMap;
    use std::os::windows::process::ExitStatusExt;

    let pubkey = "f0".repeat(32);
    let pair_a =
        crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://fail.example")
            .unwrap();
    let pair_b =
        crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://ok.example")
            .unwrap();
    let pair_a_id = pair_a.runtime_id();
    let mut records = vec![minimal_record(&pubkey)];
    records[0].last_stopped_at = None;
    let mut runtimes = HashMap::from([
        (pair_a.clone(), make_pair_runtime_placeholder()),
        (pair_b.clone(), make_pair_runtime_placeholder()),
    ]);

    let (_, first_exited) = super::super::lifecycle::sync_managed_agent_processes_with(
        None,
        &mut records,
        &mut runtimes,
        |key, _| Ok((key == &pair_a).then(|| std::process::ExitStatus::from_raw(1))),
    );
    assert_eq!(first_exited, vec![pair_a]);
    assert!(runtimes.contains_key(&pair_b));
    assert_eq!(records[0].last_exit_code, Some(1));
    assert!(records[0]
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains(&pair_a_id)));

    let (_, second_exited) = super::super::lifecycle::sync_managed_agent_processes_with(
        None,
        &mut records,
        &mut runtimes,
        |_, _| Ok(Some(std::process::ExitStatus::from_raw(0))),
    );
    assert_eq!(second_exited, vec![pair_b]);
    assert!(records[0].last_stopped_at.is_some());
    assert_eq!(records[0].last_exit_code, Some(1));
    assert!(records[0]
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains(&pair_a_id)));
}

#[cfg(windows)]
#[test]
fn terminal_clear_failure_retains_runtime_and_parseable_retry_marker() {
    use std::collections::HashMap;
    use std::os::windows::process::ExitStatusExt;

    let pubkey = "f1".repeat(32);
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://retry.example")
            .unwrap();
    let mut records = vec![minimal_record(&pubkey)];
    let mut runtimes = HashMap::from([(key.clone(), make_pair_runtime_placeholder())]);
    let (_, exited) = super::super::lifecycle::sync_managed_agent_processes_with_recovery(
        None,
        &mut records,
        &mut runtimes,
        |_, _| Ok(Some(std::process::ExitStatus::from_raw(1))),
        |_, _| Err("synthetic recovery sidecar save failure".into()),
    );

    assert!(exited.is_empty());
    assert!(runtimes.contains_key(&key));
    assert_eq!(
        crate::managed_agents::terminal_proof_pending_recovery_clears(&records[0]),
        vec![key]
    );
    assert_eq!(records[0].last_stopped_at, None);
    assert_eq!(records[0].last_exit_code, Some(1));
    assert!(records[0]
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("status") && error.contains("recovery sidecar")));
}

#[cfg(windows)]
#[test]
fn terminal_clear_success_without_main_record_commit_retains_runtime_authority() {
    use std::collections::HashMap;
    use std::os::windows::process::ExitStatusExt;

    let pubkey = "f3".repeat(32);
    let key = crate::managed_agents::ManagedAgentRuntimeKey::new(
        pubkey.clone(),
        "wss://commit-barrier.example",
    )
    .unwrap();
    let mut records = vec![minimal_record(&pubkey)];
    let mut runtimes = HashMap::from([(key.clone(), make_pair_runtime_placeholder())]);

    let (_, exited) =
        super::super::lifecycle::sync_managed_agent_processes_with_recovery_and_persist(
            None,
            &mut records,
            &mut runtimes,
            |_, _| Ok(Some(std::process::ExitStatus::from_raw(0))),
            |_, _| Ok(true),
            |_| Err("synthetic main record persistence failure".into()),
        );

    assert!(exited.is_empty());
    assert!(runtimes.contains_key(&key));

    let (_, exited) =
        super::super::lifecycle::sync_managed_agent_processes_with_recovery_and_persist(
            None,
            &mut records,
            &mut runtimes,
            |_, _| Ok(Some(std::process::ExitStatus::from_raw(0))),
            |_, _| Ok(false),
            |_| Ok(()),
        );
    assert_eq!(exited, vec![key.clone()]);
    assert!(!runtimes.contains_key(&key));
}

#[cfg(windows)]
#[test]
fn sibling_exit_failure_cannot_overwrite_remaining_recovery_pair() {
    use std::collections::HashMap;
    use std::os::windows::process::ExitStatusExt;

    let pubkey = "f2".repeat(32);
    let pair_a = crate::managed_agents::ManagedAgentRuntimeKey::new(
        pubkey.clone(),
        "wss://uncertain.example",
    )
    .unwrap();
    let pair_b =
        crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://exited.example")
            .unwrap();
    let pair_b_id = pair_b.runtime_id();
    let mut authority = crate::managed_agents::ManagedAgentRecoveryAuthority::default();
    authority.mark_pair(&pair_a, 9601, "pair A uncertainty".into());
    authority.mark_pair(&pair_b, 9602, "pair B uncertainty".into());
    let mut records = vec![minimal_record(&pubkey)];
    authority.project_compatibility(&mut records[0]);
    let mut runtimes = HashMap::from([(pair_b.clone(), make_pair_runtime_placeholder())]);

    let (_, exited) = super::super::lifecycle::sync_managed_agent_processes_with_recovery(
        None,
        &mut records,
        &mut runtimes,
        |_, _| Ok(Some(std::process::ExitStatus::from_raw(9))),
        |record, key| {
            authority.clear_pair_with_terminal_proof(key);
            authority.project_compatibility(record);
            Ok(true)
        },
    );

    assert_eq!(exited, vec![pair_b]);
    assert!(crate::managed_agents::has_unverified_job_reap(&records[0]));
    assert_eq!(records[0].last_stopped_at, None);
    assert_eq!(records[0].last_exit_code, Some(9));
    assert!(records[0]
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("uncertain.example") && error.contains(&pair_b_id)));
}
