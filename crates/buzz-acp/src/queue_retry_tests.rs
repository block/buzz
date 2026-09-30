use super::*;
use nostr::{EventBuilder, Keys, Kind, Timestamp};

#[test]
fn cancelled_only_work_obeys_backoff_but_remains_runnable_without_it() {
    for deadline in [
        None,
        Some(Instant::now()),
        Some(Instant::now() + Duration::from_secs(60)),
    ] {
        let scope = SessionScope::Conversation {
            channel_id: Uuid::new_v4(),
        };
        let mut queue = EventQueue::new(DedupMode::Queue);
        queue.push(queued(&scope, "cancelled-only work", 1));
        let batch = queue.flush_next().unwrap();
        queue.requeue_as_cancelled(batch, CancelReason::Steer);
        queue.mark_complete(&scope);
        if let Some(deadline) = deadline {
            queue.retry_after.insert(scope.clone(), deadline);
        }
        let ready = deadline.is_none_or(|deadline| deadline <= Instant::now());
        assert!(
            queue.has_undispatched_work(),
            "backoff does not discard pending work"
        );
        assert_eq!(
            queue.has_flushable_work(),
            ready,
            "cancelled-only readiness must respect deadline"
        );
        let flushed = queue.flush_next();
        assert_eq!(
            flushed.is_some(),
            ready,
            "cancelled-only dispatch must respect deadline"
        );
        if let Some(batch) = flushed {
            assert_eq!(batch.events[0].event.content, "cancelled-only work");
        }
    }
}

fn queued(scope: &SessionScope, text: &str, created_at: u64) -> QueuedEvent {
    QueuedEvent {
        channel_id: scope.channel_id(),
        scope: scope.clone(),
        event: EventBuilder::new(Kind::Custom(9), text)
            .custom_created_at(Timestamp::from(created_at))
            .sign_with_keys(&Keys::generate())
            .unwrap(),
        prompt_tag: "@mention".into(),
        received_at: Instant::now(),
    }
}

#[test]
fn failed_merge_retains_arrivals_and_sibling_scope_without_multiplying_carryover() {
    let channel_id = Uuid::new_v4();
    let scope = SessionScope::Thread {
        channel_id,
        root_event_id: "a".repeat(64),
    };
    let sibling = SessionScope::Thread {
        channel_id,
        root_event_id: "b".repeat(64),
    };
    let mut queue = EventQueue::new(DedupMode::Queue);
    queue.push(queued(&scope, "original A", 1));
    let original = queue.flush_next().unwrap();
    queue.push(queued(&scope, "new B", 2));
    queue.requeue_as_cancelled(original, CancelReason::Steer);
    queue.mark_complete(&scope);
    let batch = queue.flush_next().unwrap();
    // New arrivals during the failed attempt must remain after restored B.
    queue.push(queued(&scope, "new C", 3));
    queue.push(queued(&sibling, "unrelated thread", 4));
    assert!(queue.requeue(batch).is_none());
    queue.mark_complete(&scope);
    let other = queue.flush_next().unwrap();
    assert_eq!(
        other.scope, sibling,
        "backoff must not block another thread"
    );
    assert!(other.cancelled_events.is_empty());
    queue.mark_complete(&sibling);
    for attempt in 1..=3 {
        assert_eq!(queue.retry_count(&scope), attempt);
        queue.expire_retry_for_test(&scope);
        let retry = queue.flush_next().unwrap();
        assert_eq!(retry.cancelled_events.len(), 1);
        assert_eq!(retry.cancelled_events[0].event.content, "original A");
        assert_eq!(
            retry
                .events
                .iter()
                .map(|event| event.event.content.as_str())
                .collect::<Vec<_>>(),
            ["new B", "new C"]
        );
        assert_eq!(retry.cancel_reason, Some(CancelReason::Steer));
        assert!(queue.requeue(retry).is_none());
        queue.mark_complete(&scope);
    }
    queue.drain_channel(channel_id);
    assert!(queue.cancelled_batches.is_empty());
    assert!(queue.cancel_reasons.is_empty());
    assert!(queue.retry_counts.is_empty());
    assert!(queue.retry_after.is_empty());
}

#[test]
fn failure_restoration_preserves_existing_pending_cap_and_latest_cancel_reason() {
    let scope = SessionScope::Conversation {
        channel_id: Uuid::new_v4(),
    };
    let mut queue = EventQueue::new(DedupMode::Queue);
    queue.push(queued(&scope, "original A", 1));
    let original = queue.flush_next().unwrap();
    queue.push(queued(&scope, "new B", 2));
    queue.requeue_as_cancelled(original, CancelReason::Steer);
    queue.mark_complete(&scope);
    let batch = queue.flush_next().unwrap();
    for index in 0..MAX_PENDING_PER_SCOPE {
        queue.push(queued(
            &scope,
            &format!("arrival {index}"),
            index as u64 + 3,
        ));
    }
    // A newer cancellation reason must not be replaced by older retry metadata.
    queue
        .cancel_reasons
        .insert(scope.clone(), CancelReason::Interrupt);
    assert!(queue.requeue(batch).is_none());
    assert_eq!(queue.queued_event_count(&scope), MAX_PENDING_PER_SCOPE);
    assert_eq!(queue.queues[&scope].front().unwrap().event.content, "new B");
    assert_eq!(queue.cancelled_batches[&scope].len(), 1);
    assert_eq!(queue.cancel_reasons[&scope], CancelReason::Interrupt);
}
