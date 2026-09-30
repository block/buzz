//! Exercise the run loop's final shutdown boundary with real failure producers
//! and the paced, encrypted publisher. The test transport receives signed relay
//! commands; this does not assert network delivery or relay persistence.
use super::*;
use error_outcome_emission_tests::{dummy_agent, test_config};
use nostr::{EventBuilder, Keys, Kind};

async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

fn emit_marker(observer: &observer::ObserverHandle, channel: Option<Uuid>) {
    observer.emit(
        "turn_started",
        Some(0),
        &observer::context_for(channel, None, Some("earlier-turn".into())),
        serde_json::json!({}),
    );
}

async fn emit_terminal_failure(
    observer: &observer::ObserverHandle,
    outcome: &str,
    batch_fate: &str,
) -> serde_json::Value {
    let channel_id = Uuid::new_v4();
    let root = "a".repeat(64);
    let parent = "b".repeat(64);
    let scope = scope::SessionScope::Thread {
        channel_id,
        root_event_id: root.clone(),
    };
    let event = EventBuilder::new(Kind::Custom(9), "shutdown request")
        .tags([
            nostr::Tag::parse(["e", root.as_str(), "", "root"]).unwrap(),
            nostr::Tag::parse(["e", parent.as_str(), "", "reply"]).unwrap(),
        ])
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let mut queue = EventQueue::new(config::DedupMode::Queue);
    queue.push(QueuedEvent {
        channel_id,
        scope: scope.clone(),
        event: event.clone(),
        received_at: std::time::Instant::now(),
        prompt_tag: "test".into(),
    });
    let batch = queue.flush_next().unwrap();
    if batch_fate == "exhausted" {
        queue.set_retry_count_for_test(scope.clone(), queue::MAX_RETRIES);
    }
    let removed_channels = if batch_fate == "removed" {
        HashSet::from([channel_id])
    } else {
        HashSet::new()
    };
    let mut pool = AgentPool::from_slots(vec![None]);
    let task = if outcome == "panic" {
        pool.join_set.spawn(async { panic!("shutdown fixture") })
    } else {
        pool.join_set.spawn(async {})
    };
    pool.task_map_mut().insert(
        task.id(),
        pool::TaskMeta {
            agent_index: 0,
            channel_id: Some(channel_id),
            scope: Some(scope.clone()),
            turn_id: "stopped-turn".into(),
            triggering: queue::triggering_event_context(&batch),
            recoverable_batch: Some(batch.clone()),
            control_tx: None,
            steer_tx: None,
            successful_steer_deliveries: HashSet::new(),
        },
    );
    let mut crash_history = vec![SlotCircuit {
        crash_times: Vec::new(),
        open_until: None,
        respawn_in_flight: false,
    }];
    for _ in 0..CIRCUIT_BREAKER_THRESHOLD - 1 {
        assert!(matches!(
            crash_history[0].record_crash(),
            CrashVerdict::Respawn(_)
        ));
    }
    let (respawn_tx, _respawn_rx) = mpsc::channel(8);
    let mut respawn_tasks = tokio::task::JoinSet::new();
    emit_marker(observer, Some(channel_id));
    let action = if outcome == "panic" {
        settle().await;
        drain_ready_join_results(
            &mut pool,
            &mut queue,
            &test_config(),
            &mut false,
            &removed_channels,
            &mut HashMap::new(),
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            Some(observer.clone()),
        )
    } else {
        let result = PromptResult {
            agent: dummy_agent(0).await,
            source: PromptSource::Channel(scope),
            turn_id: "stopped-turn".into(),
            outcome: match outcome {
                "fatal" => PromptOutcome::AgentExited,
                "timeout" => PromptOutcome::Timeout(pool::TimeoutKind::Idle),
                "transport" => PromptOutcome::Error(acp::AcpError::Protocol("broken pipe".into())),
                "cancel" => PromptOutcome::CancelDrainTimeout(Duration::from_secs(5)),
                _ => unreachable!(),
            },
            batch: Some(batch),
        };
        handle_prompt_result(
            &mut pool,
            &mut queue,
            &test_config(),
            result,
            &mut false,
            &removed_channels,
            &mut crash_history,
            &respawn_tx,
            &mut respawn_tasks,
            Some(observer.clone()),
            None,
        )
    };
    assert!(
        action == LoopAction::Exit,
        "{outcome}: production handler must exit"
    );
    let terminal = observer.snapshot().pop().unwrap();
    assert_eq!(
        terminal.payload["disposition"],
        if batch_fate == "exhausted" && outcome != "cancel" {
            "dead_lettered"
        } else {
            "stopped"
        },
        "{outcome}/{batch_fate}"
    );
    assert_eq!(terminal.payload["respawnScheduled"], false, "{outcome}");
    assert_eq!(
        terminal.payload["runtimeExiting"],
        action == LoopAction::Exit,
        "{outcome}"
    );
    assert_eq!(
        terminal.payload["triggeringEventIds"],
        serde_json::json!([event.id.to_hex()])
    );
    assert_eq!(terminal.payload["triggeringRootEventId"], root);
    assert_eq!(terminal.payload["triggeringParentEventId"], parent);
    serde_json::to_value(terminal).unwrap()
}

#[tokio::test(start_paused = true)]
async fn production_shutdown_publishes_terminal_failures_before_relay_close_without_bursting() {
    for outcome in ["fatal", "timeout", "transport", "cancel", "panic"] {
        for batch_fate in ["retry", "exhausted", "removed"] {
            let observer = observer::ObserverHandle::in_process();
            // More channels than the grace can drain, plus a null-channel barrier:
            // ordinary FIFO draining cannot reach the final stopped event in time.
            for _ in 0..8 {
                emit_marker(&observer, Some(Uuid::new_v4()));
            }
            emit_marker(&observer, None);
            let expected = emit_terminal_failure(&observer, outcome, batch_fate).await;
            let agent = Keys::generate();
            let owner = Keys::generate();
            let (publisher, mut published_rx) = RelayEventPublisher::test_pair();
            let publisher_task = spawn_relay_observer_publisher(
                observer.clone(),
                publisher,
                agent.clone(),
                agent.public_key().to_hex(),
                owner.public_key().to_hex(),
                owner.public_key(),
            );
            let publisher_abort = publisher_task.abort_handle();
            settle().await;
            tokio::time::advance(Duration::from_millis(250)).await;
            let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let closed_in_shutdown = closed.clone();
            let shutdown_observer = observer.clone();
            let shutdown = tokio::spawn(async move {
                shutdown_relay_after_observer(
                    Some(&shutdown_observer),
                    Some(publisher_task),
                    async move {
                        assert!(
                            publisher_abort.is_finished(),
                            "publisher joined before relay close"
                        );
                        closed_in_shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
                    },
                )
                .await;
            });
            settle().await;
            tokio::time::advance(Duration::from_millis(749)).await;
            settle().await;
            assert!(
                published_rx.try_recv().is_err(),
                "{outcome}: no shutdown burst"
            );
            assert!(!closed.load(std::sync::atomic::Ordering::SeqCst));
            tokio::time::advance(Duration::from_millis(1)).await;
            settle().await;
            let frame = published_rx
                .try_recv()
                .expect("terminal must get first paced slot");
            let payload: serde_json::Value = decrypt_observer_payload(&owner, &frame).unwrap();
            assert_eq!(
                payload, expected,
                "{outcome}: terminal context survives shutdown"
            );
            assert!(published_rx.try_recv().is_err(), "one frame per tick");
            tokio::time::advance(Duration::from_millis(999)).await;
            settle().await;
            assert!(
                published_rx.try_recv().is_err(),
                "backlog waits for next tick"
            );
            tokio::time::advance(Duration::from_millis(1)).await;
            settle().await;
            assert!(
                published_rx.try_recv().is_ok(),
                "ordinary backlog still gets paced slots"
            );
            assert!(published_rx.try_recv().is_err(), "backlog must not burst");
            tokio::time::advance(Duration::from_millis(1251)).await;
            settle().await;
            assert!(shutdown.is_finished(), "large backlog cannot extend grace");
            shutdown.await.unwrap();
            assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
            let remaining_frames = published_rx.len();
            assert!(
                remaining_frames <= 1,
                "at most one final tick at the grace boundary"
            );
        }
    }
}

#[tokio::test(start_paused = true)]
async fn ordinary_shutdown_keeps_historical_failure_before_successful_retry() {
    for disposition in ["stopped", "dead_lettered"] {
        for exiting_flag in [None, Some(false)] {
            let observer = observer::ObserverHandle::in_process();
            emit_marker(&observer, Some(Uuid::new_v4()));
            let channel = Uuid::new_v4();
            let context = observer::context_for(Some(channel), None, Some("old-turn".into()));
            let mut payload = serde_json::json!({"disposition": disposition});
            if let Some(flag) = exiting_flag {
                payload["runtimeExiting"] = serde_json::json!(flag);
            }
            observer.emit("turn_error", Some(0), &context, payload);
            let context =
                observer::context_for(Some(channel), None, Some("successful-retry".into()));
            observer.emit("turn_started", Some(0), &context, serde_json::json!({}));
            observer.emit("turn_completed", Some(0), &context, serde_json::json!({}));
            let agent = Keys::generate();
            let owner = Keys::generate();
            let (publisher, mut published_rx) = RelayEventPublisher::test_pair();
            let publisher_task = spawn_relay_observer_publisher(
                observer.clone(),
                publisher,
                agent.clone(),
                agent.public_key().to_hex(),
                owner.public_key().to_hex(),
                owner.public_key(),
            );
            settle().await;
            let shutdown = tokio::spawn(async move {
                shutdown_relay_after_observer(Some(&observer), Some(publisher_task), async {})
                    .await;
            });
            settle().await;
            tokio::time::advance(Duration::from_secs(1)).await;
            settle().await;
            let first = published_rx.try_recv().unwrap();
            let first: serde_json::Value = decrypt_observer_payload(&owner, &first).unwrap();
            assert_eq!(
                first["kind"], "turn_started",
                "historical failure is not promoted"
            );
            assert_ne!(first["channelId"], channel.to_string());
            assert!(published_rx.try_recv().is_err());
            tokio::time::advance(Duration::from_secs(1)).await;
            settle().await;
            let frame = published_rx.try_recv().unwrap();
            let frame: serde_json::Value = decrypt_observer_payload(&owner, &frame).unwrap();
            let kinds: Vec<_> = frame["payload"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|event| event["kind"].as_str().unwrap())
                .collect();
            assert_eq!(kinds, ["turn_error", "turn_started", "turn_completed"]);
            assert!(
                shutdown.is_finished(),
                "healthy short drain finishes before grace expires"
            );
            shutdown.await.unwrap();
        }
    }
}

#[tokio::test(start_paused = true)]
async fn production_shutdown_bounds_a_blocked_relay_publisher() {
    let observer = observer::ObserverHandle::in_process();
    let expected = emit_terminal_failure(&observer, "fatal", "retry").await;
    assert_eq!(expected["payload"]["disposition"], "stopped");
    let agent = Keys::generate();
    let owner = Keys::generate();
    let (publisher, published_rx) = RelayEventPublisher::test_pair();
    let filler = EventBuilder::new(Kind::Custom(9), "fill transport")
        .sign_with_keys(&agent)
        .unwrap();
    // Saturate both 64-slot queues and the forwarding task's in-flight send.
    // The real RelayEventPublisher::publish_event call then blocks at tick 1.
    for _ in 0..129 {
        publisher.publish_event(filler.clone()).await.unwrap();
    }
    let publisher_task = spawn_relay_observer_publisher(
        observer.clone(),
        publisher,
        agent.clone(),
        agent.public_key().to_hex(),
        owner.public_key().to_hex(),
        owner.public_key(),
    );
    let abort = publisher_task.abort_handle();
    settle().await;
    let shutdown = tokio::spawn(async move {
        shutdown_relay_after_observer(Some(&observer), Some(publisher_task), async move {
            assert!(
                abort.is_finished(),
                "blocked publisher must be aborted and joined"
            );
        })
        .await;
    });
    settle().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    assert!(
        !shutdown.is_finished(),
        "publishing gets a bounded opportunity"
    );
    assert_eq!(published_rx.len(), 64, "transport remains saturated");
    tokio::time::advance(OBSERVER_SHUTDOWN_GRACE - Duration::from_secs(1)).await;
    settle().await;
    assert!(
        shutdown.is_finished(),
        "blocked transport cannot extend observer grace"
    );
    shutdown.await.unwrap();
    drop(published_rx);
}

#[tokio::test]
async fn observer_close_fences_all_clones_and_retains_snapshot() {
    let observer = observer::ObserverHandle::in_process();
    let retained = observer.clone();
    let mut rx = retained.subscribe();
    emit_marker(&observer, None);
    observer.close();
    emit_marker(&retained, None);
    assert_eq!(
        retained.snapshot().len(),
        1,
        "late emission cannot extend shutdown backlog"
    );
    assert_eq!(rx.recv().await.unwrap().seq, 1);
    assert!(matches!(
        rx.recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Closed)
    ));
    assert!(matches!(
        retained.subscribe().recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Closed)
    ));
}
