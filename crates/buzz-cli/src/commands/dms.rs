use uuid::Uuid;

use crate::client::{extract_d_tag, normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_uuid, sdk_err, validate_hex64};

/// List DM conversations for our pubkey.
///
/// WIRE FACT: kind:41001 (`KIND_DM_CREATED`) is declared in
/// `buzz-core::kind` but **no code path anywhere in this repo ever emits it** —
/// the relay creates a DM in `handle_dm_open` and then publishes NIP-29 group
/// discovery events, never a 41001. The previous implementation queried
/// `{kinds:[41001], "#p":[me]}`, so it matched nothing and printed `[]` for
/// every account on every relay, while `channels list`, `channels members` and
/// `messages get` all saw the same conversation. An empty list, not an error,
/// is why this read as "no DMs exist" rather than "my predicate is wrong".
///
/// The relay-emitted truth is kind:39000 (NIP-29 group metadata), which
/// `emit_group_discovery_events` tags with `t=<channel_type>` and, for DMs
/// only, one `p` tag per participant. We filter on `t == "dm"`.
///
/// TRUST: kind:39000 is not in `required_scope_for_kind`'s allowlist, so client
/// ingest rejects it with "restricted: unknown event kind" — only the relay can
/// author one. `t=dm` is therefore relay-attested channel_type, not a
/// client-controlled name or membership heuristic. That distinction is the
/// whole point: consumers (the buzz plugin's inbound bridge) skip the
/// @-mention test on the DM path, so whatever marks a conversation as a DM is
/// what authorises unsolicited input into a session.
pub async fn cmd_list_dms(client: &BuzzClient, limit: Option<u32>) -> Result<(), CliError> {
    let my_pk = client.keys().public_key().to_hex();
    let resp = client.query(&dm_list_filter(&my_pk, limit)).await?;
    let events = parse_query_events(&resp)?;
    let dms = dms_from_group_metadata(&events, &my_pk);
    let output = serde_json::to_string(&dms).unwrap_or_default();
    println!("{output}");
    Ok(())
}

/// The relay filter `dms list` sends.
///
/// Kept separate so a regression to a kind nothing emits (see above) is a red
/// unit test rather than a silent empty list on every account.
fn dm_list_filter(my_pk: &str, limit: Option<u32>) -> serde_json::Value {
    serde_json::json!({
        "kinds": [buzz_sdk::kind::KIND_NIP29_GROUP_METADATA],
        "#p": [my_pk],
        "limit": limit.unwrap_or(50).min(200),
    })
}

/// Parse a relay query response into an event array.
///
/// A relay error is an object (`{"error":"network_error", ...}`), not an array.
/// The old code ran it through `unwrap_or_default()` and printed `[]`, which is
/// indistinguishable from "you have no DMs" — the caller cannot tell a dead
/// relay from an empty inbox. Fail loudly instead.
fn parse_query_events(resp: &str) -> Result<Vec<serde_json::Value>, CliError> {
    match serde_json::from_str::<serde_json::Value>(resp) {
        Ok(serde_json::Value::Array(events)) => Ok(events),
        Ok(serde_json::Value::Object(obj)) => {
            let msg = obj
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unexpected object response");
            Err(CliError::Other(format!("relay query failed: {msg}")))
        }
        Ok(_) => Err(CliError::Other(
            "relay query returned a non-array response".into(),
        )),
        Err(e) => Err(CliError::Other(format!(
            "relay query returned unparseable JSON: {e}"
        ))),
    }
}

/// Project relay-signed kind:39000 group metadata events onto the DM list shape.
///
/// Keeps an event only when it is a DM (`t=dm`), carries a non-empty `d` tag
/// (the channel uuid callers pass to `messages get/send --channel`), and lists
/// us among its participants. The `#p` filter already asks the relay for that
/// last condition; re-checking locally means a relay that ignores or widens the
/// filter cannot widen a consumer's poll set.
fn dms_from_group_metadata(events: &[serde_json::Value], my_pk: &str) -> Vec<serde_json::Value> {
    let mut out: Vec<(String, Vec<String>, u64)> = Vec::new();
    for e in events {
        let tags: Vec<&Vec<serde_json::Value>> = e
            .get("tags")
            .and_then(|t| t.as_array())
            .map(|tags| tags.iter().filter_map(|t| t.as_array()).collect())
            .unwrap_or_default();

        // Take the first tag with this name that actually carries a value: a
        // valueless `["t"]` earlier in the list must not mask a later
        // `["t","dm"]`, or a malformed tag would silently hide a real DM.
        let tag_value = |name: &str| -> Option<&str> {
            tags.iter()
                .filter(|a| a.first().and_then(|v| v.as_str()) == Some(name))
                .find_map(|a| a.get(1).and_then(|v| v.as_str()))
        };

        if tag_value("t") != Some("dm") {
            continue;
        }
        let dm_id = extract_d_tag(e);
        if dm_id.is_empty() {
            continue;
        }
        let participants: Vec<String> = tags
            .iter()
            .filter(|a| a.first().and_then(|v| v.as_str()) == Some("p"))
            .filter_map(|a| a.get(1).and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        if !participants.iter().any(|p| p == my_pk) {
            continue;
        }
        let created_at = e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);

        // kind:39000 is addressable (replaceable per `d` tag), but a relay may
        // still serve more than one revision. Keep the newest per dm_id.
        match out.iter_mut().find(|(id, _, _)| id == &dm_id) {
            Some(slot) if slot.2 < created_at => *slot = (dm_id, participants, created_at),
            Some(_) => {}
            None => out.push((dm_id, participants, created_at)),
        }
    }
    out.sort_by_key(|(_, _, created_at)| std::cmp::Reverse(*created_at));
    out.into_iter()
        .map(|(dm_id, participants, created_at)| {
            serde_json::json!({
                "dm_id": dm_id,
                "participants": participants,
                "created_at": created_at,
            })
        })
        .collect()
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
    use super::{dm_list_filter, dms_from_group_metadata, parse_query_events};
    use serde_json::json;

    const ME: &str = "aa11";
    const THEM: &str = "bb22";

    fn meta(d: &str, t: &str, ps: &[&str], created_at: u64) -> serde_json::Value {
        let mut tags = vec![json!(["d", d]), json!(["name", "DM"]), json!(["t", t])];
        for p in ps {
            tags.push(json!(["p", p]));
        }
        json!({"kind": 39000, "created_at": created_at, "tags": tags})
    }

    #[test]
    fn lists_a_dm_we_participate_in() {
        let events = vec![meta("chan-1", "dm", &[ME, THEM], 100)];
        let dms = dms_from_group_metadata(&events, ME);
        assert_eq!(dms.len(), 1, "expected the DM, got {dms:?}");
        assert_eq!(dms[0]["dm_id"], "chan-1");
        assert_eq!(dms[0]["participants"], json!([ME, THEM]));
        assert_eq!(dms[0]["created_at"], 100);
    }

    #[test]
    fn drops_non_dm_channel_types() {
        // The whole defect class: a public channel must never be reported as a
        // DM, because consumers skip the @-mention test on the DM path.
        for t in ["stream", "forum", "", "DM"] {
            let events = vec![meta("chan-1", t, &[ME, THEM], 100)];
            assert!(
                dms_from_group_metadata(&events, ME).is_empty(),
                "channel_type {t:?} was reported as a DM"
            );
        }
    }

    #[test]
    fn drops_events_with_no_t_tag_at_all() {
        let events = vec![json!({
            "kind": 39000,
            "created_at": 100,
            "tags": [["d", "chan-1"], ["p", ME], ["hidden"]],
        })];
        assert!(dms_from_group_metadata(&events, ME).is_empty());
    }

    #[test]
    fn drops_dms_we_are_not_a_participant_of() {
        let events = vec![meta("chan-1", "dm", &[THEM, "cc33"], 100)];
        assert!(
            dms_from_group_metadata(&events, ME).is_empty(),
            "a relay that ignores the #p filter must not widen the poll set"
        );
    }

    #[test]
    fn drops_an_empty_or_missing_channel_uuid() {
        let events = vec![
            meta("", "dm", &[ME, THEM], 100),
            json!({"kind": 39000, "created_at": 100, "tags": [["t", "dm"], ["p", ME]]}),
        ];
        assert!(dms_from_group_metadata(&events, ME).is_empty());
    }

    #[test]
    fn keeps_the_newest_revision_per_channel_and_sorts_desc() {
        let events = vec![
            meta("chan-1", "dm", &[ME, THEM], 100),
            meta("chan-1", "dm", &[ME, THEM, "cc33"], 300),
            meta("chan-2", "dm", &[ME, THEM], 200),
        ];
        let dms = dms_from_group_metadata(&events, ME);
        assert_eq!(dms.len(), 2, "duplicate revisions not collapsed: {dms:?}");
        assert_eq!(dms[0]["dm_id"], "chan-1");
        assert_eq!(dms[0]["created_at"], 300);
        assert_eq!(dms[0]["participants"], json!([ME, THEM, "cc33"]));
        assert_eq!(dms[1]["dm_id"], "chan-2");
    }

    #[test]
    fn tolerates_malformed_tags() {
        let events = vec![json!({
            "kind": 39000,
            "created_at": 100,
            "tags": [["d", "chan-1"], "not-an-array", ["t"], ["p"], ["t", "dm"], ["p", ME]],
        })];
        let dms = dms_from_group_metadata(&events, ME);
        assert_eq!(dms.len(), 1);
        assert_eq!(dms[0]["participants"], json!([ME]));
    }

    #[test]
    fn a_relay_error_object_is_an_error_not_an_empty_list() {
        let err = parse_query_events(r#"{"error":"network_error","url":"https://x/query"}"#)
            .expect_err("a relay error must not read as an empty inbox");
        assert!(
            err.to_string().contains("network_error"),
            "error lost the relay's reason: {err}"
        );
        assert!(parse_query_events("nope").is_err());
        assert!(parse_query_events("[]")
            .expect("empty array is valid")
            .is_empty());
    }

    #[test]
    fn the_filter_asks_for_relay_signed_group_metadata_addressed_to_us() {
        let f = dm_list_filter(ME, None);
        assert_eq!(
            f["kinds"],
            json!([39000]),
            "kind:41001 is emitted by no code path — querying it returns [] for every account"
        );
        assert_eq!(f["#p"], json!([ME]), "must be scoped to our own pubkey");
        assert_eq!(f["limit"], 50);
        assert_eq!(dm_list_filter(ME, Some(9))["limit"], 9);
        assert_eq!(dm_list_filter(ME, Some(9999))["limit"], 200, "limit capped");
    }
}
