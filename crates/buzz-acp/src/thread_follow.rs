//! In-memory admission state for `--subscribe thread-follow`.
//!
//! The relay must deliver non-mentioned replies for this mode, so its NIP-01
//! filter is deliberately broader than a mention filter. This state restores a
//! strict local boundary before events enter the queue: an event is admitted
//! only when it explicitly mentions this agent or belongs to a thread that a
//! previously accepted mention opened.

use std::collections::HashMap;
use std::time::Instant;

use nostr::Event;
use uuid::Uuid;

use crate::queue::parse_thread_tags;

/// Bounded, per-channel thread-follow state.
///
/// State is process-local and intentionally lost on restart. A thread remains
/// followed until the agent is restarted, leaves the channel, or its entry is
/// evicted to make room for a newer conversation. This keeps the feature's
/// contract explicit: there is no hidden idle timeout that can end a thread.
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

/// Bounded, per-channel followed thread roots.
#[derive(Debug)]
pub struct ThreadFollowState {
    followed: HashMap<(Uuid, String), Instant>,
    capacity: usize,
}

impl ThreadFollowState {
    pub fn new(capacity: usize) -> Self {
        Self {
            followed: HashMap::new(),
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
        _now: Instant,
    ) -> ThreadFollowAdmission {
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
    /// and followed replies preserve the followed root while enforcing a
    /// bounded number of tracked conversations.
    pub fn record(&mut self, channel_id: Uuid, admission: &ThreadFollowAdmission, now: Instant) {
        let root_event_id = match admission {
            ThreadFollowAdmission::Mention { root_event_id }
            | ThreadFollowAdmission::Followed { root_event_id } => root_event_id,
            ThreadFollowAdmission::Reject => return,
        };
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
        let mut state = ThreadFollowState::new(10);
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
        let mut state = ThreadFollowState::new(10);
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
    fn evicts_oldest_thread_but_does_not_expire_an_active_thread() {
        let channel = Uuid::new_v4();
        let agent = "a".repeat(64);
        let now = Instant::now();
        let mut state = ThreadFollowState::new(1);
        let first = event(&[vec!["p", &agent]]);
        let first_admission = state.classify(channel, &first, &agent, now);
        state.record(channel, &first_admission, now);
        let second = event(&[vec!["p", &agent]]);
        let second_admission = state.classify(channel, &second, &agent, now);
        state.record(channel, &second_admission, now);
        let first_reply = event(&[vec!["e", &first.id.to_hex(), "", "reply"]]);
        assert_eq!(
            state.classify(channel, &first_reply, &agent, now),
            ThreadFollowAdmission::Reject
        );
        let second_reply = event(&[vec!["e", &second.id.to_hex(), "", "reply"]]);
        assert!(matches!(
            state.classify(
                channel,
                &second_reply,
                &agent,
                now + std::time::Duration::from_secs(60 * 60 * 24)
            ),
            ThreadFollowAdmission::Followed { .. }
        ));
    }
}
