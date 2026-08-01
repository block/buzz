use crate::managed_agents::{
    ManagedAgentRecord, ManagedAgentRecoveryAuthority, ManagedAgentRuntimeKey,
};

fn record(pubkey: &str) -> ManagedAgentRecord {
    serde_json::from_str(&format!(
        r#"{{
            "pubkey": "{pubkey}",
            "name": "recovery-review-test",
            "private_key_nsec": "nsec1fake",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "buzz-agent",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }}"#
    ))
    .unwrap()
}

#[test]
fn nonempty_authority_merges_divergent_legacy_pair_without_laundering_either_pair() {
    let pubkey = "7d".repeat(32);
    let pair_a = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://a.example").unwrap();
    let pair_b = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://b.example").unwrap();
    let mut authority = ManagedAgentRecoveryAuthority::default();
    authority.mark_pair(&pair_a, 7101, "pair A uncertain".into());
    let mut sidecar_projection = record(&pubkey);
    authority.project_compatibility(&mut sidecar_projection);
    authority.capture_compatibility_snapshot(&sidecar_projection);

    let mut fallback = ManagedAgentRecoveryAuthority::default();
    fallback.mark_pair(&pair_b, 7102, "pair B fallback".into());
    let mut divergent_record = record(&pubkey);
    fallback.project_compatibility(&mut divergent_record);

    assert!(!authority.accepts_compatibility_record(&divergent_record));
    authority
        .merge_compatibility_record(&divergent_record)
        .expect("divergent exact-pair fallback must merge fail-closed");
    assert!(authority.admission_error(&pair_a).is_some());
    assert!(authority.admission_error(&pair_b).is_some());
}

#[test]
fn tombstone_generation_does_not_accept_replayed_identical_pid_and_detail() {
    let pubkey = "7e".repeat(32);
    let pair = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://replay.example").unwrap();
    let mut first = ManagedAgentRecoveryAuthority::default();
    first.mark_pair(&pair, 7201, "same detail".into());
    let mut first_record = record(&pubkey);
    first.project_compatibility(&mut first_record);
    first.capture_compatibility_snapshot(&first_record);
    assert!(first.clear_pair_with_terminal_proof(&pair));

    let mut later = ManagedAgentRecoveryAuthority::default();
    later.mark_pair(&pair, 7201, "same detail".into());
    let mut later_record = record(&pubkey);
    later.project_compatibility(&mut later_record);

    assert_ne!(first_record.last_error, later_record.last_error);
    assert!(!first.accepts_compatibility_record(&later_record));
}

#[test]
fn recovery_runtime_key_rejects_oversized_relay_url() {
    let relay = format!("wss://relay.example/{}", "a".repeat(5000));
    assert!(ManagedAgentRuntimeKey::new("7f".repeat(32), &relay).is_err());
}

#[test]
fn terminal_proof_pending_clear_marker_is_exact_pair_scoped() {
    let mut record = record(&"aa".repeat(32));
    let exact =
        ManagedAgentRuntimeKey::new(record.pubkey.clone(), "wss://terminal.example").unwrap();
    let sibling =
        ManagedAgentRuntimeKey::new(record.pubkey.clone(), "wss://sibling.example").unwrap();
    crate::managed_agents::record_terminal_proof_pending_recovery_clear(
        &mut record,
        &exact,
        7301,
        "synthetic clear failure",
    );

    let pending = crate::managed_agents::terminal_proof_pending_recovery_clears(&record);
    assert_eq!(pending, vec![exact]);
    assert!(!pending.contains(&sibling));
}

#[test]
fn duplicate_top_level_authority_keys_are_rejected_before_map_collapse() {
    let pubkey = "ab".repeat(32);
    let key = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://duplicate.example").unwrap();
    let mut authority = ManagedAgentRecoveryAuthority::default();
    authority.mark_pair(&key, 7401, "duplicate authority fixture".into());
    let authority = serde_json::to_string(&authority).unwrap();
    let payload = format!(
        r#"{{"version":2,"authorities":{{"{pubkey}":{authority},"{pubkey}":{authority}}}}}"#
    );

    assert!(
        serde_json::from_str::<crate::managed_agents::ManagedAgentRecoveryStore>(&payload).is_err()
    );
}

#[test]
fn restore_admission_does_not_replace_exact_recovery_projection() {
    let mut record = record(&"ac".repeat(32));
    let key = ManagedAgentRuntimeKey::new(record.pubkey.clone(), "wss://restore.example").unwrap();
    let mut authority = ManagedAgentRecoveryAuthority::default();
    authority.mark_pair(&key, 9301, "restore uncertainty".into());
    authority.project_compatibility(&mut record);
    authority.capture_compatibility_snapshot(&record);
    record.last_error_code = Some(73);
    let before = record.clone();

    crate::managed_agents::record_blocked_recovery_admission(&record);
    assert_eq!(record, before);
    assert!(authority.accepts_compatibility_record(&record));
}

#[test]
fn recovery_prefixed_evidence_without_pid_quarantines() {
    let pubkey = "fa".repeat(32);
    let key = ManagedAgentRuntimeKey::new(pubkey, "wss://missing-pid.example").unwrap();
    let pair = serde_json::to_string(&key).unwrap();
    let error = format!(
        "{} generation={} pair={} pid=9401; unresolved",
        crate::managed_agents::UNVERIFIED_JOB_REAP_PREFIX,
        uuid::Uuid::new_v4(),
        pair
    );

    let authority = ManagedAgentRecoveryAuthority::from_legacy(None, Some(&error));
    assert!(authority.agent_quarantine.is_some());
    assert!(authority.admission_error(&key).is_some());
}

#[test]
fn empty_tombstone_cannot_launder_missing_pid_recovery_evidence() {
    let pubkey = "fb".repeat(32);
    let key = ManagedAgentRuntimeKey::new(pubkey.clone(), "wss://tombstone.example").unwrap();
    let pair = serde_json::to_string(&key).unwrap();
    let mut current_record = record(&pubkey);
    current_record.runtime_pid = None;
    current_record.last_error = Some(format!(
        "{} generation={} pair={} pid=9501; unresolved",
        crate::managed_agents::UNVERIFIED_JOB_REAP_PREFIX,
        uuid::Uuid::new_v4(),
        pair
    ));
    let clean_record = record(&pubkey);
    let mut tombstone = ManagedAgentRecoveryAuthority::default();
    tombstone.capture_compatibility_snapshot(&clean_record);

    tombstone
        .merge_compatibility_record(&current_record)
        .unwrap();
    tombstone.project_compatibility(&mut current_record);

    assert!(tombstone.agent_quarantine.is_some());
    assert!(tombstone.admission_error(&key).is_some());
    assert!(current_record
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("unresolved")));
}
