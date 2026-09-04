//! Conversion and verification for relay-discovered agents.

use std::collections::{BTreeSet, HashMap};

use nostr::Event;

use crate::managed_agents::{agent_events::managed_agent_content_from_event, RelayAgentInfo};

use super::{agents_from_events, first_tag_value, profile_valid_oa_owner_pubkey, tags_named};

/// Collect valid agent pubkeys from kind:30177 `d` tags for follow-up relay
/// queries. Malformed tags are ignored so one hostile event cannot invalidate
/// the whole directory request.
pub fn managed_agent_pubkeys_from_events(events: &[Event]) -> std::collections::HashSet<String> {
    events
        .iter()
        .filter(|event| event.kind == nostr::Kind::Custom(30177) && event.verify().is_ok())
        .filter_map(|event| first_tag_value(event, "d"))
        .filter_map(|pubkey| nostr::PublicKey::from_hex(pubkey).ok())
        .map(|pubkey| pubkey.to_hex())
        .collect()
}

fn event_is_newer(candidate: &Event, previous: &Event) -> bool {
    candidate.created_at > previous.created_at
        || (candidate.created_at == previous.created_at && candidate.id < previous.id)
}

fn relay_agents_from_legacy_events(events: &[Event]) -> Vec<RelayAgentInfo> {
    let mut latest: HashMap<String, &Event> = HashMap::new();
    for event in events {
        let pubkey = event.pubkey.to_hex();
        if latest
            .get(&pubkey)
            .is_none_or(|previous| event_is_newer(event, previous))
        {
            latest.insert(pubkey, event);
        }
    }

    latest
        .into_values()
        .filter_map(|event| {
            if event.kind != nostr::Kind::Custom(10100) || event.verify().is_err() {
                return None;
            }
            let value = agents_from_events(std::slice::from_ref(event));
            let mut agent: RelayAgentInfo =
                serde_json::from_value(value.get("agents")?.as_array()?.first()?.clone()).ok()?;
            // The generic converter defaults missing status to offline for
            // compatibility. Discovery must retain only explicit, known runtime
            // evidence from this verified latest event, never that fallback.
            agent.status = serde_json::from_str::<serde_json::Value>(&event.content)
                .ok()
                .and_then(|content| {
                    content
                        .get("status")?
                        .as_str()
                        .filter(|status| matches!(*status, "online" | "away" | "offline"))
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| "unknown".to_string());
            // Legacy directory entries are not authenticated managed-policy
            // coordinates, so they must not drive the live 30177 watcher.
            agent.owner_pubkey = None;
            // Channel membership is authoritative only in relay-signed kind:39002.
            agent.channel_ids.clear();
            Some(agent)
        })
        .collect()
}

/// Merge self-authored kind:10100 runtime profiles with verified Desktop-managed
/// policy records. A verified managed coordinate reserves the agent identity even
/// when its current policy is malformed, so stale legacy permissions cannot win.
pub fn relay_agents_from_directory_events(
    directory_events: &[Event],
    managed_agent_events: &[Event],
    profile_events: &[Event],
) -> Vec<RelayAgentInfo> {
    let verified_policies = latest_verified_managed_policies(managed_agent_events, profile_events);
    let mut agents: HashMap<String, RelayAgentInfo> =
        relay_agents_from_legacy_events(directory_events)
            .into_iter()
            .map(|agent| (agent.pubkey.clone(), agent))
            .collect();
    for (agent_pubkey, event) in verified_policies {
        // Remove even when policy parsing fails: invalid latest policy must not
        // revive runtime permissions. Only verified runtime liveness survives
        // a valid policy overlay; ownership, permissions and membership do not.
        let runtime = agents.remove(&agent_pubkey);
        if let Some(mut agent) = relay_agent_from_managed_policy(&agent_pubkey, event) {
            if let Some(runtime) = runtime {
                agent.status = runtime.status;
            }
            agents.insert(agent_pubkey, agent);
        }
    }

    let mut agents: Vec<_> = agents.into_values().collect();
    agents.sort_by(|left, right| left.name.cmp(&right.name));
    agents
}

/// Resolve each agent's owner from its latest signed NIP-OA profile.
pub fn verified_agent_owners_from_profiles(events: &[Event]) -> HashMap<String, String> {
    let mut latest_profiles: HashMap<String, &Event> = HashMap::new();
    for profile in events {
        let agent_pubkey = profile.pubkey.to_hex();
        if latest_profiles
            .get(&agent_pubkey)
            .is_none_or(|previous| event_is_newer(profile, previous))
        {
            latest_profiles.insert(agent_pubkey, profile);
        }
    }
    latest_profiles
        .into_iter()
        .filter_map(|(agent_pubkey, profile)| {
            if profile.kind != nostr::Kind::Metadata || profile.verify().is_err() {
                return None;
            }
            profile_valid_oa_owner_pubkey(profile).map(|owner| (agent_pubkey, owner))
        })
        .collect()
}

fn latest_verified_managed_policies<'a>(
    managed_agent_events: &'a [Event],
    profile_events: &[Event],
) -> HashMap<String, &'a Event> {
    let verified_owners = verified_agent_owners_from_profiles(profile_events);

    let mut latest: HashMap<String, &'a Event> = HashMap::new();
    for event in managed_agent_events {
        let Some(agent_pubkey) = first_tag_value(event, "d") else {
            continue;
        };
        if verified_owners.get(agent_pubkey) != Some(&event.pubkey.to_hex()) {
            continue;
        }
        if latest
            .get(agent_pubkey)
            .is_none_or(|previous| event_is_newer(event, previous))
        {
            latest.insert(agent_pubkey.to_string(), event);
        }
    }
    latest
}

fn relay_agent_from_managed_policy(agent_pubkey: &str, event: &Event) -> Option<RelayAgentInfo> {
    // Check the envelope as well as the declared author. Keep invalid latest
    // coordinates reserved above so they cannot revive older legacy permissions.
    if event.kind != nostr::Kind::Custom(30177) || event.verify().is_err() {
        return None;
    }
    let content = managed_agent_content_from_event(event).ok()?;
    Some(RelayAgentInfo {
        pubkey: agent_pubkey.to_string(),
        owner_pubkey: Some(event.pubkey.to_hex()),
        name: content.name,
        agent_type: "agent".to_string(),
        channels: Vec::new(),
        channel_ids: Vec::new(),
        capabilities: Vec::new(),
        // Ownership/policy proves discovery, not conversational liveness.
        status: "unknown".to_string(),
        respond_to: Some(content.respond_to),
        respond_to_allowlist: content.respond_to_allowlist,
    })
}

/// Build the relay agent directory from owner-authenticated managed-agent
/// records. A kind:30177 event is accepted only when its author matches the
/// owner cryptographically declared by the agent's latest kind:0 NIP-OA tag.
pub fn relay_agents_from_managed_agent_events(
    managed_agent_events: &[Event],
    profile_events: &[Event],
) -> Vec<RelayAgentInfo> {
    let mut agents: Vec<_> = latest_verified_managed_policies(managed_agent_events, profile_events)
        .into_iter()
        .filter_map(|(agent_pubkey, event)| relay_agent_from_managed_policy(&agent_pubkey, event))
        .collect();
    agents.sort_by(|left, right| left.name.cmp(&right.name));
    agents
}

/// Build a pubkey-to-channel-id candidate map from relay-signed membership
/// events. Known agent identities need not have the cosmetic `bot` role;
/// otherwise only explicit bot tags seed discovery.
pub fn member_agent_channel_ids_from_events(
    events: &[Event],
    relay_pubkey: &str,
    known_agent_pubkeys: &std::collections::HashSet<String>,
) -> HashMap<String, Vec<String>> {
    let mut latest: HashMap<String, &Event> = HashMap::new();
    for event in events {
        if event.kind != nostr::Kind::Custom(39002)
            || !event.pubkey.to_hex().eq_ignore_ascii_case(relay_pubkey)
            || event.verify().is_err()
        {
            continue;
        }
        let Some(channel_id) = first_tag_value(event, "d") else {
            continue;
        };
        if latest
            .get(channel_id)
            .is_none_or(|previous| event_is_newer(event, previous))
        {
            latest.insert(channel_id.to_string(), event);
        }
    }
    let mut channel_ids: HashMap<String, BTreeSet<String>> = HashMap::new();
    for (channel_id, event) in latest {
        for tag in tags_named(event, "p") {
            let Some(pubkey) = tag
                .get(1)
                .and_then(|key| nostr::PublicKey::from_hex(key).ok())
            else {
                continue;
            };
            let pubkey = pubkey.to_hex();
            if tag.get(3).map(String::as_str) != Some("bot")
                && !known_agent_pubkeys.contains(&pubkey)
            {
                continue;
            }
            channel_ids
                .entry(pubkey)
                .or_default()
                .insert(channel_id.to_string());
        }
    }

    channel_ids
        .into_iter()
        .map(|(pubkey, ids)| (pubkey, ids.into_iter().collect()))
        .collect()
}
