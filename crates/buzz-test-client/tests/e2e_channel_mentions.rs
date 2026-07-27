//! End-to-end integration tests for NIP-CM channel-wide mentions (`@channel` / `@here`).
//!
//! These tests cover the relay write path — the accept/reject matrix for the
//! `["notify", "channel"|"here"]` marker tag — and the read path, where an
//! accepted `@channel` event surfaces in the mentions feed of every channel
//! member (and of nobody else).
//!
//! # Running
//!
//! Start the relay, then run:
//!
//! ```text
//! RELAY_URL=ws://localhost:3000 cargo test -p buzz-test-client --test e2e_channel_mentions -- --ignored
//! ```

use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::Value;
use uuid::Uuid;

const KIND_STREAM_MESSAGE: u16 = 9;
const KIND_STREAM_MESSAGE_V2: u16 = 40002;
const KIND_CREATE_GROUP: u16 = 9007;
const KIND_JOIN_REQUEST: u16 = 9021;
const KIND_REPORT: u16 = 1984;
const KIND_PRODUCT_FEEDBACK: u16 = 42000;

fn relay_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

fn relay_http_url() -> String {
    relay_url()
        .replace("wss://", "https://")
        .replace("ws://", "http://")
        .trim_end_matches('/')
        .to_string()
}

/// Submit a signed event over `POST /events` and return the parsed body.
///
/// The bridge answers 4xx for rejections, so the status is folded into the
/// returned tuple instead of asserted here.
async fn post_event(keys: &Keys, event: &nostr::Event) -> (reqwest::StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(event).expect("serialize event"))
        .send()
        .await
        .expect("submit event");
    let status = response.status();
    let text = response.text().await.expect("read event response");
    let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
    (status, body)
}

fn accepted(body: &Value) -> bool {
    body.get("accepted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn rejection_message(status: reqwest::StatusCode, body: &Value) -> String {
    match body {
        Value::String(text) => format!("{status}: {text}"),
        other => format!("{status}: {other}"),
    }
}

async fn create_channel(keys: &Keys, channel_type: &str) -> Uuid {
    let channel_id = Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(KIND_CREATE_GROUP), "")
        .tags(vec![
            Tag::parse(["h", &channel_id.to_string()]).expect("h tag"),
            Tag::parse(["name", &format!("cm-e2e-{channel_id}")]).expect("name tag"),
            Tag::parse(["channel_type", channel_type]).expect("channel_type tag"),
            Tag::parse(["visibility", "open"]).expect("visibility tag"),
        ])
        .sign_with_keys(keys)
        .expect("sign create-group event");
    let (status, body) = post_event(keys, &event).await;
    assert!(
        status.is_success() && accepted(&body),
        "channel creation failed: {}",
        rejection_message(status, &body)
    );
    channel_id
}

async fn join_channel(keys: &Keys, channel_id: Uuid) {
    let event = EventBuilder::new(Kind::Custom(KIND_JOIN_REQUEST), "")
        .tags(vec![
            Tag::parse(["h", &channel_id.to_string()]).expect("h tag")
        ])
        .sign_with_keys(keys)
        .expect("sign join event");
    let (status, body) = post_event(keys, &event).await;
    assert!(
        status.is_success() && accepted(&body),
        "join failed: {}",
        rejection_message(status, &body)
    );
}

fn message(
    keys: &Keys,
    kind: u16,
    channel_id: Uuid,
    content: &str,
    tags: &[&[&str]],
) -> nostr::Event {
    let mut all = vec![Tag::parse(["h", &channel_id.to_string()]).expect("h tag")];
    all.extend(
        tags.iter()
            .map(|t| Tag::parse(t.iter().copied()).expect("tag")),
    );
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(all)
        .sign_with_keys(keys)
        .expect("sign message")
}

/// Query the caller's mentions feed over `POST /query`.
async fn mentions_feed(keys: &Keys) -> Vec<Value> {
    // `POST /query` takes an array of Nostr filters, as the CLI sends them.
    let filters = serde_json::json!([{
        "#p": [keys.public_key().to_hex()],
        "feed_types": ["mentions"],
        "limit": 50
    }]);
    let response = reqwest::Client::new()
        .post(format!("{}/query", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&filters).expect("serialize filters"))
        .send()
        .await
        .expect("query mentions feed");
    assert!(
        response.status().is_success(),
        "mentions feed query failed: {}",
        response.status()
    );
    response.json().await.expect("parse mentions feed")
}

fn feed_contains(feed: &[Value], event_id: &str) -> bool {
    feed.iter()
        .any(|e| e.get("id").and_then(Value::as_str) == Some(event_id))
}

// -- Write path: accept/reject matrix -----------------------------------------

#[tokio::test]
#[ignore = "requires a running relay"]
async fn notify_tag_accepted_on_stream_messages_in_both_modes() {
    let author = Keys::generate();
    let channel = create_channel(&author, "stream").await;

    for mode in ["channel", "here"] {
        let event = message(
            &author,
            KIND_STREAM_MESSAGE,
            channel,
            &format!("heads up @{mode}"),
            &[&["notify", mode]],
        );
        let (status, body) = post_event(&author, &event).await;
        assert!(
            status.is_success() && accepted(&body),
            "mode {mode} must be accepted: {}",
            rejection_message(status, &body)
        );
    }
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn notify_tag_rejected_for_invalid_mode() {
    let author = Keys::generate();
    let channel = create_channel(&author, "stream").await;

    let event = message(
        &author,
        KIND_STREAM_MESSAGE,
        channel,
        "hi",
        &[&["notify", "everyone"]],
    );
    let (status, body) = post_event(&author, &event).await;
    assert!(
        !status.is_success() || !accepted(&body),
        "an unknown notify mode must be rejected: {}",
        rejection_message(status, &body)
    );
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn notify_tag_rejected_when_missing_a_mode() {
    let author = Keys::generate();
    let channel = create_channel(&author, "stream").await;

    let event = message(&author, KIND_STREAM_MESSAGE, channel, "hi", &[&["notify"]]);
    let (status, body) = post_event(&author, &event).await;
    assert!(
        !status.is_success() || !accepted(&body),
        "a bare notify tag must be rejected: {}",
        rejection_message(status, &body)
    );
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn duplicate_notify_tags_are_rejected() {
    let author = Keys::generate();
    let channel = create_channel(&author, "stream").await;

    let event = message(
        &author,
        KIND_STREAM_MESSAGE,
        channel,
        "hi",
        &[&["notify", "channel"], &["notify", "here"]],
    );
    let (status, body) = post_event(&author, &event).await;
    assert!(
        !status.is_success() || !accepted(&body),
        "at most one notify tag is allowed: {}",
        rejection_message(status, &body)
    );
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn notify_tag_rejected_on_disallowed_kind() {
    let author = Keys::generate();
    let channel = create_channel(&author, "stream").await;

    let event = message(
        &author,
        KIND_STREAM_MESSAGE_V2,
        channel,
        "hi",
        &[&["notify", "channel"]],
    );
    let (status, body) = post_event(&author, &event).await;
    assert!(
        !status.is_success() || !accepted(&body),
        "kind 40002 may not carry a notify tag: {}",
        rejection_message(status, &body)
    );
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn notify_tag_rejected_in_dm_channels() {
    let author = Keys::generate();
    let channel = create_channel(&author, "dm").await;

    let event = message(
        &author,
        KIND_STREAM_MESSAGE,
        channel,
        "hi",
        &[&["notify", "channel"]],
    );
    let (status, body) = post_event(&author, &event).await;
    assert!(
        !status.is_success() || !accepted(&body),
        "DM channels must reject channel-wide mentions: {}",
        rejection_message(status, &body)
    );

    // Control: the same message without the tag is fine in the same channel.
    let plain = message(&author, KIND_STREAM_MESSAGE, channel, "hi", &[]);
    let (status, body) = post_event(&author, &plain).await;
    assert!(
        status.is_success() && accepted(&body),
        "untagged DM messages must still be accepted: {}",
        rejection_message(status, &body)
    );
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn notify_tag_rejected_on_kinds_with_their_own_handlers() {
    // Product feedback (42000) and reports (1984) are answered by dedicated
    // handlers that report success before the channel-row gate is reached.
    // The tag must still be rejected — and rejected *as* a notify tag, which
    // only happens if the gate runs ahead of the handler dispatch.
    let author = Keys::generate();
    let channel = create_channel(&author, "stream").await;

    for kind in [KIND_PRODUCT_FEEDBACK, KIND_REPORT] {
        let event = message(&author, kind, channel, "hi", &[&["notify", "channel"]]);
        let (status, body) = post_event(&author, &event).await;
        let detail = rejection_message(status, &body);
        assert!(
            !status.is_success() || !accepted(&body),
            "kind {kind} must not accept a notify tag: {detail}"
        );
        assert!(
            detail.contains("notify tag"),
            "kind {kind} must be rejected as a notify tag, not by its own handler: {detail}"
        );
    }
}

#[tokio::test]
#[ignore = "requires a running relay"]
async fn non_members_may_not_notify_an_open_channel() {
    let owner = Keys::generate();
    let outsider = Keys::generate();
    let channel = create_channel(&owner, "stream").await;

    // Control: an open channel accepts an ordinary message from a non-member.
    let plain = message(&outsider, KIND_STREAM_MESSAGE, channel, "just passing", &[]);
    let (status, body) = post_event(&outsider, &plain).await;
    assert!(
        status.is_success() && accepted(&body),
        "open channels accept untagged writes from non-members: {}",
        rejection_message(status, &body)
    );

    // The open-posting fallback does not extend to the notify tag.
    for mode in ["channel", "here"] {
        let event = message(
            &outsider,
            KIND_STREAM_MESSAGE,
            channel,
            &format!("blast @{mode}"),
            &[&["notify", mode]],
        );
        let (status, body) = post_event(&outsider, &event).await;
        assert!(
            !status.is_success() || !accepted(&body),
            "non-members must not use @{mode} in an open channel: {}",
            rejection_message(status, &body)
        );
    }

    // Joining the roster unlocks it.
    join_channel(&outsider, channel).await;
    let event = message(
        &outsider,
        KIND_STREAM_MESSAGE,
        channel,
        "now a member @channel",
        &[&["notify", "channel"]],
    );
    let (status, body) = post_event(&outsider, &event).await;
    assert!(
        status.is_success() && accepted(&body),
        "members may notify the channel: {}",
        rejection_message(status, &body)
    );
}

// -- Read path: mentions feed --------------------------------------------------

#[tokio::test]
#[ignore = "requires a running relay"]
async fn channel_mention_surfaces_in_member_feeds_and_here_never_does() {
    let author = Keys::generate();
    let member = Keys::generate();
    let outsider = Keys::generate();
    let channel = create_channel(&author, "stream").await;
    join_channel(&member, channel).await;

    let channel_event = message(
        &author,
        KIND_STREAM_MESSAGE,
        channel,
        "deploy window closes in 10 @channel",
        &[&["notify", "channel"]],
    );
    let (status, body) = post_event(&author, &channel_event).await;
    assert!(
        status.is_success() && accepted(&body),
        "@channel message must be accepted: {}",
        rejection_message(status, &body)
    );

    let here_event = message(
        &author,
        KIND_STREAM_MESSAGE,
        channel,
        "standup now @here",
        &[&["notify", "here"]],
    );
    let (status, body) = post_event(&author, &here_event).await;
    assert!(
        status.is_success() && accepted(&body),
        "@here message must be accepted: {}",
        rejection_message(status, &body)
    );

    let channel_event_id = channel_event.id.to_hex();
    let here_event_id = here_event.id.to_hex();

    let member_feed = mentions_feed(&member).await;
    assert!(
        feed_contains(&member_feed, &channel_event_id),
        "channel members must see the @channel event in their mentions feed"
    );
    assert!(
        !feed_contains(&member_feed, &here_event_id),
        "@here is live-only and must never reach the mentions feed"
    );

    let outsider_feed = mentions_feed(&outsider).await;
    assert!(
        !feed_contains(&outsider_feed, &channel_event_id),
        "non-members must not see the @channel event"
    );

    let author_feed = mentions_feed(&author).await;
    assert!(
        !feed_contains(&author_feed, &channel_event_id),
        "the author's own @channel event is not a mention of the author"
    );
}
