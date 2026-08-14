use std::collections::{BTreeMap, BTreeSet, HashMap};

use buzz_core_pkg::kind::{KIND_AGENT_PROFILE, KIND_MANAGED_AGENT};
use nostr::Event;

use super::{agents_from_events, first_tag_value, profile_valid_oa_owner_pubkey};
use crate::managed_agents::{agent_events::managed_agent_content_from_event, RelayAgentInfo};

/// Return the agent identities referenced by managed-agent definition events.
///
/// The `d` tag identifies the agent while the event author identifies its
/// claimed owner. Callers still need to verify that claim against the agent's
/// NIP-OA profile before trusting the definition.
pub(crate) fn managed_agent_target_pubkeys(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == KIND_MANAGED_AGENT)
        .filter_map(|event| first_tag_value(event, "d"))
        .map(str::trim)
        .filter(|pubkey| !pubkey.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Build the public agent directory from self-authored kind:10100 profiles and
/// owner-authored kind:30177 managed-agent definitions.
///
/// A managed definition is accepted only when the target agent's kind:0
/// profile contains a valid NIP-OA attestation naming the definition author as
/// its owner. This prevents another community member from publishing a forged
/// definition for someone else's agent identity.
pub(crate) fn relay_agents_from_events(
    events: &[Event],
    identity_profiles: &[Event],
) -> Vec<RelayAgentInfo> {
    let directory_events: Vec<Event> = events
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == KIND_AGENT_PROFILE)
        .cloned()
        .collect();
    let directory = agents_from_events(&directory_events);
    let mut agents: Vec<RelayAgentInfo> = directory
        .get("agents")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let mut agent_indexes: HashMap<String, usize> = agents
        .iter()
        .enumerate()
        .map(|(index, agent)| (agent.pubkey.to_ascii_lowercase(), index))
        .collect();

    let verified_owners: HashMap<String, String> = identity_profiles
        .iter()
        .filter(|event| event.kind.as_u16() == 0)
        .filter_map(|event| {
            profile_valid_oa_owner_pubkey(event)
                .map(|owner| (event.pubkey.to_hex(), owner.to_ascii_lowercase()))
        })
        .collect();

    // Parameterized replaceable events should already arrive de-duplicated,
    // but choosing the newest valid event here keeps the converter deterministic
    // with permissive or test relays.
    let mut managed_by_agent: BTreeMap<String, &Event> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.kind.as_u16() as u32 == KIND_MANAGED_AGENT)
    {
        let Some(target) = first_tag_value(event, "d")
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .map(str::to_ascii_lowercase)
        else {
            continue;
        };
        let owner = event.pubkey.to_hex();
        if verified_owners.get(&target) != Some(&owner) {
            continue;
        }
        let replace = managed_by_agent.get(&target).is_none_or(|existing| {
            (event.created_at.as_secs(), event.id.to_hex())
                > (existing.created_at.as_secs(), existing.id.to_hex())
        });
        if replace {
            managed_by_agent.insert(target, event);
        }
    }

    for (pubkey, event) in managed_by_agent {
        let Ok(content) = managed_agent_content_from_event(event) else {
            continue;
        };
        if let Some(index) = agent_indexes.get(&pubkey).copied() {
            let agent = &mut agents[index];
            if !content.name.trim().is_empty() {
                agent.name = content.name;
            }
            agent.respond_to = Some(content.respond_to);
            agent.respond_to_allowlist = content.respond_to_allowlist;
            continue;
        }

        agent_indexes.insert(pubkey.clone(), agents.len());
        agents.push(RelayAgentInfo {
            pubkey,
            name: content.name,
            agent_type: "agent".to_string(),
            channels: Vec::new(),
            channel_ids: Vec::new(),
            capabilities: Vec::new(),
            status: "offline".to_string(),
            respond_to: Some(content.respond_to),
            respond_to_allowlist: content.respond_to_allowlist,
        });
    }

    agents
}

#[cfg(test)]
mod tests {
    use buzz_core_pkg::kind::{KIND_AGENT_PROFILE, KIND_MANAGED_AGENT};
    use nostr::{Event, EventBuilder, Keys, Kind, Tag};

    use super::*;

    fn oa_profile_event_for(agent_keys: &Keys, owner_keys: &Keys, content: &str) -> Event {
        let agent_pubkey = agent_keys.public_key();
        let tag_json = buzz_sdk_pkg::nip_oa::compute_auth_tag(owner_keys, &agent_pubkey, "")
            .expect("compute auth tag");
        let tag_values: Vec<String> = serde_json::from_str(&tag_json).expect("parse auth tag json");
        let auth_tag = Tag::parse(tag_values).expect("parse auth tag");

        EventBuilder::new(Kind::Metadata, content)
            .tags(vec![auth_tag])
            .sign_with_keys(agent_keys)
            .expect("sign")
    }

    fn managed_agent_event(agent_pubkey: &str, owner_keys: &Keys, content: &str) -> Event {
        let d_tag = Tag::parse(["d", agent_pubkey]).expect("parse d tag");
        EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), content)
            .tags(vec![d_tag])
            .sign_with_keys(owner_keys)
            .expect("sign")
    }

    #[test]
    fn accepts_nip_oa_verified_managed_definition() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pubkey = agent_keys.public_key().to_hex();
        let identity_profile =
            oa_profile_event_for(&agent_keys, &owner_keys, r#"{"name":"Scout"}"#);
        let managed = managed_agent_event(
            &agent_pubkey,
            &owner_keys,
            r#"{"name":"Remote Scout","parallelism":1,"respond_to":"anyone"}"#,
        );

        assert_eq!(
            managed_agent_target_pubkeys(std::slice::from_ref(&managed)),
            vec![agent_pubkey.clone()]
        );
        let agents = relay_agents_from_events(&[managed], &[identity_profile]);

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pubkey, agent_pubkey);
        assert_eq!(agents[0].name, "Remote Scout");
        assert_eq!(
            agents[0].respond_to,
            Some(crate::managed_agents::RespondTo::Anyone)
        );
    }

    #[test]
    fn rejects_managed_definition_from_unverified_owner() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let forged_owner_keys = Keys::generate();
        let agent_pubkey = agent_keys.public_key().to_hex();
        let identity_profile = oa_profile_event_for(&agent_keys, &owner_keys, "{}");
        let forged = managed_agent_event(
            &agent_pubkey,
            &forged_owner_keys,
            r#"{"name":"Forged","parallelism":1,"respond_to":"anyone"}"#,
        );

        let agents = relay_agents_from_events(&[forged], &[identity_profile]);

        assert!(agents.is_empty());
    }

    #[test]
    fn merges_managed_access_policy_into_directory_profile() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pubkey = agent_keys.public_key().to_hex();
        let identity_profile = oa_profile_event_for(&agent_keys, &owner_keys, "{}");
        let directory = EventBuilder::new(
            Kind::Custom(KIND_AGENT_PROFILE as u16),
            r#"{"name":"Old name","channel_ids":["general"],"respond_to":"owner-only"}"#,
        )
        .sign_with_keys(&agent_keys)
        .expect("sign");
        let managed = managed_agent_event(
            &agent_pubkey,
            &owner_keys,
            r#"{"name":"Current name","parallelism":1,"respond_to":"allowlist","respond_to_allowlist":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#,
        );

        let agents = relay_agents_from_events(&[directory, managed], &[identity_profile]);

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "Current name");
        assert_eq!(agents[0].channel_ids, vec!["general"]);
        assert_eq!(
            agents[0].respond_to,
            Some(crate::managed_agents::RespondTo::Allowlist)
        );
        assert_eq!(agents[0].respond_to_allowlist, vec!["a".repeat(64)]);
    }
}
