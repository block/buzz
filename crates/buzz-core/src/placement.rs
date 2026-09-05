//! Desired placement projection, not a command executor or an admission journal.
//!
//! Callers must first authenticate and decode each event, authorize its owner,
//! community, agent and host, and supply only one owner/community/agent scope.
//! This module does not parse a wire format or confer execution authority.
//! Historical intent may be projected, but must never be replayed as commands.

use std::cmp::Ordering;

use nostr::{Event, EventId, PublicKey};

/// Signed-event precedence: newer sender seconds, then LOWER event ID wins.
/// This is neither receiver-arrival order nor causal/last-click order. A future
/// timestamp can win; no clock-skew or relay-sequencing policy is added here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventOrder {
    created_at: u64,
    id: EventId,
}

impl EventOrder {
    /// Extract signed fields from an event already verified by the caller.
    /// Extraction itself does not verify the signature or authorize the event.
    pub fn from_event(event: &Event) -> Self {
        Self {
            created_at: event.created_at.as_secs(),
            id: event.id,
        }
    }

    /// Identity of the signed event, retained unchanged on transport retry.
    pub fn event_id(self) -> EventId {
        self.id
    }
}

impl Ord for EventOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        self.created_at
            .cmp(&other.created_at)
            .then_with(|| other.id.cmp(&self.id))
    }
}

impl PartialOrd for EventOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Only Start and Stop affect placement. Restart is a separately deduplicated
/// current-host action. Move contributes a Start only after ordinary Stop
/// succeeds and its still-valid coordinator actually issues destination Start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementAction {
    /// Select this host, superseding earlier host selections.
    Start,
    /// Stop this host without cancelling another host's selected placement.
    Stop,
}

/// A decoded placement contribution from one authorized signed command.
/// The transport adapter must bind all three fields to the SAME signed event.
/// Request identity and one-shot outcomes belong in the admission journal, not
/// in this projection; duplicate intent is harmless but not execution dedup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementIntent {
    /// Signed command precedence (not receipt or relay arrival time).
    pub order: EventOrder,
    /// Authorized executor identity, not a physical machine or process ID.
    pub host: PublicKey,
    /// Decoded operation; legacy exact-run Stop must not be broadened here.
    pub action: PlacementAction,
}

/// Intent for one host. This does not describe observed process state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetIntent {
    /// No relevant intent is known. This is not permission to launch.
    Unknown,
    /// The selected Start is still desired; its identity guards continuations.
    Running(EventOrder),
    /// A local Stop or selection of another host supersedes local launch work.
    Stopped(EventOrder),
}

/// A read-only view over the relevant valid events for one scoped agent.
/// Uses two linear scans and constant auxiliary space, regardless of delivery
/// order. The caller owns history completeness and replay-safe retention.
#[derive(Debug)]
pub struct PlacementProjection<'a> {
    intents: &'a [PlacementIntent],
    latest_start: Option<&'a PlacementIntent>,
}

impl<'a> PlacementProjection<'a> {
    /// Project known intent without performing, scheduling or resuming effects.
    pub fn new(intents: &'a [PlacementIntent]) -> Self {
        let latest_start = intents
            .iter()
            .filter(|intent| intent.action == PlacementAction::Start)
            .max_by_key(|intent| intent.order);
        Self {
            intents,
            latest_start,
        }
    }

    /// Desired host and the Start that selected it, or none.
    /// Stopping the newest selection NEVER falls back to an earlier Start.
    pub fn desired(&self) -> Option<(PublicKey, EventOrder)> {
        let start = self.latest_start?;
        match self.target(start.host) {
            TargetIntent::Running(order) => Some((start.host, order)),
            _ => None,
        }
    }

    /// Project one target independently of receiver arrival. A Stop for X
    /// remains relevant even if Start X is learned later. A Stop for X cannot
    /// change Y's unchanged Start identity, including when Y is learned later.
    pub fn target(&self, host: PublicKey) -> TargetIntent {
        let stop = self
            .intents
            .iter()
            .filter(|intent| intent.host == host && intent.action == PlacementAction::Stop)
            .max_by_key(|intent| intent.order);
        match (self.latest_start, stop) {
            (None, None) => TargetIntent::Unknown,
            (None, Some(stop)) => TargetIntent::Stopped(stop.order),
            (Some(start), Some(stop)) if stop.order > start.order => {
                TargetIntent::Stopped(stop.order)
            }
            (Some(start), _) if start.host == host => TargetIntent::Running(start.order),
            (Some(start), _) => TargetIntent::Stopped(start.order),
        }
    }

    /// Check only the placement part of a pending launch's continuation guard.
    /// Recheck immediately before effects, alongside current authorization,
    /// local process state and durable one-shot admission. True does NOT grant
    /// permission to replay a Start/Restart or resume an interrupted operation.
    pub fn retains_start(&self, host: PublicKey, start: EventOrder) -> bool {
        self.target(host) == TargetIntent::Running(start)
    }
}

#[cfg(test)]
mod tests;
