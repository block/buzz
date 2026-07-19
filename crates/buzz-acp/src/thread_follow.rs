//! In-memory admission state for `--subscribe thread-follow`.
//!
//! The relay must deliver non-mentioned replies for this mode, so its NIP-01
//! filter is deliberately broader than a mention filter. This state restores a
//! strict local boundary before events enter the queue: an event is admitted
//! only when it explicitly mentions this agent or belongs to a thread that a
//! previously accepted mention opened.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nostr::Event;
use uuid::Uuid;

use crate::queue::parse_thread_tags;

/// Keep only recent active conversations. State is process-local and is
/// intentionally lost on restart, which fails closed rather than turning the
/// harness into a persistent channel-wide listener.
pub const DEFAULT_FOLLOW_TTL: Duration = Duration::from_secs(60 * 60);
pub const DEFAULT_MAX_FOLLOWED_THREADS: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadFollowAdmission {
    /// An explicit mention is always eligible and starts (or refreshes) a lease.
    Mention { root_event_id: String },
    /// A later message belongs to an active followed thread.
    Followed { root_event_id: String },
    /// Neither explicitly mentioned nor part of a followed thread.
    Reject,
}

/// Bounded, per-channel thread-follow leases.
#[derive(Debug)]
pub struct ThreadFollowState {
    followed: HashMap<(Uuid, String), Instant>,
    ttl: Duration,
    capacity: usize,
}

impl ThreadFollowState {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            followed: HashMap::new(),
            ttl,
            capacity,
        }
    }

    /// Classify an event without mutating state. Call [`record`] only after the
    /// event has been accepted by the queue, so dropped duplicate events cannot
    /// create or extend a follow lease.
    pub fn classify(
        &mut self,
        channel_id: Uuid,
        event: &Event,
        agent_pubkey_hex: &str,
        now: Instant,
    ) -> ThreadFollowAdmission {
        self.prune_expired(now);
        let thread = parse_thread_tags(event);
        let root_event_id = thread.root_event_id.unwrap_or_else(|| event.id.to_hex());
        let mentioned = thread
            .mentioned_pubkeys
            .iter()
            .any(|pubkey| pubkey == agent_pubkey_hex);

        if mentioned {
            return ThreadFollowAdmission::Mention { root_event_id };
        }
        if self
            .followed
            .contains_key(&(channel_id, root_event_id.clone()))
        {
            ThreadFollowAdmission::Followed { root_event_id }
        } else {
            ThreadFollowAdmission::Reject
        }
    }

    /// Record an event that was admitted to the queue. Both explicit mentions
    /// and followed replies refresh the idle lease while preserving the bounded
    /// number of tracked thread roots.
    pub fn record(&mut self, channel_id: Uuid, admission: &ThreadFollowAdmission, now: Instant) {
        let root_event_id = match admission {
            ThreadFollowAdmission::Mention { root_event_id }
            | ThreadFollowAdmission::Followed { root_event_id } => root_event_id,
            ThreadFollowAdmission::Reject => return,
        };
        self.prune_expired(now);
        let key = (channel_id, root_event_id.clone());
        if !self.followed.contains_key(&key) && self.followed.len() >= self.capacity {
            if let Some(oldest) = self
                .followed
                .iter()
                .min_by_key(|(_, last_active)| **last_active)
                .map(|(key, _)| key.clone())
            {
                self.followed.remove(&oldest);
            }
        }
        self.followed.insert(key, now);
    }

    pub fn remove_channel(&mut self, channel_id: Uuid) {
        self.followed
            .retain(|(channel, _), _| *channel != channel_id);
    }

    fn prune_expired(&mut self, now: Instant) {
        self.followed
            .retain(|_, last_active| now.duration_since(*last_active) <= self.ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn event(tags: &[Vec<&str>]) -> Event {
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.clone()).unwrap())
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(9), "message")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn follows_only_the_thread_started_by_a_mention() {
        let channel = Uuid::new_v4();
        let agent = "a".repeat(64);
        let now = Instant::now();
        let mut state = ThreadFollowState::new(DEFAULT_FOLLOW_TTL, 10);
        let mention = event(&[vec!["p", &agent]]);
        let admission = state.classify(channel, &mention, &agent, now);
        assert!(matches!(admission, ThreadFollowAdmission::Mention { .. }));
        let root = mention.id.to_hex();
        state.record(channel, &admission, now);

        let reply = event(&[vec!["e", &root, "", "reply"]]);
        assert!(matches!(
            state.classify(channel, &reply, &agent, now),
            ThreadFollowAdmission::Followed { .. }
        ));

        let unrelated = event(&[]);
        assert_eq!(
            state.classify(channel, &unrelated, &agent, now),
            ThreadFollowAdmission::Reject
        );
    }

    #[test]
    fn follows_nested_replies_but_not_another_channel() {
        let channel = Uuid::new_v4();
        let other_channel = Uuid::new_v4();
        let agent = "a".repeat(64);
        let now = Instant::now();
        let mut state = ThreadFollowState::new(DEFAULT_FOLLOW_TTL, 10);
        let root = "b".repeat(64);
        let mention = event(&[vec!["e", &root, "", "reply"], vec!["p", &agent]]);
        let admission = state.classify(channel, &mention, &agent, now);
        state.record(channel, &admission, now);
        let nested = event(&[
            vec!["e", &root, "", "root"],
            vec!["e", &"c".repeat(64), "", "reply"],
        ]);
        assert!(matches!(
            state.classify(channel, &nested, &agent, now),
            ThreadFollowAdmission::Followed { .. }
        ));
        assert_eq!(
            state.classify(other_channel, &nested, &agent, now),
            ThreadFollowAdmission::Reject
        );
    }

    #[test]
    fn expires_and_evicts_oldest_thread() {
        let channel = Uuid::new_v4();
        let agent = "a".repeat(64);
        let now = Instant::now();
        let mut state = ThreadFollowState::new(Duration::from_secs(10), 1);
        let first = event(&[vec!["p", &agent]]);
        let first_admission = state.classify(channel, &first, &agent, now);
        state.record(channel, &first_admission, now);
        let second = event(&[vec!["p", &agent]]);
        let second_admission =
            state.classify(channel, &second, &agent, now + Duration::from_secs(1));
        state.record(channel, &second_admission, now + Duration::from_secs(1));
        let first_reply = event(&[vec!["e", &first.id.to_hex(), "", "reply"]]);
        assert_eq!(
            state.classify(channel, &first_reply, &agent, now + Duration::from_secs(1)),
            ThreadFollowAdmission::Reject
        );
        let second_reply = event(&[vec!["e", &second.id.to_hex(), "", "reply"]]);
        assert_eq!(
            state.classify(
                channel,
                &second_reply,
                &agent,
                now + Duration::from_secs(12)
            ),
            ThreadFollowAdmission::Reject
        );
    }
}
