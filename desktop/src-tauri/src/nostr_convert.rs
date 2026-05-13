//! Nostr event → desktop model converters.
//!
//! These pure functions translate raw Nostr protocol events into the
//! model types expected by the Tauri frontend commands.
//!
//! All converters here are I/O-free and deterministic — they take owned
//! or borrowed events and return models. This makes them trivially
//! testable with hand-crafted events (see the `tests` module below).

use std::collections::{BTreeSet, HashMap};

use nostr::Event;
use serde_json::{json, Value};

use crate::models::*;

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
    // Prefer explicit ["public"] tag; fall back to NIP-29's absence-of-"private"
    // convention for relays that don't yet emit the explicit tag.
    let visibility = if has_tag(event, "public") {
        "open".to_string()
    } else if has_tag(event, "private") {
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
        Some(timestamp_to_iso(event.created_at.as_u64()))
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
    // Prefer explicit ["public"]; fall back to NIP-29 absence-of-"private".
    let visibility = if has_tag(event, "public") {
        "open".to_string()
    } else if has_tag(event, "private") {
        "private".to_string()
    } else {
        "open".to_string()
    };

    let created_at_iso = timestamp_to_iso(event.created_at.as_u64());

    let archived_at = if first_tag_value(event, "archived") == Some("true") {
        Some(timestamp_to_iso(event.created_at.as_u64()))
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
        let summary = UserProfileSummaryInfo {
            display_name: v
                .get("display_name")
                .and_then(Value::as_str)
                .or_else(|| v.get("name").and_then(Value::as_str))
                .map(str::to_string),
            avatar_url: v.get("picture").and_then(Value::as_str).map(str::to_string),
            nip05_handle: v.get("nip05").and_then(Value::as_str).map(str::to_string),
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

/// Convert kind:0 events (e.g. from a NIP-50 search) to [`SearchUsersResponse`].
/// Convert a single kind:0 event to a [`UserSearchResultInfo`].
pub fn user_search_result_from_event(ev: &Event) -> UserSearchResultInfo {
    let v: Value = serde_json::from_str(&ev.content).unwrap_or(Value::Null);
    UserSearchResultInfo {
        pubkey: ev.pubkey.to_hex(),
        display_name: v
            .get("display_name")
            .and_then(Value::as_str)
            .or_else(|| v.get("name").and_then(Value::as_str))
            .map(str::to_string),
        avatar_url: v.get("picture").and_then(Value::as_str).map(str::to_string),
        nip05_handle: v.get("nip05").and_then(Value::as_str).map(str::to_string),
    }
}

pub fn search_users_from_events(events: &[Event]) -> SearchUsersResponse {
    let users = events.iter().map(user_search_result_from_event).collect();
    SearchUsersResponse { users }
}

/// Rank and truncate kind:0 events from a NIP-50 search response for the
/// member-picker / DM-recipient autocomplete.
///
/// The relay returns results scored by Typesense BM25 against the whole kind:0
/// JSON content blob. That ranking is fine as a recall mechanism but not as
/// final ordering — a user whose `display_name` *is* the query should always
/// rank above someone whose `about` happens to mention it. We re-rank with a
/// small deterministic scoring function:
///
/// - exact match (case-insensitive)   > prefix match > substring match
/// - field priority: display_name (or name) > nip05 > pubkey hex
///
/// `limit` clamps the output. Pubkey de-duplication keeps only the
/// highest-scoring result per pubkey (Typesense should already return one doc
/// per event id, and kind:0 is a NIP-16 replaceable event so stale rows are
/// soft-deleted in the DB and filtered out before reaching us — this is
/// defense in depth in case both somehow slip through).
pub fn rank_user_search_results(
    events: &[Event],
    query: &str,
    limit: usize,
) -> SearchUsersResponse {
    let q = query.trim().to_lowercase();
    if q.is_empty() || limit == 0 {
        return SearchUsersResponse { users: Vec::new() };
    }

    // (score, input_index) — input index is a stable tiebreaker preserving
    // the relay's relevance order for ties.
    let mut scored: Vec<(u32, usize, UserSearchResultInfo)> = Vec::with_capacity(events.len());

    for (idx, ev) in events.iter().enumerate() {
        // Defensive: NIP-50 may return kinds we didn't expect if the relay
        // doesn't honor the `kinds` filter under search. Skip non-kind:0.
        if ev.kind.as_u16() != 0 {
            continue;
        }

        let info = user_search_result_from_event(ev);
        let display = info.display_name.as_deref().unwrap_or("").to_lowercase();
        let nip05 = info.nip05_handle.as_deref().unwrap_or("").to_lowercase();
        let pubkey = info.pubkey.to_lowercase();

        let score = match_score(&q, &display, &nip05, &pubkey);
        if score == 0 {
            // Typesense returned this as a content-blob match (e.g. `about`).
            // No useful name/nip05/pubkey hit — drop it from the autocomplete.
            continue;
        }
        scored.push((score, idx, info));
    }

    // Sort: higher score first; on tie, earlier input index (= higher Typesense relevance) first.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    // Dedupe by pubkey, keeping the first occurrence (already best-scored).
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut users: Vec<UserSearchResultInfo> = Vec::with_capacity(limit.min(scored.len()));
    for (_, _, info) in scored {
        if !seen.insert(info.pubkey.clone()) {
            continue;
        }
        users.push(info);
        if users.len() >= limit {
            break;
        }
    }

    SearchUsersResponse { users }
}

/// Score a user against a lowercased query.
///
/// Higher is better. Returns 0 if no field matches — caller should drop.
fn match_score(q: &str, display_name: &str, nip05: &str, pubkey_hex: &str) -> u32 {
    // Per-field tier scores. We assign large gaps so a substring match on
    // display_name still beats an exact match on a lower-priority field —
    // "the user named alice" is the autocomplete answer to "alice", not
    // "the user with alice@something.com".
    const DISPLAY_EXACT: u32 = 1000;
    const DISPLAY_PREFIX: u32 = 900;
    const DISPLAY_CONTAINS: u32 = 800;
    const NIP05_EXACT: u32 = 700;
    const NIP05_PREFIX: u32 = 600;
    const NIP05_CONTAINS: u32 = 500;
    const PUBKEY_PREFIX: u32 = 400;

    let score_field = |field: &str, exact: u32, prefix: u32, contains: u32| -> u32 {
        if field.is_empty() {
            0
        } else if field == q {
            exact
        } else if field.starts_with(q) {
            prefix
        } else if field.contains(q) {
            contains
        } else {
            0
        }
    };

    let display_score = score_field(
        display_name,
        DISPLAY_EXACT,
        DISPLAY_PREFIX,
        DISPLAY_CONTAINS,
    );
    let nip05_score = score_field(nip05, NIP05_EXACT, NIP05_PREFIX, NIP05_CONTAINS);
    let pubkey_score = if !pubkey_hex.is_empty() && pubkey_hex.starts_with(q) {
        PUBKEY_PREFIX
    } else {
        0
    };

    display_score.max(nip05_score).max(pubkey_score)
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
            created_at: ev.created_at.as_u64() as i64,
            content: ev.content.clone(),
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
        created_at: event.created_at.as_u64() as i64,
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
                created_at: ev.created_at.as_u64(),
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
            // Always overwrite the pubkey with the event author — it's the
            // authoritative source even if the content claims otherwise.
            if let Some(obj) = v.as_object_mut() {
                obj.insert("pubkey".to_string(), json!(ev.pubkey.to_hex()));
            } else {
                v = json!({ "pubkey": ev.pubkey.to_hex() });
            }
            v
        })
        .collect();
    json!({ "agents": arr })
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
fn timestamp_to_iso(secs: u64) -> String {
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
    fn channel_info_private_when_private_tag_present() {
        // Explicit ["private"] tag → private (NIP-29 convention).
        let e = ev(
            39000,
            "",
            vec![
                vec!["d", "u"],
                vec!["name", "n"],
                vec!["t", "forum"],
                vec!["private"],
            ],
        );
        let info = channel_info_from_event(&e, None, None).unwrap();
        assert_eq!(info.visibility, "private");
        assert_eq!(info.channel_type, "forum");
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
            ],
        );
        let d = channel_detail_from_event(&e).unwrap();
        assert_eq!(d.id, "uuid");
        assert_eq!(d.topic.as_deref(), Some("tt"));
        assert_eq!(d.purpose.as_deref(), Some("pp"));
        assert_eq!(d.channel_type, "dm");
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
    fn search_users_maps_each_event() {
        let e1 = ev(0, r#"{"name":"a"}"#, vec![]);
        let e2 = ev(0, r#"{"display_name":"B"}"#, vec![]);
        let r = search_users_from_events(&[e1, e2]);
        assert_eq!(r.users.len(), 2);
        assert_eq!(r.users[0].display_name.as_deref(), Some("a"));
        assert_eq!(r.users[1].display_name.as_deref(), Some("B"));
    }

    // ── rank_user_search_results ────────────────────────────────────────────

    #[test]
    fn rank_empty_query_returns_empty() {
        let e = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let r = rank_user_search_results(&[e], "  ", 10);
        assert!(r.users.is_empty());
    }

    #[test]
    fn rank_zero_limit_returns_empty() {
        let e = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let r = rank_user_search_results(&[e], "alice", 0);
        assert!(r.users.is_empty());
    }

    #[test]
    fn rank_skips_non_kind_zero_events() {
        // Even if a buggy relay returns non-kind:0 hits under a kinds:[0] filter,
        // we shouldn't surface them as users.
        let e = ev(1, "alice", vec![]);
        let r = rank_user_search_results(&[e], "alice", 10);
        assert!(r.users.is_empty());
    }

    #[test]
    fn rank_skips_results_with_no_name_or_nip05_match() {
        // Typesense matched the kind:0 because "alice" appears in the `about`
        // field. We don't want that in autocomplete — there's no useful name
        // to display against the query.
        let e = ev(
            0,
            r#"{"display_name":"Bob","nip05":"bob@example.com","about":"I work with alice"}"#,
            vec![],
        );
        let r = rank_user_search_results(&[e], "alice", 10);
        assert!(r.users.is_empty());
    }

    #[test]
    fn rank_exact_display_name_beats_substring_match() {
        let exact = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let sub = ev(0, r#"{"display_name":"alice-the-second"}"#, vec![]);
        // Put the substring match first to prove ordering isn't input-driven.
        let r = rank_user_search_results(&[sub, exact], "alice", 10);
        assert_eq!(r.users.len(), 2);
        assert_eq!(r.users[0].display_name.as_deref(), Some("alice"));
        assert_eq!(r.users[1].display_name.as_deref(), Some("alice-the-second"));
    }

    #[test]
    fn rank_display_name_beats_nip05_match_of_same_tier() {
        // "alice" exact on display_name vs. "alice" prefix on nip05 — display wins.
        let by_name = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let by_nip05 = ev(0, r#"{"display_name":"X","nip05":"alice@x.com"}"#, vec![]);
        let r = rank_user_search_results(&[by_nip05, by_name], "alice", 10);
        assert_eq!(r.users[0].display_name.as_deref(), Some("alice"));
    }

    #[test]
    fn rank_display_substring_still_beats_nip05_exact() {
        // Asserts the field-priority gap is large enough that even a display
        // substring outranks a perfect nip05 hit.
        let by_name_sub = ev(0, r#"{"display_name":"my-alice-account"}"#, vec![]);
        let by_nip05_exact = ev(0, r#"{"display_name":"Bob","nip05":"alice"}"#, vec![]);
        let r = rank_user_search_results(&[by_nip05_exact, by_name_sub], "alice", 10);
        assert_eq!(r.users[0].display_name.as_deref(), Some("my-alice-account"));
    }

    #[test]
    fn rank_pubkey_prefix_match() {
        // The query is a hex prefix; the kind:0 has no name match at all.
        // Build the event then read its pubkey hex; use the first 8 chars as
        // the query so we know the prefix matches.
        let e = ev(0, r#"{"display_name":"unrelated"}"#, vec![]);
        let prefix: String = e.pubkey.to_hex().chars().take(8).collect();
        let r = rank_user_search_results(std::slice::from_ref(&e), &prefix, 10);
        assert_eq!(r.users.len(), 1);
        assert_eq!(r.users[0].pubkey, e.pubkey.to_hex());
    }

    #[test]
    fn rank_dedupes_by_pubkey() {
        // Same author publishing two kind:0s (e.g. one stale row slipping
        // through). Dedupe should keep only one entry — the higher-scored one.
        let keys = nostr::Keys::generate();
        let mk = |content: &str| {
            EventBuilder::new(Kind::from_u16(0), content)
                .sign_with_keys(&keys)
                .expect("sign")
        };
        // First event is a substring match on display_name; second is an exact match.
        let weak = mk(r#"{"display_name":"alice-old"}"#);
        let strong = mk(r#"{"display_name":"alice"}"#);
        let r = rank_user_search_results(&[weak, strong], "alice", 10);
        assert_eq!(r.users.len(), 1);
        assert_eq!(r.users[0].display_name.as_deref(), Some("alice"));
    }

    #[test]
    fn rank_respects_limit() {
        let events: Vec<Event> = (0..5)
            .map(|i| ev(0, &format!(r#"{{"display_name":"alice{}"}}"#, i), vec![]))
            .collect();
        let r = rank_user_search_results(&events, "alice", 3);
        assert_eq!(r.users.len(), 3);
    }

    #[test]
    fn rank_case_insensitive() {
        let e = ev(0, r#"{"display_name":"AlIcE"}"#, vec![]);
        let r = rank_user_search_results(&[e], "alice", 10);
        assert_eq!(r.users.len(), 1);
        let r =
            rank_user_search_results(&[ev(0, r#"{"display_name":"alice"}"#, vec![])], "ALICE", 10);
        assert_eq!(r.users.len(), 1);
    }

    #[test]
    fn rank_tiebreak_preserves_relay_relevance_order() {
        // Two equally-scored matches → first one in input wins.
        let first = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let second = ev(0, r#"{"display_name":"alice"}"#, vec![]);
        let first_pk = first.pubkey.to_hex();
        let r = rank_user_search_results(&[first, second], "alice", 10);
        assert_eq!(r.users.len(), 2);
        assert_eq!(r.users[0].pubkey, first_pk);
    }

    #[test]
    fn rank_falls_back_to_name_when_display_name_absent() {
        // user_search_result_from_event already prefers display_name then name —
        // make sure the ranker sees the populated display_name and scores it.
        let e = ev(0, r#"{"name":"alice"}"#, vec![]);
        let r = rank_user_search_results(&[e], "alice", 10);
        assert_eq!(r.users.len(), 1);
        assert_eq!(r.users[0].display_name.as_deref(), Some("alice"));
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
        let v = agents_from_events(&[e.clone()]);
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
        let v = agents_from_events(&[e.clone()]);
        let arr = v.get("agents").and_then(Value::as_array).unwrap();
        assert_eq!(
            arr[0].get("pubkey").and_then(Value::as_str).unwrap(),
            e.pubkey.to_hex()
        );
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
