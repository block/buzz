use tauri::State;

use crate::{
    app_state::AppState,
    events,
    models::{ChannelDetailInfo, ChannelInfo, ChannelMembersResponse},
    nostr_convert,
    relay::{query_relay, submit_event},
};

// ── Reads (pure-nostr via /query) ────────────────────────────────────────────

#[tauri::command]
pub async fn get_channels(state: State<'_, AppState>) -> Result<Vec<ChannelInfo>, String> {
    get_channels_internal(state, true).await
}

pub(super) async fn get_channels_including_chats(
    state: State<'_, AppState>,
) -> Result<Vec<ChannelInfo>, String> {
    get_channels_internal(state, false).await
}

async fn get_channels_internal(
    state: State<'_, AppState>,
    exclude_chat_channels: bool,
) -> Result<Vec<ChannelInfo>, String> {
    let _profile_start = std::time::Instant::now();
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

    #[cfg(debug_assertions)]
    let t_members = _profile_start.elapsed();

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
    // kind:39000 is addressable: exactly one event per `d` tag, so a limit
    // equal to the number of ids is both necessary and sufficient. Without
    // an explicit limit, multi-value `#d` filters fall through to the relay's
    // default LIMIT and can drop results when there are many channels.
    let meta_events = if !channel_ids.is_empty() {
        query_relay(
            &state,
            &[serde_json::json!({
                "kinds": [39000],
                "#d": channel_ids,
                "limit": channel_ids.len(),
            })],
        )
        .await?
    } else {
        Vec::new()
    };

    #[cfg(debug_assertions)]
    let t_member_meta = _profile_start.elapsed();

    // Step 3: fetch ALL open channel metadata so the channel browser can show
    // discoverable channels the user hasn't joined yet. The relay's access
    // control allows reading kind:39000 for open channels regardless of membership.
    let open_meta_events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [39000],
            "limit": 5000,
        })],
    )
    .await?;

    #[cfg(debug_assertions)]
    let t_open_meta = _profile_start.elapsed();

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

    #[cfg(debug_assertions)]
    let t_member_counts = _profile_start.elapsed();

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

    #[cfg(debug_assertions)]
    let t_last_message = _profile_start.elapsed();

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
    if exclude_chat_channels {
        let chat_channel_ids = super::chats::fetch_chat_metadata_channel_ids(&state)
            .await
            .unwrap_or_default();
        if !chat_channel_ids.is_empty() {
            channels.retain(|c| c.channel_type != "chat" && !chat_channel_ids.contains(&c.id));
        }
    }

    #[cfg(debug_assertions)]
    {
        let total = _profile_start.elapsed();
        eprintln!(
            "buzz-desktop: get_channels profile channels={} members={:?} member_meta={:?} open_meta={:?} member_counts={:?} last_message={:?} hidden_dm={:?} total={:?}",
            channels.len(),
            t_members,
            t_member_meta - t_members,
            t_open_meta - t_member_meta,
            t_member_counts - t_open_meta,
            t_last_message - t_member_counts,
            total - t_last_message,
            total,
        );
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
    let mut map: std::collections::HashMap<String, ChannelMembership> =
        std::collections::HashMap::with_capacity(events.len());
    for ev in events {
        let Some(d) = ev.tags.iter().find_map(|t| {
            let s = t.as_slice();
            (s.len() >= 2 && s[0] == "d").then(|| s[1].clone())
        }) else {
            continue;
        };
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
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": [channel_id],
            "limit": 1
        })],
    )
    .await?;

    events
        .first()
        .map(nostr_convert::channel_detail_from_event)
        .transpose()?
        .ok_or_else(|| "channel not found".to_string())
}

#[tauri::command]
pub async fn get_channel_members(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<ChannelMembersResponse, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [39002],
            "#d": [channel_id],
            "limit": 1
        })],
    )
    .await?;

    let mut response = events
        .first()
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

    let builder = events::build_create_channel(
        channel_uuid,
        &name,
        vis,
        ct,
        description.as_deref(),
        ttl_seconds,
    )?;
    submit_event(builder, &state).await?;

    // Re-fetch the canonical metadata event to return ChannelInfo.
    let channel_uuid_string = channel_uuid.to_string();
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
    let builder = events::build_add_member(uuid, &pubkey, Some(role_str))?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn join_channel(channel_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    let builder = events::build_join(uuid)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[tauri::command]
pub async fn leave_channel(channel_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let uuid = parse_channel_uuid(&channel_id)?;
    let builder = events::build_leave(uuid)?;
    submit_event(builder, &state).await?;
    Ok(())
}

#[cfg(test)]
#[path = "channels_tests.rs"]
mod tests;
