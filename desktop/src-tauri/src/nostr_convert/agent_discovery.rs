use nostr::{Event, ToBech32};
use serde_json::{json, Value};

use super::profile_valid_oa_owner_pubkey;

/// Convert kind:10100 agent profile events to the agent discovery format.
pub fn agents_from_events(events: &[Event]) -> Value {
    let agents: Vec<Value> = events
        .iter()
        .filter(|event| event.kind.as_u16() == 10100)
        .map(agent_value_from_event)
        .collect();
    json!({ "agents": agents })
}

/// Convert verified NIP-OA kind:0 profiles owned by `owner_pubkey` into sparse
/// relay-agent records. Their lifecycle remains external to Desktop.
pub fn owned_agent_profiles_from_events(events: &[Event], owner_pubkey: &str) -> Value {
    let owner_pubkey = owner_pubkey.to_lowercase();
    let agents: Vec<Value> = events
        .iter()
        .filter(|event| event.kind.as_u16() == 0)
        .filter(|event| {
            profile_valid_oa_owner_pubkey(event)
                .is_some_and(|owner| owner.to_lowercase() == owner_pubkey)
        })
        .map(agent_value_from_event)
        .collect();
    json!({ "agents": agents })
}

fn agent_value_from_event(event: &Event) -> Value {
    let mut value: Value = serde_json::from_str(&event.content).unwrap_or_else(|_| json!({}));
    let pubkey = event.pubkey.to_hex();
    // Full npub fallback — truncated prefixes are grindable (see pubkey-display).
    let npub = event.pubkey.to_bech32().unwrap_or_else(|_| pubkey.clone());

    if let Some(object) = value.as_object_mut() {
        // Event authorship is authoritative even if content claims another key.
        object.insert("pubkey".to_string(), json!(pubkey.clone()));
        let fallback_name = object
            .get("display_name")
            .and_then(Value::as_str)
            .filter(|candidate| !candidate.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| npub.clone());
        if !object.get("name").is_some_and(Value::is_string) {
            object.insert("name".to_string(), json!(fallback_name));
        }
        if !object.get("agent_type").is_some_and(Value::is_string) {
            object.insert("agent_type".to_string(), json!("agent"));
        }
        if !object.get("channels").is_some_and(Value::is_array) {
            object.insert("channels".to_string(), json!([]));
        }
        if !object.get("channel_ids").is_some_and(Value::is_array) {
            object.insert("channel_ids".to_string(), json!([]));
        }
        if !object.get("capabilities").is_some_and(Value::is_array) {
            object.insert("capabilities".to_string(), json!([]));
        }
        if !object.get("status").is_some_and(Value::is_string) {
            object.insert("status".to_string(), json!("offline"));
        }
        return value;
    }

    json!({
        "pubkey": pubkey,
        "name": npub,
        "agent_type": "agent",
        "channels": [],
        "channel_ids": [],
        "capabilities": [],
        "status": "offline",
    })
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag};

    use crate::managed_agents::{RelayAgentInfo, RespondTo};

    use super::*;

    fn event(kind: u16, content: &str) -> Event {
        EventBuilder::new(Kind::from_u16(kind), content)
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    fn oa_profile_event(content: &str) -> (Event, String) {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let tag_json =
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "")
                .expect("compute auth tag");
        let tag_values: Vec<String> = serde_json::from_str(&tag_json).expect("parse auth tag");
        let auth_tag = Tag::parse(tag_values).expect("parse auth tag");
        let event = EventBuilder::new(Kind::Metadata, content)
            .tags(vec![auth_tag])
            .sign_with_keys(&agent_keys)
            .expect("sign");
        (event, owner_keys.public_key().to_hex())
    }

    #[test]
    fn overwrites_content_pubkey_with_event_author() {
        let event = event(10100, r#"{"pubkey":"forged","name":"agent-1"}"#);
        let value = agents_from_events(std::slice::from_ref(&event));
        let agents = value.get("agents").and_then(Value::as_array).unwrap();

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["pubkey"], event.pubkey.to_hex());
        assert_eq!(agents[0]["name"], "agent-1");
    }

    #[test]
    fn owned_profiles_require_verified_matching_owner() {
        let (owned, owner_pubkey) = oa_profile_event(r#"{"display_name":"External Pi"}"#);
        let (somebody_elses, _) = oa_profile_event(r#"{"display_name":"Other Pi"}"#);
        let value =
            owned_agent_profiles_from_events(&[owned.clone(), somebody_elses], &owner_pubkey);
        let agents = value.get("agents").and_then(Value::as_array).unwrap();

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["pubkey"], owned.pubkey.to_hex());
        assert_eq!(agents[0]["name"], "External Pi");
        assert_eq!(agents[0]["status"], "offline");
    }

    #[test]
    fn handles_invalid_content() {
        let event = event(10100, "not-json");
        let value = agents_from_events(std::slice::from_ref(&event));
        let agents = value.get("agents").and_then(Value::as_array).unwrap();

        assert_eq!(agents[0]["pubkey"], event.pubkey.to_hex());
    }

    #[test]
    fn defaults_sparse_directory_profiles() {
        let event = event(
            10100,
            r#"{"channel_add_policy":"owner-only","display_name":"Scout"}"#,
        );
        let value = agents_from_events(std::slice::from_ref(&event));
        let agents: Vec<RelayAgentInfo> = serde_json::from_value(value["agents"].clone()).unwrap();

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pubkey, event.pubkey.to_hex());
        assert_eq!(agents[0].name, "Scout");
        assert_eq!(agents[0].agent_type, "agent");
        assert!(agents[0].channels.is_empty());
        assert!(agents[0].capabilities.is_empty());
        assert_eq!(agents[0].status, "offline");
        assert_eq!(agents[0].respond_to, None);
    }

    #[test]
    fn preserves_public_respond_to_mode() {
        let event = event(10100, r#"{"name":"Scout","respond_to":"anyone"}"#);
        let value = agents_from_events(std::slice::from_ref(&event));
        let agents: Vec<RelayAgentInfo> = serde_json::from_value(value["agents"].clone()).unwrap();

        assert_eq!(agents[0].respond_to, Some(RespondTo::Anyone));
    }

    #[test]
    fn preserves_allowlist_metadata() {
        let event = event(
            10100,
            r#"{"name":"Scout","respond_to":"allowlist","respond_to_allowlist":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#,
        );
        let value = agents_from_events(std::slice::from_ref(&event));
        let agents: Vec<RelayAgentInfo> = serde_json::from_value(value["agents"].clone()).unwrap();

        assert_eq!(agents[0].respond_to, Some(RespondTo::Allowlist));
        assert_eq!(agents[0].respond_to_allowlist, vec!["a".repeat(64)]);
    }
}
