use tauri::State;

use crate::{
    app_state::AppState,
    events,
    models::{ChannelDetailInfo, ChannelInfo, ChannelMembersResponse},
    nostr_convert,
    relay::{query_relay, submit_event},
};

// ── Serverless membership (read-modify-write of kind:39002) ──────────────────
//
// On a generic relay there's no server to process the membership command kinds
// (9021 join, 9022 leave, 9000 add-member, 9001 remove-member). Instead the
// client mutates the replaceable kind:39002 member-list event directly: read
// the current list, add/remove the pubkey, and re-publish the whole event.
// See docs/SPROUT_LITE_MODE.md.

/// Resolve an addressable/replaceable event to its single authoritative copy.
///
/// kind:39002 (membership) and kind:39000 (metadata) are NIP-01 addressable
/// events: exactly one *logical* event exists per `(kind, author, d-tag)`, and
/// re-publishing supersedes the prior version. In a multi-relay world, however,
/// relays can disagree — a write may land on relays A and B but be dropped
/// (rate-limited, offline) by relay C, leaving C with a **stale** older copy.
/// A query that fans out and merges then sees *both* versions.
///
/// The correct resolution per NIP-01 is "latest wins": pick the event with the
/// greatest `created_at` (tie-break deterministically by event id). Picking an
/// arbitrary event (e.g. `first()` after a `limit: 1` merge) is a bug — it makes
/// the member list non-deterministic and, worse, makes read-modify-write
/// membership updates clobber members that only exist in the newer copy.
///
/// Returns `None` for an empty slice.
fn latest_event(events: &[nostr::Event]) -> Option<&nostr::Event> {
    events.iter().max_by(|a, b| {
        a.created_at
            .as_secs()
            .cmp(&b.created_at.as_secs())
            // Deterministic tie-break when two copies share a timestamp.
            .then_with(|| a.id.to_hex().cmp(&b.id.to_hex()))
    })
}

/// Group a batch of addressable events by `d`-tag and keep only the latest copy
/// of each (see [`latest_event`] for why "latest" not "arbitrary"). Events
/// without a `d` tag are skipped.
fn latest_by_d_tag(events: &[nostr::Event]) -> std::collections::HashMap<String, &nostr::Event> {
    let mut by_d: std::collections::HashMap<String, &nostr::Event> =
        std::collections::HashMap::new();
    for ev in events {
        let Some(d) = ev.tags.iter().find_map(|t| {
            let s = t.as_slice();
            (s.len() >= 2 && s[0] == "d").then(|| s[1].clone())
        }) else {
            continue;
        };
        by_d.entry(d)
            .and_modify(|cur| {
                let newer = ev.created_at.as_secs() > cur.created_at.as_secs()
                    || (ev.created_at.as_secs() == cur.created_at.as_secs()
                        && ev.id.to_hex() > cur.id.to_hex());
                if newer {
                    *cur = ev;
                }
            })
            .or_insert(ev);
    }
    by_d
}

/// Fetch the current members `(pubkey, role)` for a serverless channel from its
/// kind:39002 event. Role is the 4th element of the `p` tag (NIP-29), defaulting
/// to `member`. Returns an empty list if no members event exists yet.
async fn serverless_current_members(
    state: &AppState,
    channel_id: &str,
) -> Result<Vec<(String, String)>, String> {
    // No `limit: 1`: relays can hold divergent copies of this replaceable
    // event (a write dropped by one relay leaves it stale). Fetch all copies
    // and resolve to the latest by `created_at` — otherwise a read-modify-write
    // membership update can be based on a stale list and silently drop members.
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [39002],
            "#d": [channel_id],
        })],
    )
    .await?;

    let Some(ev) = latest_event(&events) else {
        return Ok(Vec::new());
    };
    let members = ev
        .tags
        .iter()
        .filter_map(|t| {
            let parts = t.as_slice();
            if parts.first().map(String::as_str) == Some("p") {
                let pk = parts.get(1)?.to_ascii_lowercase();
                let role = parts
                    .get(3)
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_else(|| "member".to_string());
                Some((pk, role))
            } else {
                None
            }
        })
        .collect();
    Ok(members)
}

/// Re-publish the kind:39002 member list for a serverless channel after adding
/// or removing the given pubkeys. Existing members keep their roles; newly
/// added pubkeys get the `add_role` (default `member`).
async fn serverless_set_members(
    state: &AppState,
    channel_id: &str,
    add: &[String],
    remove: &[String],
    add_role: &str,
) -> Result<(), String> {
    let mut members = serverless_current_members(state, channel_id).await?;
    for pk in remove {
        let pk = pk.to_ascii_lowercase();
        members.retain(|(m, _)| m != &pk);
    }
    for pk in add {
        let pk = pk.to_ascii_lowercase();
        if !members.iter().any(|(m, _)| m == &pk) {
            members.push((pk, add_role.to_string()));
        }
    }
    members.sort();
    members.dedup_by(|a, b| a.0 == b.0);

    let builder = events::build_channel_members_serverless_with_roles(channel_id, &members)?;
    submit_event(builder, state).await?;
    Ok(())
}

// ── Reads (pure-nostr via /query) ────────────────────────────────────────────

#[tauri::command]
pub async fn get_channels(state: State<'_, AppState>) -> Result<Vec<ChannelInfo>, String> {
    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    // Step 1: find all kind:39002 (members) events that mention me, then
    // pull the channel ids out of their `d` tags.
    let member_events = {
        let mut all = Vec::new();
        let mut until: Option<u64> = None;
        loop {
            let mut f = serde_json::json!({"kinds": [39002], "#p": [&my_pubkey], "limit": 500});
            if let Some(u) = until {
                f["until"] = serde_json::json!(u);
            }
            let page = query_relay(&state, &[f]).await?;
            let done = page.len() < 500;
            if let Some(t) = page.iter().map(|e| e.created_at.as_secs()).min() {
                until = Some(t.saturating_sub(1));
            }
            all.extend(page);
            if done {
                break;
            }
        }
        all
    };

    let mut channel_ids: Vec<String> = member_events
        .iter()
        .filter_map(|ev| {
            ev.tags.iter().find_map(|t| {
                let s = t.as_slice();
                if s.len() >= 2 && s[0] == "d" {
                    Some(s[1].clone())
                } else {
                    None
                }
            })
        })
        .collect();
    channel_ids.sort();
    channel_ids.dedup();

    // Step 2: fetch channel metadata events (kind:39000) for member channels.
    // kind:39000 is addressable: one logical event per `d` tag. We do NOT cap
    // the limit at `channel_ids.len()`: across multiple relays each channel can
    // return several copies (fresh + stale), so a tight limit can truncate and
    // drop the fresh copy of one channel while keeping a stale copy of another.
    // Over-fetch, then resolve to the latest copy per `d` tag below.
    let meta_events_raw = if !channel_ids.is_empty() {
        query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [39000],
                "#d": channel_ids,
            })],
        )
        .await?
    } else {
        Vec::new()
    };
    // Deduplicate to the latest copy per channel (relays may disagree).
    let meta_events: Vec<nostr::Event> = latest_by_d_tag(&meta_events_raw)
        .into_values()
        .cloned()
        .collect();

    // Step 3: fetch open channel metadata so the channel browser can show
    // discoverable channels the user hasn't joined yet. The relay's access
    // control allows reading kind:39000 for open channels regardless of membership.
    //
    // SERVERLESS: skip this. A generic public relay has no Sprout-specific
    // notion of "our" channels — an unfiltered kind:39000 query returns the
    // ENTIRE network's channels (hundreds of unrelated test channels from
    // damus/nos.lol), which is slow and floods the sidebar with junk. In
    // serverless mode you only see channels you're a member of (Step 1/2).
    let open_meta_events = if state.is_serverless() {
        Vec::new()
    } else {
        query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [39000],
                "limit": 5000,
            })],
        )
        .await?
    };

    // Merge: member channels (marked as member) + open channels (not yet joined).
    let member_d_tags: std::collections::HashSet<String> = meta_events
        .iter()
        .filter_map(|ev| {
            ev.tags.iter().find_map(|t| {
                let s = t.as_slice();
                if s.len() >= 2 && s[0] == "d" {
                    Some(s[1].clone())
                } else {
                    None
                }
            })
        })
        .collect();

    let mut channels = Vec::with_capacity(meta_events.len() + open_meta_events.len());
    for ev in &meta_events {
        if let Ok(info) = nostr_convert::channel_info_from_event(ev, None, Some(true)) {
            channels.push(info);
        }
    }
    for ev in &open_meta_events {
        // Skip channels already included from the member set.
        let d_tag = ev.tags.iter().find_map(|t| {
            let s = t.as_slice();
            if s.len() >= 2 && s[0] == "d" {
                Some(s[1].clone())
            } else {
                None
            }
        });
        if let Some(ref d) = d_tag {
            if member_d_tags.contains(d) {
                continue;
            }
        }
        if let Ok(info) = nostr_convert::channel_info_from_event(ev, None, Some(false)) {
            channels.push(info);
        }
    }

    // Populate member_count by batch-fetching kind:39002 for every listed
    // channel and counting unique p-tag pubkeys. The kind:40901 summary
    // sidecar that channel_info_from_event prefers isn't emitted by the
    // relay today, so without this step every channel reports 0 members
    // in the channel browser (the active-channel top bar masks this with
    // its own live members query).
    let all_d_tags: Vec<String> = channels.iter().map(|c| c.id.clone()).collect();
    if !all_d_tags.is_empty() {
        let members_events = query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [39002],
                "#d": all_d_tags,
                "limit": all_d_tags.len(),
            })],
        )
        .await
        .unwrap_or_default();

        let membership = collect_members_by_channel(&members_events);
        for channel in &mut channels {
            if let Some(info) = membership.get(&channel.id) {
                channel.member_count = info.count;
                channel.member_pubkeys = info.pubkeys.clone();
            }
        }
    }

    // Populate last_message_at by fetching the most recent human message per
    // channel. Uses per-channel filters (single #h value each) so the relay can
    // push the query to its indexed channel_id column. Multi-value #h is NOT
    // SQL-pushed and would silently drop quieter channels under the global limit.
    let channel_ids: Vec<String> = channels.iter().map(|c| c.id.clone()).collect();
    if !channel_ids.is_empty() {
        let filters: Vec<serde_json::Value> = channel_ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "kinds": [9, 40002],
                    "#h": [id],
                    "limit": 1
                })
            })
            .collect();

        let message_events = query_relay(&state, &filters).await.unwrap_or_default();

        let mut last_message_by_channel: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();
        for ev in &message_events {
            if let Some(ch_id) = ev.tags.iter().find_map(|t| {
                let s = t.as_slice();
                (s.len() >= 2 && s[0] == "h").then(|| s[1].clone())
            }) {
                let ts = ev.created_at.as_secs();
                last_message_by_channel
                    .entry(ch_id)
                    .and_modify(|existing| {
                        if ts > *existing {
                            *existing = ts;
                        }
                    })
                    .or_insert(ts);
            }
        }

        for channel in &mut channels {
            if let Some(&ts) = last_message_by_channel.get(&channel.id) {
                channel.last_message_at = Some(nostr_convert::timestamp_to_iso(ts));
            }
        }
    }

    // NIP-DV: drop DMs the viewer has hidden. The relay maintains a per-viewer
    // parameterized-replaceable snapshot (kind:30622, d=my pubkey) whose `h`
    // tags list currently-hidden DM channel ids. The snapshot also carries
    // `p`=my pubkey so the relay's #p read-gate scopes it to me; we query by
    // `#p` for that reason. Reading the latest one is the only way the client
    // learns hide state, which the relay tracks privately.
    let hidden_dms: std::collections::HashSet<String> = {
        let events = query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [buzz_core_pkg::kind::KIND_DM_VISIBILITY],
                "#p": [&my_pubkey],
                "limit": 1,
            })],
        )
        .await
        .unwrap_or_default();
        events
            .iter()
            .max_by_key(|e| e.created_at.as_secs())
            .map(|e| {
                e.tags
                    .iter()
                    .filter_map(|t| {
                        let s = t.as_slice();
                        (s.len() >= 2 && s[0] == "h").then(|| s[1].clone())
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    if !hidden_dms.is_empty() {
        channels.retain(|c| c.channel_type != "dm" || !hidden_dms.contains(&c.id));
    }

    Ok(channels)
}

struct ChannelMembership {
    count: i64,
    pubkeys: Vec<String>,
}

/// Build a `channel_id → membership` map from a batch of kind:39002 events.
/// Events without a `d` tag are skipped; member dedupe is delegated to
/// [`nostr_convert::channel_members_from_event`] so the parsing rules match the
/// per-channel `get_channel_members` path.
fn collect_members_by_channel(
    events: &[nostr::Event],
) -> std::collections::HashMap<String, ChannelMembership> {
    // Resolve each channel's `d`-tag to its latest 39002 across relays first;
    // a naive per-event insert would let a stale copy overwrite the fresh one
    // depending on iteration order.
    let latest = latest_by_d_tag(events);
    let mut map: std::collections::HashMap<String, ChannelMembership> =
        std::collections::HashMap::with_capacity(latest.len());
    for (d, ev) in latest {
        let Ok(resp) = nostr_convert::channel_members_from_event(ev) else {
            continue;
        };
        let pubkeys: Vec<String> = resp.members.iter().map(|m| m.pubkey.clone()).collect();
        map.insert(
            d,
            ChannelMembership {
                count: pubkeys.len() as i64,
                pubkeys,
            },
        );
    }
    map
}

#[tauri::command]
pub async fn get_channel_details(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<ChannelDetailInfo, String> {
    // Resolve the latest copy across relays (kind:39000 is replaceable too).
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": [channel_id],
        })],
    )
    .await?;

    latest_event(&events)
        .map(nostr_convert::channel_detail_from_event)
        .transpose()?
        .ok_or_else(|| "channel not found".to_string())
}

#[tauri::command]
pub async fn get_channel_members(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<ChannelMembersResponse, String> {
    // Fetch all copies and resolve the latest (relays may disagree); see
    // `latest_event`. A stale `limit: 1` pick would hide members added by a
    // write that one relay dropped.
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [39002],
            "#d": [channel_id],
        })],
    )
    .await?;

    let mut response = latest_event(&events)
        .map(nostr_convert::channel_members_from_event)
        .transpose()?
        .ok_or_else(|| "channel members not found".to_string())?;

    // Batch-fetch kind:0 profiles to populate display names.
    let pubkeys: Vec<String> = response.members.iter().map(|m| m.pubkey.clone()).collect();
    if !pubkeys.is_empty() {
        let profile_events = query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [0],
                "authors": pubkeys,
                "limit": pubkeys.len()
            })],
        )
        .await
        .unwrap_or_default();

        // Build pubkey → profile display metadata from kind:0 events.
        let mut profile_map = std::collections::HashMap::new();
        for ev in &profile_events {
            let pk = ev.pubkey.to_hex();
            if let Ok(profile) = nostr_convert::profile_info_from_event(ev) {
                profile_map.insert(
                    pk,
                    (
                        profile.display_name,
                        nostr_convert::profile_has_valid_oa_owner(ev),
                    ),
                );
            }
        }

        // Populate profile-derived fields on each member.
        for member in &mut response.members {
            if member.role == "bot" {
                member.is_agent = true;
            }
            if let Some((display_name, is_agent)) = profile_map.get(&member.pubkey) {
                if member.display_name.is_none() {
                    member.display_name = display_name.clone();
                }
                member.is_agent = member.is_agent || *is_agent;
            }
        }
    }

    Ok(response)
}

// ── Writes (signed events) ──────────────────────────────────────────────────

fn parse_channel_uuid(channel_id: &str) -> Result<uuid::Uuid, String> {
    uuid::Uuid::parse_str(channel_id).map_err(|_| format!("invalid channel UUID: {channel_id}"))
}

#[tauri::command]
pub async fn create_channel(
    name: String,
    channel_type: String,
    visibility: String,
    description: Option<String>,
    ttl_seconds: Option<i32>,
    state: State<'_, AppState>,
) -> Result<ChannelInfo, String> {
    let channel_uuid = uuid::Uuid::new_v4();

    let vis = match visibility.as_str() {
        "open" | "private" => visibility.as_str(),
        other => return Err(format!("invalid visibility: {other}")),
    };
    let ct = match channel_type.as_str() {
        "stream" | "forum" => channel_type.as_str(),
        other => return Err(format!("invalid channel_type: {other}")),
    };

    let channel_uuid_string = channel_uuid.to_string();

    if state.is_serverless() {
        // No relay to process a kind:9007 command — publish the kind:39000
        // metadata and a kind:39002 membership (self) directly so the channel
        // is discoverable via get_channels. Build ChannelInfo from the locally
        // signed metadata event rather than re-querying the relay, which can
        // race against propagation/indexing lag (the "nothing happened" bug).
        let (my_pubkey, keys) = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            (keys.public_key().to_hex(), keys.clone())
        };
        let meta = events::build_channel_metadata_serverless(
            &channel_uuid_string,
            &name,
            vis,
            ct,
            description.as_deref(),
            &[],
        )?;
        let meta_event = meta
            .clone()
            .sign_with_keys(&keys)
            .map_err(|e| format!("failed to sign channel metadata: {e}"))?;
        submit_event(meta, &state).await?;
        // The creator is the channel owner.
        let members = events::build_channel_members_serverless_with_roles(
            &channel_uuid_string,
            &[(my_pubkey, "owner".to_string())],
        )?;
        submit_event(members, &state).await?;
        // Return ChannelInfo derived from the event we just published (member,
        // since we created it). No relay round-trip required.
        return nostr_convert::channel_info_from_event(&meta_event, None, Some(true));
    } else {
        let builder = events::build_create_channel(
            channel_uuid,
            &name,
            vis,
            ct,
            description.as_deref(),
            ttl_seconds,
        )?;
        submit_event(builder, &state).await?;
    }

    // Re-fetch the canonical metadata event to return ChannelInfo.
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": [channel_uuid_string],
            "limit": 1
        })],
    )
    .await?;

    events
        .first()
        .map(|ev| nostr_convert::channel_info_from_event(ev, None, None))
        .transpose()?
        .ok_or_else(|| "channel created but metadata not yet available".to_string())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChannelInput {
    pub channel_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    /// Absent = leave unchanged, `null` = clear (permanent), seconds = set.
    #[serde(default, deserialize_with = "crate::util::double_option")]
    pub ttl_seconds: Option<Option<i32>>,
}

#[tauri::command]
pub async fn update_channel(
    input: UpdateChannelInput,
    state: State<'_, AppState>,
) -> Result<ChannelDetailInfo, String> {
    let uuid = parse_channel_uuid(&input.channel_id)?;
    let builder = events::build_update_channel(
        uuid,
        input.name.as_deref(),
        input.description.as_deref(),
        input.visibility.as_deref(),
        input.ttl_seconds,
    )?;
    submit_event(builder, &state).await?;

    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": [input.channel_id],
            "limit": 1
        })],
    )
    .await?;

    events
        .first()
        .map(nostr_convert::channel_detail_from_event)
        .transpose()?
        .ok_or_else(|| "channel updated but metadata not yet available".to_string())
}

#[tauri::command]
pub async fn set_channel_topic(
    channel_id: String,
    topic: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    let builder = events::build_set_topic(uuid, &topic)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn set_channel_purpose(
    channel_id: String,
    purpose: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    let builder = events::build_set_purpose(uuid, &purpose)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn archive_channel(channel_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    let builder = events::build_archive(uuid)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn unarchive_channel(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    let builder = events::build_unarchive(uuid)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn delete_channel(channel_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    if state.is_serverless() {
        // No relay to process the kind:9008 delete command. Publish a NIP-09
        // (kind 5) deletion targeting the channel's addressable 39000/39002
        // coordinates so relays drop them. Only the owner (signer of those
        // events) can effectively delete.
        let my_pubkey = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            keys.public_key().to_hex()
        };
        let builder = events::build_delete_channel_serverless(&uuid.to_string(), &my_pubkey)?;
        submit_event(builder, &state).await?;
        return Ok(());
    }
    let builder = events::build_delete_channel(uuid)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn add_channel_members(
    channel_id: String,
    pubkeys: Vec<String>,
    role: Option<String>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    let role_str = match role.as_deref() {
        Some("admin") => Some("admin"),
        Some("bot") => Some("bot"),
        Some("guest") => Some("guest"),
        Some("member") | None => None,
        Some(other) => return Err(format!("invalid role: {other}")),
    };

    if state.is_serverless() {
        // Validate pubkeys, then add them all to the kind:39002 member list.
        let valid: Vec<String> = pubkeys
            .iter()
            .filter(|p| p.len() == 64 && p.chars().all(|c| c.is_ascii_hexdigit()))
            .map(|p| p.to_ascii_lowercase())
            .collect();
        serverless_set_members(
            &state,
            &uuid.to_string(),
            &valid,
            &[],
            role_str.unwrap_or("member"),
        )
        .await?;
        return Ok(serde_json::json!({ "added": valid, "errors": [] }));
    }

    let mut added = Vec::new();
    let mut errors = Vec::<serde_json::Value>::new();

    for pubkey in &pubkeys {
        let builder = match events::build_add_member(uuid, pubkey, role_str) {
            Ok(b) => b,
            Err(e) => {
                errors.push(serde_json::json!({"pubkey": pubkey, "error": e}));
                continue;
            }
        };
        match submit_event(builder, &state).await {
            Ok(_) => added.push(pubkey.clone()),
            Err(e) => errors.push(serde_json::json!({"pubkey": pubkey, "error": e})),
        }
    }

    Ok(serde_json::json!({ "added": added, "errors": errors }))
}

#[tauri::command]
pub async fn remove_channel_member(
    channel_id: String,
    pubkey: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    if state.is_serverless() {
        return serverless_set_members(&state, &uuid.to_string(), &[], &[pubkey], "member").await;
    }
    let builder = events::build_remove_member(uuid, &pubkey)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn change_channel_member_role(
    channel_id: String,
    pubkey: String,
    role: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    // Only allow permission-tier roles for humans and bot/guest for bots.
    // Owner changes require a dedicated transfer-ownership flow.
    let role_str = match role.as_str() {
        "admin" | "member" | "guest" | "bot" => role.as_str(),
        "owner" => return Err("cannot assign owner role — use transfer ownership".into()),
        other => return Err(format!("invalid role: {other}")),
    };
    if state.is_serverless() {
        // Re-add with the requested role: remove then re-add (set_members
        // processes removals before additions), so the new role takes effect.
        let targets = [pubkey];
        return serverless_set_members(&state, &uuid.to_string(), &targets, &targets, role_str)
            .await;
    }
    let builder = events::build_add_member(uuid, &pubkey, Some(role_str))?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn join_channel(channel_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    if state.is_serverless() {
        let me = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            keys.public_key().to_hex()
        };
        return serverless_set_members(&state, &uuid.to_string(), &[me], &[], "member").await;
    }
    let builder = events::build_join(uuid)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn leave_channel(channel_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    if state.is_serverless() {
        let me = {
            let keys = state.keys.lock().map_err(|e| e.to_string())?;
            keys.public_key().to_hex()
        };
        return serverless_set_members(&state, &uuid.to_string(), &[], &[me], "member").await;
    }
    let builder = events::build_leave(uuid)?;
    submit_event(builder, &state).await?;
    Ok(())
}

/// Fetch channel message history over the multi-relay pool (serverless).
///
/// The live-WS read path (`relayClient`) connects to a single relay, which
/// split-brains against the multi-relay *write* path (a message published to
/// nos.lol is invisible to a read subscription on damus). This command queries
/// the same relay set used for writes and merges/dedups results, so reads and
/// writes converge. Returns events as JSON (the same shape the live WS yields).
#[tauri::command]
pub async fn query_channel_messages(
    channel_id: String,
    kinds: Vec<u16>,
    limit: usize,
    until: Option<u64>,
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut filter = serde_json::json!({
        "kinds": kinds,
        "#h": [channel_id],
        "limit": limit,
    });
    if let Some(u) = until {
        filter["until"] = serde_json::json!(u);
    }
    let events = query_relay(&state, &[filter]).await?;
    Ok(events
        .iter()
        .map(|ev| serde_json::to_value(ev).unwrap_or(serde_json::Value::Null))
        .filter(|v| !v.is_null())
        .collect())
}

/// Open a persistent live subscription for a channel across ALL relays
/// (serverless). Each new matching event is emitted to the frontend as a
/// `serverless-event:<channel_id>` Tauri event. Returns the subscription id;
/// pass it to `unsubscribe_channel_messages` to tear down.
///
/// This gives standard Nostr realtime in serverless mode: we subscribe to every
/// relay at once and merge, so a message that landed on relay B (because relay A
/// rate-limited the write) still streams back live — no polling, no split-brain.
#[tauri::command]
pub async fn subscribe_channel_messages(
    channel_id: String,
    kinds: Vec<u16>,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    use tauri::Emitter;

    let relay_urls = crate::relay::relay_ws_urls_with_override(&state);
    let keys = {
        let guard = state.keys.lock().map_err(|e| e.to_string())?;
        guard.clone()
    };
    let filter = serde_json::json!({
        "kinds": kinds,
        "#h": [channel_id],
        "since": chrono::Utc::now().timestamp(),
        "limit": 0,
    });

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<nostr::Event>();
    let sub_id = state
        .relay_pool
        .subscribe(&relay_urls, &keys, filter, tx)
        .await;

    // Forward events to the frontend, deduping by id (the same event arrives
    // from multiple relays). The task ends when the sender is dropped on
    // unsubscribe / pool clear.
    let event_name = format!("serverless-event:{channel_id}");
    tokio::spawn(async move {
        let mut seen = std::collections::HashSet::new();
        while let Some(ev) = rx.recv().await {
            if !seen.insert(ev.id.to_hex()) {
                continue;
            }
            if let Ok(v) = serde_json::to_value(&ev) {
                let _ = app.emit(&event_name, v);
            }
        }
    });

    Ok(sub_id)
}

/// Tear down a live subscription opened by `subscribe_channel_messages`.
#[tauri::command]
pub async fn unsubscribe_channel_messages(
    sub_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.relay_pool.unsubscribe(&sub_id).await;
    Ok(())
}

#[cfg(test)]
#[path = "channels_tests.rs"]
mod tests;
