//! Relay-backed shared-agent directory discovery.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::{
    app_state::AppState,
    commands::identity_archive,
    managed_agents::RelayAgentInfo,
    nostr_convert,
    relay::{query_relay, query_relay_at_with_keys, submit_event_at_with_keys},
};

const RELAY_DIRECTORY_PAGE_SIZE: usize = 500;
const RELAY_FILTER_BATCH_SIZE: usize = 10;
/// Per-rebuild ceiling on directory-rebuild `/query` requests in flight at once.
/// The rebuild fans dozens of exact-author batches across the relay; issuing
/// them serially dominated agent-mention send latency (~6 s for ~100
/// candidates). A bounded window collapses that to a few round trips while
/// keeping the request rate well under the relay's admission gate, which
/// back-pressures any 429 anyway. Each rebuild builds one semaphore and shares
/// it across every phase, so a single rebuild's runtime-directory and
/// owner-profile phases — which run concurrently under one `try_join!` — never
/// exceed it together. (Overlapping rebuilds each hold their own budget.)
const RELAY_DIRECTORY_MAX_CONCURRENCY: usize = 8;

/// Run one `query_relay` request per `RELAY_FILTER_BATCH_SIZE` chunk of
/// `filters`, each acquiring a permit from `semaphore` so the total in-flight
/// request count stays within the shared ceiling even when several batch sets
/// run concurrently. Returned events are concatenated; order is unspecified —
/// every caller keys the events by pubkey downstream, so ordering is irrelevant.
async fn query_filter_batches(
    state: &AppState,
    semaphore: &tokio::sync::Semaphore,
    filters: &[serde_json::Value],
    error_label: &str,
) -> Result<Vec<nostr::Event>, String> {
    let pages = futures_util::future::try_join_all(filters.chunks(RELAY_FILTER_BATCH_SIZE).map(
        |batch| async move {
            let _permit = semaphore.acquire().await.map_err(|error| {
                format!("{error_label}: directory concurrency semaphore closed: {error}")
            })?;
            query_relay(state, batch)
                .await
                .map_err(|error| format!("{error_label}: {error}"))
        },
    ))
    .await?;
    Ok(pages.into_iter().flatten().collect())
}

fn exact_author_filters(pubkeys: &[String], kind: u16) -> Vec<serde_json::Value> {
    pubkeys
        .iter()
        .map(|pubkey| {
            serde_json::json!({
                "authors": [pubkey],
                "kinds": [kind],
                "limit": 1,
            })
        })
        .collect()
}

fn managed_policy_filters(
    candidate_pubkeys: &[String],
    verified_owners: &std::collections::HashMap<String, String>,
) -> Vec<serde_json::Value> {
    candidate_pubkeys
        .iter()
        .filter_map(|agent_pubkey| {
            verified_owners.get(agent_pubkey).map(|owner_pubkey| {
                serde_json::json!({
                    "authors": [owner_pubkey],
                    "kinds": [30177],
                    "#d": [agent_pubkey],
                    "limit": 1,
                })
            })
        })
        .collect()
}

fn current_user_pubkey(state: &AppState) -> Result<String, String> {
    state
        .keys
        .lock()
        .map(|keys| keys.public_key().to_hex())
        .map_err(|error| error.to_string())
}

pub(super) fn advance_relay_cursor(filter: &mut serde_json::Value, page: &[nostr::Event]) {
    let last = page
        .last()
        .expect("a full relay page always has a last event");
    filter["until"] = serde_json::json!(last.created_at.as_secs());
    filter["before_id"] = serde_json::json!(last.id.to_hex());
}

async fn query_all_relay_pages(
    state: &AppState,
    mut filter: serde_json::Value,
) -> Result<Vec<nostr::Event>, String> {
    filter["limit"] = serde_json::json!(RELAY_DIRECTORY_PAGE_SIZE);
    let mut events = Vec::new();
    loop {
        let page = query_relay(state, &[filter.clone()]).await?;
        let done = page.len() < RELAY_DIRECTORY_PAGE_SIZE;
        if !done {
            advance_relay_cursor(&mut filter, &page);
        }
        events.extend(page);
        if done {
            return Ok(events);
        }
    }
}

fn retain_agents_allowed_by_build(agents: &mut Vec<RelayAgentInfo>, require_verified_owner: bool) {
    if require_verified_owner {
        agents.retain(|agent| agent.owner_pubkey.is_some());
    }
}

pub(crate) async fn list_relay_agents_for_state(
    state: &AppState,
) -> Result<Vec<RelayAgentInfo>, String> {
    list_relay_agents_for_selection(state, None, None).await
}

async fn list_relay_agents_for_selection(
    state: &AppState,
    requested_pubkeys: Option<&std::collections::HashSet<String>>,
    channel_id: Option<&str>,
) -> Result<Vec<RelayAgentInfo>, String> {
    let viewer_pubkey = current_user_pubkey(state)?;
    let relay_pubkey = identity_archive::fetch_relay_self(state)
        .await?
        .ok_or_else(|| "relay agent membership authority is unavailable".to_string())?;

    // Membership is the authoritative and bounded candidate source. Only
    // channels visible to this identity are read, and only bot-role p-tags can
    // drive the downstream managed-policy and owner-profile lookups.
    let mut membership_filter = serde_json::json!({
        "kinds": [39002],
        "authors": [&relay_pubkey],
        "#p": [&viewer_pubkey],
    });
    if let Some(channel_id) = channel_id {
        membership_filter["#d"] = serde_json::json!([channel_id]);
    }
    let membership_events = query_all_relay_pages(state, membership_filter)
        .await
        .map_err(|error| format!("relay agent channel-membership query failed: {error}"))?;
    let mut member_agent_channel_ids =
        nostr_convert::member_agent_channel_ids_from_events(&membership_events, &relay_pubkey);
    if let Some(requested_pubkeys) = requested_pubkeys {
        member_agent_channel_ids.retain(|pubkey, _| requested_pubkeys.contains(pubkey));
    }
    let candidate_pubkeys: Vec<String> = member_agent_channel_ids.keys().cloned().collect();
    if candidate_pubkeys.is_empty() {
        return Ok(Vec::new());
    }

    let directory_filters = exact_author_filters(&candidate_pubkeys, 10100);
    let profile_filters = exact_author_filters(&candidate_pubkeys, 0);
    // One semaphore per rebuild caps `/query` requests across this rebuild's
    // phases, so its runtime-directory and owner-profile phases below stay
    // within the ceiling even though `try_join!` runs them concurrently.
    let semaphore = tokio::sync::Semaphore::new(RELAY_DIRECTORY_MAX_CONCURRENCY);
    let (directory_events, profile_events) = tokio::try_join!(
        query_filter_batches(
            state,
            &semaphore,
            &directory_filters,
            "relay agent runtime-directory query failed",
        ),
        query_filter_batches(
            state,
            &semaphore,
            &profile_filters,
            "relay agent owner-profile query failed",
        ),
    )?;

    // Only the agent's signed NIP-OA profile can name the owner coordinate to
    // query. Each exact `(owner, d=agent)` filter returns at most one current
    // replaceable event, so forged 30177 coordinates cannot amplify or crowd
    // the authentic policy out of a bounded result page.
    let verified_owners = nostr_convert::verified_agent_owners_from_profiles(&profile_events);
    let managed_filters = managed_policy_filters(&candidate_pubkeys, &verified_owners);
    let managed_agent_events = query_filter_batches(
        state,
        &semaphore,
        &managed_filters,
        "relay agent managed-policy query failed",
    )
    .await?;

    let mut agents = nostr_convert::relay_agents_from_directory_events(
        &directory_events,
        &managed_agent_events,
        &profile_events,
    );
    // Marked builds reject legacy directory records that lack a verified
    // NIP-OA owner, but do not require that owner to equal the viewer. The
    // verified owner's signed respond_to policy remains the authorization
    // boundary for independently operated relay agents.
    retain_agents_allowed_by_build(
        &mut agents,
        crate::managed_agents::owner_only_access_build(),
    );
    agents.retain(|agent| member_agent_channel_ids.contains_key(&agent.pubkey));
    for agent in &mut agents {
        agent.channel_ids = member_agent_channel_ids
            .get(&agent.pubkey)
            .cloned()
            .unwrap_or_default();
    }
    Ok(agents)
}

#[tauri::command]
pub async fn list_relay_agents(state: State<'_, AppState>) -> Result<Vec<RelayAgentInfo>, String> {
    list_relay_agents_for_state(&state).await
}

/// Revalidate only the selected relay agents in the target channel.
///
/// This preserves the full directory command for autocomplete while keeping
/// send-time authorization bounded by the actual mention set and destination.
#[tauri::command]
pub async fn revalidate_relay_agents(
    pubkeys: Vec<String>,
    channel_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<RelayAgentInfo>, String> {
    let requested_pubkeys = pubkeys
        .into_iter()
        .filter_map(|pubkey| nostr::PublicKey::from_hex(&pubkey).ok())
        .map(|pubkey| pubkey.to_hex())
        .collect::<std::collections::HashSet<_>>();
    if requested_pubkeys.is_empty() {
        return Ok(Vec::new());
    }
    list_relay_agents_for_selection(&state, Some(&requested_pubkeys), channel_id.as_deref()).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterExistingRelayAgentInput {
    agent_pubkey: String,
    display_name: String,
    expected_owner_pubkey: String,
    expected_relay_url: String,
    expected_signer_pubkey: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterExistingRelayAgentResult {
    event_id: String,
    agent_pubkey: String,
    owner_pubkey: String,
}

fn verified_registration_owner(
    profile: &nostr::Event,
    agent_pubkey: &str,
) -> Result<String, String> {
    identity_archive::verified_oa_owner(profile, agent_pubkey)
        .map(|(owner, _)| owner)
        .ok_or_else(|| "agent profile or owner attestation failed verification".to_string())
}

/// Publish the owner-reviewed directory policy for an existing remote agent.
/// No private key, runtime, provider, prompt, process, or local agent record is
/// accepted or written by this command.
#[tauri::command]
pub async fn register_existing_relay_agent(
    input: RegisterExistingRelayAgentInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RegisterExistingRelayAgentResult, String> {
    let agent_pubkey = nostr::PublicKey::from_hex(input.agent_pubkey.trim())
        .map_err(|error| format!("invalid agent pubkey: {error}"))?
        .to_hex();
    let display_name = input.display_name.trim();
    if display_name.is_empty() || display_name.chars().count() > 120 {
        return Err("display name must be between 1 and 120 characters".to_string());
    }
    crate::managed_agents::validate_managed_agent_definition_text(display_name, None, None)
        .map_err(|error| format!("Agent name is unsafe to publish: {error}"))?;

    let target = identity_archive::capture_relay_target(&state);
    crate::relay::assert_expected_relay_scope(
        Some(&input.expected_relay_url),
        &target.api_base_url,
    )?;
    let signer = state.signing_keys()?;
    let signer_pubkey = signer.public_key().to_hex();
    crate::relay::assert_expected_signer(Some(&input.expected_signer_pubkey), &signer_pubkey)?;
    if !input
        .expected_owner_pubkey
        .eq_ignore_ascii_case(&signer_pubkey)
    {
        return Err("verified agent owner does not match the active identity".to_string());
    }
    if agent_pubkey.eq_ignore_ascii_case(&signer_pubkey) {
        return Err("an owner identity cannot be registered as its own agent".to_string());
    }

    let profiles = query_relay_at_with_keys(
        &state,
        &target.api_base_url,
        &[serde_json::json!({
            "kinds": [0],
            "authors": [&agent_pubkey],
            "limit": 1,
        })],
        &signer,
        None,
    )
    .await?;
    let profile = profiles
        .first()
        .ok_or_else(|| "agent has no signed profile on this relay".to_string())?;
    let verified_owner = verified_registration_owner(profile, &agent_pubkey)?;
    if !verified_owner.eq_ignore_ascii_case(&signer_pubkey)
        || !verified_owner.eq_ignore_ascii_case(&input.expected_owner_pubkey)
    {
        return Err("agent profile is attested to a different owner".to_string());
    }

    let archived =
        identity_archive::fetch_archived_pubkeys_strict_at(&state, &target, &signer).await?;
    if archived.iter().any(|pubkey| pubkey == &agent_pubkey) {
        return Err("archived agent identities cannot be registered".to_string());
    }

    // Re-check both mutable workspace boundaries after the network preflight,
    // then use those exact snapshots for the only side effect.
    let publish_target = identity_archive::capture_relay_target(&state);
    crate::relay::assert_expected_relay_scope(
        Some(&input.expected_relay_url),
        &publish_target.api_base_url,
    )?;
    let publish_signer = state.signing_keys()?;
    crate::relay::assert_expected_signer(
        Some(&input.expected_signer_pubkey),
        &publish_signer.public_key().to_hex(),
    )?;

    {
        let _store_guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        if crate::managed_agents::load_managed_agents(&app)?
            .iter()
            .any(|agent| agent.pubkey.eq_ignore_ascii_case(&agent_pubkey))
        {
            return Err(format!("agent {agent_pubkey} is already managed locally"));
        }
    }

    let builder = crate::managed_agents::agent_events::build_existing_agent_registration(
        &agent_pubkey,
        display_name,
    )?;
    let response = submit_event_at_with_keys(
        builder,
        &state,
        &publish_target.api_base_url,
        &publish_signer,
    )
    .await?;
    Ok(RegisterExistingRelayAgentResult {
        event_id: response.event_id,
        agent_pubkey,
        owner_pubkey: verified_owner,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_build_requires_verified_owner_without_requiring_viewer_ownership() {
        let cross_owner = "b".repeat(64);
        let mut agents = vec![
            RelayAgentInfo {
                pubkey: "a".repeat(64),
                owner_pubkey: Some(cross_owner.clone()),
                name: "Verified cross-owner".to_string(),
                agent_type: "agent".to_string(),
                channels: Vec::new(),
                channel_ids: Vec::new(),
                capabilities: Vec::new(),
                status: "offline".to_string(),
                respond_to: None,
                respond_to_allowlist: Vec::new(),
            },
            RelayAgentInfo {
                pubkey: "c".repeat(64),
                owner_pubkey: None,
                name: "Ownerless legacy".to_string(),
                agent_type: "agent".to_string(),
                channels: Vec::new(),
                channel_ids: Vec::new(),
                capabilities: Vec::new(),
                status: "online".to_string(),
                respond_to: None,
                respond_to_allowlist: Vec::new(),
            },
        ];

        retain_agents_allowed_by_build(&mut agents, true);

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "Verified cross-owner");
        assert_eq!(
            agents[0].owner_pubkey.as_deref(),
            Some(cross_owner.as_str())
        );
    }

    #[test]
    fn oss_build_preserves_ownerless_legacy_agents() {
        let mut agents = vec![RelayAgentInfo {
            pubkey: "a".repeat(64),
            owner_pubkey: None,
            name: "Ownerless legacy".to_string(),
            agent_type: "agent".to_string(),
            channels: Vec::new(),
            channel_ids: Vec::new(),
            capabilities: Vec::new(),
            status: "online".to_string(),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
        }];

        retain_agents_allowed_by_build(&mut agents, false);

        assert_eq!(agents.len(), 1);
        assert!(agents[0].owner_pubkey.is_none());
    }

    #[test]
    fn exact_author_queries_prevent_noisy_agent_crowd_out() {
        let pubkeys = vec!["a".repeat(64), "b".repeat(64)];

        let filters = exact_author_filters(&pubkeys, 10100);

        assert_eq!(filters.len(), 2);
        for (filter, pubkey) in filters.iter().zip(pubkeys) {
            assert_eq!(filter["authors"], serde_json::json!([pubkey]));
            assert_eq!(filter["kinds"], serde_json::json!([10100]));
            assert_eq!(filter["limit"], 1);
        }
    }

    #[test]
    fn managed_policy_queries_are_exact_coordinates() {
        let candidates = vec!["a".repeat(64), "b".repeat(64)];
        let owners = std::collections::HashMap::from([
            (candidates[0].clone(), "c".repeat(64)),
            (candidates[1].clone(), "d".repeat(64)),
        ]);

        let filters = managed_policy_filters(&candidates, &owners);

        assert_eq!(filters.len(), 2);
        for (filter, candidate) in filters.iter().zip(candidates) {
            assert_eq!(filter["authors"].as_array().map(Vec::len), Some(1));
            assert_eq!(filter["kinds"], serde_json::json!([30177]));
            assert_eq!(filter["#d"], serde_json::json!([candidate]));
            assert_eq!(filter["limit"], 1);
        }
    }

    #[test]
    fn relay_filter_batches_do_not_exceed_protocol_limit() {
        let pubkeys: Vec<_> = (0..25).map(|index| format!("{index:064x}")).collect();
        let filters = exact_author_filters(&pubkeys, 0);

        let batch_sizes: Vec<_> = filters
            .chunks(RELAY_FILTER_BATCH_SIZE)
            .map(<[_]>::len)
            .collect();

        assert_eq!(batch_sizes, vec![10, 10, 5]);
    }

    #[test]
    fn registration_input_rejects_secret_or_runtime_fields() {
        for field in ["privateKeyNsec", "runtime", "provider", "systemPrompt"] {
            let mut input = serde_json::json!({
                "agentPubkey": "a".repeat(64),
                "displayName": "Remote helper",
                "expectedOwnerPubkey": "b".repeat(64),
                "expectedRelayUrl": "wss://relay.example",
                "expectedSignerPubkey": "b".repeat(64),
            });
            input[field] = serde_json::json!("forbidden");
            assert!(serde_json::from_value::<RegisterExistingRelayAgentInput>(input).is_err());
        }
    }

    #[test]
    fn registration_profile_requires_the_agent_signature_and_owner_attestation() {
        let owner = nostr::Keys::generate();
        let agent = nostr::Keys::generate();
        let target = nostr::PublicKey::from_hex(&agent.public_key().to_hex()).unwrap();
        let auth_json = buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner, &target, "").unwrap();
        let auth_parts: Vec<String> = serde_json::from_str(&auth_json).unwrap();
        let profile = nostr::EventBuilder::new(nostr::Kind::Metadata, "{}")
            .tags([nostr::Tag::parse(auth_parts).unwrap()])
            .sign_with_keys(&agent)
            .unwrap();

        assert_eq!(
            verified_registration_owner(&profile, &agent.public_key().to_hex()).unwrap(),
            owner.public_key().to_hex()
        );
        assert!(verified_registration_owner(&profile, &owner.public_key().to_hex()).is_err());
    }
}

#[cfg(all(test, not(target_os = "windows")))]
mod real_relay_tests {
    use super::*;
    use crate::{app_state::build_app_state, events, managed_agents, relay};
    use buzz_core_pkg::kind::KIND_MANAGED_AGENT;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use uuid::Uuid;

    fn relay_ws_url() -> String {
        std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3037".to_string())
    }

    fn state_for(keys: Keys) -> AppState {
        let state = build_app_state();
        *state.keys.lock().unwrap() = keys;
        *state.relay_url_override.lock().unwrap() = Some(relay_ws_url());
        state
    }

    async fn publish(builder: EventBuilder, signer: &Keys, state: &AppState) {
        relay::submit_event_with_keys(builder, state, signer, None)
            .await
            .expect("publish real-relay fixture");
    }

    #[tokio::test]
    #[ignore]
    async fn newly_retained_managed_policy_replaces_open_access_immediately_on_real_relay() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let state = state_for(owner.clone());
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.path().join("retention.sqlite3");
        let initial_content = serde_json::json!({
            "name": "Immediate Policy Probe",
            "parallelism": 1,
            "respond_to": "anyone"
        })
        .to_string();
        let initial_event =
            EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), initial_content)
                .tags([Tag::parse(["d", &agent.public_key().to_hex()]).unwrap()])
                .custom_created_at(nostr::Timestamp::from(
                    nostr::Timestamp::now().as_secs().saturating_sub(1),
                ));
        publish(initial_event, &owner, &state).await;

        let updated_content = serde_json::json!({
            "name": "Immediate Policy Probe",
            "parallelism": 1,
            "respond_to": "owner-only"
        })
        .to_string();
        let event = EventBuilder::new(Kind::Custom(KIND_MANAGED_AGENT as u16), updated_content)
            .tags([Tag::parse(["d", &agent.public_key().to_hex()]).unwrap()])
            .sign_with_keys(&owner)
            .unwrap();

        {
            use managed_agents::retention::{open_retention_db, retain_event, RetainedEvent};
            use nostr::JsonUtil;

            let conn = open_retention_db(&db_path).unwrap();
            retain_event(
                &conn,
                &RetainedEvent {
                    kind: KIND_MANAGED_AGENT,
                    pubkey: owner.public_key().to_hex(),
                    d_tag: agent.public_key().to_hex(),
                    content: event.content.clone(),
                    created_at: event.created_at.as_secs() as i64,
                    raw_event: event.as_json(),
                    pending_sync: true,
                },
            )
            .unwrap();
        }

        let flushed = managed_agents::persona_events::flush_pending_events_at(
            &db_path,
            &state,
            &relay_ws_url(),
            &owner,
        )
        .await
        .expect("create-path immediate policy flush");
        assert_eq!(flushed, 1);

        let queried = query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [KIND_MANAGED_AGENT],
                "authors": [owner.public_key().to_hex()],
                "#d": [agent.public_key().to_hex()],
                "limit": 1
            })],
        )
        .await
        .expect("query immediately flushed policy");
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].id, event.id);
        assert!(queried[0].content.contains("\"respond_to\":\"owner-only\""));
    }

    #[tokio::test]
    #[ignore]
    async fn cross_identity_managed_agent_is_discovered_and_emits_exact_p_tag_from_real_relay() {
        let owner = Keys::generate();
        let viewer = Keys::generate();
        let agent = Keys::generate();
        let owner_state = state_for(owner.clone());
        let viewer_state = state_for(viewer.clone());
        let channel_id = Uuid::new_v4();

        publish(
            events::build_create_channel(
                channel_id,
                &format!("agent-discovery-e2e-{channel_id}"),
                "private",
                "stream",
                None,
                None,
            )
            .unwrap(),
            &owner,
            &owner_state,
        )
        .await;
        publish(
            events::build_add_member(channel_id, &viewer.public_key().to_hex(), None).unwrap(),
            &owner,
            &owner_state,
        )
        .await;
        publish(
            events::build_add_member(channel_id, &agent.public_key().to_hex(), Some("bot"))
                .unwrap(),
            &owner,
            &owner_state,
        )
        .await;

        let compat_owner = nostr::Keys::parse(&owner.secret_key().to_secret_hex()).unwrap();
        let compat_agent = nostr::PublicKey::from_hex(&agent.public_key().to_hex()).unwrap();
        let auth_tag =
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&compat_owner, &compat_agent, "").unwrap();
        relay::sync_managed_agent_profile(
            &owner_state,
            &relay_ws_url(),
            &agent,
            "Agent Probe",
            None,
            Some(&auth_tag),
        )
        .await
        .expect("publish agent kind:0 profile");

        let managed_content = serde_json::json!({
            "name": "Agent Probe",
            "parallelism": 1,
            "respond_to": "anyone"
        })
        .to_string();
        publish(
            EventBuilder::new(Kind::Custom(30177), managed_content).tags([Tag::parse([
                "d",
                &agent.public_key().to_hex(),
            ])
            .unwrap()]),
            &owner,
            &owner_state,
        )
        .await;

        let agents = list_relay_agents_for_state(&viewer_state)
            .await
            .expect("query production relay directory");
        assert_eq!(agents.len(), 1, "real relay directory returned {agents:?}");
        assert_eq!(agents[0].pubkey, agent.public_key().to_hex());
        assert_eq!(agents[0].name, "Agent Probe");
        assert_eq!(agents[0].channel_ids, vec![channel_id.to_string()]);

        // Exercise the final protocol boundary, not merely the directory DTO:
        // selecting this candidate must become the agent's exact lowercase
        // `p` tag in the signed stream event.
        let mention_pubkey = agents[0].pubkey.as_str();
        let signed_message = events::build_message(
            channel_id,
            "Ask @Agent Probe to reply",
            None,
            &[mention_pubkey],
            &[],
            &[],
            &[],
            &[],
            None,
            &relay_ws_url(),
        )
        .unwrap()
        .sign_with_keys(&viewer)
        .unwrap();
        let emitted_mentions: Vec<_> = signed_message
            .tags
            .iter()
            .filter_map(|tag| {
                let tag = tag.as_slice();
                (tag.first().map(String::as_str) == Some("p"))
                    .then(|| tag.get(1).cloned())
                    .flatten()
            })
            .collect();
        assert_eq!(emitted_mentions, vec![agent.public_key().to_hex()]);
    }
}
