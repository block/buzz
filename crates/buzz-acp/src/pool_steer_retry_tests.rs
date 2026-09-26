//! Failed merged turns must retry their complete work through the real producer.

use super::*;
use crate::config::{CliArgs, Config};
use crate::queue::{format_prompt, CancelReason, EventQueue, FormatPromptArgs, QueuedEvent};
use clap::Parser;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::json;

async fn retry_agent(success_on_third_prompt: bool) -> OwnedAgent {
    // Speak actual ACP over stdio. Both session creation and prompting must
    // succeed at the protocol boundary before the intended transient failure.
    let script = format!(
        r#"count=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"session/new"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"sessionId":"retry-session"}}}}\n' "$id" ;;
    *'"method":"session/prompt"'*)
      count=$((count + 1))
      if [ "$count" -eq 3 ] && [ "{success_on_third_prompt}" = true ]; then
        printf '{{"jsonrpc":"2.0","id":%s,"result":{{"stopReason":"end_turn"}}}}\n' "$id"
      else
        printf '{{"jsonrpc":"2.0","id":%s,"error":{{"code":-32603,"message":"retry fixture transient failure"}}}}\n' "$id"
      fi ;;
  esac
done"#
    );
    OwnedAgent {
        index: 0,
        acp: AcpClient::spawn("bash", &["-c".into(), script], &[], false)
            .await
            .expect("spawn fixture ACP"),
        state: SessionState::default(),
        model_capabilities: None,
        desired_model: None,
        model_overridden: false,
        desired_model_request_id: None,
        desired_model_pending_ack: false,
        startup_effort: None,
        agent_name: "retry-fixture".into(),
        goose_system_prompt_supported: None,
        protocol_version: 1,
    }
}

#[tokio::test]
async fn merged_retry_preserves_complete_work_through_repeated_errors_and_terminal_outcomes() {
    // Serve empty relay reads locally so real metadata/reaction paths remain
    // exercised without adding connection-failure retry sleeps to every turn.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = vec![0; 16_384];
            let _ = socket.read(&mut request).await;
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]").await;
        }
    });
    for reason in [CancelReason::Steer, CancelReason::Interrupt] {
        for ending in ["success", "exhausted", "removed"] {
            let keys = Keys::generate();
            let config = Config::from_args(
                CliArgs::try_parse_from([
                    "buzz-acp",
                    "--private-key",
                    &keys.secret_key().to_secret_hex(),
                    "--respond-to",
                    "anyone",
                ])
                .unwrap(),
            )
            .unwrap();
            let channel_id = Uuid::new_v4();
            let root = "c".repeat(64);
            let parent = "d".repeat(64);
            let scope = SessionScope::Thread {
                channel_id,
                root_event_id: root.clone(),
            };
            let queued = |content, timestamp| QueuedEvent {
                channel_id,
                scope: scope.clone(),
                event: EventBuilder::new(Kind::Custom(9), content)
                    .custom_created_at(Timestamp::from(timestamp))
                    .tags([
                        Tag::parse(["e", &root, "", "root"]).unwrap(),
                        Tag::parse(["e", &parent, "", "reply"]).unwrap(),
                    ])
                    .sign_with_keys(&keys)
                    .unwrap(),
                prompt_tag: "@mention".into(),
                received_at: std::time::Instant::now(),
            };
            let original = queued("unfinished request A", 1000_u64);
            let new = queued("steering request B", 1001_u64);
            let expected_ids = vec![original.event.id.to_hex(), new.event.id.to_hex()];
            let original_received = original.received_at;
            let new_received = new.received_at;
            let mut queue = EventQueue::new(DedupMode::Queue);
            queue.push(original);
            let original_batch = queue.flush_next().unwrap();
            queue.push(new);
            queue.requeue_as_cancelled(original_batch, reason);
            queue.mark_complete(&scope);

            let mut ctx = super::tests::make_prompt_context_no_owner();
            ctx.dedup_mode = DedupMode::Queue;
            ctx.rest_client.base_url = relay_url.clone();
            ctx.channel_info = ChannelInfoResolver::new(
                HashMap::from([(
                    channel_id,
                    crate::relay::ChannelInfo {
                        name: "retry-fixture".into(),
                        channel_type: "stream".into(),
                        description: None,
                    },
                )]),
                ctx.rest_client.clone(),
            );
            ctx.channel_info.projects.write().unwrap().insert(
                channel_id,
                CachedProjectInfo {
                    fetched_at: std::time::Instant::now(),
                    value: None,
                },
            );
            let ctx = Arc::new(ctx);
            let observer = observer::ObserverHandle::in_process();
            let mut agent = retry_agent(ending == "success").await;
            agent
                .state
                .canvas_sections
                .insert(scope.clone(), String::new());
            agent.acp.set_observer(Some(observer.clone()), 0);
            let mut pool = AgentPool::from_slots(vec![Some(agent)]);
            let mut circuits = vec![crate::SlotCircuit {
                crash_times: vec![],
                open_until: None,
                respawn_in_flight: false,
            }];
            let (respawn_tx, _respawn_rx) = mpsc::channel(1);
            let mut respawn_tasks = tokio::task::JoinSet::new();

            for turn in 0..3 {
                let batch = queue.flush_next().expect("complete merged retry batch");
                assert_eq!(
                    batch.cancelled_events.len(),
                    1,
                    "original work must survive every retry without duplication"
                );
                assert_eq!(batch.events.len(), 1);
                assert_eq!(batch.cancel_reason, Some(reason));
                assert_eq!(batch.cancelled_events[0].received_at, original_received);
                assert_eq!(batch.events[0].received_at, new_received);
                let triggering = crate::queue::triggering_event_context(&batch);
                assert_eq!(triggering.event_ids, expected_ids);
                let text = format_prompt(&batch, &FormatPromptArgs::default()).join("\n\n");
                assert!(text.contains("unfinished request A"));
                assert!(text.contains("steering request B"));
                match reason {
                    CancelReason::Steer => {
                        assert!(text.contains("<what-you-were-working-on>"));
                        assert!(text.contains("Continue your"));
                        assert!(!text.contains("supersedes"));
                    }
                    CancelReason::Interrupt => {
                        assert!(text.contains("<previous-request-interrupted-before-completion>"));
                        assert!(text.contains("<new-request-supersedes-previous>"));
                    }
                }

                let turn_id = format!("steer-retry-{turn}");
                let mut agent = pool.try_claim(Some(&scope)).unwrap();
                let generation = pool.record_scope_owner(scope.clone(), agent.index);
                agent
                    .state
                    .set_scope_owner_generation(scope.clone(), generation);
                let task_id = pool.join_set.spawn(async {}).id();
                pool.task_map_mut().insert(
                    task_id,
                    TaskMeta {
                        agent_index: 0,
                        channel_id: Some(channel_id),
                        scope: Some(scope.clone()),
                        turn_id: turn_id.clone(),
                        triggering,
                        recoverable_batch: Some(batch.clone()),
                        control_tx: None,
                        steer_tx: None,
                        successful_steer_deliveries: HashSet::new(),
                    },
                );
                let (result_tx, mut result_rx) = mpsc::unbounded_channel();
                tokio::time::timeout(
                    Duration::from_secs(5),
                    run_prompt_task(
                        agent,
                        Some(batch),
                        Some(text),
                        ctx.clone(),
                        result_tx,
                        None,
                        turn_id.clone(),
                    ),
                )
                .await
                .unwrap_or_else(|_| {
                    panic!(
                        "fixture prompt {turn} must finish; observer kinds: {:?}",
                        observer
                            .snapshot()
                            .iter()
                            .map(|event| event.kind.as_str())
                            .collect::<Vec<_>>()
                    )
                });
                let result = result_rx.recv().await.unwrap();
                let success = turn == 2 && ending == "success";
                if success {
                    assert!(matches!(
                        result.outcome,
                        PromptOutcome::Ok(StopReason::EndTurn)
                    ));
                    assert!(result.batch.is_none(), "completed work must not be retried");
                } else {
                    assert!(
                        matches!(&result.outcome, PromptOutcome::Error(AcpError::AgentError { code: -32603, message }) if message == "retry fixture transient failure"),
                        "fixture must reach intended prompt failure"
                    );
                }
                let removed = if turn == 2 && ending == "removed" {
                    queue.drain_channel(channel_id);
                    HashSet::from([channel_id])
                } else {
                    HashSet::new()
                };
                if turn == 2 && ending == "exhausted" {
                    queue.set_retry_count_for_test(&scope, crate::queue::MAX_RETRIES);
                }
                crate::handle_prompt_result(
                    &mut pool,
                    &mut queue,
                    &config,
                    result,
                    &mut false,
                    &removed,
                    &mut circuits,
                    &respawn_tx,
                    &mut respawn_tasks,
                    Some(observer.clone()),
                    None,
                );
                let events = observer.snapshot();
                let started = events
                    .iter()
                    .find(|event| {
                        event.kind == "turn_started" && event.turn_id.as_deref() == Some(&turn_id)
                    })
                    .unwrap();
                assert_eq!(started.payload["triggeringEventIds"], json!(expected_ids));
                let failed = events.iter().find(|event| {
                    event.kind == "turn_error" && event.turn_id.as_deref() == Some(&turn_id)
                });
                if success {
                    assert!(failed.is_none());
                } else {
                    let failed = failed.unwrap();
                    assert_eq!(
                        failed.payload["triggeringEventIds"],
                        started.payload["triggeringEventIds"]
                    );
                    assert_eq!(failed.payload["triggeringRootEventId"], json!(root));
                    assert_eq!(failed.payload["triggeringParentEventId"], json!(parent));
                    assert_eq!(
                        failed.payload["disposition"],
                        if turn < 2 {
                            "retrying"
                        } else if ending == "exhausted" {
                            "dead_lettered"
                        } else {
                            "stopped"
                        }
                    );
                }
                if turn < 2 {
                    assert_eq!(queue.retry_count(&scope), turn + 1);
                    assert!(
                        !queue.has_flushable_work(),
                        "cancelled carryover must obey retry backoff"
                    );
                    assert!(queue.flush_next().is_none(), "retry backoff still applies");
                    queue.expire_retry_for_test(&scope);
                }
            }
            assert!(
                !queue.has_pending_scope(&scope),
                "terminal outcome retires both requests"
            );
            assert!(queue.flush_next().is_none());
            assert_eq!(queue.retry_count(&scope), 0);
            if reason == CancelReason::Steer && ending == "exhausted" {
                // The third prompt is forced to exhaust the budget above. The
                // shared desktop fixture describes this exact producer trace;
                // normalize only the signed fixture identities, not semantics.
                let normalize_id = |value: &serde_json::Value| {
                    let id = value.as_str().expect("correlation must be a string");
                    if id == expected_ids[0] {
                        "request-a"
                    } else if id == expected_ids[1] {
                        "request-b"
                    } else if id == root {
                        "thread-root"
                    } else if id == parent {
                        "thread-parent"
                    } else {
                        panic!("unexpected correlation ID: {id}")
                    }
                };
                let trace: Vec<_> = observer.snapshot().into_iter()
                    .filter(|event| matches!(event.kind.as_str(), "turn_started" | "turn_error"))
                    .map(|event| {
                        assert_eq!(event.channel_id, Some(channel_id.to_string()));
                        assert_eq!(event.payload["triggeringEventIds"], json!(expected_ids));
                        assert_eq!(event.payload["triggeringRootEventId"], json!(root));
                        assert_eq!(event.payload["triggeringParentEventId"], json!(parent));
                        let mut payload = json!({
                            "triggeringEventIds": event.payload["triggeringEventIds"].as_array().unwrap().iter().map(&normalize_id).collect::<Vec<_>>(),
                            "triggeringRootEventId": normalize_id(&event.payload["triggeringRootEventId"]),
                            "triggeringParentEventId": normalize_id(&event.payload["triggeringParentEventId"]),
                        });
                        if event.kind == "turn_error" {
                            payload["disposition"] = event.payload["disposition"].clone();
                            payload["attempt"] = event.payload["attempt"].clone();
                        }
                        json!({"kind": event.kind, "turnId": event.turn_id, "payload": payload})
                    }).collect();
                let expected: serde_json::Value = serde_json::from_str(include_str!(
                    "../tests/fixtures/steer-retry-observer.json"
                ))
                .unwrap();
                assert_eq!(json!(trace), expected);
            }
            pool.try_claim(Some(&scope)).unwrap().acp.shutdown().await;
        }
    }
    server.abort();
}
