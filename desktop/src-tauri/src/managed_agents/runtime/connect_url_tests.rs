//! Connection-URL persistence tests: receipt back-compat and validation,
//! restart-target selection, and the pair-reuse conflict guard.

use super::test_fixtures::{make_pair_runtime_with_connect_url, receipt_fixture};

#[test]
fn receipt_without_connect_url_deserializes_and_validates() {
    // Receipts persisted before `connectRelayUrl` existed must keep loading
    // (field absent -> None) and keep validating.
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3100")
            .unwrap();
    let json = format!(
        r#"{{"key":{{"pubkey":"{}","relayUrl":"{}"}},"pid":{},"desktopInstanceId":"test-instance","startedAt":"now"}}"#,
        key.pubkey,
        key.relay_url,
        std::process::id(),
    );
    let receipt: crate::managed_agents::ManagedAgentRuntimeReceipt =
        serde_json::from_str(&json).expect("pre-connect-url receipt must deserialize");
    assert_eq!(receipt.connect_relay_url, None);

    let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));
    assert!(super::valid_agent_runtime_receipt_with(
        &path,
        &receipt,
        "test-instance",
        |_| true,
        |_, _| true,
    ));
}

#[test]
fn receipt_connect_url_matching_pair_validates() {
    // The configured spelling (`localhost`) canonicalizes to the receipt's own
    // key (`127.0.0.1`), so the receipt is valid — this is the normal shape
    // written by every spawn on a loopback workspace.
    let mut receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3100")
            .unwrap(),
    );
    receipt.connect_relay_url = Some("ws://localhost:3100".into());
    let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));
    assert!(super::valid_agent_runtime_receipt_with(
        &path,
        &receipt,
        "test-instance",
        |_| true,
        |_, _| true,
    ));
}

#[test]
fn receipt_connect_url_foreign_pair_rejected() {
    // A connection URL that canonicalizes to a DIFFERENT pair key is a corrupt
    // or cross-wired receipt — it must fail validation, not fall back.
    let mut receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3100")
            .unwrap(),
    );
    receipt.connect_relay_url = Some("ws://localhost:4000".into());
    let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));
    assert!(!super::valid_agent_runtime_receipt_with(
        &path,
        &receipt,
        "test-instance",
        |_| true,
        |_, _| true,
    ));
}

#[test]
fn receipt_connect_url_roundtrips_through_persistence() {
    // Present field serializes (camelCase) and deserializes unchanged, so a
    // restart in a later session re-dials the exact configured spelling.
    let mut receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3100")
            .unwrap(),
    );
    receipt.connect_relay_url = Some("ws://localhost:3100".into());
    let json = serde_json::to_string(&receipt).expect("serialize receipt");
    assert!(json.contains("\"connectRelayUrl\":\"ws://localhost:3100\""));
    let restored: crate::managed_agents::ManagedAgentRuntimeReceipt =
        serde_json::from_str(&json).expect("deserialize receipt");
    assert_eq!(restored, receipt);
}

#[test]
fn restart_targets_preserve_configured_spelling_per_pair() {
    // Restart targets come from each live pair's stamped connection URL —
    // the configured loopback spelling survives (never the canonical fold),
    // and only the requested agent's pairs are selected.
    let agent_a = "aa".repeat(32);
    let agent_b = "bb".repeat(32);
    let mut runtimes = std::collections::HashMap::new();
    runtimes.insert(
        crate::managed_agents::ManagedAgentRuntimeKey::new(agent_a.clone(), "ws://localhost:3100")
            .unwrap(),
        make_pair_runtime_with_connect_url("ws://localhost:3100"),
    );
    runtimes.insert(
        crate::managed_agents::ManagedAgentRuntimeKey::new(agent_a.clone(), "wss://other.example")
            .unwrap(),
        make_pair_runtime_with_connect_url("wss://other.example"),
    );
    runtimes.insert(
        crate::managed_agents::ManagedAgentRuntimeKey::new(agent_b, "ws://localhost:9999").unwrap(),
        make_pair_runtime_with_connect_url("ws://localhost:9999"),
    );

    let mut targets = super::managed_agent_restart_targets(&runtimes, &agent_a);
    targets.sort();
    assert_eq!(
        targets,
        vec![
            "ws://localhost:3100".to_string(),
            "wss://other.example".to_string(),
        ],
    );
}

#[test]
fn pair_reuse_with_matching_spelling_is_allowed() {
    let runtime = make_pair_runtime_with_connect_url("ws://localhost:3100");
    assert!(super::ensure_pair_connection_matches(&runtime, " ws://localhost:3100 ").is_ok());
}

#[test]
fn pair_reuse_across_spellings_is_a_connection_target_conflict() {
    // localhost and 127.0.0.1 share a canonical key but are distinct tenants:
    // reuse must fail loudly instead of reporting the requested tenant started.
    let runtime = make_pair_runtime_with_connect_url("ws://127.0.0.1:3100");
    let err = super::ensure_pair_connection_matches(&runtime, "ws://localhost:3100").unwrap_err();
    assert!(err.contains("connection-target conflict"));
    // No URL disclosure: the message must not echo either spelling.
    assert!(!err.contains("3100"));
}
