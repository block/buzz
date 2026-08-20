use super::*;

fn provider_record(deployed: bool) -> ManagedAgentRecord {
    let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
        "pubkey": "agent", "name": "Agent", "relay_url": "", "acp_command": "",
        "agent_command": "", "agent_args": [], "mcp_command": "",
        "turn_timeout_seconds": 0, "system_prompt": null, "created_at": "",
        "updated_at": "", "last_started_at": null, "last_stopped_at": null,
        "last_exit_code": null, "last_error": null
    }))
    .unwrap();
    record.backend = crate::managed_agents::BackendKind::Provider {
        id: "provider".into(),
        config: serde_json::json!({}),
    };
    record.backend_agent_id = deployed.then(|| "deployment".to_string());
    record
}

#[test]
fn deployed_provider_rejects_access_edits_that_cannot_be_revoked() {
    let error = ensure_access_policy_change_supported(&provider_record(true), true)
        .expect_err("deployed provider access edit must fail closed");
    assert!(error.contains("no explicit stop or revocation acknowledgement"));
}

#[test]
fn undeployed_provider_accepts_access_edits() {
    ensure_access_policy_change_supported(&provider_record(false), true)
        .expect("no running provider deployment can retain stale access");
}

#[test]
fn stopped_local_agent_migrates_without_changing_identity() {
    let mut record = provider_record(false);
    record.backend = crate::managed_agents::BackendKind::Local;
    record.private_key_nsec = "nsec1identity".to_string();
    record.start_on_app_launch = true;
    let pubkey = record.pubkey.clone();
    let private_key = record.private_key_nsec.clone();
    let requested = crate::managed_agents::BackendKind::Provider {
        id: "kubernetes".to_string(),
        config: serde_json::json!({"namespace": "buzz-agents"}),
    };

    assert!(apply_backend_update(
        &mut record,
        Some(&requested),
        Some("/trusted/buzz-backend-kubernetes"),
        false,
    )
    .expect("stopped local migration should succeed"));
    assert_eq!(record.backend, requested);
    assert_eq!(record.pubkey, pubkey);
    assert_eq!(record.private_key_nsec, private_key);
    assert_eq!(record.backend_agent_id, None);
    assert_eq!(
        record.provider_binary_path.as_deref(),
        Some("/trusted/buzz-backend-kubernetes")
    );
    assert!(!record.provider_policy_pending);
    assert!(!record.start_on_app_launch);
}

#[test]
fn active_local_agent_cannot_migrate() {
    let mut record = provider_record(false);
    record.backend = crate::managed_agents::BackendKind::Local;
    let requested = crate::managed_agents::BackendKind::Provider {
        id: "kubernetes".to_string(),
        config: serde_json::json!({}),
    };

    let error = apply_backend_update(
        &mut record,
        Some(&requested),
        Some("/trusted/buzz-backend-kubernetes"),
        true,
    )
    .expect_err("active local migration must fail closed");
    assert!(error.contains("Stop this agent"));
    assert_eq!(record.backend, crate::managed_agents::BackendKind::Local);
}

#[test]
fn provider_backends_cannot_be_moved_by_generic_update() {
    let mut record = provider_record(false);
    let requested = crate::managed_agents::BackendKind::Local;
    let error = apply_backend_update(&mut record, Some(&requested), None, false)
        .expect_err("provider reversal needs an explicit lifecycle protocol");
    assert!(error.contains("not supported"));
}
