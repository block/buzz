use super::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{mpsc, Mutex};

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
                Ok(page_flood_envelope(cursor))
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

const PYTHON_GOLDEN_ENVELOPE: &str = r#"{"contracts":[{"classification":"OFFICIAL","cursor":"3","entityId":"entity:one","eventId":"replication:sha256:35a2f4658db52c21f35ae16493e00a68167340f729f36a1a70d0205d99c8b128:3","hashes":{"envelope":"sha256:0a6149bb7ab9083f257a94fdec5ca10434ca5bf1c8b917db700dca1a39cbe4e4","payload":"sha256:35a2f4658db52c21f35ae16493e00a68167340f729f36a1a70d0205d99c8b128"},"kind":"replication-envelope","nodeId":"node:remote","parentRevisionIds":["sha256:35a2f4658db52c21f35ae16493e00a68167340f729f36a1a70d0205d99c8b128"],"payload":{"classification":"OFFICIAL","content":{"content":"Café ⚓","large":1e+20,"negative_zero":-0.0,"score":1e-07,"whole":1.0},"cursor":"3","entityId":"entity:one","eventId":"sha256:35a2f4658db52c21f35ae16493e00a68167340f729f36a1a70d0205d99c8b128","hashes":{"content":"sha256:de4d24ed1920a5ecd669170691b07c8bb42510324db2c3fc695070a918320d53","revision":"sha256:35a2f4658db52c21f35ae16493e00a68167340f729f36a1a70d0205d99c8b128"},"kind":"memory-revision","nodeId":"node:remote","parentRevisionIds":[],"timestamp":"2026-07-25T00:00:00Z","tombstone":false,"version":1},"timestamp":"2026-07-25T00:00:00Z","tombstone":false,"version":1},{"classification":"OFFICIAL","cursor":"4","entityId":"entity:one","eventId":"replication:sha256:ff2fb831d5a0505fe1c04254751548ba4e1c520605655ab286a2c15d69396ed6:4","hashes":{"envelope":"sha256:bcd5b02e0869bb2bfdf67f15bc7c120f664028f00d356ac4f770a4c6e2d62641","payload":"sha256:ff2fb831d5a0505fe1c04254751548ba4e1c520605655ab286a2c15d69396ed6"},"kind":"replication-envelope","nodeId":"node:remote","parentRevisionIds":["sha256:ff2fb831d5a0505fe1c04254751548ba4e1c520605655ab286a2c15d69396ed6"],"payload":{"classification":"OFFICIAL","content":null,"cursor":"4","entityId":"entity:one","eventId":"sha256:ff2fb831d5a0505fe1c04254751548ba4e1c520605655ab286a2c15d69396ed6","hashes":{"content":"sha256:26b541201f830607c71ca9327d12a0361775e2dea354371470fa7fed3c3d7b8d","revision":"sha256:ff2fb831d5a0505fe1c04254751548ba4e1c520605655ab286a2c15d69396ed6"},"kind":"memory-revision","nodeId":"node:remote","parentRevisionIds":["sha256:35a2f4658db52c21f35ae16493e00a68167340f729f36a1a70d0205d99c8b128"],"timestamp":"2026-07-25T01:00:00Z","tombstone":true,"version":1},"timestamp":"2026-07-25T01:00:00Z","tombstone":true,"version":1}],"envelope_id":"sha256:bd77f0b2a7fa6b66495cd1845c3ea9859c9b2566b94d04b839ba0247a19b6ec9","from_cursor":2,"has_more":false,"objects":{"sha256:26b541201f830607c71ca9327d12a0361775e2dea354371470fa7fed3c3d7b8d":{"kind":"tombstone","object_id":"sha256:26b541201f830607c71ca9327d12a0361775e2dea354371470fa7fed3c3d7b8d","payload":{"deleted_at":"2026-07-25T01:00:00+00:00","prior_object_id":"sha256:de4d24ed1920a5ecd669170691b07c8bb42510324db2c3fc695070a918320d53","retain_until":"2026-10-23T01:00:00+00:00","target_id":"entity:one","target_type":"entity"}},"sha256:de4d24ed1920a5ecd669170691b07c8bb42510324db2c3fc695070a918320d53":{"kind":"entity","object_id":"sha256:de4d24ed1920a5ecd669170691b07c8bb42510324db2c3fc695070a918320d53","payload":{"content":"Café ⚓","large":1e+20,"negative_zero":-0.0,"score":1e-07,"whole":1.0}}},"revisions":[{"created_at":"2026-07-25T00:00:00Z","node_id":"node:remote","object_id":"sha256:de4d24ed1920a5ecd669170691b07c8bb42510324db2c3fc695070a918320d53","parent_ids":[],"revision_id":"sha256:35a2f4658db52c21f35ae16493e00a68167340f729f36a1a70d0205d99c8b128","sequence":3,"subject_id":"entity:one","subject_type":"entity"},{"created_at":"2026-07-25T01:00:00Z","node_id":"node:remote","object_id":"sha256:26b541201f830607c71ca9327d12a0361775e2dea354371470fa7fed3c3d7b8d","parent_ids":["sha256:35a2f4658db52c21f35ae16493e00a68167340f729f36a1a70d0205d99c8b128"],"revision_id":"sha256:ff2fb831d5a0505fe1c04254751548ba4e1c520605655ab286a2c15d69396ed6","sequence":4,"subject_id":"entity:one","subject_type":"entity"}],"schema_version":1,"source_node_id":"node:remote","to_cursor":4}"#;

fn envelope(_from: u64, _to: u64, _has_more: bool) -> Value {
    serde_json::from_str(PYTHON_GOLDEN_ENVELOPE).expect("Python golden envelope")
}

fn reseal_structural_fixture(value: &mut Value) {
    value
        .as_object_mut()
        .expect("envelope object")
        .remove("envelope_id");
    let bytes = python_canonical_json_bytes(value).expect("canonical fixture");
    value["envelope_id"] = json!(format!("sha256:{}", hex::encode(Sha256::digest(bytes))));
}

fn page_flood_envelope(cursor: u64) -> Value {
    let mut value = envelope(2, 4, false);
    value["from_cursor"] = json!(cursor);
    value["to_cursor"] = json!(cursor + 1);
    value["has_more"] = json!(true);
    value["revisions"]
        .as_array_mut()
        .expect("revisions")
        .truncate(1);
    value["contracts"]
        .as_array_mut()
        .expect("contracts")
        .truncate(1);
    value["revisions"][0]["sequence"] = json!(cursor + 1);
    let revision_id = value["revisions"][0]["revision_id"]
        .as_str()
        .expect("revision id")
        .to_string();
    let object_id = value["revisions"][0]["object_id"]
        .as_str()
        .expect("object id")
        .to_string();
    value["objects"]
        .as_object_mut()
        .expect("objects")
        .retain(|key, _| key == &object_id);
    value["contracts"][0]["cursor"] = json!((cursor + 1).to_string());
    value["contracts"][0]["eventId"] = json!(format!("replication:{revision_id}:{}", cursor + 1));
    value["contracts"][0]["payload"]["cursor"] = json!((cursor + 1).to_string());
    let mut contract_basis = value["contracts"][0].clone();
    contract_basis["hashes"] = json!({"payload": revision_id});
    let contract_bytes =
        python_canonical_json_bytes(&contract_basis).expect("canonical contract fixture");
    value["contracts"][0]["hashes"]["envelope"] = json!(format!(
        "sha256:{}",
        hex::encode(Sha256::digest(contract_bytes))
    ));
    reseal_structural_fixture(&mut value);
    value
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
fn accepts_agent_memory_python_canonical_json_golden_without_self_sealing() {
    let number_vector = json!({
        "content": "Café ⚓",
        "large": 1e20,
        "negative_zero": -0.0,
        "score": 1e-7,
        "whole": 1.0
    });
    let canonical = python_canonical_json_bytes(&number_vector).expect("canonical number vector");
    assert_eq!(
        String::from_utf8(canonical.clone()).expect("canonical UTF-8"),
        r#"{"content":"Café ⚓","large":1e+20,"negative_zero":-0.0,"score":1e-07,"whole":1.0}"#
    );
    assert_eq!(
        format!("sha256:{}", hex::encode(Sha256::digest(canonical))),
        "sha256:a945d2e9f6a46c71cda2af56ccafcde08183bfc01eccc3d7ab5d35450bfd44d4"
    );

    let value: Value =
        serde_json::from_str(PYTHON_GOLDEN_ENVELOPE).expect("Python-produced JSON fixture");
    assert_eq!(
        value["envelope_id"],
        "sha256:bd77f0b2a7fa6b66495cd1845c3ea9859c9b2566b94d04b839ba0247a19b6ec9"
    );
    assert!(valid_envelope_id(
        &value,
        value["envelope_id"].as_str().expect("golden envelope id")
    ));
}

#[test]
fn matches_immutable_cpython_311_float_boundary_golden() {
    // Expected bytes and digest were captured from CPython 3.11
    // json.dumps(..., ensure_ascii=False, allow_nan=False,
    // separators=(",", ":"), sort_keys=True), never from this Rust writer.
    let value = json!({
        "unicode": "Café ⚓",
        "fixed_pos_low": 1e-4,
        "scientific_pos_low": 1e-5,
        "fixed_pos_high": 1e15,
        "scientific_pos_high": 1e16,
        "fixed_neg_low": -1e-4,
        "scientific_neg_low": -1e-5,
        "fixed_neg_high": -1e15,
        "scientific_neg_high": -1e16,
        "one": 1.0,
        "negative_zero": -0.0,
        "min_subnormal": f64::from_bits(1),
        "max_finite": f64::MAX
    });
    let canonical = python_canonical_json_bytes(&value).expect("canonical CPython vector");
    assert_eq!(
        String::from_utf8(canonical.clone()).expect("canonical UTF-8"),
        r#"{"fixed_neg_high":-1000000000000000.0,"fixed_neg_low":-0.0001,"fixed_pos_high":1000000000000000.0,"fixed_pos_low":0.0001,"max_finite":1.7976931348623157e+308,"min_subnormal":5e-324,"negative_zero":-0.0,"one":1.0,"scientific_neg_high":-1e+16,"scientific_neg_low":-1e-05,"scientific_pos_high":1e+16,"scientific_pos_low":1e-05,"unicode":"Café ⚓"}"#
    );
    assert_eq!(
        format!("sha256:{}", hex::encode(Sha256::digest(canonical))),
        "sha256:6b796b744e8a9ae9330d5787983e613e3ca959269341c50f63acb5dc22eae6ae"
    );
}

#[test]
fn rejects_adversarial_envelope_internals_before_target_import() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    let local = node("http://127.0.0.1:18006", "node:local");
    let remote = node("http://127.0.0.1:28006", "node:remote");
    let object_id = "sha256:de4d24ed1920a5ecd669170691b07c8bb42510324db2c3fc695070a918320d53";
    let mut cases = Vec::new();

    let mut duplicate_sequence = envelope(2, 4, false);
    duplicate_sequence["revisions"][1]["sequence"] = json!(3);
    cases.push(duplicate_sequence);

    let mut cursor_gap = envelope(2, 4, false);
    cursor_gap["revisions"][0]["sequence"] = json!(2);
    cases.push(cursor_gap);

    let mut out_of_order = envelope(2, 4, false);
    out_of_order["revisions"]
        .as_array_mut()
        .expect("revisions")
        .swap(0, 1);
    cases.push(out_of_order);

    let mut mismatched_object_key = envelope(2, 4, false);
    let object = mismatched_object_key["objects"]
        .as_object_mut()
        .expect("objects")
        .remove(object_id)
        .expect("golden object");
    mismatched_object_key["objects"]
        .as_object_mut()
        .expect("objects")
        .insert(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            object,
        );
    cases.push(mismatched_object_key);

    let mut bad_object_hash = envelope(2, 4, false);
    bad_object_hash["objects"][object_id]["payload"]["content"] = json!("tampered");
    cases.push(bad_object_hash);

    let mut revision_schema = envelope(2, 4, false);
    revision_schema["revisions"][0]["unexpected"] = json!(true);
    cases.push(revision_schema);

    let mut contract_schema = envelope(2, 4, false);
    contract_schema["contracts"][0]["unexpected"] = json!(true);
    cases.push(contract_schema);

    let mut broken_cross_link = envelope(2, 4, false);
    broken_cross_link["contracts"][0]["payload"]["eventId"] =
        json!("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    cases.push(broken_cross_link);

    for mut invalid in cases {
        reseal_structural_fixture(&mut invalid);
        let mut exchange = successful_fixture();
        exchange.responses[3] = Ok(invalid);
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
        assert!(
            exchange
                .requests
                .iter()
                .all(|request| request.path != "/replication/import"),
            "invalid internals must not reach the target import"
        );
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
fn valid_response_can_take_more_than_half_a_second_within_the_operation_deadline() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind delayed fixture");
    let endpoint = format!(
        "http://{}",
        listener.local_addr().expect("delayed fixture address")
    );
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept delayed fixture");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).expect("read delayed request");
        std::thread::sleep(Duration::from_millis(750));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            )
            .expect("write delayed response");
    });
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
            &mut budget,
        ),
        Ok(json!({}))
    );
    server.join().expect("join delayed fixture");
}

#[tokio::test(flavor = "current_thread")]
async fn async_transport_runs_safely_inside_the_manual_spawn_blocking_boundary() {
    let (endpoint, server) = one_response_server("200 OK", "application/json; charset=utf-8", "{}");
    let request = tokio::task::spawn_blocking(move || {
        let _state = REPLICATION_TEST_STATE
            .lock()
            .expect("lock replication state");
        let node = Node {
            endpoint,
            read_token: "read",
            replicate_token: "replicate",
            expected_node_id: "node:test",
        };
        let mut exchange = HttpJsonExchange::new().expect("build client");
        let mut budget = TransferBudget::default();
        exchange.request(
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
    });
    assert_eq!(
        request.await.expect("join blocking boundary"),
        Ok(json!({}))
    );
    server.join().expect("join blocking boundary fixture");
}

fn assert_cancellation_closes_stalled_exchange(partial_response: Option<&'static [u8]>) {
    SYNC_CANCELLED.store(false, Ordering::SeqCst);
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind stalled fixture");
    let endpoint = format!(
        "http://{}",
        listener.local_addr().expect("stalled fixture address")
    );
    let (ready_sender, ready_receiver) = mpsc::sync_channel(0);
    let (closed_sender, closed_receiver) = mpsc::sync_channel(0);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept stalled fixture");
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let count = stream.read(&mut chunk).expect("read stalled request");
            if count == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..count]);
        }
        if let Some(response) = partial_response {
            stream
                .write_all(response)
                .expect("write partial stalled response");
            stream.flush().expect("flush partial stalled response");
        }
        ready_sender.send(()).expect("signal stalled request");
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("bound stalled socket observation");
        let closed = loop {
            match stream.read(&mut chunk) {
                Ok(0) => break true,
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break false;
                }
                Err(_) => break true,
            }
        };
        closed_sender.send(closed).expect("report socket closure");
    });
    let canceller = std::thread::spawn(move || {
        ready_receiver.recv().expect("wait for stalled request");
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
    let result = exchange.request(
        &node,
        JsonRequest {
            method: Method::GET,
            path: "/replication/readiness",
            capability: Capability::Read,
            payload: None,
        },
        Instant::now() + Duration::from_secs(2),
        &mut budget,
    );
    let elapsed = started.elapsed();
    let socket_closed = closed_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("server must report socket state");
    canceller.join().expect("join stalled canceller");
    server.join().expect("join stalled server");
    SYNC_CANCELLED.store(false, Ordering::SeqCst);

    assert_eq!(result, Err(MemoryError::Cancelled));
    assert!(
        elapsed < Duration::from_millis(500),
        "cancellation must abort stalled socket I/O within the shutdown bound"
    );
    assert!(socket_closed, "cancelled request must close its socket");
}

#[test]
fn cancellation_aborts_a_request_stalled_before_response_headers() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    assert_cancellation_closes_stalled_exchange(None);
}

#[test]
fn cancellation_aborts_a_response_stalled_after_one_body_chunk() {
    let _state = REPLICATION_TEST_STATE
        .lock()
        .expect("lock replication state");
    assert_cancellation_closes_stalled_exchange(Some(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{",
    ));
}

#[test]
fn cancellation_interrupts_a_streaming_response_between_chunks() {
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
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(100));
            if stream.write_all(b" ").is_err() {
                break;
            }
            let _ = stream.flush();
        }
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
    assert!(started.elapsed() < Duration::from_secs(1));
}
