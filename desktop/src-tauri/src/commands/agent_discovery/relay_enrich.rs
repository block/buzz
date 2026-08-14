//! Build the relay agent directory from the union of kind:10100 and kind:30177,
//! then enrich it with kind:30177 policy and kind:39002 channel membership
//! (#4913 / #5363).
//!
//! Neither kind alone is the directory. `kind:10100` is self-published by the
//! agent (headless `buzz-acp` seats, `buzz channels set-add-policy`, hand-rolled
//! records); `kind:30177` is published by the *owner's* Desktop for the agents it
//! manages. An agent can have either without the other, so seeding from just one
//! silently drops a whole deployment shape: 10100-only for headless seats,
//! 30177-only for a Desktop-managed agent that never self-published.

use std::collections::{HashMap, HashSet};

use buzz_core_pkg::kind::KIND_MANAGED_AGENT;

use crate::{
    app_state::AppState,
    managed_agents::{agent_events::managed_agent_content_from_event, RelayAgentInfo},
    nostr_convert,
    relay::query_relay,
};

/// Relay HTTP `/query` default page size when `limit` is omitted.
const RELAY_DEFAULT_QUERY_LIMIT: usize = 100;
/// Relay hard cap for explicit `limit` (`DEFAULT_MAX_PAGE_LIMIT` in buzz-db).
const RELAY_MAX_QUERY_LIMIT: usize = 1000;

/// kind:39002 returns one members-list event per channel, not per agent.
///
/// `seeded_agent_count` is the number of candidates discovered from kind:30177
/// alone. That kind carries no channel list at all, so their cardinality is
/// unknown and the query has to ask for the max page.
fn channel_membership_query_limit(agents: &[RelayAgentInfo], seeded_agent_count: usize) -> usize {
    let sparse_10100 = agents.iter().all(|agent| agent.channel_ids.is_empty());
    if sparse_10100 || seeded_agent_count > 0 {
        // kind:10100 often omits channel_ids; cardinality is unknown — request max page.
        return RELAY_MAX_QUERY_LIMIT;
    }

    // Past this point every agent came from kind:10100 with a known channel list.
    let agent_count = agents.len();

    let mut channel_ids = HashSet::new();
    for agent in agents {
        channel_ids.extend(agent.channel_ids.iter().cloned());
    }
    // One 39002 event per channel where any agent is a member.
    let channel_cardinality = channel_ids.len().max(agent_count);
    channel_cardinality
        .saturating_mul(2)
        .clamp(RELAY_DEFAULT_QUERY_LIMIT, RELAY_MAX_QUERY_LIMIT)
}

/// kind:30177 is replaceable by (author, kind, d): several events can share a d-tag.
fn managed_agent_definition_query_limit(agent_count: usize) -> usize {
    agent_count
        .saturating_mul(4)
        .clamp(RELAY_DEFAULT_QUERY_LIMIT, RELAY_MAX_QUERY_LIMIT)
}

fn d_tag_from_event(event: &nostr::Event) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let slice = tag.as_slice();
        if slice.first().map(String::as_str) == Some("d") {
            slice.get(1).filter(|value| !value.is_empty()).cloned()
        } else {
            None
        }
    })
}

fn channel_ids_by_agent_from_membership_events(
    events: &[nostr::Event],
    agent_pubkeys: &HashSet<String>,
) -> HashMap<String, Vec<String>> {
    let mut by_agent: HashMap<String, HashSet<String>> = HashMap::new();
    for event in events {
        let Some(channel_id) = d_tag_from_event(event) else {
            continue;
        };
        for tag in event.tags.iter() {
            let slice = tag.as_slice();
            if slice.first().map(String::as_str) != Some("p") {
                continue;
            }
            let Some(agent_pubkey) = slice.get(1).filter(|value| !value.is_empty()) else {
                continue;
            };
            if agent_pubkeys.contains(agent_pubkey) {
                by_agent
                    .entry(agent_pubkey.clone())
                    .or_default()
                    .insert(channel_id.clone());
            }
        }
    }

    let mut sorted: HashMap<String, Vec<String>> = HashMap::with_capacity(by_agent.len());
    for (agent_pubkey, channel_ids) in by_agent {
        let mut channel_ids: Vec<String> = channel_ids.into_iter().collect();
        channel_ids.sort();
        sorted.insert(agent_pubkey, channel_ids);
    }
    sorted
}

/// Owner-verified kind:30177 projection used to enrich — and, for agents that
/// never published a kind:10100, to seed — a directory entry.
#[derive(Debug, Clone)]
struct ManagedAgentDefinition {
    name: String,
    respond_to: crate::managed_agents::RespondTo,
    respond_to_allowlist: Vec<String>,
}

fn collect_managed_agent_definitions(
    events: &[nostr::Event],
    expected_owners: &HashMap<String, String>,
) -> HashMap<String, ManagedAgentDefinition> {
    let mut definitions: HashMap<String, (ManagedAgentDefinition, u64, String)> = HashMap::new();
    for event in events {
        let Some(agent_pubkey) = d_tag_from_event(event) else {
            tracing::warn!("list_relay_agents: skipping kind:30177 event without d-tag");
            continue;
        };
        let event_author = event.pubkey.to_hex();
        let Some(expected_owner) = expected_owners.get(&agent_pubkey) else {
            continue;
        };
        if event_author != *expected_owner {
            continue;
        }
        let Ok(content) = managed_agent_content_from_event(event) else {
            tracing::warn!(
                agent_pubkey = %agent_pubkey,
                "list_relay_agents: skipping unparsable kind:30177 content"
            );
            continue;
        };
        let created_at = event.created_at.as_secs();
        let event_id = event.id.to_hex();
        match definitions.get(&agent_pubkey) {
            Some((_, existing_created_at, existing_event_id))
                if created_at < *existing_created_at
                    || (created_at == *existing_created_at && event_id <= *existing_event_id) =>
            {
                continue;
            }
            _ => {
                definitions.insert(
                    agent_pubkey,
                    (
                        ManagedAgentDefinition {
                            name: content.name,
                            respond_to: content.respond_to,
                            respond_to_allowlist: content.respond_to_allowlist,
                        },
                        created_at,
                        event_id,
                    ),
                );
            }
        }
    }
    definitions
        .into_iter()
        .map(|(agent_pubkey, (definition, _, _))| (agent_pubkey, definition))
        .collect()
}

/// Discover every kind:30177 record on the relay.
///
/// Unlike the previous `#d`-scoped query, this is not filtered by the kind:10100
/// pubkeys — that filter is exactly what made a Desktop-managed agent with no
/// self-published kind:10100 invisible. Authorship is still verified downstream
/// in [`collect_managed_agent_definitions`], which drops any record whose author
/// is not the agent's own NIP-OA owner, so an unfiltered read cannot let a third
/// party inject a directory entry.
async fn fetch_managed_agent_events(state: &AppState) -> Vec<nostr::Event> {
    let filter = serde_json::json!({
        "kinds": [KIND_MANAGED_AGENT],
        "limit": RELAY_MAX_QUERY_LIMIT,
    });

    match query_relay(state, &[filter]).await {
        Ok(events) => {
            if events.len() >= RELAY_MAX_QUERY_LIMIT {
                tracing::warn!(
                    returned = events.len(),
                    "list_relay_agents: kind:30177 discovery hit the relay page cap; \
                     agents beyond the page are seeded only if they published a kind:10100"
                );
            }
            events
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "list_relay_agents: kind:30177 discovery failed; continuing with kind:10100 only"
            );
            Vec::new()
        }
    }
}

/// Fetch kind:30177 records for agents already known from kind:10100.
///
/// The unfiltered discovery query above shares one page across every agent in
/// the community, so on a large relay it can truncate before reaching a given
/// agent. This `#d`-scoped query keeps policy enrichment for the agents already
/// in the directory guaranteed regardless of community size — discovery only
/// ever *adds* entries, it can never cost an existing one its policy.
///
/// Deliberately not filtered by `authors`: authorship is verified in
/// [`collect_managed_agent_definitions`], so the filter would be an
/// optimization, and dropping it lets this run before the owner lookup.
async fn fetch_managed_agent_events_for_known_agents(
    state: &AppState,
    agent_pubkeys: &[String],
) -> Vec<nostr::Event> {
    if agent_pubkeys.is_empty() {
        return Vec::new();
    }

    let filter = serde_json::json!({
        "kinds": [KIND_MANAGED_AGENT],
        "#d": agent_pubkeys,
        "limit": managed_agent_definition_query_limit(agent_pubkeys.len()),
    });

    match query_relay(state, &[filter]).await {
        Ok(events) => events,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "list_relay_agents: kind:30177 enrich failed; continuing with kind:10100 only"
            );
            Vec::new()
        }
    }
}

/// Build a directory entry for an agent known only from kind:30177.
///
/// Mirrors the defaults `nostr_convert::agents_from_events` applies to a sparse
/// kind:10100 record, so a seeded entry is indistinguishable downstream from a
/// self-published one. `channel_ids` is filled in by the kind:39002 pass.
fn seed_from_definition(pubkey: &str, definition: &ManagedAgentDefinition) -> RelayAgentInfo {
    RelayAgentInfo {
        pubkey: pubkey.to_string(),
        name: definition.name.clone(),
        agent_type: "agent".to_string(),
        channels: Vec::new(),
        channel_ids: Vec::new(),
        capabilities: Vec::new(),
        status: "offline".to_string(),
        respond_to: Some(definition.respond_to),
        respond_to_allowlist: definition.respond_to_allowlist.clone(),
        // Filled by the caller from the same owner lookup that verified this record.
        owner_pubkey: None,
    }
}

async fn fetch_agent_owner_pubkeys(
    state: &AppState,
    agent_pubkeys: &[String],
) -> HashMap<String, String> {
    if agent_pubkeys.is_empty() {
        return HashMap::new();
    }

    match query_relay(
        state,
        &[serde_json::json!({
            "kinds": [0],
            "authors": agent_pubkeys,
            "limit": agent_pubkeys.len(),
        })],
    )
    .await
    {
        Ok(profile_events) => profile_events
            .into_iter()
            .filter_map(|event| {
                nostr_convert::profile_valid_oa_owner_pubkey(&event)
                    .map(|owner| (event.pubkey.to_hex(), owner))
            })
            .collect(),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "list_relay_agents: kind:0 owner lookup failed; skipping kind:30177 enrichment"
            );
            HashMap::new()
        }
    }
}

async fn fetch_channel_ids_by_agent(
    state: &AppState,
    agents: &[RelayAgentInfo],
    seeded_agent_pubkeys: &[String],
) -> Option<HashMap<String, Vec<String>>> {
    if agents.is_empty() && seeded_agent_pubkeys.is_empty() {
        return Some(HashMap::new());
    }

    let agent_pubkeys: Vec<String> = agents
        .iter()
        .map(|agent| agent.pubkey.clone())
        .chain(seeded_agent_pubkeys.iter().cloned())
        .collect();
    let agent_pubkey_set: HashSet<String> = agent_pubkeys.iter().cloned().collect();

    match query_relay(
        state,
        &[serde_json::json!({
            "kinds": [39002],
            "#p": agent_pubkeys,
            "limit": channel_membership_query_limit(agents, seeded_agent_pubkeys.len()),
        })],
    )
    .await
    {
        Ok(membership_events) => Some(channel_ids_by_agent_from_membership_events(
            &membership_events,
            &agent_pubkey_set,
        )),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "list_relay_agents: kind:39002 membership enrich failed"
            );
            None
        }
    }
}

pub(super) async fn list_relay_agents_enriched(
    state: &AppState,
    events: Vec<nostr::Event>,
) -> Result<Vec<RelayAgentInfo>, String> {
    // The convert helper returns `{"agents": [...]}`. Extract and re-deserialize
    // into the strongly-typed `Vec<RelayAgentInfo>` the frontend expects.
    let value = nostr_convert::agents_from_events(&events);
    let agents = value
        .get("agents")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let agents: Vec<RelayAgentInfo> =
        serde_json::from_value(agents).map_err(|e| format!("agent parse failed: {e}"))?;

    // kind:10100 profiles are sparse: respond_to policy lives on kind:30177 and
    // channel membership is on kind:39002. Merge both so Desktop mention
    // eligibility (#4913 / #5363) sees the same data iOS already uses.
    Ok(enrich_relay_agents_from_relay(state, agents).await)
}

async fn enrich_relay_agents_from_relay(
    state: &AppState,
    mut agents: Vec<RelayAgentInfo>,
) -> Vec<RelayAgentInfo> {
    // kind:30177 is the other half of the directory, not just an enrichment
    // source: its `d` tag names an agent that may never have published a
    // kind:10100 of its own. The scoped read guarantees policy for the agents
    // already listed; the unfiltered read discovers the ones that are missing.
    let known_pubkeys: HashSet<String> = agents.iter().map(|agent| agent.pubkey.clone()).collect();
    let known_pubkey_list: Vec<String> = known_pubkeys.iter().cloned().collect();
    let (discovered_events, scoped_events) = tokio::join!(
        fetch_managed_agent_events(state),
        fetch_managed_agent_events_for_known_agents(state, &known_pubkey_list),
    );
    let definition_events: Vec<nostr::Event> =
        discovered_events.into_iter().chain(scoped_events).collect();

    let mut seeded_pubkeys: Vec<String> = definition_events
        .iter()
        .filter_map(d_tag_from_event)
        .filter(|pubkey| !known_pubkeys.contains(pubkey))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    seeded_pubkeys.sort();

    // Owner lookup covers both halves; membership is queried for the union so a
    // seeded agent gets its channels in the same round trip.
    let owner_lookup_pubkeys: Vec<String> = known_pubkeys
        .iter()
        .cloned()
        .chain(seeded_pubkeys.iter().cloned())
        .collect();
    let (expected_owners, channel_ids_by_agent) = tokio::join!(
        fetch_agent_owner_pubkeys(state, &owner_lookup_pubkeys),
        fetch_channel_ids_by_agent(state, &agents, &seeded_pubkeys),
    );

    let definitions = collect_managed_agent_definitions(&definition_events, &expected_owners);

    // A seed survives only if `collect_managed_agent_definitions` verified its
    // author against the agent's own NIP-OA owner. An unverified `d` tag is a
    // third party's claim about someone else's agent and is dropped here.
    for pubkey in &seeded_pubkeys {
        if let Some(definition) = definitions.get(pubkey) {
            agents.push(seed_from_definition(pubkey, definition));
        }
    }

    for agent in &mut agents {
        // The NIP-OA owner was resolved to verify kind:30177 authorship; expose it
        // so the eligibility layer can answer "is the viewer this agent's owner?"
        // without a second kind:0 round trip. Absent means unresolved, not unowned.
        agent.owner_pubkey = expected_owners.get(&agent.pubkey).cloned();

        // Owner-verified kind:30177 overrides kind:10100 self-declared policy when present.
        if let Some(definition) = definitions.get(&agent.pubkey) {
            agent.respond_to = Some(definition.respond_to);
            agent.respond_to_allowlist = definition.respond_to_allowlist.clone();
        }

        if let Some(discovered_by_agent) = &channel_ids_by_agent {
            // kind:39002 is authoritative for membership; keep 10100 hints on query failure
            // or when an agent is absent from a truncated page (R5 M2).
            let existing_channel_ids = agent.channel_ids.clone();
            agent.channel_ids = discovered_by_agent
                .get(&agent.pubkey)
                .cloned()
                .unwrap_or(existing_channel_ids);
        }
    }

    agents
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_membership_event(channel_id: &str, member_pubkey: &str) -> nostr::Event {
        use nostr::{EventBuilder, Keys, Kind, Tag};
        let keys = Keys::generate();
        EventBuilder::new(Kind::Custom(39_002), "")
            .tags(vec![
                Tag::parse(["d", channel_id]).unwrap(),
                Tag::parse(["p", member_pubkey, "", "member"]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap()
    }

    fn test_managed_agent_definition_event(
        agent_pubkey: &str,
        respond_to: &str,
        author_keys: &nostr::Keys,
    ) -> nostr::Event {
        use nostr::{EventBuilder, Kind, Tag};
        let content = serde_json::json!({
            "name": "Scout",
            "parallelism": 1,
            "respond_to": respond_to,
        });
        EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), content.to_string())
            .tags(vec![Tag::parse(["d", agent_pubkey]).unwrap()])
            .sign_with_keys(author_keys)
            .unwrap()
    }

    #[test]
    fn test_d_tag_from_event_reads_first_non_empty_d_tag() {
        let event = test_membership_event("273e2bad-b694-4a0e-bc2b-aefcc7d027bb", &"a".repeat(64));
        assert_eq!(
            d_tag_from_event(&event).as_deref(),
            Some("273e2bad-b694-4a0e-bc2b-aefcc7d027bb")
        );
    }

    #[test]
    fn test_d_tag_from_event_returns_none_without_d_tag() {
        use nostr::{EventBuilder, Keys, Kind};
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(39_002), "")
            .sign_with_keys(&keys)
            .unwrap();
        assert!(d_tag_from_event(&event).is_none());
    }

    #[test]
    fn test_channel_ids_by_agent_from_membership_events_groups_by_p_tag() {
        let agent_a = "a".repeat(64);
        let agent_b = "b".repeat(64);
        let events = vec![
            test_membership_event("channel-a", &agent_a),
            test_membership_event("channel-b", &agent_b),
            test_membership_event("channel-c", &agent_a),
        ];
        let agent_pubkeys = HashSet::from([agent_a.clone(), agent_b.clone()]);
        let by_agent = channel_ids_by_agent_from_membership_events(&events, &agent_pubkeys);
        assert_eq!(
            by_agent.get(&agent_a),
            Some(&vec!["channel-a".to_string(), "channel-c".to_string()])
        );
        assert_eq!(by_agent.get(&agent_b), Some(&vec!["channel-b".to_string()]));
    }

    #[test]
    fn test_collect_managed_agent_definitions_indexes_by_d_tag() {
        use nostr::Keys;
        let agent_pubkey = "a".repeat(64);
        let owner_keys = Keys::generate();
        let events = vec![test_managed_agent_definition_event(
            &agent_pubkey,
            "anyone",
            &owner_keys,
        )];
        let expected_owners =
            HashMap::from([(agent_pubkey.clone(), owner_keys.public_key().to_hex())]);
        let definitions = collect_managed_agent_definitions(&events, &expected_owners);
        let definition = definitions.get(&agent_pubkey).unwrap();
        assert_eq!(
            definition.respond_to,
            crate::managed_agents::RespondTo::Anyone
        );
        assert!(definition.respond_to_allowlist.is_empty());
    }

    #[test]
    fn test_collect_managed_agent_definitions_prefers_newest_created_at() {
        use nostr::{EventBuilder, Kind, Tag, Timestamp};
        let agent_pubkey = "a".repeat(64);
        let owner_keys = nostr::Keys::generate();
        let older = EventBuilder::new(
            Kind::Custom(KIND_MANAGED_AGENT as u16),
            r#"{"name":"Scout","parallelism":1,"respond_to":"owner-only"}"#,
        )
        .tags(vec![Tag::parse(["d", &agent_pubkey]).unwrap()])
        .custom_created_at(Timestamp::from(100))
        .sign_with_keys(&owner_keys)
        .unwrap();
        let newer = EventBuilder::new(
            Kind::Custom(KIND_MANAGED_AGENT as u16),
            r#"{"name":"Scout","parallelism":1,"respond_to":"anyone"}"#,
        )
        .tags(vec![Tag::parse(["d", &agent_pubkey]).unwrap()])
        .custom_created_at(Timestamp::from(200))
        .sign_with_keys(&owner_keys)
        .unwrap();
        let expected_owners =
            HashMap::from([(agent_pubkey.clone(), owner_keys.public_key().to_hex())]);
        let definitions = collect_managed_agent_definitions(&[older, newer], &expected_owners);
        let definition = definitions.get(&agent_pubkey).unwrap();
        assert_eq!(
            definition.respond_to,
            crate::managed_agents::RespondTo::Anyone
        );
    }

    #[test]
    fn test_collect_managed_agent_definitions_filters_unexpected_authors() {
        use nostr::Keys;
        let agent_pubkey = "a".repeat(64);
        let owner_keys = Keys::generate();
        let spoof_keys = Keys::generate();
        let expected_owners =
            HashMap::from([(agent_pubkey.clone(), owner_keys.public_key().to_hex())]);
        let legitimate = test_managed_agent_definition_event(&agent_pubkey, "anyone", &owner_keys);
        let spoofed = test_managed_agent_definition_event(&agent_pubkey, "owner-only", &spoof_keys);
        let definitions =
            collect_managed_agent_definitions(&[legitimate, spoofed], &expected_owners);
        let definition = definitions.get(&agent_pubkey).unwrap();
        assert_eq!(
            definition.respond_to,
            crate::managed_agents::RespondTo::Anyone
        );
    }

    #[test]
    fn test_collect_managed_agent_definitions_skips_when_owner_unverified() {
        use nostr::Keys;
        let agent_pubkey = "a".repeat(64);
        let owner_keys = Keys::generate();
        let events = vec![test_managed_agent_definition_event(
            &agent_pubkey,
            "anyone",
            &owner_keys,
        )];
        let definitions = collect_managed_agent_definitions(&events, &HashMap::new());
        assert!(definitions.is_empty());
    }

    fn test_relay_agent(channel_ids: &[&str]) -> RelayAgentInfo {
        RelayAgentInfo {
            pubkey: "a".repeat(64),
            name: "Scout".to_string(),
            agent_type: "agent".to_string(),
            channels: vec![],
            channel_ids: channel_ids.iter().map(|id| (*id).to_string()).collect(),
            capabilities: vec![],
            status: "offline".to_string(),
            respond_to: None,
            respond_to_allowlist: vec![],
            owner_pubkey: None,
        }
    }

    #[test]
    fn test_channel_membership_query_limit_scales_with_channel_cardinality() {
        let agents = vec![
            test_relay_agent(&["channel-a", "channel-b", "channel-c"]),
            test_relay_agent(&["channel-d"]),
        ];
        assert_eq!(
            channel_membership_query_limit(&agents, 0),
            RELAY_DEFAULT_QUERY_LIMIT
        );
    }

    #[test]
    fn test_channel_membership_query_limit_scales_above_relay_default() {
        let channel_ids: Vec<String> = (0..51).map(|i| format!("channel-{i}")).collect();
        let channel_refs: Vec<&str> = channel_ids.iter().map(String::as_str).collect();
        let agents = vec![test_relay_agent(&channel_refs)];
        assert_eq!(channel_membership_query_limit(&agents, 0), 102);
    }

    #[test]
    fn test_channel_membership_query_limit_uses_max_page_when_10100_sparse() {
        assert_eq!(
            channel_membership_query_limit(&[test_relay_agent(&[])], 0),
            RELAY_MAX_QUERY_LIMIT
        );
    }

    #[test]
    fn test_channel_membership_query_limit_uses_max_page_for_seeded_agents() {
        // A kind:30177-seeded agent carries no channel list, so cardinality is
        // unknown even when every kind:10100 entry already has channel_ids.
        let agents = vec![test_relay_agent(&["channel-a"])];
        assert_eq!(
            channel_membership_query_limit(&agents, 1),
            RELAY_MAX_QUERY_LIMIT
        );
    }

    #[test]
    fn test_managed_agent_definition_query_limit_allows_multiple_authors_per_d_tag() {
        assert_eq!(
            managed_agent_definition_query_limit(5),
            RELAY_DEFAULT_QUERY_LIMIT
        );
        assert_eq!(
            managed_agent_definition_query_limit(1),
            RELAY_DEFAULT_QUERY_LIMIT
        );
    }

    #[test]
    fn test_managed_agent_definition_query_limit_scales_above_relay_default() {
        assert_eq!(managed_agent_definition_query_limit(26), 104);
    }

    #[test]
    fn test_relay_agent_info_serializes_owner_pubkey_as_snake_case() {
        // The Tauri payload contract is snake_case; `fromRawRelayAgent` in
        // desktop/src/shared/api/tauri.ts does the camelCase mapping. A rename
        // here would silently land `undefined` on the TS side.
        let mut agent = test_relay_agent(&[]);
        agent.owner_pubkey = Some("d".repeat(64));
        let value = serde_json::to_value(&agent).unwrap();

        assert_eq!(
            value.get("owner_pubkey").and_then(|v| v.as_str()),
            Some("d".repeat(64).as_str())
        );
        assert!(value.get("ownerPubkey").is_none());
    }

    #[test]
    fn test_seed_from_definition_mirrors_sparse_10100_defaults() {
        let definition = ManagedAgentDefinition {
            name: "Scout".to_string(),
            respond_to: crate::managed_agents::RespondTo::Anyone,
            respond_to_allowlist: vec![],
        };
        let seeded = seed_from_definition(&"a".repeat(64), &definition);

        assert_eq!(seeded.pubkey, "a".repeat(64));
        assert_eq!(seeded.name, "Scout");
        assert_eq!(seeded.agent_type, "agent");
        assert_eq!(seeded.status, "offline");
        assert!(seeded.channels.is_empty());
        assert!(seeded.channel_ids.is_empty());
        assert!(seeded.capabilities.is_empty());
        assert_eq!(
            seeded.respond_to,
            Some(crate::managed_agents::RespondTo::Anyone)
        );
    }

    #[test]
    fn test_collect_managed_agent_definitions_carries_name_for_seeding() {
        let agent_pubkey = "b".repeat(64);
        let owner_keys = nostr::Keys::generate();
        let owner_pubkey = owner_keys.public_key().to_hex();
        let event = test_managed_agent_definition_event(&agent_pubkey, "anyone", &owner_keys);

        let expected_owners: HashMap<String, String> =
            [(agent_pubkey.clone(), owner_pubkey)].into_iter().collect();
        let definitions = collect_managed_agent_definitions(&[event], &expected_owners);

        let definition = definitions
            .get(&agent_pubkey)
            .expect("owner-authored definition must be kept");
        assert_eq!(definition.name, "Scout");
        assert_eq!(
            definition.respond_to,
            crate::managed_agents::RespondTo::Anyone
        );
    }

    #[test]
    fn test_collect_managed_agent_definitions_drops_non_owner_author() {
        // The seeding path reads `d` tags from an unfiltered kind:30177 query, so
        // a third party claiming another owner's agent must not survive.
        let agent_pubkey = "c".repeat(64);
        let impostor_keys = nostr::Keys::generate();
        let real_owner = nostr::Keys::generate().public_key().to_hex();
        let event = test_managed_agent_definition_event(&agent_pubkey, "anyone", &impostor_keys);

        let expected_owners: HashMap<String, String> =
            [(agent_pubkey.clone(), real_owner)].into_iter().collect();
        let definitions = collect_managed_agent_definitions(&[event], &expected_owners);

        assert!(
            !definitions.contains_key(&agent_pubkey),
            "a kind:30177 not authored by the agent's NIP-OA owner must never seed the directory"
        );
    }
}
