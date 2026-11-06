use uuid::Uuid;

use crate::client::{extract_d_tag, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_uuid, sdk_err, validate_hex64};

/// Tag values named `name` on a relay event.
fn tag_values<'a>(event: &'a serde_json::Value, name: &'a str) -> impl Iterator<Item = &'a str> {
    event
        .get("tags")
        .and_then(|t| t.as_array())
        .into_iter()
        .flatten()
        .filter_map(move |tag| {
            let arr = tag.as_array()?;
            (arr.first()?.as_str()? == name)
                .then(|| arr.get(1)?.as_str())
                .flatten()
        })
}

/// List visible DM conversations: the viewer's DM group metadata (kind:39000
/// `t=dm`, one `p` per participant), minus the DMs in the viewer's newest
/// relay-signed visibility snapshot (kind:30622, one `h` per hidden DM).
pub async fn cmd_list_dms(client: &BuzzClient, limit: Option<u32>) -> Result<(), CliError> {
    let my_pk = client.public_key().to_hex();
    let limit = limit.unwrap_or(50).min(200);
    let snapshots = client
        .query_paginated(
            serde_json::json!({
                "kinds": [buzz_sdk::kind::KIND_DM_VISIBILITY],
                "#p": [my_pk],
            }),
            10,
        )
        .await?;
    let hidden: std::collections::HashSet<&str> = snapshots
        .iter()
        .max_by_key(|e| e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0))
        .map(|e| tag_values(e, "h").collect())
        .unwrap_or_default();
    let events = client
        .query_paginated(
            serde_json::json!({
                "kinds": [39000],
                "#t": ["dm"],
                "#p": [my_pk],
            }),
            200,
        )
        .await?;
    let dms: Vec<serde_json::Value> = events
        .iter()
        .map(|e| (extract_d_tag(e), e))
        .filter(|(dm_id, _)| !dm_id.is_empty() && !hidden.contains(dm_id.as_str()))
        .take(limit as usize)
        .map(|(dm_id, e)| {
            let participants: Vec<&str> = tag_values(e, "p").collect();
            serde_json::json!({
                "dm_id": dm_id,
                "participants": participants,
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
            })
        })
        .collect();
    let output = serde_json::to_string(&dms).unwrap_or_default();
    println!("{output}");
    Ok(())
}

/// Open a DM with one or more users — sign and submit a kind:41010 event with a d-tag.
pub async fn cmd_open_dm(client: &BuzzClient, pubkeys: &[String]) -> Result<(), CliError> {
    if pubkeys.is_empty() || pubkeys.len() > 8 {
        return Err(CliError::Usage("--pubkey: must provide 1-8 pubkeys".into()));
    }
    for pk in pubkeys {
        validate_hex64(pk)?;
    }
    let dm_id = Uuid::new_v4().to_string();
    let refs: Vec<&str> = pubkeys.iter().map(String::as_str).collect();

    // build_dm_open doesn't accept a d-tag, so we build the event manually
    // using the SDK builder and add the d-tag ourselves.
    use nostr::{EventBuilder, Kind, Tag};
    let mut tags: Vec<Tag> = refs
        .iter()
        .map(|pk| Tag::parse(["p", *pk]).map_err(|e| CliError::Other(format!("tag error: {e}"))))
        .collect::<Result<Vec<_>, _>>()?;
    tags.push(Tag::parse(["d", &dm_id]).map_err(|e| CliError::Other(format!("tag error: {e}")))?);
    let builder = EventBuilder::new(Kind::Custom(41010), "").tags(tags);
    let event = client.sign_event(builder).await?;

    let resp = client.submit_event(event).await?;
    // Try to extract relay-assigned channel_id from response message.
    // Relay returns: {"event_id":"...","accepted":true,"message":"response:{\"channel_id\":\"...\",\"created\":true}"}
    let relay_dm_id = serde_json::from_str::<serde_json::Value>(&resp)
        .ok()
        .and_then(|v| v.get("message")?.as_str().map(|s| s.to_string()))
        .and_then(|msg| {
            let json_part = msg.strip_prefix("response:")?;
            serde_json::from_str::<serde_json::Value>(json_part).ok()
        })
        .and_then(|v| v.get("channel_id")?.as_str().map(|s| s.to_string()));
    let final_dm_id = relay_dm_id.unwrap_or(dm_id);

    let mut normalized: serde_json::Value =
        serde_json::from_str(&resp).unwrap_or(serde_json::json!({}));
    normalized["dm_id"] = serde_json::json!(final_dm_id);
    if normalized.get("accepted").is_none() {
        normalized["accepted"] = serde_json::json!(true);
    }
    println!("{normalized}");
    Ok(())
}

/// Hide a DM channel — sign and submit a kind:41012 event with h-tag.
pub async fn cmd_hide_dm(client: &BuzzClient, channel_id: &str) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;

    use nostr::{EventBuilder, Kind, Tag};
    let tags = vec![Tag::parse(["h", &channel_uuid.to_string()])
        .map_err(|e| CliError::Other(format!("tag error: {e}")))?];
    let builder =
        EventBuilder::new(Kind::Custom(buzz_sdk::kind::KIND_DM_HIDE as u16), "").tags(tags);
    let event = client.sign_event(builder).await?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

/// Add a member to a DM group — sign and submit a kind:41011 event.
pub async fn cmd_add_dm_member(
    client: &BuzzClient,
    channel_id: &str,
    pubkey: &str,
) -> Result<(), CliError> {
    let channel_uuid = parse_uuid(channel_id)?;
    validate_hex64(pubkey)?;

    let builder = buzz_sdk::build_dm_add_member(channel_uuid, pubkey).map_err(sdk_err)?;
    let event = client.sign_event(builder).await?;

    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn dispatch(cmd: crate::DmsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::DmsCmd;
    match cmd {
        DmsCmd::List { limit } => cmd_list_dms(client, limit).await,
        DmsCmd::Open { pubkeys } => cmd_open_dm(client, &pubkeys).await,
        DmsCmd::AddMember { channel, pubkey } => cmd_add_dm_member(client, &channel, &pubkey).await,
        DmsCmd::Hide { channel } => cmd_hide_dm(client, &channel).await,
    }
}
