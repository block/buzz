// Exhausted HTTP authority retry through the real relay command/replay loop.
use super::*;
use buzz_core::kind::{KIND_STREAM_MESSAGE, KIND_WORKFLOW_DEF, KIND_WORKFLOW_MENTION_WAKE};
use buzz_core::workflow_wake::WorkflowMentionWake;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::sync::Arc;

#[tokio::test]
async fn exhausted_authority_failure_replays_exact_wake_and_verifies_before_dispatch() {
    assert_authority_recovery(AuthorityFailure::Status).await;
}

#[tokio::test]
async fn truncated_authority_body_replays_before_dispatch() {
    assert_authority_recovery(AuthorityFailure::TruncatedBody).await;
}

#[tokio::test]
async fn stalled_authority_body_replays_before_dispatch() {
    assert_authority_recovery(AuthorityFailure::StalledBody).await;
}

#[derive(Clone, Copy)]
enum AuthorityFailure {
    Status,
    TruncatedBody,
    StalledBody,
}

async fn assert_authority_recovery(failure: AuthorityFailure) {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let relay_key = Keys::generate();
    let channel = Uuid::new_v4();
    let run = Uuid::new_v4();
    let workflow = Uuid::new_v4();
    let definition = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF as u16),
        "name: wake\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify\n    action: send_message\n    text: work\n")
        .tags([Tag::parse(["d", &workflow.to_string()]).unwrap(),
            Tag::parse(["h", &channel.to_string()]).unwrap()])
        .sign_with_keys(&owner).unwrap();
    let message = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "work")
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::public_key(agent.public_key()),
            Tag::parse(["buzz:workflow", "true"]).unwrap(),
            Tag::parse(["buzz:workflow-owner", &owner.public_key().to_hex()]).unwrap(),
            Tag::parse(["buzz:workflow-mention", &agent.public_key().to_hex()]).unwrap(),
            Tag::parse(["workflow-run", &run.to_string()]).unwrap(),
            Tag::parse(["workflow-definition", &definition.id.to_hex()]).unwrap(),
            Tag::parse(["workflow-step", "notify"]).unwrap(),
        ])
        .sign_with_keys(&relay_key)
        .unwrap();
    let wake =
        WorkflowMentionWake::new(agent.public_key(), channel, run, definition.id, message.id)
            .sign(&relay_key)
            .unwrap();
    let body = json!({"run_id":run, "channel_id":channel, "workflow_id":workflow,
        "definition_event_id":definition.id.to_hex(), "workflow_owner":owner.public_key().to_hex(),
        "definition":definition, "message":message})
    .to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // Status failures exhaust the HTTP retry budget. Body failures arrive
    // after successful headers and must independently reopen transport replay.
    let failures = if matches!(failure, AuthorityFailure::Status) {
        4
    } else {
        1
    };
    let http_server = tokio::spawn(async move {
        for index in 0..=failures {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let len = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..len]);
            assert!(request.starts_with(&format!("GET /workflow-wakes/{run}/")));
            if index < failures {
                match failure {
                    AuthorityFailure::Status => {
                        stream.write_all(b"HTTP/1.1 503 test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    }
                    AuthorityFailure::TruncatedBody | AuthorityFailure::StalledBody => {
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();
                        if matches!(failure, AuthorityFailure::StalledBody) {
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_secs(5)).await;
                                drop(stream);
                            });
                        }
                    }
                }
            } else {
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        }
    });
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .unwrap();
    let rest = RestClient {
        http: http.clone(),
        base_url: format!("http://{address}"),
        keys: agent.clone(),
        auth_tag_json: None,
    };
    let (ws, mut server) = test_ws_pair().await;
    let (event_tx, event_rx) = mpsc::channel(16);
    let (observer_control_tx, observer_control_rx) = mpsc::channel(16);
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let bg = tokio::spawn(run_background_task(
        ws,
        VecDeque::new(),
        event_tx,
        observer_control_tx,
        cmd_rx,
        agent.clone(),
        "ws://unused".into(),
        agent.public_key().to_hex(),
        None,
    ));
    let mut harness = HarnessRelay {
        event_rx,
        observer_control_rx: Some(observer_control_rx),
        cmd_tx,
        http,
        relay_url: "ws://unused".into(),
        keys: agent.clone(),
        auth_tag: None,
        bg_handle: Some(bg),
    };
    harness
        .subscribe_channel_from(
            channel,
            ChannelFilter {
                kinds: Some(vec![KIND_WORKFLOW_MENTION_WAKE]),
                require_mention: false,
            },
            Some(wake.created_at.as_secs()),
        )
        .await
        .unwrap();
    let initial = next_data_frame(&mut server).await;
    let frame = json!(["EVENT", channel_sub_id(channel), wake]).to_string();
    server
        .send(Message::Text(frame.clone().into()))
        .await
        .unwrap();
    let received = timeout(Duration::from_secs(2), harness.next_event())
        .await
        .unwrap()
        .unwrap();
    let authenticated =
        crate::workflow_wake::authenticate(&received.event, relay_key.public_key()).unwrap();
    let error = rest
        .workflow_wake_authority(authenticated.run_id(), &authenticated.message_event_id())
        .await
        .expect_err("authority transfer fails");
    assert!(error.is_transient());
    assert!(
        harness.event_rx.try_recv().is_err(),
        "no fabricated event on lookup failure"
    );
    harness
        .replay_event(
            channel,
            received.event.id.to_hex(),
            received.event.created_at.as_secs(),
        )
        .await
        .unwrap();
    let replay = next_data_frame(&mut server).await;
    assert_eq!(replay[0], "REQ");
    assert_eq!(replay[1], initial[1]);
    assert_eq!(replay[2]["kinds"], json!([KIND_WORKFLOW_MENTION_WAKE]));
    assert_eq!(replay[2]["#h"], json!([channel.to_string()]));
    assert_eq!(replay[2]["#p"], json!([agent.public_key().to_hex()]));
    assert!(replay[2]["since"].as_u64().unwrap() <= wake.created_at.as_secs());
    server
        .send(Message::Text(frame.clone().into()))
        .await
        .unwrap();
    let replayed = timeout(Duration::from_secs(2), harness.next_event())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replayed.event.id, wake.id);
    let authority = rest
        .workflow_wake_authority(run, &message.id)
        .await
        .unwrap();
    let (verified, principal) = crate::workflow_wake::verify(
        &replayed.event,
        authority,
        relay_key.public_key(),
        agent.public_key(),
        channel,
    )
    .expect("full authority verified");
    assert_eq!(verified.id, message.id);
    assert_eq!(principal, owner.public_key().to_hex());
    server.send(Message::Text(frame.into())).await.unwrap();
    assert!(
        timeout(Duration::from_millis(100), harness.next_event())
            .await
            .is_err(),
        "normal dedup resumes"
    );
    http_server.await.unwrap();
    harness.shutdown().await;
}

async fn next_data_frame(server: &mut WebSocketStream<tokio::net::TcpStream>) -> Value {
    timeout(Duration::from_secs(2), async {
        loop {
            match server.next().await.expect("websocket open").expect("frame") {
                Message::Text(text) => return serde_json::from_str(&text).expect("JSON frame"),
                Message::Ping(payload) => server.send(Message::Pong(payload)).await.expect("pong"),
                other => panic!("unexpected frame {other:?}"),
            }
        }
    })
    .await
    .expect("data frame before timeout")
}

#[tokio::test]
async fn complete_malformed_authority_body_is_terminal() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for body in ["{", "{}"] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    });
    let rest = RestClient {
        http: reqwest::Client::new(),
        base_url: format!("http://{address}"),
        keys: Keys::generate(),
        auth_tag_json: None,
    };
    for _ in 0..2 {
        let error = rest
            .workflow_wake_authority(Uuid::new_v4(), &nostr::EventId::all_zeros())
            .await
            .unwrap_err();
        assert!(
            !error.is_transient(),
            "complete malformed authority must not replay"
        );
    }
    server.await.unwrap();
}

#[tokio::test]
async fn missing_identity_ingress_does_not_reopen_transport_dedup() {
    assert_identity_ingress(true).await;
}

#[tokio::test]
async fn pending_rotation_ingress_reopens_transport_dedup_until_identity_recovers() {
    assert_identity_ingress(false).await;
}

async fn assert_identity_ingress(missing: bool) {
    use crate::inbound_author_gate::InboundAuthorGate;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let agent = Keys::generate();
    let old = Keys::generate();
    let new = Keys::generate();
    let channel = Uuid::new_v4();
    let wake = WorkflowMentionWake::new(agent.public_key(), channel, Uuid::new_v4(),
        nostr::EventId::from_byte_array([1; 32]), nostr::EventId::from_byte_array([2; 32]))
        .sign(&new).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let probes = Arc::new(AtomicUsize::new(0));
    let count = probes.clone();
    let key = new.public_key();
    let http_server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let len = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..len]);
            assert!(request.starts_with("GET / ") || request.starts_with("GET /info "));
            let index = count.fetch_add(1, Ordering::SeqCst);
            let (status, body) = if missing {
                ("200 OK", json!({}).to_string())
            } else if index == 1 || index == 2 {
                ("500 Internal Server Error", String::new())
            } else {
                ("200 OK", json!({"self": if index == 0 { old.public_key() } else { key }.to_hex()}).to_string())
            };
            stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let http = reqwest::Client::new();
    let rest = RestClient { http: http.clone(), base_url: format!("http://{address}"), keys: agent.clone(), auth_tag_json: None };
    let mut gate = InboundAuthorGate::connect(&rest, &agent.public_key().to_hex(), "test").await;
    let (ws, mut server) = test_ws_pair().await;
    let (event_tx, event_rx) = mpsc::channel(16);
    let (observer_control_tx, observer_control_rx) = mpsc::channel(16);
    let (cmd_tx, cmd_rx) = mpsc::channel(16);
    let bg = tokio::spawn(run_background_task(ws, VecDeque::new(), event_tx, observer_control_tx,
        cmd_rx, agent.clone(), "ws://unused".into(), agent.public_key().to_hex(), None));
    let mut harness = HarnessRelay { event_rx, observer_control_rx: Some(observer_control_rx), cmd_tx,
        http, relay_url: "ws://unused".into(), keys: agent, auth_tag: None, bg_handle: Some(bg) };
    harness.subscribe_channel_from(channel, ChannelFilter { kinds: Some(vec![KIND_WORKFLOW_MENTION_WAKE]),
        require_mention: false }, Some(wake.created_at.as_secs())).await.unwrap();
    let initial = next_data_frame(&mut server).await;
    let frame = json!(["EVENT", channel_sub_id(channel), wake]).to_string();
    server.send(Message::Text(frame.clone().into())).await.unwrap();
    let mut received = timeout(Duration::from_secs(2), harness.next_event()).await.unwrap().unwrap();
    // Supply the reconnect generation at the production ingress seam. The
    // transport below is real; its separate reconnect tests cover the counter.
    received.connection_generation = u64::from(!missing);
    let start = tokio::time::Instant::now();
    assert!(crate::workflow_wake::authenticate_for_listener(&mut gate, &harness, &rest, &received).await.is_none());
    if missing {
        // The transport heartbeat can send Ping independently of replay.
        // Answer control frames and assert that no application REQ is sent.
        assert!(timeout(Duration::from_millis(100), next_data_frame(&mut server)).await.is_err(), "terminal absence must not issue replay REQ");
        assert_eq!(probes.load(Ordering::SeqCst), 2);
    } else {
        assert!(start.elapsed() >= Duration::from_secs(1), "immediate discovery failures must be paced");
        let replay = next_data_frame(&mut server).await;
        assert_eq!(replay[0], "REQ");
        assert_eq!(replay[1], initial[1]);
        server.send(Message::Text(frame.clone().into())).await.unwrap();
        let mut replayed = timeout(Duration::from_secs(2), harness.next_event()).await.unwrap().unwrap();
        assert_eq!(replayed.event.id, wake.id);
        replayed.connection_generation = 1;
        let (_, current) = crate::workflow_wake::authenticate_for_listener(&mut gate, &harness, &rest, &replayed).await.expect("same wake recovers before definitive signer rejection");
        assert_eq!(current, key);
        assert_eq!(probes.load(Ordering::SeqCst), 4);
    }
    server.send(Message::Text(frame.into())).await.unwrap();
    assert!(timeout(Duration::from_millis(100), harness.next_event()).await.is_err(), "terminal/successful ingress preserves dedup");
    http_server.abort();
    harness.shutdown().await;
}
