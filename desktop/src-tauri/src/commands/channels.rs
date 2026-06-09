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

        // Build pubkey → display_name map from kind:0 events
        let mut name_map = std::collections::HashMap::new();
        for ev in &profile_events {
            let pk = ev.pubkey.to_hex();
            if let Ok(profile) = nostr_convert::profile_info_from_event(ev) {
                if let Some(name) = profile.display_name {
                    name_map.insert(pk, name);
                }
            }
        }

        // Populate display_name on each member
        for member in &mut response.members {
            if member.display_name.is_none() {
                member.display_name = name_map.get(&member.pubkey).cloned();
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
