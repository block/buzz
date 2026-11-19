use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use futures_util::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use uuid::Uuid;

use crate::{check_sibling_via_profile, relay, OwnerCache, SiblingVerdict};

const MAX_PENDING_SENDERS: usize = 16;
const MAX_PENDING_PER_SENDER: usize = 8;
const MAX_ATTEMPTS: u8 = 4;

struct PendingSender {
    events: VecDeque<relay::BuzzEvent>,
    attempts: u8,
}

/// One bounded profile lookup per sender. The listener polls these alongside
/// the relay, so waiting on an unknown sender never delays a verified one.
#[derive(Default)]
pub(super) struct SiblingRetries {
    pending: HashMap<String, PendingSender>,
    checks: FuturesUnordered<BoxFuture<'static, (String, SiblingVerdict)>>,
    ready: VecDeque<relay::BuzzEvent>,
}

impl SiblingRetries {
    pub(super) fn hold(
        &mut self,
        event: relay::BuzzEvent,
        author: String,
        owner: &str,
        retry_at: Option<Instant>,
        rest: &relay::RestClient,
    ) {
        let at_capacity = self.pending.len() >= MAX_PENDING_SENDERS;
        if let Some(pending) = self.pending.get_mut(&author) {
            if pending
                .events
                .iter()
                .any(|held| held.event.id == event.event.id)
            {
                return;
            }
            if pending.events.len() >= MAX_PENDING_PER_SENDER {
                log_dropped(&event, "sender queue full");
                return;
            }
            log_held(&event);
            pending.events.push_back(event);
        } else if at_capacity {
            log_dropped(&event, "too many unverified senders");
        } else {
            log_held(&event);
            self.pending.insert(
                author.clone(),
                PendingSender {
                    events: VecDeque::from([event]),
                    attempts: 1,
                },
            );
            self.schedule(author, owner, retry_at, rest);
        }
    }

    fn schedule(
        &self,
        author: String,
        owner: &str,
        retry_at: Option<Instant>,
        rest: &relay::RestClient,
    ) {
        let owner = owner.to_owned();
        let rest = rest.clone();
        self.checks.push(Box::pin(async move {
            if let Some(retry_at) = retry_at {
                tokio::time::sleep_until(retry_at.into()).await;
            }
            let verdict = check_sibling_via_profile(&author, &owner, &rest).await;
            (author, verdict)
        }));
    }

    /// Return the original event for the normal listener checks, including
    /// the current relay identity, channel policy and subscription filters.
    pub(super) async fn next_event(
        &mut self,
        cache: &OwnerCache,
        rest: &relay::RestClient,
    ) -> relay::BuzzEvent {
        loop {
            if let Some(event) = self.ready.pop_front() {
                tracing::debug!(event_id = %event.event.id, kind = %event.event.kind, "retrying held event after sibling verification");
                return event;
            }

            let Some((author, verdict)) = self.checks.next().await else {
                return std::future::pending().await;
            };
            let Some(mut pending) = self.pending.remove(&author) else {
                continue;
            };
            cache.record_sibling_verdict(&author, verdict);
            match verdict {
                SiblingVerdict::Sibling => self.ready.append(&mut pending.events),
                SiblingVerdict::NotSibling => {
                    for event in pending.events {
                        tracing::info!(event_id = %event.event.id, kind = %event.event.kind, "dropping event from an agent without the same owner");
                    }
                }
                SiblingVerdict::Indeterminate => {
                    if let Some(owner) = cache
                        .get()
                        .filter(|_| !pending.events.is_empty() && pending.attempts < MAX_ATTEMPTS)
                    {
                        pending.attempts += 1;
                        let retry_at = cache.sibling_retry_at(&author);
                        self.schedule(author.clone(), owner, retry_at, rest);
                        self.pending.insert(author, pending);
                    } else {
                        for event in pending.events {
                            log_dropped(&event, "sibling verification retry limit reached");
                        }
                    }
                }
            }
        }
    }

    pub(super) fn discard_channel(&mut self, channel_id: Uuid) {
        let keep = |event: &relay::BuzzEvent| {
            if event.channel_id == channel_id {
                tracing::debug!(event_id = %event.event.id, kind = %event.event.kind, %channel_id, "discarding held event after channel membership removal");
                false
            } else {
                true
            }
        };
        self.ready.retain(keep);
        for sender in self.pending.values_mut() {
            sender.events.retain(keep);
        }
    }
}

fn log_held(event: &relay::BuzzEvent) {
    tracing::debug!(event_id = %event.event.id, kind = %event.event.kind, "holding event for sibling verification");
}

fn log_dropped(event: &relay::BuzzEvent, reason: &str) {
    tracing::warn!(event_id = %event.event.id, kind = %event.event.kind, reason, "dropping event during sibling verification");
}
