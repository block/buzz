//! In-process relay for the read-state command tests: NIP-11 `/info`, the
//! atomic read-state snapshot, tag-filtered `/query`, and replaceable-event
//! `/events` writes.

use std::sync::{Arc, Mutex};

use axum::{body::Bytes, extract::State, routing::get, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use crate::commands::read_state::tests::{snapshot_descriptor, snapshot_envelope};

#[derive(Default)]
pub(crate) struct Relay {
    pub events: Vec<Value>,
    pub filters: Vec<Value>,
    pub published: Vec<Value>,
}
pub(crate) type Shared = Arc<Mutex<Relay>>;

fn tag_values(filter: &Value, key: &str) -> Vec<String> {
    filter[key]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn has_tag(event: &Value, name: &str, wanted: &[String]) -> bool {
    wanted.is_empty()
        || event["tags"].as_array().is_some_and(|tags| {
            tags.iter().any(|t| {
                t[0].as_str() == Some(name)
                    && wanted.iter().any(|w| t[1].as_str() == Some(w.as_str()))
            })
        })
}

fn matches(event: &Value, filter: &Value) -> bool {
    let kinds: Vec<u64> = filter["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    kinds.contains(&event["kind"].as_u64().unwrap())
        && filter["authors"]
            .as_array()
            .is_none_or(|a| a.iter().any(|x| x == &event["pubkey"]))
        && event["created_at"].as_u64().unwrap() >= filter["since"].as_u64().unwrap_or(0)
        && ["h", "d", "p", "t"]
            .iter()
            .all(|name| has_tag(event, name, &tag_values(filter, &format!("#{name}"))))
}

fn d_tag(event: &Value) -> Option<&str> {
    event["tags"]
        .as_array()?
        .iter()
        .find(|t| t[0].as_str() == Some("d"))?[1]
        .as_str()
}

pub(crate) async fn fake_relay(relay: Shared) -> String {
    let app = Router::new()
        .route(
            "/query",
            post(|State(relay): State<Shared>, body: Bytes| async move {
                let filters: Vec<Value> = serde_json::from_slice(&body).unwrap();
                let [filter] = filters.as_slice() else {
                    panic!("one filter expected")
                };
                let mut relay = relay.lock().unwrap();
                relay.filters.push(filter.clone());
                if filter.get("read_state_snapshot").is_some() {
                    let own: Vec<Value> = relay
                        .events
                        .iter()
                        .filter(|e| e["kind"] == 30078 && filter["authors"] == json!([e["pubkey"]]))
                        .cloned()
                        .collect();
                    let author = filter["authors"][0].as_str().unwrap();
                    return Json(snapshot_envelope(author, own));
                }
                let mut hits: Vec<Value> = relay
                    .events
                    .iter()
                    .filter(|e| matches(e, filter))
                    .cloned()
                    .collect();
                hits.sort_by_key(|e| std::cmp::Reverse(e["created_at"].as_u64().unwrap()));
                hits.truncate(filter["limit"].as_u64().unwrap() as usize);
                Json(json!(hits))
            }),
        )
        .route(
            "/events",
            post(|State(relay): State<Shared>, body: Bytes| async move {
                let event: Value = serde_json::from_slice(&body).unwrap();
                let mut relay = relay.lock().unwrap();
                relay.published.push(event.clone());
                let at = |e: &Value| e["created_at"].as_u64().unwrap();
                let held = relay.events.iter().position(|e| {
                    e["pubkey"] == event["pubkey"]
                        && e["kind"] == event["kind"]
                        && d_tag(e) == d_tag(&event)
                });
                let message = match held {
                    Some(i) if at(&relay.events[i]) >= at(&event) => "duplicate: superseded",
                    Some(i) => {
                        relay.events[i] = event.clone();
                        ""
                    }
                    None => {
                        relay.events.push(event.clone());
                        ""
                    }
                };
                Json(json!({"event_id": event["id"], "accepted": true, "message": message}))
            }),
        )
        .route(
            "/info",
            get(|| async { Json(json!({"read_state_snapshot": snapshot_descriptor()})) }),
        )
        .with_state(relay);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}
