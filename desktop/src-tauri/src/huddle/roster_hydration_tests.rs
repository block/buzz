//! Huddle roster hydration through the post-connect and hot-start entrypoints,
//! with the production relay reads, when the managed-agent records cannot be
//! read. The voice-model and audio steps are replaced.
use super::*;
use axum::{extract::State as AxumState, http::StatusCode, routing::post, Json, Router};
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};

use crate::app_state::build_app_state;

const CHANNEL: &str = "4b1d1f0a-6a7e-4c47-9a49-2f3c55c0b0a1";

struct Relay {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Serves one kind:39002 roster and fails every kind:30177 record read.
async fn relay(roster: nostr::Event) -> Relay {
    let router =
        Router::new()
            .route(
                "/query",
                post(
                    |AxumState(roster): AxumState<nostr::Event>,
                     Json(filters): Json<Vec<Value>>| async move {
                        if filters.iter().any(|f| f["kinds"] == json!([30177])) {
                            return (
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(json!({"error": "records unavailable"})),
                            );
                        }
                        (StatusCode::OK, Json(json!([roster])))
                    },
                ),
            )
            .with_state(roster);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Relay { url, task }
}

/// An active Huddle with no known agents, whose relay roster has the viewer,
/// a `bot` agent and an ordinary member.
async fn huddle() -> (Relay, AppState, u64, String, String) {
    let relay_keys = Keys::generate();
    let viewer = Keys::generate();
    let bot = Keys::generate().public_key().to_hex();
    let member = Keys::generate().public_key().to_hex();
    let viewer_hex = viewer.public_key().to_hex();
    let tags = [
        vec!["d", CHANNEL],
        vec!["p", &viewer_hex, "", "owner"],
        vec!["p", &bot, "", "bot"],
        vec!["p", &member, "", "member"],
    ];
    let roster = EventBuilder::new(Kind::from_u16(39002), "")
        .tags(tags.into_iter().map(|tag| Tag::parse(tag).unwrap()))
        .sign_with_keys(&relay_keys)
        .unwrap();
    let relay = relay(roster).await;
    let state = build_app_state();
    *state.keys.lock().unwrap() = viewer;
    *state.relay_url_override.lock().unwrap() = Some(relay.url.clone());
    let generation = {
        let mut hs = state.huddle().unwrap();
        let generation = hs.begin_huddle_lifetime();
        hs.phase = HuddlePhase::Active;
        hs.ephemeral_channel_id = Some(CHANNEL.to_owned());
        // An explicit choice keeps the refresh from starting transcription.
        hs.set_transcription_enabled_by_user(false);
        generation
    };
    (relay, state, generation, bot, member)
}

fn agents(state: &AppState) -> Vec<String> {
    state
        .huddle()
        .unwrap()
        .agent_pubkeys
        .lock()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn post_connect_keeps_bot_agents_when_records_are_unreadable() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let (_relay, state, generation, bot, _member) = huddle().await;

    let mut media_started = false;
    let outcome = post_connect_setup(&state, CHANNEL, generation, async |state, _, _| {
        // Media setup reads the agents, so they must be stored first.
        assert_eq!(agents(state), std::slice::from_ref(&bot));
        media_started = true;
        Ok(PostConnectOutcome::Ready)
    })
    .await
    .unwrap();
    assert!(matches!(outcome, PostConnectOutcome::Ready));
    assert!(media_started);
    assert_eq!(agents(&state), [bot]);
    assert_eq!(state.huddle().unwrap().participants.len(), 3);

    // The same incomplete roster must not decide that the Huddle can end.
    let roster = fetch_huddle_roster(CHANNEL, &state).await.unwrap();
    assert!(!roster.complete);
    assert_eq!(roster.humans_remaining(), None);
}

#[tokio::test]
async fn hotstart_refresh_keeps_known_agents_when_records_are_unreadable() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    let departed = Keys::generate().public_key().to_hex();
    let (_relay, state, _generation, bot, member) = huddle().await;
    // `member` was proven an agent by an earlier record read.
    *state.huddle().unwrap().agent_pubkeys.lock().unwrap() = vec![member.clone(), departed];

    pipeline_hotstart(&state, async |_, _| Ok(()))
        .await
        .unwrap();
    assert_eq!(agents(&state), [member, bot]);
    assert!(state.huddle().unwrap().last_agent_refresh.is_some());
}
