//! Exercise actual clap parsing, dispatch, signing and HTTP requests. The remote
//! fixture returns known signed states; relay state-machine behavior is tested
//! separately through the DB's acceptance seam.
use super::*;
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use nostr::Keys;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

#[derive(Parser)]
struct Command {
    #[command(subcommand)]
    cmd: InteractionsCmd,
}

struct Bridge {
    relay: Keys,
    enabled: AtomicBool,
    closed: AtomicBool,
    foreign_state: AtomicBool,
    posted: Mutex<Vec<Event>>,
    filters: Mutex<Vec<Value>>,
}

async fn info(State(s): State<Arc<Bridge>>) -> Json<Value> {
    Json(
        json!({"self":s.relay.public_key().to_hex(),"supported_extensions":if s.enabled.load(Ordering::SeqCst) {vec![EXTENSION]} else {vec![]}}),
    )
}

async fn submit(State(s): State<Arc<Bridge>>, Json(event): Json<Event>) -> Json<Value> {
    event.verify().unwrap();
    let id = event.id.to_hex();
    if event.kind.as_u16() as u32 == KIND_INTERACTION_RESPONSE {
        assert_eq!(
            buzz_core::interaction::single_tag(&event, "choice").unwrap(),
            Some("approve")
        );
        s.closed.store(true, Ordering::SeqCst);
    }
    s.posted.lock().unwrap().push(event);
    Json(json!({"event_id":id,"accepted":true,"message":""}))
}

async fn query(State(s): State<Arc<Bridge>>, Json(filters): Json<Vec<Value>>) -> Json<Vec<Event>> {
    assert_eq!(filters.len(), 1);
    let filter = filters[0].clone();
    s.filters.lock().unwrap().push(filter.clone());
    let posted = s.posted.lock().unwrap();
    let Some(prompt) = posted
        .iter()
        .find(|e| e.kind.as_u16() as u32 == KIND_INTERACTION_PROMPT)
    else {
        return Json(vec![]);
    };
    if filter["kinds"][0] == KIND_INTERACTION_PROMPT {
        assert_eq!(filter["ids"][0], prompt.id.to_hex());
        return Json(vec![prompt.clone()]);
    }
    assert_eq!(filter["kinds"][0], KIND_INTERACTION_STATE);
    assert_eq!(filter["authors"][0], s.relay.public_key().to_hex());
    assert_eq!(filter["#d"][0], prompt.id.to_hex());
    let channel = buzz_core::interaction::channel(prompt).unwrap().to_string();
    assert_eq!(filter["#h"][0], channel);
    let closed = s.closed.load(Ordering::SeqCst);
    let foreign = Keys::generate();
    let event = EventBuilder::new(Kind::Custom(KIND_INTERACTION_STATE as u16),json!({"version":1,"revision":if closed {1} else {0},"status":if closed {"closed"} else {"open"},"close_reason":if closed {Some("first")} else {None},"winner":if closed {Some("approve")} else {None},"tally":{"approve":if closed {1} else {0},"deny":0},"responders":[]}).to_string())
        .tags([pair("d",prompt.id).unwrap(),pair("h",channel).unwrap()])
        .sign_with_keys(if s.foreign_state.load(Ordering::SeqCst) { &foreign } else { &s.relay }).unwrap();
    Json(vec![event])
}

#[tokio::test]
async fn interactions_dispatch_round_trip_and_capability_failure() {
    let state = Arc::new(Bridge {
        relay: Keys::generate(),
        enabled: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        foreign_state: AtomicBool::new(false),
        posted: Mutex::new(vec![]),
        filters: Mutex::new(vec![]),
    });
    let app = Router::new()
        .route("/info", get(info))
        .route("/events", post(submit))
        .route("/query", post(query))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = BuzzClient::new(url, Keys::generate(), None, None).unwrap();
    let channel = uuid::Uuid::new_v4().to_string();
    let ask = || {
        Command::try_parse_from([
            "interactions",
            "ask",
            "--channel",
            &channel,
            "--text",
            "Render E001?",
            "--option",
            "approve:Approve:primary",
            "--option",
            "deny:Deny:danger",
            "--expires",
            "1h",
        ])
        .unwrap()
        .cmd
    };
    assert!(dispatch(ask(), &client).await.is_err());
    assert!(state.posted.lock().unwrap().is_empty());
    state.enabled.store(true, Ordering::SeqCst);
    dispatch(ask(), &client).await.unwrap();
    let prompt = state.posted.lock().unwrap()[0].clone();
    assert_eq!(prompt.pubkey, client.keys().public_key());
    assert_eq!(Prompt::parse(&prompt).unwrap().options.len(), 2);
    let id = prompt.id.to_hex();
    let answer = Command::try_parse_from([
        "interactions",
        "answer",
        "--prompt",
        &id,
        "--choice",
        "approve",
        "--comment",
        "Ready",
    ])
    .unwrap()
    .cmd;
    dispatch(answer, &client).await.unwrap();
    let response = state.posted.lock().unwrap()[1].clone();
    assert_eq!(response.pubkey, client.keys().public_key());
    assert_eq!(response.content, "Ready");
    assert_eq!(
        buzz_core::interaction::prompt_id(&response).unwrap(),
        prompt.id
    );
    dispatch(InteractionsCmd::Get { prompt: id.clone() }, &client)
        .await
        .unwrap();
    dispatch(
        InteractionsCmd::Wait {
            prompt: id.clone(),
            timeout: "1s".into(),
        },
        &client,
    )
    .await
    .unwrap();
    state.foreign_state.store(true, Ordering::SeqCst);
    assert!(
        dispatch(InteractionsCmd::Get { prompt: id.clone() }, &client)
            .await
            .is_err()
    );
    state.foreign_state.store(false, Ordering::SeqCst);
    state.closed.store(false, Ordering::SeqCst);
    assert!(matches!(
        dispatch(
            InteractionsCmd::Wait {
                prompt: id,
                timeout: "1s".into()
            },
            &client
        )
        .await,
        Err(CliError::Other(_))
    ));
    server.abort();
}
