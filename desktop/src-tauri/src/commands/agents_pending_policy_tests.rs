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
fn selective_flush_recovers_failed_tombstone_after_disk_removal() {
    use crate::managed_agents::retention::get_retained_event;
    use nostr::JsonUtil;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retention.db");
    let store = dir.path().join("managed-agents.json");
    let keys = nostr::Keys::generate();
    let mut record = record();
    record.persona_id = Some("definition-before-delete".into());
    let conn = open_retention_db(&path).unwrap();
    let mut records = vec![record.clone()];
    std::fs::write(&store, serde_json::to_vec(&records).unwrap()).unwrap();
    super::super::run_managed_agent_deletion(
        dir.path(),
        &record.pubkey,
        &mut records,
        |record| retain_managed_agent_with_policy(&conn, &keys, record, &policy()),
        |records| {
            records.clear();
            std::fs::write(&store, serde_json::to_vec(records).unwrap()).map_err(|e| e.to_string())
        },
    )
    .unwrap();
    // Fail AFTER the actual production deletion seam removes the disk record.
    conn.execute_batch("CREATE TRIGGER deny_archive BEFORE INSERT ON persona_events WHEN NEW.kind = 9035 BEGIN SELECT RAISE(ABORT, 'archive blocked'); END;").unwrap();
    assert!(tombstone_managed_agent_at(&path, &keys, &record.pubkey).is_err());
    conn.execute_batch("DROP TRIGGER deny_archive").unwrap();
    drop(conn);
    // Reopen through the same recovery boundary used before selective publish.
    let registered = local_keys_for_flush_at(&path, &keys, &store, &policy()).unwrap();
    let conn = open_retention_db(&path).unwrap();
    let pending = get_pending_sync(&conn).unwrap();
    assert!(
        get_retained_event(&conn, 30177, &keys.public_key().to_hex(), &record.pubkey)
            .unwrap()
            .is_none(),
        "deleted head must not be republished"
    );
    let mut kinds: Vec<_> = pending.iter().map(|row| row.kind).collect();
    kinds.sort();
    assert_eq!(kinds, vec![5, 9035]);
    for row in &pending {
        assert!(sync::allows_coordinate(&registered, row.kind, &row.d_tag));
        nostr::Event::from_json(&row.raw_event)
            .unwrap()
            .verify()
            .unwrap();
    }
    assert!(pending
        .iter()
        .find(|row| row.kind == 9035)
        .unwrap()
        .content
        .contains("definition-before-delete"));
    let before: Vec<_> = pending.iter().map(|row| row.raw_event.clone()).collect();
    local_keys_for_flush_at(&path, &keys, &store, &policy()).unwrap();
    assert_eq!(
        get_pending_sync(&conn)
            .unwrap()
            .iter()
            .map(|row| row.raw_event.clone())
            .collect::<Vec<_>>(),
        before,
        "successful deletes must not be re-enqueued on every restart"
    );
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
fn recovery_never_infers_deletion_from_missing_or_corrupt_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retention.db");
    let store = dir.path().join("managed-agents.json");
    let keys = nostr::Keys::generate();
    let record = record();
    let conn = open_retention_db(&path).unwrap();
    retain_managed_agent_with_policy(&conn, &keys, &record, &policy()).unwrap();
    assert!(local_keys_for_flush_at(&path, &keys, &store, &policy()).is_err());
    for bytes in ["broken", "null", "{}", "[{\"pubkey\":42}]"] {
        std::fs::write(&store, bytes).unwrap();
        assert!(local_keys_for_flush_at(&path, &keys, &store, &policy()).is_err());
        assert_eq!(get_pending_sync(&conn).unwrap().len(), 1);
        assert_eq!(get_pending_sync(&conn).unwrap()[0].kind, 30177);
    }
}

#[test]
fn recovery_leaves_live_remote_only_and_protected_heads_untouched() {
    use crate::managed_agents::reconcile::retain_agent_record;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("retention.db");
    let store = dir.path().join("managed-agents.json");
    let keys = nostr::Keys::generate();
    let live = record();
    let remote = record();
    let protected = record();
    let conn = open_retention_db(&path).unwrap();
    retain_managed_agent_with_policy(&conn, &keys, &live, &policy()).unwrap();
    retain_managed_agent_with_policy(&conn, &keys, &protected, &policy()).unwrap();
    retain_agent_record(&conn, &keys, &remote).unwrap();
    std::fs::write(&store, serde_json::to_vec(&[live]).unwrap()).unwrap();
    let mut policy = policy();
    policy.preferred_agents.push(
        crate::managed_agents::device_policy::model::PreferredAgent {
            relay_url: "https://relay.example".into(),
            owner_pubkey: keys.public_key().to_hex(),
            name: "protected".into(),
            pubkey: protected.pubkey,
            persona_id: None,
        },
    );
    local_keys_for_flush_at(&path, &keys, &store, &policy).unwrap();
    let pending = get_pending_sync(&conn).unwrap();
    assert_eq!(pending.len(), 3);
    assert!(pending.iter().all(|row| row.kind == 30177));
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
