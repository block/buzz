//! End-to-end test for NIP-PM channel pins against a running relay: the `OK`
//! answer to each command, the relay-signed list over a live subscription and
//! a fresh read, and the list after a delete.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_channel_pins -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{Event, EventBuilder, Filter, Keys, Kind, Tag};
use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(10);

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sign(keys: &Keys, kind: u16, content: &str, tags: &[&[&str]]) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(
            tags.iter()
                .map(|tag| Tag::parse(tag.iter().copied()).expect("tag")),
        )
        .sign_with_keys(keys)
        .expect("sign")
}

/// Send `event` and return the message of its `OK true`.
async fn accept(client: &mut BuzzTestClient, event: Event) -> String {
    let kind = event.kind;
    let ok = client.send_event(event).await.expect("OK");
    assert!(ok.accepted, "kind:{kind} refused: {}", ok.message);
    ok.message
}

fn filter(value: Value) -> Filter {
    serde_json::from_value(value).expect("filter")
}

fn pinned(list: &Event) -> Vec<String> {
    list.tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag[0] == "e")
        .map(|tag| tag[1].clone())
        .collect()
}

/// The next pin list the relay sends on the live subscription.
async fn next_list(reader: &mut BuzzTestClient, relay: &str) -> Vec<String> {
    loop {
        if let RelayMessage::Event { event, .. } = reader.recv_event(WAIT).await.expect("live list")
        {
            assert_eq!(event.kind.as_u16(), 30623);
            assert_eq!(event.pubkey.to_hex(), relay, "the relay signs the list");
            return pinned(&event);
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_channel_pins_wire() {
    let url = relay_url();
    let http = url
        .replace("wss://", "https://")
        .replace("ws://", "http://");
    let info: Value = reqwest::Client::new()
        .get(&http)
        .header(reqwest::header::ACCEPT, "application/nostr+json")
        .send()
        .await
        .expect("NIP-11")
        .json()
        .await
        .expect("NIP-11 JSON");
    let relay = info["self"].as_str().expect("relay pubkey").to_owned();

    let (owner, member) = (Keys::generate(), Keys::generate());
    let mut owner_ws = BuzzTestClient::connect(&url, &owner).await.expect("owner");
    let channel = uuid::Uuid::new_v4().to_string();
    let create = sign(
        &owner,
        9007,
        "",
        &[
            &["h", &channel],
            &["name", &format!("pins-{channel}")],
            &["channel_type", "stream"],
            &["visibility", "open"],
        ],
    );
    accept(&mut owner_ws, create).await;
    let mut messages = Vec::new();
    for content in ["first", "second"] {
        let message = sign(&owner, 9, content, &[&["h", &channel]]);
        messages.push(message.id.to_hex());
        accept(&mut owner_ws, message).await;
    }
    let (first, second) = (messages[0].clone(), messages[1].clone());

    let mut reader = BuzzTestClient::connect(&url, &member)
        .await
        .expect("member");
    reader
        .subscribe(
            "live",
            vec![filter(json!({"kinds": [30623], "#h": [channel]}))],
        )
        .await
        .expect("subscribe");
    let history = reader.collect_until_eose("live", WAIT).await.expect("EOSE");
    assert!(history.is_empty(), "no list before the first pin");

    let pin = sign(&owner, 40010, "", &[&["h", &channel], &["e", &first]]);
    assert_eq!(accept(&mut owner_ws, pin.clone()).await, "");
    assert_eq!(next_list(&mut reader, &relay).await, vec![first.clone()]);
    assert_eq!(accept(&mut owner_ws, pin).await, "duplicate:");

    // Any member who may post can pin.
    let mut member_ws = BuzzTestClient::connect(&url, &member)
        .await
        .expect("member");
    let pin = sign(&member, 40010, "", &[&["h", &channel], &["e", &second]]);
    assert_eq!(accept(&mut member_ws, pin).await, "");
    assert_eq!(
        next_list(&mut reader, &relay).await,
        [second.clone(), first.clone()]
    );

    let mut fresh = BuzzTestClient::connect(&url, &member)
        .await
        .expect("member");
    fresh
        .subscribe(
            "list",
            vec![filter(json!({"kinds": [30623], "#h": [channel]}))],
        )
        .await
        .expect("subscribe");
    let lists = fresh.collect_until_eose("list", WAIT).await.expect("EOSE");
    assert_eq!(
        lists.iter().map(pinned).collect::<Vec<_>>(),
        [vec![second.clone(), first.clone()]]
    );
    fresh
        .subscribe(
            "notices",
            vec![filter(json!({"kinds": [40099], "#h": [channel]}))],
        )
        .await
        .expect("subscribe");
    let notices = fresh
        .collect_until_eose("notices", WAIT)
        .await
        .expect("EOSE");
    let mut announced: Vec<(String, String)> = notices
        .iter()
        .map(|notice| serde_json::from_str::<Value>(&notice.content).expect("notice"))
        .filter(|notice| notice["type"] == "message_pinned")
        .map(|notice| {
            let field = |name: &str| notice[name].as_str().expect(name).to_owned();
            (field("actor"), field("target_event_id"))
        })
        .collect();
    announced.sort();
    let mut expected = vec![
        (owner.public_key().to_hex(), first.clone()),
        (member.public_key().to_hex(), second.clone()),
    ];
    expected.sort();
    assert_eq!(announced, expected);

    // Deleting a pinned message takes it out of the list.
    accept(&mut owner_ws, sign(&owner, 5, "", &[&["e", &second]])).await;
    assert_eq!(next_list(&mut reader, &relay).await, vec![first.clone()]);

    let unpin = sign(&member, 40011, "", &[&["h", &channel], &["e", &first]]);
    assert_eq!(accept(&mut member_ws, unpin).await, "");
    assert!(next_list(&mut reader, &relay).await.is_empty());

    let foreign = sign(&owner, 40010, "", &[&["h", &channel], &["e", &second]]);
    let refused = owner_ws.send_event(foreign).await.expect("OK");
    assert!(!refused.accepted);
    assert_eq!(
        refused.message,
        "invalid: target is not a message in this channel"
    );
}
