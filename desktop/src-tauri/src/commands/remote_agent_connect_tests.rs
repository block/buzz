use super::*;

const AGENT_HEX: &str = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
const AGENT_NPUB: &str = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

fn sample_record() -> ConnectedAgentRecord {
    connected_record(
        "workstation",
        AGENT_HEX,
        "Scout",
        Some("claude".to_string()),
        Some("wss://community.example".to_string()),
        "2026-07-28T00:00:00Z",
    )
}

#[test]
fn npub_and_hex_normalize_to_the_same_stored_form() {
    // Both are forms a user legitimately has on hand. If they normalized
    // differently, connecting the same agent twice — once from each form —
    // would pass the pubkey collision check and produce two records for one
    // identity.
    assert_eq!(normalize_agent_pubkey(AGENT_NPUB).unwrap(), AGENT_HEX);
    assert_eq!(normalize_agent_pubkey(AGENT_HEX).unwrap(), AGENT_HEX);
}

#[test]
fn uppercase_hex_is_normalized_rather_than_stored_verbatim() {
    let shouty = AGENT_HEX.to_uppercase();
    assert_eq!(normalize_agent_pubkey(&shouty).unwrap(), AGENT_HEX);
}

#[test]
fn surrounding_whitespace_is_tolerated() {
    // Pasted from a terminal, an npub routinely arrives with a trailing
    // newline.
    assert_eq!(
        normalize_agent_pubkey(&format!("  {AGENT_NPUB}\n")).unwrap(),
        AGENT_HEX
    );
}

#[test]
fn a_pasted_secret_key_is_refused_with_a_specific_message() {
    // The whole point of this feature is that the agent's secret stays on its
    // own machine. A user who pastes an nsec has made a serious mistake, and
    // "invalid pubkey" would not tell them what it was.
    let error =
        normalize_agent_pubkey("nsec1vl029mgpspedva04g90vltkh6fvh240zqtv9k0t9af8935ke9laqsnlfe5")
            .expect_err("an nsec must never be accepted as an agent pubkey");
    assert!(
        error.contains("secret key"),
        "message must name the mistake: {error}"
    );
    assert!(
        !error.contains("nsec1vl029"),
        "the secret must not be echoed back into an error string: {error}"
    );
}

#[test]
fn malformed_pubkeys_are_refused() {
    for bad in [
        "",
        "   ",
        "not-a-key",
        "npub1truncated",
        // 63 hex chars — one short.
        "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459",
    ] {
        assert!(
            normalize_agent_pubkey(bad).is_err(),
            "expected {bad:?} to be refused"
        );
    }
}

#[test]
fn names_are_trimmed_and_bounded() {
    assert_eq!(validate_connected_name("  Scout  ").unwrap(), "Scout");
    assert!(validate_connected_name("").is_err());
    assert!(validate_connected_name("   ").is_err());
    assert!(validate_connected_name(&"n".repeat(65)).is_err());
    assert!(validate_connected_name(&"n".repeat(64)).is_ok());
    // A newline in a name would break every single-line list rendering it.
    assert!(validate_connected_name("Sco\nut").is_err());
}

#[test]
fn a_connected_record_stores_the_identity_and_the_host_and_nothing_else() {
    // The exhaustive field set, asserted against the serialized form rather
    // than field-by-field. Under the previous design each absent capability
    // needed its own assertion (`private_key_nsec` empty, `agent_command`
    // blank, `start_on_app_launch` false, `runtime_pid` none) because the
    // fields existed and merely held harmless values. They no longer exist, so
    // the honest test is that the shape itself cannot express them: anyone
    // widening this type to carry a key or a command breaks this test.
    let record = sample_record();
    let json = serde_json::to_value(&record).unwrap();
    let mut keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();

    assert_eq!(
        keys,
        [
            "community",
            "created_at",
            "harness",
            "host",
            "name",
            "pubkey",
            "updated_at"
        ]
    );
    assert_eq!(record.pubkey, AGENT_HEX);
    assert_eq!(record.host, "workstation");
    assert_eq!(record.harness.as_deref(), Some("claude"));
}

#[test]
fn a_probeless_connect_stores_no_harness_key_at_all() {
    // `harness` is an observation, so "not observed" must be representable.
    // `skip_serializing_if` keeps it out of the file rather than writing null,
    // which keeps the stored shape honest about what was actually seen.
    let record = connected_record(
        "workstation",
        AGENT_HEX,
        "Scout",
        None,
        None,
        "2026-07-28T00:00:00Z",
    );
    let json = serde_json::to_value(&record).unwrap();
    assert!(!json.as_object().unwrap().contains_key("harness"));
}

#[test]
fn a_connected_record_round_trips_through_the_store_format() {
    let record = sample_record();
    let json = serde_json::to_string(&record).unwrap();
    let restored: ConnectedAgentRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, record);
    assert_eq!(
        restored.host, "workstation",
        "the host must survive a store write/read cycle — it is the probe target"
    );
}

#[test]
fn the_summary_projection_omits_lifecycle_and_secrets() {
    // `ConnectedAgentSummary` is intentionally narrower than
    // `ManagedAgentSummary`. Serialize it and assert the absent fields stay
    // absent: a later widening that reintroduces `status` or `pid` would give
    // the UI something to render a start button from.
    let record = sample_record();
    let summary = ConnectedAgentSummary::from(&record);
    let json = serde_json::to_value(&summary).unwrap();
    let object = json.as_object().unwrap();

    assert_eq!(object.get("host").unwrap(), "workstation");
    assert_eq!(object.get("harness").unwrap(), "claude");
    assert_eq!(object.get("pubkey").unwrap(), AGENT_HEX);
    for absent in [
        "status",
        "pid",
        "logPath",
        "log_path",
        "needsRestart",
        "startOnAppLaunch",
        "privateKeyNsec",
        "private_key_nsec",
        "relayUrl",
    ] {
        assert!(
            !object.contains_key(absent),
            "{absent} must not reach the connected-agent surface"
        );
    }
}

#[test]
fn the_summary_projection_is_total() {
    // The custody-field version of this projection read the host out of an
    // `Option` and fell back to an empty string for a record that arrived under
    // local custody — a case that could only happen if a caller's filtering was
    // wrong. With a dedicated type there is no such case and no fallback, so an
    // empty host in the UI can now only mean an empty host on disk.
    let record = sample_record();
    assert_eq!(ConnectedAgentSummary::from(&record).host, record.host);
}

#[test]
fn an_unknown_ssh_host_is_refused_with_the_fix_named() {
    // The host is a probe target. Accepting a free-form string would create a
    // row whose reachability can never be reported, which reads as a broken
    // feature rather than a missing config entry.
    let error = resolve_connect_host("definitely-not-in-any-ssh-config-xyzzy")
        .expect_err("an unknown alias must be refused");
    assert!(
        error.contains("~/.ssh/config"),
        "the message must name the fix: {error}"
    );
}

#[test]
fn a_blank_host_is_refused() {
    assert!(resolve_connect_host("").is_err());
    assert!(resolve_connect_host("   ").is_err());
}
