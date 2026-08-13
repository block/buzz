use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[derive(Clone)]
struct FakeRelay {
    metadata: Event,
    members: Event,
    thread_events: Vec<Event>,
    accepted: Arc<Mutex<Option<Value>>>,
    publish_bodies: Arc<Mutex<Vec<Vec<u8>>>>,
}

async fn query(State(state): State<FakeRelay>, Json(filters): Json<Value>) -> Json<Value> {
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
    Json(serde_json::to_value(events).expect("serialize query response"))
}

async fn publish(State(state): State<FakeRelay>, body: Bytes) -> impl IntoResponse {
    let event: Value = serde_json::from_slice(&body).expect("signed event body");
    let mut accepted = state.accepted.lock().expect("accepted-event lock");
    if accepted.is_none() {
        *accepted = Some(event);
    }
    state
        .publish_bodies
        .lock()
        .expect("publish-body lock")
        .push(body.to_vec());

    // The fake relay accepted the event but the proxy lost the success response.
    (
        StatusCode::BAD_GATEWAY,
        Json(json!({"error": "response lost after acceptance"})),
    )
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

#[tokio::test]
async fn ambiguous_retry_reuses_identical_signed_event() {
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
    let execution_id = "11".repeat(32);

    let prepare_args = [
        "buzz",
        "--relay",
        relay_url.as_str(),
        "--private-key",
        agent_secret.as_str(),
        "--auth-tag",
        auth_tag.as_str(),
        "messages",
        "prepare",
        "--channel",
        channel,
        "--content",
        "-",
        "--reply-to",
        parent_id.as_str(),
        "--thread-root",
        parent_id.as_str(),
        "--execution-id",
        execution_id.as_str(),
        "--out",
        record.to_str().expect("UTF-8 fixture path"),
    ];
    assert!(
        !prepare_args.contains(&"one stable reply"),
        "private reply content must never enter argv",
    );
    let mut prepare_stdout = Vec::new();
    let mut prepare_stderr = Vec::new();
    let prepare = buzz_cli::run_from_args_with_io(
        prepare_args,
        b"one stable reply",
        &mut prepare_stdout,
        &mut prepare_stderr,
    )
    .await;
    assert_eq!(
        prepare, 0,
        "prepare must persist one signed event after bounded authoritative reads and before relay publication: {}",
        String::from_utf8_lossy(&prepare_stderr),
    );

    let prepare_result: Value =
        serde_json::from_slice(&prepare_stdout).expect("prepare JSON stdout");
    assert_eq!(prepare_result["prepared"], json!(true));
    assert_eq!(
        prepare_result["path"],
        json!(record.to_str().expect("path"))
    );
    let prepared_bytes = std::fs::read(&record).expect("prepared record");
    let prepared: Value = serde_json::from_slice(&prepared_bytes).expect("prepared record JSON");
    assert_eq!(prepared["executionId"], json!(execution_id));
    let mut expected_members = vec![
        json!({"pubkey": owner_hex, "role": "member"}),
        json!({"pubkey": agent_hex, "role": "member"}),
    ];
    expected_members.sort_by(|left, right| {
        left["pubkey"]
            .as_str()
            .expect("pubkey")
            .cmp(right["pubkey"].as_str().expect("pubkey"))
    });
    assert_eq!(prepared["channel"]["members"], json!(expected_members));
    let expected_revision = membership_revision(channel, &prepared["channel"]["members"]);
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

    let sibling_parent_args = [
        "buzz",
        "--relay",
        relay_url.as_str(),
        "--private-key",
        agent_secret.as_str(),
        "--auth-tag",
        auth_tag.as_str(),
        "messages",
        "prepare",
        "--channel",
        channel,
        "--content",
        "-",
        "--reply-to",
        sibling_id.as_str(),
        "--thread-root",
        parent_id.as_str(),
        "--execution-id",
        execution_id.as_str(),
        "--out",
        record.to_str().expect("UTF-8 fixture path"),
    ];
    let mut sibling_stdout = Vec::new();
    let mut sibling_stderr = Vec::new();
    let sibling_adoption = buzz_cli::run_from_args_with_io(
        sibling_parent_args,
        b"one stable reply",
        &mut sibling_stdout,
        &mut sibling_stderr,
    )
    .await;
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
    let different_execution_args = [
        "buzz",
        "--relay",
        relay_url.as_str(),
        "--private-key",
        agent_secret.as_str(),
        "--auth-tag",
        auth_tag.as_str(),
        "messages",
        "prepare",
        "--channel",
        channel,
        "--content",
        "-",
        "--reply-to",
        parent_id.as_str(),
        "--thread-root",
        parent_id.as_str(),
        "--execution-id",
        different_execution_id.as_str(),
        "--out",
        record.to_str().expect("UTF-8 fixture path"),
    ];
    let mut execution_stdout = Vec::new();
    let mut execution_stderr = Vec::new();
    let cross_execution_adoption = buzz_cli::run_from_args_with_io(
        different_execution_args,
        b"one stable reply",
        &mut execution_stdout,
        &mut execution_stderr,
    )
    .await;
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

    let publish_args = [
        "buzz",
        "--relay",
        relay_url.as_str(),
        "--private-key",
        agent_secret.as_str(),
        "--auth-tag",
        auth_tag.as_str(),
        "messages",
        "publish-prepared",
        "--file",
        record.to_str().expect("UTF-8 fixture path"),
    ];
    let mut first_stdout = Vec::new();
    let mut first_stderr = Vec::new();
    let first_publish =
        buzz_cli::run_from_args_with_io(publish_args, b"", &mut first_stdout, &mut first_stderr)
            .await;
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

    let mut recovery_stdout = Vec::new();
    let mut recovery_stderr = Vec::new();
    let recovery = buzz_cli::run_from_args_with_io(
        publish_args,
        b"",
        &mut recovery_stdout,
        &mut recovery_stderr,
    )
    .await;
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

    let bodies = state.publish_bodies.lock().expect("publish-body lock");
    assert!(
        !bodies.is_empty(),
        "the first publication must reach the relay"
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
    let accepted = state
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

    server.abort();
}
