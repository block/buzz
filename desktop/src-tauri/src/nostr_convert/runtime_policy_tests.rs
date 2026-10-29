//! Bind availability provenance to the production runtime/policy merge.

use super::*;

fn fixture() -> (Keys, Event, Event) {
    let owner = Keys::generate();
    let agent = Keys::generate();
    let auth = buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "")
        .expect("compute ownership");
    let values: Vec<String> = serde_json::from_str(&auth).expect("parse ownership");
    let profile = EventBuilder::new(Kind::Metadata, "{}")
        .tags([Tag::parse(values).expect("ownership tag")])
        .sign_with_keys(&agent)
        .expect("sign identity");
    let policy = managed_agent_event(
        &owner,
        &agent.public_key().to_hex(),
        "Policy name",
        "allowlist",
        &["a".repeat(64)],
    );
    (agent, profile, policy)
}

fn runtime(keys: &Keys, status: Option<Value>, timestamp: u64) -> Event {
    let mut content = serde_json::json!({
        "name": "Runtime name",
        "owner_pubkey": "b".repeat(64),
        "respond_to": "anyone",
        "respond_to_allowlist": ["b".repeat(64)],
        "channels": ["Untrusted"],
        "channel_ids": ["untrusted-channel"],
        "capabilities": ["untrusted-capability"]
    });
    if let Some(status) = status {
        content["status"] = status;
    }
    EventBuilder::new(Kind::Custom(10100), content.to_string())
        .custom_created_at(nostr::Timestamp::from(timestamp))
        .sign_with_keys(keys)
        .expect("sign runtime")
}

fn assert_merge(directory: &[Event], profile: &Event, policy: &Event, status: &str) {
    let merged = relay_agents_from_directory_events(
        directory,
        std::slice::from_ref(policy),
        std::slice::from_ref(profile),
    );
    assert_eq!(merged.len(), 1);
    let agent = &merged[0];
    assert_eq!(agent.status, status);
    assert_eq!(serde_json::to_value(agent).unwrap()["status"], status);
    assert_eq!(agent.pubkey, profile.pubkey.to_hex());
    assert_eq!(agent.owner_pubkey, Some(policy.pubkey.to_hex()));
    assert_eq!(agent.name, "Policy name");
    assert_eq!(
        agent.respond_to,
        Some(crate::managed_agents::RespondTo::Allowlist)
    );
    assert_eq!(agent.respond_to_allowlist, vec!["a".repeat(64)]);
    assert!(
        agent.channel_ids.is_empty(),
        "runtime cannot grant membership"
    );
    assert!(agent.channels.is_empty());
    assert!(agent.capabilities.is_empty());
}

fn assert_known_status(status: &str) {
    let (keys, profile, policy) = fixture();
    assert_merge(
        &[runtime(&keys, Some(json!(status)), 10)],
        &profile,
        &policy,
        status,
    );
}

#[test]
fn policy_preserves_signed_online_runtime() {
    assert_known_status("online");
}

#[test]
fn policy_preserves_signed_away_runtime() {
    assert_known_status("away");
}

#[test]
fn policy_preserves_signed_offline_runtime() {
    assert_known_status("offline");
}

#[test]
fn missing_or_unrecognized_runtime_status_is_unknown() {
    let (keys, profile, policy) = fixture();
    for status in [
        None,
        Some(Value::Null),
        Some(json!(42)),
        Some(json!("busy")),
        Some(json!("unknown")),
    ] {
        let directory = runtime(&keys, status, 10);
        assert_merge(
            std::slice::from_ref(&directory),
            &profile,
            &policy,
            "unknown",
        );
        let legacy = relay_agents_from_directory_events(&[directory], &[], &[]);
        assert_eq!(legacy[0].status, "unknown", "no default offline evidence");
    }
}

#[test]
fn policy_only_has_unknown_availability() {
    let (_, profile, policy) = fixture();
    assert_merge(&[], &profile, &policy, "unknown");
}

#[test]
fn latest_runtime_without_status_does_not_revive_older_online_status() {
    let (keys, profile, policy) = fixture();
    let online = runtime(&keys, Some(json!("online")), 10);
    let missing = runtime(&keys, None, 20);
    for directory in [[online.clone(), missing.clone()], [missing, online]] {
        assert_merge(&directory, &profile, &policy, "unknown");
    }
}

#[test]
fn forged_latest_runtime_cannot_supply_or_revive_availability() {
    let (keys, profile, policy) = fixture();
    let old = runtime(&keys, Some(json!("online")), 10);
    let new = runtime(&keys, Some(json!("away")), 20);
    let mut value = serde_json::to_value(new).unwrap();
    value["content"] = json!(r#"{"status":"online"}"#);
    let forged: Event = serde_json::from_value(value).unwrap();
    assert!(forged.verify().is_err());
    assert_merge(&[old, forged], &profile, &policy, "unknown");
}
