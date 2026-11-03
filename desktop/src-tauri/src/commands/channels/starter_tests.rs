//! Starter-channel creation must report every channel it created, even when a
//! later step fails, so the frontend can route that channel's roster reads to
//! the writer.
use super::*;
use axum::{routing::post, Json, Router};
use nostr::Keys;
use serde_json::json;

#[tokio::test]
async fn created_starter_channels_are_reported_when_metadata_is_unavailable() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    // The relay accepts every create, but its replica never serves metadata.
    let router = Router::new()
        .route("/query", post(|| async { Json(json!([])) }))
        .route(
            "/events",
            post(|| async { Json(json!({"event_id": "0", "accepted": true, "message": ""})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let state = crate::app_state::build_app_state();
    *state.keys.lock().unwrap() = Keys::generate();
    *state.relay_url_override.lock().unwrap() = Some(url);

    let mut changed = Vec::new();
    let result = ensure_starter_channels_inner(&state, &mut changed).await;
    server.abort();

    assert_eq!(
        result.err().as_deref(),
        Some("starter channels created but metadata not yet available")
    );
    let scope = relay_api_base_url_with_override(&state);
    let expected: Vec<String> = STARTER_CHANNELS
        .iter()
        .map(|spec| starter_channel_uuid(&scope, spec.slug).to_string())
        .collect();
    assert_eq!(changed, expected);
}

const LIVE_GENERAL_ID: &str = "0b9f4c1e-6a51-4f7e-9d38-2f4b7f0e3c11";
const LIVE_WELCOME_ID: &str = "5d2a7e90-3c4b-4f18-8e6d-91a0b2c4d5e6";

/// A kind:39000 open stream, relay-signed like production.
fn channel_event(id: &str, name: &str) -> serde_json::Value {
    let tags = vec![
        nostr::Tag::parse(["d", id]).unwrap(),
        nostr::Tag::parse(["name", name]).unwrap(),
        nostr::Tag::parse(["t", "stream"]).unwrap(),
        nostr::Tag::parse(["visibility", "open"]).unwrap(),
    ];
    let event = nostr::EventBuilder::new(nostr::Kind::from_u16(39000), "")
        .tags(tags)
        .sign_with_keys(&Keys::generate())
        .unwrap();
    serde_json::to_value(&event).unwrap()
}

/// Runs starter setup against a fake relay whose directory lists a live
/// `general` this user has not joined. The relay refuses every channel create
/// with `create_refusal`: HTTP 400 for a rejected event, as the relay sends
/// it, or HTTP 200 with `accepted: false` for a duplicate. With
/// `renamed_starter`, a live channel under that name holds `welcome-everyone`'s
/// deterministic id, and is listed and served like any live channel. With
/// `writer_only_welcome`, a live `welcome-everyone` under another id is listed
/// only by reads pinned to the writer, as when a replica lags. Returns the
/// result, the kinds of the events the relay accepted, and whether the
/// deterministic id is still marked as pending-owned.
async fn ensure_with_unusable_welcome(
    create_refusal: (axum::http::StatusCode, &'static str),
    renamed_starter: Option<&'static str>,
    writer_only_welcome: bool,
) -> (Result<Vec<ChannelInfo>, String>, Vec<u64>, bool) {
    use std::sync::{Arc, Mutex};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let state = crate::app_state::build_app_state();
    let keys = Keys::generate();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() =
        Some(format!("http://{}", listener.local_addr().unwrap()));
    let starter_id = starter_channel_uuid(
        &relay_api_base_url_with_override(&state),
        "welcome-everyone",
    )
    .to_string();

    let general = channel_event(LIVE_GENERAL_ID, "general");
    let welcome = channel_event(LIVE_WELCOME_ID, "welcome-everyone");
    let renamed = renamed_starter.map(|name| channel_event(&starter_id, name));
    let accepted = Arc::new(Mutex::new(Vec::new()));
    let accepted_kinds = accepted.clone();
    let router = Router::new()
        .route(
            "/query",
            post(move |Json(filters): Json<serde_json::Value>| {
                let filter = filters[0].clone();
                let events = match (filter["kinds"][0].as_u64(), filter.get("#d")) {
                    // The open directory lists live channels only. A lagging
                    // replica misses the late `welcome-everyone`; the writer
                    // has it.
                    (Some(39000), None) => {
                        let from_writer = filter["consistency"] == "strong";
                        std::iter::once(general.clone())
                            .chain(renamed.clone())
                            .chain((writer_only_welcome && from_writer).then(|| welcome.clone()))
                            .collect()
                    }
                    // Metadata by id.
                    (Some(39000), Some(ids)) => renamed
                        .iter()
                        .filter(|event| ids.as_array().unwrap().contains(&event["tags"][0][1]))
                        .cloned()
                        .collect(),
                    _ => Vec::new(),
                };
                async move { Json(json!(events)) }
            }),
        )
        .route(
            "/events",
            post(move |Json(event): Json<serde_json::Value>| {
                let kind = event["kind"].as_u64().unwrap();
                let (status, message) = create_refusal;
                let response = if kind != 9007 {
                    accepted.lock().unwrap().push(kind);
                    (
                        status_ok(),
                        json!({"event_id": "0", "accepted": true, "message": ""}),
                    )
                } else if status == status_ok() {
                    (
                        status,
                        json!({"event_id": "0", "accepted": false, "message": message}),
                    )
                } else {
                    (status, json!({"error": message}))
                };
                async move { (response.0, Json(response.1)) }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let mut changed = Vec::new();
    let result = ensure_starter_channels_inner(&state, &mut changed).await;
    server.abort();
    let kinds = accepted_kinds.lock().unwrap().clone();
    let still_pending = state.is_pending_owned_channel(&keys.public_key().to_hex(), &starter_id);
    (result, kinds, still_pending)
}

fn status_ok() -> axum::http::StatusCode {
    axum::http::StatusCode::OK
}

fn joined_live_general(channels: &[ChannelInfo]) -> bool {
    channels
        .iter()
        .any(|channel| channel.id == LIVE_GENERAL_ID && channel.is_member)
}

#[tokio::test]
async fn a_starter_name_held_by_another_channel_is_skipped() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let (result, accepted, _) = ensure_with_unusable_welcome(
        (
            axum::http::StatusCode::BAD_REQUEST,
            "invalid: an open channel named \"welcome-everyone\" already exists",
        ),
        None,
        false,
    )
    .await;

    let channels = result.expect("a taken starter name must not fail setup");
    assert!(joined_live_general(&channels));
    assert_eq!(accepted, vec![9021], "only the join to general");
}

#[tokio::test]
async fn a_starter_id_held_by_an_archived_channel_is_skipped() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    // The relay refuses any event aimed at an archived channel before its
    // duplicate check.
    let (result, accepted, _) = ensure_with_unusable_welcome(
        (
            axum::http::StatusCode::BAD_REQUEST,
            "invalid: channel is archived",
        ),
        None,
        false,
    )
    .await;

    let channels = result.expect("an archived starter must not fail setup");
    assert!(joined_live_general(&channels));
    assert_eq!(accepted, vec![9021], "only the join to general");
}

#[tokio::test]
async fn a_starter_id_held_by_a_renamed_channel_is_skipped() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let (result, accepted, still_pending) = ensure_with_unusable_welcome(
        (status_ok(), "duplicate: channel already exists"),
        Some("intros"),
        false,
    )
    .await;

    let channels = result.expect("a renamed starter must not fail setup");
    assert!(joined_live_general(&channels));
    assert_eq!(accepted, vec![9021], "only the join to general");
    assert!(
        !still_pending,
        "the renamed channel must not be shown as one this user owns"
    );
}

#[tokio::test]
async fn a_starter_id_held_by_an_unseen_channel_is_skipped() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    // A deleted channel keeps its id, so the relay answers `duplicate`, but
    // never serves its metadata.
    let (result, accepted, still_pending) = ensure_with_unusable_welcome(
        (status_ok(), "duplicate: channel already exists"),
        None,
        false,
    )
    .await;

    let channels = result.expect("a deleted starter must not fail setup");
    assert!(joined_live_general(&channels));
    assert_eq!(accepted, vec![9021], "only the join to general");
    assert!(
        !still_pending,
        "the deleted channel must not be shown as one this user owns"
    );
}

#[tokio::test]
async fn other_create_refusals_still_fail_setup() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let (result, accepted, _) = ensure_with_unusable_welcome(
        (
            axum::http::StatusCode::BAD_REQUEST,
            "invalid: channel name too long",
        ),
        None,
        false,
    )
    .await;

    assert_eq!(
        result.err().as_deref(),
        Some("relay returned 400 Bad Request: invalid: channel name too long")
    );
    assert!(accepted.is_empty());
}

#[tokio::test]
async fn a_starter_name_held_by_a_live_starter_a_replica_missed_is_joined() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let (result, accepted, _) = ensure_with_unusable_welcome(
        (
            axum::http::StatusCode::BAD_REQUEST,
            "invalid: an open channel named \"welcome-everyone\" already exists",
        ),
        None,
        true,
    )
    .await;

    let channels = result.expect("a live starter must not fail setup");
    assert!(joined_live_general(&channels));
    assert!(
        channels
            .iter()
            .any(|channel| channel.id == LIVE_WELCOME_ID && channel.is_member),
        "the live welcome-everyone must be joined"
    );
    assert_eq!(
        accepted,
        vec![9021, 9021],
        "joins to general and welcome-everyone"
    );
}
