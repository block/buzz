use super::*;

#[test]
fn broad_and_buzz_overlapping_read_roots_fail_closed() {
    let protected = protected_data_roots().unwrap();
    assert!(validate_read_only_roots(&[PathBuf::from("/")], &protected).is_err());
    if let Some(home) = dirs::home_dir() {
        assert!(validate_read_only_roots(std::slice::from_ref(&home), &protected).is_err());
        let buzz = home.join(".buzz");
        if buzz.is_dir() {
            assert!(validate_read_only_roots(&[buzz], &protected).is_err());
        }
    }
}

#[test]
fn invalid_identity_is_rejected_before_creating_a_run_root() {
    let profile = FilesystemIsolationProfile::Ephemeral {
        read_only_roots: Vec::new(),
    };
    let error =
        isolated_agent_command(&profile, "not-a-pubkey", "test", Path::new("/bin/sh")).unwrap_err();
    assert!(error.contains("exact 64-character"));
}

#[test]
fn prepared_receipt_uses_owner_ui_shape_without_changing_attestation_shape() {
    let receipt = PreparedFilesystemIsolation {
        identity_pubkey: "ab".repeat(32),
        run_id: "c".repeat(32),
        run_root: PathBuf::from("/tmp/run"),
        attestation: FilesystemIsolationAttestation {
            version: 1,
            enforcement: "test",
            identity_pubkey: "ab".repeat(32),
            run_id: "c".repeat(32),
            run_root: PathBuf::from("/tmp/run"),
            allowed_read_roots: Vec::new(),
            allowed_write_roots: vec![PathBuf::from("/tmp/run")],
            denied_roots: vec![PathBuf::from("/Users")],
        },
    };
    let value = serde_json::to_value(receipt).unwrap();
    assert_eq!(value["runId"], "c".repeat(32));
    assert_eq!(value["runRoot"], "/tmp/run");
    assert_eq!(value["attestation"]["run_id"], "c".repeat(32));
    assert!(value["attestation"].get("runId").is_none());
}

#[cfg(target_os = "macos")]
#[test]
fn seatbelt_receipt_never_allows_home_or_shared_buzz_root() {
    let profile = FilesystemIsolationProfile::Ephemeral {
        read_only_roots: Vec::new(),
    };
    let (_command, run) =
        isolated_agent_command(&profile, &"ab".repeat(32), "test", Path::new("/bin/sh")).unwrap();
    let home = dirs::home_dir().unwrap();
    assert!(!run.attestation.allowed_read_roots.contains(&home));
    assert!(!run
        .attestation
        .allowed_read_roots
        .iter()
        .any(|root| root == &home.join(".buzz")));
    assert!(run
        .attestation
        .denied_roots
        .contains(&PathBuf::from("/Users")));
    assert!(run
        .root()
        .starts_with(std::env::temp_dir().canonicalize().unwrap()));
}

#[cfg(target_os = "macos")]
#[test]
fn prepared_root_exists_before_spawn_and_is_consumed_exactly_once() {
    let profile = FilesystemIsolationProfile::Ephemeral {
        read_only_roots: Vec::new(),
    };
    let identity = "cd".repeat(32);
    let prepared =
        prepare_isolated_agent_run(&profile, &identity, "test", Path::new("/bin/sh")).unwrap();
    assert!(prepared.run_root.is_dir());
    assert_eq!(
        get_prepared_isolated_agent_run(&identity).unwrap(),
        Some(prepared.clone())
    );

    let marker = prepared.run_root.join("M2");
    fs::write(&marker, "harmless-marker").unwrap();
    assert_eq!(fs::read_to_string(&marker).unwrap(), "harmless-marker");

    let (_command, run) =
        consume_prepared_isolated_agent_command(&profile, &identity, "test", Path::new("/bin/sh"))
            .unwrap();
    assert!(marker.is_file(), "prepared marker disappeared before spawn");
    assert!(consume_prepared_isolated_agent_command(
        &profile,
        &identity,
        "test",
        Path::new("/bin/sh"),
    )
    .unwrap_err()
    .contains("owner-prepared"));
    drop(run);
    assert!(!prepared.run_root.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn concurrent_prepare_fails_closed_and_abort_is_exact() {
    let profile = FilesystemIsolationProfile::Ephemeral {
        read_only_roots: Vec::new(),
    };
    let identity = "ef".repeat(32);
    let first =
        prepare_isolated_agent_run(&profile, &identity, "test", Path::new("/bin/sh")).unwrap();
    assert!(
        prepare_isolated_agent_run(&profile, &identity, "test", Path::new("/bin/sh"))
            .unwrap_err()
            .contains("durable filesystem-isolation receipt")
    );
    assert!(abort_prepared_isolated_agent_run(&identity, &"0".repeat(32)).is_err());
    assert!(first.run_root.exists());
    abort_prepared_isolated_agent_run(&identity, &first.run_id).unwrap();
    assert!(!first.run_root.exists());

    let second =
        prepare_isolated_agent_run(&profile, &identity, "test", Path::new("/bin/sh")).unwrap();
    assert_ne!(first.run_id, second.run_id);
    abort_prepared_isolated_agent_run(&identity, &second.run_id).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn seatbelt_denies_sibling_markers_and_nested_children_across_fresh_runs() {
    let operator = tempfile::tempdir().unwrap();
    let outside = operator.path().join("outside.txt");
    let outside_write = operator.path().join("outside-write.txt");
    fs::write(&outside, "OUTSIDE-TOKEN").unwrap();
    assert!(outside.metadata().unwrap().is_file());

    let profile = FilesystemIsolationProfile::Ephemeral {
        read_only_roots: Vec::new(),
    };
    let mut previous_root = None;
    for _ in 0..2 {
        let (mut command, mut run) =
            isolated_agent_command(&profile, &"ab".repeat(32), "test", Path::new("/bin/sh"))
                .unwrap();
        let run_root = run.root().to_path_buf();
        if let Some(previous) = &previous_root {
            assert_ne!(previous, &run_root);
        }
        let inside = run_root.join("inside.txt");
        fs::write(&inside, "INSIDE-TOKEN").unwrap();

        command
            .arg("-c")
            .arg(
                r#"
cat "$1" > "$3/inside.out"
inside_status=$?
cat "$2" > "$3/outside.out" 2>&1
outside_status=$?
printf x > "$3/inside-write.txt"
inside_write_status=$?
printf x > "$4" 2>/dev/null
outside_write_status=$?
/bin/sh -c 'cat "$1"' probe "$2" > "$3/nested.out" 2>&1
nested_status=$?
printf 'EXPLAIN\n' | /usr/bin/nc -U /private/tmp/buzz-isolation-control-v1.sock > "$3/receipt.json" 2>&1
control_status=$?
printf '%s %s %s %s %s %s' "$inside_status" "$outside_status" "$inside_write_status" "$outside_write_status" "$nested_status" "$control_status"
"#,
            )
            .arg("probe")
            .arg(&inside)
            .arg(&outside)
            .arg(&run_root)
            .arg(&outside_write)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let child = command.spawn().unwrap();
        run.bind_pid(child.id()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "status={:?} stdout={:?} stderr={:?}",
            output.status,
            output.stdout,
            output.stderr
        );
        assert_eq!(String::from_utf8_lossy(&output.stdout), "0 1 0 1 1 0");
        assert_eq!(
            fs::read_to_string(run_root.join("inside.out")).unwrap(),
            "INSIDE-TOKEN"
        );
        assert!(!fs::read_to_string(run_root.join("outside.out"))
            .unwrap()
            .contains("OUTSIDE-TOKEN"));
        assert!(!fs::read_to_string(run_root.join("nested.out"))
            .unwrap()
            .contains("OUTSIDE-TOKEN"));
        assert!(!outside_write.exists());
        let receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(run_root.join("receipt.json")).unwrap()).unwrap();
        assert_eq!(receipt["identity_pubkey"], "ab".repeat(32));
        assert_eq!(receipt["run_root"], run_root.to_string_lossy().as_ref());

        drop(run);
        assert!(!run_root.exists());
        previous_root = Some(run_root);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn bind_failure_terminates_and_reaps_spawned_harness_before_cleanup() {
    use std::os::unix::process::CommandExt;

    let profile = FilesystemIsolationProfile::Ephemeral {
        read_only_roots: Vec::new(),
    };
    let (mut command, mut run) =
        isolated_agent_command(&profile, &"ab".repeat(32), "test", Path::new("/bin/sh")).unwrap();
    let root = run.root().to_path_buf();
    command
        .arg("-c")
        .arg("while :; do :; done")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0);
    let mut child = command.spawn().unwrap();
    let pid = child.id();

    // Pre-register the same run to force the post-fsync control-registry
    // collision path inside bind_pid.
    run.control.bind_pid(pid).unwrap();
    let error = bind_isolation_process(&mut run, &mut child).unwrap_err();
    assert!(error.contains("failed to bind filesystem isolation"));
    assert!(!process_is_live(pid), "bind failure left child {pid} live");
    assert!(
        child.try_wait().unwrap().is_some(),
        "bind failure did not reap child {pid}"
    );

    drop(run);
    assert!(!root.exists(), "normal run cleanup left residue");
}

#[test]
fn startup_recovery_preserves_spawn_window_and_live_runs() {
    let operator = tempfile::tempdir().unwrap();
    let base = operator.path().join(RUNS_DIR);
    create_private_dir(&base).unwrap();
    let receipts = receipts_dir(&base).unwrap();

    let make_run = |run_id: &str,
                    desktop_pid: u32,
                    agent_pid: Option<u32>,
                    phase: Option<IsolationRunPhase>| {
        let root = base.join(format!("abababababababab-{run_id}"));
        create_private_dir(&root).unwrap();
        fs::write(root.join("residue"), "test").unwrap();
        let receipt = IsolationRunOwnership {
            version: 1,
            identity_pubkey: "ab".repeat(32),
            desktop_instance_id: "test".into(),
            run_id: run_id.into(),
            run_root: root.clone(),
            desktop_pid,
            agent_pid,
            phase,
        };
        write_ownership_receipt(&receipts.join(format!("{run_id}.json")), &receipt, true).unwrap();
        root
    };

    let abandoned = make_run(
        &"a".repeat(32),
        10,
        Some(11),
        Some(IsolationRunPhase::Bound),
    );
    let unbound_spawn_window =
        make_run(&"b".repeat(32), 10, None, Some(IsolationRunPhase::Spawning));
    let live_desktop = make_run(
        &"c".repeat(32),
        20,
        Some(21),
        Some(IsolationRunPhase::Bound),
    );
    let live_agent = make_run(
        &"d".repeat(32),
        10,
        Some(30),
        Some(IsolationRunPhase::Bound),
    );
    let dead_prepared = make_run(&"e".repeat(32), 10, None, Some(IsolationRunPhase::Prepared));
    let live_prepared = make_run(&"f".repeat(32), 20, None, Some(IsolationRunPhase::Prepared));
    let mut removed =
        recover_abandoned_isolation_runs_in(&base, |pid| pid == 20 || pid == 30).unwrap();
    removed.sort();
    let mut expected_removed = vec![abandoned.clone(), dead_prepared.clone()];
    expected_removed.sort();

    assert_eq!(removed, expected_removed);
    assert!(!abandoned.exists());
    assert!(unbound_spawn_window.exists());
    assert!(
        receipts.join(format!("{}.json", "b".repeat(32))).exists(),
        "ambiguous crash-between-spawn-and-bind receipt was removed"
    );
    assert!(live_desktop.exists());
    assert!(live_agent.exists());
    assert!(!dead_prepared.exists());
    assert!(live_prepared.exists());
}
