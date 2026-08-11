//! Pins the kind:10100 agent-directory content contract between the
//! harness-side producer (`crates/buzz-acp/src/directory.rs`,
//! `build_directory_content`) and the desktop consumer
//! (`nostr_convert::agents_from_events`). The fixture below mirrors the
//! producer's output shape — keep the two in sync so harness-advertised
//! agents stay parseable by the discovery pipeline.
//!
//! Kept as an integration test (rather than growing the `nostr_convert`
//! in-module test suite) so the schema pin lives beside the contract it
//! guards without pushing an already-at-limit file past the size ratchet.
//! Deserialization of this JSON into `RelayAgentInfo` — including the
//! `respond_to` / `respond_to_allowlist` fields — is pinned by the
//! in-module `agents_*_for_directory_parse` tests.

use nostr::{EventBuilder, Keys, Kind};
use serde_json::Value;

const HARNESS_CONTENT: &str = r#"{"name":"Scout","agent_type":"claude-agent-acp","status":"online","respond_to":"anyone","respond_to_allowlist":[],"channels":["general","hq"],"channel_ids":["00000000-0000-0000-0000-000000000001","00000000-0000-0000-0000-000000000002"],"capabilities":[],"channel_add_policy":"anyone"}"#;

#[test]
fn agents_from_events_parses_harness_published_directory_record() {
    let keys = Keys::generate();
    let event = EventBuilder::new(Kind::Custom(10100), HARNESS_CONTENT)
        .tags([])
        .sign_with_keys(&keys)
        .expect("sign fixture event");

    let value = buzz_lib::nostr_convert::agents_from_events(std::slice::from_ref(&event));
    let agents = value
        .get("agents")
        .and_then(Value::as_array)
        .expect("agents array");
    assert_eq!(agents.len(), 1);

    let agent = &agents[0];
    // Event author is authoritative — content has no pubkey, converter adds it.
    assert_eq!(
        agent.get("pubkey").and_then(Value::as_str),
        Some(keys.public_key().to_hex().as_str())
    );
    assert_eq!(agent.get("name").and_then(Value::as_str), Some("Scout"));
    assert_eq!(
        agent.get("agent_type").and_then(Value::as_str),
        Some("claude-agent-acp")
    );
    assert_eq!(agent.get("status").and_then(Value::as_str), Some("online"));
    assert_eq!(
        agent.get("respond_to").and_then(Value::as_str),
        Some("anyone")
    );
    assert_eq!(
        agent.get("respond_to_allowlist"),
        Some(&serde_json::json!([]))
    );
    assert_eq!(
        agent.get("channels"),
        Some(&serde_json::json!(["general", "hq"]))
    );
    assert_eq!(
        agent.get("channel_ids"),
        Some(&serde_json::json!([
            "00000000-0000-0000-0000-000000000001",
            "00000000-0000-0000-0000-000000000002"
        ]))
    );
    assert_eq!(agent.get("capabilities"), Some(&serde_json::json!([])));
    assert_eq!(
        agent.get("channel_add_policy").and_then(Value::as_str),
        Some("anyone")
    );
}
