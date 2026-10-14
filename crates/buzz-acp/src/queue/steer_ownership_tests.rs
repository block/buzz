//! Real queue expiry and production admission/ACK/result seams, without a journal.
use super::*;
use crate::error_outcome_emission_tests::{bind_agent_scope_owner, dummy_agent, test_config};
use crate::*;

fn event(scope: &SessionScope) -> QueuedEvent {
    QueuedEvent {
        channel_id: scope.channel_id(),
        scope: scope.clone(),
        event: nostr::EventBuilder::new(nostr::Kind::Custom(9), "follow-up")
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap(),
        received_at: Instant::now(),
        prompt_tag: "mention".into(),
        edit: None,
    }
}

fn task(
    pool: &mut AgentPool,
    agent: &mut OwnedAgent,
    scope: &SessionScope,
    turn: &str,
) -> (
    tokio::task::Id,
    mpsc::Receiver<pool::SteerRequest>,
    tokio::sync::oneshot::Receiver<ControlSignal>,
) {
    bind_agent_scope_owner(pool, agent, scope.clone());
    let id = pool.join_set.spawn(std::future::pending()).id();
    let (steer_tx, rx) = mpsc::channel(1);
    let (control_tx, control_rx) = tokio::sync::oneshot::channel();
    pool.task_map_mut().insert(
        id,
        pool::TaskMeta {
            agent_index: agent.index,
            channel_id: Some(scope.channel_id()),
            scope: Some(scope.clone()),
            turn_id: turn.into(),
            recoverable_batch: None,
            control_tx: Some(control_tx),
            steer_tx: Some(steer_tx),
            successful_steer_deliveries: HashSet::new(),
        },
    );
    (id, rx, control_rx)
}

async fn admit(
    pool: &mut AgentPool,
    queue: &mut EventQueue,
    event: QueuedEvent,
    rx: &mut mpsc::Receiver<pool::SteerRequest>,
) -> (pool::SteerRequest, mpsc::UnboundedReceiver<SteerAckEvent>) {
    let scope = event.scope.clone();
    let batch_event = BatchEvent {
        event: event.event.clone(),
        received_at: event.received_at,
        prompt_tag: event.prompt_tag.clone(),
        edit: None,
    };
    queue.push(event);
    let (tx, ack_rx) = mpsc::unbounded_channel();
    assert!(try_native_steer(pool, queue, scope, batch_event, &tx));
    (
        rx.try_recv().expect("current task receives the steer"),
        ack_rx,
    )
}

fn circuit() -> SlotCircuit {
    SlotCircuit {
        crash_times: vec![],
        open_until: None,
        respawn_in_flight: false,
    }
}

#[tokio::test]
async fn expired_turn_result_or_task_failure_cannot_settle_replacement_follow_up() {
    for (panic, invalidate_owner) in [(false, false), (true, false), (false, true), (true, true)] {
        let scope = SessionScope::Conversation {
            channel_id: Uuid::new_v4(),
        };
        let mut queue = EventQueue::new(DedupMode::Queue);
        queue.push(event(&scope));
        let _original = queue.flush_next().unwrap();
        let mut old = dummy_agent(0).await;
        let mut next = dummy_agent(1).await;
        let mut pool = AgentPool::from_slots(vec![None, None]);
        let (old_id, mut old_rx, _old_control) = task(&mut pool, &mut old, &scope, "old");
        let (pending, _old_ack) = admit(&mut pool, &mut queue, event(&scope), &mut old_rx).await;
        queue
            .in_flight_deadlines
            .insert(scope.clone(), Instant::now());
        let recovered = queue
            .flush_next()
            .expect("real expiry recovers original steer");
        assert_eq!(recovered.events.len(), 1);
        let (_, mut next_rx, _next_control) = task(&mut pool, &mut next, &scope, "next");
        let follow_up = event(&scope);
        let follow_up_id = follow_up.event.id.to_hex();
        let (accepted, mut ack_rx) = admit(&mut pool, &mut queue, follow_up, &mut next_rx).await;
        accepted
            .ack_tx
            .send(pool::SteerAck::Success {
                session_id: "next-session".into(),
            })
            .unwrap();
        let ack = tokio::time::timeout(Duration::from_secs(2), ack_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ack.turn_id, "next");
        handle_steer_ack(&mut pool, &mut queue, ack, 60);
        if invalidate_owner {
            pool.invalidate_scope_session(&scope);
            let (ack_tx, _ack_rx) = tokio::sync::oneshot::channel();
            assert!(matches!(
                pool.send_steer(
                    &scope,
                    pool::SteerRequest {
                        prompt_blocks: vec!["ambiguous".into()],
                        ack_tx,
                    }
                ),
                Err(pool::SteerError::PromptCompleted)
            ));
            assert!(!signal_in_flight_task_for_scope(
                &mut pool,
                &scope,
                ControlSignal::Steer
            ));
            assert!(old_rx.try_recv().is_err());
            assert!(next_rx.try_recv().is_err());
            assert!(pool
                .task_map()
                .values()
                .all(|meta| meta.control_tx.is_some()));
        }
        let mut history = [circuit(), circuit()];
        let (tx, _rx) = mpsc::channel(4);
        let mut respawns = tokio::task::JoinSet::new();
        if panic {
            // Preserve the real task identity so production panic cleanup runs.
            pool.join_set.abort_all();
            let mut old_error = None;
            while let Some(joined) = pool.join_set.join_next().await {
                let error = joined.unwrap_err();
                if error.id() == old_id {
                    old_error = Some(error);
                }
            }
            let mut typing = HashMap::from([(scope.clone(), ThreadTags::default())]);
            recover_panicked_agent(
                &mut pool,
                &mut queue,
                &test_config(),
                old_error.unwrap(),
                &mut false,
                &HashSet::new(),
                &mut typing,
                &mut history,
                &tx,
                &mut respawns,
                None,
            );
            assert!(
                typing.contains_key(&scope),
                "old panic must not clear replacement typing"
            );
            old.acp.shutdown().await;
        } else {
            handle_prompt_result(
                &mut pool,
                &mut queue,
                &test_config(),
                PromptResult {
                    agent: old,
                    source: PromptSource::Channel(scope.clone()),
                    turn_id: "old".into(),
                    outcome: PromptOutcome::Ok(acp::StopReason::EndTurn),
                    batch: None,
                },
                &mut false,
                &HashSet::new(),
                &mut history,
                &tx,
                &mut respawns,
                None,
                None,
                &mut tokio::time::Instant::now(),
            );
        }
        assert!(
            queue.is_scope_in_flight(&scope),
            "old completion cannot clear replacement guard"
        );
        assert!(
            queue.owns_native_steer(&scope, &follow_up_id, "next"),
            "old completion cannot discard new follow-up"
        );
        assert!(queue.flush_next().is_none());
        next.acp.shutdown().await;
        handle_prompt_result(
            &mut pool,
            &mut queue,
            &test_config(),
            PromptResult {
                agent: next,
                source: PromptSource::Channel(scope.clone()),
                turn_id: "next".into(),
                outcome: PromptOutcome::AgentExited,
                batch: None,
            },
            &mut false,
            &HashSet::new(),
            &mut history,
            &tx,
            &mut respawns,
            None,
            None,
            &mut tokio::time::Instant::now(),
        );
        let retry = queue
            .flush_next()
            .expect("replacement failure retains accepted follow-up");
        assert_eq!(retry.events.len(), 1);
        assert_eq!(retry.events[0].event.id.to_hex(), follow_up_id);
        drop(pending);
        respawns.shutdown().await;
        shutdown_agent_pool(&mut pool).await;
    }
}

#[tokio::test]
async fn expired_turn_ack_cannot_settle_or_cancel_the_same_event_in_a_new_turn() {
    for late_ack in [
        pool::SteerAck::Success {
            session_id: "old-session".into(),
        },
        pool::SteerAck::Err(pool::SteerError::Transport("fixture failure".into())),
        pool::SteerAck::PromptCompletedNeutral,
    ] {
        let scope = SessionScope::Conversation {
            channel_id: Uuid::new_v4(),
        };
        let mut queue = EventQueue::new(DedupMode::Queue);
        queue.push(event(&scope));
        queue.flush_next().unwrap();
        let mut old = dummy_agent(0).await;
        let mut next = dummy_agent(1).await;
        let mut pool = AgentPool::from_slots(vec![None, None]);
        let (_, mut old_rx, mut old_control) = task(&mut pool, &mut old, &scope, "old");
        let follow_up = event(&scope);
        let follow_up_id = follow_up.event.id.to_hex();
        let (pending, mut old_ack_rx) =
            admit(&mut pool, &mut queue, follow_up.clone(), &mut old_rx).await;
        queue
            .in_flight_deadlines
            .insert(scope.clone(), Instant::now());
        queue.flush_next().unwrap();
        let (_, mut next_rx, mut next_control) = task(&mut pool, &mut next, &scope, "next");
        let (accepted, mut next_ack_rx) =
            admit(&mut pool, &mut queue, follow_up, &mut next_rx).await;
        let deadline = queue.in_flight_deadlines[&scope];
        pending.ack_tx.send(late_ack).unwrap();
        let ack = tokio::time::timeout(Duration::from_secs(2), old_ack_rx.recv())
            .await
            .unwrap()
            .unwrap();
        handle_steer_ack(&mut pool, &mut queue, ack, 7200);
        assert_eq!(queue.in_flight_deadlines[&scope], deadline);
        assert!(queue.owns_native_steer(&scope, &follow_up_id, "next"));
        assert!(matches!(
            next_control.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        // Completing this turn before its own ack cannot consume the stale ack.
        queue.finish_native_steers(&scope, "next", true);
        assert!(queue.owns_native_steer(&scope, &follow_up_id, "next"));
        accepted
            .ack_tx
            .send(pool::SteerAck::Success {
                session_id: "next-session".into(),
            })
            .unwrap();
        let ack = tokio::time::timeout(Duration::from_secs(2), next_ack_rx.recv())
            .await
            .unwrap()
            .unwrap();
        handle_steer_ack(&mut pool, &mut queue, ack, 60);
        assert!(!queue.owns_native_steer(&scope, &follow_up_id, "next"));
        let (rejected, mut fallback_rx) =
            admit(&mut pool, &mut queue, event(&scope), &mut next_rx).await;
        rejected
            .ack_tx
            .send(pool::SteerAck::Err(pool::SteerError::Transport(
                "fallback".into(),
            )))
            .unwrap();
        let ack = tokio::time::timeout(Duration::from_secs(2), fallback_rx.recv())
            .await
            .unwrap()
            .unwrap();
        handle_steer_ack(&mut pool, &mut queue, ack, 60);
        assert!(matches!(next_control.try_recv(), Ok(ControlSignal::Steer)));
        assert!(matches!(
            old_control.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        old.acp.shutdown().await;
        next.acp.shutdown().await;
        shutdown_agent_pool(&mut pool).await;
    }
}
