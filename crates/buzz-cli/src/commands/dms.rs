use uuid::Uuid;

use crate::client::{extract_d_tag, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_uuid, sdk_err, validate_hex64};

/// Build the filter that finds the caller's DM conversations.
///
/// DMs are discoverable through their kind:39000 channel metadata, which the
/// relay signs on creation with `t=dm`, `hidden`, and one `p` tag per
/// participant (`emit_group_discovery_events`). Filtering that kind by our own
/// pubkey therefore returns exactly the DMs we are a party to — `p` tags are
/// only attached to DM discovery, and channel-scoped storage keeps other
/// people's DMs unreadable regardless.
fn dm_discovery_filter(my_pubkey: &str, limit: u32) -> serde_json::Value {
    serde_json::json!({
        "kinds": [39000],
        "#p": [my_pubkey],
        "limit": limit
    })
}

/// Read every value of a single-letter tag off an event.
fn tag_values(event: &serde_json::Value, name: &str) -> Vec<String> {
    event
        .get("tags")
        .and_then(|t| t.as_array())
        .map(|tags| {
            tags.iter()
                .filter_map(|tag| {
                    let arr = tag.as_array()?;
                    if arr.first()?.as_str()? == name {
                        arr.get(1)?.as_str().map(str::to_string)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Project a kind:39000 discovery event into a `dms list` row.
///
/// Returns `None` for anything that is not a DM. The filter cannot express
/// "has a `p` tag *and* is a DM" any more precisely than `#p`, so the type
/// check happens here against the `t` tag the relay always writes.
fn dm_summary(event: &serde_json::Value) -> Option<serde_json::Value> {
    if !tag_values(event, "t").iter().any(|t| t == "dm") {
        return None;
    }
    let dm_id = extract_d_tag(event);
    if dm_id.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "dm_id": dm_id,
        "participants": tag_values(event, "p"),
        "created_at": event.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
    }))
}

/// List the DM conversations the authenticated identity is a party to.
pub async fn cmd_list_dms(client: &BuzzClient, limit: Option<u32>) -> Result<(), CliError> {
    let my_pk = client.keys().public_key().to_hex();
    let limit = limit.unwrap_or(50).min(200);
    let resp = client.query(&dm_discovery_filter(&my_pk, limit)).await?;
    let events: Vec<serde_json::Value> = serde_json::from_str(&resp).unwrap_or_default();
    let dms: Vec<serde_json::Value> = events.iter().filter_map(dm_summary).collect();
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
    use super::{dm_discovery_filter, dm_summary};
    use serde_json::json;

    const ME: &str = "4c619afde8bb468423c541af450121aec954e9adc8e8d33daea957e281272125";
    const THEM: &str = "f95f47023f13e605778566950e2ce795676930c5db06a2cb7f6e35cae73c2f84";
    const DM_ID: &str = "1da3e67e-3626-4ec1-8e13-59345cafcb93";

    fn dm_discovery_event() -> serde_json::Value {
        // Shape written by emit_group_discovery_events for a DM channel.
        json!({
            "kind": 39000,
            "created_at": 1_754_000_000u64,
            "tags": [
                ["d", DM_ID],
                ["public"],
                ["hidden"],
                ["p", ME],
                ["p", THEM],
                ["closed"],
                ["t", "dm"],
            ],
        })
    }

    #[test]
    fn the_filter_asks_for_dm_discovery_by_participant() {
        // kind:41001 is never published by the relay, so the old filter could
        // only ever return an empty list.
        let filter = dm_discovery_filter(ME, 50);
        assert_eq!(filter["kinds"], json!([39000]));
        assert_eq!(filter["#p"], json!([ME]));
        assert_eq!(filter["limit"], json!(50));
    }

    #[test]
    fn a_dm_projects_to_its_id_and_participants() {
        let row = dm_summary(&dm_discovery_event()).expect("a dm must be listed");
        assert_eq!(row["dm_id"], json!(DM_ID));
        assert_eq!(row["participants"], json!([ME, THEM]));
        assert_eq!(row["created_at"], json!(1_754_000_000u64));
    }

    #[test]
    fn a_non_dm_channel_is_not_listed() {
        // A stream channel we are p-tagged on must not appear in dms list.
        let mut event = dm_discovery_event();
        event["tags"] = json!([["d", DM_ID], ["p", ME], ["t", "stream"]]);
        assert!(dm_summary(&event).is_none());
    }

    #[test]
    fn an_untyped_event_is_not_listed() {
        let mut event = dm_discovery_event();
        event["tags"] = json!([["d", DM_ID], ["p", ME]]);
        assert!(dm_summary(&event).is_none());
    }

    #[test]
    fn a_dm_with_no_d_tag_is_skipped_rather_than_listed_without_an_id() {
        // An id-less row is worse than a missing one: the caller cannot act on
        // it and cannot tell it apart from a real DM.
        let mut event = dm_discovery_event();
        event["tags"] = json!([["p", ME], ["t", "dm"]]);
        assert!(dm_summary(&event).is_none());
    }
}
