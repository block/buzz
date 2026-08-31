//! Prepared-event CLI contract tests against a validating fake relay.
//!
//! The fake relay is deliberately hostile-by-default (finding H3): every
//! `/query` and `/events` request must carry a verifiable NIP-98
//! `Authorization` header AND an `x-auth-tag` NIP-OA header whose recovered
//! owner matches the fixture's relay-member owner — mirroring
//! `check_relay_membership` in `crates/buzz-relay/src/api/mod.rs` (signature
//! verification via `buzz_sdk::nip_oa::verify_auth_tag` binds the tag to the
//! authenticated agent pubkey; the recovered owner must be the tenant's relay
//! member). The NIP-OA conditions grammar has no channel clause, so "wrong
//! scope" is expressed the way the real relay expresses it: an auth tag whose
//! owner is not admitted for this tenant.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// How the fake relay's `/events` endpoint behaves for this scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublishMode {
    /// Store the first body, then always answer 502: the relay accepted the
    /// event but the success response was lost (stored-first-write ambiguity,
    /// the `duplicate: true` recovery path).
    StoreAndLoseResponse,
    /// Answer 502 WITHOUT storing anything: the write itself was lost
    /// (lost-first-write ambiguity — recovery must republish).
    RejectWithoutStore,
    /// Store and answer 200 (healthy relay).
    Accept,
}

#[derive(Clone)]
struct FakeRelay {
    metadata: Event,
    members: Event,
    thread_events: Vec<Event>,
    /// The only NIP-OA owner admitted by this fake tenant.
    expected_owner: PublicKey,
    /// When set, `/query` answers 401 regardless of credentials.
    reject_queries: Arc<AtomicBool>,
    /// When set, `/query` never answers. Used to cancel a durable reply after
    /// its replay intent is fsynced but before a prepared event exists.
    hang_queries: Arc<AtomicBool>,
    /// When set, id-based `/query` requests answer with this raw JSON value
    /// instead of the genuinely stored event (finding S2-4: a relay that
    /// substitutes a different body under a queried id). Served verbatim so
    /// scenarios can return tampered bytes.
    query_substitute: Arc<Mutex<Option<Value>>>,
    publish_mode: Arc<Mutex<PublishMode>>,
    accepted: Arc<Mutex<Option<Value>>>,
    publish_bodies: Arc<Mutex<Vec<Vec<u8>>>>,
}

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "auth_required", "message": message})),
    )
        .into_response()
}

/// Authenticate the request exactly the way the real relay does:
/// `Authorization: Nostr <base64 kind-27235 event>` proves control of the
/// agent key; `x-auth-tag` must be a valid NIP-OA tag bound to THAT agent
/// pubkey; the recovered owner must be the admitted relay member.
fn authorize(state: &FakeRelay, headers: &HeaderMap) -> Result<PublicKey, Box<Response>> {
    let encoded = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Nostr "))
        .ok_or_else(|| Box::new(unauthorized("missing NIP-98 Authorization")))?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| Box::new(unauthorized("NIP-98 Authorization is not base64")))?;
    let nip98 = Event::from_json(
        std::str::from_utf8(&decoded)
            .map_err(|_| Box::new(unauthorized("NIP-98 event is not UTF-8")))?,
    )
    .map_err(|_| Box::new(unauthorized("NIP-98 event is malformed")))?;
    if nip98.kind != Kind::Custom(27235) {
        return Err(Box::new(unauthorized("NIP-98 event has the wrong kind")));
    }
    nip98
        .verify()
        .map_err(|_| Box::new(unauthorized("NIP-98 signature is invalid")))?;
    let agent_pubkey = nip98.pubkey;

    let tag_json = headers
        .get("x-auth-tag")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| Box::new(unauthorized("missing x-auth-tag header")))?;
    let owner = buzz_sdk::nip_oa::verify_auth_tag(tag_json, &agent_pubkey)
        .map_err(|_| Box::new(unauthorized("x-auth-tag failed NIP-OA verification")))?;
    if owner != state.expected_owner {
        // Mirrors MembershipDecision::Denied in the real relay: the tag is
        // cryptographically valid but its owner is not admitted here.
        return Err(Box::new(
            (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "relay_membership_required",
                    "message": "auth tag owner is not a relay member of this tenant"
                })),
            )
                .into_response(),
        ));
    }
    Ok(agent_pubkey)
}

async fn query(
    State(state): State<FakeRelay>,
    headers: HeaderMap,
    Json(filters): Json<Value>,
) -> Response {
    if state.hang_queries.load(Ordering::SeqCst) {
        std::future::pending::<()>().await;
    }
    if state.reject_queries.load(Ordering::SeqCst) {
        return unauthorized("query rejected by scenario");
    }
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    let filter = filters
        .as_array()
        .and_then(|values| values.first())
        .cloned()
        .unwrap_or(Value::Null);
    let kind = filter
        .get("kinds")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(Value::as_u64);

    let events = match kind {
        Some(39000) => vec![state.metadata],
        Some(39002) => vec![state.members],
        _ => {
            if let Some(substitute) = state
                .query_substitute
                .lock()
                .expect("query-substitute lock")
                .clone()
            {
                // Scenario override: the relay answers the id query with a
                // substituted body, bypassing Event parsing so tampered
                // bytes reach the CLI verbatim.
                return Json(json!([substitute])).into_response();
            }
            let requested = filter
                .get("ids")
                .and_then(Value::as_array)
                .and_then(|values| values.first())
                .and_then(Value::as_str);
            if let Some(event) = state
                .thread_events
                .iter()
                .find(|event| requested.is_some_and(|id| event.id.to_hex() == id))
                .cloned()
            {
                vec![event]
            } else {
                state
                    .accepted
                    .lock()
                    .expect("accepted-event lock")
                    .clone()
                    .and_then(|event| {
                        (event.get("id").and_then(Value::as_str) == requested).then_some(event)
                    })
                    .into_iter()
                    .map(|event| Event::from_json(event.to_string()).expect("stored event fixture"))
                    .collect()
            }
        }
    };
    Json(serde_json::to_value(events).expect("serialize query response")).into_response()
}

async fn publish(State(state): State<FakeRelay>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    let mode = *state.publish_mode.lock().expect("publish-mode lock");
    state
        .publish_bodies
        .lock()
        .expect("publish-body lock")
        .push(body.to_vec());
    match mode {
        PublishMode::RejectWithoutStore => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": "write lost before storage"})),
        )
            .into_response(),
        PublishMode::StoreAndLoseResponse => {
            let event: Value = serde_json::from_slice(&body).expect("signed event body");
            let mut accepted = state.accepted.lock().expect("accepted-event lock");
            if accepted.is_none() {
                *accepted = Some(event);
            }
            // The fake relay accepted the event but the proxy lost the response.
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "response lost after acceptance"})),
            )
                .into_response()
        }
        PublishMode::Accept => {
            let event: Value = serde_json::from_slice(&body).expect("signed event body");
            let mut accepted = state.accepted.lock().expect("accepted-event lock");
            if accepted.is_none() {
                *accepted = Some(event);
            }
            (StatusCode::OK, Json(json!({"accepted": true}))).into_response()
        }
    }
}

fn signed_event(keys: &Keys, kind: u16, content: &str, tags: Vec<Tag>) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign authoritative fixture")
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => {
            assert!(value.is_i64() || value.is_u64(), "floats are not wire-safe");
            value.to_string()
        }
        Value::String(value) => serde_json::to_string(value).expect("canonical string"),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let mut keys: Vec<&String> = values.keys().collect();
            keys.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("canonical key"),
                        canonical_json(&values[key]),
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn membership_revision(channel: &str, members: &Value) -> String {
    let canonical = canonical_json(&json!({
        "version": 1,
        "channelId": channel,
        "members": members,
    }));
    format!("v1:{}", hex::encode(Sha256::digest(canonical.as_bytes())))
}

/// One owner/agent DM fixture with a running validating fake relay.
struct Fixture {
    channel: &'static str,
    /// Kept so callers can mint alternative attestations for the same DM.
    #[allow(dead_code)]
    owner: Keys,
    agent: Keys,
    owner_hex: String,
    agent_hex: String,
    auth_tag: String,
    parent_id: String,
    sibling_id: String,
    relay_url: String,
    state: FakeRelay,
    server: tokio::task::JoinHandle<()>,
    /// RAII guard: dropping it deletes the outbox directory.
    #[allow(dead_code)]
    temp: tempfile::TempDir,
    record: std::path::PathBuf,
    agent_secret: String,
}

impl Fixture {
    async fn start() -> Self {
        let channel = "216209f0-1896-4d63-9e06-4411951562ec";
        let owner = Keys::parse("0202020202020202020202020202020202020202020202020202020202020202")
            .expect("owner keys");
        let agent = Keys::parse("0101010101010101010101010101010101010101010101010101010101010101")
            .expect("agent keys");
        let relay = Keys::generate();
        let owner_hex = owner.public_key().to_hex();
        let agent_hex = agent.public_key().to_hex();
        let auth_tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &agent.public_key(), "")
            .expect("agent owner attestation");

        let metadata = signed_event(
            &relay,
            39000,
            "",
            vec![
                Tag::parse(["d", channel]).expect("d tag"),
                Tag::parse(["private"]).expect("private tag"),
                Tag::parse(["hidden"]).expect("hidden tag"),
                Tag::parse(["closed"]).expect("closed tag"),
                Tag::parse(["t", "dm"]).expect("type tag"),
                Tag::parse(["p", &owner_hex]).expect("owner participant"),
                Tag::parse(["p", &agent_hex]).expect("agent participant"),
            ],
        );
        let members = signed_event(
            &relay,
            39002,
            "",
            vec![
                Tag::parse(["d", channel]).expect("d tag"),
                Tag::parse(["p", &owner_hex, "", "member"]).expect("owner participant"),
                Tag::parse(["p", &agent_hex, "", "member"]).expect("agent membership"),
            ],
        );
        let parent = signed_event(
            &owner,
            9,
            "owner prompt",
            vec![Tag::parse(["h", channel]).expect("channel tag")],
        );
        let parent_id = parent.id.to_hex();
        let sibling = signed_event(
            &owner,
            9,
            "owner follow-up",
            vec![
                Tag::parse(["h", channel]).expect("channel tag"),
                Tag::parse(["e", parent_id.as_str(), "", "root"]).expect("root tag"),
                Tag::parse(["e", parent_id.as_str(), "", "reply"]).expect("reply tag"),
            ],
        );
        let sibling_id = sibling.id.to_hex();
        let state = FakeRelay {
            metadata,
            members,
            thread_events: vec![parent, sibling],
            expected_owner: owner.public_key(),
            reject_queries: Arc::new(AtomicBool::new(false)),
            hang_queries: Arc::new(AtomicBool::new(false)),
            query_substitute: Arc::new(Mutex::new(None)),
            publish_mode: Arc::new(Mutex::new(PublishMode::StoreAndLoseResponse)),
            accepted: Arc::new(Mutex::new(None)),
            publish_bodies: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new()
            .route(
                "/info",
                get(|| async { Json(json!({"name": "fake-buzz"})) }),
            )
            .route("/query", post(query))
            .route("/events", post(publish))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake relay");
        let relay_url = format!("http://{}", listener.local_addr().expect("fake relay addr"));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve fake relay");
        });

        let temp = tempfile::tempdir().expect("temporary outbox");
        #[cfg(unix)]
        std::fs::set_permissions(
            temp.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .expect("secure temporary outbox");
        let record = temp.path().join("reply.json");
        let agent_secret = agent.secret_key().to_secret_hex();

        Self {
            channel,
            owner,
            agent,
            owner_hex,
            agent_hex,
            auth_tag,
            parent_id,
            sibling_id,
            relay_url,
            state,
            server,
            temp,
            record,
            agent_secret,
        }
    }

    fn set_publish_mode(&self, mode: PublishMode) {
        *self.state.publish_mode.lock().expect("publish-mode lock") = mode;
    }

    fn set_query_substitute(&self, substitute: Option<Value>) {
        *self
            .state
            .query_substitute
            .lock()
            .expect("query-substitute lock") = substitute;
    }

    fn set_query_hang(&self, enabled: bool) {
        self.state.hang_queries.store(enabled, Ordering::SeqCst);
    }

    fn record_path(&self) -> &str {
        self.record.to_str().expect("UTF-8 fixture path")
    }

    fn prepare_args(&self, execution_id: &'_ str, reply_to: &'_ str) -> Vec<String> {
        self.prepare_args_with_auth(&self.auth_tag, execution_id, reply_to)
    }

    fn prepare_args_with_auth(
        &self,
        auth_tag: &'_ str,
        execution_id: &'_ str,
        reply_to: &'_ str,
    ) -> Vec<String> {
        [
            "buzz",
            "--relay",
            self.relay_url.as_str(),
            "--private-key",
            self.agent_secret.as_str(),
            "--auth-tag",
            auth_tag,
            "messages",
            "prepare",
            "--channel",
            self.channel,
            "--content",
            "-",
            "--reply-to",
            reply_to,
            "--thread-root",
            self.parent_id.as_str(),
            "--execution-id",
            execution_id,
            "--out",
            self.record_path(),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    fn publish_args(&self) -> Vec<String> {
        [
            "buzz",
            "--relay",
            self.relay_url.as_str(),
            "--private-key",
            self.agent_secret.as_str(),
            "--auth-tag",
            self.auth_tag.as_str(),
            "messages",
            "publish-prepared",
            "--file",
            self.record_path(),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }
}

async fn run_cli(args: &[String], stdin: &[u8]) -> (i32, Vec<u8>, Vec<u8>) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = buzz_cli::run_from_args_with_io(
        args.iter().map(String::as_str),
        stdin,
        &mut stdout,
        &mut stderr,
    )
    .await;
    (code, stdout, stderr)
}

#[tokio::test]
async fn ambiguous_retry_reuses_identical_signed_event() {
    let fixture = Fixture::start().await;
    let execution_id = "11".repeat(32);

    let prepare_args = fixture.prepare_args(&execution_id, &fixture.parent_id.clone());
    assert!(
        !prepare_args.iter().any(|arg| arg == "one stable reply"),
        "private reply content must never enter argv",
    );
    let (prepare, prepare_stdout, prepare_stderr) =
        run_cli(&prepare_args, b"one stable reply").await;
    assert_eq!(
        prepare, 0,
        "prepare must persist one signed event after bounded authoritative reads and before relay publication: {}",
        String::from_utf8_lossy(&prepare_stderr),
    );

    let prepare_result: Value =
        serde_json::from_slice(&prepare_stdout).expect("prepare JSON stdout");
    assert_eq!(prepare_result["prepared"], json!(true));
    assert_eq!(prepare_result["path"], json!(fixture.record_path()));
    let prepared_bytes = std::fs::read(&fixture.record).expect("prepared record");
    let prepared: Value = serde_json::from_slice(&prepared_bytes).expect("prepared record JSON");
    assert_eq!(prepared["executionId"], json!(execution_id));
    let mut expected_members = vec![
        json!({"pubkey": fixture.owner_hex, "role": "member"}),
        json!({"pubkey": fixture.agent_hex, "role": "member"}),
    ];
    expected_members.sort_by(|left, right| {
        left["pubkey"]
            .as_str()
            .expect("pubkey")
            .cmp(right["pubkey"].as_str().expect("pubkey"))
    });
    assert_eq!(prepared["channel"]["members"], json!(expected_members));
    let expected_revision = membership_revision(fixture.channel, &prepared["channel"]["members"]);
    assert_eq!(
        prepared["channel"]["membershipRevision"],
        json!(expected_revision),
        "prepared record membership revision must be independently derived from the authoritative sorted member/role set",
    );
    let prepared_event =
        Event::from_json(prepared["event"].to_string()).expect("complete prepared signed event");
    prepared_event.verify().expect("prepared event signature");
    let signed_revision = prepared_event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|tag| tag.first().map(String::as_str) == Some("buzz_membership_revision"))
        .and_then(|tag| tag.get(1))
        .expect("signed membership revision tag");
    assert_eq!(signed_revision, &expected_revision);
    assert_eq!(prepared_event.content, "one stable reply");
    assert_eq!(
        prepare_result["event_id"],
        json!(prepared_event.id.to_hex()),
    );

    let sibling_parent_args = fixture.prepare_args(&execution_id, &fixture.sibling_id.clone());
    let (sibling_adoption, _sibling_stdout, sibling_stderr) =
        run_cli(&sibling_parent_args, b"one stable reply").await;
    assert_eq!(
        sibling_adoption, 1,
        "a sibling immediate parent must not adopt the existing signed reply",
    );
    let sibling_error: Value =
        serde_json::from_slice(&sibling_stderr).expect("sibling-parent error JSON");
    assert_eq!(sibling_error["error"], json!("user_error"));
    assert!(
        sibling_error["message"]
            .as_str()
            .is_some_and(|message| message.contains("different execution fingerprint")),
        "sibling parent must fail at the adoption fingerprint boundary",
    );

    let different_execution_id = "22".repeat(32);
    let different_execution_args =
        fixture.prepare_args(&different_execution_id, &fixture.parent_id.clone());
    let (cross_execution_adoption, _execution_stdout, execution_stderr) =
        run_cli(&different_execution_args, b"one stable reply").await;
    assert_eq!(
        cross_execution_adoption, 1,
        "a different durable execution must not adopt the existing signed reply",
    );
    let execution_error: Value =
        serde_json::from_slice(&execution_stderr).expect("cross-execution error JSON");
    assert_eq!(execution_error["error"], json!("user_error"));
    assert!(
        execution_error["message"]
            .as_str()
            .is_some_and(|message| message.contains("different execution fingerprint")),
        "execution ID must fail at the adoption fingerprint boundary",
    );

    let publish_args = fixture.publish_args();
    let (first_publish, _first_stdout, first_stderr) = run_cli(&publish_args, b"").await;
    assert_eq!(
        first_publish,
        2,
        "lost acceptance response must surface delivery_unknown without resigning: {}",
        String::from_utf8_lossy(&first_stderr),
    );
    let delivery_unknown: Value =
        serde_json::from_slice(&first_stderr).expect("delivery_unknown JSON stderr");
    assert_eq!(delivery_unknown["error"], json!("delivery_unknown"));
    assert_eq!(delivery_unknown["retryable"], json!(true));
    assert_eq!(
        delivery_unknown["event_id"],
        json!(prepared_event.id.to_hex()),
    );

    let bodies_before_recovery = fixture
        .state
        .publish_bodies
        .lock()
        .expect("publish-body lock")
        .len();
    assert!(
        bodies_before_recovery >= 1,
        "the first publication (including its internal retries) must reach the relay",
    );

    let (recovery, recovery_stdout, recovery_stderr) = run_cli(&publish_args, b"").await;
    assert_eq!(
        recovery,
        0,
        "recovery must find and accept the identical event already stored by the relay: {}",
        String::from_utf8_lossy(&recovery_stderr),
    );
    let recovery_result: Value =
        serde_json::from_slice(&recovery_stdout).expect("recovery JSON stdout");
    assert_eq!(recovery_result["accepted"], json!(true));
    assert_eq!(recovery_result["duplicate"], json!(true));
    assert_eq!(
        recovery_result["event_id"],
        json!(prepared_event.id.to_hex()),
    );

    let bodies = fixture
        .state
        .publish_bodies
        .lock()
        .expect("publish-body lock");
    // Stored-first-write contract: recovery finds the identical stored event
    // via /query and must NOT republish — the body count is unchanged from
    // before the recovery run. This is the explicit assertion the old
    // pairwise-only check could not state: identical retry bytes alone would
    // also pass if recovery had (wrongly) republished the same bytes again.
    assert_eq!(
        bodies.len(),
        bodies_before_recovery,
        "recovery after a stored-but-lost response must not republish",
    );
    assert!(
        bodies.windows(2).all(|pair| pair[0] == pair[1]),
        "every ambiguous retry must reuse identical signed bytes",
    );
    let expected_event_bytes =
        serde_json::to_vec(&prepared["event"]).expect("canonical prepared event bytes");
    assert_eq!(
        bodies[0], expected_event_bytes,
        "publication must use the exact signed event bytes persisted before network I/O",
    );
    let accepted = fixture
        .state
        .accepted
        .lock()
        .expect("accepted-event lock")
        .clone()
        .expect("accepted event");
    let accepted_id = accepted
        .get("id")
        .and_then(Value::as_str)
        .expect("accepted event id");
    let body_id = serde_json::from_slice::<Value>(&bodies[0])
        .expect("published event JSON")
        .get("id")
        .and_then(Value::as_str)
        .expect("published event id")
        .to_owned();
    assert_eq!(accepted_id, body_id);
    assert_eq!(accepted_id, prepared_event.id.to_hex());

    fixture.server.abort();
}

/// Byte-different stored event under the same claimed id (finding S2-4):
/// when recovery's `/query` returns an event that is not byte-identical to
/// the persisted record, the CLI must fail closed with a `manual_review`
/// outcome and publish nothing.
///
/// Two adversary strengths are covered:
/// 1. a tampered copy of the stored event (same claimed id/sig, different
///    content bytes) — caught by signature/id verification in
///    `query_exact_event` before the byte comparison is even reached;
/// 2. a validly signed DIFFERENT event substituted under the queried id —
///    passes signature verification, so the byte comparison
///    (`existing != record.event` → `event_body_mismatch`) is the only line
///    of defense.
#[tokio::test]
async fn recovery_query_body_mismatch_fails_closed_without_publishing() {
    let fixture = Fixture::start().await;
    let execution_id = "44".repeat(32);

    let prepare_args = fixture.prepare_args(&execution_id, &fixture.parent_id.clone());
    let (prepare, _prepare_stdout, prepare_stderr) =
        run_cli(&prepare_args, b"one stable reply").await;
    assert_eq!(
        prepare,
        0,
        "prepare must succeed: {}",
        String::from_utf8_lossy(&prepare_stderr),
    );
    let prepared_bytes = std::fs::read(&fixture.record).expect("prepared record");
    let prepared: Value = serde_json::from_slice(&prepared_bytes).expect("prepared record JSON");
    let prepared_id = prepared["event"]["id"].as_str().expect("prepared event id");

    // First publish: the relay stores the event but the response is lost.
    let publish_args = fixture.publish_args();
    let (first_publish, _first_stdout, _first_stderr) = run_cli(&publish_args, b"").await;
    assert_eq!(
        first_publish, 2,
        "first publish must surface delivery_unknown"
    );
    let bodies_after_first = fixture
        .state
        .publish_bodies
        .lock()
        .expect("publish-body lock")
        .len();
    assert!(
        bodies_after_first >= 1,
        "the first publication (including its internal retries) must reach the relay",
    );

    // Case 1: tampered copy — same claimed id, different content bytes.
    let mut tampered = prepared["event"].clone();
    tampered["content"] = json!("tampered body bytes");
    fixture.set_query_substitute(Some(tampered));
    let (tampered_exit, _tampered_stdout, tampered_stderr) = run_cli(&publish_args, b"").await;
    assert_eq!(
        tampered_exit, 1,
        "a tampered same-id event must fail closed at signature verification",
    );
    let tampered_error: Value =
        serde_json::from_slice(&tampered_stderr).expect("tampered-case error JSON");
    assert_eq!(tampered_error["error"], json!("manual_review"));
    assert_eq!(tampered_error["retryable"], json!(false));

    // Case 2: a validly signed different event substituted under the queried
    // id. It verifies, so only the byte comparison can catch it.
    let substitute_event = signed_event(
        &fixture.agent,
        9,
        "a different validly signed body",
        vec![Tag::parse(["h", fixture.channel]).expect("channel tag")],
    );
    fixture.set_query_substitute(Some(
        serde_json::from_str(&substitute_event.as_json()).expect("substitute event JSON"),
    ));
    let (mismatch_exit, mismatch_stdout, mismatch_stderr) = run_cli(&publish_args, b"").await;
    assert_eq!(
        mismatch_exit,
        1,
        "a byte-different stored event must fail closed: stdout {} stderr {}",
        String::from_utf8_lossy(&mismatch_stdout),
        String::from_utf8_lossy(&mismatch_stderr),
    );
    assert!(
        mismatch_stdout.is_empty(),
        "a body mismatch must not produce a success payload",
    );
    let mismatch_error: Value =
        serde_json::from_slice(&mismatch_stderr).expect("mismatch error JSON");
    assert_eq!(mismatch_error["error"], json!("manual_review"));
    assert_eq!(mismatch_error["reason"], json!("event_body_mismatch"));
    assert_eq!(mismatch_error["retryable"], json!(false));
    assert_eq!(mismatch_error["event_id"], json!(prepared_id));

    // Neither manual-review outcome may have published anything further.
    let bodies = fixture
        .state
        .publish_bodies
        .lock()
        .expect("publish-body lock");
    assert_eq!(
        bodies.len(),
        bodies_after_first,
        "manual_review outcomes must publish nothing beyond the first attempt",
    );

    fixture.server.abort();
}

/// Lost-first-write ambiguity: the relay 502s WITHOUT storing, so recovery's
/// `/query` finds nothing and the CLI must republish the exact persisted
/// bytes. `bodies.len() >= 2` is asserted BEFORE the pairwise byte-equality so
/// the vacuous-window failure mode (a single body making `windows(2)` pass
/// trivially) cannot reappear.
#[tokio::test]
async fn lost_first_write_republishes_identical_persisted_bytes() {
    let fixture = Fixture::start().await;
    let execution_id = "33".repeat(32);

    let prepare_args = fixture.prepare_args(&execution_id, &fixture.parent_id.clone());
    let (prepare, _prepare_stdout, prepare_stderr) =
        run_cli(&prepare_args, b"one stable reply").await;
    assert_eq!(
        prepare,
        0,
        "prepare must succeed: {}",
        String::from_utf8_lossy(&prepare_stderr),
    );
    let prepared_bytes = std::fs::read(&fixture.record).expect("prepared record");
    let prepared: Value = serde_json::from_slice(&prepared_bytes).expect("prepared record JSON");
    let prepared_event =
        Event::from_json(prepared["event"].to_string()).expect("prepared signed event");

    fixture.set_publish_mode(PublishMode::RejectWithoutStore);
    let publish_args = fixture.publish_args();
    let (first_publish, _first_stdout, first_stderr) = run_cli(&publish_args, b"").await;
    assert_eq!(
        first_publish,
        2,
        "a lost write must surface delivery_unknown: {}",
        String::from_utf8_lossy(&first_stderr),
    );
    let delivery_unknown: Value =
        serde_json::from_slice(&first_stderr).expect("delivery_unknown JSON stderr");
    assert_eq!(delivery_unknown["error"], json!("delivery_unknown"));
    assert_eq!(delivery_unknown["retryable"], json!(true));
    assert!(
        fixture
            .state
            .accepted
            .lock()
            .expect("accepted-event lock")
            .is_none(),
        "the lost first write must not have stored anything on the relay",
    );
    let bodies_after_loss = fixture
        .state
        .publish_bodies
        .lock()
        .expect("publish-body lock")
        .len();
    assert!(
        bodies_after_loss >= 1,
        "the lost publication must have reached the relay at least once",
    );

    fixture.set_publish_mode(PublishMode::Accept);
    let (recovery, recovery_stdout, recovery_stderr) = run_cli(&publish_args, b"").await;
    assert_eq!(
        recovery,
        0,
        "recovery must republish when /query finds nothing: {}",
        String::from_utf8_lossy(&recovery_stderr),
    );
    let recovery_result: Value =
        serde_json::from_slice(&recovery_stdout).expect("recovery JSON stdout");
    assert_eq!(recovery_result["accepted"], json!(true));
    assert_eq!(
        recovery_result["duplicate"],
        json!(false),
        "a republish after a lost write is a fresh acceptance, not a duplicate",
    );
    assert_eq!(
        recovery_result["event_id"],
        json!(prepared_event.id.to_hex()),
    );

    let bodies = fixture
        .state
        .publish_bodies
        .lock()
        .expect("publish-body lock");
    // Ordering is load-bearing: length FIRST, so the pairwise comparison below
    // can never pass vacuously on a single (or zero) publication.
    assert!(
        bodies.len() >= 2,
        "recovery must republish — expected at least two publications, got {}",
        bodies.len(),
    );
    let expected_event_bytes =
        serde_json::to_vec(&prepared["event"]).expect("canonical prepared event bytes");
    for (index, body) in bodies.iter().enumerate() {
        assert_eq!(
            body, &expected_event_bytes,
            "publication {index} must reuse the exact signed bytes persisted before network I/O",
        );
    }
    let accepted = fixture
        .state
        .accepted
        .lock()
        .expect("accepted-event lock")
        .clone()
        .expect("republished event stored");
    assert_eq!(
        accepted.get("id").and_then(Value::as_str),
        Some(prepared_event.id.to_hex().as_str()),
    );

    fixture.server.abort();
}

/// The relay refusing the recovery `/query` with 401 must fail closed as
/// manual_review/query_unauthorized — never as a blind republish.
#[tokio::test]
async fn unauthorized_query_fails_closed_without_republishing() {
    let fixture = Fixture::start().await;
    let execution_id = "44".repeat(32);

    let prepare_args = fixture.prepare_args(&execution_id, &fixture.parent_id.clone());
    let (prepare, _stdout, prepare_stderr) = run_cli(&prepare_args, b"one stable reply").await;
    assert_eq!(
        prepare,
        0,
        "prepare must succeed before the relay turns hostile: {}",
        String::from_utf8_lossy(&prepare_stderr),
    );
    let prepared_bytes = std::fs::read(&fixture.record).expect("prepared record");
    let prepared: Value = serde_json::from_slice(&prepared_bytes).expect("prepared record JSON");
    let prepared_event =
        Event::from_json(prepared["event"].to_string()).expect("prepared signed event");

    fixture.state.reject_queries.store(true, Ordering::SeqCst);
    let publish_args = fixture.publish_args();
    let (publish, _publish_stdout, publish_stderr) = run_cli(&publish_args, b"").await;
    assert_eq!(
        publish,
        1,
        "a 401 on the recovery query must fail closed: {}",
        String::from_utf8_lossy(&publish_stderr),
    );
    let error: Value = serde_json::from_slice(&publish_stderr).expect("error JSON stderr");
    assert_eq!(error["error"], json!("manual_review"));
    assert_eq!(error["retryable"], json!(false));
    assert_eq!(error["reason"], json!("query_unauthorized"));
    assert_eq!(error["event_id"], json!(prepared_event.id.to_hex()));
    assert!(
        fixture
            .state
            .publish_bodies
            .lock()
            .expect("publish-body lock")
            .is_empty(),
        "an unauthorized query must never fall through to a publish",
    );

    // Prepare itself must also fail closed when the relay 401s its
    // authoritative reads.
    let blocked_prepare_args = fixture.prepare_args(&"55".repeat(32), &fixture.parent_id.clone());
    let (blocked_prepare, _stdout, blocked_stderr) =
        run_cli(&blocked_prepare_args, b"another reply").await;
    assert_eq!(
        blocked_prepare, 1,
        "prepare must fail closed on a 401 query"
    );
    let prepare_error: Value =
        serde_json::from_slice(&blocked_stderr).expect("prepare error JSON stderr");
    assert_eq!(prepare_error["error"], json!("user_error"));
    assert_eq!(prepare_error["retryable"], json!(false));
    assert!(
        prepare_error["message"]
            .as_str()
            .is_some_and(|message| message.contains("HTTP 401")),
        "the 401 must surface in the structured error: {prepare_error}",
    );

    fixture.server.abort();
}

/// An auth tag that is cryptographically valid but scoped to the wrong
/// authority must be rejected by the relay and fail the CLI closed. The
/// NIP-OA conditions grammar carries no channel clause, so scope is what the
/// real relay enforces: the recovered owner must be the admitted relay
/// member for this tenant (`check_relay_membership`).
#[tokio::test]
async fn foreign_owner_auth_tag_fails_closed() {
    let fixture = Fixture::start().await;

    // Valid NIP-OA tag, bound to the right agent, signed by the WRONG owner.
    let stranger = Keys::generate();
    let foreign_tag =
        buzz_sdk::nip_oa::compute_auth_tag(&stranger, &fixture.agent.public_key(), "")
            .expect("foreign owner attestation");
    let args =
        fixture.prepare_args_with_auth(&foreign_tag, &"66".repeat(32), &fixture.parent_id.clone());
    let (code, _stdout, stderr) = run_cli(&args, b"one stable reply").await;
    assert_eq!(
        code,
        1,
        "a foreign-owner auth tag must fail closed: {}",
        String::from_utf8_lossy(&stderr),
    );
    let error: Value = serde_json::from_slice(&stderr).expect("error JSON stderr");
    assert_eq!(error["error"], json!("user_error"));
    assert_eq!(error["retryable"], json!(false));
    assert!(
        error["message"]
            .as_str()
            .is_some_and(|message| message.contains("HTTP 403")),
        "the relay's membership denial must surface in the structured error: {error}",
    );
    assert!(
        fixture
            .state
            .publish_bodies
            .lock()
            .expect("publish-body lock")
            .is_empty(),
        "nothing may be published under a rejected auth tag",
    );
    assert!(
        !fixture.record.exists(),
        "no prepared record may be persisted under a rejected auth tag",
    );

    fixture.server.abort();
}

#[tokio::test]
async fn cancelled_reply_intent_replays_end_to_end_before_new_work() {
    let fixture = Fixture::start().await;
    let execution_id = "78".repeat(32);
    let out = fixture
        .temp
        .path()
        .join(format!("buzz-outbox-{execution_id}.json"));
    let intent = fixture
        .temp
        .path()
        .join(format!("buzz-intent-{execution_id}.json"));
    fixture.set_query_hang(true);

    let relay_url = fixture.relay_url.clone();
    let keys = fixture.agent.clone();
    let auth_tag = fixture.auth_tag.clone();
    let channel = fixture.channel.to_string();
    let parent = fixture.parent_id.clone();
    let owner = fixture.owner_hex.clone();
    let task_out = out.clone();
    let task_execution_id = execution_id.clone();
    let task = tokio::spawn(async move {
        buzz_cli::prepare_and_publish_reply(buzz_cli::DurableReplyRequest {
            relay_url: &relay_url,
            keys: &keys,
            auth_tag: &auth_tag,
            channel: &channel,
            content: "reply recovered after graceful shutdown",
            reply_to: &parent,
            thread_root: &parent,
            execution_id: &task_execution_id,
            mentions: &[owner],
            out: &task_out,
        })
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !intent.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("durable response intent was not installed before the network wait");
    task.abort();
    assert!(task
        .await
        .expect_err("durable reply task must be cancelled")
        .is_cancelled());
    assert!(intent.is_file());
    assert!(!out.exists(), "cancel occurred before event preparation");

    fixture.set_query_hang(false);
    fixture.set_publish_mode(PublishMode::Accept);
    let receipt = buzz_cli::replay_prepared_replies(buzz_cli::ReplayPreparedRequest {
        relay_url: &fixture.relay_url,
        keys: &fixture.agent,
        auth_tag: &fixture.auth_tag,
        outbox_dir: fixture.temp.path(),
    })
    .await
    .expect("startup must prepare and publish the exact durable response intent");
    assert_eq!(receipt.reconciled, 1);
    assert!(!intent.exists(), "accepted response intent must be retired");
    assert!(out.is_file(), "startup must leave the signed audit record");
    let accepted = fixture
        .state
        .accepted
        .lock()
        .expect("accepted-event lock")
        .clone()
        .expect("startup replay must publish a response");
    assert_eq!(
        accepted["content"],
        json!("reply recovered after graceful shutdown")
    );

    fixture.server.abort();
}

#[tokio::test]
async fn startup_replay_reconciles_canonical_records_ignores_only_exact_staging_and_blocks_markers()
{
    let fixture = Fixture::start().await;
    let execution_id = "77".repeat(32);
    let prepare_args = fixture.prepare_args(&execution_id, &fixture.parent_id.clone());
    let (prepare, _stdout, stderr) = run_cli(&prepare_args, b"startup replay reply").await;
    assert_eq!(
        prepare,
        0,
        "prepare startup replay fixture: {}",
        String::from_utf8_lossy(&stderr)
    );
    let canonical = fixture
        .temp
        .path()
        .join(format!("buzz-outbox-{execution_id}.json"));
    std::fs::rename(&fixture.record, &canonical).expect("install canonical replay record");
    let staging = fixture
        .temp
        .path()
        .join(format!(".buzz-outbox-{execution_id}.json.12345.tmp"));
    std::fs::write(&staging, b"uncommitted staging bytes").expect("write exact staging fixture");

    fixture.set_publish_mode(PublishMode::Accept);
    let receipt = buzz_cli::replay_prepared_replies(buzz_cli::ReplayPreparedRequest {
        relay_url: &fixture.relay_url,
        keys: &fixture.agent,
        auth_tag: &fixture.auth_tag,
        outbox_dir: fixture.temp.path(),
    })
    .await
    .expect("startup replay must reconcile the canonical record");
    assert_eq!(receipt.reconciled, 1);
    assert!(
        fixture
            .state
            .accepted
            .lock()
            .expect("accepted-event lock")
            .is_some(),
        "startup replay must publish the exact prepared event"
    );

    let marker = fixture
        .temp
        .path()
        .join(format!("buzz-outbox-{execution_id}.json.manual-review"));
    std::fs::write(&marker, b"cancelled_or_panicked\n").expect("write manual-review fixture");
    let blocked = buzz_cli::replay_prepared_replies(buzz_cli::ReplayPreparedRequest {
        relay_url: &fixture.relay_url,
        keys: &fixture.agent,
        auth_tag: &fixture.auth_tag,
        outbox_dir: fixture.temp.path(),
    })
    .await
    .expect_err("manual-review marker must block startup replay");
    assert!(!blocked.retryable());

    fixture.server.abort();
}
