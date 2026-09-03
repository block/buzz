use std::collections::{BTreeMap, HashMap, HashSet};

use nostr::nips::nip44::{self, Version};
use nostr::{Event, EventBuilder, Kind, Tag, Timestamp};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::client::{extract_d_tag, extract_tag_value, BuzzClient};
use crate::commands::parse_write_response;
use crate::error::CliError;
use crate::validate::{read_file_or_stdin, validate_content_size};
use crate::{SidebarCmd, SidebarSectionsCmd};

const KIND_CHANNEL_SECTIONS: u16 = 30_078;
const CHANNEL_METADATA_KIND: u16 = 39_000;
const D_TAG: &str = "channel-sections";
const MAX_SECTIONS: usize = 100;
const MAX_ASSIGNED_CHANNELS: usize = 5_000;
const MAX_SECTION_NAME_BYTES: usize = 128;
const MAX_ICON_BYTES: usize = 64;
const NIP44_PLAINTEXT_MAX_BYTES: usize = 65_535;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct WireSection {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    order: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct WireStore {
    version: u8,
    sections: Vec<WireSection>,
    assignments: BTreeMap<String, String>,
}

impl Default for WireStore {
    fn default() -> Self {
        Self {
            version: 1,
            sections: Vec::new(),
            assignments: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DesiredLayout {
    #[serde(default)]
    revision: Option<String>,
    version: u8,
    sections: Vec<DesiredSection>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DesiredSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(default)]
    channels: Vec<ChannelSelector>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum ChannelSelector {
    NameOrId(String),
    Detailed(ChannelDescriptor),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChannelDescriptor {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct ListedLayout {
    revision: Option<String>,
    version: u8,
    sections: Vec<ListedSection>,
}

#[derive(Debug, Serialize)]
struct ListedSection {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    channels: Vec<ChannelDescriptor>,
}

struct RemoteHead {
    revision: String,
    created_at: u64,
    store: WireStore,
}

#[derive(Default)]
struct ChannelIndex {
    by_id: BTreeMap<String, String>,
    by_name: HashMap<String, Vec<String>>,
}

pub async fn dispatch(cmd: SidebarCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        SidebarCmd::Sections(SidebarSectionsCmd::List) => cmd_list(client).await,
        SidebarCmd::Sections(SidebarSectionsCmd::Create {
            name,
            icon,
            expected_revision,
        }) => cmd_create(client, &name, icon.as_deref(), &expected_revision).await,
        SidebarCmd::Sections(SidebarSectionsCmd::Assign {
            section,
            channel,
            expected_revision,
        }) => cmd_assign(client, &section, channel, &expected_revision).await,
        SidebarCmd::Sections(SidebarSectionsCmd::Unassign {
            channel,
            expected_revision,
        }) => cmd_unassign(client, channel, &expected_revision).await,
        SidebarCmd::Sections(SidebarSectionsCmd::Rename {
            section,
            name,
            icon,
            clear_icon,
            expected_revision,
        }) => {
            let icon_change = if clear_icon {
                Some(None)
            } else {
                icon.map(Some)
            };
            cmd_rename(
                client,
                &section,
                name.as_deref(),
                icon_change.as_ref().map(|value| value.as_deref()),
                &expected_revision,
            )
            .await
        }
        SidebarCmd::Sections(SidebarSectionsCmd::Reorder {
            section,
            expected_revision,
        }) => cmd_reorder(client, section, &expected_revision).await,
        SidebarCmd::Sections(SidebarSectionsCmd::Delete {
            section,
            expected_revision,
        }) => cmd_delete(client, &section, &expected_revision).await,
        SidebarCmd::Sections(SidebarSectionsCmd::Apply {
            input,
            expected_revision,
        }) => cmd_apply(client, &input, expected_revision.as_deref()).await,
    }
}

async fn cmd_list(client: &BuzzClient) -> Result<(), CliError> {
    let head = fetch_remote_head(client).await?;
    let index = fetch_channel_index(client).await?;
    let layout = listed_layout(head.as_ref(), &index);
    println!(
        "{}",
        serde_json::to_string(&layout)
            .map_err(|e| CliError::Other(format!("failed to serialize sidebar layout: {e}")))?
    );
    Ok(())
}

async fn cmd_apply(
    client: &BuzzClient,
    input: &str,
    expected_revision_arg: Option<&str>,
) -> Result<(), CliError> {
    let raw = read_file_or_stdin(input)?;
    validate_content_size(&raw)?;
    let desired: DesiredLayout = serde_json::from_str(&raw)
        .map_err(|e| CliError::Usage(format!("invalid sidebar layout JSON: {e}")))?;
    let expected = resolve_expected_revision(desired.revision.as_deref(), expected_revision_arg)?;

    let head = fetch_remote_head(client).await?;
    ensure_expected_revision(head.as_ref(), expected.as_deref())?;

    let index = fetch_channel_index(client).await?;
    let current = head
        .as_ref()
        .map(|remote| &remote.store)
        .cloned()
        .unwrap_or_default();
    let store = materialize_layout(desired, &current, &index)?;

    publish_store(client, expected.as_deref(), store).await
}

async fn cmd_create(
    client: &BuzzClient,
    name: &str,
    icon: Option<&str>,
    expected_revision: &str,
) -> Result<(), CliError> {
    let expected = parse_revision(expected_revision)?;
    let head = fetch_remote_head(client).await?;
    ensure_expected_revision(head.as_ref(), expected.as_deref())?;
    let mut store = current_store(head.as_ref());
    create_section(&mut store, name, icon)?;
    publish_store(client, expected.as_deref(), store).await
}

async fn cmd_assign(
    client: &BuzzClient,
    section: &str,
    channels: Vec<String>,
    expected_revision: &str,
) -> Result<(), CliError> {
    let expected = parse_revision(expected_revision)?;
    let head = fetch_remote_head(client).await?;
    ensure_expected_revision(head.as_ref(), expected.as_deref())?;
    let index = fetch_channel_index(client).await?;
    let mut store = current_store(head.as_ref());
    assign_channels(&mut store, section, channels, &index)?;
    publish_store(client, expected.as_deref(), store).await
}

async fn cmd_unassign(
    client: &BuzzClient,
    channels: Vec<String>,
    expected_revision: &str,
) -> Result<(), CliError> {
    let expected = parse_revision(expected_revision)?;
    let head = fetch_remote_head(client).await?;
    ensure_expected_revision(head.as_ref(), expected.as_deref())?;
    let index = fetch_channel_index(client).await?;
    let mut store = current_store(head.as_ref());
    unassign_channels(&mut store, channels, &index)?;
    publish_store(client, expected.as_deref(), store).await
}

async fn cmd_rename(
    client: &BuzzClient,
    section: &str,
    name: Option<&str>,
    icon_change: Option<Option<&str>>,
    expected_revision: &str,
) -> Result<(), CliError> {
    if name.is_none() && icon_change.is_none() {
        return Err(CliError::Usage(
            "rename requires --name, --icon, or --clear-icon".into(),
        ));
    }
    let expected = parse_revision(expected_revision)?;
    let head = fetch_remote_head(client).await?;
    ensure_expected_revision(head.as_ref(), expected.as_deref())?;
    let mut store = current_store(head.as_ref());
    rename_section(&mut store, section, name, icon_change)?;
    publish_store(client, expected.as_deref(), store).await
}

async fn cmd_reorder(
    client: &BuzzClient,
    sections: Vec<String>,
    expected_revision: &str,
) -> Result<(), CliError> {
    let expected = parse_revision(expected_revision)?;
    let head = fetch_remote_head(client).await?;
    ensure_expected_revision(head.as_ref(), expected.as_deref())?;
    let mut store = current_store(head.as_ref());
    reorder_sections(&mut store, sections)?;
    publish_store(client, expected.as_deref(), store).await
}

async fn cmd_delete(
    client: &BuzzClient,
    section: &str,
    expected_revision: &str,
) -> Result<(), CliError> {
    let expected = parse_revision(expected_revision)?;
    let head = fetch_remote_head(client).await?;
    ensure_expected_revision(head.as_ref(), expected.as_deref())?;
    let mut store = current_store(head.as_ref());
    delete_section(&mut store, section)?;
    publish_store(client, expected.as_deref(), store).await
}

async fn publish_store(
    client: &BuzzClient,
    expected_revision: Option<&str>,
    store: WireStore,
) -> Result<(), CliError> {
    validate_wire_store(&store)?;
    // Every mutation is based on an earlier read. Re-check immediately before
    // signing so paginated name resolution or local processing cannot overwrite
    // a layout changed in the meantime.
    let head = fetch_remote_head(client).await?;
    ensure_expected_revision(head.as_ref(), expected_revision)?;
    let current = current_store(head.as_ref());

    if store == current {
        println!(
            "{}",
            serde_json::json!({
                "changed": false,
                "revision": head.as_ref().map(|remote| remote.revision.as_str()),
            })
        );
        return Ok(());
    }

    let plaintext = serde_json::to_string(&store)
        .map_err(|e| CliError::Other(format!("failed to serialize sidebar layout: {e}")))?;
    validate_content_size(&plaintext)?;
    if plaintext.len() > NIP44_PLAINTEXT_MAX_BYTES {
        return Err(CliError::Usage(format!(
            "sidebar layout exceeds the NIP-44 plaintext limit of {NIP44_PLAINTEXT_MAX_BYTES} bytes"
        )));
    }
    let public_key = client.keys().public_key();
    let ciphertext = nip44::encrypt(
        client.keys().secret_key(),
        &public_key,
        &plaintext,
        Version::V2,
    )
    .map_err(|e| CliError::Other(format!("failed to encrypt sidebar layout: {e}")))?;

    let now = Timestamp::now().as_secs();
    let created_at = match head.as_ref() {
        Some(remote) => now.max(
            remote
                .created_at
                .checked_add(1)
                .ok_or_else(|| CliError::Other("sidebar revision timestamp overflow".into()))?,
        ),
        None => now,
    };
    let tags = vec![make_tag(&["d", D_TAG])?, make_tag(&["t", D_TAG])?];
    let event = client.sign_event(
        EventBuilder::new(Kind::Custom(KIND_CHANNEL_SECTIONS), ciphertext)
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at)),
    )?;
    let revision = event.id.to_hex();
    let raw_response = client.submit_event(event).await?;
    parse_write_response(
        &raw_response,
        "sidebar layout changed concurrently; list it again before retrying",
    )?;
    let observed = fetch_remote_head(client).await?;
    if observed.as_ref().map(|head| head.revision.as_str()) != Some(revision.as_str()) {
        return Err(CliError::Conflict(
            "sidebar layout changed concurrently after publish; list it again before retrying"
                .into(),
        ));
    }
    println!(
        "{}",
        serde_json::json!({"changed": true, "revision": revision})
    );
    Ok(())
}

fn current_store(head: Option<&RemoteHead>) -> WireStore {
    head.map(|remote| remote.store.clone()).unwrap_or_default()
}

async fn fetch_remote_head(client: &BuzzClient) -> Result<Option<RemoteHead>, CliError> {
    let author = client.keys().public_key().to_hex();
    let filter = serde_json::json!({
        "kinds": [KIND_CHANNEL_SECTIONS],
        "authors": [author],
        "#d": [D_TAG],
        "limit": 1,
    });
    let raw = client.query(&filter).await?;
    let mut events: Vec<Event> = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse sidebar query response: {e}")))?;
    let Some(event) = events.pop() else {
        return Ok(None);
    };
    validate_remote_event(&event, &author)?;

    let plaintext = nip44::decrypt(
        client.keys().secret_key(),
        &client.keys().public_key(),
        &event.content,
    )
    .map_err(|_| CliError::Other("current sidebar layout could not be decrypted".into()))?;
    validate_content_size(&plaintext)
        .map_err(|_| CliError::Other("current sidebar layout exceeds the supported size".into()))?;
    if plaintext.len() > NIP44_PLAINTEXT_MAX_BYTES {
        return Err(CliError::Other(
            "current sidebar layout exceeds the NIP-44 plaintext limit".into(),
        ));
    }
    let mut store: WireStore = serde_json::from_str(&plaintext)
        .map_err(|e| CliError::Other(format!("current sidebar layout is malformed: {e}")))?;
    validate_wire_store(&store)
        .map_err(|e| CliError::Other(format!("current sidebar layout is invalid: {e}")))?;
    // Desktop stores order as an explicit field and may serialize the backing
    // vector unsorted after drag-and-drop. Normalize before listing/comparing.
    normalize_wire_store(&mut store);

    Ok(Some(RemoteHead {
        revision: event.id.to_hex(),
        created_at: event.created_at.as_secs(),
        store,
    }))
}

fn validate_remote_event(event: &Event, expected_author: &str) -> Result<(), CliError> {
    if event.kind != Kind::Custom(KIND_CHANNEL_SECTIONS) {
        return Err(CliError::Other("sidebar event has the wrong kind".into()));
    }
    if event.pubkey.to_hex() != expected_author {
        return Err(CliError::Other(
            "sidebar event author does not match the current identity".into(),
        ));
    }
    let d_tags: Vec<&Tag> = event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some("d"))
        .collect();
    if d_tags.len() != 1 || d_tags[0].as_slice() != ["d", D_TAG] {
        return Err(CliError::Other(
            "sidebar event must have exactly one canonical d tag".into(),
        ));
    }
    event.verify().map_err(|e| {
        CliError::Other(format!(
            "sidebar event failed cryptographic verification: {e}"
        ))
    })?;
    Ok(())
}

async fn fetch_channel_index(client: &BuzzClient) -> Result<ChannelIndex, CliError> {
    let events = client
        .query_paginated(
            serde_json::json!({"kinds": [CHANNEL_METADATA_KIND]}),
            (MAX_ASSIGNED_CHANNELS + 1) as u32,
        )
        .await?;
    if events.len() > MAX_ASSIGNED_CHANNELS {
        return Err(CliError::Other(format!(
            "visible channel count exceeds the supported limit of {MAX_ASSIGNED_CHANNELS}"
        )));
    }

    let mut index = ChannelIndex::default();
    for event in events {
        let id = extract_d_tag(&event);
        let name = extract_tag_value(&event, "name");
        if name.is_empty() {
            continue;
        }
        let id = canonical_uuid(&id, "channel ID").map_err(|_| {
            CliError::Other(format!(
                "relay returned malformed channel metadata ID: {id}"
            ))
        })?;
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(CliError::Other(format!(
                "relay returned an invalid name for channel {id}"
            )));
        }
        index.by_id.insert(id, name);
    }
    for (id, name) in &index.by_id {
        index
            .by_name
            .entry(name.to_lowercase())
            .or_default()
            .push(id.clone());
    }
    Ok(index)
}

fn listed_layout(head: Option<&RemoteHead>, index: &ChannelIndex) -> ListedLayout {
    let store = head.map(|remote| &remote.store);
    let sections = store
        .map(|store| {
            store
                .sections
                .iter()
                .map(|section| {
                    let mut channels: Vec<ChannelDescriptor> = store
                        .assignments
                        .iter()
                        .filter(|(_, section_id)| *section_id == &section.id)
                        .map(|(channel_id, _)| ChannelDescriptor {
                            id: channel_id.clone(),
                            name: index.by_id.get(channel_id).cloned(),
                        })
                        .collect();
                    channels.sort_by(|a, b| {
                        a.name
                            .as_deref()
                            .unwrap_or("")
                            .cmp(b.name.as_deref().unwrap_or(""))
                            .then_with(|| a.id.cmp(&b.id))
                    });
                    ListedSection {
                        id: section.id.clone(),
                        name: section.name.clone(),
                        icon: section.icon.clone(),
                        channels,
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    ListedLayout {
        revision: head.map(|remote| remote.revision.clone()),
        version: 1,
        sections,
    }
}

fn materialize_layout(
    desired: DesiredLayout,
    current: &WireStore,
    index: &ChannelIndex,
) -> Result<WireStore, CliError> {
    if desired.version != 1 {
        return Err(CliError::Usage(format!(
            "unsupported sidebar layout version: {}",
            desired.version
        )));
    }
    if desired.sections.len() > MAX_SECTIONS {
        return Err(CliError::Usage(format!(
            "sidebar layout exceeds the maximum of {MAX_SECTIONS} sections"
        )));
    }

    let mut current_by_name: HashMap<String, Vec<String>> = HashMap::new();
    for section in &current.sections {
        current_by_name
            .entry(section.name.to_lowercase())
            .or_default()
            .push(section.id.clone());
    }
    let mut desired_name_counts: HashMap<String, usize> = HashMap::new();
    for section in &desired.sections {
        let name = validate_label(&section.name, "section name", MAX_SECTION_NAME_BYTES)?;
        *desired_name_counts.entry(name.to_lowercase()).or_default() += 1;
    }

    let mut section_ids = HashSet::new();
    let mut sections = Vec::with_capacity(desired.sections.len());
    let mut desired_channels = Vec::with_capacity(desired.sections.len());
    for (order, section) in desired.sections.into_iter().enumerate() {
        let name = validate_label(&section.name, "section name", MAX_SECTION_NAME_BYTES)?;
        if section.id.is_none()
            && desired_name_counts
                .get(&name.to_lowercase())
                .copied()
                .unwrap_or_default()
                > 1
        {
            return Err(CliError::Usage(format!(
                "duplicate sidebar section name {name:?} requires explicit IDs"
            )));
        }
        let icon = section
            .icon
            .as_deref()
            .map(|value| validate_label(value, "section icon", MAX_ICON_BYTES))
            .transpose()?;
        let id = match section.id {
            Some(id) => canonical_uuid(&id, "section ID")?,
            None => match current_by_name.get(&name.to_lowercase()) {
                Some(matches) if matches.len() == 1 => matches[0].clone(),
                Some(_) => {
                    return Err(CliError::Usage(format!(
                        "section name {name:?} is ambiguous; specify its id"
                    )))
                }
                None => Uuid::new_v4().to_string(),
            },
        };
        if !section_ids.insert(id.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate sidebar section ID: {id}"
            )));
        }
        sections.push(WireSection {
            id,
            name,
            icon,
            order: order as u32,
        });
        desired_channels.push(section.channels);
    }

    let mut assignments = BTreeMap::new();
    for (section, selectors) in sections.iter().zip(desired_channels) {
        for selector in selectors {
            let channel_id = resolve_channel(selector, index)?;
            if let Some(previous_section) =
                assignments.insert(channel_id.clone(), section.id.clone())
            {
                return Err(CliError::Usage(format!(
                    "channel {channel_id} is assigned to both {previous_section} and {}",
                    section.id
                )));
            }
        }
    }
    if assignments.len() > MAX_ASSIGNED_CHANNELS {
        return Err(CliError::Usage(format!(
            "sidebar layout exceeds the maximum of {MAX_ASSIGNED_CHANNELS} assigned channels"
        )));
    }

    let store = WireStore {
        version: 1,
        sections,
        assignments,
    };
    validate_wire_store(&store)?;
    Ok(store)
}

fn resolve_channel(selector: ChannelSelector, index: &ChannelIndex) -> Result<String, CliError> {
    match selector {
        ChannelSelector::Detailed(channel) => {
            let id = canonical_uuid(&channel.id, "channel ID")?;
            if !index.by_id.contains_key(&id) {
                return Err(CliError::Usage(format!(
                    "channel does not exist or is not visible: {id}"
                )));
            }
            Ok(id)
        }
        ChannelSelector::NameOrId(value) => {
            let value = validate_label(&value, "channel reference", MAX_SECTION_NAME_BYTES)?;
            if let Ok(id) = Uuid::parse_str(&value) {
                let id = id.to_string();
                if !index.by_id.contains_key(&id) {
                    return Err(CliError::Usage(format!(
                        "channel does not exist or is not visible: {id}"
                    )));
                }
                return Ok(id);
            }
            match index.by_name.get(&value.to_lowercase()) {
                Some(matches) if matches.len() == 1 => Ok(matches[0].clone()),
                Some(matches) => Err(CliError::Usage(format!(
                    "channel name {value:?} is ambiguous across {} visible channels; use an ID",
                    matches.len()
                ))),
                None => Err(CliError::Usage(format!(
                    "channel does not exist or is not visible: {value}"
                ))),
            }
        }
    }
}

fn create_section(store: &mut WireStore, name: &str, icon: Option<&str>) -> Result<(), CliError> {
    if store.sections.len() >= MAX_SECTIONS {
        return Err(CliError::Usage(format!(
            "sidebar layout already has the maximum of {MAX_SECTIONS} sections"
        )));
    }
    let name = validate_label(name, "section name", MAX_SECTION_NAME_BYTES)?;
    let icon = icon
        .map(|value| validate_label(value, "section icon", MAX_ICON_BYTES))
        .transpose()?;
    store.sections.push(WireSection {
        id: Uuid::new_v4().to_string(),
        name,
        icon,
        order: store.sections.len() as u32,
    });
    Ok(())
}

fn assign_channels(
    store: &mut WireStore,
    section: &str,
    channels: Vec<String>,
    index: &ChannelIndex,
) -> Result<(), CliError> {
    if channels.len() > MAX_ASSIGNED_CHANNELS {
        return Err(CliError::Usage(format!(
            "cannot assign more than {MAX_ASSIGNED_CHANNELS} channels at once"
        )));
    }
    let section_id = resolve_section_id(store, section)?;
    let channel_ids = channels
        .into_iter()
        .map(ChannelSelector::NameOrId)
        .map(|selector| resolve_channel(selector, index))
        .collect::<Result<Vec<_>, _>>()?;
    for channel_id in channel_ids {
        store.assignments.insert(channel_id, section_id.clone());
    }
    Ok(())
}

fn unassign_channels(
    store: &mut WireStore,
    channels: Vec<String>,
    index: &ChannelIndex,
) -> Result<(), CliError> {
    if channels.len() > MAX_ASSIGNED_CHANNELS {
        return Err(CliError::Usage(format!(
            "cannot unassign more than {MAX_ASSIGNED_CHANNELS} channels at once"
        )));
    }
    let channel_ids = channels
        .into_iter()
        .map(ChannelSelector::NameOrId)
        .map(|selector| resolve_channel(selector, index))
        .collect::<Result<Vec<_>, _>>()?;
    for channel_id in channel_ids {
        store.assignments.remove(&channel_id);
    }
    Ok(())
}

fn rename_section(
    store: &mut WireStore,
    section: &str,
    name: Option<&str>,
    icon_change: Option<Option<&str>>,
) -> Result<(), CliError> {
    let section_id = resolve_section_id(store, section)?;
    let name = name
        .map(|value| validate_label(value, "section name", MAX_SECTION_NAME_BYTES))
        .transpose()?;
    let icon_change = icon_change
        .map(|value| {
            value
                .map(|icon| validate_label(icon, "section icon", MAX_ICON_BYTES))
                .transpose()
        })
        .transpose()?;
    let target = store
        .sections
        .iter_mut()
        .find(|candidate| candidate.id == section_id)
        .expect("resolved section must exist");
    if let Some(name) = name {
        target.name = name;
    }
    if let Some(icon) = icon_change {
        target.icon = icon;
    }
    Ok(())
}

fn reorder_sections(store: &mut WireStore, selectors: Vec<String>) -> Result<(), CliError> {
    if selectors.len() != store.sections.len() {
        return Err(CliError::Usage(format!(
            "reorder requires every section exactly once (expected {}, got {})",
            store.sections.len(),
            selectors.len()
        )));
    }
    let mut ordered_ids = Vec::with_capacity(selectors.len());
    let mut seen = HashSet::new();
    for selector in selectors {
        let id = resolve_section_id(store, &selector)?;
        if !seen.insert(id.clone()) {
            return Err(CliError::Usage(format!(
                "duplicate section in reorder: {selector}"
            )));
        }
        ordered_ids.push(id);
    }
    for section in &mut store.sections {
        section.order = ordered_ids
            .iter()
            .position(|id| id == &section.id)
            .expect("complete reorder must contain every section") as u32;
    }
    normalize_wire_store(store);
    Ok(())
}

fn delete_section(store: &mut WireStore, section: &str) -> Result<(), CliError> {
    let section_id = resolve_section_id(store, section)?;
    store
        .sections
        .retain(|candidate| candidate.id != section_id);
    store
        .assignments
        .retain(|_, assigned_section| assigned_section != &section_id);
    for (order, section) in store.sections.iter_mut().enumerate() {
        section.order = order as u32;
    }
    Ok(())
}

fn resolve_section_id(store: &WireStore, selector: &str) -> Result<String, CliError> {
    let selector = validate_label(selector, "section reference", MAX_SECTION_NAME_BYTES)?;
    if let Ok(id) = Uuid::parse_str(&selector) {
        let id = id.to_string();
        if store.sections.iter().any(|section| section.id == id) {
            return Ok(id);
        }
        return Err(CliError::Usage(format!(
            "sidebar section does not exist: {id}"
        )));
    }
    let matches: Vec<&WireSection> = store
        .sections
        .iter()
        .filter(|section| section.name.eq_ignore_ascii_case(&selector))
        .collect();
    match matches.as_slice() {
        [section] => Ok(section.id.clone()),
        [] => Err(CliError::Usage(format!(
            "sidebar section does not exist: {selector}"
        ))),
        _ => Err(CliError::Usage(format!(
            "sidebar section name {selector:?} is ambiguous; use an ID"
        ))),
    }
}

fn validate_wire_store(store: &WireStore) -> Result<(), CliError> {
    if store.version != 1 {
        return Err(CliError::Usage(format!(
            "unsupported sidebar layout version: {}",
            store.version
        )));
    }
    if store.sections.len() > MAX_SECTIONS {
        return Err(CliError::Usage(format!(
            "sidebar layout exceeds the maximum of {MAX_SECTIONS} sections"
        )));
    }
    if store.assignments.len() > MAX_ASSIGNED_CHANNELS {
        return Err(CliError::Usage(format!(
            "sidebar layout exceeds the maximum of {MAX_ASSIGNED_CHANNELS} assigned channels"
        )));
    }

    let mut ids = HashSet::new();
    let mut orders = HashSet::new();
    for section in &store.sections {
        canonical_uuid(&section.id, "section ID")?;
        validate_label(&section.name, "section name", MAX_SECTION_NAME_BYTES)?;
        if let Some(icon) = section.icon.as_deref() {
            validate_label(icon, "section icon", MAX_ICON_BYTES)?;
        }
        if !ids.insert(section.id.as_str()) {
            return Err(CliError::Usage(format!(
                "duplicate sidebar section ID: {}",
                section.id
            )));
        }
        if !orders.insert(section.order) {
            return Err(CliError::Usage(format!(
                "duplicate sidebar section order: {}",
                section.order
            )));
        }
    }
    if (0..store.sections.len() as u32).any(|order| !orders.contains(&order)) {
        return Err(CliError::Usage(
            "sidebar section orders must be contiguous from zero".into(),
        ));
    }
    for (channel_id, section_id) in &store.assignments {
        canonical_uuid(channel_id, "assigned channel ID")?;
        canonical_uuid(section_id, "assigned section ID")?;
        if !ids.contains(section_id.as_str()) {
            return Err(CliError::Usage(format!(
                "channel {channel_id} references unknown section {section_id}"
            )));
        }
    }
    Ok(())
}

fn normalize_wire_store(store: &mut WireStore) {
    store.sections.sort_by_key(|section| section.order);
}

fn resolve_expected_revision(
    embedded: Option<&str>,
    argument: Option<&str>,
) -> Result<Option<String>, CliError> {
    let embedded = embedded.map(parse_revision).transpose()?;
    let argument = argument.map(parse_revision).transpose()?;
    match (embedded, argument) {
        (Some(left), Some(right)) if left != right => Err(CliError::Usage(
            "input revision and --expected-revision disagree".into(),
        )),
        (Some(value), _) | (_, Some(value)) => Ok(value),
        (None, None) => Ok(None),
    }
}

fn parse_revision(value: &str) -> Result<Option<String>, CliError> {
    if value == "none" {
        return Ok(None);
    }
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(CliError::Usage(
            "revision must be 'none' or a 64-character lowercase event ID".into(),
        ));
    }
    Ok(Some(value.to_string()))
}

fn ensure_expected_revision(
    current: Option<&RemoteHead>,
    expected: Option<&str>,
) -> Result<(), CliError> {
    let actual = current.map(|head| head.revision.as_str());
    if actual == expected {
        return Ok(());
    }
    Err(CliError::Conflict(format!(
        "sidebar layout revision mismatch (expected {}, found {}); list it again before retrying",
        expected.unwrap_or("none"),
        actual.unwrap_or("none")
    )))
}

fn canonical_uuid(value: &str, label: &str) -> Result<String, CliError> {
    let parsed =
        Uuid::parse_str(value).map_err(|_| CliError::Usage(format!("invalid {label}: {value}")))?;
    let canonical = parsed.to_string();
    if canonical != value {
        return Err(CliError::Usage(format!(
            "{label} must use canonical lowercase hyphenated UUID form: {value}"
        )));
    }
    Ok(canonical)
}

fn validate_label(value: &str, label: &str, max_bytes: usize) -> Result<String, CliError> {
    let value = value.trim();
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(CliError::Usage(format!(
            "{label} must be 1-{max_bytes} bytes and contain no control characters"
        )));
    }
    Ok(value.to_string())
}

fn make_tag(parts: &[&str]) -> Result<Tag, CliError> {
    Tag::parse(parts.iter().copied())
        .map_err(|e| CliError::Other(format!("failed to construct sidebar tag: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECTION_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
    const OTHER_SECTION_ID: &str = "550e8400-e29b-41d4-a716-446655440001";
    const CHANNEL_ID: &str = "d9428888-122b-11e1-b85c-61cd3cbb3210";

    fn index(entries: &[(&str, &str)]) -> ChannelIndex {
        let mut index = ChannelIndex::default();
        for (id, name) in entries {
            index.by_id.insert((*id).to_string(), (*name).to_string());
            index
                .by_name
                .entry(name.to_lowercase())
                .or_default()
                .push((*id).to_string());
        }
        index
    }

    fn empty_layout(section: DesiredSection) -> DesiredLayout {
        DesiredLayout {
            revision: None,
            version: 1,
            sections: vec![section],
        }
    }

    #[test]
    fn materialize_resolves_channel_name_and_preserves_existing_section_id() {
        let current = WireStore {
            version: 1,
            sections: vec![WireSection {
                id: SECTION_ID.into(),
                name: "Core".into(),
                icon: None,
                order: 0,
            }],
            assignments: BTreeMap::new(),
        };
        let desired = empty_layout(DesiredSection {
            id: None,
            name: "Core".into(),
            icon: Some("folder".into()),
            channels: vec![ChannelSelector::NameOrId("General".into())],
        });

        let store = materialize_layout(desired, &current, &index(&[(CHANNEL_ID, "General")]))
            .expect("materialize layout");

        assert_eq!(store.sections[0].id, SECTION_ID);
        assert_eq!(store.sections[0].icon.as_deref(), Some("folder"));
        assert_eq!(store.assignments.get(CHANNEL_ID).unwrap(), SECTION_ID);
    }

    #[test]
    fn materialize_rejects_ambiguous_channel_name() {
        let second_channel = "d9428888-122b-11e1-b85c-61cd3cbb3211";
        let desired = empty_layout(DesiredSection {
            id: Some(SECTION_ID.into()),
            name: "Core".into(),
            icon: None,
            channels: vec![ChannelSelector::NameOrId("General".into())],
        });

        let error = materialize_layout(
            desired,
            &WireStore::default(),
            &index(&[(CHANNEL_ID, "General"), (second_channel, "general")]),
        )
        .expect_err("ambiguous names must fail");

        assert!(error.to_string().contains("ambiguous"));
    }

    #[test]
    fn materialize_rejects_channel_assigned_to_two_sections() {
        let desired = DesiredLayout {
            revision: None,
            version: 1,
            sections: vec![
                DesiredSection {
                    id: Some(SECTION_ID.into()),
                    name: "Core".into(),
                    icon: None,
                    channels: vec![ChannelSelector::NameOrId(CHANNEL_ID.into())],
                },
                DesiredSection {
                    id: Some(OTHER_SECTION_ID.into()),
                    name: "Other".into(),
                    icon: None,
                    channels: vec![ChannelSelector::NameOrId(CHANNEL_ID.into())],
                },
            ],
        };

        let error = materialize_layout(
            desired,
            &WireStore::default(),
            &index(&[(CHANNEL_ID, "General")]),
        )
        .expect_err("duplicate assignment must fail");

        assert!(error.to_string().contains("assigned to both"));
    }

    #[test]
    fn duplicate_section_names_require_explicit_ids() {
        let mut first = DesiredSection {
            id: None,
            name: "Core".into(),
            icon: None,
            channels: Vec::new(),
        };
        let second = DesiredSection {
            id: Some(OTHER_SECTION_ID.into()),
            name: "core".into(),
            icon: None,
            channels: Vec::new(),
        };
        let layout = |first: DesiredSection| DesiredLayout {
            revision: None,
            version: 1,
            sections: vec![first, second.clone()],
        };

        assert!(materialize_layout(
            layout(first.clone()),
            &WireStore::default(),
            &ChannelIndex::default(),
        )
        .unwrap_err()
        .to_string()
        .contains("requires explicit IDs"));

        first.id = Some(SECTION_ID.into());
        assert!(materialize_layout(
            layout(first),
            &WireStore::default(),
            &ChannelIndex::default(),
        )
        .is_ok());
    }

    #[test]
    fn individual_mutations_create_move_rename_and_unassign() {
        let mut store = WireStore::default();
        create_section(&mut store, " Core ", Some("folder")).expect("create section");
        let section_id = store.sections[0].id.clone();
        let channels = index(&[(CHANNEL_ID, "General")]);

        assign_channels(&mut store, "Core", vec!["General".into()], &channels)
            .expect("assign by exact name");
        assert_eq!(store.assignments.get(CHANNEL_ID), Some(&section_id));

        rename_section(&mut store, &section_id, Some("Operations"), Some(None))
            .expect("rename and clear icon");
        assert_eq!(store.sections[0].name, "Operations");
        assert_eq!(store.sections[0].icon, None);

        unassign_channels(&mut store, vec![CHANNEL_ID.into()], &channels).expect("unassign by ID");
        assert!(store.assignments.is_empty());
        validate_wire_store(&store).expect("valid mutated store");
    }

    #[test]
    fn individual_reorder_and_delete_keep_store_contiguous() {
        let mut store = WireStore {
            version: 1,
            sections: vec![
                WireSection {
                    id: SECTION_ID.into(),
                    name: "First".into(),
                    icon: None,
                    order: 0,
                },
                WireSection {
                    id: OTHER_SECTION_ID.into(),
                    name: "Second".into(),
                    icon: None,
                    order: 1,
                },
            ],
            assignments: BTreeMap::from([(CHANNEL_ID.into(), SECTION_ID.into())]),
        };

        reorder_sections(&mut store, vec!["Second".into(), "First".into()])
            .expect("reorder complete layout");
        assert_eq!(store.sections[0].id, OTHER_SECTION_ID);
        assert_eq!(store.sections[1].order, 1);

        delete_section(&mut store, "First").expect("delete section");
        assert_eq!(store.sections.len(), 1);
        assert_eq!(store.sections[0].order, 0);
        assert!(store.assignments.is_empty());
        validate_wire_store(&store).expect("valid mutated store");
    }

    #[test]
    fn individual_operations_fail_on_ambiguous_section_names() {
        let store = WireStore {
            version: 1,
            sections: vec![
                WireSection {
                    id: SECTION_ID.into(),
                    name: "Core".into(),
                    icon: None,
                    order: 0,
                },
                WireSection {
                    id: OTHER_SECTION_ID.into(),
                    name: "core".into(),
                    icon: None,
                    order: 1,
                },
            ],
            assignments: BTreeMap::new(),
        };

        assert!(resolve_section_id(&store, "CORE")
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
    }

    #[test]
    fn wire_validation_rejects_non_contiguous_orders_and_orphans() {
        let invalid_order = WireStore {
            version: 1,
            sections: vec![WireSection {
                id: SECTION_ID.into(),
                name: "Core".into(),
                icon: None,
                order: 1,
            }],
            assignments: BTreeMap::new(),
        };
        assert!(validate_wire_store(&invalid_order)
            .unwrap_err()
            .to_string()
            .contains("contiguous"));

        let orphan = WireStore {
            version: 1,
            sections: Vec::new(),
            assignments: BTreeMap::from([(CHANNEL_ID.into(), SECTION_ID.into())]),
        };
        assert!(validate_wire_store(&orphan)
            .unwrap_err()
            .to_string()
            .contains("unknown section"));
    }

    #[test]
    fn listed_layout_round_trips_as_apply_input() {
        let head = RemoteHead {
            revision: "a".repeat(64),
            created_at: 1,
            store: WireStore {
                version: 1,
                sections: vec![WireSection {
                    id: SECTION_ID.into(),
                    name: "Core".into(),
                    icon: None,
                    order: 0,
                }],
                assignments: BTreeMap::from([(CHANNEL_ID.into(), SECTION_ID.into())]),
            },
        };
        let json = serde_json::to_string(&listed_layout(
            Some(&head),
            &index(&[(CHANNEL_ID, "General")]),
        ))
        .expect("serialize listed layout");
        let desired: DesiredLayout = serde_json::from_str(&json).expect("parse listed layout");
        let materialized =
            materialize_layout(desired, &head.store, &index(&[(CHANNEL_ID, "General")]))
                .expect("materialize listed layout");

        assert_eq!(materialized, head.store);
    }

    #[test]
    fn listed_layout_uses_explicit_section_order() {
        let head = RemoteHead {
            revision: "a".repeat(64),
            created_at: 1,
            store: WireStore {
                version: 1,
                sections: vec![
                    WireSection {
                        id: OTHER_SECTION_ID.into(),
                        name: "Second".into(),
                        icon: None,
                        order: 1,
                    },
                    WireSection {
                        id: SECTION_ID.into(),
                        name: "First".into(),
                        icon: None,
                        order: 0,
                    },
                ],
                assignments: BTreeMap::new(),
            },
        };

        let mut normalized = head.store.clone();
        normalize_wire_store(&mut normalized);
        let listed = listed_layout(
            Some(&RemoteHead {
                store: normalized,
                ..head
            }),
            &ChannelIndex::default(),
        );

        assert_eq!(listed.sections[0].name, "First");
        assert_eq!(listed.sections[1].name, "Second");
    }

    #[test]
    fn self_encrypted_event_matches_desktop_envelope() {
        let keys = nostr::Keys::generate();
        let plaintext = serde_json::to_string(&WireStore::default()).unwrap();
        let ciphertext = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            &plaintext,
            Version::V2,
        )
        .expect("encrypt to self");
        let event = EventBuilder::new(Kind::Custom(KIND_CHANNEL_SECTIONS), ciphertext)
            .tags([
                make_tag(&["d", D_TAG]).unwrap(),
                make_tag(&["t", D_TAG]).unwrap(),
            ])
            .sign_with_keys(&keys)
            .expect("sign event");

        validate_remote_event(&event, &keys.public_key().to_hex()).expect("valid envelope");
        let decrypted = nip44::decrypt(keys.secret_key(), &keys.public_key(), &event.content)
            .expect("decrypt from self");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn expected_revision_rejects_disagreement_and_stale_head() {
        let first = "a".repeat(64);
        let second = "b".repeat(64);
        assert!(resolve_expected_revision(Some(&first), Some(&second)).is_err());

        let head = RemoteHead {
            revision: second,
            created_at: 1,
            store: WireStore::default(),
        };
        let error = ensure_expected_revision(Some(&head), Some(&first))
            .expect_err("stale expected revision must fail");
        assert!(matches!(error, CliError::Conflict(_)));
    }
}
