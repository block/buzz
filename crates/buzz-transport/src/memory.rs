//! In-process [`Transport`]: a hub fanning signed events out between
//! participants, entirely in memory.
//!
//! This is the reference implementation of the seam — every contract rule
//! ([`SignedEvent`] verification, `h`-tag channel scoping, subscription
//! matching, replace-on-resubscribe) in a few dozen lines with no I/O. Use
//! it to integration-test transport consumers against a *working* stream
//! (several participants exchanging events through `Box<dyn Transport>`),
//! or to wire co-located components together without a relay. For scripted
//! inputs and call-log assertions use `MockTransport` (feature `test-utils`)
//! instead.
//!
//! Semantics:
//!
//! - Publishing verifies the event and routes it by its `h` tag; events
//!   without a channel UUID in an `h` tag are rejected.
//! - Delivery requires a matching subscription (channel, `kinds`,
//!   `require_mention` against the participant's pubkey).
//! - A participant's own publishes are not echoed back to it.
//! - There is no history: `replay_since` is advisory (see
//!   [`Subscription`]) and ignored here, like a bridge without storage.
//! - There is no connection to lose: `next_event` pends until an event
//!   matches, and `reconnect` is a no-op that never fails.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::warn;
use uuid::Uuid;

use crate::{BoxFuture, SignedEvent, Subscription, Transport, TransportError, TransportEvent};

/// Buffered deliveries per participant. A participant that stops polling
/// `next_event` skips the oldest events past this depth (with a warning)
/// rather than blocking publishers.
const HUB_CAPACITY: usize = 256;

/// One published event in flight inside the hub.
#[derive(Debug, Clone)]
struct Delivery {
    origin: u64,
    channel_id: Uuid,
    event: SignedEvent,
}

/// An in-memory event hub that participants [`connect`](InMemoryHub::connect)
/// to. Cloning the hub yields another handle to the same hub.
#[derive(Debug, Clone)]
pub struct InMemoryHub {
    tx: broadcast::Sender<Delivery>,
    next_id: Arc<AtomicU64>,
}

impl InMemoryHub {
    /// Create an empty hub.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(HUB_CAPACITY);
        Self {
            tx,
            next_id: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Connect a participant. `pubkey` is the identity `require_mention`
    /// subscriptions are matched against — the same role the `hello` pubkey
    /// plays on the remote bridge.
    pub fn connect(&self, pubkey: impl Into<String>) -> InMemoryTransport {
        InMemoryTransport {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            pubkey: pubkey.into(),
            rx: self.tx.subscribe(),
            tx: self.tx.clone(),
            subscriptions: HashMap::new(),
        }
    }
}

impl Default for InMemoryHub {
    fn default() -> Self {
        Self::new()
    }
}

/// One participant's [`Transport`] onto an [`InMemoryHub`].
#[derive(Debug)]
pub struct InMemoryTransport {
    id: u64,
    pubkey: String,
    rx: broadcast::Receiver<Delivery>,
    tx: broadcast::Sender<Delivery>,
    subscriptions: HashMap<Uuid, Subscription>,
}

impl InMemoryTransport {
    /// Verify and route an event into the hub (shared by both publish paths).
    fn send(&self, event: SignedEvent) -> Result<(), TransportError> {
        event.verify()?;
        let channel_id = channel_of(&event).ok_or_else(|| {
            TransportError::InvalidEvent("event has no channel UUID in an `h` tag".into())
        })?;
        // Send fails only with zero receivers; `self.rx` alone guarantees one.
        self.tx
            .send(Delivery {
                origin: self.id,
                channel_id,
                event,
            })
            .map_err(|_| TransportError::Closed)?;
        Ok(())
    }

    /// Does this delivery match one of our subscriptions?
    fn wants(&self, delivery: &Delivery) -> bool {
        if delivery.origin == self.id {
            return false;
        }
        let Some(sub) = self.subscriptions.get(&delivery.channel_id) else {
            return false;
        };
        // NIP-01 edge: an explicit empty `kinds` list matches nothing.
        if let Some(kinds) = &sub.kinds {
            if !kinds.contains(&delivery.event.kind) {
                return false;
            }
        }
        if sub.require_mention {
            let mentioned = delivery
                .event
                .tags
                .iter()
                .any(|t| t.len() >= 2 && t[0] == "p" && t[1] == self.pubkey);
            if !mentioned {
                return false;
            }
        }
        true
    }
}

/// The channel an event belongs to: the first `h` tag holding a UUID.
fn channel_of(event: &SignedEvent) -> Option<Uuid> {
    event
        .tags
        .iter()
        .filter(|t| t.len() >= 2 && t[0] == "h")
        .find_map(|t| Uuid::parse_str(&t[1]).ok())
}

impl Transport for InMemoryTransport {
    fn subscribe(
        &mut self,
        subscription: Subscription,
    ) -> BoxFuture<'_, Result<(), TransportError>> {
        // Replace-on-resubscribe, per the seam contract.
        self.subscriptions
            .insert(subscription.channel_id, subscription);
        Box::pin(async { Ok(()) })
    }

    fn unsubscribe(&mut self, channel_id: Uuid) -> BoxFuture<'_, Result<(), TransportError>> {
        self.subscriptions.remove(&channel_id);
        Box::pin(async { Ok(()) })
    }

    fn next_event(&mut self) -> BoxFuture<'_, Option<TransportEvent>> {
        Box::pin(async move {
            loop {
                match self.rx.recv().await {
                    Ok(delivery) if self.wants(&delivery) => {
                        return Some(TransportEvent {
                            channel_id: delivery.channel_id,
                            event: delivery.event,
                        });
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "in-memory transport lagged; events skipped");
                    }
                    // All senders gone — unreachable while `self.tx` lives,
                    // but the honest mapping is end-of-stream.
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
    }

    fn publish(&self, event: SignedEvent) -> BoxFuture<'_, Result<(), TransportError>> {
        let result = self.send(event);
        Box::pin(async move { result })
    }

    fn try_publish(&self, event: SignedEvent) -> Result<(), TransportError> {
        self.send(event)
    }

    fn reconnect(&mut self) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async { Ok(()) })
    }

    fn shutdown(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_with_tags(tags: Vec<Vec<String>>) -> SignedEvent {
        SignedEvent {
            id: "00".repeat(32),
            pubkey: "11".repeat(32),
            created_at: 1_700_000_000,
            kind: 9,
            tags,
            content: "x".into(),
            sig: "22".repeat(64),
        }
    }

    #[test]
    fn channel_of_takes_the_first_uuid_h_tag() {
        let channel = Uuid::new_v4();
        let event = event_with_tags(vec![
            vec!["p".into(), "aa".repeat(32)],
            vec!["h".into(), "not-a-uuid".into()],
            vec!["h".into(), channel.to_string()],
        ]);
        assert_eq!(channel_of(&event), Some(channel));

        assert_eq!(channel_of(&event_with_tags(vec![])), None);
        assert_eq!(
            channel_of(&event_with_tags(vec![vec!["h".into()]])),
            None,
            "a bare h tag with no value must not match"
        );
    }
}
