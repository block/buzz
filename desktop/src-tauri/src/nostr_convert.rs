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

use crate::models::*;

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
/// NIP-OA requires a valid event and exactly one well-formed `auth` tag whose
/// owner signature and every condition apply to the event. Time clauses constrain
/// the profile's `created_at`, not the verifier's clock. Ownership is provenance;
/// the agent's pubkey remains the author.
pub(crate) fn profile_valid_oa_owner_pubkey(event: &Event) -> Option<String> {
    if event.kind != nostr::Kind::Metadata {
        return None;
    }

    let mut auth_tags = tags_named(event, "auth");
    let auth_tag = auth_tags.next()?;
    // Count malformed auth tags too: no first-valid-tag fallback is permitted.
    if auth_tags.next().is_some() {
        return None;
    }

    let json = serde_json::to_string(auth_tag).ok()?;
    // The structural parser also enforces canonical lowercase key/signature hex.
    buzz_sdk_pkg::nip_oa::parse_auth_tag(&json).ok()?;
    event.verify().ok()?;
    let owner = buzz_sdk_pkg::nip_oa::verify_auth_tag(&json, &event.pubkey).ok()?;
    let conditions = auth_tag.get(2)?;
    // Syntax/ranges were checked by the SDK; evaluate every signed clause as-is.
    let applies = conditions.is_empty()
        || conditions.split('&').all(|clause| {
            if let Some(value) = clause.strip_prefix("kind=") {
                value.parse::<u16>() == Ok(event.kind.as_u16())
            } else if let Some(value) = clause.strip_prefix("created_at<") {
                value
                    .parse::<u64>()
                    .is_ok_and(|bound| event.created_at.as_secs() < bound)
            } else if let Some(value) = clause.strip_prefix("created_at>") {
                value
                    .parse::<u64>()
                    .is_ok_and(|bound| event.created_at.as_secs() > bound)
            } else {
                false
            }
        });

    applies.then(|| owner.to_hex())
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

mod agent_directory;
pub use agent_directory::{
    managed_agent_pubkeys_from_events, member_agent_channel_ids_from_events,
    relay_agents_from_directory_events, relay_agents_from_managed_agent_events,
    verified_agent_owners_from_profiles,
};

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
mod tests;

#[cfg(test)]
mod oa_profile_tests;
