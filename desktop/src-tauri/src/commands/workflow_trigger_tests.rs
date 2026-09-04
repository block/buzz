use super::*;
use nostr::JsonUtil;
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    time::Duration,
};

// This HTTP peer models the already-tested relay event-ID dedupe contract.
// It records verified signed payloads, not command invocation counts. No DB or
// Redis is needed to exercise Desktop's production GET/sign/submit boundary.
#[tokio::test]
async fn response_loss_replays_exact_signed_event_and_distinct_run_is_explicit() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state = crate::app_state::build_app_state();
    let keys = nostr::Keys::generate();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    let owner = keys.public_key().to_hex();
    let id = uuid::Uuid::new_v4().to_string();
    let server = std::thread::spawn(move || {
        let mut runs = HashMap::new();
        let mut posts = Vec::new();
        let mut effects = 0;
        for _ in 0..6 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut one = [0; 1];
            while !bytes.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut one).unwrap();
                bytes.push(one[0]);
            }
            let headers = String::from_utf8(bytes).unwrap();
            assert!(headers.to_lowercase().contains("authorization: nostr "));
            let body = if headers.starts_with("GET ") {
                format!(r#"{{"id":"{}"}}"#, "ab".repeat(32))
            } else {
                let len: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut payload = vec![0; len];
                socket.read_exact(&mut payload).unwrap();
                let event = Event::from_json(&payload).unwrap();
                event.verify().unwrap();
                let next_run = format!("run-{}", runs.len() + 1);
                let run = runs.entry(event.id).or_insert_with(|| {
                    effects += 1;
                    next_run
                });
                posts.push(payload);
                if posts.len() == 1 {
                    // Commit first, then truncate the successful response body.
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{",
                        )
                        .unwrap();
                    continue;
                }
                serde_json::json!({"accepted":true,"event_id":event.id.to_hex(),"message":serde_json::json!({"run_id":run}).to_string()}).to_string()
            };
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
        (posts, runs.len(), effects)
    });
    let event = prepare_trigger(&id, &base, &owner, &state).await.unwrap();
    assert!(submit_trigger(id.clone(), &event, &base, &owner, &state)
        .await
        .is_err());
    let retry = submit_trigger(id.clone(), &event, &base, &owner, &state)
        .await
        .unwrap();
    let replay = submit_trigger(id.clone(), &event, &base, &owner, &state)
        .await
        .unwrap();
    assert_eq!(retry, replay);
    let next = prepare_trigger(&id, &base, &owner, &state).await.unwrap();
    assert_ne!(event.id, next.id);
    let distinct = submit_trigger(id, &next, &base, &owner, &state)
        .await
        .unwrap();
    assert_ne!(retry.run_id, distinct.run_id);
    let (posts, runs, effects) = server.join().unwrap();
    assert_eq!(
        posts[0], posts[1],
        "retry must preserve ID, payload AND signature"
    );
    assert_eq!(posts[1], posts[2]);
    assert_ne!(posts[2], posts[3]);
    assert_eq!(
        (runs, effects),
        (2, 2),
        "one effect per intentional logical run"
    );
}

#[tokio::test]
async fn prepare_keeps_scope_across_get_and_submission_refuses_switched_scope() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(crate::app_state::build_app_state());
    let keys = nostr::Keys::generate();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    let owner = keys.public_key().to_hex();
    let other_keys = nostr::Keys::generate();
    let switched = Arc::clone(&state);
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut one = [0; 1];
        while !bytes.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut one).unwrap();
            bytes.push(one[0]);
        }
        // Switch after the GET was sent but before signing its revision.
        *switched.keys.lock().unwrap() = other_keys;
        *switched.relay_url_override.lock().unwrap() = Some("http://127.0.0.1:9".into());
        let body = format!(r#"{{"id":"{}"}}"#, "ab".repeat(32));
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });
    let id = uuid::Uuid::new_v4().to_string();
    let event = prepare_trigger(&id, &base, &owner, &state).await.unwrap();
    server.join().unwrap();
    assert_eq!(event.pubkey, keys.public_key());
    assert!(submit_trigger(id.clone(), &event, &base, &owner, &state)
        .await
        .unwrap_err()
        .contains("community changed"));
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    assert!(submit_trigger(id.clone(), &event, &base, &owner, &state)
        .await
        .unwrap_err()
        .contains("identity changed"));
    assert!(prepare_trigger(&id, &base, &owner, &state)
        .await
        .unwrap_err()
        .contains("identity changed"));
    assert!(prepare_trigger(&id, "", &owner, &state).await.is_err());
}

#[tokio::test]
async fn revision_and_submit_rejections_propagate_without_a_false_run_ack() {
    let _serial = crate::relay_admission::TEST_SERIAL.lock().await;
    crate::relay_admission::reset_rate_limit_gate();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let state = crate::app_state::build_app_state();
    let keys = nostr::Keys::generate();
    *state.keys.lock().unwrap() = keys.clone();
    *state.relay_url_override.lock().unwrap() = Some(base.clone());
    let owner = keys.public_key().to_hex();
    let server = std::thread::spawn(move || {
        let mut posts = Vec::new();
        for attempt in 0..4 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut one = [0; 1];
            while !bytes.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut one).unwrap();
                bytes.push(one[0]);
            }
            let headers = String::from_utf8(bytes).unwrap();
            let (status, body) = if attempt == 0 {
                (
                    "403 Forbidden",
                    "{\"error\":\"revision unavailable\"}".to_string(),
                )
            } else if attempt == 1 {
                ("200 OK", format!(r#"{{"id":"{}"}}"#, "ab".repeat(32)))
            } else {
                let len: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut payload = vec![0; len];
                socket.read_exact(&mut payload).unwrap();
                let event = Event::from_json(&payload).unwrap();
                posts.push(payload);
                ("200 OK", serde_json::json!({"event_id":event.id.to_hex(),"accepted":attempt == 3,"message": if attempt == 2 { "stale revision".to_string() } else { "{\"run_id\":\"run-1\"}".to_string() }}).to_string())
            };
            write!(
                socket,
                "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                body.len(),
                body
            )
            .unwrap();
        }
        assert_eq!(posts[0], posts[1]);
    });
    let id = uuid::Uuid::new_v4().to_string();
    assert!(prepare_trigger(&id, &base, &owner, &state)
        .await
        .unwrap_err()
        .contains("revision unavailable"));
    let event = prepare_trigger(&id, &base, &owner, &state).await.unwrap();
    assert!(submit_trigger(id.clone(), &event, &base, &owner, &state)
        .await
        .unwrap_err()
        .contains("stale revision"));
    assert_eq!(
        submit_trigger(id, &event, &base, &owner, &state)
            .await
            .unwrap()
            .run_id,
        "run-1"
    );
    server.join().unwrap();
}
