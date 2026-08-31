use super::*;
use tokio::sync::{broadcast, oneshot};

pub(super) struct RelayObserverPublisherTask {
    shutdown_tx: oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<Result<(), String>>,
}

impl RelayObserverPublisherTask {
    pub(super) async fn shutdown(self) -> Result<(), String> {
        let _ = self.shutdown_tx.send(());
        self.handle
            .await
            .map_err(|error| format!("observer publisher join: {error}"))?
    }
}

pub(super) fn spawn_relay_observer_publisher(
    observer: observer::ObserverHandle,
    publisher: RelayEventPublisher,
    keys: nostr::Keys,
    agent_pubkey_hex: String,
    owner_pubkey_hex: String,
    owner_pubkey: PublicKey,
) -> RelayObserverPublisherTask {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let subscription = observer.subscribe_with_snapshot();
        run_relay_observer_publisher_until(
            subscription,
            publisher,
            keys,
            agent_pubkey_hex,
            owner_pubkey_hex,
            owner_pubkey,
            Some(shutdown_rx),
        )
        .await
    });
    RelayObserverPublisherTask {
        shutdown_tx,
        handle,
    }
}

#[cfg(test)]
pub(super) async fn run_relay_observer_publisher(
    subscription: (
        Vec<observer::ObserverEvent>,
        broadcast::Receiver<observer::ObserverEvent>,
        u64,
    ),
    publisher: RelayEventPublisher,
    keys: nostr::Keys,
    agent_pubkey_hex: String,
    owner_pubkey_hex: String,
    owner_pubkey: PublicKey,
) {
    run_relay_observer_publisher_until(
        subscription,
        publisher,
        keys,
        agent_pubkey_hex,
        owner_pubkey_hex,
        owner_pubkey,
        None,
    )
    .await
    .expect("observer publisher");
}

async fn run_relay_observer_publisher_until(
    subscription: (
        Vec<observer::ObserverEvent>,
        broadcast::Receiver<observer::ObserverEvent>,
        u64,
    ),
    publisher: RelayEventPublisher,
    keys: nostr::Keys,
    agent_pubkey_hex: String,
    owner_pubkey_hex: String,
    owner_pubkey: PublicKey,
    mut shutdown_rx: Option<oneshot::Receiver<()>>,
) -> Result<(), String> {
    let (snapshot, mut rx, replay_dropped) = subscription;
    let mut queue = ObserverPublishQueue::default();
    queue.note_gap(ObserverGapReason::ReplayBufferOverflow, replay_dropped);
    let max_snapshot_seq = snapshot.iter().map(|event| event.seq).max().unwrap_or(0);
    for event in snapshot {
        queue.ingest(event);
    }

    let mut publish_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + OBSERVER_PUBLISH_TICK,
        OBSERVER_PUBLISH_TICK,
    );
    publish_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut closed = false;
    loop {
        tokio::select! {
            _ = wait_for_shutdown(&mut shutdown_rx), if !closed => {
                drain_receiver(&mut rx, &mut queue, max_snapshot_seq);
                closed = true;
            }
            result = rx.recv(), if !closed => match result {
                Ok(event) if event.seq > max_snapshot_seq => queue.ingest(event),
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    queue.note_gap(ObserverGapReason::BroadcastLag, count);
                    tracing::warn!(dropped = count, "relay observer publisher lagged");
                }
                Err(broadcast::error::RecvError::Closed) => closed = true,
            },
            _ = publish_tick.tick() => {
                if let Some(frame) = queue.next_frame() {
                    let result = publish_relay_observer_event(
                        &publisher, &keys, &agent_pubkey_hex,
                        &owner_pubkey_hex, &owner_pubkey, frame.event,
                    ).await;
                    if result.is_err() {
                        queue.restore_gaps(frame.reported_gaps);
                        queue.note_gap(ObserverGapReason::PublishFailure, frame.source_events);
                    }
                }
                if closed && queue.is_empty() {
                    publisher.flush_observer().await.map_err(|error| {
                        format!("flush observer publisher: {error}")
                    })?;
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn wait_for_shutdown(shutdown_rx: &mut Option<oneshot::Receiver<()>>) {
    if let Some(receiver) = shutdown_rx {
        let _ = receiver.await;
    } else {
        std::future::pending::<()>().await;
    }
}

fn drain_receiver(
    rx: &mut broadcast::Receiver<observer::ObserverEvent>,
    queue: &mut ObserverPublishQueue,
    max_snapshot_seq: u64,
) {
    loop {
        match rx.try_recv() {
            Ok(event) if event.seq > max_snapshot_seq => queue.ingest(event),
            Ok(_) => {}
            Err(broadcast::error::TryRecvError::Lagged(count)) => {
                queue.note_gap(ObserverGapReason::BroadcastLag, count);
            }
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn explicit_shutdown_drains_and_flushes_before_join() {
        let observer = observer::ObserverHandle::in_process();
        let agent_keys = nostr::Keys::generate();
        let owner_keys = nostr::Keys::generate();
        let (publisher, mut published_rx) = RelayEventPublisher::test_pair();

        for channel in [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()] {
            observer.emit(
                "test_event",
                None,
                &observer::context_for(Some(channel), None, None),
                serde_json::json!({ "channel": channel }),
            );
        }
        let task = spawn_relay_observer_publisher(
            observer.clone(),
            publisher,
            agent_keys.clone(),
            agent_keys.public_key().to_hex(),
            owner_keys.public_key().to_hex(),
            owner_keys.public_key(),
        );
        let shutdown = tokio::spawn(task.shutdown());

        tokio::task::yield_now().await;
        assert!(published_rx.try_recv().is_err(), "shutdown must not burst");
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;

        shutdown.await.expect("shutdown join").expect("flush");
        let mut published = 0;
        while published_rx.try_recv().is_ok() {
            published += 1;
        }
        assert_eq!(published, 2, "every queued channel frame is flushed");
    }
}
