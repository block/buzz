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
    let dms = list_dms(client, limit.unwrap_or(50).min(200)).await?;
    println!("{}", serde_json::Value::Array(dms));
    Ok(())
}

async fn list_dms(client: &BuzzClient, limit: u32) -> Result<Vec<serde_json::Value>, CliError> {
    let my_pk = client.public_key().to_hex();
    let snapshots = client
        .query_paginated(
            serde_json::json!({
                "kinds": [buzz_sdk::kind::KIND_DM_VISIBILITY],
                "#p": [my_pk],
            }),
            10,
        )
        .await?;
    let hidden: std::collections::HashSet<String> = snapshots
        .iter()
        .max_by_key(|e| e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0))
        .map(|e| tag_values(e, "h").map(str::to_owned).collect())
        .unwrap_or_default();
    let events = client
        .query_paginated_keeping(
            serde_json::json!({
                "kinds": [39000],
                "#t": ["dm"],
                "#p": [my_pk],
            }),
            limit,
            |e| {
                let dm_id = extract_d_tag(e);
                !dm_id.is_empty() && !hidden.contains(&dm_id)
            },
        )
        .await?;
    Ok(events
        .iter()
        .map(|e| {
            let participants: Vec<&str> = tag_values(e, "p").collect();
            serde_json::json!({
                "dm_id": extract_d_tag(e),
                "participants": participants,
                "created_at": e.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0),
            })
        })
        .collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A relay holding `events` (newest first) that pages `/query` by its
    /// composite cursor, logging each filter it answers.
    async fn relay(events: Vec<serde_json::Value>) -> (String, Arc<Mutex<Vec<serde_json::Value>>>) {
        use axum::{routing::post, Json, Router};
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        let app = Router::new().route(
            "/query",
            post(move |Json(filters): Json<Vec<serde_json::Value>>| {
                let events = events.clone();
                let seen = seen.clone();
                async move {
                    let f = filters[0].clone();
                    seen.lock().unwrap().push(f.clone());
                    let kind = f["kinds"][0].as_u64();
                    let limit = f["limit"].as_u64().unwrap_or(500) as usize;
                    let after_cursor = |e: &&serde_json::Value| match f["before_id"].as_str() {
                        None => true,
                        Some(id) => {
                            let at = f["until"].as_u64().unwrap();
                            let t = e["created_at"].as_u64().unwrap();
                            t < at || (t == at && e["id"].as_str().unwrap() > id)
                        }
                    };
                    let page: Vec<_> = events
                        .iter()
                        .filter(|e| e["kind"].as_u64() == kind)
                        .filter(after_cursor)
                        .take(limit)
                        .cloned()
                        .collect();
                    Json(page)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, log)
    }

    fn dm(n: u64) -> serde_json::Value {
        serde_json::json!({
            "id": format!("{n:064x}"),
            "kind": 39000,
            "created_at": 1_000 + n,
            "tags": [["d", format!("dm-{n}")], ["t", "dm"]],
        })
    }

    #[tokio::test]
    async fn hidden_dms_do_not_crowd_out_older_visible_ones() {
        // 200 newer hidden DMs, then one older visible DM.
        let mut events: Vec<_> = (1..=200).rev().map(dm).collect();
        events.push(dm(0));
        let hidden: Vec<_> = (1..=200)
            .map(|n| serde_json::json!(["h", format!("dm-{n}")]))
            .collect();
        events.push(serde_json::json!({
            "id": "f".repeat(64), "kind": 30622, "created_at": 5_000, "tags": hidden,
        }));
        let (base, _) = relay(events).await;
        let client = BuzzClient::new(
            base,
            crate::credentials::Credentials::from_keys(nostr::Keys::generate()),
        )
        .unwrap();

        let dms = list_dms(&client, 200).await.unwrap();

        assert_eq!(dms.len(), 1, "{dms:?}");
        assert_eq!(dms[0]["dm_id"], "dm-0");
    }

    #[tokio::test]
    async fn a_small_limit_still_reads_full_pages_past_hidden_dms() {
        // 150 newer hidden DMs, then one older visible DM, with limit 1.
        let mut events: Vec<_> = (1..=150).rev().map(dm).collect();
        events.push(dm(0));
        let hidden: Vec<_> = (1..=150)
            .map(|n| serde_json::json!(["h", format!("dm-{n}")]))
            .collect();
        events.push(serde_json::json!({
            "id": "f".repeat(64), "kind": 30622, "created_at": 5_000, "tags": hidden,
        }));
        let (base, log) = relay(events).await;

        let dms = list_dms(&client(base), 1).await.unwrap();

        assert_eq!(dms.len(), 1, "{dms:?}");
        assert_eq!(dms[0]["dm_id"], "dm-0");
        let log = log.lock().unwrap();
        let dm_pages: Vec<_> = log.iter().filter(|f| f["kinds"][0] == 39000).collect();
        assert_eq!(dm_pages.len(), 1, "{dm_pages:?}");
        assert_eq!(dm_pages[0]["limit"], crate::client::QUERY_PAGE_SIZE);
    }

    #[tokio::test]
    async fn pages_past_a_hidden_dm_with_the_same_timestamp() {
        // Equal timestamps come back in ascending id order.
        let same = |n: u64| {
            let mut e = dm(n);
            e["created_at"] = serde_json::json!(1_000);
            e
        };
        // A full page of hidden DMs, then a visible one with the same
        // timestamp and a larger id, so the cursor must stay on that second.
        let page = crate::client::QUERY_PAGE_SIZE as u64;
        let mut events: Vec<_> = (1..=page + 1).map(same).collect();
        let hidden: Vec<_> = (1..=page)
            .map(|n| serde_json::json!(["h", format!("dm-{n}")]))
            .collect();
        events.push(serde_json::json!({
            "id": "f".repeat(64), "kind": 30622, "created_at": 5_000, "tags": hidden,
        }));
        let (base, log) = relay(events).await;

        let dms = list_dms(&client(base), 1).await.unwrap();

        assert_eq!(dms.len(), 1, "{dms:?}");
        assert_eq!(dms[0]["dm_id"], format!("dm-{}", page + 1));
        assert!(log
            .lock()
            .unwrap()
            .iter()
            .any(|f| f["before_id"].is_string()));
    }

    /// A relay that answers every DM-metadata query with `page(filter)`.
    async fn fixed_relay(
        page: fn(&serde_json::Value) -> Vec<serde_json::Value>,
    ) -> (String, Arc<Mutex<usize>>) {
        use axum::{routing::post, Json, Router};
        let count = Arc::new(Mutex::new(0));
        let seen = count.clone();
        let app = Router::new().route(
            "/query",
            post(move |Json(filters): Json<Vec<serde_json::Value>>| {
                let seen = seen.clone();
                async move {
                    let f = &filters[0];
                    if f["kinds"][0].as_u64() != Some(39000) {
                        return Json(Vec::new());
                    }
                    *seen.lock().unwrap() += 1;
                    Json(page(f))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, count)
    }

    /// A full page of DMs with no `d` tag, which `list_dms` drops, older
    /// than the filter's cursor.
    fn dropped_page(f: &serde_json::Value) -> Vec<serde_json::Value> {
        let top = f["until"].as_u64().unwrap_or(1_000_000);
        (1..=f["limit"].as_u64().unwrap())
            .map(|n| {
                serde_json::json!({
                    "id": "a".repeat(64), "kind": 39000, "created_at": top - n, "tags": [],
                })
            })
            .collect()
    }

    fn client(base: String) -> BuzzClient {
        BuzzClient::new(
            base,
            crate::credentials::Credentials::from_keys(nostr::Keys::generate()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn a_relay_repeating_the_same_page_fails() {
        let (base, count) = fixed_relay(|f| {
            let mut f = f.clone();
            f["until"] = serde_json::Value::Null;
            dropped_page(&f)
        })
        .await;

        let err = list_dms(&client(base), 10).await.unwrap_err();

        assert_eq!(crate::error::exit_code(&err), 2, "{err}");
        assert!(err.to_string().contains("cursor"), "{err}");
        assert_eq!(*count.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn a_relay_that_never_runs_out_fails_at_the_page_cap() {
        let (base, count) = fixed_relay(dropped_page).await;

        let err = list_dms(&client(base), 10).await.unwrap_err();

        assert_eq!(crate::error::exit_code(&err), 2, "{err}");
        assert!(err.to_string().contains("pages"), "{err}");
        assert_eq!(*count.lock().unwrap(), crate::client::MAX_QUERY_PAGES);
    }
}
