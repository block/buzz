// Included in error_outcome_emission_tests: reuse its real OwnedAgent fixture,
// Config and production result handler, rather than model a second lifecycle.
async fn receipt_result_case(
    outcome: PromptOutcome,
    mode: config::DedupMode,
    cancelled: bool,
    return_batch: bool,
    native: Option<bool>,
    capacity: usize,
) -> (Vec<&'static str>, bool) {
    let agent = dummy_agent(0).await;
    let channel_id = Uuid::new_v4();
    let scope = scope::SessionScope::Conversation { channel_id };
    let mut queue = EventQueue::new(mode);
    let event = EventBuilder::new(Kind::Custom(9), "receipt task")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let (receipt, mut probe) = relay::DeliveryProbe::new(&event.id.to_hex(), capacity);
    receipt.accepted();
    let receipt = match native {
        Some(injected) => {
            let gated = receipt.native_pending();
            if injected {
                gated.mark_native_injected();
            }
            gated
        }
        None => receipt,
    };
    queue.push(QueuedEvent {
        delivery: Some(receipt),
        channel_id,
        scope: scope.clone(),
        event,
        received_at: std::time::Instant::now(),
        prompt_tag: "test".into(),
    });
    let batch = queue.flush_next().unwrap();
    let mut pool = AgentPool::from_slots(vec![None]);
    let task = pool.join_set.spawn(async {}).id();
    let (control_tx, _rx) = tokio::sync::oneshot::channel();
    pool.task_map_mut().insert(
        task,
        pool::TaskMeta {
            agent_index: 0,
            channel_id: Some(channel_id),
            scope: Some(scope.clone()),
            turn_id: "receipt-turn".into(),
            recoverable_batch: Some(batch.clone()),
            control_tx: if cancelled { None } else { Some(control_tx) },
            steer_tx: None,
            successful_steer_deliveries: HashSet::new(),
        },
    );
    let mut config = test_config();
    config.dedup_mode = mode;
    let mut history = vec![SlotCircuit {
        crash_times: vec![],
        open_until: Some(std::time::Instant::now() + Duration::from_secs(3600)),
        respawn_in_flight: false,
    }];
    let (tx, _rx) = mpsc::channel(8);
    let mut tasks = tokio::task::JoinSet::new();
    handle_prompt_result(
        &mut pool,
        &mut queue,
        &config,
        PromptResult {
            agent,
            source: PromptSource::Channel(scope),
            turn_id: "receipt-turn".into(),
            outcome,
            batch: if return_batch { Some(batch) } else { None },
        },
        &mut false,
        &HashSet::new(),
        &mut history,
        &tx,
        &mut tasks,
        None,
        None,
    );
    assert!(tasks.is_empty());
    (probe.outcomes(), queue.has_undispatched_work())
}

#[tokio::test]
async fn receipt_only_end_turn_is_completed_in_queue_and_drop_modes() {
    for mode in [config::DedupMode::Queue, config::DedupMode::Drop] {
        for stop in [
            acp::StopReason::EndTurn,
            acp::StopReason::Cancelled,
            acp::StopReason::MaxTokens,
            acp::StopReason::MaxTurnRequests,
            acp::StopReason::Refusal,
        ] {
            let complete = matches!(stop, acp::StopReason::EndTurn);
            let (acks, queued) =
                receipt_result_case(PromptOutcome::Ok(stop), mode, false, false, None, 8).await;
            assert_eq!(
                acks,
                if complete {
                    vec!["accepted", "completed"]
                } else {
                    vec!["accepted"]
                }
            );
            assert!(
                !queued,
                "feedback uncertainty must not invent a local execution retry"
            );
        }
    }
}

#[tokio::test]
async fn receipt_cancel_failure_and_dead_letter_never_complete() {
    for (outcome, returned, queued) in [
        (PromptOutcome::Cancelled, true, true),
        (PromptOutcome::Cancelled, false, false), // explicit Cancel/Rotate or Drop
        (
            PromptOutcome::CancelDrainTimeout(Duration::from_secs(5)),
            true,
            true,
        ),
        (
            PromptOutcome::CancelDrainTimeout(Duration::from_secs(5)),
            false,
            false,
        ),
        (
            PromptOutcome::Timeout(TimeoutKind::Hard {
                recently_active: false,
            }),
            true,
            false,
        ),
        (PromptOutcome::Timeout(TimeoutKind::Idle), true, true),
        (PromptOutcome::AgentExited, true, true),
        (
            PromptOutcome::Error(AcpError::Protocol("before EndTurn".into())),
            true,
            true,
        ),
        (PromptOutcome::Ok(acp::StopReason::Cancelled), false, false),
    ] {
        let (acks, pending) =
            receipt_result_case(outcome, config::DedupMode::Queue, true, returned, None, 8).await;
        assert_eq!(acks, vec!["accepted"]);
        assert_eq!(pending, queued);
    }
}

#[tokio::test]
async fn receipt_completed_outcome_outlives_control_in_queue_and_drop_modes() {
    for mode in [config::DedupMode::Queue, config::DedupMode::Drop] {
        for native in [None, Some(false), Some(true)] {
            let (acks, queued) = receipt_result_case(
                PromptOutcome::Ok(acp::StopReason::EndTurn),
                mode,
                true,
                false,
                native,
                8,
            )
            .await;
            assert_eq!(
                acks,
                if native == Some(false) {
                    vec!["accepted"]
                } else {
                    vec!["accepted", "completed"]
                }
            );
            assert!(!queued);
        }
    }
}

#[tokio::test]
async fn receipt_native_gate_and_feedback_refusal_do_not_fabricate_completion_or_requeue() {
    for injected in [false, true] {
        let (acks, queued) = receipt_result_case(
            PromptOutcome::Ok(acp::StopReason::EndTurn),
            config::DedupMode::Queue,
            false,
            false,
            Some(injected),
            8,
        )
        .await;
        assert_eq!(
            acks,
            if injected {
                vec!["accepted", "completed"]
            } else {
                vec!["accepted"]
            }
        );
        assert!(!queued);
    }
    let (acks, queued) = receipt_result_case(
        PromptOutcome::Ok(acp::StopReason::EndTurn),
        config::DedupMode::Queue,
        false,
        false,
        None,
        1,
    )
    .await;
    assert_eq!(acks, vec!["accepted"]);
    assert!(
        !queued,
        "lost terminal feedback must not repeat successful side effects locally"
    );
}

#[tokio::test]
async fn receipt_native_submission_owns_ledger_before_delayed_watcher_and_fences_successor() {
    let mut pool = AgentPool::from_slots(vec![]);
    let channel_id = Uuid::new_v4();
    let scope = scope::SessionScope::Conversation { channel_id };
    let mut queue = EventQueue::new(config::DedupMode::Queue);
    let base = EventBuilder::new(Kind::Custom(9), "base")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    queue.push(QueuedEvent {
        delivery: None,
        channel_id,
        scope: scope.clone(),
        event: base,
        received_at: std::time::Instant::now(),
        prompt_tag: "base".into(),
    });
    let batch = queue.flush_next().unwrap();
    let task = pool.join_set.spawn(async {}).id();
    let (control_tx, _control_rx) = tokio::sync::oneshot::channel();
    let (steer_tx, mut steer_rx) = mpsc::channel(1);
    pool.task_map_mut().insert(
        task,
        pool::TaskMeta {
            agent_index: 0,
            channel_id: Some(channel_id),
            scope: Some(scope.clone()),
            turn_id: "old-turn".into(),
            recoverable_batch: Some(batch),
            control_tx: Some(control_tx),
            steer_tx: Some(steer_tx),
            successful_steer_deliveries: HashSet::new(),
        },
    );
    let event = EventBuilder::new(Kind::Custom(9), "steered")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let (receipt, mut probe) = relay::DeliveryProbe::new(&event.id.to_hex(), 8);
    queue.push(QueuedEvent {
        delivery: Some(receipt),
        channel_id,
        scope: scope.clone(),
        event: event.clone(),
        received_at: std::time::Instant::now(),
        prompt_tag: "steer".into(),
    });
    let (acks, mut watcher_rx) = mpsc::unbounded_channel();
    assert!(try_native_steer(
        &mut pool,
        &mut queue,
        scope.clone(),
        event.clone(),
        "steer".into(),
        &acks
    ));
    let request = steer_rx.try_recv().unwrap();
    let ledger = pool
        .task_map()
        .get(&task)
        .unwrap()
        .recoverable_batch
        .as_ref()
        .unwrap();
    assert_eq!(ledger.events.len(), 2);
    ledger.events[1].delivery.as_ref().unwrap().completed();
    assert!(probe.outcomes().is_empty()); // mailbox admission != completion
    request.delivery.as_ref().unwrap().mark_native_injected(); // real ACP test binds this read-loop transition
    let _ = request.ack_tx.send(pool::SteerAck::Success {
        session_id: "session".into(),
    });
    // Complete directly from the ORIGINAL ledger even with no watcher processing.
    ledger.events[1].delivery.as_ref().unwrap().completed();
    assert_eq!(probe.outcomes(), vec!["completed"]);
    pool.task_map_mut().get_mut(&task).unwrap().turn_id = "successor".into();
    assert!(!pool.record_successful_steer_for_turn(
        &scope,
        "old-turn",
        event.id.to_hex(),
        "session".into()
    ));
    assert!(pool
        .task_map()
        .get(&task)
        .unwrap()
        .successful_steer_deliveries
        .is_empty());
    let ack = tokio::time::timeout(Duration::from_secs(1), watcher_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ack.turn_id, "old-turn");
}

#[tokio::test]
async fn receipt_queue_transformations_and_overflow_remain_nonterminal() {
    let ch = Uuid::new_v4();
    let scope = scope::SessionScope::Conversation { channel_id: ch };
    let mut q = EventQueue::new(config::DedupMode::Queue);
    let event = EventBuilder::new(Kind::Custom(9), "transform")
        .sign_with_keys(&Keys::generate())
        .unwrap();
    let (receipt, mut probe) = relay::DeliveryProbe::new(&event.id.to_hex(), 8);
    q.push(QueuedEvent {
        delivery: Some(receipt),
        channel_id: ch,
        scope: scope.clone(),
        event: event.clone(),
        received_at: std::time::Instant::now(),
        prompt_tag: "test".into(),
    });
    let batch = q.flush_next().unwrap();
    q.requeue_preserve_timestamps(batch);
    q.mark_complete(&scope);
    let batch = q.flush_next().unwrap();
    q.requeue_as_cancelled(batch, queue::CancelReason::Steer);
    q.mark_complete(&scope);
    let batch = q.flush_next().unwrap();
    assert!(batch.events[0].delivery.is_some());
    q.requeue_preserve_timestamps(batch);
    q.mark_complete(&scope);
    assert!(q.mark_native_steer_pending(&scope, &event.id.to_hex()));
    q.release_native_steer(&scope, &event.id.to_hex());
    let batch = q.flush_next().unwrap();
    assert_eq!(batch.events[0].event.id, event.id);
    assert!(batch.events[0].delivery.is_some());
    assert!(probe.outcomes().is_empty());
    q.requeue_preserve_timestamps(batch);
    q.mark_complete(&scope);
    for i in 0..501 {
        let event = EventBuilder::new(Kind::Custom(9), format!("overflow-{i}"))
            .sign_with_keys(&Keys::generate())
            .unwrap();
        q.push(QueuedEvent {
            delivery: None,
            channel_id: ch,
            scope: scope.clone(),
            event,
            received_at: std::time::Instant::now(),
            prompt_tag: "test".into(),
        });
    }
    assert!(
        q.delivery_event(&scope, &event.id.to_hex()).is_none(),
        "characterize existing oldest eviction"
    );
    assert!(
        probe.outcomes().is_empty(),
        "overflow is NOT deliberate policy rejection"
    );
}

#[tokio::test]
async fn receipt_drop_mode_panic_does_not_requeue_completion_only_ledger() {
    for mode in [config::DedupMode::Drop, config::DedupMode::Queue] {
        let ch = Uuid::new_v4();
        let scope = scope::SessionScope::Conversation { channel_id: ch };
        let mut queue = EventQueue::new(mode);
        let event = EventBuilder::new(Kind::Custom(9), "panic receipt")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let (receipt, mut probe) = relay::DeliveryProbe::new(&event.id.to_hex(), 8);
        receipt.accepted();
        queue.push(QueuedEvent {
            delivery: Some(receipt),
            channel_id: ch,
            scope: scope.clone(),
            event,
            received_at: std::time::Instant::now(),
            prompt_tag: "test".into(),
        });
        let batch = queue.flush_next().unwrap();
        let mut pool = AgentPool::from_slots(vec![None]);
        let task = pool
            .join_set
            .spawn(async {
                panic!("intentional receipt regression");
            })
            .id();
        pool.task_map_mut().insert(
            task,
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(ch),
                scope: Some(scope.clone()),
                turn_id: "panic".into(),
                recoverable_batch: Some(batch),
                control_tx: None,
                steer_tx: None,
                successful_steer_deliveries: HashSet::new(),
            },
        );
        let error = pool.join_set.join_next().await.unwrap().unwrap_err();
        let mut config = test_config();
        config.dedup_mode = mode;
        let mut history = vec![SlotCircuit {
            crash_times: vec![],
            open_until: Some(std::time::Instant::now() + Duration::from_secs(3600)),
            respawn_in_flight: false,
        }];
        let (tx, _rx) = mpsc::channel(8);
        let mut tasks = tokio::task::JoinSet::new();
        recover_panicked_agent(
            &mut pool,
            &mut queue,
            &config,
            error,
            &mut false,
            &HashSet::new(),
            &mut HashMap::new(),
            &mut history,
            &tx,
            &mut tasks,
            None,
        );
        assert!(tasks.is_empty());
        assert_eq!(probe.outcomes(), vec!["accepted"]);
        assert_eq!(
            queue.has_undispatched_work(),
            matches!(mode, config::DedupMode::Queue)
        );
        assert!(!queue.is_scope_in_flight(&scope));
    }
}

/// The real ACP read loop resolves EndTurn and closes its steer mailbox while
/// PromptResult is still pending. Drive the late transport fallback BEFORE
/// consuming that result, without timing sleeps or a second disposition model.
#[tokio::test]
async fn receipt_end_turn_pending_result_survives_closed_steer_control() {
    for channel_control in [false, true] {
        let script = r#"
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":2,"_meta":{"steering":{"supported":true}}}}'
read -r line
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"outcome":"injected"}}'
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}'
while read -r line; do :; done
"#;
        let mut agent = dummy_agent(0).await;
        agent.acp.shutdown().await;
        agent.acp = AcpClient::spawn("bash", &["-c".into(), script.into()], &[], false)
            .await
            .unwrap();
        agent.acp.initialize().await.unwrap();
        let channel_id = Uuid::new_v4();
        let scope = scope::SessionScope::Conversation { channel_id };
        let mut queue = EventQueue::new(config::DedupMode::Queue);
        let mut events = vec![];
        let mut receipts = vec![];
        let mut probes = vec![];
        for name in ["original", "injected", "withheld", "late"] {
            let event = EventBuilder::new(Kind::Custom(9), name)
                .sign_with_keys(&Keys::generate())
                .unwrap();
            let (receipt, probe) = relay::DeliveryProbe::new(&event.id.to_hex(), 8);
            receipt.accepted();
            events.push(event);
            receipts.push(receipt);
            probes.push(probe);
        }
        let push = |queue: &mut EventQueue, i: usize| {
            queue.push(QueuedEvent {
                delivery: Some(receipts[i].clone()),
                channel_id,
                scope: scope.clone(),
                event: events[i].clone(),
                received_at: std::time::Instant::now(),
                prompt_tag: "test".into(),
            });
        };
        push(&mut queue, 0);
        let batch = queue.flush_next().unwrap();
        let mut pool = AgentPool::from_slots(vec![None]);
        let (control_tx, control_rx) = tokio::sync::oneshot::channel::<ControlSignal>();
        let (steer_tx, steer_rx) = mpsc::channel(1);
        agent.acp.install_steer_rx(steer_rx);
        let result_tx = pool.result_tx();
        let task_scope = scope.clone();
        let task = pool
            .join_set
            .spawn(async move {
                let stop = agent
                    .acp
                    .session_prompt_blocks_with_idle_timeout(
                        "session",
                        &["original"],
                        Duration::from_secs(2),
                        Duration::from_secs(2),
                    )
                    .await
                    .unwrap();
                assert!(matches!(stop, acp::StopReason::EndTurn));
                assert!(!agent.acp.has_in_flight_prompt());
                drop(control_rx);
                result_tx
                    .send(PromptResult {
                        agent,
                        source: PromptSource::Channel(task_scope),
                        turn_id: "completed-turn".into(),
                        outcome: PromptOutcome::Ok(stop),
                        batch: None,
                    })
                    .unwrap();
            })
            .id();
        pool.task_map_mut().insert(
            task,
            pool::TaskMeta {
                agent_index: 0,
                channel_id: Some(channel_id),
                scope: Some(scope.clone()),
                turn_id: "completed-turn".into(),
                recoverable_batch: Some(batch),
                control_tx: Some(control_tx),
                steer_tx: Some(steer_tx),
                successful_steer_deliveries: HashSet::new(),
            },
        );
        let (ack_tx, mut ack_rx) = mpsc::unbounded_channel();
        push(&mut queue, 1);
        assert!(try_native_steer(
            &mut pool,
            &mut queue,
            scope.clone(),
            events[1].clone(),
            "test".into(),
            &ack_tx
        ));
        let injected_ack = tokio::time::timeout(Duration::from_secs(3), ack_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            injected_ack.ack,
            Ok(pool::SteerAck::Success { .. })
        ));
        // Deliberately do not process the success watcher in the main loop:
        // receipt eligibility is owned by the read loop, not watcher timing.
        push(&mut queue, 2);
        assert!(try_native_steer(
            &mut pool,
            &mut queue,
            scope.clone(),
            events[2].clone(),
            "test".into(),
            &ack_tx
        ));
        tokio::time::timeout(Duration::from_secs(3), pool.join_set.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let withheld_ack = tokio::time::timeout(Duration::from_secs(3), ack_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            withheld_ack.ack,
            Ok(pool::SteerAck::PromptCompletedNeutral)
        ));
        for probe in &mut probes {
            assert_eq!(
                probe.outcomes(),
                vec!["accepted"],
                "no completion before result disposition"
            );
        }
        // The task is still registered; its result has not been consumed.
        push(&mut queue, 3);
        assert!(!try_native_steer(
            &mut pool,
            &mut queue,
            scope.clone(),
            events[3].clone(),
            "test".into(),
            &ack_tx
        ));
        assert!(if channel_control {
            signal_in_flight_task(&mut pool, channel_id, ControlSignal::Cancel)
        } else {
            signal_in_flight_task_for_scope(&mut pool, &scope, ControlSignal::Steer)
        });
        assert!(pool.task_map()[&task].control_tx.is_none());
        let result = pool.result_rx_try_recv().unwrap();
        let mut history = vec![SlotCircuit {
            crash_times: vec![],
            open_until: None,
            respawn_in_flight: false,
        }];
        let (respawn_tx, _respawn_rx) = mpsc::channel(8);
        let mut tasks = tokio::task::JoinSet::new();
        handle_prompt_result(
            &mut pool,
            &mut queue,
            &test_config(),
            result,
            &mut false,
            &HashSet::new(),
            &mut history,
            &respawn_tx,
            &mut tasks,
            None,
            None,
        );
        // Explicitly reap the owned scripted subprocess before assertions that
        // deliberately fail on the old production predicate (red regression).
        let mut returned = pool.try_claim(Some(&scope)).unwrap();
        returned.acp.shutdown().await;
        assert!(tasks.is_empty());
        assert!(pool.task_map().is_empty());
        for (i, probe) in probes.iter_mut().enumerate() {
            assert_eq!(
                probe.outcomes(),
                if i < 2 { vec!["completed"] } else { vec![] },
                "receipt {i}"
            );
        }
        // Repeated attempts cannot duplicate the local terminal disposition.
        receipts[0].completed();
        receipts[1].completed();
        assert!(probes[0].outcomes().is_empty());
        assert!(probes[1].outcomes().is_empty());
        queue.remove_event(&scope, &events[1].id.to_hex());
        queue.release_native_steer(&scope, &events[2].id.to_hex());
        let next = queue.flush_next().unwrap();
        let ids: HashSet<_> = next.events.iter().map(|e| e.event.id).collect();
        assert_eq!(ids, HashSet::from([events[2].id, events[3].id]));
    }
}
