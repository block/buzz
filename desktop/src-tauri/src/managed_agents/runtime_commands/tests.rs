#[cfg(windows)]
use super::super::process_lifecycle::finalize_tracked_runtime_with;
use super::*;

#[cfg(windows)]
#[tokio::test]
async fn tracked_windows_runtime_stop_uses_the_owned_job_and_is_bounded() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "ping -t 127.0.0.1 | Out-Null",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let process = crate::managed_agents::process_lifecycle::spawn_managed_agent_process(
        command,
        std::path::PathBuf::new(),
        0,
        false,
        None,
        "test-generation".into(),
    )
    .expect("spawn tracked Windows stop fixture inside a Job Object");
    let mut runtime = ManagedAgentPairRuntime::starting(process);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);

    let started = Instant::now();
    std::thread::spawn(move || {
        let result = finalize_tracked_runtime_with(&mut runtime, || Ok(()));
        let _ = sender.send((result, runtime.job.is_some()));
    });
    let (result, job_retained) = receiver
        .recv_timeout(Duration::from_secs(7))
        .expect("tracked Windows stop exceeded its external bound");
    result.expect("owned tracked runtime must terminate and reap");

    assert!(started.elapsed() < Duration::from_secs(7));
    assert!(
        !job_retained,
        "successful Stop must consume the kill-on-close Job handle"
    );
    println!(
        "NATIVE_STOP_PROOF elapsed_ms={} job_members=0 child_reaped=true",
        started.elapsed().as_millis()
    );
}

#[cfg(windows)]
#[tokio::test]
async fn tracked_windows_runtime_stop_reaps_a_descendant_process() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_nanos();
    let fixture_dir =
        std::env::temp_dir().join(format!("buzz-stop-tree-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&fixture_dir).expect("create process-tree fixture directory");
    let pid_path = fixture_dir.join("child.pid");
    let quote = |path: &std::path::Path| path.display().to_string().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $child=Start-Process -FilePath 'ping.exe' \
           -ArgumentList '-t','127.0.0.1' -WindowStyle Hidden -PassThru; \
         Set-Content -LiteralPath '{}' -Value $child.Id; \
         Wait-Process -Id $child.Id",
        quote(&pid_path),
    );
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let process = crate::managed_agents::process_lifecycle::spawn_managed_agent_process(
        command,
        std::path::PathBuf::new(),
        0,
        false,
        None,
        "tree-test-generation".into(),
    )
    .expect("spawn process-tree fixture inside a Job Object");

    let pid_deadline = Instant::now() + Duration::from_secs(5);
    let descendant_pid = loop {
        if let Ok(raw) = std::fs::read_to_string(&pid_path) {
            break raw
                .trim()
                .parse::<u32>()
                .expect("parse descendant process id");
        }
        assert!(
            Instant::now() < pid_deadline,
            "descendant process did not start within five seconds"
        );
        std::thread::sleep(Duration::from_millis(25));
    };

    let mut runtime = ManagedAgentPairRuntime::starting(process);
    let started = Instant::now();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = finalize_tracked_runtime_with(&mut runtime, || Ok(()));
        let _ = sender.send((result, runtime.job.is_some()));
    });
    let (result, job_retained) = receiver
        .recv_timeout(Duration::from_secs(7))
        .expect("process-tree stop exceeded its external bound");
    result.expect("process-tree stop must succeed");
    assert!(
        !job_retained,
        "tree Stop must consume the kill-on-close Job handle"
    );

    let output = Command::new("tasklist.exe")
        .args([
            "/FI",
            &format!("PID eq {descendant_pid}"),
            "/FO",
            "CSV",
            "/NH",
        ])
        .output()
        .expect("query descendant process after stop");
    let listing = String::from_utf8_lossy(&output.stdout);
    assert!(
        !listing.contains(&format!(",\"{descendant_pid}\",")),
        "descendant process {descendant_pid} survived tracked Stop: {listing}"
    );
    println!(
        "NATIVE_TREE_STOP_PROOF elapsed_ms={} job_members=0 child_reaped=true descendant_pid={} descendant_alive=false",
        started.elapsed().as_millis(),
        descendant_pid
    );
    let _ = std::fs::remove_dir_all(&fixture_dir);
}

#[cfg(windows)]
#[tokio::test]
async fn syncing_an_exited_launcher_finalizes_its_live_job_descendant() {
    use std::collections::HashMap;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let fixture_dir = std::env::temp_dir().join(format!(
        "buzz-sync-exited-tree-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&fixture_dir).expect("create exited-tree fixture directory");
    let pid_path = fixture_dir.join("child.pid");
    let script = format!(
        "$child=Start-Process -FilePath 'ping.exe' \
           -ArgumentList '-t','127.0.0.1' -WindowStyle Hidden -PassThru; \
         Set-Content -LiteralPath '{}' -Value $child.Id",
        pid_path.display().to_string().replace('\'', "''"),
    );
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let process = crate::managed_agents::process_lifecycle::spawn_managed_agent_process(
        command,
        std::path::PathBuf::new(),
        0,
        false,
        None,
        "sync-exited-generation".into(),
    )
    .expect("spawn exited-tree fixture inside a Job Object");
    let key = ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://relay.example")
        .expect("exited-tree runtime key");
    let mut runtimes = HashMap::from([(key.clone(), ManagedAgentPairRuntime::starting(process))]);
    let deadline = Instant::now() + Duration::from_secs(5);
    let descendant_pid = loop {
        if let Ok(raw) = std::fs::read_to_string(&pid_path) {
            break raw.trim().parse::<u32>().expect("parse descendant pid");
        }
        assert!(Instant::now() < deadline, "descendant pid was not written");
        std::thread::sleep(Duration::from_millis(20));
    };
    while runtimes
        .get_mut(&key)
        .expect("tracked exited-tree runtime")
        .process
        .child
        .try_wait()
        .expect("inspect launcher")
        .is_none()
    {
        assert!(Instant::now() < deadline, "launcher did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !runtimes[&key]
            .process
            .job
            .as_ref()
            .expect("owned Job")
            .members()
            .expect("query Job members")
            .is_empty(),
        "fixture must have a live descendant after launcher exit"
    );

    let mut records = vec![record_with_relay("wss://relay.example")];
    let started = Instant::now();
    let (_, exited) = super::super::runtime::sync_managed_agent_processes_with(
        None,
        &mut records,
        &mut runtimes,
        |_key, runtime| {
            let status = runtime.child.try_wait()?;
            if status.is_some() {
                return super::super::process_lifecycle::finalize_tracked_runtime_with(
                    runtime,
                    || Ok(()),
                )
                .map(Some)
                .map_err(std::io::Error::other);
            }
            Ok(status)
        },
    );
    assert_eq!(exited, vec![key.clone()]);
    assert!(runtimes.is_empty());
    let output = Command::new("tasklist.exe")
        .args([
            "/FI",
            &format!("PID eq {descendant_pid}"),
            "/FO",
            "CSV",
            "/NH",
        ])
        .output()
        .expect("query reconciled descendant process");
    let listing = String::from_utf8_lossy(&output.stdout);
    assert!(
        !listing.contains(&format!(",\"{descendant_pid}\",")),
        "descendant {descendant_pid} survived exited-launcher reconciliation: {listing}"
    );
    println!(
        "NATIVE_EXITED_LAUNCHER_PROOF elapsed_ms={} job_members=0 child_reaped=true descendant_pid={} descendant_alive=false",
        started.elapsed().as_millis(),
        descendant_pid
    );
    let _ = std::fs::remove_dir_all(fixture_dir);
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

#[test]
fn polled_terminal_clear_failure_keeps_runtime_and_exact_retry_marker() {
    let pubkey = "bc".repeat(32);
    let key = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://poll-retry.example").unwrap();
    let mut record: super::super::ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{"pubkey":"{pubkey}","name":"test","private_key_nsec":"nsec1fake","relay_url":"","acp_command":"buzz-acp","agent_command":"buzz-agent","agent_args":[],"mcp_command":"","turn_timeout_seconds":320,"system_prompt":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null}}"#
    ))
    .unwrap();
    let mut runtimes = std::collections::HashMap::from([(key.clone(), ())]);

    assert!(
        commit_polled_terminal_recovery(&mut record, &key, 9001, &runtimes, |_, _| {
            Err("synthetic sidecar clear failure".into())
        })
        .is_err()
    );
    assert!(runtimes.contains_key(&key));
    assert_eq!(
        super::super::terminal_proof_pending_recovery_clears(&record),
        vec![key]
    );

    let success_key = ManagedAgentRuntimeKey::new(pubkey, "wss://poll-commit.example").unwrap();
    runtimes.insert(success_key.clone(), ());
    commit_polled_terminal_recovery(&mut record, &success_key, 9002, &runtimes, |_, _| Ok(true))
        .unwrap();
    assert!(
        runtimes.contains_key(&success_key),
        "terminal token retires only after the main record commit"
    );
}

#[cfg(windows)]
#[test]
fn polled_nonzero_terminal_status_is_persisted() {
    use std::os::windows::process::ExitStatusExt;

    let pubkey = "bd".repeat(32);
    let mut record: super::super::ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{"pubkey":"{pubkey}","name":"test","private_key_nsec":"nsec1fake","relay_url":"","acp_command":"buzz-acp","agent_command":"buzz-agent","agent_args":[],"mcp_command":"","turn_timeout_seconds":320,"system_prompt":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null}}"#
    ))
    .unwrap();
    let status = std::process::ExitStatus::from_raw(7);

    let key = ManagedAgentRuntimeKey::new(pubkey, "wss://poll-exit.example").unwrap();
    record_polled_terminal_outcome(&mut record, &key, status, None);
    assert_eq!(record.last_exit_code, Some(7));
    assert!(record
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains('7')));

    record_polled_terminal_outcome(
        &mut record,
        &key,
        std::process::ExitStatus::from_raw(0),
        None,
    );
    assert_eq!(record.last_exit_code, Some(7));
}

#[cfg(windows)]
#[test]
fn polled_exit_diagnostic_preserves_sibling_recovery_projection() {
    use std::os::windows::process::ExitStatusExt;

    let pubkey = "be".repeat(32);
    let sibling = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://sibling.example").unwrap();
    let exited = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://failed.example").unwrap();
    let mut record: super::super::ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{"pubkey":"{pubkey}","name":"test","private_key_nsec":"nsec1fake","relay_url":"","acp_command":"buzz-acp","agent_command":"buzz-agent","agent_args":[],"mcp_command":"","turn_timeout_seconds":320,"system_prompt":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null}}"#
    ))
    .unwrap();
    let mut authority = super::super::ManagedAgentRecoveryAuthority::default();
    authority.mark_pair(&sibling, 9701, "sibling uncertainty".into());
    authority.project_compatibility(&mut record);

    record_polled_terminal_outcome(
        &mut record,
        &exited,
        std::process::ExitStatus::from_raw(11),
        None,
    );

    assert!(super::super::has_unverified_job_reap(&record));
    assert!(record.last_error.as_deref().is_some_and(|error| {
        error.contains("sibling.example") && error.contains(&exited.runtime_id())
    }));
    let failures = super::super::pending_pair_failures(&record);
    assert_eq!(failures.len(), 1);
    assert!(failures
        .get(&exited.runtime_id())
        .is_some_and(|message| message.contains("11")));
}

#[test]
fn pending_pair_failure_encoding_does_not_invent_siblings() {
    let pubkey = "bf".repeat(32);
    let key = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://encoded.example").unwrap();
    let mut record: super::super::ManagedAgentRecord = serde_json::from_str(&format!(
        r#"{{"pubkey":"{pubkey}","name":"test","private_key_nsec":"nsec1fake","relay_url":"","acp_command":"buzz-acp","agent_command":"buzz-agent","agent_args":[],"mcp_command":"","turn_timeout_seconds":320,"system_prompt":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","last_started_at":null,"last_stopped_at":null,"last_exit_code":null,"last_error":null}}"#
    ))
    .unwrap();

    super::super::record_pending_pair_failure(
        &mut record,
        &key,
        Some(19),
        &super::super::storage::AgentLogError {
            message: "primary failure; fake-runtime: not a sibling".into(),
            code: None,
        },
    );

    assert_eq!(
        super::super::pending_pair_failures(&record),
        std::collections::BTreeMap::from([(
            key.runtime_id(),
            "primary failure; fake-runtime: not a sibling".to_string()
        )])
    );
}
