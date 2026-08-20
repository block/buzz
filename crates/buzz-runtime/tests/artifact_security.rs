use buzz_runtime::{
    canonicalize_workspace, canonicalize_workspace_roots, job_attempt_dir, read_runtime_receipt,
    runner_receipt_health, write_runtime_receipt, ManagedAgentRuntimeKey, RunnerReceiptHealth,
    RuntimeReceipt, SecretToken, CONTROL_PROTOCOL_VERSION, RUNTIME_RECEIPT_SCHEMA_VERSION,
};
use chrono::Utc;
use uuid::Uuid;

fn receipt() -> RuntimeReceipt {
    RuntimeReceipt {
        schema_version: RUNTIME_RECEIPT_SCHEMA_VERSION,
        key: ManagedAgentRuntimeKey {
            pubkey: "ab".repeat(32),
            relay_url: "wss://relay.example".into(),
        },
        runtime_id: "runtime".into(),
        pid: std::process::id(),
        process_start_marker: "marker".into(),
        generation: Uuid::new_v4(),
        control_addr: "127.0.0.1:12345".parse().unwrap(),
        controller_token: SecretToken::new("a".repeat(64)),
        model_token: SecretToken::new("b".repeat(64)),
        started_at: Utc::now(),
        protocol_version: CONTROL_PROTOCOL_VERSION,
        lock_protocol_version: 1,
        lock_path_hash: "cd".repeat(32),
        ready: true,
    }
}

#[test]
fn receipt_round_trips_owner_only_and_debug_redacts_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state").join("receipt.json");
    let original = receipt();
    write_runtime_receipt(&path, &original).unwrap();
    assert_eq!(read_runtime_receipt(&path).unwrap(), original);
    let debug = format!("{original:?}");
    assert!(!debug.contains(&"a".repeat(64)));
    assert!(!debug.contains(&"b".repeat(64)));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}

#[test]
fn job_layout_has_only_fixed_and_typed_components() {
    let root = std::path::Path::new("/safe/runtime");
    let job = Uuid::new_v4();
    let path = job_attempt_dir(root, job, 7).unwrap();
    assert_eq!(
        path,
        root.join("jobs")
            .join(job.hyphenated().to_string())
            .join("7")
    );
    assert!(!path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir)));
    assert!(job_attempt_dir(root, job, 0).is_err());
    assert_eq!(
        runner_receipt_health(root, job, 7),
        RunnerReceiptHealth::Missing
    );
}

#[cfg(unix)]
#[test]
fn canonical_workspace_rejects_symlink_escape() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    let outside = directory.path().join("outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    symlink(&outside, root.join("escape")).unwrap();
    let roots = canonicalize_workspace_roots(vec![root]).unwrap();
    assert!(canonicalize_workspace(&roots[0].join("escape"), &roots).is_err());
}
