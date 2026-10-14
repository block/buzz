//! Result/ack orderings through the production result handler and queue.
use super::*;
use error_outcome_emission_tests::{bind_agent_scope_owner, dummy_agent, test_config};

fn follow_up(scope: &scope::SessionScope) -> QueuedEvent {
    QueuedEvent {
        channel_id: scope.channel_id(),
        scope: scope.clone(),
        event: nostr::EventBuilder::new(nostr::Kind::Custom(9), "follow-up")
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap(),
        received_at: std::time::Instant::now(),
        prompt_tag: "mention".into(),
        edit: None,
    }
}

async fn settle(ack_first: bool, success: bool) {
    let scope = scope::SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut agent = dummy_agent(0).await;
    let mut pool = AgentPool::from_slots(vec![None]);
    bind_agent_scope_owner(&mut pool, &mut agent, scope.clone());
    let task_id = pool.join_set.spawn(async {}).id();
    pool.task_map_mut().insert(
        task_id,
        pool::TaskMeta {
            agent_index: 0,
            channel_id: Some(scope.channel_id()),
            scope: Some(scope.clone()),
            turn_id: "turn".into(),
            recoverable_batch: None,
            control_tx: None,
            steer_tx: None,
            successful_steer_deliveries: HashSet::new(),
        },
    );
    let mut queue = EventQueue::new(DedupMode::Queue);
    let event = follow_up(&scope);
    let event_id = event.event.id.to_hex();
    queue.push(event);
    assert!(queue.mark_native_steer_pending(&scope, &event_id, "turn"));
    if ack_first {
        assert!(queue.acknowledge_native_steer(&scope, &event_id));
    }
    assert!(
        queue.flush_next().is_none(),
        "acceptance alone must not redeliver"
    );
    let (tx, _rx) = mpsc::channel(1);
    let mut tasks = tokio::task::JoinSet::new();
    handle_prompt_result(
        &mut pool,
        &mut queue,
        &test_config(),
        PromptResult {
            agent,
            source: PromptSource::Channel(scope.clone()),
            turn_id: "turn".into(),
            outcome: if success {
                PromptOutcome::Ok(acp::StopReason::EndTurn)
            } else {
                PromptOutcome::AgentExited
            },
            batch: None,
        },
        &mut false,
        &HashSet::new(),
        &mut [SlotCircuit {
            crash_times: vec![],
            open_until: None,
            respawn_in_flight: false,
        }],
        &tx,
        &mut tasks,
        None,
        None,
        &mut tokio::time::Instant::now(),
    );
    if !ack_first {
        assert_eq!(queue.acknowledge_native_steer(&scope, &event_id), success);
    }
    let recovered = queue.flush_next();
    tasks.shutdown().await;
    shutdown_agent_pool(&mut pool).await;
    if success {
        assert!(recovered.is_none(), "completed follow-up must not replay");
    } else {
        let recovered = recovered.expect("failed turn must retain accepted follow-up");
        assert_eq!(recovered.events.len(), 1);
        assert_eq!(recovered.events[0].event.id.to_hex(), event_id);
        assert_eq!(recovered.events[0].event.content, "follow-up");
    }
}

#[tokio::test]
async fn native_follow_up_survives_worker_failure_in_both_ack_orderings() {
    settle(true, false).await;
    settle(false, false).await;
}

#[tokio::test]
async fn completed_native_follow_up_is_not_replayed_in_either_ack_ordering() {
    settle(true, true).await;
    settle(false, true).await;
}

#[tokio::test]
async fn acknowledged_native_follow_up_survives_prompt_task_panic() {
    let scope = scope::SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut pool = AgentPool::from_slots(vec![None]);
    let task = pool
        .join_set
        .spawn(async { panic!("fixture prompt panic") });
    pool.task_map_mut().insert(
        task.id(),
        pool::TaskMeta {
            agent_index: 0,
            channel_id: Some(scope.channel_id()),
            scope: Some(scope.clone()),
            turn_id: "turn".into(),
            recoverable_batch: None,
            control_tx: None,
            steer_tx: None,
            successful_steer_deliveries: HashSet::new(),
        },
    );
    let mut queue = EventQueue::new(DedupMode::Queue);
    let event = follow_up(&scope);
    let id = event.event.id.to_hex();
    queue.push(event);
    assert!(queue.mark_native_steer_pending(&scope, &id, "turn"));
    assert!(queue.acknowledge_native_steer(&scope, &id));
    let failure = pool.join_set.join_next().await.unwrap().unwrap_err();
    let (tx, _rx) = mpsc::channel(1);
    let mut tasks = tokio::task::JoinSet::new();
    recover_panicked_agent(
        &mut pool,
        &mut queue,
        &test_config(),
        failure,
        &mut false,
        &HashSet::new(),
        &mut HashMap::new(),
        &mut [SlotCircuit {
            crash_times: vec![],
            open_until: None,
            respawn_in_flight: false,
        }],
        &tx,
        &mut tasks,
        None,
    );
    tasks.shutdown().await;
    assert_eq!(queue.flush_next().unwrap().events[0].event.id.to_hex(), id);
}

#[test]
fn a_later_turn_does_not_overwrite_a_failed_turn_waiting_for_its_ack() {
    let scope = scope::SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut queue = EventQueue::new(DedupMode::Queue);
    let event = follow_up(&scope);
    let id = event.event.id.to_hex();
    queue.push(event);
    assert!(queue.mark_native_steer_pending(&scope, &id, "turn"));
    queue.finish_native_steers(&scope, "turn", false);
    queue.finish_native_steers(&scope, "later-turn", true);
    assert!(!queue.acknowledge_native_steer(&scope, &id));
    assert_eq!(queue.flush_next().unwrap().events[0].event.id.to_hex(), id);
}

#[tokio::test]
async fn original_completion_before_steer_ack_retries_only_the_follow_up() {
    let scope = scope::SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut queue = EventQueue::new(DedupMode::Queue);
    let original = follow_up(&scope);
    let original_id = original.event.id.to_hex();
    queue.push(original);
    let batch = queue.flush_next().unwrap();
    let follow_up = follow_up(&scope);
    let follow_up_id = follow_up.event.id.to_hex();
    queue.push(follow_up);
    assert!(queue.mark_native_steer_pending(&scope, &follow_up_id, "original"));

    // Complete the original request only after the steer is on the wire,
    // then report a detached follow-up. The original work is definitively done.
    let script = r#"read initialize
echo '{"jsonrpc":"2.0","id":0,"result":{"_meta":{"steering":{"supported":true}}}}'
read prompt
read steer
echo '{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}'
echo '{"jsonrpc":"2.0","id":2,"result":{"outcome":"startedNewTurn"}}'
cat >/dev/null"#;
    let mut agent = dummy_agent(0).await;
    agent.acp.shutdown().await;
    agent.acp = AcpClient::spawn("/bin/sh", &["-c".into(), script.into()], &[], false)
        .await
        .unwrap();
    agent.acp.initialize().await.unwrap();
    agent.state.sessions.insert(scope.clone(), "session".into());
    let (steer_tx, steer_rx) = mpsc::channel(1);
    agent.acp.install_steer_rx(steer_rx);
    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    steer_tx
        .send(pool::SteerRequest {
            prompt_blocks: vec!["follow-up".into()],
            ack_tx,
        })
        .await
        .unwrap();

    let mut context = pool::tests::make_prompt_context_no_owner();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    context.rest_client.base_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = [0; 16 * 1024];
            let _ = socket.read(&mut request).await;
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]").await;
        }
    });
    context.dedup_mode = DedupMode::Queue;
    context.channel_info = pool::ChannelInfoResolver::new(
        HashMap::from([(
            scope.channel_id(),
            relay::ChannelInfo {
                name: "test".into(),
                channel_type: "stream".into(),
                description: None,
            },
        )]),
        context.rest_client.clone(),
    );
    let mut pool = AgentPool::from_slots(vec![None]);
    bind_agent_scope_owner(&mut pool, &mut agent, scope.clone());
    let task_id = pool.join_set.spawn(async {}).id();
    pool.task_map_mut().insert(
        task_id,
        pool::TaskMeta {
            agent_index: 0,
            channel_id: Some(scope.channel_id()),
            scope: Some(scope.clone()),
            turn_id: "original".into(),
            recoverable_batch: Some(batch.clone()),
            control_tx: None,
            steer_tx: None,
            successful_steer_deliveries: HashSet::new(),
        },
    );
    let (result_tx, mut result_rx) = mpsc::unbounded_channel();
    tokio::time::timeout(
        Duration::from_secs(5),
        pool::run_prompt_task(
            agent,
            Some(batch),
            None,
            Arc::new(context),
            result_tx,
            None,
            "original".into(),
            Default::default(),
        ),
    )
    .await
    .unwrap();
    let result = result_rx.recv().await.unwrap();
    assert!(
        matches!(result.outcome, PromptOutcome::Ok(acp::StopReason::EndTurn)),
        "a confirmed original completion must remain successful"
    );
    assert!(
        result.batch.is_none(),
        "completed original batch must not be retried"
    );
    assert!(matches!(
        ack_rx.await.unwrap(),
        pool::SteerAck::PromptCompletedNeutral
    ));
    queue.release_native_steer(&scope, &follow_up_id);
    let (tx, _rx) = mpsc::channel(1);
    let mut tasks = tokio::task::JoinSet::new();
    let mut crash_history = [SlotCircuit {
        crash_times: vec![],
        open_until: None,
        respawn_in_flight: false,
    }];
    handle_prompt_result(
        &mut pool,
        &mut queue,
        &test_config(),
        result,
        &mut false,
        &HashSet::new(),
        &mut crash_history,
        &tx,
        &mut tasks,
        None,
        None,
        &mut tokio::time::Instant::now(),
    );
    assert!(
        crash_history[0].respawn_in_flight,
        "retired worker must be replaced"
    );
    assert!(
        pool.agents_mut()[0].is_none(),
        "retired worker must not be reused"
    );
    let retry = queue.flush_next().unwrap();
    assert_eq!(retry.events.len(), 1);
    assert_eq!(retry.events[0].event.id.to_hex(), follow_up_id);
    assert_ne!(retry.events[0].event.id.to_hex(), original_id);
    tasks.shutdown().await;
    shutdown_agent_pool(&mut pool).await;
    server.abort();
}

/// Drive both wire orderings through real subprocess replacement three times,
/// then prove actual failures still open the same slot's circuit.
#[tokio::test]
async fn planned_steer_retirements_preserve_availability_and_genuine_crashes_count() {
    for original_first in [true, false] {
        let original = r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}"#;
        let steer = r#"{"jsonrpc":"2.0","id":2,"result":{"outcome":"startedNewTurn"}}"#;
        let (first, second) = if original_first {
            (original, steer)
        } else {
            (steer, original)
        };
        let script = format!("read initialize\necho '{{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{{\"_meta\":{{\"steering\":{{\"supported\":true}}}}}}}}'\nread prompt\nread steer\necho '{first}'\necho '{second}'\ncat >/dev/null");
        let mut config = test_config();
        config.agent_command = "/bin/sh".into();
        config.agent_args = vec!["-c".into(), script];
        let mut agent = dummy_agent(0).await;
        agent.acp.shutdown().await;
        agent.acp = spawn_and_init(
            &config.agent_command,
            &config.agent_args,
            &[],
            false,
            0,
            None,
            AGENT_INITIALIZE_TIMEOUT,
        )
        .await
        .unwrap()
        .0;
        let mut pool = AgentPool::from_slots(vec![Some(agent)]);
        let mut history = [SlotCircuit {
            crash_times: vec![],
            open_until: None,
            respawn_in_flight: false,
        }];
        let (tx, mut rx) = mpsc::channel(1);
        let mut tasks = tokio::task::JoinSet::new();
        for round in 0..6 {
            let planned = round < 3;
            let scope = scope::SessionScope::Conversation {
                channel_id: Uuid::new_v4(),
            };
            let turn_id = format!("turn-{round}");
            let mut agent = pool
                .try_claim(None)
                .expect("replacement must remain available");
            bind_agent_scope_owner(&mut pool, &mut agent, scope.clone());
            let task_id = pool.join_set.spawn(async {}).id();
            pool.task_map_mut().insert(
                task_id,
                pool::TaskMeta {
                    agent_index: 0,
                    channel_id: Some(scope.channel_id()),
                    scope: Some(scope.clone()),
                    turn_id: turn_id.clone(),
                    recoverable_batch: None,
                    control_tx: None,
                    steer_tx: None,
                    successful_steer_deliveries: HashSet::new(),
                },
            );
            let mut queue = EventQueue::new(DedupMode::Queue);
            let event = follow_up(&scope);
            let event_id = event.event.id.to_hex();
            queue.push(event);
            assert!(queue.mark_native_steer_pending(&scope, &event_id, &turn_id));
            let outcome = if planned {
                let (steer_tx, steer_rx) = mpsc::channel(1);
                agent.acp.install_steer_rx(steer_rx);
                let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
                steer_tx
                    .send(pool::SteerRequest {
                        prompt_blocks: vec!["follow-up".into()],
                        ack_tx,
                    })
                    .await
                    .unwrap();
                let result = tokio::time::timeout(
                    Duration::from_secs(5),
                    agent.acp.session_prompt_with_idle_timeout(
                        "session",
                        "original",
                        Duration::from_secs(3),
                        Duration::from_secs(3),
                    ),
                )
                .await
                .unwrap();
                assert!(agent.acp.retired_for_untracked_steer());
                assert!(agent.acp.is_shutdown());
                assert!(matches!(
                    ack_rx.await.unwrap(),
                    pool::SteerAck::PromptCompletedNeutral | pool::SteerAck::Err(_)
                ));
                queue.release_native_steer(&scope, &event_id);
                if original_first {
                    PromptOutcome::Ok(result.expect("known original success preserved"))
                } else {
                    assert!(matches!(result, Err(acp::AcpError::AgentExited)));
                    PromptOutcome::AgentExited
                }
            } else {
                // A failure path can also have already shut down the client.
                // Only the explicitly planned retirement flag exempts a crash.
                agent.acp.shutdown().await;
                assert!(!agent.acp.retired_for_untracked_steer());
                PromptOutcome::AgentExited
            };
            let action = handle_prompt_result(
                &mut pool,
                &mut queue,
                &config,
                PromptResult {
                    agent,
                    source: PromptSource::Channel(scope.clone()),
                    turn_id,
                    outcome,
                    batch: None,
                },
                &mut false,
                &HashSet::new(),
                &mut history,
                &tx,
                &mut tasks,
                None,
                None,
                &mut tokio::time::Instant::now(),
            );
            if planned {
                assert!(
                    history[0].crash_times.is_empty(),
                    "planned retirement must not charge crash history"
                );
                assert!(history[0].open_until.is_none());
                assert_eq!(
                    queue.flush_next().unwrap().events[0].event.id.to_hex(),
                    event_id
                );
            } else {
                assert_eq!(history[0].crash_times.len(), round - 2);
            }
            if round == 5 {
                assert!(matches!(action, LoopAction::Exit));
                assert!(
                    history[0].open_until.is_some(),
                    "three real failures must still open the circuit"
                );
            } else {
                assert!(matches!(action, LoopAction::Continue));
                assert!(history[0].respawn_in_flight);
                let replacement = tokio::time::timeout(Duration::from_secs(5), rx.recv())
                    .await
                    .unwrap()
                    .unwrap();
                assert!(replacement.result.is_ok(), "replacement handshake succeeds");
                handle_respawn_result(replacement, &mut history, &mut pool, &config);
                assert!(!history[0].respawn_in_flight);
                assert!(pool.any_idle());
            }
        }
        tasks.shutdown().await;
        shutdown_agent_pool(&mut pool).await;
    }
}
