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
fn receipt_target_mismatch_is_rejected_before_termination_selection() {
    use std::cell::Cell;

    let mut receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3100")
            .unwrap(),
    );
    receipt.connect_relay_url = Some("ws://localhost:3100".into());
    let validated = Cell::new(false);
    let terminated = Cell::new(false);
    let error = super::terminate_runtime_receipt_for_target_with(
        std::path::Path::new("pair.json"),
        &receipt,
        "ws://127.0.0.1:3100",
        |_, _| {
            validated.set(true);
            true
        },
        |_| {
            terminated.set(true);
            Ok(())
        },
        |_| false,
        |_| {},
    )
    .unwrap_err();

    assert!(error.contains("connection-target conflict"));
    assert!(
        !error.contains("3100"),
        "error must not disclose either URL"
    );
    assert!(
        validated.get(),
        "receipt ownership must be validated before target selection"
    );
    assert!(
        !terminated.get(),
        "mismatched receipt must never be terminated"
    );
}

#[test]
fn invalid_receipt_is_ignored_without_target_selection() {
    use std::cell::Cell;

    let mut receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3100")
            .unwrap(),
    );
    receipt.connect_relay_url = Some("ws://localhost:3100".into());
    let terminated = Cell::new(false);

    super::terminate_runtime_receipt_for_target_with(
        std::path::Path::new("pair.json"),
        &receipt,
        "ws://127.0.0.1:3100",
        |_, _| false,
        |_| {
            terminated.set(true);
            Ok(())
        },
        |_| false,
        |_| {},
    )
    .expect("an invalid receipt cannot select a process");

    assert!(!terminated.get());
}

#[test]
fn legacy_receipt_uses_its_historical_canonical_dial_target() {
    use std::cell::Cell;

    let receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3100")
            .unwrap(),
    );
    let terminated = Cell::new(false);
    let error = super::terminate_runtime_receipt_for_target_with(
        std::path::Path::new("pair.json"),
        &receipt,
        "ws://localhost:3100",
        |_, _| true,
        |_| {
            terminated.set(true);
            Ok(())
        },
        |_| false,
        |_| {},
    )
    .unwrap_err();

    // Legacy children dialed key.relay_url (`127.0.0.1` after canonical
    // normalization), so a localhost request must fail closed.
    assert!(error.contains("connection-target conflict"));
    assert!(!terminated.get());
}

#[test]
fn tracked_runtime_selection_requires_the_requested_connection_target() {
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3100")
            .unwrap();
    let runtimes = std::collections::HashMap::from([(
        key.clone(),
        make_pair_runtime_with_connect_url("ws://localhost:3100"),
    )]);

    assert!(
        super::tracked_pair_runtime_for_target(&runtimes, &key, "ws://LocalHost:3100/",)
            .unwrap()
            .is_some()
    );
    let error =
        super::tracked_pair_runtime_for_target(&runtimes, &key, "ws://127.0.0.1:3100").unwrap_err();
    assert!(error.contains("connection-target conflict"));
    assert!(
        runtimes.contains_key(&key),
        "failed selection must not mutate the map"
    );
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

#[test]
fn pair_reuse_folds_connection_equivalent_spellings() {
    // Host case, explicit default port, root slash, and the FQDN root dot
    // are connection-equivalent per the tenancy authority - none of these
    // may read as a conflict after harmless config formatting drift.
    for (live, requested) in [
        ("ws://localhost:3000", "ws://LocalHost:3000"),
        ("wss://relay.example", "wss://relay.example:443"),
        ("ws://localhost:3000", "ws://localhost:3000/"),
        ("wss://relay.example", "wss://relay.example."),
        ("ws://relay.example:80/ws", "ws://Relay.Example:80/ws"),
        ("ws://relay.example?token=x", "ws://relay.example/?token=x"),
        ("ws://[::1]:3000", "ws://[0:0:0:0:0:0:0:1]:3000"),
    ] {
        let runtime = make_pair_runtime_with_connect_url(live);
        assert!(
            super::ensure_pair_connection_matches(&runtime, requested).is_ok(),
            "equivalent spellings must not conflict: {live} vs {requested}",
        );
    }
}

#[test]
fn pair_reuse_keeps_tenancy_significant_differences_conflicting() {
    // Scheme, non-default port, and path differences are real target
    // differences - and the loopback split stays a conflict (see
    // pair_reuse_across_spellings_is_a_connection_target_conflict).
    for (live, requested) in [
        ("ws://relay.example:3000", "wss://relay.example:3000"),
        ("ws://relay.example:3000", "ws://relay.example:3001"),
        ("ws://relay.example:3000/a", "ws://relay.example:3000/b"),
        ("ws://relay.example:443", "ws://relay.example"),
        ("wss://relay.example:80", "wss://relay.example"),
        ("ws://relay.example?token=x", "ws://relay.example?token=y"),
        ("ws://[::1]:3000", "ws://localhost:3000"),
        ("ws://[::1]:3000", "ws://127.0.0.1:3000"),
    ] {
        let runtime = make_pair_runtime_with_connect_url(live);
        assert!(
            super::ensure_pair_connection_matches(&runtime, requested).is_err(),
            "distinct targets must conflict: {live} vs {requested}",
        );
    }
}
