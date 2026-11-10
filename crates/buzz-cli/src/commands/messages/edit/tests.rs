use super::*;
use axum::{extract::State, routing::post, Json, Router};
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn message(keys: &Keys, channel: Uuid, media: &[Vec<String>]) -> Event {
    EventBuilder::new(Kind::Custom(9), "original")
        .tags(
            std::iter::once(Tag::parse(["h", &channel.to_string()]).unwrap())
                .chain(media.iter().map(|t| Tag::parse(t.clone()).unwrap())),
        )
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() - 20))
        .sign_with_keys(keys)
        .unwrap()
}
fn edit(keys: &Keys, original: &Event, at: u64, extra: &[Vec<String>]) -> Event {
    let channel = original
        .tags
        .iter()
        .find(|t| t.as_slice()[0] == "h")
        .unwrap()
        .content()
        .unwrap();
    EventBuilder::new(Kind::Custom(40003), "edit")
        .tags(
            std::iter::once(Tag::parse(["h", channel]).unwrap())
                .chain(extra.iter().map(|t| Tag::parse(t.clone()).unwrap())),
        )
        .custom_created_at(Timestamp::from(at))
        .sign_with_keys(keys)
        .unwrap()
}
fn imeta(name: &str) -> Vec<String> {
    vec![
        "imeta".into(),
        format!("url https://example.com/{name}"),
        "m image/png".into(),
        "alt original label".into(),
        "blurhash complete metadata".into(),
    ]
}

#[test]
fn latest_uses_server_order_target_and_channel() {
    let keys = Keys::generate();
    let original = message(&keys, Uuid::new_v4(), &[imeta("original")]);
    let target = vec!["e".into(), original.id.to_hex()];
    let now = Timestamp::now().as_secs();
    let older = edit(&keys, &original, now - 1, &[target.clone(), imeta("older")]);
    let cleared = edit(&keys, &original, now, std::slice::from_ref(&target));
    let tied = edit(&keys, &original, now, &[target.clone(), imeta("tie")]);
    let winner = if cleared.id > tied.id {
        &cleared
    } else {
        &tied
    };
    assert_eq!(
        latest_edit(&original, &[older.clone(), cleared.clone(), tied.clone()]).id,
        winner.id
    );
    // First valid e wins; a later matching tag must not select this edit.
    let decoy = edit(
        &keys,
        &original,
        now + 1,
        &[
            vec!["e".into(), "f".repeat(64)],
            target.clone(),
            imeta("decoy"),
        ],
    );
    let wrong_channel_original = message(&keys, Uuid::new_v4(), &[]);
    let wrong_channel = edit(
        &keys,
        &wrong_channel_original,
        now + 2,
        std::slice::from_ref(&target),
    );
    assert_eq!(
        latest_edit(&original, &[older, cleared.clone(), decoy, wrong_channel]).id,
        cleared.id
    );
    let mixed = original
        .id
        .to_hex()
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if i % 2 == 0 {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect::<String>();
    let valid_after_invalid = edit(
        &keys,
        &original,
        now + 3,
        &[vec!["e".into(), "invalid".into()], vec!["e".into(), mixed]],
    );
    assert_eq!(
        latest_edit(&original, std::slice::from_ref(&valid_after_invalid)).id,
        valid_after_invalid.id
    );
    assert_eq!(latest_edit(&original, &[]).id, original.id);
}

#[derive(Clone)]
struct Relay {
    events: Arc<Mutex<Vec<Event>>>,
    submitted: Arc<Mutex<Vec<Event>>>,
    fail_history: bool,
    effective_queries: Arc<std::sync::atomic::AtomicUsize>,
    ignore_extension: Arc<std::sync::atomic::AtomicBool>,
}
async fn relay(events: Vec<Event>, fail_history: bool) -> (String, Relay) {
    let state = Relay {
        events: Arc::new(Mutex::new(events)),
        submitted: Default::default(),
        fail_history,
        effective_queries: Default::default(),
        ignore_extension: Default::default(),
    };
    let app = Router::new()
        .route("/query", post(|State(state): State<Relay>, Json(filters): Json<Vec<serde_json::Value>>| async move {
            let filter = &filters[0];
            if filter.get("effective_edit").is_some() {
                state.effective_queries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            if state.fail_history && filter.get("kinds").is_some() {
                return (axum::http::StatusCode::FORBIDDEN, Json(serde_json::json!({"error":"denied"})));
            }
            if filter.get("effective_edit").is_some() && !state.ignore_extension.load(std::sync::atomic::Ordering::SeqCst) {
                assert_eq!(filter["consistency"], "strong");
                assert_eq!(filter, &serde_json::json!({
                    "kinds":[9,40002,40008,45001,45003], "ids":filter["ids"],
                    "#h":filter["#h"], "effective_edit":1, "consistency":"strong"
                }));
                let events = state.events.lock().unwrap();
                let original = events.iter().find(|event| serde_json::json!(event.id.to_hex()) == filter["ids"][0]).unwrap();
                let latest = events.iter().filter(|event| event.kind.as_u16() == 40003 && buzz_core::media::first_edit_target(event).as_deref() == Some(original.id.as_bytes().as_slice()))
                    .max_by_key(|event| (event.created_at,event.id));
                return (axum::http::StatusCode::OK, Json(serde_json::json!({"effective_edit":1,"original":original,"edit":latest})));
            }
            let events: Vec<Event> = state.events.lock().unwrap().iter().filter(|event| {
                filter["ids"].as_array().is_none_or(|ids| ids.contains(&serde_json::json!(event.id.to_hex())))
                    && filter["kinds"].as_array().is_none_or(|kinds| kinds.contains(&serde_json::json!(event.kind.as_u16())))
            }).cloned().collect();
            (axum::http::StatusCode::OK, Json(serde_json::json!(events)))
        }))
        .route("/events", post(|State(state): State<Relay>, Json(event): Json<Event>| async move {
            event.verify().unwrap();
            state.events.lock().unwrap().push(event.clone());
            state.submitted.lock().unwrap().push(event.clone());
            Json(serde_json::json!({"event_id":event.id.to_hex(), "accepted":true}))
        })).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, state)
}
fn client(url: String, keys: Keys) -> BuzzClient {
    BuzzClient::new(url, crate::credentials::Credentials::from_keys(keys)).unwrap()
}

#[tokio::test]
async fn command_retains_latest_full_set_then_keeps_clear_across_same_second_edits() {
    let keys = Keys::generate();
    let channel = Uuid::new_v4();
    let original = message(&keys, channel, &[imeta("removed"), imeta("kept")]);
    let replacement = edit(
        &keys,
        &original,
        Timestamp::now().as_secs(),
        &[vec!["e".into(), original.id.to_hex()], imeta("kept")],
    );
    let (url, state) = relay(vec![original.clone(), replacement.clone()], false).await;
    let client = client(url, keys);
    for index in 0..4 {
        edit_message(
            &client,
            &original.id.to_hex(),
            &format!("edit {index}"),
            index == 2,
        )
        .await
        .unwrap();
    }
    let submitted = state.submitted.lock().unwrap();
    let mut previous = replacement.created_at;
    for (index, event) in submitted.iter().enumerate() {
        let tags: Vec<_> = event
            .tags
            .iter()
            .filter(|t| t.as_slice()[0] == "imeta")
            .map(|t| t.as_slice().to_vec())
            .collect();
        assert_eq!(
            tags,
            if index < 2 {
                vec![imeta("kept")]
            } else {
                vec![]
            }
        );
        assert!(event.created_at > previous);
        previous = event.created_at;
    }
}

#[tokio::test]
async fn command_aborts_on_unreadable_history_or_future_head() {
    for fail_history in [true, false] {
        let keys = Keys::generate();
        let original = message(&keys, Uuid::new_v4(), &[imeta("kept")]);
        let poisoned = edit(
            &keys,
            &original,
            Timestamp::now().as_secs() + 1000,
            &[vec!["e".into(), original.id.to_hex()]],
        );
        let (url, state) = relay(vec![original.clone(), poisoned], fail_history).await;
        let client = client(url, keys);
        assert!(edit_message(&client, &original.id.to_hex(), "new", false)
            .await
            .is_err());
        assert!(state.submitted.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn command_refuses_an_older_relay_that_ignores_the_extension() {
    for kind in [9, 40002, 40008, 45001, 45003] {
        let keys = Keys::generate();
        let channel = Uuid::new_v4();
        let original = EventBuilder::new(Kind::Custom(kind), "original")
            .tags([
                Tag::parse(["h", &channel.to_string()]).unwrap(),
                Tag::parse(imeta("kept")).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        let (url, state) = relay(vec![original.clone()], false).await;
        state
            .ignore_extension
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let client = client(url, keys);
        let error = edit_message(&client, &original.id.to_hex(), "new", false)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("upgrade the relay"),
            "kind {kind}"
        );
        assert!(state.submitted.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn other_channel_kinds_keep_plain_edits_on_older_relays() {
    for kind in [40003, 7, 45002, 12345] {
        let keys = Keys::generate();
        let channel = Uuid::new_v4();
        let original = EventBuilder::new(Kind::Custom(kind), "original")
            .tags([
                Tag::parse(["h", &channel.to_string()]).unwrap(),
                Tag::parse(imeta("original")).unwrap(),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        let (url, state) = relay(vec![original.clone()], false).await;
        state
            .ignore_extension
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let client = client(url, keys);
        for content in ["first edit", "second edit"] {
            edit_message(&client, &original.id.to_hex(), content, false)
                .await
                .unwrap();
        }
        let error = edit_message(&client, &original.id.to_hex(), "clear", true)
            .await
            .unwrap_err();
        assert!(error
            .to_string()
            .contains(&format!("attachments on kind {kind} are not edit-governed")));
        assert_eq!(
            state
                .effective_queries
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        let submitted = state.submitted.lock().unwrap();
        assert_eq!(submitted.len(), 2, "clear must not publish");
        for (event, content) in submitted.iter().zip(["first edit", "second edit"]) {
            assert_eq!(event.kind.as_u16(), 40003);
            assert_eq!(event.content, content);
            assert_eq!(
                buzz_core::media::first_edit_target(event).as_deref(),
                Some(original.id.as_bytes().as_slice())
            );
            assert!(!event.tags.iter().any(|tag| tag.as_slice()[0] == "imeta"));
        }
    }
}
