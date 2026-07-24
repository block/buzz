use super::*;
use serde_json::json;
use std::collections::VecDeque;
use std::io::Write;
use std::net::TcpListener;
use std::sync::Mutex;

static HTTP_ENV_LOCK: Mutex<()> = Mutex::new(());
static REPLICATION_TEST_STATE: Mutex<()> = Mutex::new(());

#[derive(Debug)]
struct RecordedRequest {
    endpoint: String,
    method: Method,
    path: &'static str,
    capability: Capability,
    payload: Option<Value>,
}

#[derive(Default)]
struct FakeExchange {
    responses: VecDeque<Result<Value, MemoryError>>,
    requests: Vec<RecordedRequest>,
    cancel_after: Option<usize>,
}

impl JsonExchange for FakeExchange {
    fn request(
        &mut self,
        node: &Node<'_>,
        request: JsonRequest<'_>,
        deadline: Instant,
        budget: &mut TransferBudget,
    ) -> Result<Value, MemoryError> {
        check_active(deadline)?;
        self.requests.push(RecordedRequest {
            endpoint: node.endpoint.clone(),
            method: request.method,
            path: request.path,
            capability: request.capability,
            payload: request.payload.cloned(),
        });
        if self.cancel_after == Some(self.requests.len()) {
            SYNC_CANCELLED.store(true, Ordering::SeqCst);
        }
        let value = self
            .responses
            .pop_front()
            .expect("fixture response")
            .inspect(|value| {
                let count = serde_json::to_vec(value).expect("fixture JSON").len();
                budget.add_bytes(count).expect("fixture budget");
            })?;
        Ok(value)
    }
}

#[derive(Default)]
struct PageFloodExchange {
    requests: u64,
}

impl JsonExchange for PageFloodExchange {
    fn request(
        &mut self,
        node: &Node<'_>,
        request: JsonRequest<'_>,
        deadline: Instant,
        _budget: &mut TransferBudget,
    ) -> Result<Value, MemoryError> {
        check_active(deadline)?;
        self.requests += 1;
        match request.path {
            "/replication/readiness" => Ok(ready(node.expected_node_id, 0)),
            "/replication/ack" => {
                let payload = request.payload.expect("ack payload");
                Ok(ack(
                    payload["peer_node_id"].as_str().expect("peer node"),
                    payload["cursor"].as_u64().expect("ack cursor"),
                ))
            }
            "/replication/export" => {
                let cursor = request.payload.expect("export payload")["cursor"]
                    .as_u64()
                    .expect("export cursor");
                let mut value = envelope(cursor, cursor + 1, true);
                value["revisions"] = json!([{"sequence": cursor + 1}]);
                value["objects"] = json!({"sha256:a": {"kind": "entity"}});
                value["contracts"] = json!([{"cursor": cursor + 1}]);
                seal(&mut value);
                Ok(value)
            }
            "/replication/import" => {
                let envelope = &request.payload.expect("import payload")["envelope"];
                Ok(json!({
                    "source_node_id": "node:remote",
                    "accepted": 1,
                    "duplicates": 0,
                    "conflicts": 0,
                    "cursor": envelope["to_cursor"]
                }))
            }
            _ => panic!("unexpected path"),
        }
    }
}

fn node<'a>(endpoint: &str, expected_node_id: &'a str) -> Node<'a> {
    Node {
        endpoint: endpoint.to_string(),
        read_token: "read-token",
        replicate_token: "replicate-token",
        expected_node_id,
    }
}

fn ready(node_id: &str, conflicts: u64) -> Value {
    json!({
        "status": "ready",
        "schema_version": 1,
        "node_id": node_id,
        "revision_count": 4,
        "conflict_count": conflicts,
        "max_page_items": 200,
        "max_envelope_bytes": 2097152,
        "markdown_canonical": true,
        "sqlite_derived": true
    })
}

fn ack(peer: &str, cursor: u64) -> Value {
    json!({"peer_node_id": peer, "cursor": cursor})
}

fn envelope(from: u64, to: u64, has_more: bool) -> Value {
    let mut value = json!({
        "schema_version": 1,
        "source_node_id": "node:remote",
        "from_cursor": from,
        "to_cursor": to,
        "has_more": has_more,
        "revisions": [{"sequence": from + 1}, {"sequence": to}],
        "objects": {
            "sha256:a": {"kind": "entity"},
            "sha256:b": {"kind": "tombstone"}
        },
        "contracts": [{"cursor": from + 1}, {"cursor": to}]
    });
    seal(&mut value);
    value
}

fn seal(value: &mut Value) {
    value
        .as_object_mut()
        .expect("envelope object")
        .remove("envelope_id");
    let bytes = serde_json::to_vec(&canonicalize(value)).expect("canonical fixture");
    value["envelope_id"] = json!(format!("sha256:{}", hex::encode(Sha256::digest(bytes))));
}

fn successful_fixture() -> FakeExchange {
    FakeExchange {
        responses: [
            Ok(ready("node:remote", 0)),
            Ok(ready("node:local", 0)),
            Ok(ack("node:local", 2)),
            Ok(envelope(2, 4, false)),
            Ok(json!({
                "source_node_id": "node:remote",
                "accepted": 1,
                "duplicates": 1,
                "conflicts": 1,
                "cursor": 4
            })),
            Ok(ack("node:local", 4)),
            Ok(ready("node:local", 3)),
        ]
        .into(),
        ..FakeExchange::default()
    }
}

#[test]
fn matches_python_sequence_and_counts_resume_duplicates_conflicts_and_tombstones() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    let local = node("http://127.0.0.1:18006", "node:local");
    let remote = node("http://127.0.0.1:28006", "node:remote");
    let mut exchange = successful_fixture();

    let result = replicate_with_exchange(
        "pull",
        &local,
        &remote,
        Duration::from_secs(2),
        &mut exchange,
    )
    .expect("valid differential fixture");

    assert_eq!(
        exchange
            .requests
            .iter()
            .map(|request| (request.method.clone(), request.path, request.capability))
            .collect::<Vec<_>>(),
        vec![
            (Method::GET, "/replication/readiness", Capability::Read),
            (Method::GET, "/replication/readiness", Capability::Read),
            (Method::POST, "/replication/ack", Capability::Replicate),
            (Method::POST, "/replication/export", Capability::Replicate),
            (Method::POST, "/replication/import", Capability::Replicate),
            (Method::POST, "/replication/ack", Capability::Replicate),
            (Method::GET, "/replication/readiness", Capability::Read),
        ]
    );
    assert_eq!(exchange.requests[0].endpoint, remote.endpoint);
    assert_eq!(exchange.requests[1].endpoint, local.endpoint);
    assert_eq!(
        exchange.requests[2].payload,
        Some(json!({"peer_node_id": "node:local", "cursor": 0}))
    );
    assert_eq!(result.from_cursor, 2);
    assert_eq!(result.to_cursor, 4);
    assert_eq!(result.accepted, 1);
    assert_eq!(result.duplicates, 1);
    assert_eq!(result.conflicts, 1);
    assert_eq!(result.objects, 2);
    assert_eq!(result.tombstones, 1);
    assert_eq!(result.target_conflict_count, 3);
    assert!(!format!("{result:?}").contains("token"));
}

#[test]
fn authentication_and_node_pin_denials_precede_replicate_capability() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    let local = node("http://127.0.0.1:18006", "node:local");
    let remote = node("http://127.0.0.1:28006", "node:remote");
    for first_response in [
        Err(MemoryError::AuthenticationFailed),
        Ok(ready("node:attacker", 0)),
    ] {
        let mut exchange = FakeExchange {
            responses: [first_response].into(),
            ..FakeExchange::default()
        };
        let error = replicate_with_exchange(
            "pull",
            &local,
            &remote,
            Duration::from_secs(2),
            &mut exchange,
        )
        .expect_err("pre-capability denial");
        assert!(matches!(
            error,
            MemoryError::AuthenticationFailed | MemoryError::NodeIdentityMismatch
        ));
        assert_eq!(exchange.requests.len(), 1);
        assert_eq!(exchange.requests[0].capability, Capability::Read);
    }
}

#[test]
fn rejects_repeated_cursors_unknown_fields_excess_objects_and_bad_counters() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    let local = node("http://127.0.0.1:18006", "node:local");
    let remote = node("http://127.0.0.1:28006", "node:remote");
    let mut invalid_envelopes = Vec::new();
    let mut repeated = envelope(2, 4, false);
    repeated["to_cursor"] = json!(2);
    invalid_envelopes.push(repeated);
    let mut unknown = envelope(2, 4, false);
    unknown["unexpected"] = json!(true);
    invalid_envelopes.push(unknown);
    let mut excessive = envelope(2, 4, false);
    excessive["objects"] = Value::Object(
        (0..=MAXIMUM_OBJECTS_PER_PAGE)
            .map(|index| (index.to_string(), json!({"kind": "entity"})))
            .collect(),
    );
    invalid_envelopes.push(excessive);

    for invalid in invalid_envelopes {
        let mut exchange = FakeExchange {
            responses: [
                Ok(ready("node:remote", 0)),
                Ok(ready("node:local", 0)),
                Ok(ack("node:local", 2)),
                Ok(invalid),
            ]
            .into(),
            ..FakeExchange::default()
        };
        assert_eq!(
            replicate_with_exchange(
                "pull",
                &local,
                &remote,
                Duration::from_secs(2),
                &mut exchange
            ),
            Err(MemoryError::InvalidResponse)
        );
    }

    let mut exchange = successful_fixture();
    exchange.responses[4] = Ok(json!({
        "source_node_id": "node:remote",
        "accepted": 2,
        "duplicates": 2,
        "conflicts": 0,
        "cursor": 4
    }));
    assert_eq!(
        replicate_with_exchange(
            "pull",
            &local,
            &remote,
            Duration::from_secs(2),
            &mut exchange
        ),
        Err(MemoryError::InvalidResponse)
    );
}

#[test]
fn rejects_acknowledgement_rollback_and_cancels_between_pages() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    let local = node("http://127.0.0.1:18006", "node:local");
    let remote = node("http://127.0.0.1:28006", "node:remote");
    let mut rollback = successful_fixture();
    rollback.responses[5] = Ok(ack("node:local", 3));
    assert_eq!(
        replicate_with_exchange(
            "pull",
            &local,
            &remote,
            Duration::from_secs(2),
            &mut rollback
        ),
        Err(MemoryError::InvalidResponse)
    );

    SYNC_CANCELLED.store(false, Ordering::SeqCst);
    let mut cancelled = successful_fixture();
    cancelled.cancel_after = Some(4);
    assert_eq!(
        replicate_with_exchange(
            "pull",
            &local,
            &remote,
            Duration::from_secs(2),
            &mut cancelled
        ),
        Err(MemoryError::Cancelled)
    );
    SYNC_CANCELLED.store(false, Ordering::SeqCst);
}

#[test]
fn rejects_more_than_ten_thousand_pages() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    let local = node("http://127.0.0.1:18006", "node:local");
    let remote = node("http://127.0.0.1:28006", "node:remote");
    let mut exchange = PageFloodExchange::default();

    assert_eq!(
        replicate_with_exchange(
            "pull",
            &local,
            &remote,
            Duration::from_secs(10),
            &mut exchange
        )
        .expect_err("page flood must fail closed"),
        MemoryError::ResponseTooLarge
    );
    assert_eq!(exchange.requests, 3 + (MAXIMUM_PAGES * 3));
}

#[test]
fn enforces_total_byte_object_and_deadline_bounds() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    let mut budget = TransferBudget {
        bytes: MAXIMUM_TOTAL_BYTES,
        objects: MAXIMUM_TOTAL_OBJECTS,
    };
    assert_eq!(budget.add_bytes(1), Err(MemoryError::ResponseTooLarge));
    assert_eq!(budget.add_objects(1), Err(MemoryError::ResponseTooLarge));
    assert_eq!(check_active(Instant::now()), Err(MemoryError::Timeout));
}

fn one_response_server(
    status: &str,
    content_type: &str,
    body: &str,
) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind HTTP fixture");
    let endpoint = format!("http://{}", listener.local_addr().expect("fixture address"));
    let status = status.to_string();
    let content_type = content_type.to_string();
    let body = body.to_string();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept HTTP fixture");
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).expect("read HTTP fixture");
            if count == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..count]);
        }
        write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("write HTTP fixture");
        String::from_utf8(request).expect("request UTF-8")
    });
    (endpoint, handle)
}

#[test]
fn http_exchange_ignores_proxy_env_uses_exact_capability_and_rejects_media_and_redirects() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    let _guard = HTTP_ENV_LOCK.lock().expect("lock proxy environment");
    std::env::set_var("HTTP_PROXY", "http://127.0.0.1:1");
    let (endpoint, server) = one_response_server("200 OK", "Application/JSON; Charset=UTF-8", "{}");
    let node = Node {
        endpoint,
        read_token: "read-only-secret",
        replicate_token: "replicate-only-secret",
        expected_node_id: "node:test",
    };
    let mut exchange = HttpJsonExchange::new().expect("build no-proxy client");
    let mut budget = TransferBudget::default();
    exchange
        .request(
            &node,
            JsonRequest {
                method: Method::GET,
                path: "/replication/readiness",
                capability: Capability::Read,
                payload: None,
            },
            Instant::now() + Duration::from_secs(2),
            &mut budget,
        )
        .expect("valid JSON media type");
    let request = server.join().expect("join HTTP fixture");
    std::env::remove_var("HTTP_PROXY");
    assert!(request.contains("authorization: Bearer read-only-secret"));
    assert!(!request.contains("replicate-only-secret"));

    for (status, media, body, expected) in [
        ("200 OK", "text/plain", "{}", MemoryError::InvalidResponse),
        (
            "200 OK",
            "application/json",
            "{",
            MemoryError::InvalidResponse,
        ),
        (
            "302 Found",
            "application/json",
            "{}",
            MemoryError::LocalServiceUnavailable,
        ),
    ] {
        let (endpoint, server) = one_response_server(status, media, body);
        let node = Node {
            endpoint,
            read_token: "read",
            replicate_token: "replicate",
            expected_node_id: "node:test",
        };
        let mut exchange = HttpJsonExchange::new().expect("build client");
        let mut budget = TransferBudget::default();
        assert_eq!(
            exchange.request(
                &node,
                JsonRequest {
                    method: Method::GET,
                    path: "/replication/readiness",
                    capability: Capability::Read,
                    payload: None,
                },
                Instant::now() + Duration::from_secs(2),
                &mut budget
            ),
            Err(expected)
        );
        server.join().expect("join rejection fixture");
    }
}

#[test]
fn cancellation_interrupts_a_streaming_response_within_the_request_slice() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    SYNC_CANCELLED.store(false, Ordering::SeqCst);
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind streaming fixture");
    let endpoint = format!(
        "http://{}",
        listener.local_addr().expect("streaming fixture address")
    );
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept streaming fixture");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).expect("read streaming request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{",
            )
            .expect("write partial streaming response");
        stream.flush().expect("flush partial response");
        std::thread::sleep(Duration::from_millis(750));
    });
    let canceller = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(50));
        SYNC_CANCELLED.store(true, Ordering::SeqCst);
    });
    let node = Node {
        endpoint,
        read_token: "read",
        replicate_token: "replicate",
        expected_node_id: "node:test",
    };
    let mut exchange = HttpJsonExchange::new().expect("build client");
    let mut budget = TransferBudget::default();
    let started = Instant::now();
    let error = exchange
        .request(
            &node,
            JsonRequest {
                method: Method::GET,
                path: "/replication/readiness",
                capability: Capability::Read,
                payload: None,
            },
            Instant::now() + Duration::from_secs(2),
            &mut budget,
        )
        .expect_err("stream cancellation must fail closed");
    canceller.join().expect("join canceller");
    server.join().expect("join streaming fixture");
    SYNC_CANCELLED.store(false, Ordering::SeqCst);

    assert_eq!(error, MemoryError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(2));
}
