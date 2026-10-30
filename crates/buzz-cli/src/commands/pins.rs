//! `buzz pins` — pinned channel messages (kind:40004).
//!
//! A pin is its own event; unpinning deletes that event, never the message. Your
//! own pin goes with a NIP-09 deletion (kind:5), which leaves no trace in the
//! channel; anyone else's goes with a kind:9005, which needs channel admin
//! rights and posts a "removed a message" row. `add` and `rm` are idempotent: a
//! message that is already pinned, or not pinned, is reported as `unchanged`
//! and nothing is published.

use std::collections::HashMap;

use buzz_core::kind::KIND_STREAM_MESSAGE_PINNED;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{normalize_write_response, parse_write_response};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::{parse_event_id, parse_uuid, validate_hex64};
use crate::PinsCmd;

/// Bound on the pins read per channel; reaching it is an error, not a truncation.
const MAX_PINS: u32 = 1_000;
const MESSAGE_BATCH: usize = 50;

fn parse_target(event: &str) -> Result<nostr::EventId, CliError> {
    validate_hex64(event)?;
    parse_event_id(event)
}

fn tag_values<'a>(event: &'a Value, name: &'a str) -> impl Iterator<Item = &'a str> {
    event
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(move |tag| tag.get(0).and_then(Value::as_str) == Some(name))
        .filter_map(|tag| tag.get(1).and_then(Value::as_str))
}

/// The pinned message id of a well-formed pin event, if `event` is one for `channel`.
fn pinned_message(event: &Value, channel: &str) -> Option<String> {
    if event.get("kind").and_then(Value::as_u64) != Some(u64::from(KIND_STREAM_MESSAGE_PINNED)) {
        return None;
    }
    if !tag_values(event, "h").any(|h| h == channel) {
        return None;
    }
    let mut targets = tag_values(event, "e");
    let (Some(target), None) = (targets.next(), targets.next()) else {
        return None;
    };
    (target.len() == 64
        && target
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
    .then(|| target.to_string())
}

/// Live pin events of `channel` (optionally of one message), oldest first.
async fn fetch_pins(
    client: &BuzzClient,
    channel: &str,
    message: Option<&str>,
) -> Result<Vec<(String, Value)>, CliError> {
    let mut filter = json!({"kinds": [KIND_STREAM_MESSAGE_PINNED], "#h": [channel]});
    if let Some(message) = message {
        filter["#e"] = json!([message]);
    }
    let mut pins: Vec<(String, Value)> = client
        .query_all_bounded(filter, MAX_PINS)
        .await?
        .into_iter()
        .filter_map(|event| Some((pinned_message(&event, channel)?, event)))
        .filter(|(target, _)| message.is_none_or(|m| m == target))
        .collect();
    pins.sort_by_key(|(_, event)| {
        (
            event.get("created_at").and_then(Value::as_u64).unwrap_or(0),
            event.get("id").and_then(Value::as_str).map(str::to_string),
        )
    });
    Ok(pins)
}

async fn fetch_messages(
    client: &BuzzClient,
    ids: &[String],
) -> Result<HashMap<String, Value>, CliError> {
    let mut messages = HashMap::new();
    for chunk in ids.chunks(MESSAGE_BATCH) {
        let raw = client
            .query(&json!({"ids": chunk, "limit": MESSAGE_BATCH}))
            .await?;
        let events: Vec<Value> = serde_json::from_str(&raw)
            .map_err(|e| CliError::Other(format!("failed to parse message query: {e}")))?;
        for event in events {
            if let Some(id) = event.get("id").and_then(Value::as_str) {
                let message = json!({
                    "id": event["id"],
                    "pubkey": event["pubkey"],
                    "kind": event["kind"],
                    "created_at": event["created_at"],
                    "content": event["content"],
                });
                messages.insert(id.to_string(), message);
            }
        }
    }
    Ok(messages)
}

async fn list(client: &BuzzClient, channel: &str) -> Result<Value, CliError> {
    let mut pins = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (target, event) in fetch_pins(client, channel, None).await? {
        if seen.insert(target.clone()) {
            pins.push((target, event));
        }
    }
    pins.reverse();
    let ids: Vec<String> = pins.iter().map(|(target, _)| target.clone()).collect();
    let mut messages = fetch_messages(client, &ids).await?;
    let pins: Vec<Value> = pins
        .into_iter()
        .map(|(target, event)| {
            json!({
                "event_id": target,
                "pin_event_id": event["id"],
                "pinned_by": event["pubkey"],
                "pinned_at": event["created_at"],
                "message": messages.remove(&target),
            })
        })
        .collect();
    Ok(json!({"channel_id": channel, "pins": pins}))
}

async fn add(
    client: &BuzzClient,
    channel: Uuid,
    message: nostr::EventId,
) -> Result<Value, CliError> {
    let (channel_hex, message_hex) = (channel.hyphenated().to_string(), message.to_hex());
    let mut response = match fetch_pins(client, &channel_hex, Some(&message_hex))
        .await?
        .first()
    {
        Some((_, existing)) => json!({
            "event_id": existing["id"],
            "accepted": true,
            "message": "unchanged",
        }),
        None => {
            let builder = buzz_sdk::build_pin_message(channel, message)
                .map_err(|e| CliError::Other(format!("build_pin_message failed: {e}")))?;
            let raw = client.submit_event(client.sign_event(builder)?).await?;
            let normalized = parse_write_response(
                &raw,
                "this message was unpinned in the same second; run the command again",
            )?;
            serde_json::from_str(&normalized)
                .map_err(|e| CliError::Other(format!("relay response is not JSON: {e}")))?
        }
    };
    response["channel_id"] = json!(channel_hex);
    response["message_id"] = json!(message_hex);
    Ok(response)
}

async fn remove(
    client: &BuzzClient,
    channel: Uuid,
    message: nostr::EventId,
) -> Result<Value, CliError> {
    let (channel_hex, message_hex) = (channel.hyphenated().to_string(), message.to_hex());
    let me = client.keys().public_key().to_hex();
    let mut removed = Vec::new();
    for (_, pin) in fetch_pins(client, &channel_hex, Some(&message_hex)).await? {
        let pin_id = parse_event_id(pin["id"].as_str().unwrap_or_default())?;
        let builder = if pin["pubkey"].as_str() == Some(&me) {
            buzz_sdk::build_delete_compat(channel, pin_id)
        } else {
            buzz_sdk::build_delete_message(channel, pin_id)
        }
        .map_err(|e| CliError::Other(format!("failed to build unpin event: {e}")))?;
        let raw = client.submit_event(client.sign_event(builder)?).await?;
        let response: Value = serde_json::from_str(&normalize_write_response(&raw))
            .map_err(|e| CliError::Other(format!("relay response is not JSON: {e}")))?;
        if response["accepted"] != json!(true) {
            return Err(CliError::Other(format!(
                "relay rejected unpin of {}: {}",
                pin_id.to_hex(),
                response["message"].as_str().unwrap_or_default()
            )));
        }
        removed.push(pin_id.to_hex());
    }
    let message_text = if removed.is_empty() { "unchanged" } else { "" };
    Ok(json!({
        "accepted": true,
        "message": message_text,
        "channel_id": channel_hex,
        "message_id": message_hex,
        "removed": removed,
    }))
}

async fn run(cmd: PinsCmd, client: &BuzzClient) -> Result<Value, CliError> {
    match cmd {
        PinsCmd::List { channel } => {
            list(client, &parse_uuid(&channel)?.hyphenated().to_string()).await
        }
        PinsCmd::Add { channel, event } => {
            add(client, parse_uuid(&channel)?, parse_target(&event)?).await
        }
        PinsCmd::Rm { channel, event } => {
            remove(client, parse_uuid(&channel)?, parse_target(&event)?).await
        }
    }
}

pub async fn dispatch(cmd: PinsCmd, client: &BuzzClient) -> Result<(), CliError> {
    println!("{}", run(cmd, client).await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Bytes, extract::State, http::StatusCode, routing::post, Json, Router};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const CHANNEL: &str = "6f1c7c1e-1d5b-4c55-9d43-0c1f0b6a2f10";
    const OTHER_CHANNEL: &str = "0a0a0a0a-1d5b-4c55-9d43-0c1f0b6a2f10";

    #[derive(Default)]
    struct Relay {
        events: Vec<Value>,
        published: Vec<Value>,
        refuse: bool,
        duplicate: bool,
    }
    type Shared = Arc<Mutex<Relay>>;

    fn matches(event: &Value, filter: &Value) -> bool {
        let tag_match = |name: &str, key: &str| {
            filter.get(key).is_none_or(|wanted| {
                let wanted = wanted.as_array().unwrap();
                tag_values(event, name).any(|v| wanted.iter().any(|w| w == v))
            })
        };
        filter
            .get("kinds")
            .is_none_or(|kinds| kinds.as_array().unwrap().contains(&event["kind"]))
            && filter
                .get("ids")
                .is_none_or(|ids| ids.as_array().unwrap().contains(&event["id"]))
            && tag_match("h", "#h")
            && tag_match("e", "#e")
    }

    async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route(
                "/query",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                    let relay = relay.lock().unwrap();
                    let found: Vec<_> = relay
                        .events
                        .iter()
                        .filter(|event| filters.iter().any(|f| matches(event, f)))
                        .cloned()
                        .collect();
                    Json(json!(found))
                }),
            )
            .route(
                "/events",
                post(|State(relay): State<Shared>, body: Bytes| async move {
                    let event: Value = serde_json::from_slice(&body).unwrap();
                    let mut relay = relay.lock().unwrap();
                    if relay.refuse {
                        return (
                            StatusCode::FORBIDDEN,
                            Json(json!({"error": "restricted: only members can post in this channel"})),
                        );
                    }
                    if relay.duplicate {
                        return (
                            StatusCode::OK,
                            Json(json!({"event_id": event["id"], "accepted": true, "message": "duplicate:"})),
                        );
                    }
                    relay.published.push(event.clone());
                    if event["kind"] == json!(9005) || event["kind"] == json!(5) {
                        let target = event["tags"][1][1].clone();
                        relay.events.retain(|stored| stored["id"] != target);
                    } else {
                        relay.events.push(event.clone());
                    }
                    (
                        StatusCode::OK,
                        Json(json!({"event_id": event["id"], "accepted": true, "message": ""})),
                    )
                }),
            )
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    async fn setup() -> (BuzzClient, Shared) {
        let relay = Shared::default();
        let url = fake_relay(relay.clone()).await;
        let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();
        (client, relay)
    }

    fn seed_as(
        relay: &Shared,
        keys: &Keys,
        kind: u16,
        created_at: u64,
        content: &str,
        tags: &[Vec<String>],
    ) -> Value {
        let event = EventBuilder::new(Kind::Custom(kind), content)
            .tags(tags.iter().map(|t| Tag::parse(t.iter()).unwrap()))
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap();
        let event = serde_json::to_value(event).unwrap();
        relay.lock().unwrap().events.push(event.clone());
        event
    }

    fn seed(
        relay: &Shared,
        kind: u16,
        created_at: u64,
        content: &str,
        tags: &[Vec<String>],
    ) -> Value {
        seed_as(relay, &Keys::generate(), kind, created_at, content, tags)
    }

    fn tags(pairs: &[(&str, &str)]) -> Vec<Vec<String>> {
        pairs
            .iter()
            .map(|(k, v)| vec![k.to_string(), v.to_string()])
            .collect()
    }

    fn message(relay: &Shared, channel: &str, content: &str) -> String {
        let event = seed(relay, 9, 1_000, content, &tags(&[("h", channel)]));
        event["id"].as_str().unwrap().to_string()
    }

    fn seed_pin(relay: &Shared, channel: &str, target: &str, created_at: u64) -> String {
        seed_pin_as(relay, &Keys::generate(), channel, target, created_at)
    }

    fn seed_pin_as(
        relay: &Shared,
        keys: &Keys,
        channel: &str,
        target: &str,
        created_at: u64,
    ) -> String {
        let event = seed_as(
            relay,
            keys,
            40004,
            created_at,
            "",
            &tags(&[("h", channel), ("e", target)]),
        );
        event["id"].as_str().unwrap().to_string()
    }

    fn published(relay: &Shared) -> Vec<Value> {
        relay.lock().unwrap().published.clone()
    }

    fn add_cmd(event: &str) -> PinsCmd {
        PinsCmd::Add {
            channel: CHANNEL.into(),
            event: event.into(),
        }
    }

    fn rm_cmd(event: &str) -> PinsCmd {
        PinsCmd::Rm {
            channel: CHANNEL.into(),
            event: event.into(),
        }
    }

    #[tokio::test]
    async fn add_publishes_one_pin_and_a_repeat_is_unchanged() {
        let (client, relay) = setup().await;
        let target = message(&relay, CHANNEL, "ship it");

        let out = run(add_cmd(&target), &client).await.unwrap();
        let sent = published(&relay);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["kind"], json!(40004));
        assert_eq!(sent[0]["tags"], json!([["h", CHANNEL], ["e", target]]));
        assert_eq!(sent[0]["content"], json!(""));
        assert_eq!(
            out,
            json!({
                "event_id": sent[0]["id"],
                "accepted": true,
                "message": "",
                "channel_id": CHANNEL,
                "message_id": target,
            })
        );

        let again = run(add_cmd(&target), &client).await.unwrap();
        assert_eq!(published(&relay).len(), 1, "a repeat must not publish");
        assert_eq!(again["message"], json!("unchanged"));
        assert_eq!(again["event_id"], sent[0]["id"]);
    }

    #[tokio::test]
    async fn add_normalizes_case_and_rejects_bad_input_before_the_network() {
        let (client, relay) = setup().await;
        let target = message(&relay, CHANNEL, "x");
        run(add_cmd(&target.to_uppercase()), &client).await.unwrap();
        assert_eq!(published(&relay)[0]["tags"][1], json!(["e", target]));

        for cmd in [
            PinsCmd::Add {
                channel: "general".into(),
                event: target.clone(),
            },
            add_cmd("abc"),
            PinsCmd::List {
                channel: "nope".into(),
            },
        ] {
            assert!(matches!(run(cmd, &client).await, Err(CliError::Usage(_))));
        }
        assert_eq!(published(&relay).len(), 1);
    }

    #[tokio::test]
    async fn a_refused_pin_is_a_relay_error() {
        let (client, relay) = setup().await;
        let target = message(&relay, CHANNEL, "x");
        relay.lock().unwrap().refuse = true;
        let error = run(add_cmd(&target), &client).await.unwrap_err();
        assert!(
            matches!(&error, CliError::Relay { status: 403, body } if body.starts_with("restricted:")),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn list_returns_newest_pin_first_with_the_message() {
        let (client, relay) = setup().await;
        let old = message(&relay, CHANNEL, "first");
        let new = message(&relay, CHANNEL, "second");
        let gone = "c".repeat(64);
        let old_pin = seed_pin(&relay, CHANNEL, &old, 2_000);
        let new_pin = seed_pin(&relay, CHANNEL, &new, 3_000);
        seed_pin(&relay, CHANNEL, &gone, 4_000);
        seed_pin(&relay, CHANNEL, &new, 5_000);
        seed_pin(&relay, OTHER_CHANNEL, &old, 6_000);
        seed(&relay, 40004, 7_000, "", &tags(&[("h", CHANNEL)]));
        seed(
            &relay,
            40004,
            7_000,
            "",
            &tags(&[("h", CHANNEL), ("e", &old), ("e", &new)]),
        );

        let out = run(
            PinsCmd::List {
                channel: CHANNEL.into(),
            },
            &client,
        )
        .await
        .unwrap();
        let pins = out["pins"].as_array().unwrap();
        let ids: Vec<_> = pins
            .iter()
            .map(|p| p["event_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [gone.as_str(), new.as_str(), old.as_str()]);
        assert_eq!(pins[0]["message"], Value::Null);
        assert_eq!(pins[1]["pin_event_id"], json!(new_pin));
        assert_eq!(pins[1]["pinned_at"], json!(3_000));
        assert_eq!(pins[1]["message"]["content"], json!("second"));
        assert_eq!(pins[2]["pin_event_id"], json!(old_pin));
        assert_eq!(out["channel_id"], json!(CHANNEL));
    }

    #[tokio::test]
    async fn list_of_a_channel_without_pins_is_empty() {
        let (client, _relay) = setup().await;
        let out = run(
            PinsCmd::List {
                channel: CHANNEL.into(),
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(out, json!({"channel_id": CHANNEL, "pins": []}));
    }

    #[tokio::test]
    async fn rm_deletes_every_pin_of_the_message_and_a_repeat_is_unchanged() {
        let (client, relay) = setup().await;
        let target = message(&relay, CHANNEL, "x");
        let kept = message(&relay, CHANNEL, "y");
        let theirs = seed_pin(&relay, CHANNEL, &target, 2_000);
        let mine = seed_pin_as(&relay, client.keys(), CHANNEL, &target, 3_000);
        let kept_pin = seed_pin(&relay, CHANNEL, &kept, 4_000);

        let out = run(rm_cmd(&target), &client).await.unwrap();
        assert_eq!(out["removed"], json!([theirs, mine]));
        assert_eq!(out["message"], json!(""));
        let sent = published(&relay);
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0]["kind"], json!(9005), "another member's pin");
        assert_eq!(sent[0]["tags"], json!([["h", CHANNEL], ["e", theirs]]));
        assert_eq!(
            sent[1]["kind"],
            json!(5),
            "your own pin leaves no tombstone"
        );
        assert_eq!(sent[1]["tags"], json!([["h", CHANNEL], ["e", mine]]));

        let again = run(rm_cmd(&target), &client).await.unwrap();
        assert_eq!(again["message"], json!("unchanged"));
        assert_eq!(again["removed"], json!([]));
        assert_eq!(published(&relay).len(), 2);

        let list = run(
            PinsCmd::List {
                channel: CHANNEL.into(),
            },
            &client,
        )
        .await
        .unwrap();
        assert_eq!(list["pins"][0]["pin_event_id"], json!(kept_pin));
        assert_eq!(list["pins"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_pin_the_relay_already_holds_is_a_conflict_not_a_success() {
        let (client, relay) = setup().await;
        let target = message(&relay, CHANNEL, "x");
        relay.lock().unwrap().duplicate = true;
        let error = run(add_cmd(&target), &client).await.unwrap_err();
        assert!(matches!(error, CliError::Conflict(_)), "{error:?}");
    }
}
