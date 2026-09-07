use super::*;
use crate::acp::{AcpError, StopReason};
use buzz_sdk::reminders::{build, Content, Status};
use nostr::Keys;
use std::collections::{HashMap, HashSet};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn event(keys: &Keys, id: &str, due: u64, created: u64) -> Event {
    build(
        keys,
        id,
        &Content {
            status: Status::Pending,
            note: Some("Inspect the durable experiment results".into()),
            target: None,
            extra: Default::default(),
        },
        Some(due),
        created,
    )
    .unwrap()
    .sign_with_keys(keys)
    .unwrap()
}

fn state(directory: &std::path::Path, keys: &Keys) -> Reminders {
    Reminders {
        receipts: Receipts::open(directory, "https://relay.test", &keys.public_key().to_hex())
            .unwrap(),
        pending: vec![],
        in_flight: None,
        retries: HashMap::new(),
        delivered: HashSet::new(),
    }
}

#[test]
fn old_created_due_reminder_recovers_after_restart_and_delivered_version_does_not() {
    let directory = tempfile::tempdir().unwrap();
    let keys = Keys::generate();
    let reminder = Reminder::decrypt(&event(&keys, "work", 2, 1), &keys).unwrap();
    let mut first = state(directory.path(), &keys);
    first.refresh(vec![reminder.clone()]);
    assert!(first.next().unwrap().is_some());
    first.started("interrupted-turn".into(), reminder.clone());
    drop(first);

    let mut restarted = state(directory.path(), &keys);
    restarted.refresh(vec![reminder.clone()]);
    assert!(restarted.next().unwrap().is_some());
    restarted.started("successful-turn".into(), reminder.clone());
    restarted.finished("successful-turn", &PromptOutcome::Ok(StopReason::EndTurn));
    assert!(restarted.next().unwrap().is_none());
    assert_eq!(restarted.pending[0].content.status, Status::Pending);
    drop(restarted);

    let mut again = state(directory.path(), &keys);
    again.refresh(vec![reminder.clone()]);
    assert!(again.next().unwrap().is_none());
    let snoozed = Reminder::decrypt(&event(&keys, "work", 3, 2), &keys).unwrap();
    again.refresh(vec![snoozed]);
    assert!(again.next().unwrap().is_some());
}

#[test]
fn failed_cancelled_and_limited_turns_back_off_without_starving_other_work() {
    let directory = tempfile::tempdir().unwrap();
    let keys = Keys::generate();
    let reminder = Reminder::decrypt(&event(&keys, "work", 2, 1), &keys).unwrap();
    let other = Reminder::decrypt(&event(&keys, "other", 2, 1), &keys).unwrap();
    let mut state = state(directory.path(), &keys);
    state.refresh(vec![reminder.clone(), other.clone()]);
    for outcome in [
        PromptOutcome::Error(AcpError::Protocol("offline".into())),
        PromptOutcome::Cancelled,
        PromptOutcome::Ok(StopReason::MaxTokens),
        PromptOutcome::Ok(StopReason::Cancelled),
    ] {
        state.started("turn".into(), reminder.clone());
        state.finished("turn", &outcome);
        assert!(!state.receipts.contains(&reminder).unwrap());
        assert_eq!(state.next().unwrap().unwrap().id, other.id);
    }
    state.started("panicked".into(), other.clone());
    state.recover_missing_turn(std::iter::empty());
    assert!(state.in_flight.is_none());
    assert!(!state.receipts.contains(&other).unwrap());
}

#[test]
fn future_snoozed_and_cancelled_heads_supersede_queued_intent() {
    let directory = tempfile::tempdir().unwrap();
    let keys = Keys::generate();
    let mut state = state(directory.path(), &keys);
    let old = event(&keys, "id", 2, 1);
    state.refresh(current_heads(vec![old.clone()], &keys));
    assert!(state.next().unwrap().is_some());
    let future = event(&keys, "id", Timestamp::now().as_secs() + 86_400, 2);
    state.refresh(current_heads(vec![old, future.clone()], &keys));
    assert!(state.next().unwrap().is_none());
    let mut content = Reminder::decrypt(&future, &keys).unwrap().content;
    content.status = Status::Cancelled;
    let cancelled = build(&keys, "id", &content, None, 3)
        .unwrap()
        .sign_with_keys(&keys)
        .unwrap();
    state.refresh(current_heads(vec![future, cancelled], &keys));
    assert!(state.next().unwrap().is_none());
}

#[test]
fn receipt_lock_and_scope_prevent_local_competing_consumers() {
    let directory = tempfile::tempdir().unwrap();
    let first = Receipts::open(directory.path(), "https://one.test", "author").unwrap();
    assert!(Receipts::open(directory.path(), "https://one.test", "author").is_err());
    assert!(Receipts::open(directory.path(), "https://two.test", "author").is_ok());
    assert!(Receipts::open(directory.path(), "https://one.test", "other").is_ok());
    drop(first);
    assert!(Receipts::open(directory.path(), "https://one.test", "author").is_ok());
}

async fn mock_relay(
    keys: Keys,
    responses: Vec<Value>,
) -> (RestClient, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (end, length) = loop {
                let mut buf = [0u8; 4096];
                let count = socket.read(&mut buf).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buf[..count]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    assert!(header.to_lowercase().contains("authorization: nostr "));
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    break (end + 4, length);
                }
            };
            while bytes.len() < end + length {
                let mut buf = [0u8; 4096];
                let count = socket.read(&mut buf).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buf[..count]);
            }
            requests.push(serde_json::from_slice(&bytes[end..end + length]).unwrap());
            let body = response.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (
        RestClient {
            http: reqwest::Client::new(),
            base_url: format!("http://{address}"),
            keys,
            auth_tag_json: None,
        },
        task,
    )
}

#[tokio::test]
async fn authenticated_recovery_pages_old_created_heads_and_ignores_bad_items() {
    let keys = Keys::generate();
    let recent = event(&keys, "future", Timestamp::now().as_secs() + 86_400, 100);
    let old = event(&keys, "due", 2, 1);
    let first = vec![json!(recent); 1000];
    let (client, server) = mock_relay(
        keys.clone(),
        vec![json!(first), json!([old,{"malformed":true}])],
    )
    .await;
    let heads = fetch_heads(&client, None).await.unwrap();
    assert_eq!(heads.len(), 2);
    assert!(heads
        .iter()
        .any(|r| r.id == "due" && r.is_due(Timestamp::now().as_secs())));
    let requests = server.await.unwrap();
    assert!(requests[0][0].get("since").is_none());
    assert!(requests[0][0].get("until").is_none());
    assert_eq!(
        requests[0][0]["authors"],
        json!([keys.public_key().to_hex()])
    );
    assert_eq!(requests[1][0]["until"], 100);
    assert_eq!(requests[1][0]["before_id"], recent.id.to_hex());
}
