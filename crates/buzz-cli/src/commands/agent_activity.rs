//! `buzz agents activity` — read-only view of an owned agent's observer frames.
//!
//! The harness publishes ephemeral kind:24200 telemetry frames, NIP-44
//! encrypted to the owner. The relay never stores them, so this command
//! listens for a bounded window and decrypts what arrives with the signer key.

use std::time::Duration;

use buzz_core::kind::KIND_AGENT_OBSERVER_FRAME;
use buzz_core::observer::{
    decrypt_observer_payload, OBSERVER_AGENT_TAG, OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY,
};
use nostr::{Event, Keys, PublicKey};
use serde_json::{json, Map, Value};

use super::agents::require_agent_owner;
use super::messages::resolve_author;
use crate::client::BuzzClient;
use crate::error::CliError;

/// Frames returned when `--limit` is not given.
pub const DEFAULT_LIMIT: usize = 50;
/// Largest accepted `--limit`.
pub const MAX_LIMIT: usize = 200;
/// Seconds listened when `--wait` is not given.
pub const DEFAULT_WAIT_SECS: u64 = 10;
/// Largest accepted `--wait`.
pub const MAX_WAIT_SECS: u64 = 60;
const DEFAULT_LOOKBACK_SECS: u64 = 300;
const MAX_LOOKBACK_SECS: u64 = 3_600;
const MAX_COLLECTED: usize = 1_000;
const TEXT_EXCERPT_CHARS: usize = 280;
const BATCH_KIND: &str = "batch";

/// Frames to print plus how many received frames could not be read.
#[derive(Debug)]
pub(crate) struct Activity {
    pub frames: Vec<Value>,
    pub unreadable: usize,
}

pub(crate) async fn run(
    client: &BuzzClient,
    agent: &str,
    since: Option<u64>,
    limit: usize,
    wait: u64,
) -> Result<(), CliError> {
    let activity = fetch_activity(client, agent, since, limit, wait, unix_now()).await?;
    if activity.unreadable > 0 {
        eprintln!(
            "{}",
            json!({"warning": format!(
                "{} frame(s) could not be decrypted or parsed and were left out",
                activity.unreadable
            )})
        );
    }
    println!("{}", Value::Array(activity.frames));
    Ok(())
}

pub(crate) async fn fetch_activity(
    client: &BuzzClient,
    agent: &str,
    since: Option<u64>,
    limit: usize,
    wait: u64,
    now: u64,
) -> Result<Activity, CliError> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(CliError::Usage(format!("--limit must be 1-{MAX_LIMIT}")));
    }
    if !(1..=MAX_WAIT_SECS).contains(&wait) {
        return Err(CliError::Usage(format!("--wait must be 1-{MAX_WAIT_SECS}")));
    }
    let since = resolve_since(since, now)?;
    let agent_hex = resolve_author(client, agent).await?;
    require_agent_owner(client, &agent_hex).await?;
    let agent_key = PublicKey::from_hex(&agent_hex)
        .map_err(|e| CliError::Usage(format!("invalid agent pubkey: {e}")))?;

    let filter = json!({
        "kinds": [KIND_AGENT_OBSERVER_FRAME],
        "authors": [agent_hex],
        "#p": [client.keys().public_key().to_hex()],
        "since": since,
        "limit": MAX_COLLECTED,
    });
    let events = client
        .listen_events(&filter, Duration::from_secs(wait), MAX_COLLECTED)
        .await?;
    decode_activity(client.keys(), &agent_key, &events, since, limit)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn resolve_since(since: Option<u64>, now: u64) -> Result<u64, CliError> {
    let Some(since) = since else {
        return Ok(now.saturating_sub(DEFAULT_LOOKBACK_SECS));
    };
    if since > now {
        return Err(CliError::Usage("--since is in the future".into()));
    }
    if now - since > MAX_LOOKBACK_SECS {
        return Err(CliError::Usage(format!(
            "--since is more than {MAX_LOOKBACK_SECS} seconds ago; observer frames are not stored"
        )));
    }
    Ok(since)
}

/// Decrypt and normalize the agent's telemetry frames from `events`, oldest
/// first, keeping the newest `limit`. Events from other agents of the same
/// owner are ignored. Frames that fail verification, decryption, or parsing
/// are counted; when nothing readable remains that is an error, not `[]`.
pub(crate) fn decode_activity(
    keys: &Keys,
    agent: &PublicKey,
    events: &[Event],
    since: u64,
    limit: usize,
) -> Result<Activity, CliError> {
    let agent_hex = agent.to_hex();
    let mut decoded: Vec<((i64, u64), Value)> = Vec::new();
    let mut unreadable = 0;
    for event in events {
        if !is_agent_telemetry(event, &agent_hex) || event.created_at.as_secs() < since {
            continue;
        }
        let payload = event
            .verify()
            .ok()
            .and_then(|()| decrypt_observer_payload::<Value>(keys, event).ok());
        let Some(payload) = payload else {
            unreadable += 1;
            continue;
        };
        for inner in unwrap_batch(&payload) {
            match normalize_frame(event.created_at.as_secs(), inner) {
                Some(frame) => decoded.push(frame),
                None => unreadable += 1,
            }
        }
    }
    if decoded.is_empty() && unreadable > 0 {
        return Err(CliError::Other(format!(
            "{unreadable} frame(s) from the agent could not be decrypted or parsed; \
             is your key the agent's owner key?"
        )));
    }
    decoded.sort_by_key(|(key, _)| *key);
    let skip = decoded.len().saturating_sub(limit);
    Ok(Activity {
        frames: decoded.into_iter().skip(skip).map(|(_, v)| v).collect(),
        unreadable,
    })
}

pub(super) fn is_agent_telemetry(event: &Event, agent_hex: &str) -> bool {
    let tag = |name: &str| {
        event
            .tags
            .iter()
            .find(|t| t.kind().to_string() == name)
            .and_then(|t| t.content())
    };
    event.kind.as_u16() as u32 == KIND_AGENT_OBSERVER_FRAME
        && event.pubkey.to_hex() == agent_hex
        && tag(OBSERVER_AGENT_TAG) == Some(agent_hex)
        && tag(OBSERVER_FRAME_TAG) == Some(OBSERVER_FRAME_TELEMETRY)
}

pub(super) fn unwrap_batch(frame: &Value) -> Vec<&Value> {
    if frame.get("kind").and_then(Value::as_str) != Some(BATCH_KIND) {
        return vec![frame];
    }
    match frame.pointer("/payload/events").and_then(Value::as_array) {
        Some(inner) if !inner.is_empty() => inner.iter().collect(),
        _ => vec![frame],
    }
}

fn normalize_frame(created_at: u64, frame: &Value) -> Option<((i64, u64), Value)> {
    let kind = frame.get("kind")?.as_str()?;
    let seq = frame.get("seq").and_then(Value::as_u64).unwrap_or(0);
    let timestamp = frame.get("timestamp").and_then(Value::as_str);
    let millis = timestamp
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())
        .unwrap_or(created_at as i64 * 1_000);

    let mut out = Map::new();
    out.insert("created_at".into(), created_at.into());
    out.insert(
        "timestamp".into(),
        timestamp.map_or(Value::Null, Value::from),
    );
    out.insert("seq".into(), seq.into());
    out.insert("type".into(), kind.into());
    for (from, to) in [
        ("channelId", "channel_id"),
        ("turnId", "turn_id"),
        ("sessionId", "session_id"),
    ] {
        if let Some(value) = frame.get(from).filter(|v| v.is_string()) {
            out.insert(to.into(), value.clone());
        }
    }
    let empty = Value::Null;
    summarize(kind, frame.get("payload").unwrap_or(&empty), &mut out);
    Some(((millis, seq), Value::Object(out)))
}

fn put(out: &mut Map<String, Value>, key: &str, value: Option<&Value>) {
    if let Some(value) = value.filter(|v| !v.is_null()) {
        out.insert(key.into(), value.clone());
    }
}

fn excerpt(text: &str) -> Value {
    if text.chars().count() <= TEXT_EXCERPT_CHARS {
        return text.into();
    }
    let mut cut: String = text.chars().take(TEXT_EXCERPT_CHARS).collect();
    cut.push('…');
    cut.into()
}

fn block_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(block_text).collect::<Vec<_>>().join("\n"),
        Value::Object(map) => ["text", "content"]
            .iter()
            .filter_map(|key| map.get(*key))
            .map(block_text)
            .find(|text| !text.is_empty())
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Per-type summary fields, matching what the desktop activity views read.
fn summarize(kind: &str, payload: &Value, out: &mut Map<String, Value>) {
    match kind {
        "turn_started" => {
            put(out, "source", payload.get("source"));
            put(
                out,
                "thread_root_event_id",
                payload.get("threadRootEventId"),
            );
            if let Some(ids) = payload.get("triggeringEventIds").and_then(Value::as_array) {
                out.insert("triggering_event_count".into(), ids.len().into());
            }
        }
        "turn_error" | "agent_panic" => {
            put(out, "outcome", payload.get("outcome"));
            put(out, "code", payload.get("code"));
            if let Some(error) = payload.get("error").and_then(Value::as_str) {
                out.insert("error".into(), excerpt(error));
            }
        }
        "control_result" => {
            put(out, "control", payload.get("type"));
            put(out, "status", payload.get("status"));
            put(out, "request_id", payload.get("requestId"));
            put(out, "model_id", payload.get("modelId"));
        }
        "acp_read" | "acp_write" => {
            put(out, "method", payload.get("method"));
            if let Some(reason) = payload.pointer("/result/stopReason") {
                put(out, "stop_reason", Some(reason));
            }
            let Some(update) = payload.pointer("/params/update") else {
                return;
            };
            put(out, "update_type", update.get("sessionUpdate"));
            put(out, "tool_call_id", update.get("toolCallId"));
            put(out, "tool_title", update.get("title"));
            put(out, "tool_status", update.get("status"));
            let text = update.get("content").map(block_text).unwrap_or_default();
            if !text.is_empty() {
                out.insert("text".into(), excerpt(&text));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::{extract::State, response::IntoResponse, routing::get, routing::post, Json, Router};
    use buzz_core::observer::{encrypt_observer_payload, OBSERVER_FRAME_CONTROL};
    use nostr::{EventBuilder, Kind, Timestamp};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    const NOW: u64 = 1_800_000_000;

    #[derive(Default)]
    struct Relay {
        /// Frames sent right after the REQ, stored or live alike.
        frames: Vec<Event>,
        profiles: Vec<Value>,
        reqs: Vec<Value>,
        refuse_req: Option<String>,
    }
    type Shared = Arc<Mutex<Relay>>;

    async fn socket(State(relay): State<Shared>, ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(move |mut ws: WebSocket| async move {
            let _ = ws
                .send(Message::Text(
                    json!(["AUTH", "challenge"]).to_string().into(),
                ))
                .await;
            while let Some(Ok(Message::Text(text))) = ws.recv().await {
                let msg: Vec<Value> = serde_json::from_str(&text).unwrap();
                match msg[0].as_str() {
                    Some("AUTH") => {
                        let ok = json!(["OK", msg[1]["id"], true, ""]);
                        let _ = ws.send(Message::Text(ok.to_string().into())).await;
                    }
                    Some("REQ") => {
                        let (frames, refusal) = {
                            let mut relay = relay.lock().unwrap();
                            relay.reqs.push(msg[2].clone());
                            (relay.frames.clone(), relay.refuse_req.clone())
                        };
                        if let Some(reason) = refusal {
                            let closed = json!(["CLOSED", msg[1], reason]);
                            let _ = ws.send(Message::Text(closed.to_string().into())).await;
                            continue;
                        }
                        for event in frames {
                            let m = json!(["EVENT", msg[1], event]);
                            let _ = ws.send(Message::Text(m.to_string().into())).await;
                        }
                        let eose = json!(["EOSE", msg[1]]);
                        let _ = ws.send(Message::Text(eose.to_string().into())).await;
                    }
                    _ => {}
                }
            }
        })
    }

    async fn query(State(relay): State<Shared>, Json(filters): Json<Vec<Value>>) -> Json<Value> {
        let relay = relay.lock().unwrap();
        let author = &filters[0]["authors"][0];
        let found: Vec<_> = relay
            .profiles
            .iter()
            .filter(|p| &p["pubkey"] == author)
            .cloned()
            .collect();
        Json(json!(found))
    }

    async fn fake_relay(relay: Shared) -> String {
        let app = Router::new()
            .route("/", get(socket))
            .route("/query", post(query))
            .with_state(relay);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn profile(agent: &Keys, owner: &Keys) -> Value {
        let sig = "a".repeat(128);
        let event = EventBuilder::new(Kind::Metadata, "{\"name\":\"Scout\"}")
            .tags([nostr::Tag::parse(["auth", &owner.public_key().to_hex(), "", &sig]).unwrap()])
            .sign_with_keys(agent)
            .unwrap();
        serde_json::to_value(event).unwrap()
    }

    fn frame_event(agent: &Keys, owner: &PublicKey, payload: &Value, created_at: u64) -> Event {
        let encrypted = encrypt_observer_payload(agent, owner, payload).unwrap();
        buzz_sdk::build_agent_observer_frame(
            &owner.to_hex(),
            &agent.public_key().to_hex(),
            OBSERVER_FRAME_TELEMETRY,
            &encrypted,
        )
        .unwrap()
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(agent)
        .unwrap()
    }

    fn frame(seq: u64, kind: &str, payload: Value) -> Value {
        json!({
            "seq": seq,
            "timestamp": chrono::DateTime::from_timestamp(NOW as i64 - 100 + seq as i64, 0)
                .unwrap()
                .to_rfc3339(),
            "kind": kind,
            "agentIndex": 0,
            "channelId": "7c07e659-3610-42f4-9a5e-1e9973c09da9",
            "sessionId": null,
            "turnId": "turn-1",
            "payload": payload,
        })
    }

    async fn setup(owner: &Keys, agent: &Keys) -> (BuzzClient, Shared) {
        let relay = Shared::default();
        relay.lock().unwrap().profiles.push(profile(agent, owner));
        let url = fake_relay(relay.clone()).await;
        let client = BuzzClient::new(url, owner.clone(), None, None).unwrap();
        (client, relay)
    }

    fn seed(relay: &Shared, agent: &Keys, owner: &Keys, frames: &[Value]) {
        let events = frames
            .iter()
            .enumerate()
            .map(|(i, f)| frame_event(agent, &owner.public_key(), f, NOW - 100 + i as u64))
            .collect();
        relay.lock().unwrap().frames = events;
    }

    #[tokio::test]
    async fn owner_reads_normalized_frames_oldest_first_newest_kept() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (client, relay) = setup(&owner, &agent).await;
        let other_agent = Keys::generate();
        let update = |body: Value| json!({"method": "session/update", "params": {"update": body}});
        seed(
            &relay,
            &agent,
            &owner,
            &[
                frame(
                    3,
                    "acp_read",
                    update(
                        json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "hello"}}),
                    ),
                ),
                frame(
                    1,
                    "turn_started",
                    json!({"source": "channel", "triggeringEventIds": ["a", "b"]}),
                ),
                frame(
                    4,
                    "control_result",
                    json!({"type": "switch_model", "status": "sent", "modelId": "m1", "requestId": "r1"}),
                ),
                frame(
                    2,
                    "acp_read",
                    update(
                        json!({"sessionUpdate": "tool_call", "toolCallId": "tc1", "title": "Run", "status": "pending"}),
                    ),
                ),
            ],
        );
        relay.lock().unwrap().frames.push(frame_event(
            &other_agent,
            &owner.public_key(),
            &frame(9, "turn_started", json!({})),
            NOW - 10,
        ));

        let all = fetch_activity(&client, &agent.public_key().to_hex(), None, 50, 1, NOW)
            .await
            .unwrap();
        let types: Vec<_> = all.frames.iter().map(|f| f["type"].clone()).collect();
        assert_eq!(
            types,
            ["turn_started", "acp_read", "acp_read", "control_result"]
        );
        assert_eq!(all.frames[0]["triggering_event_count"], 2);
        assert_eq!(all.frames[0]["source"], "channel");
        assert_eq!(all.frames[1]["tool_call_id"], "tc1");
        assert_eq!(all.frames[1]["tool_status"], "pending");
        assert_eq!(all.frames[2]["text"], "hello");
        assert_eq!(all.frames[2]["update_type"], "agent_message_chunk");
        assert_eq!(all.frames[3]["request_id"], "r1");
        assert_eq!(all.frames[3]["control"], "switch_model");
        assert_eq!(
            all.frames[0]["channel_id"],
            "7c07e659-3610-42f4-9a5e-1e9973c09da9"
        );

        let newest = fetch_activity(&client, &agent.public_key().to_hex(), None, 2, 1, NOW)
            .await
            .unwrap();
        let seqs: Vec<_> = newest.frames.iter().map(|f| f["seq"].clone()).collect();
        assert_eq!(seqs, [3, 4]);

        let req = relay.lock().unwrap().reqs[0].clone();
        assert_eq!(req["kinds"], json!([24_200]));
        assert_eq!(req["#p"], json!([owner.public_key().to_hex()]));
        assert_eq!(req["authors"], json!([agent.public_key().to_hex()]));
        assert_eq!(req["since"], NOW - DEFAULT_LOOKBACK_SECS);
        assert_eq!(req["limit"], MAX_COLLECTED);
    }

    #[tokio::test]
    async fn batch_envelopes_are_expanded_into_their_inner_events() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (client, relay) = setup(&owner, &agent).await;
        let batch = |seq: u64, inner: Vec<Value>| frame(seq, BATCH_KIND, json!({"events": inner}));
        let chunk = frame(
            2,
            "acp_read",
            json!({"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": [{"text": "a"}, {"content": {"text": "b"}}]}}}),
        );
        let events = vec![
            frame_event(
                &agent,
                &owner.public_key(),
                &batch(
                    9,
                    vec![
                        frame(1, "turn_started", json!({"source": "channel"})),
                        chunk,
                    ],
                ),
                NOW - 30,
            ),
            frame_event(
                &agent,
                &owner.public_key(),
                &batch(
                    10,
                    vec![frame(
                        3,
                        "control_result",
                        json!({"type": "cancel_turn", "status": "sent", "requestId": "r1"}),
                    )],
                ),
                NOW - 20,
            ),
            frame_event(&agent, &owner.public_key(), &batch(11, vec![]), NOW - 10),
        ];
        relay.lock().unwrap().frames = events;

        let hex = agent.public_key().to_hex();
        let all = fetch_activity(&client, &hex, None, 50, 1, NOW)
            .await
            .unwrap();
        let shown: Vec<_> = all
            .frames
            .iter()
            .map(|f| {
                (
                    f["type"].as_str().unwrap().to_owned(),
                    f["seq"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("turn_started".to_owned(), 1),
                ("acp_read".to_owned(), 2),
                ("control_result".to_owned(), 3),
                ("batch".to_owned(), 11),
            ]
        );
        assert_eq!(all.frames[1]["text"], "a\nb");
        assert_eq!(all.frames[2]["request_id"], "r1");

        let newest = fetch_activity(&client, &hex, None, 2, 1, NOW)
            .await
            .unwrap();
        let seqs: Vec<_> = newest.frames.iter().map(|f| f["seq"].clone()).collect();
        assert_eq!(seqs, [3, 11]);
    }

    #[tokio::test]
    async fn non_owner_gets_an_auth_error_not_an_empty_list() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (_, relay) = setup(&owner, &agent).await;
        seed(
            &relay,
            &agent,
            &owner,
            &[frame(1, "turn_started", json!({}))],
        );
        let url = fake_relay(relay.clone()).await;
        let stranger = BuzzClient::new(url, Keys::generate(), None, None).unwrap();

        let error = fetch_activity(&stranger, &agent.public_key().to_hex(), None, 50, 1, NOW)
            .await
            .unwrap_err();
        assert!(matches!(error, CliError::Auth(_)), "{error:?}");
        assert_eq!(crate::error::exit_code(&error), 3);
        assert!(relay.lock().unwrap().reqs.is_empty());
    }

    #[tokio::test]
    async fn unreachable_relay_is_a_network_error() {
        let client =
            BuzzClient::new("http://127.0.0.1:1".into(), Keys::generate(), None, None).unwrap();
        let agent = Keys::generate().public_key().to_hex();
        let error = fetch_activity(&client, &agent, None, 50, 1, NOW)
            .await
            .unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 2, "{error:?}");
    }

    #[tokio::test]
    async fn only_the_newest_events_are_kept_under_a_flood() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (client, relay) = setup(&owner, &agent).await;
        seed(
            &relay,
            &agent,
            &owner,
            &(1..=5)
                .map(|seq| frame(seq, "turn_liveness", json!({})))
                .collect::<Vec<_>>(),
        );
        let events = client
            .listen_events(&json!({"kinds": [24_200]}), Duration::from_secs(1), 2)
            .await
            .unwrap();
        let created: Vec<_> = events.iter().map(|e| e.created_at.as_secs()).collect();
        assert_eq!(created, [NOW - 97, NOW - 96]);
    }

    #[tokio::test]
    async fn unknown_agent_is_not_found() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (client, _) = setup(&owner, &agent).await;
        let error = fetch_activity(
            &client,
            &Keys::generate().public_key().to_hex(),
            None,
            50,
            1,
            NOW,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, CliError::NotFound(_)), "{error:?}");
    }

    #[tokio::test]
    async fn undecryptable_frames_are_an_error_or_a_counted_warning() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (client, relay) = setup(&owner, &agent).await;
        let wrong_recipient = Keys::generate();
        let bad = frame_event(
            &agent,
            &wrong_recipient.public_key(),
            &frame(1, "turn_started", json!({})),
            NOW - 50,
        );
        relay.lock().unwrap().frames = vec![bad];
        let error = fetch_activity(&client, &agent.public_key().to_hex(), None, 50, 1, NOW)
            .await
            .unwrap_err();
        assert!(matches!(error, CliError::Other(_)), "{error:?}");

        let good = frame_event(
            &agent,
            &owner.public_key(),
            &frame(2, "turn_completed", json!({})),
            NOW - 40,
        );
        relay.lock().unwrap().frames.push(good);
        let mixed = fetch_activity(&client, &agent.public_key().to_hex(), None, 50, 1, NOW)
            .await
            .unwrap();
        assert_eq!(mixed.frames.len(), 1);
        assert_eq!(mixed.unreadable, 1);
    }

    #[tokio::test]
    async fn relay_refusal_and_empty_window_are_distinct() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (client, relay) = setup(&owner, &agent).await;
        let empty = fetch_activity(&client, &agent.public_key().to_hex(), None, 50, 1, NOW)
            .await
            .unwrap();
        assert!(empty.frames.is_empty());

        relay.lock().unwrap().refuse_req = Some("restricted: not allowed".into());
        let error = fetch_activity(&client, &agent.public_key().to_hex(), None, 50, 1, NOW)
            .await
            .unwrap_err();
        assert!(matches!(error, CliError::Auth(_)), "{error:?}");
    }

    #[tokio::test]
    async fn spoofed_and_tampered_frames_are_never_shown_as_the_agents() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (client, relay) = setup(&owner, &agent).await;
        let impostor = Keys::generate();
        let spoofed_tag = |signer: &Keys, tag_agent: &Keys, frame_tag: &str| {
            let encrypted = encrypt_observer_payload(
                signer,
                &owner.public_key(),
                &frame(7, "turn_started", json!({})),
            )
            .unwrap();
            buzz_sdk::build_agent_observer_frame(
                &owner.public_key().to_hex(),
                &tag_agent.public_key().to_hex(),
                frame_tag,
                &encrypted,
            )
            .unwrap()
            .custom_created_at(Timestamp::from(NOW - 10))
            .sign_with_keys(signer)
            .unwrap()
        };
        let mut tampered = serde_json::to_value(frame_event(
            &agent,
            &owner.public_key(),
            &frame(8, "turn_started", json!({})),
            NOW - 20,
        ))
        .unwrap();
        tampered["created_at"] = json!(NOW - 19);
        let tampered: Event = serde_json::from_value(tampered).unwrap();
        relay.lock().unwrap().frames = vec![
            spoofed_tag(&impostor, &agent, OBSERVER_FRAME_TELEMETRY),
            spoofed_tag(&agent, &impostor, OBSERVER_FRAME_TELEMETRY),
            spoofed_tag(&agent, &agent, OBSERVER_FRAME_CONTROL),
            tampered,
        ];

        let error = fetch_activity(&client, &agent.public_key().to_hex(), None, 50, 1, NOW)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("1 frame(s)"), "{error}");
    }

    #[tokio::test]
    async fn out_of_range_limit_and_wait_are_usage_errors() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let (client, relay) = setup(&owner, &agent).await;
        let hex = agent.public_key().to_hex();
        for (limit, wait) in [(0, 1), (MAX_LIMIT + 1, 1), (1, 0), (1, MAX_WAIT_SECS + 1)] {
            let error = fetch_activity(&client, &hex, None, limit, wait, NOW)
                .await
                .unwrap_err();
            assert!(matches!(error, CliError::Usage(_)), "{limit} {wait}");
        }
        assert!(relay.lock().unwrap().reqs.is_empty());
    }

    #[test]
    fn window_and_limits_are_bounded() {
        assert_eq!(resolve_since(None, NOW).unwrap(), NOW - 300);
        assert_eq!(resolve_since(Some(NOW - 3_600), NOW).unwrap(), NOW - 3_600);
        for bad in [NOW - 3_601, NOW + 1] {
            assert!(matches!(
                resolve_since(Some(bad), NOW),
                Err(CliError::Usage(_))
            ));
        }
    }

    #[test]
    fn frames_before_since_are_dropped_and_text_is_bounded() {
        let (owner, agent) = (Keys::generate(), Keys::generate());
        let long = "é".repeat(TEXT_EXCERPT_CHARS + 50);
        let events = [
            frame_event(
                &agent,
                &owner.public_key(),
                &frame(1, "turn_started", json!({})),
                NOW - 500,
            ),
            frame_event(
                &agent,
                &owner.public_key(),
                &frame(
                    2,
                    "acp_read",
                    json!({"method": "session/update", "params": {"update": {"sessionUpdate": "agent_message_chunk", "content": {"text": long}}}}),
                ),
                NOW - 50,
            ),
        ];
        let activity =
            decode_activity(&owner, &agent.public_key(), &events, NOW - 100, 10).unwrap();
        assert_eq!(activity.frames.len(), 1);
        let text = activity.frames[0]["text"].as_str().unwrap();
        assert_eq!(text.chars().count(), TEXT_EXCERPT_CHARS + 1);
    }
}
