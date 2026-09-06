use uuid::Uuid;

use crate::client::{extract_d_tag, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_uuid, sdk_err, validate_hex64};

fn has_tag(event: &serde_json::Value, name: &str, value: &str) -> bool {
    event
        .get("tags")
        .and_then(|tags| tags.as_array())
        .is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.as_array().is_some_and(|values| {
                    values.first().and_then(|entry| entry.as_str()) == Some(name)
                        && values.get(1).and_then(|entry| entry.as_str()) == Some(value)
                })
            })
        })
}

fn is_dm_metadata(event: &serde_json::Value, my_pubkey: &str) -> bool {
    has_tag(event, "t", "dm") && has_tag(event, "p", my_pubkey)
}

/// List DMs from canonical kind:39000 metadata, scoped to this participant.
pub async fn cmd_list_dms(client: &BuzzClient, limit: Option<u32>) -> Result<(), CliError> {
    let my_pk = client.keys().public_key().to_hex();
    let limit = limit.unwrap_or(50).min(200);
    let filter = serde_json::json!({
        "kinds": [39000],
        "#p": [my_pk],
        "limit": limit
    });
    let resp = client.query(&filter).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let dms: Vec<serde_json::Value> = events
        .iter()
        .filter(|event| is_dm_metadata(event, &my_pk))
        .map(|event| {
            let dm_id = extract_d_tag(event);
            let participants: Vec<String> = event
                .get("tags")
                .and_then(|tags| tags.as_array())
                .map(|tags| {
                    tags.iter()
                        .filter_map(|tag| {
                            let values = tag.as_array()?;
                            if values.first()?.as_str()? == "p" {
                                values.get(1)?.as_str().map(str::to_owned)
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            serde_json::json!({
                "dm_id": dm_id,
                "participants": participants,
                "created_at": event.get("created_at").and_then(|value| value.as_u64()).unwrap_or(0),
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
    let event = client.sign_event(builder)?;

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
    let event = client.sign_event(builder)?;

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
    let event = client.sign_event(builder)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn recognizes_canonical_dm_metadata() {
        let event = serde_json::json!({
            "created_at": 42,
            "tags": [
                ["d", "3f5a452d-afd9-4d05-8263-8cfdf68b16b7"],
                ["hidden"],
                ["t", "dm"],
                ["p", VIEWER],
                ["p", OTHER]
            ]
        });

        assert!(is_dm_metadata(&event, VIEWER));
    }

    #[test]
    fn rejects_non_dm_or_mis_scoped_metadata() {
        let stream = serde_json::json!({
            "tags": [["d", "85ca0239-d670-4bc9-ba2b-82fb063b3f3d"], ["t", "stream"], ["p", VIEWER]]
        });
        let other_dm = serde_json::json!({
            "tags": [["d", "5a4021da-4652-4f52-9aa1-b2f937cbeab6"], ["t", "dm"], ["p", OTHER]]
        });

        assert!(!is_dm_metadata(&stream, VIEWER));
        assert!(!is_dm_metadata(&other_dm, VIEWER));
    }
}
