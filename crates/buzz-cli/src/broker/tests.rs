use std::sync::{Arc, Mutex};

use axum::{body::Bytes, extract::State, http::HeaderMap, routing::post, Json, Router};
use base64::Engine;
use nostr::{Event, EventBuilder, Kind, Timestamp};
use sha2::{Digest, Sha256};

use super::*;

fn keys(n: u8) -> Keys {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Keys::parse(&format!("{n:064x}")).expect("fixture key")
}

fn scope() -> Scope {
    Scope {
        community: Uuid::from_u128(1),
        channel: Uuid::from_u128(2),
        relay_key: keys(1).public_key(),
        requester: keys(3).public_key(),
    }
}

fn event(signer: &Keys, kind: u16, tags: Vec<Vec<String>>, content: &str) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(
            tags.into_iter()
                .map(|tag| Tag::parse(tag).expect("fixture tag")),
        )
        .custom_created_at(Timestamp::from_secs(100))
        .sign_with_keys(signer)
        .expect("fixture signature")
}

fn policy(public: bool, members: &[u8]) -> Vec<Event> {
    let id = scope().channel.to_string();
    vec![
        event(
            &keys(1),
            39000,
            vec![
                vec!["d".into(), id.clone()],
                vec![if public { "public" } else { "private" }.into()],
                // emit_group_discovery_events uses `t`, not `type` or
                // the creation command's `channel_type` tag.
                vec!["t".into(), "stream".into()],
            ],
            "",
        ),
        event(
            &keys(1),
            39002,
            std::iter::once(vec!["d".into(), id])
                .chain(
                    members
                        .iter()
                        .map(|n| vec!["p".into(), keys(*n).public_key().to_hex()]),
                )
                .collect(),
            "",
        ),
    ]
}

fn message(channel: Uuid) -> Event {
    event(
        &keys(3),
        9,
        vec![vec!["h".into(), channel.to_string()]],
        "hello",
    )
}

struct RelayState {
    policy: Vec<Event>,
    messages: Vec<Event>,
    sent: Vec<Event>,
    filters: Vec<Value>,
    // Change membership while a read is in flight, after its first check.
    policy_after_read: Option<Vec<Event>>,
    reject_publish: bool,
}

struct Fixture {
    url: String,
    state: Arc<Mutex<RelayState>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

// Check real NIP-98 signatures and body hashes in the mock relay. These tests
// exercise HTTP signing, JSON framing, event verification, and IFC together.
fn authenticate(headers: &HeaderMap, body: &[u8]) {
    let auth = headers["authorization"]
        .to_str()
        .expect("auth string")
        .strip_prefix("Nostr ")
        .expect("Nostr auth");
    let event: Event = serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(auth)
            .expect("base64"),
    )
    .expect("auth event");
    event.verify().expect("valid signature");
    assert_eq!(event.pubkey, keys(2).public_key());
    assert!(event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["payload", &hex::encode(Sha256::digest(body))]));
}

async fn query(
    State(state): State<Arc<Mutex<RelayState>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Json<Value> {
    authenticate(&headers, &body);
    let filters: Vec<Value> = serde_json::from_slice(&body).expect("filters");
    assert_eq!(filters.len(), 1);
    let mut state = state.lock().expect("state");
    let filter = &filters[0];
    state.filters.push(filter.clone());
    let events = if filter["kinds"][0] == 39000 {
        state.policy.clone()
    } else {
        assert_eq!(filter["#h"], json!([scope().channel.to_string()]));
        assert_eq!(filter["kinds"], json!([9, 40002]));
        if let Some(policy) = state.policy_after_read.take() {
            state.policy = policy;
        }
        state.messages.clone()
    };
    Json(json!(events))
}

async fn publish(
    State(state): State<Arc<Mutex<RelayState>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Json<Value> {
    authenticate(&headers, &body);
    let event: Event = serde_json::from_slice(&body).expect("published event");
    event.verify().expect("valid signature");
    assert_eq!(event.pubkey, keys(2).public_key());
    assert_eq!(event.kind, Kind::Custom(9));
    assert_eq!(
        event
            .tags
            .iter()
            .map(|t| t.as_slice().to_vec())
            .collect::<Vec<_>>(),
        vec![vec!["h".to_owned(), scope().channel.to_string()]]
    );
    let mut state = state.lock().expect("state");
    state.sent.push(event.clone());
    Json(
        json!({"event_id": event.id, "accepted": !state.reject_publish, "message": "not forwarded"}),
    )
}

impl Fixture {
    async fn new(public: bool) -> Self {
        let state = Arc::new(Mutex::new(RelayState {
            policy: policy(public, &[2, 3]),
            messages: vec![message(scope().channel)],
            sent: vec![],
            filters: vec![],
            policy_after_read: None,
            reject_publish: false,
        }));
        let router = Router::new()
            .route("/query", post(query))
            .route("/events", post(publish))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.expect("relay");
        });
        Self { url, state, task }
    }

    async fn broker(&self) -> Broker {
        Broker::open(
            Relay::new(self.url.clone(), keys(2), None).expect("client"),
            scope(),
        )
        .await
        .expect("broker")
    }
}

// Pin the production socket path, not just a test-only policy predicate. Only
// the broker fixture has a key; its caller gets signed, scoped messages and
// a receipt, never a signer or arbitrary relay transport.
#[tokio::test]
async fn keyless_socket_client_reads_and_replies_in_public_and_private_channels() {
    for public in [true, false] {
        let fixture = Fixture::new(public).await;
        let mut broker = fixture.broker().await;
        for request in [
            Request::Read { limit: 20 },
            Request::Reply {
                content: "answer".into(),
            },
        ] {
            let (mut client, server) = UnixStream::pair().expect("socket pair");
            let (served, received) = tokio::join!(connection(&mut broker, server), async {
                send(&mut client, &request, REQUEST_BYTES)
                    .await
                    .expect("request");
                receive::<Response>(&mut client, RESPONSE_BYTES)
                    .await
                    .expect("response")
            });
            served.expect("served");
            let Response::Ok { result } = received else {
                panic!("expected success")
            };
            match request {
                Request::Read { .. } => assert_eq!(result[0]["content"], "hello"),
                Request::Reply { .. } => assert_eq!(result["accepted"], true),
            }
            assert!(!result
                .to_string()
                .contains(&keys(2).secret_key().to_secret_hex()));
        }
        assert_eq!(
            fixture.state.lock().expect("state").sent[0].content,
            "answer"
        );
    }
}

// Removing any signature, channel, or kind check must admit one of these
// responses. Rejection happens before a single event is returned to the agent.
#[tokio::test]
async fn reads_reject_forged_cross_channel_duplicate_tag_and_wrong_kind_events() {
    let fixture = Fixture::new(false).await;
    let mut broker = fixture.broker().await;
    let mut forged = message(scope().channel);
    forged.content = "unsigned edit".into();
    let duplicate = event(
        &keys(3),
        9,
        vec![
            vec!["h".into(), scope().channel.to_string()],
            vec!["h".into(), Uuid::from_u128(99).to_string()],
        ],
        "secret",
    );
    let wrong_kind = event(
        &keys(3),
        1,
        vec![vec!["h".into(), scope().channel.to_string()]],
        "secret",
    );
    for message in [forged, message(Uuid::from_u128(99)), duplicate, wrong_kind] {
        fixture.state.lock().expect("state").messages = vec![message];
        assert!(broker.handle(Request::Read { limit: 20 }).await.is_err());
    }
}

// A valid signature from another identity is not relay authority. Missing or
// duplicate snapshots must not default to a public audience.
#[tokio::test]
async fn startup_rejects_untrusted_or_incomplete_channel_policy() {
    let fixture = Fixture::new(false).await;
    let original = policy(false, &[2, 3]);
    let mut forged = original.clone();
    forged[0].content = "tampered".into();
    let mut wrong_author = original.clone();
    wrong_author[0] = event(
        &keys(4),
        39000,
        original[0]
            .tags
            .iter()
            .map(|t| t.as_slice().to_vec())
            .collect(),
        "",
    );
    let mut duplicate = original.clone();
    duplicate.push(original[0].clone());
    for policy in [
        vec![],
        vec![original[0].clone()],
        forged,
        wrong_author,
        duplicate,
        policy(false, &[3]),
        policy(false, &[2, 4]),
    ] {
        fixture.state.lock().expect("state").policy = policy;
        assert!(Broker::open(
            Relay::new(fixture.url.clone(), keys(2), None).expect("client"),
            scope()
        )
        .await
        .is_err());
    }
}

// Observing a membership or visibility change poisons the retained session.
// Restoring the old snapshot must not reset it and authorize another reply.
#[tokio::test]
async fn policy_change_permanently_blocks_publication_even_after_rollback() {
    for changed in [
        policy(false, &[2, 3, 4]),
        policy(true, &[2, 3]),
        policy(false, &[3]),
    ] {
        let fixture = Fixture::new(false).await;
        let mut broker = fixture.broker().await;
        fixture.state.lock().expect("state").policy = changed;
        assert!(broker
            .handle(Request::Reply {
                content: "secret".into()
            })
            .await
            .is_err());
        fixture.state.lock().expect("state").policy = policy(false, &[2, 3]);
        assert!(broker
            .handle(Request::Reply {
                content: "still secret".into()
            })
            .await
            .is_err());
        assert!(fixture.state.lock().expect("state").sent.is_empty());
    }
}

// The check after fetching is required: checking only before a read delivers
// data even when a membership change was observable before delivery.
#[tokio::test]
async fn membership_change_during_read_prevents_delivery() {
    let fixture = Fixture::new(false).await;
    let mut broker = fixture.broker().await;
    fixture.state.lock().expect("state").policy_after_read = Some(policy(false, &[2, 3, 4]));
    assert!(broker.handle(Request::Read { limit: 20 }).await.is_err());
}

// Unknown JSON fields are rejected, not ignored. Otherwise an agent could
// believe a destination, signer, or raw event it supplied had been honored.
#[test]
fn requests_cannot_expand_the_broker_scope() {
    for request in [
        json!({"op":"sign", "bytes":"secret"}),
        json!({"op":"read", "limit":20, "channel":"elsewhere"}),
        json!({"op":"reply", "content":"text", "relay":"https://attacker.test"}),
        json!({"op":"reply", "content":"text", "tags":[["h","elsewhere"]]}),
        json!({"op":"reply", "content":"text", "pubkey":"another identity"}),
    ] {
        assert!(serde_json::from_value::<Request>(request).is_err());
    }
}

// Bounds are applied before relay work and before allocating an IPC frame.
#[tokio::test]
async fn malformed_or_oversized_requests_never_reach_the_relay() {
    let fixture = Fixture::new(true).await;
    let mut broker = fixture.broker().await;
    fixture.state.lock().expect("state").filters.clear();
    for request in [
        Request::Read { limit: 0 },
        Request::Read { limit: 101 },
        Request::Reply {
            content: " ".into(),
        },
        Request::Reply {
            content: "x".repeat(16385),
        },
    ] {
        assert!(broker.handle(request).await.is_err());
    }
    let (mut client, server) = UnixStream::pair().expect("sockets");
    let (served, response) = tokio::join!(connection(&mut broker, server), async {
        client.write_u32(u32::MAX).await.expect("length");
        receive::<Response>(&mut client, RESPONSE_BYTES)
            .await
            .expect("response")
    });
    served.expect("error response sent");
    assert!(matches!(response, Response::Error { .. }));
    assert!(fixture.state.lock().expect("state").filters.is_empty());
}

// A relay rejection is an error, never an accepted receipt, and is not retried.
#[tokio::test]
async fn failed_publication_is_not_reported_as_success_or_retried() {
    let fixture = Fixture::new(true).await;
    let mut broker = fixture.broker().await;
    fixture.state.lock().expect("state").reject_publish = true;
    assert!(broker
        .handle(Request::Reply {
            content: "hello".into()
        })
        .await
        .is_err());
    assert_eq!(fixture.state.lock().expect("state").sent.len(), 1);
}

// An idle local peer cannot monopolize the single-session broker indefinitely.
#[tokio::test(start_paused = true)]
async fn silent_socket_client_has_a_deadline() {
    // No HTTP needed: the deadline expires before any request is decoded.
    let relay = Relay::new("http://127.0.0.1:1".into(), keys(2), None).expect("relay");
    let capabilities = buzz_ifc::CapabilitySet::default();
    let domain = buzz_ifc::derive_execution_domain(
        buzz_ifc::DomainFacts {
            community: buzz_ifc::CommunityId::from_uuid(scope().community),
            channel_id: scope().channel,
            kind: buzz_ifc::ConversationKind::Public,
            epoch: buzz_ifc::MembershipEpoch::new("test"),
            members: Default::default(),
            executing_agent: buzz_ifc::Principal::from_public_key(&keys(2).public_key())
                .expect("principal"),
            requesters: [
                buzz_ifc::Principal::from_public_key(&keys(3).public_key()).expect("principal")
            ]
            .into(),
            system_principal: None,
            owner: None,
        },
        &buzz_ifc::CapabilityPolicy::new(capabilities.clone(), capabilities),
    )
    .expect("domain");
    let mut broker = Broker {
        relay,
        scope: scope(),
        session: IfcSession::enter(domain),
    };
    let (_client, server) = UnixStream::pair().expect("sockets");
    assert!(connection(&mut broker, server).await.is_err());
}

#[test]
fn relay_url_rejects_insecure_or_ambiguous_destinations() {
    for url in [
        "http://example.com",
        "https://user:pass@example.com",
        "https://example.com/path",
        "https://example.com/?relay=other",
        "https://example.com/#fragment",
        "file:///tmp/key",
    ] {
        assert!(Relay::new(url.into(), keys(2), None).is_err(), "{url}");
    }
}

// Exercise the CLI dispatch before key parsing, using the real listener and
// handler. Removing the early keyless branch makes both calls fail.
#[tokio::test]
async fn cli_clients_need_neither_a_key_nor_a_relay_and_never_fall_back() {
    let fixture = Fixture::new(false).await;
    let mut broker = fixture.broker().await;
    let (_directory, listener, socket) = bind_socket().expect("socket");
    for command in [
        Command::Read {
            socket: socket.clone(),
            limit: 20,
        },
        Command::Reply {
            socket: socket.clone(),
            content: "from CLI".into(),
        },
    ] {
        let cli = crate::Cli {
            relay: "not a relay URL".into(),
            private_key: None,
            auth_tag: None,
            format: crate::OutputFormat::Json,
            command: crate::Cmd::Broker(command),
        };
        let (served, result) = tokio::join!(
            async {
                let (stream, _) = listener.accept().await.expect("accept");
                connection(&mut broker, stream).await
            },
            crate::run(cli)
        );
        served.expect("serve client");
        result.expect("keyless CLI call");
    }
    drop(listener);
    assert!(call(&Command::Reply {
        socket,
        content: "must not go direct".into()
    })
    .await
    .is_err());
    assert_eq!(fixture.state.lock().expect("state").sent.len(), 1);
}

// Permissions belong to the actual listener setup. A temporary-directory
// default of 0755 would expose the channel socket to other OS users.
#[tokio::test]
async fn socket_is_private_and_a_new_session_never_reuses_its_name() {
    let (directory, listener, path) = bind_socket().expect("socket");
    assert_eq!(
        std::fs::metadata(directory.path())
            .expect("directory")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("socket")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let (_another_directory, _another_listener, another_path) =
        bind_socket().expect("second socket");
    assert_ne!(path, another_path);
    drop(listener);
    drop(directory);
    assert!(!path.exists());
}

// A disconnected caller may have lost an acknowledgement. The broker must
// not repeat the signed publication when writing its response fails.
#[tokio::test]
async fn disconnected_client_does_not_repeat_publication() {
    let fixture = Fixture::new(true).await;
    let mut broker = fixture.broker().await;
    let (mut client, server) = UnixStream::pair().expect("sockets");
    send(
        &mut client,
        &Request::Reply {
            content: "once".into(),
        },
        REQUEST_BYTES,
    )
    .await
    .expect("request");
    drop(client);
    assert!(connection(&mut broker, server).await.is_err());
    assert_eq!(fixture.state.lock().expect("state").sent.len(), 1);
}

// A malicious or broken relay must not turn its response body into unbounded
// allocation, leak an error body, or redirect a signed request to another host.
#[tokio::test]
async fn relay_responses_are_bounded_and_redirects_and_error_bodies_are_rejected() {
    use axum::http::StatusCode;
    for (status, body) in [
        (StatusCode::OK, "x".repeat(1024 * 1024 + 1)),
        (StatusCode::FORBIDDEN, "private relay diagnostic".into()),
        (
            StatusCode::TEMPORARY_REDIRECT,
            "private redirect body".into(),
        ),
    ] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let app = Router::new().route(
            "/query",
            post(move || {
                let body = body.clone();
                async move { (status, [("location", "https://invalid.example/")], body) }
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("relay");
        });
        let result = Relay::new(url, keys(2), None)
            .expect("client")
            .domain(&scope())
            .await;
        task.abort();
        let error = result.expect_err("bad relay response").to_string();
        assert!(
            !error.contains("private"),
            "unscoped relay body must not escape"
        );
        if status.is_redirection() {
            assert!(error.contains("307"));
        }
        if status.is_success() {
            assert!(error.contains("too large"));
        }
    }
}

// Run the actual executable on macOS/Unix, including startup, JSON readiness,
// keyless child environments, and Ctrl-C cleanup. The relay still uses only
// fixture keys; no user's Buzz account or deployed channel is touched.
#[tokio::test]
#[ignore = "build buzz and set BUZZ_TEST_BROKER_BIN to its absolute path"]
async fn broker_executable_smoke() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command as Process;

    let binary = std::env::var("BUZZ_TEST_BROKER_BIN").expect("built buzz path");
    let fixture = Fixture::new(false).await;
    let mut child = Process::new(&binary)
        .env_clear()
        .env("BUZZ_PRIVATE_KEY", keys(2).secret_key().to_secret_hex())
        .args([
            "--relay",
            &fixture.url,
            "broker",
            "serve",
            "--community",
            &scope().community.to_string(),
            "--channel",
            &scope().channel.to_string(),
            "--relay-key",
            &scope().relay_key.to_hex(),
            "--requester",
            &scope().requester.to_hex(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("broker process");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout").take(4096));
    let mut line = String::new();
    timeout(Duration::from_secs(10), stdout.read_line(&mut line))
        .await
        .expect("startup deadline")
        .expect("readiness");
    let ready: Value = serde_json::from_str(&line).expect("JSON readiness");
    let socket = ready["socket"].as_str().expect("socket path");
    for args in [
        vec!["broker", "read", "--socket", socket],
        vec![
            "broker",
            "reply",
            "--socket",
            socket,
            "--content",
            "process smoke",
        ],
    ] {
        let output = timeout(
            Duration::from_secs(10),
            Process::new(&binary)
                .env_clear()
                .args(args)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .expect("client deadline")
        .expect("client process");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).expect("JSON result");
    }
    assert_eq!(fixture.state.lock().expect("state").sent.len(), 1);
    let stopped = Process::new("/bin/kill")
        .args(["-INT", &child.id().expect("pid").to_string()])
        .status()
        .await
        .expect("interrupt broker");
    assert!(stopped.success());
    assert!(timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("shutdown deadline")
        .expect("wait")
        .success());
    assert!(
        !std::path::Path::new(socket).exists(),
        "normal shutdown removes the socket"
    );
}
