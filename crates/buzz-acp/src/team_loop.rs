//! Opt-in limit on consecutive teammate-authored triggers in a conversation.
//! The relay subscription is widened when enabled so admitted human messages
//! that do not mention this agent can release the cap. Ordinary subscription
//! rules continue to decide which admitted events can start a turn.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::queue::ResolvedEdit;
use crate::scope::SessionPolicy;
use crate::scope::SessionScope;
use nostr::Event;
use uuid::Uuid;

const SEEN_CAPACITY: usize = 4096;

pub(crate) struct TeamLoop {
    teammates: HashSet<String>,
    limit: usize,
    counts: HashMap<SessionScope, usize>,
    seen: HashMap<String, bool>,
    seen_order: VecDeque<String>,
}

impl TeamLoop {
    /// Guard scope is always thread-rooted in channels, independently of the
    /// provider session policy. DMs have no thread boundary.
    pub(crate) fn scope(
        channel_id: Uuid,
        is_dm: bool,
        event: &Event,
        edit: Option<&ResolvedEdit>,
    ) -> SessionScope {
        SessionScope::derive_routed(SessionPolicy::Thread, channel_id, is_dm, event, edit)
    }
    pub(crate) fn new(mut teammates: HashSet<String>, self_pubkey: String, limit: usize) -> Self {
        teammates.insert(self_pubkey);
        Self {
            teammates,
            limit,
            counts: HashMap::new(),
            seen: HashMap::new(),
            seen_order: VecDeque::new(),
        }
    }

    pub(crate) fn is_teammate(&self, author: &str) -> bool {
        self.teammates.contains(author)
    }

    pub(crate) fn targets_team(&self, event: &nostr::Event) -> bool {
        event.tags.iter().any(|tag| {
            let fields = tag.as_slice();
            fields.first().map(String::as_str) == Some("p")
                && fields
                    .get(1)
                    .is_some_and(|pubkey| self.teammates.contains(pubkey))
        })
    }

    /// Admitted human messages reset the count even when they do not match this
    /// agent's subscription rule. An auxiliary event cannot reset a thread.
    pub(crate) fn observe_human(&mut self, scope: &SessionScope, id: String, kind: u16) {
        if matches!(kind, 9 | 45001 | 45003) && self.remember(id, true).is_none() {
            self.counts.remove(scope);
        }
    }

    /// Count teammate messages addressed to any team member, even when they
    /// address another agent. A locally matched message also counts without a
    /// `p` tag (for custom subscribe-all policies). All agents in the channel
    /// therefore observe the same six-message budget for a team conversation.
    pub(crate) fn observe_teammate(
        &mut self,
        scope: &SessionScope,
        id: String,
        is_trigger: bool,
    ) -> bool {
        if !is_trigger {
            return true;
        }
        if let Some(previous) = self.seen.get(&id) {
            return *previous;
        }
        let count = self.counts.entry(scope.clone()).or_default();
        *count = count.saturating_add(1);
        let allowed = *count <= self.limit;
        self.remember(id, allowed);
        allowed
    }

    fn remember(&mut self, id: String, allowed: bool) -> Option<bool> {
        if let Some(previous) = self.seen.insert(id.clone(), allowed) {
            return Some(previous);
        }
        self.seen_order.push_back(id);
        if self.seen_order.len() > SEEN_CAPACITY {
            if let Some(oldest) = self.seen_order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use uuid::Uuid;

    #[test]
    fn guard_keys_channel_events_by_thread_even_when_session_policy_is_channel() {
        let channel_id = Uuid::new_v4();
        let make_reply = |root: &str| {
            EventBuilder::new(Kind::Custom(9), "ping")
                .tags(vec![Tag::parse(vec!["e", root, "", "root"]).unwrap()])
                .sign_with_keys(&Keys::generate())
                .unwrap()
        };
        let left = make_reply(&"a".repeat(64));
        let right = make_reply(&"b".repeat(64));
        assert_ne!(
            TeamLoop::scope(channel_id, false, &left, None),
            TeamLoop::scope(channel_id, false, &right, None)
        );
        assert_eq!(
            TeamLoop::scope(channel_id, true, &left, None),
            TeamLoop::scope(channel_id, true, &right, None)
        );
    }

    #[test]
    fn stops_teammates_until_human_posts_in_same_scope() {
        let team = "a".repeat(64);
        let mut guard = TeamLoop::new(HashSet::from([team.clone()]), "self".into(), 6);
        let first = SessionScope::Conversation {
            channel_id: Uuid::new_v4(),
        };
        let second = SessionScope::Conversation {
            channel_id: Uuid::new_v4(),
        };
        for i in 0..6 {
            assert!(guard.observe_teammate(&first, format!("team-{i}"), true));
        }
        assert!(!guard.observe_teammate(&first, "blocked".into(), true));
        assert!(guard.observe_teammate(&second, "other-channel".into(), true));
        guard.observe_human(&first, "human".into(), 9);
        assert!(guard.observe_teammate(&first, "after-human".into(), true));
    }

    #[test]
    fn auxiliary_events_do_not_reset_and_replayed_events_do_not_increment() {
        let team = "a".repeat(64);
        let mut guard = TeamLoop::new(HashSet::from([team.clone()]), "self".into(), 1);
        let scope = SessionScope::Conversation {
            channel_id: Uuid::new_v4(),
        };
        assert!(guard.observe_teammate(&scope, "first".into(), true));
        assert!(guard.observe_teammate(&scope, "first".into(), true));
        guard.observe_human(&scope, "edit".into(), 40003);
        assert!(!guard.observe_teammate(&scope, "second".into(), true));
        guard.observe_human(&scope, "reset".into(), 9);
        assert!(guard.observe_teammate(&scope, "third".into(), true));
        guard.observe_human(&scope, "reset".into(), 9);
        assert!(!guard.observe_teammate(&scope, "fourth".into(), true));
    }

    #[test]
    fn thread_scopes_do_not_share_the_cap() {
        let mut guard = TeamLoop::new(HashSet::from(["a".repeat(64)]), "self".into(), 1);
        let channel_id = Uuid::new_v4();
        let left = SessionScope::Thread {
            channel_id,
            root_event_id: "1".repeat(64),
        };
        let right = SessionScope::Thread {
            channel_id,
            root_event_id: "2".repeat(64),
        };
        assert!(guard.observe_teammate(&left, "left-1".into(), true));
        assert!(!guard.observe_teammate(&left, "left-2".into(), true));
        assert!(guard.observe_teammate(&right, "right-1".into(), true));
        guard.observe_human(&left, "left-human".into(), 45003);
        assert!(guard.observe_teammate(&left, "left-3".into(), true));
        assert!(!guard.observe_teammate(&right, "right-2".into(), true));
    }

    #[test]
    fn unrelated_team_chat_does_not_spend_the_budget() {
        let mut guard = TeamLoop::new(HashSet::from(["a".repeat(64)]), "self".into(), 1);
        let scope = SessionScope::Conversation {
            channel_id: Uuid::new_v4(),
        };
        assert!(guard.observe_teammate(&scope, "status".into(), false));
        assert!(guard.observe_teammate(&scope, "first-trigger".into(), true));
        assert!(!guard.observe_teammate(&scope, "second-trigger".into(), true));
    }
}
