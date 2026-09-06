use super::*;
use crate::managed_agents::{
    device_policy::{model::DeviceAgentPolicy, sync},
    retention::{get_pending_sync, open_retention_db},
};

fn record() -> ManagedAgentRecord {
    serde_json::from_value(serde_json::json!({
        "pubkey": nostr::Keys::generate().public_key().to_hex(),
        "name": "Laptop Agent", "relay_url": "https://relay.example",
        "acp_command": "buzz-acp", "agent_command": "goose", "agent_args": [],
        "mcp_command": "", "turn_timeout_seconds": 320, "system_prompt": "Test",
        "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z"
    }))
    .unwrap()
}

fn policy() -> DeviceAgentPolicy {
    DeviceAgentPolicy {
        unique_names: true,
        ..Default::default()
    }
}

#[test]
fn deleting_preexisting_local_identity_leaves_both_effects_eligible_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retention.db");
    let keys = nostr::Keys::generate();
    let record = record();
    let conn = open_retention_db(&path).unwrap();
    assert!(sync::registered(&conn).unwrap().is_empty());
    // The deletion command uses this same retention seam before removing the record.
    let mut records = vec![record.clone()];
    super::super::run_managed_agent_deletion(
        dir.path(),
        &record.pubkey,
        &mut records,
        |record| retain_managed_agent_with_policy(&conn, &keys, record, &policy()),
        |records| {
            records.clear();
            Ok(())
        },
    )
    .unwrap();
    assert!(records.is_empty());
    drop(conn);
    tombstone_managed_agent_at(&path, &keys, &record.pubkey).unwrap();
    let conn = open_retention_db(&path).unwrap();
    let registered = sync::registered(&conn).unwrap();
    let pending = get_pending_sync(&conn).unwrap();
    assert_eq!(pending.len(), 2);
    for event in pending {
        assert!(sync::allows_coordinate(
            &registered,
            event.kind,
            &event.d_tag
        ));
    }
    assert!(!sync::allows_coordinate(
        &registered,
        9035,
        "unrelated-old-identity"
    ));
}

#[test]
fn registry_failure_propagates_and_retry_retains_the_local_edit() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let keys = nostr::Keys::generate();
    let record = record();
    sync::registered(&conn).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER deny_registration BEFORE INSERT ON device_local_agent_keys
        BEGIN SELECT RAISE(ABORT, 'registration blocked'); END;",
    )
    .unwrap();
    let error = retain_managed_agent_with_policy(&conn, &keys, &record, &policy()).unwrap_err();
    assert!(error.contains("registration blocked"));
    assert!(get_pending_sync(&conn).unwrap().is_empty());
    conn.execute_batch("DROP TRIGGER deny_registration")
        .unwrap();
    retain_managed_agent_with_policy(&conn, &keys, &record, &policy()).unwrap();
    assert_eq!(get_pending_sync(&conn).unwrap().len(), 1);
    assert!(sync::registered(&conn).unwrap().contains(&record.pubkey));
}

#[test]
fn failed_retention_does_not_release_an_old_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let keys = nostr::Keys::generate();
    let record = record();
    conn.execute_batch(
        "CREATE TRIGGER deny_retention BEFORE INSERT ON persona_events
        BEGIN SELECT RAISE(ABORT, 'retention blocked'); END;",
    )
    .unwrap();
    assert!(retain_managed_agent_with_policy(&conn, &keys, &record, &policy()).is_err());
    assert!(sync::registered(&conn).unwrap().is_empty());
}

#[test]
fn protected_identity_never_enrolls_for_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let keys = nostr::Keys::generate();
    let record = record();
    let mut policy = policy();
    policy.preferred_agents.push(
        crate::managed_agents::device_policy::model::PreferredAgent {
            relay_url: record.relay_url.clone(),
            owner_pubkey: keys.public_key().to_hex(),
            name: record.name.clone(),
            pubkey: record.pubkey.clone(),
            persona_id: None,
        },
    );
    assert!(retain_managed_agent_with_policy(&conn, &keys, &record, &policy).is_err());
    assert!(sync::registered(&conn).unwrap().is_empty());
    assert!(get_pending_sync(&conn).unwrap().is_empty());
}

#[test]
fn deletion_preparation_failure_preserves_the_only_local_record() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_retention_db(&dir.path().join("retention.db")).unwrap();
    let keys = nostr::Keys::generate();
    let record = record();
    let mut records = vec![record.clone()];
    sync::registered(&conn).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER deny_registration BEFORE INSERT ON device_local_agent_keys
        BEGIN SELECT RAISE(ABORT, 'registration blocked'); END;",
    )
    .unwrap();
    let result = super::super::run_managed_agent_deletion(
        dir.path(),
        &record.pubkey,
        &mut records,
        |record| retain_managed_agent_with_policy(&conn, &keys, record, &policy()),
        |_records| -> Result<(), String> { panic!("delete must not run after preparation fails") },
    );
    assert!(result.is_err());
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].pubkey, record.pubkey);
}
