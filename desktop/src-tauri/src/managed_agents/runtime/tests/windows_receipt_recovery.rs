#![cfg(windows)]

#[test]
fn stale_exact_pair_receipt_is_durably_retired_without_pid_authority() {
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://relay.example")
            .unwrap();
    let mut receipt = super::receipt_fixture(key.clone());
    receipt.pid = u32::MAX;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(format!("{}.json", key.runtime_id()));
    std::fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();

    assert!(super::super::persisted_windows_receipt_matches(
        &path, &receipt, &key
    ));
    assert!(super::super::persisted_windows_receipt_can_retire(&receipt).unwrap());
    crate::managed_agents::remove_agent_runtime_receipt_path(&path).unwrap();
    assert!(!path.exists());
    let tombstone = crate::managed_agents::receipt_deletion_tombstone(&path);
    assert!(!tombstone.exists());
    std::fs::write(&tombstone, b"retired-receipt").unwrap();
    crate::managed_agents::remove_agent_runtime_receipt_path(&path).unwrap();
    assert!(!tombstone.exists());
}

#[test]
fn live_legacy_receipt_fails_closed_without_pid_kill_authority() {
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("bb".repeat(32), "wss://relay.example")
            .unwrap();
    let mut receipt = super::receipt_fixture(key);
    receipt.pid = std::process::id();
    receipt.windows_job_contained = false;

    assert!(!super::super::persisted_windows_receipt_can_retire(&receipt).unwrap());
}

#[test]
fn exited_legacy_receipt_can_converge_without_pid_termination() {
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("cc".repeat(32), "wss://relay.example")
            .unwrap();
    let mut receipt = super::receipt_fixture(key);
    receipt.pid = u32::MAX;
    receipt.windows_job_contained = false;

    assert!(super::super::persisted_windows_receipt_can_retire(&receipt).unwrap());
}

#[test]
fn receipt_store_uncertainty_blocks_pair_admission_before_pid_authority() {
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("ee".repeat(32), "wss://relay.example")
            .unwrap();
    let discovery: Result<
        Vec<(
            std::path::PathBuf,
            crate::managed_agents::ManagedAgentRuntimeReceipt,
        )>,
        String,
    > = Err("receipt store indeterminate".to_string());

    let error = super::super::persisted_windows_receipt_for_pair(discovery, &key).unwrap_err();
    assert!(error.contains("receipt store indeterminate"));
}

#[test]
fn missing_containment_marker_deserializes_as_legacy() {
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("dd".repeat(32), "wss://relay.example")
            .unwrap();
    let receipt = super::receipt_fixture(key);
    let mut value = serde_json::to_value(receipt).unwrap();
    value.as_object_mut().unwrap().remove("windowsJobContained");

    let decoded: crate::managed_agents::ManagedAgentRuntimeReceipt =
        serde_json::from_value(value).unwrap();
    assert!(!decoded.windows_job_contained);
}
