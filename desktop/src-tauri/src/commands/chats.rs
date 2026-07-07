use nostr::{EventBuilder, Kind, Tag};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tauri::State;
use uuid::Uuid;

use crate::{
    app_state::AppState,
    events,
    models::{ChannelInfo, SendChannelMessageResponse},
    nostr_convert,
    relay::{query_relay, submit_event},
};

const LEGACY_CHAT_METADATA_KIND: u16 = 30078;
const LEGACY_CHAT_METADATA_D_PREFIX: &str = "buzz:chat:";
const MAX_CONTENT_BYTES: usize = 64 * 1024;

fn tag(parts: Vec<&str>) -> Result<Tag, String> {
    Tag::parse(parts).map_err(|e| format!("invalid tag: {e}"))
}

fn check_pubkey(pubkey: &str) -> Result<(), String> {
    if !pubkey.chars().all(|c| c.is_ascii_hexdigit()) || pubkey.len() != 64 {
        return Err(format!(
            "pubkey must be a 64-character hex string (got {} chars)",
            pubkey.len()
        ));
    }
    Ok(())
}

fn check_content(content: &str) -> Result<(), String> {
    if content.len() > MAX_CONTENT_BYTES {
        return Err(format!(
            "content exceeds maximum size of {MAX_CONTENT_BYTES} bytes (got {})",
            content.len()
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_chat_metadata_tags(
    channel_id: Uuid,
    legacy: bool,
    title: Option<&str>,
    default_agent_pubkey: Option<&str>,
    template_id: Option<&str>,
    project_id: Option<&str>,
    project_name: Option<&str>,
    project_path: Option<&str>,
    project_template_id: Option<&str>,
    source_channel_id: Option<&str>,
    source_event_id: Option<&str>,
    source_thread_root_id: Option<&str>,
) -> Result<Vec<Tag>, String> {
    let channel_id_string = channel_id.to_string();
    let mut tags = if legacy {
        let d_tag = format!("{LEGACY_CHAT_METADATA_D_PREFIX}{channel_id_string}");
        vec![
            tag(vec!["d", &d_tag])?,
            tag(vec!["chat_h", &channel_id_string])?,
        ]
    } else {
        vec![
            tag(vec!["d", &channel_id_string])?,
            tag(vec!["h", &channel_id_string])?,
        ]
    };

    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        tags.push(tag(vec!["title", title])?);
    }
    if let Some(pubkey) = default_agent_pubkey
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        check_pubkey(pubkey)?;
        tags.push(tag(vec!["default_agent", &pubkey.to_ascii_lowercase()])?);
    }
    if let Some(template_id) = template_id.map(str::trim).filter(|value| !value.is_empty()) {
        tags.push(tag(vec!["template", template_id])?);
    }
    if let Some(project_id) = project_id.map(str::trim).filter(|value| !value.is_empty()) {
        tags.push(tag(vec!["project_id", project_id])?);
    }
    if let Some(project_name) = project_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tags.push(tag(vec!["project_name", project_name])?);
    }
    if let Some(project_path) = project_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tags.push(tag(vec!["project_path", project_path])?);
    }
    if let Some(project_template_id) = project_template_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tags.push(tag(vec!["project_template", project_template_id])?);
    }
    if let Some(source_channel_id) = source_channel_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tags.push(tag(vec!["source_h", source_channel_id])?);
    }
    if let Some(source_event_id) = source_event_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tags.push(tag(vec!["source_e", source_event_id])?);
    }
    if let Some(source_thread_root_id) = source_thread_root_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tags.push(tag(vec!["source_root", source_thread_root_id])?);
    }

    Ok(tags)
}

#[allow(clippy::too_many_arguments)]
fn build_chat_metadata(
    channel_id: Uuid,
    title: Option<&str>,
    default_agent_pubkey: Option<&str>,
    template_id: Option<&str>,
    project_id: Option<&str>,
    project_name: Option<&str>,
    project_path: Option<&str>,
    project_template_id: Option<&str>,
    source_channel_id: Option<&str>,
    source_event_id: Option<&str>,
    source_thread_root_id: Option<&str>,
) -> Result<EventBuilder, String> {
    let tags = build_chat_metadata_tags(
        channel_id,
        false,
        title,
        default_agent_pubkey,
        template_id,
        project_id,
        project_name,
        project_path,
        project_template_id,
        source_channel_id,
        source_event_id,
        source_thread_root_id,
    )?;
    Ok(EventBuilder::new(
        Kind::Custom(buzz_core_pkg::kind::KIND_CHAT_METADATA as u16),
        "",
    )
    .tags(tags))
}

#[allow(clippy::too_many_arguments)]
fn build_legacy_chat_metadata(
    channel_id: Uuid,
    title: Option<&str>,
    default_agent_pubkey: Option<&str>,
    template_id: Option<&str>,
    project_id: Option<&str>,
    project_name: Option<&str>,
    project_path: Option<&str>,
    project_template_id: Option<&str>,
    source_channel_id: Option<&str>,
    source_event_id: Option<&str>,
    source_thread_root_id: Option<&str>,
) -> Result<EventBuilder, String> {
    let tags = build_chat_metadata_tags(
        channel_id,
        true,
        title,
        default_agent_pubkey,
        template_id,
        project_id,
        project_name,
        project_path,
        project_template_id,
        source_channel_id,
        source_event_id,
        source_thread_root_id,
    )?;
    Ok(EventBuilder::new(Kind::Custom(LEGACY_CHAT_METADATA_KIND), "").tags(tags))
}

fn build_chat_context_message(
    channel_id: Uuid,
    content: &str,
    source_channel_id: Option<&str>,
    source_event_id: Option<&str>,
    source_thread_root_id: Option<&str>,
) -> Result<EventBuilder, String> {
    check_content(content)?;
    let channel_id_string = channel_id.to_string();
    let mut tags = vec![
        tag(vec!["h", &channel_id_string])?,
        tag(vec!["chat_context", "source"])?,
    ];

    if let Some(source_channel_id) = source_channel_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tags.push(tag(vec!["source_h", source_channel_id])?);
    }
    if let Some(source_event_id) = source_event_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tags.push(tag(vec!["source_e", source_event_id])?);
    }
    if let Some(source_thread_root_id) = source_thread_root_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        tags.push(tag(vec!["source_root", source_thread_root_id])?);
    }

    Ok(EventBuilder::new(Kind::Custom(9), content).tags(tags))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSourceInput {
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub thread_root_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateChatInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub default_agent_pubkey: Option<String>,
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub project_template_id: Option<String>,
    #[serde(default)]
    pub source: Option<ChatSourceInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChatMetadataInput {
    pub channel_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub default_agent_pubkey: Option<String>,
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    #[serde(default)]
    pub project_path: Option<String>,
    #[serde(default)]
    pub project_template_id: Option<String>,
    #[serde(default)]
    pub source: Option<ChatSourceInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendChatContextMessageInput {
    pub channel_id: String,
    pub content: String,
    #[serde(default)]
    pub source: Option<ChatSourceInput>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatMetadataInfo {
    pub channel_id: String,
    pub author_pubkey: String,
    pub title: Option<String>,
    pub default_agent_pubkey: Option<String>,
    pub template_id: Option<String>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub project_path: Option<String>,
    pub project_template_id: Option<String>,
    pub source_channel_id: Option<String>,
    pub source_event_id: Option<String>,
    pub source_thread_root_id: Option<String>,
    pub updated_at: i64,
}

fn trimmed(value: Option<&String>) -> Option<&str> {
    value
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn source_channel_id(source: Option<&ChatSourceInput>) -> Option<&str> {
    source.and_then(|source| trimmed(source.channel_id.as_ref()))
}

fn source_event_id(source: Option<&ChatSourceInput>) -> Option<&str> {
    source.and_then(|source| trimmed(source.event_id.as_ref()))
}

fn source_thread_root_id(source: Option<&ChatSourceInput>) -> Option<&str> {
    source.and_then(|source| trimmed(source.thread_root_id.as_ref()))
}

fn first_tag_value(event: &nostr::Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        (parts.len() >= 2 && parts[0] == name).then(|| parts[1].clone())
    })
}

fn legacy_chat_metadata_d_tag(channel_id: &str) -> String {
    format!("{LEGACY_CHAT_METADATA_D_PREFIX}{channel_id}")
}

fn chat_metadata_from_event(event: &nostr::Event) -> Option<ChatMetadataInfo> {
    let channel_id = if event.kind.as_u16() == LEGACY_CHAT_METADATA_KIND {
        first_tag_value(event, "chat_h").or_else(|| {
            first_tag_value(event, "d").and_then(|d| {
                d.strip_prefix(LEGACY_CHAT_METADATA_D_PREFIX)
                    .map(str::to_string)
            })
        })?
    } else {
        first_tag_value(event, "d")?
    };
    Some(ChatMetadataInfo {
        channel_id,
        author_pubkey: event.pubkey.to_hex(),
        title: first_tag_value(event, "title"),
        default_agent_pubkey: first_tag_value(event, "default_agent"),
        template_id: first_tag_value(event, "template"),
        project_id: first_tag_value(event, "project_id"),
        project_name: first_tag_value(event, "project_name"),
        project_path: first_tag_value(event, "project_path"),
        project_template_id: first_tag_value(event, "project_template"),
        source_channel_id: first_tag_value(event, "source_h"),
        source_event_id: first_tag_value(event, "source_e"),
        source_thread_root_id: first_tag_value(event, "source_root"),
        updated_at: event.created_at.as_secs() as i64,
    })
}

fn is_unsupported_chat_channel_type_error(error: &str) -> bool {
    error.contains("invalid channel_type: chat")
}

fn is_unknown_event_kind_error(error: &str) -> bool {
    error.contains("restricted: unknown event kind")
}

fn current_pubkey(state: &AppState) -> Result<String, String> {
    let keys = state.keys.lock().map_err(|e| e.to_string())?;
    Ok(keys.public_key().to_hex())
}

#[allow(clippy::too_many_arguments)]
async fn submit_chat_metadata(
    state: &AppState,
    channel_id: uuid::Uuid,
    title: Option<&str>,
    default_agent_pubkey: Option<&str>,
    template_id: Option<&str>,
    project_id: Option<&str>,
    project_name: Option<&str>,
    project_path: Option<&str>,
    project_template_id: Option<&str>,
    source_channel_id: Option<&str>,
    source_event_id: Option<&str>,
    source_thread_root_id: Option<&str>,
) -> Result<(), String> {
    let metadata = build_chat_metadata(
        channel_id,
        title,
        default_agent_pubkey,
        template_id,
        project_id,
        project_name,
        project_path,
        project_template_id,
        source_channel_id,
        source_event_id,
        source_thread_root_id,
    )?;
    match submit_event(metadata, state).await {
        Ok(_) => Ok(()),
        Err(error) if is_unknown_event_kind_error(&error) => {
            eprintln!(
                "buzz-desktop: relay does not support kind:30623 yet; writing legacy kind:30078 chat metadata"
            );
            let legacy_metadata = build_legacy_chat_metadata(
                channel_id,
                title,
                default_agent_pubkey,
                template_id,
                project_id,
                project_name,
                project_path,
                project_template_id,
                source_channel_id,
                source_event_id,
                source_thread_root_id,
            )?;
            submit_event(legacy_metadata, state)
                .await
                .map(|_| ())
                .map_err(|legacy_error| {
                    format!("native chat metadata unsupported ({error}); legacy metadata failed: {legacy_error}")
                })
        }
        Err(error) => Err(error),
    }
}

async fn fetch_chat_metadata_infos(state: &AppState) -> Result<Vec<ChatMetadataInfo>, String> {
    let native_events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [buzz_core_pkg::kind::KIND_CHAT_METADATA],
            "limit": 500
        })],
    )
    .await
    .unwrap_or_default();
    let legacy_events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [LEGACY_CHAT_METADATA_KIND],
            "authors": [current_pubkey(state)?],
            "limit": 5000
        })],
    )
    .await
    .unwrap_or_default();

    let mut latest_by_channel: HashMap<String, ChatMetadataInfo> = HashMap::new();
    for metadata in native_events
        .iter()
        .chain(legacy_events.iter())
        .filter_map(chat_metadata_from_event)
    {
        let should_replace = latest_by_channel
            .get(&metadata.channel_id)
            .map(|existing| existing.updated_at < metadata.updated_at)
            .unwrap_or(true);
        if should_replace {
            latest_by_channel.insert(metadata.channel_id.clone(), metadata);
        }
    }

    Ok(latest_by_channel.into_values().collect())
}

pub(super) async fn fetch_chat_metadata_channel_ids(
    state: &AppState,
) -> Result<HashSet<String>, String> {
    Ok(fetch_chat_metadata_infos(state)
        .await?
        .into_iter()
        .map(|metadata| metadata.channel_id)
        .collect())
}

async fn fetch_channel_info(state: &AppState, channel_id: &str) -> Result<ChannelInfo, String> {
    let events = query_relay(
        state,
        &[serde_json::json!({
            "kinds": [39000],
            "#d": [channel_id],
            "limit": 1
        })],
    )
    .await?;

    events
        .first()
        .map(|ev| nostr_convert::channel_info_from_event(ev, None, None))
        .transpose()?
        .ok_or_else(|| "chat created but metadata not yet available".to_string())
}

async fn fetch_chat_channel_info(
    state: &AppState,
    channel_id: &str,
    title: Option<&str>,
) -> Result<ChannelInfo, String> {
    let mut channel = fetch_channel_info(state, channel_id).await?;
    channel.channel_type = "chat".to_string();
    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        channel.name = title.to_string();
    }
    Ok(channel)
}

#[tauri::command]
pub async fn list_chat_metadata(
    state: State<'_, AppState>,
) -> Result<Vec<ChatMetadataInfo>, String> {
    fetch_chat_metadata_infos(&state).await
}

#[tauri::command]
pub async fn list_chats(state: State<'_, AppState>) -> Result<Vec<ChannelInfo>, String> {
    let metadata_infos = fetch_chat_metadata_infos(&state).await.unwrap_or_default();
    let metadata_by_channel: HashMap<String, ChatMetadataInfo> = metadata_infos
        .into_iter()
        .map(|metadata| (metadata.channel_id.clone(), metadata))
        .collect();
    let chat_ids: HashSet<String> = metadata_by_channel.keys().cloned().collect();
    let channels = super::channels::get_channels_including_chats(state).await?;
    Ok(channels
        .into_iter()
        .filter_map(|mut channel| {
            if channel.channel_type != "chat" && !chat_ids.contains(&channel.id) {
                return None;
            }
            channel.channel_type = "chat".to_string();
            if let Some(title) = metadata_by_channel
                .get(&channel.id)
                .and_then(|metadata| metadata.title.as_deref())
                .map(str::trim)
                .filter(|title| !title.is_empty())
            {
                channel.name = title.to_string();
            }
            Some(channel)
        })
        .collect())
}

#[tauri::command]
pub async fn create_chat(
    input: CreateChatInput,
    state: State<'_, AppState>,
) -> Result<ChannelInfo, String> {
    let channel_uuid = uuid::Uuid::new_v4();
    let title = trimmed(input.title.as_ref()).unwrap_or("New chat");

    let builder = events::build_create_channel(channel_uuid, title, "private", "chat", None, None)?;
    if let Err(error) = submit_event(builder, &state).await {
        if !is_unsupported_chat_channel_type_error(&error) {
            return Err(error);
        }

        eprintln!(
            "buzz-desktop: relay does not support channel_type=chat yet; creating private compatibility channel"
        );
        let fallback_builder =
            events::build_create_channel(channel_uuid, title, "private", "stream", None, None)?;
        if let Err(fallback_error) = submit_event(fallback_builder, &state).await {
            eprintln!(
                "buzz-desktop: failed to create private compatibility chat channel: {fallback_error}"
            );
            return Err(format!(
                "could not create private compatibility chat channel: {fallback_error}"
            ));
        }
    }

    let source = input.source.as_ref();
    if let Err(metadata_error) = submit_chat_metadata(
        &state,
        channel_uuid,
        Some(title),
        trimmed(input.default_agent_pubkey.as_ref()),
        trimmed(input.template_id.as_ref()),
        trimmed(input.project_id.as_ref()),
        trimmed(input.project_name.as_ref()),
        trimmed(input.project_path.as_ref()),
        trimmed(input.project_template_id.as_ref()),
        source_channel_id(source),
        source_event_id(source),
        source_thread_root_id(source),
    )
    .await
    {
        eprintln!("buzz-desktop: failed to write chat metadata: {metadata_error}");
        return Err(format!(
            "chat channel was created, but metadata failed: {metadata_error}"
        ));
    }

    fetch_chat_channel_info(&state, &channel_uuid.to_string(), Some(title))
        .await
        .map_err(|error| format!("chat channel was created, but could not be loaded: {error}"))
}

#[tauri::command]
pub async fn get_chat_metadata(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<Option<ChatMetadataInfo>, String> {
    let native_events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [buzz_core_pkg::kind::KIND_CHAT_METADATA],
            "#d": [channel_id],
            "limit": 1
        })],
    )
    .await
    .unwrap_or_default();
    if let Some(metadata) = native_events.first().and_then(chat_metadata_from_event) {
        return Ok(Some(metadata));
    }

    let legacy_events = query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [LEGACY_CHAT_METADATA_KIND],
            "#d": [legacy_chat_metadata_d_tag(&channel_id)],
            "authors": [current_pubkey(&state)?],
            "limit": 1
        })],
    )
    .await
    .unwrap_or_default();

    Ok(legacy_events.first().and_then(chat_metadata_from_event))
}

#[tauri::command]
pub async fn update_chat_metadata(
    input: UpdateChatMetadataInput,
    state: State<'_, AppState>,
) -> Result<ChatMetadataInfo, String> {
    let uuid = uuid::Uuid::parse_str(&input.channel_id)
        .map_err(|_| format!("invalid channel UUID: {}", input.channel_id))?;
    let source = input.source.as_ref();
    submit_chat_metadata(
        &state,
        uuid,
        trimmed(input.title.as_ref()),
        trimmed(input.default_agent_pubkey.as_ref()),
        trimmed(input.template_id.as_ref()),
        trimmed(input.project_id.as_ref()),
        trimmed(input.project_name.as_ref()),
        trimmed(input.project_path.as_ref()),
        trimmed(input.project_template_id.as_ref()),
        source_channel_id(source),
        source_event_id(source),
        source_thread_root_id(source),
    )
    .await?;

    get_chat_metadata(input.channel_id, state)
        .await?
        .ok_or_else(|| "chat metadata not available after update".to_string())
}

#[tauri::command]
pub async fn send_chat_context_message(
    input: SendChatContextMessageInput,
    state: State<'_, AppState>,
) -> Result<SendChannelMessageResponse, String> {
    let uuid = uuid::Uuid::parse_str(&input.channel_id)
        .map_err(|_| format!("invalid channel UUID: {}", input.channel_id))?;
    let source = input.source.as_ref();
    let builder = build_chat_context_message(
        uuid,
        input.content.trim(),
        source_channel_id(source),
        source_event_id(source),
        source_thread_root_id(source),
    )?;
    let result = submit_event(builder, &state).await?;

    Ok(SendChannelMessageResponse {
        event_id: result.event_id,
        parent_event_id: None,
        root_event_id: None,
        depth: 0,
        created_at: chrono::Utc::now().timestamp(),
    })
}
