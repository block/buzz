use super::*;

fn signed_pr(agent: &Keys, owner: &Keys, channel: Uuid, created_at: u64) -> Event {
    let repository = format!("30617:{}:buzz", owner.public_key().to_hex());
    EventBuilder::new(Kind::Custom(KIND_GIT_PULL_REQUEST as u16), "ship it")
        .tags([
            Tag::parse(["a", repository.as_str()]).unwrap(),
            Tag::parse(["p", owner.public_key().to_hex().as_str()]).unwrap(),
            Tag::parse(["subject", "Lifecycle wakeup"]).unwrap(),
            Tag::parse(["c", "a".repeat(40).as_str()]).unwrap(),
            Tag::parse(["h", channel.to_string().as_str()]).unwrap(),
            Tag::parse(["clone", "https://example.test/buzz.git"]).unwrap(),
        ])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(agent)
        .unwrap()
}

fn signed_status(owner: &Keys, agent: &Keys, pr: &Event, created_at: u64) -> Event {
    let repository = format!("30617:{}:buzz", owner.public_key().to_hex());
    let pr_id = pr.id.to_hex();
    let merge = "b".repeat(40);
    EventBuilder::new(Kind::Custom(KIND_GIT_STATUS_MERGED as u16), "")
        .tags([
            Tag::parse(["e", pr_id.as_str(), "", "root"]).unwrap(),
            Tag::parse(["a", repository.as_str()]).unwrap(),
            Tag::parse(["p", owner.public_key().to_hex().as_str()]).unwrap(),
            Tag::parse(["p", agent.public_key().to_hex().as_str()]).unwrap(),
            Tag::parse(["merge-commit", merge.as_str()]).unwrap(),
            Tag::parse(["r", merge.as_str()]).unwrap(),
        ])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(owner)
        .unwrap()
}

async fn lifecycle_test_server(
    pull_request: Event,
) -> (
    RestClient,
    mpsc::Receiver<(String, serde_json::Value)>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let keys = Keys::generate();
    let client = RestClient {
        http: reqwest::Client::builder().build().unwrap(),
        base_url,
        keys,
        auth_tag_json: None,
    };
    let (request_tx, request_rx) = mpsc::channel(8);
    let server = tokio::spawn(async move {
        for _ in 0..4 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            let header_end;
            let content_length;
            loop {
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "request closed before headers");
                request.extend_from_slice(&chunk[..read]);
                assert!(request.len() <= 64 * 1024, "test request exceeded bound");
                if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    header_end = end + 4;
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length: ")
                                .or_else(|| line.strip_prefix("Content-Length: "))
                        })
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break;
                }
            }
            while request.len() < header_end + content_length {
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "request closed before body");
                request.extend_from_slice(&chunk[..read]);
                assert!(request.len() <= 64 * 1024, "test request exceeded bound");
            }
            let first_line = String::from_utf8_lossy(&request[..header_end])
                .lines()
                .next()
                .unwrap()
                .to_string();
            let path = first_line.split_whitespace().nth(1).unwrap().to_string();
            let body: serde_json::Value =
                serde_json::from_slice(&request[header_end..header_end + content_length]).unwrap();
            request_tx.send((path.clone(), body.clone())).await.unwrap();

            let response_body = if path == "/query"
                && body[0]["kinds"] == serde_json::json!([KIND_GIT_PULL_REQUEST])
            {
                serde_json::to_string(&vec![&pull_request]).unwrap()
            } else if path == "/query" {
                "[]".to_string()
            } else {
                serde_json::json!({
                    "event_id": "a".repeat(64),
                    "accepted": true,
                    "message": "stored"
                })
                .to_string()
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (client, request_rx, server)
}

#[tokio::test]
async fn manager_binds_resolution_persistence_routing_and_completion_seams() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let channel = Uuid::new_v4();
    let pull_request = signed_pr(&agent, &owner, channel, 10);
    let status = signed_status(&owner, &agent, &pull_request, 20);
    let status_id = status.id.to_hex();
    let (mut rest, mut requests, server) = lifecycle_test_server(pull_request).await;
    rest.keys = agent.clone();
    let (addressed_tx, addressed_rx) = mpsc::channel(4);
    let (handle, mut output_rx, manager) = start(agent, rest, addressed_rx).await.unwrap();

    addressed_tx.send(status).await.unwrap();
    let output = tokio::time::timeout(Duration::from_secs(2), output_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let AddressedEvent::Merge(wakeup) = output else {
        panic!("expected merge wakeup");
    };
    assert_eq!(wakeup.buzz_event.channel_id, channel);
    assert_eq!(wakeup.buzz_event.event.id.to_hex(), status_id);
    handle.ignore(status_id).unwrap();

    let mut recorded = Vec::new();
    for _ in 0..4 {
        recorded.push(
            tokio::time::timeout(Duration::from_secs(2), requests.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(recorded[0].0, "/query");
    assert_eq!(
        recorded[0].1[0]["kinds"],
        serde_json::json!([KIND_READ_STATE])
    );
    assert_eq!(
        recorded[1].1[0]["kinds"],
        serde_json::json!([KIND_GIT_PULL_REQUEST])
    );
    assert_eq!(recorded[2].0, "/events");
    assert_eq!(recorded[2].1["kind"], KIND_READ_STATE);
    assert_eq!(recorded[3].0, "/events");
    assert_eq!(recorded[3].1["kind"], KIND_READ_STATE);

    manager.abort();
    server.await.unwrap();
}

#[test]
fn production_validators_route_only_agent_authored_pr() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let channel = Uuid::new_v4();
    let pr = signed_pr(&agent, &owner, channel, 10);
    let status = signed_status(&owner, &agent, &pr, 20);
    let meta = validate_status(&status, &agent.public_key()).unwrap();
    let resolved = validate_pull_request(&pr, &status, &meta, &agent.public_key()).unwrap();
    assert_eq!(resolved.channel_id, channel);

    let other_agent = Keys::generate();
    assert!(validate_pull_request(&pr, &status, &meta, &other_agent.public_key()).is_err());
}

#[test]
fn status_signer_must_be_repository_owner() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let impostor = Keys::generate();
    let pr = signed_pr(&agent, &owner, Uuid::new_v4(), 10);
    let repository = format!("30617:{}:buzz", owner.public_key().to_hex());
    let pr_id = pr.id.to_hex();
    let merge = "b".repeat(40);
    let status = EventBuilder::new(Kind::Custom(KIND_GIT_STATUS_MERGED as u16), "")
        .tags([
            Tag::parse(["e", pr_id.as_str(), "", "root"]).unwrap(),
            Tag::parse(["a", repository.as_str()]).unwrap(),
            Tag::parse(["p", owner.public_key().to_hex().as_str()]).unwrap(),
            Tag::parse(["p", agent.public_key().to_hex().as_str()]).unwrap(),
            Tag::parse(["merge-commit", merge.as_str()]).unwrap(),
            Tag::parse(["r", merge.as_str()]).unwrap(),
        ])
        .custom_created_at(Timestamp::from(20))
        .sign_with_keys(&impostor)
        .unwrap();
    assert!(validate_status(&status, &agent.public_key()).is_err());
}

#[test]
fn semantic_dedup_survives_distinct_status_event_ids() {
    let key = semantic_key(
        &format!("30617:{}:buzz", "a".repeat(64)),
        &"b".repeat(64),
        &"c".repeat(40),
    );
    let mut ledger = Ledger::default();
    assert!(ledger.add_pending("d".repeat(64), key.clone()));
    assert!(!ledger.add_pending("e".repeat(64), key));
    assert!(ledger.complete(&"d".repeat(64)));
    ledger.validate().unwrap();
}

#[test]
fn lifecycle_ledger_round_trips_through_signed_self_encrypted_event() {
    let keys = Keys::generate();
    let mut ledger = Ledger::default();
    assert!(ledger.add_pending("d".repeat(64), "e".repeat(64)));
    let (event, created_at) = build_ledger_event(&keys, &ledger, 42).unwrap();
    assert!(created_at >= 43);
    assert_ne!(event.content, serde_json::to_string(&ledger).unwrap());
    assert_eq!(decode_ledger_event(&event, &keys).unwrap(), ledger);
}

#[test]
fn dispatched_batch_tracks_lifecycle_ids_across_cancel_merge_framing() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let channel = Uuid::new_v4();
    let pr = signed_pr(&agent, &owner, channel, 10);
    let status = signed_status(&owner, &agent, &pr, 20);
    let lifecycle_id = status.id.to_hex();
    let lifecycle = crate::queue::BatchEvent {
        event: status,
        prompt_tag: PROMPT_TAG.into(),
        received_at: std::time::Instant::now(),
    };
    let ordinary = crate::queue::BatchEvent {
        event: signed_pr(&agent, &owner, channel, 30),
        prompt_tag: "all".into(),
        received_at: std::time::Instant::now(),
    };
    let batch = crate::queue::FlushBatch {
        channel_id: channel,
        scope: crate::scope::SessionScope::Conversation {
            channel_id: channel,
        },
        events: vec![ordinary],
        cancelled_events: vec![lifecycle],
        cancel_reason: Some(crate::queue::CancelReason::Steer),
    };
    assert_eq!(batch_event_ids(&batch), vec![lifecycle_id]);
}

#[test]
fn production_prompt_formatter_is_bound_to_validated_prompt_tag() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let channel = Uuid::new_v4();
    let pr = signed_pr(&agent, &owner, channel, 10);
    let status = signed_status(&owner, &agent, &pr, 20);
    let mut batch_event = crate::queue::BatchEvent {
        event: status,
        prompt_tag: "all".into(),
        received_at: std::time::Instant::now(),
    };
    let ordinary = crate::queue::format_event_block(channel, None, &batch_event, None);
    assert!(!ordinary.contains("verified lifecycle wakeup"));

    batch_event.prompt_tag = PROMPT_TAG.into();
    let lifecycle = crate::queue::format_event_block(channel, None, &batch_event, None);
    assert!(lifecycle.contains("pull request merged successfully"));
    assert!(lifecycle.contains(&pr.id.to_hex()));
    assert!(lifecycle.contains("Do not post a generic acknowledgement"));
    assert!(lifecycle.contains("cannot be a channel-message reply parent"));
}
