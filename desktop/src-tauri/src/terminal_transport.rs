//! Credit and viewport ordering for one renderer subscription.
//!
//! Tauri channels are ordered, but they do not expose consumer credit and
//! channel delivery is unordered with respect to invoke responses.  This state
//! machine therefore permits one frame on the wire, retains at most one newer
//! complete snapshot, and independently gates a resized viewport until the
//! renderer confirms that it knows the applied [`Viewport`].

use buzz_terminal::{damage::Frame, Viewport};
use std::fmt;

use uuid::Uuid;

/// A renderer attachment. Messages from an attachment that has been replaced
/// cannot release the new attachment's credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SubscriptionId(Uuid);

impl SubscriptionId {
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| "invalid terminal subscription id".to_string())
    }
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A frame carrying the identity and sequence that its ACK must repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Publication {
    pub(crate) subscription_id: SubscriptionId,
    pub(crate) sequence: u64,
    pub(crate) frame: Frame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OfferError {
    /// Replacing pending incremental damage would lose changes that existed in
    /// the older pending frame. Only a complete snapshot may be replaceable.
    PendingFrameMustBeSnapshot,
}

struct Subscription {
    id: SubscriptionId,
    next_sequence: u64,
    in_flight: Option<u64>,
    pending: Option<Frame>,
    viewport_ready: bool,
}

/// Publication state for a terminal session.
///
/// PTY ownership is deliberately elsewhere. Faulting or detaching this state
/// stops renderer publication only; it must never stop the PTY reader.
pub(crate) struct FramePublisher {
    applied: Viewport,
    subscription: Option<Subscription>,
}

impl FramePublisher {
    pub(crate) fn new(applied: Viewport) -> Self {
        Self {
            applied,
            subscription: None,
        }
    }

    pub(crate) fn applied_viewport(&self) -> Viewport {
        self.applied
    }

    /// Attach with a complete snapshot of the current viewport. Attachment is
    /// a fresh bootstrap: no credit or readiness from the old subscription is
    /// inherited.
    pub(crate) fn attach(
        &mut self,
        id: SubscriptionId,
        snapshot: Frame,
    ) -> Result<Publication, OfferError> {
        self.require_current_snapshot(&snapshot)?;
        let mut subscription = Subscription {
            id,
            next_sequence: 1,
            in_flight: None,
            pending: None,
            viewport_ready: true,
        };
        let publication = Self::publish(&mut subscription, snapshot);
        self.subscription = Some(subscription);
        Ok(publication)
    }

    /// Whether the next capture must be complete so it can safely replace the
    /// single pending value. Callers use this to select the engine's snapshot
    /// path before taking the terminal lock.
    pub(crate) fn requires_snapshot(&self) -> bool {
        self.subscription.as_ref().is_some_and(|subscription| {
            subscription.in_flight.is_some() || !subscription.viewport_ready
        })
    }

    /// Offer the newest capture. Frames for an overtaken viewport are inert.
    /// While credit/readiness is held, the newest complete snapshot replaces
    /// the prior one and pending storage remains exactly one frame.
    pub(crate) fn offer(&mut self, frame: Frame) -> Result<Option<Publication>, OfferError> {
        if frame.viewport != self.applied {
            return Ok(None);
        }
        let Some(subscription) = &mut self.subscription else {
            return Ok(None);
        };

        if subscription.in_flight.is_some() || !subscription.viewport_ready {
            if !frame.full {
                return Err(OfferError::PendingFrameMustBeSnapshot);
            }
            subscription.pending = Some(frame);
            return Ok(None);
        }

        Ok(Some(Self::publish(subscription, frame)))
    }

    /// Apply the viewport returned by the engine resize call. A real change
    /// invalidates pending state and closes the readiness gate. Same-size
    /// resizes are inert because the engine returns the unchanged viewport.
    pub(crate) fn resize_applied(&mut self, viewport: Viewport) {
        if viewport == self.applied {
            return;
        }
        self.applied = viewport;
        if let Some(subscription) = &mut self.subscription {
            subscription.viewport_ready = false;
            subscription.pending = None;
        }
    }

    /// Release ordinary frame credit only for the exact live subscription and
    /// exact frame in flight.
    pub(crate) fn acknowledge(&mut self, id: SubscriptionId, sequence: u64) -> Option<Publication> {
        let subscription = self.subscription.as_mut()?;
        if subscription.id != id || subscription.in_flight != Some(sequence) {
            return None;
        }
        subscription.in_flight = None;
        Self::flush(subscription)
    }

    /// Renderer confirmation that the resize invoke result has been installed.
    /// Equality against the one atomic viewport value is the entire policy:
    /// old subscription IDs and superseded viewport values are inert.
    pub(crate) fn viewport_ready(
        &mut self,
        id: SubscriptionId,
        viewport: Viewport,
    ) -> Option<Publication> {
        let subscription = self.subscription.as_mut()?;
        if subscription.id != id || viewport != self.applied {
            return None;
        }
        subscription.viewport_ready = true;
        Self::flush(subscription)
    }

    /// Drop publication state after channel close, invoke rejection, renderer
    /// teardown, or explicit detach. The caller owns safe Buzz focus recovery.
    pub(crate) fn fault(&mut self, id: SubscriptionId) -> bool {
        if self
            .subscription
            .as_ref()
            .is_some_and(|subscription| subscription.id == id)
        {
            self.subscription = None;
            true
        } else {
            false
        }
    }

    /// Permanently detach publication as the session begins closing.
    pub(crate) fn close(&mut self) {
        self.subscription = None;
    }

    fn require_current_snapshot(&self, frame: &Frame) -> Result<(), OfferError> {
        if frame.viewport == self.applied && frame.full {
            Ok(())
        } else {
            Err(OfferError::PendingFrameMustBeSnapshot)
        }
    }

    fn flush(subscription: &mut Subscription) -> Option<Publication> {
        if subscription.in_flight.is_some() || !subscription.viewport_ready {
            return None;
        }
        let frame = subscription.pending.take()?;
        Some(Self::publish(subscription, frame))
    }

    fn publish(subscription: &mut Subscription, frame: Frame) -> Publication {
        debug_assert!(subscription.in_flight.is_none());
        let sequence = subscription.next_sequence;
        subscription.next_sequence = subscription
            .next_sequence
            .checked_add(1)
            .expect("terminal publication sequence exhausted");
        subscription.in_flight = Some(sequence);
        Publication {
            subscription_id: subscription.id,
            sequence,
            frame,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_terminal::damage::{CursorFrame, RowFrame};

    fn viewport(generation: u64, columns: usize, screen_lines: usize) -> Viewport {
        Viewport {
            generation,
            columns,
            screen_lines,
        }
    }

    fn frame(viewport: Viewport, marker: usize, full: bool) -> Frame {
        Frame {
            rows: vec![RowFrame {
                line: marker,
                spans: Vec::new(),
            }],
            cursor: CursorFrame {
                line: 0,
                column: 0,
                visible: true,
            },
            cursor_changed: true,
            full,
            viewport,
        }
    }

    #[test]
    fn attach_bootstraps_then_ack_releases_only_latest_snapshot() {
        let v0 = viewport(0, 80, 24);
        let mut publisher = FramePublisher::new(v0);
        let id = SubscriptionId::new();
        let bootstrap = publisher.attach(id, frame(v0, 0, true)).unwrap();
        assert_eq!(bootstrap.sequence, 1);
        assert!(publisher.requires_snapshot());

        assert_eq!(publisher.offer(frame(v0, 1, true)).unwrap(), None);
        assert_eq!(publisher.offer(frame(v0, 2, true)).unwrap(), None);
        let successor = publisher.acknowledge(id, 1).unwrap();
        assert_eq!(successor.sequence, 2);
        assert_eq!(successor.frame.rows[0].line, 2);
    }

    #[test]
    fn pending_incremental_frame_is_rejected_instead_of_losing_damage() {
        let v0 = viewport(0, 80, 24);
        let mut publisher = FramePublisher::new(v0);
        let id = SubscriptionId::new();
        publisher.attach(id, frame(v0, 0, true)).unwrap();
        assert_eq!(
            publisher.offer(frame(v0, 1, false)),
            Err(OfferError::PendingFrameMustBeSnapshot)
        );
    }

    #[test]
    fn late_old_frame_is_acked_but_never_crosses_viewports() {
        let v0 = viewport(0, 80, 24);
        let v1 = viewport(1, 100, 30);
        let mut publisher = FramePublisher::new(v0);
        let id = SubscriptionId::new();
        publisher.attach(id, frame(v0, 0, true)).unwrap();

        publisher.resize_applied(v1);
        assert_eq!(publisher.offer(frame(v0, 7, true)).unwrap(), None);
        assert_eq!(publisher.offer(frame(v1, 8, true)).unwrap(), None);
        assert_eq!(publisher.acknowledge(id, 1), None);
        let current = publisher.viewport_ready(id, v1).unwrap();
        assert_eq!(current.frame.viewport, v1);
        assert_eq!(current.frame.rows[0].line, 8);
    }

    #[test]
    fn early_new_frame_is_held_despite_real_capture_pressure() {
        let v0 = viewport(0, 80, 24);
        let v1 = viewport(1, 120, 40);
        let mut publisher = FramePublisher::new(v0);
        let id = SubscriptionId::new();
        publisher.attach(id, frame(v0, 0, true)).unwrap();
        publisher.acknowledge(id, 1);

        publisher.resize_applied(v1);
        // A real non-empty capture occurred. Zero publication before readiness
        // therefore cannot pass vacuously because no output happened.
        let captured = frame(v1, 9, true);
        assert!(!captured.rows.is_empty());
        assert_eq!(publisher.offer(captured).unwrap(), None);
        let publication = publisher.viewport_ready(id, v1).unwrap();
        assert_eq!(publication.frame.rows[0].line, 9);
        assert_eq!(publication.frame.viewport, v1);
    }

    #[test]
    fn ack_and_readiness_may_arrive_in_either_order() {
        let v0 = viewport(0, 80, 24);
        let v1 = viewport(1, 100, 30);
        let mut publisher = FramePublisher::new(v0);
        let id = SubscriptionId::new();
        publisher.attach(id, frame(v0, 0, true)).unwrap();
        publisher.resize_applied(v1);
        publisher.offer(frame(v1, 1, true)).unwrap();

        assert_eq!(publisher.viewport_ready(id, v1), None);
        assert!(publisher.acknowledge(id, 1).is_some());
    }

    #[test]
    fn superseded_readiness_and_same_size_resize_are_inert() {
        let v0 = viewport(0, 80, 24);
        let v1 = viewport(1, 100, 30);
        let v2 = viewport(2, 120, 40);
        let mut publisher = FramePublisher::new(v0);
        let id = SubscriptionId::new();
        publisher.attach(id, frame(v0, 0, true)).unwrap();
        publisher.resize_applied(v1);
        publisher.resize_applied(v2);
        publisher.resize_applied(v2);
        assert_eq!(publisher.applied_viewport(), v2);
        publisher.offer(frame(v2, 2, true)).unwrap();
        publisher.acknowledge(id, 1);
        assert_eq!(publisher.viewport_ready(id, v1), None);
        assert!(publisher.viewport_ready(id, v2).is_some());
    }

    #[test]
    fn old_subscription_messages_cannot_release_reattached_subscription() {
        let v0 = viewport(0, 80, 24);
        let mut publisher = FramePublisher::new(v0);
        let old = SubscriptionId::new();
        publisher.attach(old, frame(v0, 0, true)).unwrap();
        assert!(publisher.fault(old));

        let new = SubscriptionId::new();
        publisher.attach(new, frame(v0, 1, true)).unwrap();
        publisher.offer(frame(v0, 2, true)).unwrap();
        assert_eq!(publisher.acknowledge(old, 1), None);
        assert_eq!(publisher.viewport_ready(old, v0), None);
        let successor = publisher.acknowledge(new, 1).unwrap();
        assert_eq!(successor.frame.rows[0].line, 2);
    }

    #[test]
    fn stale_fault_cannot_detach_current_subscription() {
        let v0 = viewport(0, 80, 24);
        let mut publisher = FramePublisher::new(v0);
        let old = SubscriptionId::new();
        let new = SubscriptionId::new();
        publisher.attach(old, frame(v0, 0, true)).unwrap();
        publisher.attach(new, frame(v0, 1, true)).unwrap();
        assert!(!publisher.fault(old));
        assert!(publisher.fault(new));
    }
}
