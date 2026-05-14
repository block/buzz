use uuid::Uuid;

use crate::client::SproutClient;
use crate::error::CliError;
use crate::validate::{parse_uuid, read_or_stdin, validate_hex64, validate_uuid};

// ---------------------------------------------------------------------------
// Read commands — POST /query
// ---------------------------------------------------------------------------

pub async fn cmd_list_channels(
    client: &SproutClient,
    _visibility: Option<&str>,
    _member: Option<bool>,
) -> Result<(), CliError> {
    // Query kind:39002 channel metadata events.
    // If member=true, filter by #p tag containing our pubkey.
    let my_pk = client.keys().public_key().to_hex();
    let mut filter = serde_json::json!({
        "kinds": [39002]
    });
    // When member filter is requested, query channels where we're a participant
    if _member == Some(true) {
        filter["#p"] = serde_json::json!([my_pk]);
    }
    // Visibility filtering is done client-side from the returned events
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_get_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    // Query kind:39002 with #h tag matching the channel UUID
    let filter = serde_json::json!({
        "kinds": [39002],
        "#h": [channel_id]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_list_channel_members(
    client: &SproutClient,
    channel_id: &str,
) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    // Query kind:39002 channel metadata — members are in the p-tags
    let filter = serde_json::json!({
        "kinds": [39002],
        "#h": [channel_id]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_get_canvas(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    validate_uuid(channel_id)?;
    // Canvas is kind:40100 with #h tag
    let filter = serde_json::json!({
        "kinds": [40100],
        "#h": [channel_id]
    });
    let resp = client.query(&filter).await?;
    println!("{resp}");
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
    println!("{resp}");
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

    let builder = sprout_sdk::build_update_channel(channel_uuid, name, description)
        .map_err(|e| CliError::Other(format!("build_update_channel failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
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
    println!("{resp}");
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
    println!("{resp}");
    Ok(())
}

pub async fn cmd_join_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_join(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_join failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_leave_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_leave(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_leave failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
    Ok(())
}

pub async fn cmd_archive_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_archive(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_archive failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
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
    println!("{resp}");
    Ok(())
}

pub async fn cmd_delete_channel(client: &SproutClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    let builder = sprout_sdk::build_delete_channel(channel_uuid)
        .map_err(|e| CliError::Other(format!("build_delete_channel failed: {e}")))?;

    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{resp}");
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
    println!("{resp}");
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
    println!("{resp}");
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
    println!("{resp}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

pub async fn dispatch(cmd: crate::ChannelsCmd, client: &SproutClient) -> Result<(), CliError> {
    use crate::ChannelsCmd;
    match cmd {
        ChannelsCmd::List { visibility, member } => {
            let vis_str = visibility.as_ref().map(|v| v.to_string());
            cmd_list_channels(client, vis_str.as_deref(), Some(member)).await
        }
        ChannelsCmd::Get { channel } => cmd_get_channel(client, &channel).await,
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
    }
}

pub async fn dispatch_canvas(cmd: crate::CanvasCmd, client: &SproutClient) -> Result<(), CliError> {
    use crate::CanvasCmd;
    match cmd {
        CanvasCmd::Get { channel } => cmd_get_canvas(client, &channel).await,
        CanvasCmd::Set { channel, content } => cmd_set_canvas(client, &channel, &content).await,
    }
}
