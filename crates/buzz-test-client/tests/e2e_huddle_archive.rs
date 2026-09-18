//! End-to-end tests for huddle auto-archive convergence.
//!
//! These tests require a running relay instance. By default they are marked
//! `#[ignore]` so that `cargo test` does not fail in CI when the relay is not
//! available.
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! cargo test --test e2e_huddle_archive -- --ignored
//! ```
//!
//! Override the relay URL with the `RELAY_URL` environment variable:
//!
//! ```text
//! RELAY_URL=ws://relay.example.com cargo test --test e2e_huddle_archive -- --ignored
//! ```

use std::time::Duration;

use buzz_test_client::BuzzTestClient;
use buzz_ws_client::build_auth_event;
use futures_util::{SinkExt, StreamExt};
use nostr::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// NIP-29 group metadata — what clients build their channel list from.
const KIND_NIP29_GROUP_METADATA: u16 = 39000;
/// Huddle lifecycle start — links an ephemeral channel to its parent.
const KIND_HUDDLE_STARTED: u16 = 48100;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn sub_id(name: &str) -> String {
    format!("e2e-{name}-{}", uuid::Uuid::new_v4())
}

/// Create a channel owned by `keys`. Passing `ttl_seconds` makes it ephemeral,
/// which is how a huddle's backing channel is created.
async fn create_channel(
    client: &mut BuzzTestClient,
    keys: &Keys,
    name: &str,
    ttl_seconds: Option<u32>,
) -> uuid::Uuid {
    let channel_id = uuid::Uuid::new_v4();
    let mut tags = vec![
        Tag::parse(["h", &channel_id.to_string()]).unwrap(),
        Tag::parse(["name", &format!("{name}-{channel_id}")]).unwrap(),
        Tag::parse(["channel_type", "stream"]).unwrap(),
        Tag::parse(["visibility", "open"]).unwrap(),
    ];
    if let Some(ttl) = ttl_seconds {
        tags.push(Tag::parse(["ttl", &ttl.to_string()]).unwrap());
    }
    let event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap();
    let ok = client.send_event(event).await.expect("send create-channel");
    assert!(ok.accepted, "channel should be created: {}", ok.message);
    channel_id
}

/// Fetch the newest kind:39000 for a channel and report whether it carries
/// `["archived", "true"]`.
///
/// Returns `None` when no metadata event exists at all.
async fn metadata_says_archived(
    client: &mut BuzzTestClient,
    channel_id: &uuid::Uuid,
    label: &str,
) -> Option<bool> {
    let sid = sub_id(label);
    let filter = Filter::new()
        .kind(Kind::Custom(KIND_NIP29_GROUP_METADATA))
        .custom_tag(
            SingleLetterTag::lowercase(Alphabet::D),
            channel_id.to_string(),
        );
    client
        .subscribe(&sid, vec![filter])
        .await
        .expect("subscribe");
    let events = client
        .collect_until_eose(&sid, Duration::from_secs(5))
        .await
        .expect("collect");

    // kind:39000 is replaceable, so at most one live event should come back;
    // take the newest defensively in case the relay returns history.
    let newest = events.iter().max_by_key(|e| e.created_at)?;
    Some(
        newest
            .tags
            .iter()
            .any(|t| t.as_slice().first().map(String::as_str) == Some("archived")),
    )
}

/// Publish the creator-signed kind:48100 event that links an ephemeral huddle
/// channel to its parent. The audio door refuses to admit a peer to an
/// ephemeral channel without it ("ephemeral channel is not linked to claimed
/// parent"), so this is what a real client emits when starting a huddle.
async fn link_huddle_to_parent(
    client: &mut BuzzTestClient,
    keys: &Keys,
    parent_id: &uuid::Uuid,
    huddle_id: &uuid::Uuid,
) {
    let content = serde_json::json!({ "ephemeral_channel_id": huddle_id.to_string() }).to_string();
    let event = EventBuilder::new(Kind::Custom(KIND_HUDDLE_STARTED), content)
        .tags(vec![Tag::parse(["h", &parent_id.to_string()]).unwrap()])
        .sign_with_keys(keys)
        .unwrap();
    let ok = client.send_event(event).await.expect("send huddle-started");
    assert!(
        ok.accepted,
        "huddle-started should be accepted: {}",
        ok.message
    );
}

/// Connect one audio peer to a huddle, wait until the relay confirms the join,
/// then drop the socket. Dropping the last peer is what triggers the relay's
/// auto-end path.
async fn join_and_drop_audio_peer(huddle_id: &uuid::Uuid, parent_id: &uuid::Uuid, keys: &Keys) {
    let base = relay_url();
    let audio_url = format!("{base}/huddle/{huddle_id}/audio");
    let (mut ws, _) = connect_async(&audio_url)
        .await
        .expect("connect huddle audio websocket");

    // 1. Relay opens with a NIP-42 challenge.
    let challenge = loop {
        let msg = ws
            .next()
            .await
            .expect("audio socket closed before challenge")
            .expect("audio socket error");
        if let Message::Text(text) = msg {
            let v: serde_json::Value = serde_json::from_str(&text).expect("challenge json");
            if v["type"] == "challenge" {
                break v["challenge"]
                    .as_str()
                    .expect("challenge string")
                    .to_string();
            }
        }
    };

    // 2. Answer it. The relay expects the auth event's relay tag to name the
    //    host it was reached on.
    let auth_event = build_auth_event(&challenge, &base, keys, None).expect("build auth event");
    let auth_msg = serde_json::json!({
        "type": "auth",
        "event": auth_event,
        "parent_channel_id": parent_id.to_string(),
    })
    .to_string();
    ws.send(Message::Text(auth_msg.into()))
        .await
        .expect("send auth");

    // 3. Wait for the join to be confirmed before leaving, otherwise the peer
    //    was never in the room and the auto-end path is not exercised.
    let joined = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(msg)) = ws.next().await {
            if let Message::Text(text) = msg {
                let v: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match v["type"].as_str() {
                    Some("joined") => return true,
                    Some("error") => panic!("huddle join rejected: {text}"),
                    _ => continue,
                }
            }
        }
        false
    })
    .await
    .expect("timed out waiting for huddle join");
    assert!(joined, "audio socket closed before the join was confirmed");

    // 4. Leave. This is the last peer, so the relay auto-ends the huddle.
    let _ = ws.close(None).await;
    drop(ws);
}

/// When the last audio peer leaves, the relay auto-archives the huddle's
/// ephemeral channel. It must also republish kind:39000 so clients learn about
/// it — the database and the event projection have to converge.
///
/// Regression test for issue #4879 — before the fix the audio auto-end path
/// wrote `channels.archived_at` and emitted only kind:48103, never calling
/// `emit_group_discovery_events`. Clients build their channel list from
/// kind:39000, so every client (including a fresh install) kept showing the
/// huddle channel forever. Nothing repaired it: the ephemeral reaper's UPDATE
/// filters `archived_at IS NULL`, so an already-archived row is never picked
/// up, and kind:39000 is replaceable, so the stale event is what future clients
/// sync.
#[tokio::test]
#[ignore]
async fn test_huddle_auto_archive_republishes_group_metadata() {
    let url = relay_url();
    let keys = Keys::generate();
    let mut client = BuzzTestClient::connect(&url, &keys).await.expect("connect");

    let parent_id = create_channel(&mut client, &keys, "huddle-parent", None).await;
    let huddle_id = create_channel(&mut client, &keys, "huddle", Some(3600)).await;
    link_huddle_to_parent(&mut client, &keys, &parent_id, &huddle_id).await;

    // Sanity check: a live huddle channel must not advertise itself as archived.
    let before = metadata_says_archived(&mut client, &huddle_id, "huddle-pre").await;
    assert_eq!(
        before,
        Some(false),
        "huddle channel metadata should exist and not be archived before the huddle ends"
    );

    join_and_drop_audio_peer(&huddle_id, &parent_id, &keys).await;

    // The auto-end path runs during connection teardown; poll briefly rather
    // than assuming it has already completed.
    let mut archived = Some(false);
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        archived = metadata_says_archived(&mut client, &huddle_id, "huddle-post").await;
        if archived == Some(true) {
            break;
        }
    }

    assert_eq!(
        archived,
        Some(true),
        "after the last audio peer leaves, the newest kind:39000 for the huddle channel \
         must carry [\"archived\", \"true\"] — otherwise clients keep the channel forever"
    );

    client.disconnect().await.expect("disconnect");
}
