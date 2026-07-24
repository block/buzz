//! In-memory [`Transport`] for tests of transport consumers.
//!
//! Gated behind `#[cfg(any(test, feature = "test-utils"))]` — enable the
//! `test-utils` feature from a dependent crate's dev-dependencies to drive a
//! consumer with scripted inbound events and observe everything it publishes.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{BoxFuture, SignedEvent, Subscription, Transport, TransportError, TransportEvent};

/// Shared observation log between [`MockTransport`] and [`MockTransportHandle`].
#[derive(Default)]
struct MockLog {
    subscriptions: Mutex<Vec<Subscription>>,
    unsubscribed: Mutex<Vec<Uuid>>,
    published: Mutex<VecDeque<SignedEvent>>,
    reconnects: AtomicUsize,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    // A poisoned lock only means a panicking test thread — the data is
    // plain Vec/VecDeque state and safe to keep using.
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Test-side controller for a [`MockTransport`].
pub struct MockTransportHandle {
    event_tx: mpsc::Sender<Option<TransportEvent>>,
    log: Arc<MockLog>,
}

impl MockTransportHandle {
    /// Deliver an inbound event to the consumer.
    pub async fn inject(&self, event: TransportEvent) {
        let _ = self.event_tx.send(Some(event)).await;
    }

    /// Simulate connection loss: the consumer's `next_event` yields `None`.
    pub async fn drop_connection(&self) {
        let _ = self.event_tx.send(None).await;
    }

    /// All subscriptions the consumer has registered, in order.
    pub fn subscriptions(&self) -> Vec<Subscription> {
        lock(&self.log.subscriptions).clone()
    }

    /// All channels the consumer has unsubscribed, in order.
    pub fn unsubscribed(&self) -> Vec<Uuid> {
        lock(&self.log.unsubscribed).clone()
    }

    /// Pop the oldest event the consumer published, if any.
    pub fn next_published(&self) -> Option<SignedEvent> {
        lock(&self.log.published).pop_front()
    }

    /// How many times the consumer called `reconnect`.
    pub fn reconnects(&self) -> usize {
        self.log.reconnects.load(Ordering::SeqCst)
    }
}

/// In-memory [`Transport`] with scripted inbound events and observable
/// outbound state.
pub struct MockTransport {
    event_rx: mpsc::Receiver<Option<TransportEvent>>,
    log: Arc<MockLog>,
}

impl MockTransport {
    /// Build a transport plus the handle that scripts and observes it.
    pub fn pair() -> (Self, MockTransportHandle) {
        let (event_tx, event_rx) = mpsc::channel(64);
        let log = Arc::new(MockLog::default());
        (
            Self {
                event_rx,
                log: Arc::clone(&log),
            },
            MockTransportHandle { event_tx, log },
        )
    }
}

impl Transport for MockTransport {
    fn subscribe(
        &mut self,
        subscription: Subscription,
    ) -> BoxFuture<'_, Result<(), TransportError>> {
        lock(&self.log.subscriptions).push(subscription);
        Box::pin(async { Ok(()) })
    }

    fn unsubscribe(&mut self, channel_id: Uuid) -> BoxFuture<'_, Result<(), TransportError>> {
        lock(&self.log.unsubscribed).push(channel_id);
        Box::pin(async { Ok(()) })
    }

    fn next_event(&mut self) -> BoxFuture<'_, Option<TransportEvent>> {
        Box::pin(async move { self.event_rx.recv().await.flatten() })
    }

    fn publish(&self, event: SignedEvent) -> BoxFuture<'_, Result<(), TransportError>> {
        lock(&self.log.published).push_back(event);
        Box::pin(async { Ok(()) })
    }

    fn try_publish(&self, event: SignedEvent) -> Result<(), TransportError> {
        lock(&self.log.published).push_back(event);
        Ok(())
    }

    fn reconnect(&mut self) -> BoxFuture<'_, Result<(), TransportError>> {
        self.log.reconnects.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }

    fn shutdown(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind};

    use super::*;

    fn signed_event(content: &str) -> SignedEvent {
        let event = EventBuilder::new(Kind::Custom(9), content)
            .sign_with_keys(&Keys::generate())
            .unwrap();
        SignedEvent::from_nostr(&event).unwrap()
    }

    #[tokio::test]
    async fn scripted_roundtrip_through_the_trait() {
        let (transport, handle) = MockTransport::pair();
        let mut transport: Box<dyn Transport> = Box::new(transport);
        let channel_id = Uuid::new_v4();

        transport
            .subscribe(Subscription::all(channel_id))
            .await
            .unwrap();
        assert_eq!(handle.subscriptions().len(), 1);

        handle
            .inject(TransportEvent {
                channel_id,
                event: signed_event("inbound"),
            })
            .await;
        let received = transport.next_event().await.unwrap();
        assert_eq!(received.channel_id, channel_id);
        assert_eq!(received.event.content, "inbound");

        transport.publish(signed_event("outbound")).await.unwrap();
        assert_eq!(
            handle.next_published().map(|e| e.content),
            Some("outbound".to_string())
        );

        handle.drop_connection().await;
        assert!(transport.next_event().await.is_none());
        transport.reconnect().await.unwrap();
        assert_eq!(handle.reconnects(), 1);

        transport.unsubscribe(channel_id).await.unwrap();
        assert_eq!(handle.unsubscribed(), vec![channel_id]);
        transport.shutdown().await;
    }
}
