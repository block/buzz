use uuid::Uuid;

use crate::client::{
    extract_d_tag, extract_p_tags, extract_tag_value, normalize_write_response,
    print_create_response, SproutClient,
};
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin, validate_hex64, validate_uuid};

// ---------------------------------------------------------------------------
// Read commands — POST /query
// ---------------------------------------------------------------------------

fn extract_channel_metadata(e: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "channel_id": extract_d_tag(e),
        "name": extract_tag_value(e, "name"),
        "description": extract_tag_value(e, "about"),
        "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
    })
}

pub async fn cmd_list_channels(
    client: &SproutClient,
    visibility: Option<&str>,
    member: Option<bool>,
    limit: Option<u32>,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    let effective_limit = limit.unwrap_or(500);
    let raw = if member == Some(true) {
        // Step 1: find channel IDs where we're a member (kind:39002)
        let my_pk = client.keys().public_key().to_hex();
        let member_filter = serde_json::json!({
            "kinds": [39002],
            "#p": [my_pk],
            "limit": effective_limit
        });
        let member_resp = client.query(&member_filter).await?;
        let member_events: Vec<serde_json::Value> =
            serde_json::from_str(&member_resp).unwrap_or_default();
        let channel_ids: Vec<String> = member_events
            .iter()
            .map(extract_d_tag)
            .filter(|id| !id.is_empty())
            .collect();
        if channel_ids.is_empty() {
            println!("[]");
            return Ok(());
        }
        // Step 2: fetch kind:39000 metadata for those channels
        let metadata_filter = serde_json::json!({
            "kinds": [39000],
            "#d": channel_ids,
            "limit": effective_limit
        });
        client.query(&metadata_filter).await?
    } else {
        let filter = serde_json::json!({
            "kinds": [39000],
            "limit": effective_limit
        });
        client.query(&filter).await?
    };

    let events: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
    let channels: Vec<serde_json::Value> = events
        .iter()
        .filter(|e| {
            if let Some(vis) = visibility {
                // NIP-29: relay emits ["public"] or ["private"] single-element tags
                let nip29_tag = match vis {
                    "open" => "public",
                    _ => vis,
                };
                e.get("tags")
                    .and_then(|t| t.as_array())
                    .map(|tags| {
                        tags.iter().any(|tag| {
                            tag.as_array()
                                .map(|a| {
                                    a.len() == 1
                                        && a.first().and_then(|v| v.as_str()) == Some(nip29_tag)
                                })
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            } else {
                true
            }
        })
        .map(extract_channel_metadata)
        .collect();
    let output = match format {
        crate::OutputFormat::Compact => {
            let compact: Vec<serde_json::Value> = channels
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "channel_id": c.get("channel_id").cloned().unwrap_or_default(),
                        "name": c.get("name").cloned().unwrap_or_default(),
                    })
                })
                .collect();
            serde_json::to_string(&compact).unwrap_or_default()
        }
        crate::OutputFormat::Json => serde_json::to_string(&channels).unwrap_or_default(),
    };
    println!("{output}");
    Ok(())
}

/// Search channels by human-readable name (kind:39000 group metadata).
///
/// The relay's access control already filters out channels the caller can't see
/// (private channels they're not a member of), so we just post-filter the
/// returned events by name and project them into a stable JSON shape.
pub async fn cmd_search_channels(
    client: &SproutClient,
    query: &str,
    exact: bool,
    include_archived: bool,
    limit: u32,
) -> Result<(), CliError> {
    if query.trim().is_empty() {
        return Err(CliError::Usage("--query cannot be empty".into()));
    }

    let filter = serde_json::json!({
        "kinds": [39000],
        "limit": limit,
    });
    let raw = client.query(&filter).await?;

    let events: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| CliError::Other(format!("failed to parse response: {e}")))?;
    let Some(arr) = events.as_array() else {
        println!("[]");
        return Ok(());
    };

    let needle = query.to_ascii_lowercase();
    let mut matches: Vec<ChannelSummary> = arr
        .iter()
        .filter_map(ChannelSummary::from_event)
        .filter(|c| if include_archived { true } else { !c.archived })
        .filter(|c| name_matches(&c.name, &needle, exact))
        .collect();
    matches.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.channel_id.cmp(&b.channel_id))
    });

    let output = serde_json::to_string(&matches).expect("serializing ChannelSummary");
    println!("{output}");
    Ok(())
}

/// Stable, scriptable projection of a kind:39000 channel-metadata event.
#[derive(serde::Serialize)]
struct ChannelSummary {
    channel_id: String,
    name: String,
    channel_type: Option<String>,
    visibility: Option<String>,
    archived: bool,
    about: Option<String>,
    topic: Option<String>,
    purpose: Option<String>,
}

impl ChannelSummary {
    /// Parse a kind:39000 event JSON value into a summary. Returns `None` if the
    /// event lacks the required `d` (channel UUID) or `name` tags.
    fn from_event(event: &serde_json::Value) -> Option<Self> {
        let tags = event.get("tags")?.as_array()?;
        let mut channel_id: Option<String> = None;
        let mut name: Option<String> = None;
        let mut channel_type: Option<String> = None;
        let mut visibility: Option<String> = None;
        let mut archived = false;
        let mut about: Option<String> = None;
        let mut topic: Option<String> = None;
        let mut purpose: Option<String> = None;

        for tag in tags {
            let Some(tag_arr) = tag.as_array() else {
                continue;
            };
            let key = tag_arr.first().and_then(|v| v.as_str()).unwrap_or("");
            let val = tag_arr.get(1).and_then(|v| v.as_str());
            match key {
                "d" => channel_id = val.map(str::to_string),
                "name" => name = val.map(str::to_string),
                "t" => channel_type = val.map(str::to_string),
                // NIP-29 emits both `private` and `public` (Sprout adds the latter).
                // The presence of either tag is the source of truth; tag value is unused.
                "private" => visibility = Some("private".to_string()),
                "public" => visibility = Some("public".to_string()),
                "about" => about = val.map(str::to_string),
                "topic" => topic = val.map(str::to_string),
                "purpose" => purpose = val.map(str::to_string),
                "archived" => archived = val == Some("true"),
                _ => {}
            }
        }

        Some(ChannelSummary {
            channel_id: channel_id?,
            name: name?,
            channel_type,
            visibility,
            archived,
            about,
            topic,
            purpose,
        })
    }
}

fn name_matches(name: &str, needle_lower: &str, exact: bool) -> bool {
    let hay = name.to_ascii_lowercase();
    if exact {
        hay == needle_lower
    } else {
        hay.contains(needle_lower)
    }
}

pub async fn cmd_get_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [39000],
        "#d": [channel_id],
        "limit": 1
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    if let Some(e) = events.first() {
        let mut normalized = extract_channel_metadata(e);
        normalized["pubkey"] =
            serde_json::json!(e.get("pubkey").and_then(|v| v.as_str()).unwrap_or(""));
        println!("{normalized}");
    } else {
        println!("null");
    }
    Ok(())
}

pub async fn cmd_list_channel_members(
    client: &SproutClient,
    channel_id: &str,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [39002],
        "#d": [channel_id],
        "limit": 1
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let members = events.first().map(extract_p_tags).unwrap_or_default();
    let output = serde_json::to_string(&members).unwrap_or_default();
    println!("{output}");
    Ok(())
}

pub async fn cmd_get_canvas(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    let filter = serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id]
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    if let Some(content) = events
        .first()
        .and_then(|e| e.get("content"))
        .and_then(|c| c.as_str())
    {
        println!("{content}");
    } else {
        println!("null");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Write commands — signed events via POST /events
// ---------------------------------------------------------------------------

pub async fn cmd_create_channel(
    client: &SproutClient,
    name: &str,
    channel_type: &str,
    visibility: &str,
    description: Option<&str>,
) -> Result<(), CliError> {
    match channel_type {
        "stream" | "forum" => {}
        _ => {
            return Err(CliError::Usage(format!(
                "--type must be 'stream' or 'forum' (got: {channel_type})"
            )))
        }
    }
    match visibility {
        "open" | "private" => {}
        _ => {
            return Err(CliError::Usage(format!(
                "--visibility must be 'open' or 'private' (got: {visibility})"
            )))
        }
    }

    let channel_uuid = Uuid::new_v4();

    let vis = match visibility {
        "open" => sprout_sdk::Visibility::Open,
        "private" => sprout_sdk::Visibility::Private,
        _ => unreachable!(),
    };
    let ct = match channel_type {
        "stream" => sprout_sdk::ChannelKind::Stream,
        "forum" => sprout_sdk::ChannelKind::Forum,
        _ => unreachable!(),
    };
    let builder =
        sprout_sdk::build_create_channel(channel_uuid, name, Some(vis), Some(ct), description)
            .map_err(|e| CliError::Other(format!("build_create_channel failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    print_create_response(&resp, "channel_id", &channel_uuid.to_string());
    Ok(())
}

pub async fn cmd_update_channel(
    client: &SproutClient,
    channel_id: &str,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<(), CliError> {
    if name.is_none() && description.is_none() {
        return Err(CliError::Usage(
            "at least one field required (--name, --description)".into(),
        ));
    }
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_update_channel(channel_uuid, name, description, None, None)
        .map_err(|e| CliError::Other(format!("build_update_channel failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_set_channel_topic(
    client: &SproutClient,
    channel_id: &str,
    topic: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_set_topic(channel_uuid, topic)
        .map_err(|e| CliError::Other(format!("build_set_topic failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_set_channel_purpose(
    client: &SproutClient,
    channel_id: &str,
    purpose: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_set_purpose(channel_uuid, purpose)
        .map_err(|e| CliError::Other(format!("build_set_purpose failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_join_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_join(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_join failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_leave_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_leave(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_leave failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_archive_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_archive(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_archive failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_unarchive_channel(
    client: &SproutClient,
    channel_id: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_unarchive(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_unarchive failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_delete_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_delete_channel(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_delete_channel failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_add_channel_member(
    client: &SproutClient,
    channel_id: &str,
    pubkey: &str,
    role: Option<&str>,
) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    let channel_uuid = parse_uuid(channel_id)?;

    let typed_role = match role {
        None => None,
        Some("owner") => Some(sprout_sdk::MemberRole::Owner),
        Some("admin") => Some(sprout_sdk::MemberRole::Admin),
        Some("member") => Some(sprout_sdk::MemberRole::Member),
        Some("guest") => Some(sprout_sdk::MemberRole::Guest),
        Some("bot") => Some(sprout_sdk::MemberRole::Bot),
        Some(other) => {
            return Err(CliError::Usage(format!(
                "--role must be owner/admin/member/guest/bot (got: {other})"
            )))
        }
    };
    let builder = sprout_sdk::build_add_member(channel_uuid, pubkey, typed_role)
        .map_err(|e| CliError::Other(format!("build_add_member failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_remove_channel_member(
    client: &SproutClient,
    channel_id: &str,
    pubkey: &str,
) -> Result<(), CliError> {
    validate_hex64(pubkey)?;
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_remove_member(channel_uuid, pubkey)
        .map_err(|e| CliError::Other(format!("build_remove_member failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Set the channel addition policy — sign and submit a kind:10100 (agent profile) event.
pub async fn cmd_set_add_policy(client: &SproutClient, policy: &str) -> Result<(), CliError> {
    match policy {
        "anyone" | "owner_only" | "nobody" => {}
        _ => {
            return Err(CliError::Usage(format!(
                "--policy must be 'anyone', 'owner_only', or 'nobody' (got: {policy})"
            )))
        }
    }

    let content = serde_json::json!({ "channel_add_policy": policy }).to_string();
    use nostr::{EventBuilder, Kind};
    let builder = EventBuilder::new(
        Kind::Custom(sprout_sdk::kind::KIND_AGENT_PROFILE as u16),
        &content,
    )
    .tags([]);
    let event = client.sign_event(builder)?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn cmd_set_canvas(
    client: &SproutClient,
    channel_id: &str,
    content: &str,
) -> Result<(), CliError> {
    let content = read_or_stdin(content)?;
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_set_canvas(channel_uuid, &content)
        .map_err(|e| CliError::Other(format!("build_set_canvas failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn dispatch(
    cmd: crate::ChannelsCmd,
    client: &SproutClient,
    format: &crate::OutputFormat,
) -> Result<(), CliError> {
    use crate::ChannelsCmd;
    match cmd {
        ChannelsCmd::List {
            visibility,
            member,
            limit,
        } => {
            let vis_str = visibility.as_ref().map(|v| v.to_string());
            cmd_list_channels(client, vis_str.as_deref(), Some(member), limit, format).await
        }
        ChannelsCmd::Get { channel } => cmd_get_channel(client, &channel).await,
        ChannelsCmd::Search {
            query,
            exact,
            include_archived,
            limit,
        } => cmd_search_channels(client, &query, exact, include_archived, limit).await,
        ChannelsCmd::Create {
            name,
            channel_type,
            visibility,
            description,
        } => {
            cmd_create_channel(
                client,
                &name,
                &channel_type.to_string(),
                &visibility.to_string(),
                description.as_deref(),
            )
            .await
        }
        ChannelsCmd::Update {
            channel,
            name,
            description,
        } => cmd_update_channel(client, &channel, name.as_deref(), description.as_deref()).await,
        ChannelsCmd::Topic { channel, topic } => {
            cmd_set_channel_topic(client, &channel, &topic).await
        }
        ChannelsCmd::Purpose { channel, purpose } => {
            cmd_set_channel_purpose(client, &channel, &purpose).await
        }
        ChannelsCmd::Join { channel } => cmd_join_channel(client, &channel).await,
        ChannelsCmd::Leave { channel } => cmd_leave_channel(client, &channel).await,
        ChannelsCmd::Archive { channel } => cmd_archive_channel(client, &channel).await,
        ChannelsCmd::Unarchive { channel } => cmd_unarchive_channel(client, &channel).await,
        ChannelsCmd::Delete { channel } => cmd_delete_channel(client, &channel).await,
        ChannelsCmd::Members { channel } => cmd_list_channel_members(client, &channel).await,
        ChannelsCmd::AddMember {
            channel,
            pubkey,
            role,
        } => cmd_add_channel_member(client, &channel, &pubkey, role.as_deref()).await,
        ChannelsCmd::RemoveMember { channel, pubkey } => {
            cmd_remove_channel_member(client, &channel, &pubkey).await
        }
        ChannelsCmd::SetAddPolicy { policy } => cmd_set_add_policy(client, &policy).await,
    }
}

pub async fn dispatch_canvas(cmd: crate::CanvasCmd, client: &SproutClient) -> Result<(), CliError> {
    use crate::CanvasCmd;
    match cmd {
        CanvasCmd::Get { channel } => cmd_get_canvas(client, &channel).await,
        CanvasCmd::Set { channel, content } => cmd_set_canvas(client, &channel, &content).await,
    }
}

#[cfg(test)]
mod tests {
    use super::{name_matches, ChannelSummary};
    use serde_json::json;

    fn event(tags: serde_json::Value) -> serde_json::Value {
        json!({ "tags": tags })
    }

    #[test]
    fn from_event_extracts_known_tags() {
        let ev = event(json!([
            ["d", "11111111-1111-1111-1111-111111111111"],
            ["name", "sprout-chat-composer"],
            ["t", "stream"],
            ["public"],
            ["about", "About text"],
            ["topic", "Composer work"],
            ["purpose", "Track UI for the composer"],
        ]));
        let s = ChannelSummary::from_event(&ev).expect("parse");
        assert_eq!(s.channel_id, "11111111-1111-1111-1111-111111111111");
        assert_eq!(s.name, "sprout-chat-composer");
        assert_eq!(s.channel_type.as_deref(), Some("stream"));
        assert_eq!(s.visibility.as_deref(), Some("public"));
        assert!(!s.archived);
        assert_eq!(s.about.as_deref(), Some("About text"));
        assert_eq!(s.topic.as_deref(), Some("Composer work"));
        assert_eq!(s.purpose.as_deref(), Some("Track UI for the composer"));
    }

    #[test]
    fn from_event_marks_archived() {
        let ev = event(json!([
            ["d", "11111111-1111-1111-1111-111111111111"],
            ["name", "old-channel"],
            ["archived", "true"],
        ]));
        let s = ChannelSummary::from_event(&ev).expect("parse");
        assert!(s.archived);
    }

    #[test]
    fn from_event_marks_private() {
        let ev = event(json!([
            ["d", "11111111-1111-1111-1111-111111111111"],
            ["name", "secret"],
            ["private"],
        ]));
        let s = ChannelSummary::from_event(&ev).expect("parse");
        assert_eq!(s.visibility.as_deref(), Some("private"));
    }

    #[test]
    fn from_event_returns_none_without_required_tags() {
        // missing `name`
        let ev = event(json!([["d", "11111111-1111-1111-1111-111111111111"]]));
        assert!(ChannelSummary::from_event(&ev).is_none());
        // missing `d`
        let ev = event(json!([["name", "no-id"]]));
        assert!(ChannelSummary::from_event(&ev).is_none());
    }

    #[test]
    fn from_event_tolerates_malformed_tags() {
        // Non-array tag entry, empty tag, single-element tag — all must be skipped, not panic.
        let ev = event(json!([
            "not-an-array",
            [],
            ["name"],
            ["d", "11111111-1111-1111-1111-111111111111"],
            ["name", "fine"],
        ]));
        let s = ChannelSummary::from_event(&ev).expect("parse");
        assert_eq!(s.name, "fine");
    }

    // `name_matches` takes a pre-lowercased needle (caller responsibility, set in
    // cmd_search_channels). Tests follow the same contract.

    #[test]
    fn name_matches_substring_case_insensitive() {
        assert!(name_matches("Sprout-Chat-Composer", "composer", false));
        assert!(name_matches("Sprout-Chat-Composer", "sprout", false));
        assert!(!name_matches("design", "composer", false));
    }

    #[test]
    fn name_matches_exact_case_insensitive() {
        assert!(name_matches("Sprout", "sprout", true));
        assert!(!name_matches("Sprout-Chat", "sprout", true));
    }
}
