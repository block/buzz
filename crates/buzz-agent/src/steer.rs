//! Mid-turn steering, owned by buzz-agent.
//!
//! Steering is a message injected into a turn already in flight, without
//! cancelling it. goose has an equivalent queue on its `Agent`, but
//! `drain_pending_steers` / `has_pending_steers` are `pub(crate)` — only
//! `Agent::reply` can consume them. Since buzz-agent owns the loop, it owns
//! the queue too.
//!
//! The semantics match what buzz-acp expects and what goose does internally:
//!
//! * the message is queued, never applied mid-inference — it lands at the
//!   **round boundary**, so it cannot corrupt a partially streamed response;
//! * a pending steer **prevents the turn from ending**, so a user who steers
//!   just as the model finishes is not silently ignored;
//! * steering never restarts or cancels the turn.

use std::collections::VecDeque;
use std::sync::Arc;

use goose_provider_types::conversation::message::Message;
use tokio::sync::Mutex;

/// A session's pending steer messages.
///
/// Cloneable and shared: `session/steer` pushes from the JSON-RPC dispatch
/// task while the turn drains from the loop task.
#[derive(Clone, Default)]
pub struct SteerQueue {
    inner: Arc<Mutex<VecDeque<Message>>>,
}

impl SteerQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn push(&self, message: Message) {
        self.inner.lock().await.push_back(message);
    }

    pub async fn is_pending(&self) -> bool {
        !self.inner.lock().await.is_empty()
    }

    /// Take everything queued so far.
    ///
    /// Draining and appending is a single logical step for the loop: a steer
    /// that arrives *during* the drain stays queued for the next boundary
    /// rather than being lost.
    pub async fn drain(&self) -> Vec<Message> {
        self.inner.lock().await.drain(..).collect()
    }

    /// Drop anything queued. Used when a turn ends so a late steer cannot leak
    /// into the next one.
    pub async fn clear(&self) {
        self.inner.lock().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn drain_takes_everything_in_order() {
        let queue = SteerQueue::new();
        queue.push(Message::user().with_text("first")).await;
        queue.push(Message::user().with_text("second")).await;
        assert!(queue.is_pending().await);

        let drained = queue.drain().await;
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].as_concat_text(), "first");
        assert_eq!(drained[1].as_concat_text(), "second");
        assert!(!queue.is_pending().await, "drain must empty the queue");
    }

    #[tokio::test]
    async fn clear_discards_a_late_steer() {
        let queue = SteerQueue::new();
        queue.push(Message::user().with_text("too late")).await;
        queue.clear().await;
        assert!(!queue.is_pending().await);
    }
}
