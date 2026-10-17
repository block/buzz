//! Real hook -> native ack watcher -> production queue handler regressions.
use super::*;
use crate::pool::{AgentPool, ControlSignal, SteerAck, SteerError, TaskMeta};
use crate::queue::{BatchEvent, EventQueue, QueuedEvent};
use crate::scope::SessionScope;
use nostr::{EventBuilder, Keys, Kind, Tag};
use std::collections::HashSet;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

async fn notice_server() -> (
    crate::relay::RestClient,
    mpsc::UnboundedReceiver<nostr::Event>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rest = crate::relay::RestClient {
        http: reqwest::Client::new(),
        base_url: format!("http://{}", listener.local_addr().unwrap()),
        keys: Keys::generate(),
        auth_tag_json: None,
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(socket);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            assert_eq!(line, "POST /events HTTP/1.1\r\n");
            let mut length = None;
            for _ in 0..64 {
                line.clear();
                assert_ne!(reader.read_line(&mut line).await.unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = Some(value.trim().parse::<usize>().unwrap());
                }
            }
            let size = length.expect("bounded notice body");
            assert!(size < 65536);
            let mut body = vec![0; size];
            reader.read_exact(&mut body).await.unwrap();
            reader.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await.unwrap();
            tx.send(serde_json::from_slice(&body).unwrap()).unwrap();
        }
    });
    (rest, rx, server)
}

async fn rejected_steer_notice(hook_action: &str, threaded: bool) {
    let root = tempfile::tempdir().unwrap();
    let capture = root.path().join("wire.jsonl");
    let complete = root.path().join("complete");
    let hook = root.path().join("message.sh");
    std::fs::write(
        &hook,
        format!("input=$(cat)\ncase \"$input\" in *REJECT_FOLLOWUP*) {hook_action};; esac\n"),
    )
    .unwrap();
    // Keep the original prompt alive until the test explicitly permits its
    // normal completion. Capture every request, including any forbidden steer,
    // cancellation or later re-delivery. No model or real adapter is involved.
    let mut client = AcpClient::spawn(
        "/bin/sh",
        &[
            "-c".into(),
            r#"
        n=0
        while IFS= read -r line; do
            printf '%s\n' "$line" >> "$CAPTURE"
            while [ ! -f "$COMPLETE" ]; do sleep 0.01; done
            printf '{"jsonrpc":"2.0","id":%s,"result":{"stopReason":"end_turn"}}\n' "$n"
            n=$((n+1))
        done
    "#
            .into(),
        ],
        &[
            ("CAPTURE".into(), capture.to_string_lossy().into_owned()),
            ("COMPLETE".into(), complete.to_string_lossy().into_owned()),
        ],
        false,
    )
    .await
    .unwrap();
    client.steering_supported = true;
    client.set_persona_message_hook(
        Some(hook.to_string_lossy().into_owned()),
        root.path().to_string_lossy().into_owned(),
    );
    let (steer_tx, steer_rx) = mpsc::channel(1);
    client.install_steer_rx(steer_rx);
    let (control_tx, mut control_rx) = oneshot::channel::<ControlSignal>();
    let (done_tx, done_rx) = oneshot::channel();
    let mut pool = AgentPool::from_slots(vec![None]);
    let task = pool.join_set.spawn(async move {
        let result = tokio::select! {
            result = client.session_prompt_blocks_with_idle_timeout("fixture", &["original task"], Duration::from_secs(20), Duration::from_secs(30)) => result,
            signal = &mut control_rx => panic!("rejection must not interrupt the original task: {signal:?}"),
        };
        done_tx.send((client, result)).ok().unwrap();
    });
    let channel_id = uuid::Uuid::new_v4();
    let scope = SessionScope::Conversation { channel_id };
    pool.task_map_mut().insert(
        task.id(),
        TaskMeta {
            agent_index: 0,
            channel_id: Some(channel_id),
            scope: Some(scope.clone()),
            turn_id: "original-turn".into(),
            recoverable_batch: None,
            control_tx: Some(control_tx),
            steer_tx: Some(steer_tx),
            successful_steer_deliveries: HashSet::new(),
        },
    );
    let mut queue = EventQueue::new(crate::config::DedupMode::Queue);
    let keys = Keys::generate();
    let root_id = "aa".repeat(32);
    let parent_id = "bb".repeat(32);
    let tags = if threaded {
        vec![
            Tag::parse(["e", &root_id, "", "root"]).unwrap(),
            Tag::parse(["e", &parent_id, "", "reply"]).unwrap(),
        ]
    } else {
        vec![]
    };
    let make_event = |content| {
        EventBuilder::new(Kind::Custom(9), content)
            .tags(tags.clone())
            .sign_with_keys(&keys)
            .unwrap()
    };
    let push = |queue: &mut EventQueue, event: nostr::Event| {
        assert!(queue.push(QueuedEvent {
            channel_id,
            scope: scope.clone(),
            event,
            received_at: std::time::Instant::now(),
            prompt_tag: "mention".into(),
            edit: None
        }));
    };
    let original_tags = if threaded {
        vec![
            Tag::parse(["e", &root_id, "", "root"]).unwrap(),
            Tag::parse(["e", &"cc".repeat(32), "", "reply"]).unwrap(),
        ]
    } else {
        vec![]
    };
    push(
        &mut queue,
        EventBuilder::new(Kind::Custom(9), "original task")
            .tags(original_tags)
            .sign_with_keys(&keys)
            .unwrap(),
    );
    let original = queue.flush_next().unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !capture.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let followup = make_event("REJECT_FOLLOWUP");
    push(&mut queue, followup.clone());
    let (ack_tx, mut ack_rx) = mpsc::unbounded_channel();
    assert!(crate::try_native_steer(
        &mut pool,
        &mut queue,
        scope.clone(),
        BatchEvent {
            event: followup,
            received_at: std::time::Instant::now(),
            prompt_tag: "mention".into(),
            edit: None,
        },
        &ack_tx
    ));
    let ack = tokio::time::timeout(Duration::from_secs(10), ack_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        ack.ack,
        Ok(SteerAck::Err(SteerError::PersonaHookRejected))
    ));
    let (rest, mut notices, server) = notice_server().await;
    crate::steering_ack::handle_steer_ack(&mut pool, &mut queue, ack, 30, Some(&rest));
    let notice = tokio::time::timeout(Duration::from_secs(3), notices.recv())
        .await
        .unwrap()
        .unwrap();
    notice.verify().unwrap();
    assert_eq!(notice.pubkey, rest.keys.public_key());
    assert_eq!(notice.kind, Kind::Custom(9));
    assert_eq!(notice.content, "The persona’s before-message script failed or timed out, so this message was not forwarded as a prompt or steering update. It remains in Buzz history. Review the script before sending again.");
    let channels: Vec<_> = notice
        .tags
        .iter()
        .filter(|t| t.as_slice()[0] == "h")
        .map(|t| t.as_slice()[1].clone())
        .collect();
    assert_eq!(channels, vec![channel_id.to_string()]);
    let routing = crate::queue::parse_thread_tags(&notice);
    assert_eq!(routing.root_event_id, threaded.then_some(root_id));
    assert_eq!(routing.parent_event_id, threaded.then_some(parent_id));
    assert!(!task.is_finished(), "original task must still be running");
    assert!(
        pool.task_map()[&task.id()].control_tx.is_some(),
        "no cancel/merge signal"
    );
    assert!(pool.task_map()[&task.id()]
        .successful_steer_deliveries
        .is_empty());
    assert!(queue.has_in_flight());
    assert!(queue.flush_next().is_none());
    std::fs::write(&complete, "finish normally").unwrap();
    let (mut client, result) = tokio::time::timeout(Duration::from_secs(3), done_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Ok(StopReason::EndTurn)));
    pool.join_set.join_next().await.unwrap().unwrap();
    queue.mark_complete(&original.scope);
    assert!(!queue.has_undispatched_work());
    assert!(queue.next_retry_deadline().is_none());
    assert!(
        queue.flush_next().is_none(),
        "rejected follow-up must never requeue"
    );
    // A subsequent legitimate message must be delivered alone, by the same
    // client, with no cancelled/merged copy of either earlier message.
    push(&mut queue, make_event("later allowed task"));
    let next = queue.flush_next().unwrap();
    assert_eq!(next.events.len(), 1);
    assert_eq!(next.events[0].event.content, "later allowed task");
    assert!(next.cancelled_events.is_empty());
    assert!(next.cancel_reason.is_none());
    client
        .session_prompt_blocks_with_idle_timeout(
            "fixture",
            &[&next.events[0].event.content],
            Duration::from_secs(3),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    let wire = std::fs::read_to_string(&capture).unwrap();
    let requests: Vec<serde_json::Value> = wire
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(requests.len(), 2, "only original and later allowed prompts");
    for request in &requests {
        assert_eq!(request["method"], "session/prompt");
    }
    assert_eq!(requests[0]["params"]["prompt"][0]["text"], "original task");
    assert_eq!(
        requests[1]["params"]["prompt"][0]["text"],
        "later allowed task"
    );
    assert!(!wire.contains("REJECT_FOLLOWUP"));
    assert!(matches!(
        ack_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(150), notices.recv())
            .await
            .is_err(),
        "exactly one notice"
    );
    client.shutdown().await;
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn failed_native_steer_hook_posts_one_threaded_notice_without_redelivery() {
    rejected_steer_notice("exit 7", true).await;
}

#[tokio::test]
async fn timed_out_native_steer_hook_posts_one_notice_without_interrupting_task() {
    rejected_steer_notice("sleep 30", false).await;
}
