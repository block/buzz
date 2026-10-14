//! Retained follow-ups share the existing channel-wide overflow budget.
use super::*;

fn thread(channel_id: Uuid, root: u8) -> SessionScope {
    SessionScope::Thread {
        channel_id,
        root_event_id: format!("{root:064x}"),
    }
}

fn event(scope: &SessionScope, received_at: Instant, index: usize) -> QueuedEvent {
    QueuedEvent {
        channel_id: scope.channel_id(),
        scope: scope.clone(),
        event: nostr::EventBuilder::new(nostr::Kind::Custom(9), format!("event-{index}"))
            .sign_with_keys(&nostr::Keys::generate())
            .unwrap(),
        received_at,
        prompt_tag: "mention".into(),
        edit: None,
    }
}

fn pending_ids(queue: &EventQueue, channel_id: Uuid) -> HashSet<String> {
    queue
        .queues
        .iter()
        .filter(|(scope, _)| scope.channel_id() == channel_id)
        .flat_map(|(_, events)| events.iter().map(|event| event.event.id.to_hex()))
        .chain(
            queue
                .withheld_native_steer
                .iter()
                .filter(|(scope, _)| scope.channel_id() == channel_id)
                .flat_map(|(_, entries)| {
                    entries.iter().map(|entry| entry.queued.event.id.to_hex())
                }),
        )
        .collect()
}

#[test]
fn mixed_queued_and_retained_scopes_evict_the_global_oldest_from_either_store() {
    for retained_oldest in [true, false] {
        let mut queue = EventQueue::new(DedupMode::Queue);
        let channel_id = Uuid::new_v4();
        let retained_scope = thread(channel_id, 1);
        let queued_scope = thread(channel_id, 2);
        let extra_scope = thread(channel_id, 3);
        let other_channel = Uuid::new_v4();
        let start = Instant::now();
        queue.push(event(&thread(other_channel, 1), start, 9999));
        let other_ids = pending_ids(&queue, other_channel);
        let mut victim = String::new();
        for index in 0..MAX_PENDING_PER_CHANNEL / 2 {
            // Reverse receipt order in the side table: the oldest retained
            // event is not necessarily its first entry.
            let age = 250 - index + if retained_oldest { 0 } else { 1000 };
            let queued = event(
                &retained_scope,
                start + Duration::from_millis(age as u64),
                index,
            );
            let id = queued.event.id.to_hex();
            queue.push(queued);
            assert!(queue.can_native_steer(&retained_scope, &id));
            assert!(queue.mark_native_steer_pending(&retained_scope, &id, "turn"));
            assert!(queue.acknowledge_native_steer(&retained_scope, &id));
            if retained_oldest {
                victim = id;
            }
        }
        for index in 0..MAX_PENDING_PER_CHANNEL / 2 {
            let age = index + if retained_oldest { 1000 } else { 0 };
            let queued = event(
                &queued_scope,
                start + Duration::from_millis(age as u64),
                500 + index,
            );
            if !retained_oldest && index == 0 {
                victim = queued.event.id.to_hex();
            }
            queue.push(queued);
        }
        let before = pending_ids(&queue, channel_id);
        assert_eq!(before.len(), MAX_PENDING_PER_CHANNEL);
        assert_eq!(queue.channel_event_total(channel_id), before.len());
        let extra = event(&extra_scope, start + Duration::from_secs(5), 1000);
        let extra_id = extra.event.id.to_hex();
        queue.push(extra);
        let after = pending_ids(&queue, channel_id);
        assert_eq!(
            after.len(),
            MAX_PENDING_PER_CHANNEL,
            "retention cannot double the channel budget"
        );
        assert!(!after.contains(&victim));
        assert!(after.contains(&extra_id));
        assert_eq!(
            before.difference(&after).cloned().collect::<Vec<_>>(),
            vec![victim.clone()]
        );
        assert_eq!(pending_ids(&queue, other_channel), other_ids);
        if retained_oldest {
            assert!(!queue.acknowledge_native_steer(&retained_scope, &victim));
            queue.release_native_steer(&retained_scope, &victim);
            assert_eq!(
                pending_ids(&queue, channel_id),
                after,
                "late ACK/release cannot recreate evicted work"
            );
        }
        queue.finish_native_steers(&retained_scope, "turn", false);
        assert_eq!(
            pending_ids(&queue, channel_id),
            after,
            "failure release must not cause mass eviction"
        );
        assert_eq!(
            queue.channel_event_total(channel_id),
            MAX_PENDING_PER_CHANNEL
        );
        assert!(!queue.withheld_native_steer.contains_key(&retained_scope));
    }
}

#[tokio::test]
async fn a_full_retained_channel_can_transfer_admitted_work_but_never_steers_an_evicted_event() {
    let mut queue = EventQueue::new(DedupMode::Queue);
    let channel_id = Uuid::new_v4();
    let retained_scope = thread(channel_id, 1);
    let current_scope = thread(channel_id, 2);
    let start = Instant::now();
    for index in 0..MAX_PENDING_PER_CHANNEL {
        let queued = event(
            &retained_scope,
            start + Duration::from_millis(index as u64),
            index,
        );
        let id = queued.event.id.to_hex();
        queue.push(queued);
        assert!(queue.can_native_steer(&retained_scope, &id));
        assert!(queue.mark_native_steer_pending(&retained_scope, &id, "turn"));
    }
    let next = event(&current_scope, start + Duration::from_secs(5), 1000);
    let next_id = next.event.id.to_hex();
    queue.push(next);
    assert_eq!(
        pending_ids(&queue, channel_id).len(),
        MAX_PENDING_PER_CHANNEL
    );
    assert!(
        queue.can_native_steer(&current_scope, &next_id),
        "transfer at the cap adds no records"
    );
    assert!(queue.mark_native_steer_pending(&current_scope, &next_id, "next-turn"));
    assert_eq!(
        pending_ids(&queue, channel_id).len(),
        MAX_PENDING_PER_CHANNEL
    );

    let evicted = event(&current_scope, start - Duration::from_secs(1), 1001);
    let evicted_id = evicted.event.id.to_hex();
    let batch_event = evicted.clone().into_batch_event();
    queue.push(evicted);
    assert!(!pending_ids(&queue, channel_id).contains(&evicted_id));
    assert!(!queue.can_native_steer(&current_scope, &evicted_id));

    let mut pool = crate::pool::AgentPool::from_slots(vec![None]);
    pool.record_scope_owner(current_scope.clone(), 0);
    let id = pool.join_set.spawn(async {}).id();
    let (steer_tx, mut steer_rx) = tokio::sync::mpsc::channel(1);
    pool.task_map_mut().insert(
        id,
        crate::pool::TaskMeta {
            agent_index: 0,
            channel_id: Some(channel_id),
            scope: Some(current_scope.clone()),
            turn_id: "next-turn".into(),
            recoverable_batch: None,
            control_tx: None,
            steer_tx: Some(steer_tx),
            successful_steer_deliveries: HashSet::new(),
        },
    );
    let (ack_tx, _ack_rx) = tokio::sync::mpsc::unbounded_channel();
    assert!(!crate::try_native_steer(
        &mut pool,
        &mut queue,
        current_scope,
        batch_event,
        &ack_tx
    ));
    assert!(
        steer_rx.try_recv().is_err(),
        "no unretained payload may reach the ACP worker"
    );
    pool.join_set.shutdown().await;
}
