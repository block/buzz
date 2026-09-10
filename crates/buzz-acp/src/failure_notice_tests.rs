//! Production seam tests for addressed ACP failure notices.
//!
//! This module is included from `lib.rs`'s test module.  It deliberately calls
//! the notice builder and `pool::post_failure_notice`; testing only the tag
//! collector would allow the submitted event to diverge from the contract.

use std::sync::{Arc, Mutex};

use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

fn event(keys: &Keys, content: &str, thread: Option<(&str, &str)>, failure: bool) -> Event {
    let mut builder = EventBuilder::new(Kind::Custom(9), content);
    if let Some((root, parent)) = thread {
        builder = builder
            .tag(Tag::parse(vec!["e", root, "", "root"]).expect("root tag"))
            .tag(Tag::parse(vec!["e", parent, "", "reply"]).expect("parent tag"));
    }
    if failure {
        builder = builder.tag(Tag::parse(vec!["buzz:agent-failure", "1"]).expect("failure tag"));
    }
    builder.sign_with_keys(keys).expect("test event signs")
}

fn batch(
    channel_id: uuid::Uuid,
    events: Vec<Event>,
    cancelled_events: Vec<Event>,
) -> queue::FlushBatch {
    queue::FlushBatch {
        channel_id,
        scope: scope::SessionScope::Conversation { channel_id },
        events: events
            .into_iter()
            .map(|event| queue::BatchEvent {
                event,
                prompt_tag: "@mention".into(),
                received_at: std::time::Instant::now(),
            })
            .collect(),
        cancelled_events: cancelled_events
            .into_iter()
            .map(|event| queue::BatchEvent {
                event,
                prompt_tag: "@mention".into(),
                received_at: std::time::Instant::now(),
            })
            .collect(),
        cancel_reason: Some(queue::CancelReason::Steer),
    }
}

fn batch_with_scope(
    channel_id: uuid::Uuid,
    scope: scope::SessionScope,
    events: Vec<Event>,
    cancelled_events: Vec<Event>,
) -> queue::FlushBatch {
    let mut batch = batch(channel_id, events, cancelled_events);
    batch.scope = scope;
    batch
}

fn tag_values(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let fields = tag.as_slice();
            (fields.first().map(String::as_str) == Some(name))
                .then(|| fields.get(1).cloned())
                .flatten()
        })
        .collect()
}

#[test]
fn build_addresses_distinct_authors_and_excludes_processing_agent() {
    let agent = Keys::generate();
    let author_a = Keys::generate();
    let author_b = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    let batch = batch(
        channel_id,
        vec![
            event(&author_a, "a-1", None, false),
            event(&agent, "self", None, false),
            event(&author_a, "a-2", None, false),
            event(&author_b, "b", None, false),
        ],
        vec![],
    );

    let notice = failure_notice::build(&agent, &batch, "limit").expect("notice builds");
    assert_eq!(
        tag_values(&notice, "p"),
        vec![
            author_b.public_key().to_hex(),
            author_a.public_key().to_hex()
        ]
    );
    assert_eq!(tag_values(&notice, "buzz:agent-failure"), vec!["1"]);
}

#[test]
fn build_collects_cancelled_and_current_events_but_skips_marked_failures() {
    let agent = Keys::generate();
    let author_a = Keys::generate();
    let author_b = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    let root = event(&author_a, "root", None, false);
    let root_id = root.id.to_hex();
    let cancelled = event(&author_a, "cancelled", Some((&root_id, &root_id)), false);
    let failed_sender = Keys::generate();
    let old_failure = event(
        &failed_sender,
        "old failure",
        Some((&root_id, &root_id)),
        true,
    );
    let current = event(&author_b, "current", Some((&root_id, &root_id)), false);
    let current_id = current.id.to_hex();
    let batch = batch(channel_id, vec![old_failure, current], vec![cancelled]);

    let notice = failure_notice::build(&agent, &batch, "limit").expect("notice builds");
    assert_eq!(
        tag_values(&notice, "p"),
        vec![
            author_b.public_key().to_hex(),
            author_a.public_key().to_hex()
        ]
    );
    assert_eq!(tag_values(&notice, "buzz:agent-failure"), vec!["1"]);
    assert_eq!(tag_values(&notice, "e"), vec![root_id, current_id]);
}

#[test]
fn build_top_level_anchors_notice_root_and_parent_to_trigger() {
    let agent = Keys::generate();
    let author = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    let trigger = event(&author, "trigger", None, false);
    let trigger_id = trigger.id.to_hex();
    let notice = failure_notice::build(&agent, &batch(channel_id, vec![trigger], vec![]), "limit")
        .expect("notice builds");
    let tags = queue::parse_thread_tags(&notice);
    assert_eq!(tags.root_event_id.as_deref(), Some(trigger_id.as_str()));
    assert_eq!(tags.parent_event_id.as_deref(), Some(trigger_id.as_str()));
}

#[test]
fn build_caps_recipients_at_fifty_without_duplicates_or_self() {
    let agent = Keys::generate();
    let authors: Vec<Keys> = (0..55).map(|_| Keys::generate()).collect();
    let channel_id = uuid::Uuid::new_v4();
    let mut events = vec![event(&agent, "self", None, false)];
    events.extend(
        authors
            .iter()
            .map(|keys| event(keys, "request", None, false)),
    );
    let notice = failure_notice::build(&agent, &batch(channel_id, events, vec![]), "limit")
        .expect("notice builds");
    let recipients = tag_values(&notice, "p");
    assert_eq!(recipients.len(), 50);
    assert_eq!(
        recipients,
        authors[5..]
            .iter()
            .rev()
            .map(|k| k.public_key().to_hex())
            .collect::<Vec<_>>()
    );
    assert!(!recipients.contains(&agent.public_key().to_hex()));
}

#[tokio::test]
async fn post_failure_notice_submits_signed_event_through_local_http() {
    let agent = Keys::generate();
    let author = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    let trigger = event(&author, "request", None, false);
    let trigger_id = trigger.id.to_hex();
    let batch = batch(channel_id, vec![trigger], vec![]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let base_url = format!("http://{}", listener.local_addr().expect("fixture address"));
    let captured: Arc<Mutex<Option<Event>>> = Arc::new(Mutex::new(None));
    let captured_server = Arc::clone(&captured);
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept fixture request");
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let size = socket.read(&mut chunk).await.expect("read fixture request");
            assert!(size > 0, "fixture closed before headers");
            request.extend_from_slice(&chunk[..size]);
            if let Some(offset) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                break offset + 4;
            }
            assert!(request.len() <= 32 * 1024, "fixture headers exceed bound");
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        assert!(
            headers.starts_with("POST /events "),
            "notice must POST /events"
        );
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))
            })
            .and_then(|value| value.trim().parse::<usize>().ok())
            .expect("content length header");
        assert!(content_length <= 64 * 1024, "fixture body exceeds bound");
        while request.len() - header_end < content_length {
            let size = socket
                .read(&mut chunk)
                .await
                .expect("read complete fixture body");
            assert!(size > 0, "fixture closed before complete body");
            request.extend_from_slice(&chunk[..size]);
        }
        let body = &request[header_end..header_end + content_length];
        let submitted: Event = serde_json::from_slice(body).expect("submitted event JSON");
        *captured_server.lock().expect("capture lock") = Some(submitted);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .await
            .expect("write fixture response");
    });
    let rest = relay::RestClient {
        http: reqwest::Client::new(),
        base_url,
        keys: agent.clone(),
        auth_tag_json: None,
    };

    pool::post_failure_notice(&rest, &batch, "limit").await;
    tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .expect("fixture completes")
        .expect("fixture task succeeds");
    let submitted = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("event captured");
    assert_eq!(submitted.kind, Kind::Custom(9));
    assert_eq!(submitted.pubkey, agent.public_key());
    assert!(submitted.verify().is_ok());
    assert_eq!(
        tag_values(&submitted, "p"),
        vec![author.public_key().to_hex()]
    );
    assert_eq!(tag_values(&submitted, "buzz:agent-failure"), vec!["1"]);
    assert_eq!(tag_values(&submitted, "h"), vec![channel_id.to_string()]);
    let tags = queue::parse_thread_tags(&submitted);
    assert_eq!(tags.root_event_id.as_deref(), Some(trigger_id.as_str()));
    assert_eq!(tags.parent_event_id.as_deref(), Some(trigger_id.as_str()));
}

#[test]
fn failure_chain_is_not_addressed_again_and_does_not_loop() {
    let coordinator = Keys::generate();
    let worker = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    let request = event(&coordinator, "delegated request", None, false);
    let worker_notice = failure_notice::build(
        &worker,
        &batch(channel_id, vec![request], vec![]),
        "worker unavailable",
    )
    .expect("worker notice");
    assert_eq!(
        tag_values(&worker_notice, "p"),
        vec![coordinator.public_key().to_hex()]
    );
    let coordinator_notice = failure_notice::build(
        &coordinator,
        &batch(channel_id, vec![worker_notice], vec![]),
        "coordinator also unavailable",
    )
    .expect("coordinator notice");
    assert!(
        tag_values(&coordinator_notice, "p").is_empty(),
        "failure must not wake worker again"
    );
    assert_eq!(
        tag_values(&coordinator_notice, "buzz:agent-failure"),
        vec!["1"]
    );
}

#[test]
fn original_mentions_are_not_notification_recipients() {
    let agent = Keys::generate();
    let author = Keys::generate();
    let unrelated = Keys::generate().public_key().to_hex();
    let channel_id = uuid::Uuid::new_v4();
    let request = EventBuilder::new(Kind::Custom(9), "delegated request")
        .tag(Tag::parse(["p", unrelated.as_str()]).expect("mention tag"))
        .sign_with_keys(&author)
        .expect("sign request");
    let notice = failure_notice::build(&agent, &batch(channel_id, vec![request], vec![]), "limit")
        .expect("notice");
    assert_eq!(tag_values(&notice, "p"), vec![author.public_key().to_hex()]);
    assert!(!tag_values(&notice, "p").contains(&unrelated));
}

#[test]
fn thread_scope_canonical_root_wins_over_raw_event_markers() {
    let agent = Keys::generate();
    let author = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    let canonical_root = "ab".repeat(32);
    let raw_root = "cd".repeat(32);
    let trigger = event(&author, "reply", Some((&raw_root, &raw_root)), false);
    let batch = batch_with_scope(
        channel_id,
        scope::SessionScope::Thread {
            channel_id,
            root_event_id: canonical_root.clone(),
        },
        vec![trigger],
        vec![],
    );
    let notice = failure_notice::build(&agent, &batch, "limit").expect("notice builds");
    let e = tag_values(&notice, "e");
    assert_eq!(e[0], canonical_root);
    assert_ne!(e[0], raw_root);
    assert_eq!(e.len(), 2);
}

#[test]
fn empty_batch_and_scope_channel_mismatch_fail_closed() {
    let agent = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    assert!(failure_notice::build(&agent, &batch(channel_id, vec![], vec![]), "limit").is_err());
    let other_channel = uuid::Uuid::new_v4();
    let author = Keys::generate();
    let mismatch = batch_with_scope(
        channel_id,
        scope::SessionScope::Conversation {
            channel_id: other_channel,
        },
        vec![event(&author, "request", None, false)],
        vec![],
    );
    assert!(failure_notice::build(&agent, &mismatch, "limit").is_err());
}

#[test]
fn owner_originated_failure_has_no_synthetic_reserve_mention() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let channel_id = uuid::Uuid::new_v4();
    let notice = failure_notice::build(
        &agent,
        &batch(
            channel_id,
            vec![event(&owner, "owner request", None, false)],
            vec![],
        ),
        "limit",
    )
    .expect("notice builds");
    assert_eq!(tag_values(&notice, "p"), vec![owner.public_key().to_hex()]);
    assert_eq!(tag_values(&notice, "buzz:agent-failure"), vec!["1"]);
    // No reserve-agent key is invented; a coordinator-originated failure still
    // needs a separate owner/status recovery path when no distinct sender exists.
}
