use tauri::State;

use crate::{
    app_state::AppState,
    events,
    models::{ChannelDetailInfo, ChannelInfo, ChannelMembersResponse},
    nostr_convert,
    relay::{query_relay, relay_api_base_url_with_override, submit_event, submit_event_with_keys},
};

// ── Reads (pure-nostr via /query) ────────────────────────────────────────────

const DIRECTORY_PAGE_SIZE: usize = 500;
const STARTER_CHANNEL_NAMESPACE: uuid::Uuid = uuid::uuid!("3ce33bea-8f09-5f1b-9c85-8a7d2659e6b0");

struct StarterChannelSpec {
    slug: &'static str,
    name: &'static str,
    description: &'static str,
}

const STARTER_CHANNELS: &[StarterChannelSpec] = &[
    StarterChannelSpec {
        slug: "general",
        name: "general",
        description: "General conversation and community updates.",
    },
    StarterChannelSpec {
        slug: "welcome-everyone",
        name: "welcome-everyone",
        description: "Say hi, ask a question, or share what brought you here.",
    },
];

fn advance_directory_cursor(filter: &mut serde_json::Value, page: &[nostr::Event]) {
    let last = page
        .last()
        .expect("a full relay page always has a last event");
    filter["until"] = serde_json::json!(last.created_at.as_secs());
    filter["before_id"] = serde_json::json!(last.id.to_hex());
}

/// Fetch every page for a historical relay filter using the relay's composite
/// `(until, before_id)` cursor. A timestamp-only cursor can skip rows when more
/// than one page of events shares the same second.
async fn query_relay_all(
    state: &AppState,
    mut filter: serde_json::Value,
) -> Result<Vec<nostr::Event>, String> {
    filter["limit"] = serde_json::json!(DIRECTORY_PAGE_SIZE);
    let mut all = Vec::new();

    loop {
        let page = query_relay(state, &[filter.clone()]).await?;
        let done = page.len() < DIRECTORY_PAGE_SIZE;

        if !done {
            advance_directory_cursor(&mut filter, &page);
        }

        all.extend(page);
        if done {
            return Ok(all);
        }
    }
}

/// Whether an open channel not yet in the real member set should still be
/// classified `is_member=true` via the pending-owner overlay. Pulled out of
/// `get_channels`'s open-channel branch so the exact `(d_tag, my_pubkey,
/// overlay) -> is_member` decision — including the identity binding that
/// keeps one identity's pending entry from covering another's — is directly
/// unit-testable without going through the async relay-backed command.
fn classify_pending_owner(state: &AppState, my_pubkey: &str, d_tag: Option<&str>) -> bool {
    d_tag.is_some_and(|d| state.is_pending_owned_channel(my_pubkey, d))
}

#[tauri::command]
pub async fn get_channels(state: State<'_, AppState>) -> Result<Vec<ChannelInfo>, String> {
    let _profile_start = std::time::Instant::now();
    let my_pubkey = {
        let keys = state.keys.lock().map_err(|e| e.to_string())?;
        keys.public_key().to_hex()
    };

    // Step 1: find all kind:39002 (members) events that mention me, then
    // pull the channel ids out of their `d` tags.
    let member_events = query_relay_all(
        &state,
        serde_json::json!({"kinds": [39002], "#p": [&my_pubkey]}),
    )
    .await?;

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

    // The real kind:39002 membership has now resolved for these channels —
    // drop them from the pending-owner overlay (see `AppState::pending_owned_channels`)
    // so a channel this identity created no longer speaks through the overlay
    // once genuine membership is observable, and a later leave correctly
    // flips it back to `is_member=false`.
    for id in &channel_ids {
        state.clear_pending_owned_channel(&my_pubkey, id);
    }

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
    let open_meta_events = query_relay_all(&state, serde_json::json!({"kinds": [39000]})).await?;

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
        // The overlay (`AppState::pending_owned_channels`) marks channels this
        // identity just created via `create_channel` whose kind:39002 owner
        // membership hasn't propagated yet (#1761) — a fresh channel has no
        // member event and would otherwise fall through to `is_member=false`
        // here, disabling the owner's own composer until that snapshot lands.
        // The overlay can only be populated by this process's own
        // `create_channel` call (never by relay data) and is keyed by
        // `(my_pubkey, d_tag)`, so it adds no trust-boundary risk and can
        // never speak for a channel a different identity created; `channel_ids`
        // above clears it once real membership is observed for `my_pubkey`.
        let is_pending_owner = classify_pending_owner(&state, &my_pubkey, d_tag.as_deref());
        if let Ok(info) = nostr_convert::channel_info_from_event(ev, None, Some(is_pending_owner)) {
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

fn normalize_channel_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn starter_channel_uuid(relay_scope: &str, slug: &str) -> uuid::Uuid {
    let name = format!("starter-channel:v1:{}:{}", relay_scope.trim(), slug);
    uuid::Uuid::new_v5(&STARTER_CHANNEL_NAMESPACE, name.as_bytes())
}

fn is_duplicate_channel_rejection(error: &str) -> bool {
    error.contains("relay rejected event:") && error.contains("duplicate: channel already exists")
}

fn is_matching_starter_channel(channel: &ChannelInfo, spec: &StarterChannelSpec) -> bool {
    normalize_channel_name(&channel.name) == normalize_channel_name(spec.name)
        && channel.channel_type == "stream"
        && channel.visibility == "open"
        && channel.archived_at.is_none()
}

fn has_all_starter_channels(channels: &[ChannelInfo]) -> bool {
    STARTER_CHANNELS.iter().all(|spec| {
        channels
            .iter()
            .any(|channel| is_matching_starter_channel(channel, spec))
    })
}

async fn ensure_starter_channel_memberships(
    state: &AppState,
    keys: &nostr::Keys,
    channels: &mut [ChannelInfo],
) -> Result<(), String> {
    for spec in STARTER_CHANNELS {
        let Some(channel) = channels
            .iter_mut()
            .find(|channel| is_matching_starter_channel(channel, spec))
        else {
            continue;
        };

        if channel.is_member {
            continue;
        }

        let channel_uuid = parse_channel_uuid(&channel.id)?;
        let builder = events::build_join(channel_uuid)?;
        submit_event_with_keys(builder, state, keys, None).await?;
        channel.is_member = true;
    }

    Ok(())
}

async fn fetch_starter_channel_metadata(
    state: &AppState,
    channel_ids: &[String],
) -> Result<Vec<ChannelInfo>, String> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }

    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": channel_ids,
            "limit": channel_ids.len(),
        })],
    )
    .await?;

    events
        .iter()
        .map(|ev| nostr_convert::channel_info_from_event(ev, None, Some(false)))
        .collect()
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

    // Capture the signing identity before submission so the pending-owner
    // mark below is bound to whoever actually signed this create — not
    // whoever `state.keys` holds once the network round-trip completes. An
    // in-process identity swap while the request is in flight must not be
    // able to retarget the mark onto the new identity.
    let creator_keys = state.signing_keys()?;
    let creator_pubkey = creator_keys.public_key().to_hex();
    submit_event_with_keys(builder, &state, &creator_keys, None).await?;

    // Mark this channel pending-owner: we just created it, so we know we're
    // the owner, but the relay's kind:39002 membership entry (#1761) is
    // provisioned asynchronously. `get_channels` consults this overlay to
    // classify us as `is_member=true` until that entry is observable. Bound
    // to the identity that signed the create above, so an in-process
    // identity swap can neither inherit nor retarget this entry.
    let channel_uuid_string = channel_uuid.to_string();
    state.mark_pending_owned_channel(&creator_pubkey, &channel_uuid_string);

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

#[tauri::command]
pub async fn ensure_starter_channels(
    state: State<'_, AppState>,
) -> Result<Vec<ChannelInfo>, String> {
    let mut existing_channels = get_channels(state.clone()).await?;
    let relay_scope = relay_api_base_url_with_override(&state);
    let creator_keys = state.signing_keys()?;
    let creator_pubkey = creator_keys.public_key().to_hex();
    let mut starter_ids = Vec::with_capacity(STARTER_CHANNELS.len());
    let mut created_ids = std::collections::HashSet::new();

    for spec in STARTER_CHANNELS {
        if existing_channels
            .iter()
            .any(|channel| is_matching_starter_channel(channel, spec))
        {
            continue;
        }

        let channel_uuid = starter_channel_uuid(&relay_scope, spec.slug);
        let channel_uuid_string = channel_uuid.to_string();
        starter_ids.push(channel_uuid_string.clone());
        let builder = events::build_create_channel(
            channel_uuid,
            spec.name,
            "open",
            "stream",
            Some(spec.description),
            None,
        )?;

        match submit_event_with_keys(builder, &state, &creator_keys, None).await {
            Ok(_) => {
                state.mark_pending_owned_channel(&creator_pubkey, &channel_uuid_string);
                created_ids.insert(channel_uuid_string.clone());
            }
            Err(error) if is_duplicate_channel_rejection(&error) => {
                state.mark_pending_owned_channel(&creator_pubkey, &channel_uuid_string);
            }
            Err(error) => return Err(error),
        }
    }

    for _ in 0..3 {
        let metadata = fetch_starter_channel_metadata(&state, &starter_ids).await?;
        for mut channel in metadata {
            if created_ids.contains(&channel.id) {
                channel.is_member = true;
            }
            if !existing_channels
                .iter()
                .any(|existing| existing.id == channel.id)
            {
                existing_channels.push(channel);
            }
        }
        if has_all_starter_channels(&existing_channels) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    if !has_all_starter_channels(&existing_channels) {
        existing_channels = get_channels(state.clone()).await?;
    }

    if !has_all_starter_channels(&existing_channels) {
        return Err("starter channels created but metadata not yet available".to_string());
    }

    ensure_starter_channel_memberships(&state, &creator_keys, &mut existing_channels).await?;
    Ok(existing_channels)
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
