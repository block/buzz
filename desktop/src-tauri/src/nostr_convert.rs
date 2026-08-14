//! Nostr event → desktop model converters.
//!
//! These pure functions translate raw Nostr protocol events into the
//! model types expected by the Tauri frontend commands.
//!
//! All converters here are I/O-free and deterministic — they take owned
//! or borrowed events and return models. This makes them trivially
//! testable with hand-crafted events (see the `tests` module below).

use std::collections::{BTreeSet, HashMap};

use nostr::{Event, ToBech32};
use serde_json::{json, Value};

use crate::{
    managed_agents::{agent_events::managed_agent_content_from_event, RelayAgentInfo},
    models::*,
};

mod user_search;
pub use user_search::{
    list_user_search_results, rank_user_search_results, search_users_from_events,
    user_search_result_from_event,
};

// ── Tag helpers ─────────────────────────────────────────────────────────────

/// Find the first tag whose name matches `name` and return its first value.
///
/// e.g. for tag `["name", "general"]` with `name="name"` returns `Some("general")`.
fn first_tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    for tag in event.tags.iter() {
        let s = tag.as_slice();
        if s.len() >= 2 && s[0] == name {
            return Some(s[1].as_str());
        }
    }
    None
}

/// Return true if the event has a tag with the given name (any value).
fn has_tag(event: &Event, name: &str) -> bool {
    event
        .tags
        .iter()
        .any(|t| t.as_slice().first().map(|s| s.as_str()) == Some(name))
}

/// Iterate every tag whose name matches `name`, returning the full slice.
fn tags_named<'a>(event: &'a Event, name: &'a str) -> impl Iterator<Item = &'a [String]> + 'a {
    event.tags.iter().filter_map(move |t| {
        let s = t.as_slice();
        if !s.is_empty() && s[0] == name {
            Some(s)
        } else {
            None
        }
    })
}

/// Return the owner pubkey from a valid NIP-OA owner tag on a kind:0 profile.
///
/// NIP-OA marks an agent identity by having the owner sign an `auth` tag for
/// the agent pubkey. We verify the tag against the profile event author, not
/// against the owner, so a forged or stale marker does not turn a person into
/// an agent in mention search.
pub(crate) fn profile_valid_oa_owner_pubkey(event: &Event) -> Option<String> {
    let target_hex = event.pubkey.to_hex();
    let Ok(target_pubkey) = nostr::PublicKey::from_hex(&target_hex) else {
        return None;
    };

    for tag in event.tags.iter() {
        let slice = tag.as_slice();
        if slice.first().map(String::as_str) != Some("auth") || slice.len() != 4 {
            continue;
        }
        let Ok(json) = serde_json::to_string(slice) else {
            continue;
        };
        if let Ok(owner_pubkey) = buzz_sdk_pkg::nip_oa::verify_auth_tag(&json, &target_pubkey) {
            return Some(owner_pubkey.to_hex());
        }
    }

    None
}

pub(crate) fn profile_has_valid_oa_owner(event: &Event) -> bool {
    profile_valid_oa_owner_pubkey(event).is_some()
}

// ── kind:39000 / 39002 (NIP-29) ─────────────────────────────────────────────

/// Convert a NIP-29 kind:39000 channel metadata event to [`ChannelInfo`].
///
/// Optionally merges with a kind:40901 channel summary sidecar event for
/// `member_count` and `last_message_at`.
pub fn channel_info_from_event(
    event: &Event,
    summary: Option<&Event>,
    is_member: Option<bool>,
) -> Result<ChannelInfo, String> {
    let id = first_tag_value(event, "d")
        .ok_or_else(|| "kind:39000 missing required `d` tag".to_string())?
        .to_string();

    let name = first_tag_value(event, "name").unwrap_or("").to_string();
    let description = first_tag_value(event, "about").unwrap_or("").to_string();
    let topic = first_tag_value(event, "topic").map(str::to_string);
    let purpose = first_tag_value(event, "purpose").map(str::to_string);
    // Prefer explicit ["t", type] tag; fall back to inferring from ["hidden"]
    // (= dm) for relays that don't yet emit the type tag.
    let channel_type = first_tag_value(event, "t")
        .map(str::to_string)
        .unwrap_or_else(|| {
            if has_tag(event, "hidden") {
                "dm".to_string()
            } else {
                "stream".to_string()
            }
        });
    let visibility_tag = first_tag_value(event, "visibility");
    let visibility = if has_tag(event, "public") || visibility_tag == Some("open") {
        "open".to_string()
    } else if has_tag(event, "private") || visibility_tag == Some("private") {
        "private".to_string()
    } else {
        "open".to_string()
    };

    // For DM-type channels, p-tags identify the participants.
    let participant_pubkeys: Vec<String> = tags_named(event, "p")
        .filter_map(|s| s.get(1).cloned())
        .collect();
    let participants = participant_pubkeys.clone();

    // Summary sidecar carries member_count + last_message_at as JSON content.
    let (member_count, last_message_at) = if let Some(s) = summary {
        let v: Value = serde_json::from_str(&s.content).unwrap_or(Value::Null);
        let mc = v.get("member_count").and_then(Value::as_i64).unwrap_or(0);
        let lma = v
            .get("last_message_at")
            .and_then(Value::as_str)
            .map(str::to_string);
        (mc, lma)
    } else {
        (0, None)
    };

    // If the relay emits ["archived", "true"], surface it as a timestamp placeholder
    // so the frontend knows the channel is archived. The exact timestamp isn't available
    // from the tag alone, so we use the event's created_at as a proxy.
    let archived_at = if first_tag_value(event, "archived") == Some("true") {
        Some(timestamp_to_iso(event.created_at.as_secs()))
    } else {
        None
    };

    // Ephemeral channel TTL — relay emits ["ttl", "<seconds>"] and ["ttl_deadline", "<iso>"].
    let ttl_seconds = first_tag_value(event, "ttl").and_then(|v| v.parse::<i32>().ok());
    let ttl_deadline = first_tag_value(event, "ttl_deadline").map(str::to_string);

    Ok(ChannelInfo {
        id,
        name,
        channel_type,
        visibility,
        description,
        topic,
        purpose,
        member_count,
        member_pubkeys: Vec::new(),
        last_message_at,
        archived_at,
        participants,
        participant_pubkeys,
        is_member: is_member.unwrap_or(true),
        ttl_seconds,
        ttl_deadline,
    })
}

/// Convert a NIP-29 kind:39000 event to [`ChannelDetailInfo`].
pub fn channel_detail_from_event(event: &Event) -> Result<ChannelDetailInfo, String> {
    let id = first_tag_value(event, "d")
        .ok_or_else(|| "kind:39000 missing required `d` tag".to_string())?
        .to_string();

    let name = first_tag_value(event, "name").unwrap_or("").to_string();
    let description = first_tag_value(event, "about").unwrap_or("").to_string();
    let topic = first_tag_value(event, "topic").map(str::to_string);
    let purpose = first_tag_value(event, "purpose").map(str::to_string);
    // Prefer explicit ["t", type]; fall back to ["hidden"] = dm, else "stream".
    let channel_type = first_tag_value(event, "t")
        .map(str::to_string)
        .unwrap_or_else(|| {
            if has_tag(event, "hidden") {
                "dm".to_string()
            } else {
                "stream".to_string()
            }
        });
    let visibility_tag = first_tag_value(event, "visibility");
    let visibility = if has_tag(event, "public") || visibility_tag == Some("open") {
        "open".to_string()
    } else if has_tag(event, "private") || visibility_tag == Some("private") {
        "private".to_string()
    } else {
        "open".to_string()
    };

    let created_at_iso = timestamp_to_iso(event.created_at.as_secs());

    let archived_at = if first_tag_value(event, "archived") == Some("true") {
        Some(timestamp_to_iso(event.created_at.as_secs()))
    } else {
        None
    };

    Ok(ChannelDetailInfo {
        id,
        name,
        channel_type,
        visibility,
        description,
        topic,
        topic_set_by: None,
        topic_set_at: None,
        purpose,
        purpose_set_by: None,
        purpose_set_at: None,
        created_by: event.pubkey.to_hex(),
        created_at: created_at_iso.clone(),
        updated_at: created_at_iso,
        archived_at,
        member_count: 0,
        topic_required: false,
        max_members: None,
        nip29_group_id: None,
        ttl_seconds: first_tag_value(event, "ttl").and_then(|v| v.parse::<i32>().ok()),
        ttl_deadline: first_tag_value(event, "ttl_deadline").map(str::to_string),
    })
}

/// Convert a NIP-29 kind:39002 members event to [`ChannelMembersResponse`].
///
/// Members come from p-tags shaped as `["p", pubkey, relay_url?, role?]`.
/// Role defaults to `"member"` when absent. `joined_at` is `None` because
/// kind:39002 does not carry per-member join timestamps.
pub fn channel_members_from_event(event: &Event) -> Result<ChannelMembersResponse, String> {
    // Validate that this is a members event (`d` tag identifies the channel).
    if first_tag_value(event, "d").is_none() {
        return Err("kind:39002 missing required `d` tag".to_string());
    }

    let mut seen = BTreeSet::new();
    let mut members = Vec::new();
    for slice in tags_named(event, "p") {
        let Some(pubkey) = slice.get(1) else { continue };
        if pubkey.is_empty() || !seen.insert(pubkey.clone()) {
            continue;
        }
        let role = slice
            .get(3)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "member".to_string());
        members.push(ChannelMemberInfo {
            pubkey: pubkey.clone(),
            is_agent: role == "bot",
            role,
            joined_at: None,
            display_name: None,
        });
    }

    Ok(ChannelMembersResponse {
        members,
        next_cursor: None,
    })
}

// ── kind:0 (profile metadata) ───────────────────────────────────────────────

/// Convert a kind:0 metadata event to [`ProfileInfo`].
///
/// The event's `content` is a JSON object per NIP-01:
/// `{"name":"...","display_name":"...","picture":"...","about":"...","nip05":"..."}`.
pub fn profile_info_from_event(event: &Event) -> Result<ProfileInfo, String> {
    let v: Value = serde_json::from_str(&event.content)
        .map_err(|e| format!("kind:0 content is not valid JSON: {e}"))?;

    let display_name = v
        .get("display_name")
        .and_then(Value::as_str)
        .or_else(|| v.get("name").and_then(Value::as_str))
        .map(str::to_string);
    let avatar_url = v.get("picture").and_then(Value::as_str).map(str::to_string);
    let about = v.get("about").and_then(Value::as_str).map(str::to_string);
    let nip05_handle = v.get("nip05").and_then(Value::as_str).map(str::to_string);

    Ok(ProfileInfo {
        pubkey: event.pubkey.to_hex(),
        display_name,
        avatar_url,
        about,
        nip05_handle,
        owner_pubkey: profile_valid_oa_owner_pubkey(event),
        has_profile_event: true,
    })
}

/// Convert multiple kind:0 events to [`UsersBatchResponse`].
///
/// `requested_pubkeys` lets us populate `missing` for any pubkey that had
/// no metadata event in the input set.
pub fn users_batch_from_events(
    events: &[Event],
    requested_pubkeys: &[String],
) -> UsersBatchResponse {
    // Keep only the most recent kind:0 per pubkey.
    let mut latest: HashMap<String, &Event> = HashMap::new();
    for ev in events {
        let pk = ev.pubkey.to_hex();
        let take = match latest.get(&pk) {
            None => true,
            Some(prev) => ev.created_at > prev.created_at,
        };
        if take {
            latest.insert(pk, ev);
        }
    }

    let mut profiles = HashMap::new();
    for (pk, ev) in &latest {
        let v: Value = serde_json::from_str(&ev.content).unwrap_or(Value::Null);
        let owner_pubkey = profile_valid_oa_owner_pubkey(ev);
        let summary = UserProfileSummaryInfo {
            display_name: v
                .get("display_name")
                .and_then(Value::as_str)
                .or_else(|| v.get("name").and_then(Value::as_str))
                .map(str::to_string),
            name: v.get("name").and_then(Value::as_str).map(str::to_string),
            avatar_url: v.get("picture").and_then(Value::as_str).map(str::to_string),
            nip05_handle: v.get("nip05").and_then(Value::as_str).map(str::to_string),
            is_agent: owner_pubkey.is_some(),
            owner_pubkey,
        };
        profiles.insert(pk.clone(), summary);
    }

    let missing: Vec<String> = requested_pubkeys
        .iter()
        .filter(|pk| !profiles.contains_key(*pk))
        .cloned()
        .collect();

    UsersBatchResponse { profiles, missing }
}

// ── kind:1 (notes) ──────────────────────────────────────────────────────────

/// Convert kind:1 events to [`UserNotesResponse`].
///
/// Notes are returned in the input order. The cursor is built from the
/// oldest note (last in newest-first ordering) so the caller can page back.
pub fn user_notes_from_events(events: &[Event]) -> UserNotesResponse {
    let notes: Vec<UserNoteInfo> = events
        .iter()
        .map(|ev| UserNoteInfo {
            id: ev.id.to_hex(),
            pubkey: ev.pubkey.to_hex(),
            created_at: ev.created_at.as_secs() as i64,
            content: ev.content.clone(),
            tags: ev.tags.iter().map(|tag| tag.as_slice().to_vec()).collect(),
        })
        .collect();

    let next_cursor = notes.last().map(|n| UserNotesCursor {
        before: n.created_at,
        before_id: n.id.clone(),
    });

    UserNotesResponse { notes, next_cursor }
}

// ── kind:3 (contact list) ───────────────────────────────────────────────────

/// Convert a kind:3 contact list event to [`ContactListResponse`].
pub fn contact_list_from_event(event: &Event) -> Result<ContactListResponse, String> {
    let tags: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();

    Ok(ContactListResponse {
        id: event.id.to_hex(),
        pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs() as i64,
        tags,
        content: event.content.clone(),
    })
}

// ── NIP-50 search results ───────────────────────────────────────────────────

/// Convert search-result events (any kind) to [`SearchResponse`].
///
/// NIP-50 does not carry a relevance score on the wire; we use the input
/// position as a proxy: position 0 → score 1.0, dropping linearly to 0.
pub fn search_response_from_events(events: &[Event]) -> SearchResponse {
    let total = events.len();
    let hits: Vec<SearchHitInfo> = events
        .iter()
        .enumerate()
        .map(|(idx, ev)| {
            // Channel id is stored on a NIP-29 `h` tag when present.
            let channel_id = first_tag_value(ev, "h").map(str::to_string);
            let score = if total <= 1 {
                1.0
            } else {
                1.0 - (idx as f64) / (total as f64)
            };
            SearchHitInfo {
                event_id: ev.id.to_hex(),
                content: ev.content.clone(),
                kind: ev.kind.as_u16() as u32,
                pubkey: ev.pubkey.to_hex(),
                channel_id,
                channel_name: None,
                created_at: ev.created_at.as_secs(),
                score,
            }
        })
        .collect();

    SearchResponse {
        found: hits.len() as u64,
        hits,
    }
}

// ── kind:10100 (agent profiles) ─────────────────────────────────────────────

/// Convert kind:10100 agent profile events to the agent discovery format.
///
/// Returns a JSON array of `{pubkey, name, ...}` objects parsed from each
/// event's content.
pub fn agents_from_events(events: &[Event]) -> Value {
    let arr: Vec<Value> = events
        .iter()
        .map(|ev| {
            let mut v: Value = serde_json::from_str(&ev.content).unwrap_or_else(|_| json!({}));
            let pubkey = ev.pubkey.to_hex();
            // Full npub fallback — truncated prefixes are grindable (see pubkey-display).
            let npub = ev.pubkey.to_bech32().unwrap_or_else(|_| pubkey.clone());
            // Always overwrite the pubkey with the event author — it's the
            // authoritative source even if the content claims otherwise.
            if let Some(obj) = v.as_object_mut() {
                obj.insert("pubkey".to_string(), json!(pubkey.clone()));
                let fallback_name = obj
                    .get("display_name")
                    .and_then(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| npub.clone());
                if !obj.get("name").is_some_and(Value::is_string) {
                    obj.insert("name".to_string(), json!(fallback_name));
                }
                if !obj.get("agent_type").is_some_and(Value::is_string) {
                    obj.insert("agent_type".to_string(), json!("agent"));
                }
                if !obj.get("channels").is_some_and(Value::is_array) {
                    obj.insert("channels".to_string(), json!([]));
                }
                if !obj.get("channel_ids").is_some_and(Value::is_array) {
                    obj.insert("channel_ids".to_string(), json!([]));
                }
                if !obj.get("capabilities").is_some_and(Value::is_array) {
                    obj.insert("capabilities".to_string(), json!([]));
                }
                if !obj.get("status").is_some_and(Value::is_string) {
                    obj.insert("status".to_string(), json!("offline"));
                }
            } else {
                v = json!({
                    "pubkey": pubkey,
                    "name": npub,
                    "agent_type": "agent",
                    "channels": [],
                    "channel_ids": [],
                    "capabilities": [],
                    "status": "offline",
                });
            }
            v
        })
        .collect();
    json!({ "agents": arr })
}

// ── kind:0 + kind:30177 managed-agent directory ────────────────────────────

/// Collect valid agent pubkeys from kind:30177 `d` tags for follow-up relay
/// queries. Malformed tags are ignored so one hostile event cannot invalidate
/// the whole directory request.
pub fn managed_agent_pubkeys_from_events(events: &[Event]) -> std::collections::HashSet<String> {
    events
        .iter()
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
            let value = agents_from_events(std::slice::from_ref(event));
            let mut agent: RelayAgentInfo =
                serde_json::from_value(value.get("agents")?.as_array()?.first()?.clone()).ok()?;
            // Channel membership is authoritative only in relay-signed kind:39002.
            agent.channel_ids.clear();
            Some(agent)
        })
        .collect()
}

/// Merge self-authored kind:10100 runtime profiles with verified Desktop-managed
/// policy records. Managed policy wins for the same agent; kind:10100-only
/// headless agents remain discoverable.
pub fn relay_agents_from_directory_events(
    directory_events: &[Event],
    managed_agent_events: &[Event],
    profile_events: &[Event],
) -> Vec<RelayAgentInfo> {
    let mut agents: HashMap<String, RelayAgentInfo> =
        relay_agents_from_legacy_events(directory_events)
            .into_iter()
            .map(|agent| (agent.pubkey.clone(), agent))
            .collect();
    for agent in relay_agents_from_managed_agent_events(managed_agent_events, profile_events) {
        agents.insert(agent.pubkey.clone(), agent);
    }

    let mut agents: Vec<_> = agents.into_values().collect();
    agents.sort_by(|left, right| left.name.cmp(&right.name));
    agents
}

/// Build the relay agent directory from owner-authenticated managed-agent
/// records. A kind:30177 event is accepted only when its author matches the
/// owner cryptographically declared by the agent's own kind:0 NIP-OA tag.
pub fn relay_agents_from_managed_agent_events(
    managed_agent_events: &[Event],
    profile_events: &[Event],
) -> Vec<RelayAgentInfo> {
    let mut latest_profiles: HashMap<String, &Event> = HashMap::new();
    for profile in profile_events {
        let agent_pubkey = profile.pubkey.to_hex();
        let replace = latest_profiles
            .get(&agent_pubkey)
            .is_none_or(|previous| event_is_newer(profile, previous));
        if replace {
            latest_profiles.insert(agent_pubkey, profile);
        }
    }
    let verified_owners: HashMap<String, String> = latest_profiles
        .into_iter()
        .filter_map(|(agent_pubkey, profile)| {
            profile_valid_oa_owner_pubkey(profile).map(|owner| (agent_pubkey, owner))
        })
        .collect();

    let mut latest: HashMap<String, (&Event, RelayAgentInfo)> = HashMap::new();
    for event in managed_agent_events {
        let Some(agent_pubkey) = first_tag_value(event, "d") else {
            continue;
        };
        let Some(verified_owner) = verified_owners.get(agent_pubkey) else {
            continue;
        };
        if event.pubkey.to_hex() != *verified_owner {
            continue;
        }
        let Ok(content) = managed_agent_content_from_event(event) else {
            continue;
        };
        let info = RelayAgentInfo {
            pubkey: agent_pubkey.to_string(),
            name: content.name,
            agent_type: "agent".to_string(),
            channels: Vec::new(),
            channel_ids: Vec::new(),
            capabilities: Vec::new(),
            status: "offline".to_string(),
            respond_to: Some(content.respond_to),
            respond_to_allowlist: content.respond_to_allowlist,
        };
        let replace = latest
            .get(agent_pubkey)
            .is_none_or(|(previous, _)| event_is_newer(event, previous));
        if replace {
            latest.insert(agent_pubkey.to_string(), (event, info));
        }
    }

    let mut agents: Vec<_> = latest.into_values().map(|(_, info)| info).collect();
    agents.sort_by(|left, right| left.name.cmp(&right.name));
    agents
}

/// Build a pubkey-to-channel-id map from current kind:39002 membership heads.
pub fn agent_channel_ids_from_member_events(
    events: &[Event],
    agent_pubkeys: &std::collections::HashSet<String>,
    relay_pubkey: &str,
) -> HashMap<String, Vec<String>> {
    let mut channel_ids: HashMap<String, BTreeSet<String>> = HashMap::new();
    for event in events {
        if !event.pubkey.to_hex().eq_ignore_ascii_case(relay_pubkey) {
            continue;
        }
        let Some(channel_id) = first_tag_value(event, "d") else {
            continue;
        };
        for tag in tags_named(event, "p") {
            let Some(pubkey) = tag.get(1) else {
                continue;
            };
            if agent_pubkeys.contains(pubkey) {
                channel_ids
                    .entry(pubkey.clone())
                    .or_default()
                    .insert(channel_id.to_string());
            }
        }
    }

    channel_ids
        .into_iter()
        .map(|(pubkey, ids)| (pubkey, ids.into_iter().collect()))
        .collect()
}

// ── kind:13534 (relay membership list) ──────────────────────────────────────

/// Convert a kind:13534 relay membership list to the relay members format.
///
/// The relay emits `["member", pubkey]` or `["member", pubkey, role]` tags.
/// For backward compatibility, also accepts `["p", pubkey, relay_url?, role?]`.
pub fn relay_members_from_event(event: &Event) -> Value {
    let mut seen = BTreeSet::new();
    let mut members: Vec<Value> = Vec::new();

    // Primary: parse ["member", pubkey, role?] tags (current relay format).
    for slice in tags_named(event, "member") {
        let Some(pubkey) = slice.get(1).filter(|s| !s.is_empty()) else {
            continue;
        };
        if !seen.insert(pubkey.clone()) {
            continue;
        }
        let role = slice
            .get(2)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "member".to_string());
        members.push(json!({ "pubkey": pubkey, "role": role }));
    }

    // Fallback: parse ["p", pubkey, relay_url?, role?] tags (NIP-29 convention).
    for slice in tags_named(event, "p") {
        let Some(pubkey) = slice.get(1).filter(|s| !s.is_empty()) else {
            continue;
        };
        if !seen.insert(pubkey.clone()) {
            continue;
        }
        let role = slice
            .get(3)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "member".to_string());
        members.push(json!({ "pubkey": pubkey, "role": role }));
    }

    json!({ "members": members })
}

// ── Time helpers ────────────────────────────────────────────────────────────

/// Convert a unix-seconds timestamp to a UTC RFC-3339 string.
pub(crate) fn timestamp_to_iso(secs: u64) -> String {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let dt = UNIX_EPOCH + Duration::from_secs(secs);
    // Format manually as RFC-3339 — the `time` crate is already a transitive
    // dep, but using SystemTime keeps this self-contained.
    let dur = dt
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs_total = dur.as_secs() as i64;
    // Days since epoch, seconds within day.
    let (days, sod) = (secs_total.div_euclid(86_400), secs_total.rem_euclid(86_400));
    let h = sod / 3600;
    let m = (sod % 3600) / 60;
    let s = sod % 60;
    let (y, mo, d) = days_to_ymd(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Convert days-since-1970-01-01 to (year, month, day) using the civil-from-days
/// algorithm by Howard Hinnant (public domain).
fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    /// Build a signed event for testing with the given kind, content, and tags.
    fn ev(kind: u16, content: &str, tags: Vec<Vec<&str>>) -> Event {
        let keys = Keys::generate();
        let parsed: Vec<Tag> = tags
            .into_iter()
            .map(|t| Tag::parse(t).expect("parse tag"))
            .collect();
        EventBuilder::new(Kind::from_u16(kind), content)
            .tags(parsed)
            .sign_with_keys(&keys)
            .expect("sign")
    }

    /// Build a kind:0 profile with a valid NIP-OA auth tag.
    fn oa_profile_event(content: &str) -> (Event, String) {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pubkey = agent_keys.public_key();
        let tag_json = buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_pubkey, "")
            .expect("compute auth tag");
        let tag_values: Vec<String> = serde_json::from_str(&tag_json).expect("parse auth tag json");
        let auth_tag = Tag::parse(tag_values).expect("parse auth tag");

        let event = EventBuilder::new(Kind::Metadata, content)
            .tags(vec![auth_tag])
            .sign_with_keys(&agent_keys)
            .expect("sign");
        (event, owner_keys.public_key().to_hex())
    }

    fn managed_agent_event(
        owner_keys: &Keys,
        agent_pubkey: &str,
        name: &str,
        respond_to: &str,
        respond_to_allowlist: &[String],
    ) -> Event {
        let content = serde_json::json!({
            "name": name,
            "parallelism": 1,
            "respond_to": respond_to,
            "respond_to_allowlist": respond_to_allowlist,
        })
        .to_string();
        EventBuilder::new(Kind::Custom(30177), content)
            .tags([Tag::parse(["d", agent_pubkey]).expect("parse d tag")])
            .sign_with_keys(owner_keys)
            .expect("sign managed-agent event")
    }

    #[test]
    fn channel_info_minimal() {
        let e = ev(
            39000,
            "",
            vec![
                vec!["d", "chan-uuid-1"],
                vec!["name", "general"],
                vec!["about", "main channel"],
                vec!["t", "stream"],
                vec!["public"],
            ],
        );
        let info = channel_info_from_event(&e, None, None).unwrap();
        assert_eq!(info.id, "chan-uuid-1");
        assert_eq!(info.name, "general");
        assert_eq!(info.description, "main channel");
        assert_eq!(info.channel_type, "stream");
        assert_eq!(info.visibility, "open");
        assert_eq!(info.member_count, 0);
        assert!(info.is_member);
    }

    #[test]
    fn channel_info_private_when_visibility_tag_present() {
        let e = ev(
            39000,
            "",
            vec![
                vec!["d", "u"],
                vec!["name", "n"],
                vec!["t", "forum"],
                vec!["visibility", "private"],
                vec!["ttl", "86400"],
            ],
        );
        let info = channel_info_from_event(&e, None, None).unwrap();
        assert_eq!(info.visibility, "private");
        assert_eq!(info.channel_type, "forum");
        assert_eq!(info.ttl_seconds, Some(86400));
    }

    #[test]
    fn channel_info_open_when_neither_public_nor_private() {
        // Neither tag present → open (matches NIP-29 default).
        let e = ev(
            39000,
            "",
            vec![vec!["d", "u"], vec!["name", "n"], vec!["t", "forum"]],
        );
        let info = channel_info_from_event(&e, None, None).unwrap();
        assert_eq!(info.visibility, "open");
    }

    #[test]
    fn channel_info_dm_inferred_from_hidden_tag() {
        // Fallback: relays without ["t", "dm"] still emit ["hidden"] for DMs.
        let e = ev(
            39000,
            "",
            vec![vec!["d", "u"], vec!["name", "n"], vec!["hidden"]],
        );
        let info = channel_info_from_event(&e, None, None).unwrap();
        assert_eq!(info.channel_type, "dm");
    }

    #[test]
    fn channel_info_merges_summary() {
        let chan = ev(39000, "", vec![vec!["d", "u"], vec!["name", "n"]]);
        let summary = ev(
            40901,
            r#"{"member_count": 7, "last_message_at": "2026-01-01T00:00:00Z"}"#,
            vec![vec!["d", "u"]],
        );
        let info = channel_info_from_event(&chan, Some(&summary), None).unwrap();
        assert_eq!(info.member_count, 7);
        assert_eq!(
            info.last_message_at.as_deref(),
            Some("2026-01-01T00:00:00Z")
        );
    }

    #[test]
    fn channel_info_missing_d_errors() {
        let e = ev(39000, "", vec![vec!["name", "n"]]);
        assert!(channel_info_from_event(&e, None, None).is_err());
    }

    #[test]
    fn channel_detail_basic() {
        let e = ev(
            39000,
            "",
            vec![
                vec!["d", "uuid"],
                vec!["name", "n"],
                vec!["about", "desc"],
                vec!["topic", "tt"],
                vec!["purpose", "pp"],
                vec!["t", "dm"],
                vec!["visibility", "private"],
                vec!["ttl", "86400"],
                vec!["ttl_deadline", "2026-06-11T00:00:00Z"],
            ],
        );
        let d = channel_detail_from_event(&e).unwrap();
        assert_eq!(d.id, "uuid");
        assert_eq!(d.topic.as_deref(), Some("tt"));
        assert_eq!(d.purpose.as_deref(), Some("pp"));
        assert_eq!(d.channel_type, "dm");
        assert_eq!(d.visibility, "private");
        assert_eq!(d.ttl_seconds, Some(86400));
        assert_eq!(d.ttl_deadline.as_deref(), Some("2026-06-11T00:00:00Z"));
        assert!(d.created_at.ends_with("Z"));
        assert_eq!(d.created_by, e.pubkey.to_hex());
    }

    #[test]
    fn channel_members_extracts_p_tags() {
        let pk1 = "a".repeat(64);
        let pk2 = "b".repeat(64);
        let e = ev(
            39002,
            "",
            vec![
                vec!["d", "uuid"],
                vec!["p", &pk1, "", "admin"],
                vec!["p", &pk2],
                // Duplicate must be deduped.
                vec!["p", &pk1, "wss://x", "owner"],
            ],
        );
        let r = channel_members_from_event(&e).unwrap();
        assert_eq!(r.members.len(), 2);
        assert_eq!(r.members[0].pubkey, pk1);
        assert_eq!(r.members[0].role, "admin");
        assert!(r.members[0].joined_at.is_none());
        assert_eq!(r.members[1].role, "member"); // default
    }

    #[test]
    fn channel_members_missing_d_errors() {
        let e = ev(39002, "", vec![]);
        assert!(channel_members_from_event(&e).is_err());
    }

    #[test]
    fn profile_info_parses_content() {
        let e = ev(
            0,
            r#"{"name":"alice","display_name":"Alice","picture":"http://x/a.png","about":"hi","nip05":"alice@x"}"#,
            vec![],
        );
        let p = profile_info_from_event(&e).unwrap();
        assert_eq!(p.display_name.as_deref(), Some("Alice"));
        assert_eq!(p.avatar_url.as_deref(), Some("http://x/a.png"));
        assert_eq!(p.about.as_deref(), Some("hi"));
        assert_eq!(p.nip05_handle.as_deref(), Some("alice@x"));
        assert_eq!(p.pubkey, e.pubkey.to_hex());
        assert!(p.owner_pubkey.is_none());
    }

    #[test]
    fn profile_info_extracts_valid_nip_oa_owner() {
        let (event, owner_pubkey) = oa_profile_event(r#"{"display_name":"Mira"}"#);
        let p = profile_info_from_event(&event).unwrap();

        assert_eq!(p.owner_pubkey.as_deref(), Some(owner_pubkey.as_str()));
    }

    #[test]
    fn profile_info_falls_back_to_name() {
        let e = ev(0, r#"{"name":"bob"}"#, vec![]);
        let p = profile_info_from_event(&e).unwrap();
        assert_eq!(p.display_name.as_deref(), Some("bob"));
    }

    #[test]
    fn profile_info_invalid_json_errors() {
        let e = ev(0, "not-json", vec![]);
        assert!(profile_info_from_event(&e).is_err());
    }

    #[test]
    fn users_batch_keeps_latest_and_reports_missing() {
        let e1 = ev(0, r#"{"name":"old"}"#, vec![]);
        // Same author, newer event with display_name.
        let keys = Keys::generate();
        let e_old = EventBuilder::new(Kind::Metadata, r#"{"name":"old"}"#)
            .custom_created_at(nostr::Timestamp::from(1000))
            .sign_with_keys(&keys)
            .unwrap();
        let e_new = EventBuilder::new(Kind::Metadata, r#"{"display_name":"New"}"#)
            .custom_created_at(nostr::Timestamp::from(2000))
            .sign_with_keys(&keys)
            .unwrap();
        let pk = keys.public_key().to_hex();
        let other_pk = e1.pubkey.to_hex();

        let missing_pk = "f".repeat(64);
        let resp = users_batch_from_events(
            &[e1, e_old, e_new],
            &[pk.clone(), other_pk.clone(), missing_pk.clone()],
        );
        assert_eq!(resp.profiles.len(), 2);
        assert_eq!(resp.profiles[&pk].display_name.as_deref(), Some("New"));
        assert_eq!(resp.missing, vec![missing_pk]);
    }

    #[test]
    fn users_batch_marks_valid_nip_oa_profiles_as_agents() {
        let (agent, owner_pubkey) = oa_profile_event(r#"{"display_name":"Mira"}"#);
        let pubkey = agent.pubkey.to_hex();
        let resp =
            users_batch_from_events(std::slice::from_ref(&agent), std::slice::from_ref(&pubkey));

        assert!(resp.profiles[&pubkey].is_agent);
        assert_eq!(
            resp.profiles[&pubkey].owner_pubkey.as_deref(),
            Some(owner_pubkey.as_str())
        );
    }

    #[test]
    fn user_notes_builds_cursor_from_last() {
        let e1 = ev(1, "first", vec![]);
        let e2 = ev(1, "second", vec![]);
        let r = user_notes_from_events(&[e1, e2]);
        assert_eq!(r.notes.len(), 2);
        assert_eq!(r.notes[0].content, "first");
        let cursor = r.next_cursor.expect("cursor");
        assert_eq!(cursor.before_id, r.notes[1].id);
    }

    #[test]
    fn user_notes_empty_has_no_cursor() {
        let r = user_notes_from_events(&[]);
        assert!(r.notes.is_empty());
        assert!(r.next_cursor.is_none());
    }

    #[test]
    fn contact_list_preserves_tags_and_content() {
        let pk = "1".repeat(64);
        let e = ev(3, "rel-json", vec![vec!["p", &pk]]);
        let r = contact_list_from_event(&e).unwrap();
        assert_eq!(r.content, "rel-json");
        assert_eq!(r.tags.len(), 1);
        assert_eq!(r.tags[0], vec!["p".to_string(), pk]);
    }

    #[test]
    fn search_response_assigns_descending_scores() {
        let e1 = ev(1, "one", vec![vec!["h", "chan"]]);
        let e2 = ev(1, "two", vec![]);
        let r = search_response_from_events(&[e1, e2]);
        assert_eq!(r.found, 2);
        assert!(r.hits[0].score > r.hits[1].score);
        assert_eq!(r.hits[0].channel_id.as_deref(), Some("chan"));
        assert!(r.hits[1].channel_id.is_none());
    }

    #[test]
    fn search_response_single_hit_full_score() {
        let e = ev(1, "only", vec![]);
        let r = search_response_from_events(&[e]);
        assert_eq!(r.hits.len(), 1);
        assert_eq!(r.hits[0].score, 1.0);
    }

    #[test]
    fn agents_overwrites_pubkey_from_event_author() {
        let e = ev(10100, r#"{"pubkey":"forged","name":"agent-1"}"#, vec![]);
        let v = agents_from_events(std::slice::from_ref(&e));
        let arr = v.get("agents").and_then(Value::as_array).unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(
            arr[0].get("pubkey").and_then(Value::as_str).unwrap(),
            e.pubkey.to_hex()
        );
        assert_eq!(arr[0].get("name").and_then(Value::as_str), Some("agent-1"));
    }

    #[test]
    fn agents_handles_invalid_content() {
        let e = ev(10100, "not-json", vec![]);
        let v = agents_from_events(std::slice::from_ref(&e));
        let arr = v.get("agents").and_then(Value::as_array).unwrap();
        assert_eq!(
            arr[0].get("pubkey").and_then(Value::as_str).unwrap(),
            e.pubkey.to_hex()
        );
    }

    #[test]
    fn agents_default_sparse_agent_profiles_for_directory_parse() {
        let e = ev(
            10100,
            r#"{"channel_add_policy":"owner-only","display_name":"Scout"}"#,
            vec![],
        );
        let v = agents_from_events(std::slice::from_ref(&e));
        let agents = v.get("agents").cloned().unwrap();
        let parsed: Vec<crate::managed_agents::RelayAgentInfo> =
            serde_json::from_value(agents).unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].pubkey, e.pubkey.to_hex());
        assert_eq!(parsed[0].name, "Scout");
        assert_eq!(parsed[0].agent_type, "agent");
        assert_eq!(parsed[0].channels, Vec::<String>::new());
        assert_eq!(parsed[0].capabilities, Vec::<String>::new());
        assert_eq!(parsed[0].status, "offline");
        assert_eq!(parsed[0].respond_to, None);
    }

    #[test]
    fn agents_preserves_public_respond_to_mode_for_directory_parse() {
        let e = ev(10100, r#"{"name":"Scout","respond_to":"anyone"}"#, vec![]);
        let v = agents_from_events(std::slice::from_ref(&e));
        let agents = v.get("agents").cloned().unwrap();
        let parsed: Vec<crate::managed_agents::RelayAgentInfo> =
            serde_json::from_value(agents).unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].respond_to,
            Some(crate::managed_agents::RespondTo::Anyone)
        );
    }

    #[test]
    fn agents_preserves_allowlist_metadata_for_directory_parse() {
        let e = ev(
            10100,
            r#"{"name":"Scout","respond_to":"allowlist","respond_to_allowlist":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#,
            vec![],
        );
        let v = agents_from_events(std::slice::from_ref(&e));
        let agents = v.get("agents").cloned().unwrap();
        let parsed: Vec<crate::managed_agents::RelayAgentInfo> =
            serde_json::from_value(agents).unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(
            parsed[0].respond_to,
            Some(crate::managed_agents::RespondTo::Allowlist)
        );
        assert_eq!(parsed[0].respond_to_allowlist, vec!["a".repeat(64)]);
    }

    #[test]
    fn managed_agent_directory_accepts_only_the_verified_owner_policy() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let attacker_keys = Keys::generate();
        let agent_pubkey = agent_keys.public_key().to_hex();
        let viewer_pubkey = "a".repeat(64);

        let auth_tag_json =
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "")
                .expect("compute auth tag");
        let auth_tag_values: Vec<String> =
            serde_json::from_str(&auth_tag_json).expect("parse auth tag json");
        let profile = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Codex"}"#)
            .tags([Tag::parse(auth_tag_values).expect("parse auth tag")])
            .sign_with_keys(&agent_keys)
            .expect("sign profile");
        let authentic = managed_agent_event(
            &owner_keys,
            &agent_pubkey,
            "Codex",
            "allowlist",
            std::slice::from_ref(&viewer_pubkey),
        );
        let forged =
            managed_agent_event(&attacker_keys, &agent_pubkey, "Fake Codex", "anyone", &[]);

        let agents = relay_agents_from_managed_agent_events(
            &[forged, authentic],
            std::slice::from_ref(&profile),
        );

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].pubkey, agent_pubkey);
        assert_eq!(agents[0].name, "Codex");
        assert_eq!(
            agents[0].respond_to,
            Some(crate::managed_agents::RespondTo::Allowlist)
        );
        assert_eq!(agents[0].respond_to_allowlist, vec![viewer_pubkey]);
    }

    #[test]
    fn managed_agent_directory_rejects_agents_without_verified_owner_profiles() {
        let owner_keys = Keys::generate();
        let unverified_agent_keys = Keys::generate();
        let agent_pubkey = unverified_agent_keys.public_key().to_hex();
        let profile = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Codex"}"#)
            .sign_with_keys(&unverified_agent_keys)
            .expect("sign profile");
        let managed = managed_agent_event(&owner_keys, &agent_pubkey, "Codex", "anyone", &[]);

        let agents = relay_agents_from_managed_agent_events(
            std::slice::from_ref(&managed),
            std::slice::from_ref(&profile),
        );

        assert!(agents.is_empty());
    }

    #[test]
    fn managed_agent_directory_uses_the_latest_profile_head() {
        let agent_keys = Keys::generate();
        let owner_keys = Keys::generate();
        let agent_pubkey = agent_keys.public_key().to_hex();
        let auth_tag_json =
            buzz_sdk_pkg::nip_oa::compute_auth_tag(&owner_keys, &agent_keys.public_key(), "")
                .expect("compute auth tag");
        let auth_tag_values: Vec<String> =
            serde_json::from_str(&auth_tag_json).expect("parse auth tag json");
        let verified_profile = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Codex"}"#)
            .tags([Tag::parse(auth_tag_values).expect("parse auth tag")])
            .custom_created_at(nostr::Timestamp::from(10))
            .sign_with_keys(&agent_keys)
            .expect("sign verified profile");
        let revoked_profile = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Codex"}"#)
            .custom_created_at(nostr::Timestamp::from(20))
            .sign_with_keys(&agent_keys)
            .expect("sign revoked profile");
        let managed = managed_agent_event(&owner_keys, &agent_pubkey, "Codex", "anyone", &[]);

        let agents = relay_agents_from_managed_agent_events(
            std::slice::from_ref(&managed),
            &[verified_profile, revoked_profile],
        );

        assert!(agents.is_empty());
    }

    #[test]
    fn managed_agent_channel_ids_use_current_membership_events() {
        let relay_keys = Keys::generate();
        let agent_pubkey = "a".repeat(64);
        let stranger = "b".repeat(64);
        let candidates = [agent_pubkey.clone()].into_iter().collect();
        let general = EventBuilder::new(Kind::Custom(39002), "")
            .tags([
                Tag::parse(["d", "family"]).expect("parse d tag"),
                Tag::parse(["p", &agent_pubkey, "", "bot"]).expect("parse agent tag"),
                Tag::parse(["p", &stranger, "", "member"]).expect("parse member tag"),
            ])
            .sign_with_keys(&relay_keys)
            .expect("sign membership");
        let forged = ev(
            39002,
            "",
            vec![vec!["d", "forged"], vec!["p", &agent_pubkey, "", "bot"]],
        );

        let channel_ids = agent_channel_ids_from_member_events(
            &[forged, general],
            &candidates,
            &relay_keys.public_key().to_hex(),
        );

        assert_eq!(
            channel_ids.get(&agent_pubkey),
            Some(&vec!["family".to_string()])
        );
        assert!(!channel_ids.contains_key(&stranger));
    }

    #[test]
    fn managed_agent_directory_query_pubkeys_reject_malformed_d_tags() {
        let valid_pubkey = Keys::generate().public_key().to_hex();
        let valid = ev(30177, "{}", vec![vec!["d", &valid_pubkey]]);
        let malformed = ev(30177, "{}", vec![vec!["d", "not-a-pubkey"]]);

        let pubkeys = managed_agent_pubkeys_from_events(&[malformed, valid]);

        assert_eq!(pubkeys, [valid_pubkey].into_iter().collect());
    }

    #[test]
    fn relay_agent_directory_preserves_headless_profiles_and_prefers_verified_managed_policy() {
        let owner_keys = Keys::generate();
        let managed_agent_keys = Keys::generate();
        let managed_pubkey = managed_agent_keys.public_key().to_hex();
        let headless_keys = Keys::generate();
        let headless_pubkey = headless_keys.public_key().to_hex();
        let viewer_pubkey = "a".repeat(64);

        let headless_profile = EventBuilder::new(
            Kind::Custom(10100),
            serde_json::json!({
                "name": "Headless",
                "respond_to": "anyone",
                "channel_ids": ["untrusted-channel"]
            })
            .to_string(),
        )
        .sign_with_keys(&headless_keys)
        .expect("sign headless directory profile");
        let stale_managed_profile = EventBuilder::new(
            Kind::Custom(10100),
            serde_json::json!({
                "name": "Stale Codex",
                "respond_to": "anyone"
            })
            .to_string(),
        )
        .sign_with_keys(&managed_agent_keys)
        .expect("sign managed directory profile");

        let auth_tag_json = buzz_sdk_pkg::nip_oa::compute_auth_tag(
            &owner_keys,
            &managed_agent_keys.public_key(),
            "",
        )
        .expect("compute auth tag");
        let auth_tag_values: Vec<String> =
            serde_json::from_str(&auth_tag_json).expect("parse auth tag json");
        let managed_identity = EventBuilder::new(Kind::Metadata, r#"{"display_name":"Codex"}"#)
            .tags([Tag::parse(auth_tag_values).expect("parse auth tag")])
            .sign_with_keys(&managed_agent_keys)
            .expect("sign managed profile");
        let managed_policy = managed_agent_event(
            &owner_keys,
            &managed_pubkey,
            "Codex",
            "allowlist",
            std::slice::from_ref(&viewer_pubkey),
        );

        let agents = relay_agents_from_directory_events(
            &[headless_profile, stale_managed_profile],
            std::slice::from_ref(&managed_policy),
            std::slice::from_ref(&managed_identity),
        );

        assert_eq!(agents.len(), 2);
        let headless = agents
            .iter()
            .find(|agent| agent.pubkey == headless_pubkey)
            .expect("headless profile retained");
        assert_eq!(
            headless.respond_to,
            Some(crate::managed_agents::RespondTo::Anyone)
        );
        assert!(
            headless.channel_ids.is_empty(),
            "claimed channel ids are not trusted"
        );

        let managed = agents
            .iter()
            .find(|agent| agent.pubkey == managed_pubkey)
            .expect("managed profile retained");
        assert_eq!(managed.name, "Codex");
        assert_eq!(
            managed.respond_to,
            Some(crate::managed_agents::RespondTo::Allowlist)
        );
        assert_eq!(managed.respond_to_allowlist, vec![viewer_pubkey]);
    }

    #[test]
    fn relay_agent_directory_resolves_equal_timestamp_heads_by_event_id() {
        let keys = Keys::generate();
        let timestamp = nostr::Timestamp::from(42);
        let first = EventBuilder::new(
            Kind::Custom(10100),
            r#"{"name":"First","respond_to":"anyone"}"#,
        )
        .custom_created_at(timestamp)
        .sign_with_keys(&keys)
        .expect("sign first directory head");
        let second = EventBuilder::new(
            Kind::Custom(10100),
            r#"{"name":"Second","respond_to":"anyone"}"#,
        )
        .custom_created_at(timestamp)
        .sign_with_keys(&keys)
        .expect("sign second directory head");
        let expected_name = if first.id < second.id {
            "First"
        } else {
            "Second"
        };

        let forward =
            relay_agents_from_directory_events(&[first.clone(), second.clone()], &[], &[]);
        let reverse = relay_agents_from_directory_events(&[second, first], &[], &[]);

        assert_eq!(forward.len(), 1);
        assert_eq!(reverse.len(), 1);
        assert_eq!(forward[0].name, expected_name);
        assert_eq!(reverse[0].name, expected_name);
    }

    #[test]
    fn forged_managed_policy_cannot_suppress_a_headless_directory_agent() {
        let attacker_keys = Keys::generate();
        let targeted_agent_keys = Keys::generate();
        let targeted_pubkey = targeted_agent_keys.public_key().to_hex();
        let headless_keys = Keys::generate();
        let headless_pubkey = headless_keys.public_key().to_hex();
        let targeted_profile = EventBuilder::new(
            Kind::Custom(10100),
            r#"{"name":"Targeted","respond_to":"anyone"}"#,
        )
        .sign_with_keys(&targeted_agent_keys)
        .expect("sign targeted profile");
        let headless = EventBuilder::new(
            Kind::Custom(10100),
            r#"{"name":"Headless","respond_to":"anyone"}"#,
        )
        .sign_with_keys(&headless_keys)
        .expect("sign headless profile");
        let forged_policy = managed_agent_event(
            &attacker_keys,
            &targeted_pubkey,
            "Codex",
            "allowlist",
            &["a".repeat(64)],
        );

        let agents = relay_agents_from_directory_events(
            &[targeted_profile, headless],
            std::slice::from_ref(&forged_policy),
            &[],
        );

        assert_eq!(agents.len(), 2);
        assert!(agents.iter().any(|agent| agent.pubkey == targeted_pubkey));
        assert!(agents.iter().any(|agent| agent.pubkey == headless_pubkey));
    }

    #[test]
    fn relay_members_dedupes_and_defaults_role() {
        let pk1 = "a".repeat(64);
        let pk2 = "b".repeat(64);
        // Current relay format: ["member", pubkey, role]
        let e = ev(
            13534,
            "",
            vec![
                vec!["member", &pk1, "owner"],
                vec!["member", &pk2],
                vec!["member", &pk1, "moderator"], // dupe — ignored
            ],
        );
        let v = relay_members_from_event(&e);
        let arr = v.get("members").and_then(Value::as_array).unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].get("role").and_then(Value::as_str), Some("owner"));
        assert_eq!(arr[1].get("role").and_then(Value::as_str), Some("member"));
    }

    #[test]
    fn relay_members_fallback_p_tags() {
        let pk1 = "a".repeat(64);
        let pk2 = "b".repeat(64);
        // Legacy/fallback format: ["p", pubkey, relay_url?, role?]
        let e = ev(
            13534,
            "",
            vec![vec!["p", &pk1, "", "admin"], vec!["p", &pk2]],
        );
        let v = relay_members_from_event(&e);
        let arr = v.get("members").and_then(Value::as_array).unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].get("role").and_then(Value::as_str), Some("admin"));
        assert_eq!(arr[1].get("role").and_then(Value::as_str), Some("member"));
    }

    #[test]
    fn timestamp_to_iso_known_value() {
        // 2021-01-01T00:00:00Z = 1609459200
        assert_eq!(timestamp_to_iso(1_609_459_200), "2021-01-01T00:00:00Z");
        // Epoch
        assert_eq!(timestamp_to_iso(0), "1970-01-01T00:00:00Z");
    }
}
