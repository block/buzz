use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use anyhow::Result;
use uuid::Uuid;

use crate::relay::RelayEventPublisher;

const DENIAL_BODY: &str =
    "I can only respond to my owner. This build does not allow changing that access setting.";

const DENIAL_WINDOW: Duration = Duration::from_secs(10 * 60);
const DENIALS_PER_CHANNEL_PER_WINDOW: usize = 3;
const MAX_TRACKED_DENIAL_KEYS: usize = 256;

/// Inbound author-gate outcome. The same value decides both forwarding and
/// whether the event loop publishes a denial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum GateOutcome {
    /// The author is eligible. Continue to filtering and queueing.
    Proceed,
    /// The author is ineligible. Drop the event without a reply.
    Drop,
    /// The author is ineligible. Publish a denial, then drop the event.
    DropAndDeny,
}

/// Classify an inbound event using values computed at the event-loop boundary.
///
/// Keeping all four inputs as values makes every branch directly testable. The
/// caller must publish only for [`GateOutcome::DropAndDeny`].
pub(crate) fn classify_inbound(
    allowed: bool,
    mentioned: bool,
    cooled_down: bool,
    kind_allows_denial: bool,
) -> GateOutcome {
    if allowed {
        GateOutcome::Proceed
    } else if mentioned && cooled_down && kind_allows_denial {
        GateOutcome::DropAndDeny
    } else {
        GateOutcome::Drop
    }
}

/// Return whether this event category may produce a denial.
///
/// `denials_enabled` is false for `respond-to=nobody`, which is deliberately
/// silent. Workflow traffic, other denials, and non-message kinds are also
/// suppressed before they reach the classifier's fourth input.
#[must_use]
pub(crate) fn event_allows_denial(
    event: &nostr::Event,
    kind_u32: u32,
    denials_enabled: bool,
) -> bool {
    denials_enabled
        && kind_u32 == buzz_core::kind::KIND_STREAM_MESSAGE
        && !has_marker(event, "buzz:denial")
        && !has_marker(event, "buzz:workflow")
}

fn has_marker(event: &nostr::Event, marker: &str) -> bool {
    event
        .tags
        .iter()
        .any(|tag| tag.as_slice().first().map(String::as_str) == Some(marker))
}

/// Publish an unmentioned, in-thread denial for a rejected inbound event.
pub(crate) async fn publish_denial(
    publisher: &RelayEventPublisher,
    keys: &nostr::Keys,
    channel_id: Uuid,
    triggering_event: &nostr::Event,
) -> Result<()> {
    use buzz_sdk::ThreadRef;

    let thread_tags = crate::queue::parse_thread_tags(triggering_event);
    let root_event_id = match thread_tags.root_event_id {
        Some(root) => nostr::EventId::from_hex(&root)
            .map_err(|error| anyhow::anyhow!("invalid inbound denial thread root: {error}"))?,
        None => triggering_event.id,
    };
    let thread_ref = ThreadRef {
        root_event_id,
        parent_event_id: root_event_id,
    };
    let denial_tag = nostr::Tag::parse(["buzz:denial", "1"])
        .map_err(|error| anyhow::anyhow!("invalid inbound denial marker: {error}"))?;
    // Deliberately pass no mentions. A p-tag-free denial cannot trigger another
    // mention-mode denial. In all-mode the cooldown and cap remain the bound.
    let builder =
        buzz_sdk::build_message(channel_id, DENIAL_BODY, Some(&thread_ref), &[], false, &[])
            .map_err(|error| anyhow::anyhow!("failed to build inbound denial: {error}"))?
            .tags([denial_tag]);
    let event = builder
        .sign_with_keys(keys)
        .map_err(|error| anyhow::anyhow!("failed to sign inbound denial: {error}"))?;
    publisher
        .publish_event(event)
        .await
        .map_err(|error| anyhow::anyhow!("failed to publish inbound denial: {error}"))
}

/// In-memory denial rate limiter.
///
/// The per-author cooldown limits repeated replies to one sender. The
/// per-channel cap is the load-bearing volume bound in DMs, where clients
/// p-tag every recipient on every message, and in many-author storms.
pub(crate) struct DenialLimiter {
    by_author: HashMap<(Uuid, String), Instant>,
    by_channel: HashMap<Uuid, VecDeque<Instant>>,
}

impl DenialLimiter {
    pub(crate) fn new() -> Self {
        Self {
            by_author: HashMap::new(),
            by_channel: HashMap::new(),
        }
    }

    /// Return whether both rate limits and both bookkeeping capacities permit a
    /// denial, without changing limiter state. Existing cooldown timestamps and
    /// channel deques are never evicted. While either map is full, every new key
    /// is refused, including a new author in an existing channel with room. The
    /// ten-minute pruning window eventually removes expired state and resumes
    /// denial service.
    #[must_use]
    pub(crate) fn allows(&mut self, channel_id: Uuid, author: &str) -> bool {
        self.allows_at(channel_id, author, Instant::now())
    }

    /// Attempt to record a denial after the pure classifier selected
    /// `DropAndDeny`. Capacity is rechecked so an unexpected state change fails
    /// closed rather than overrunning a bookkeeping map. The event loop has no
    /// await between its allowance check and this call, so the recheck cannot
    /// currently disagree before the caller publishes.
    pub(crate) fn record(&mut self, channel_id: Uuid, author: &str) {
        let _ = self.try_record_at(channel_id, author, Instant::now());
    }

    fn prune(&mut self, now: Instant) {
        self.by_author
            .retain(|_, sent_at| now.saturating_duration_since(*sent_at) < DENIAL_WINDOW);
        for sent_at in self.by_channel.values_mut() {
            while sent_at
                .front()
                .is_some_and(|at| now.saturating_duration_since(*at) >= DENIAL_WINDOW)
            {
                sent_at.pop_front();
            }
        }
        self.by_channel.retain(|_, sent_at| !sent_at.is_empty());
    }

    fn allows_at(&mut self, channel_id: Uuid, author: &str, now: Instant) -> bool {
        self.prune(now);
        let author_key = (channel_id, author.to_string());
        let author_allowed = !self.by_author.contains_key(&author_key)
            && self.by_author.len() < MAX_TRACKED_DENIAL_KEYS;
        let channel_allowed = self
            .by_channel
            .get(&channel_id)
            .is_some_and(|sent_at| sent_at.len() < DENIALS_PER_CHANNEL_PER_WINDOW)
            || (!self.by_channel.contains_key(&channel_id)
                && self.by_channel.len() < MAX_TRACKED_DENIAL_KEYS);
        author_allowed && channel_allowed
    }

    fn try_record_at(&mut self, channel_id: Uuid, author: &str, now: Instant) -> bool {
        if !self.allows_at(channel_id, author, now) {
            return false;
        }
        let author_key = (channel_id, author.to_string());
        self.by_author.insert(author_key, now);
        self.by_channel
            .entry(channel_id)
            .or_default()
            .push_back(now);
        true
    }

    #[cfg(test)]
    fn try_acquire_at(&mut self, channel_id: Uuid, author: &str, now: Instant) -> bool {
        self.try_record_at(channel_id, author, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use tokio::sync::mpsc;

    fn signed_event(author: &Keys, kind: u32, content: &str, tags: Vec<Tag>) -> nostr::Event {
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags(tags)
            .sign_with_keys(author)
            .expect("sign triggering event")
    }

    fn marker(event: &nostr::Event, name: &str) -> bool {
        event
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some(name))
    }

    fn tag_values(event: &nostr::Event, name: &str) -> Vec<Vec<String>> {
        event
            .tags
            .iter()
            .map(|tag| tag.as_slice())
            .filter(|parts| parts.first().map(String::as_str) == Some(name))
            .map(ToOwned::to_owned)
            .collect()
    }

    async fn assert_no_publish(published_rx: &mut mpsc::Receiver<nostr::Event>, message: &str) {
        assert!(
            tokio::time::timeout(Duration::from_millis(20), published_rx.recv())
                .await
                .is_err(),
            "{message}"
        );
    }

    #[test]
    fn allowed_input_proceeds_when_every_denial_condition_is_true() {
        assert_eq!(
            classify_inbound(true, true, true, true),
            GateOutcome::Proceed,
            "allowed input must make the event proceed"
        );
    }

    #[test]
    fn mentioned_input_suppresses_denial_when_false() {
        assert_eq!(
            classify_inbound(false, false, true, true),
            GateOutcome::Drop,
            "an event without an explicit agent mention must not produce a denial"
        );
    }

    #[test]
    fn cooled_down_input_suppresses_denial_when_false() {
        assert_eq!(
            classify_inbound(false, true, false, true),
            GateOutcome::Drop,
            "an event inside the denial cooldown must not produce a denial"
        );
    }

    #[test]
    fn kind_input_suppresses_denial_when_false() {
        assert_eq!(
            classify_inbound(false, true, true, false),
            GateOutcome::Drop,
            "a non-message, workflow, denial, or deliberately silent mode must not produce a denial"
        );
    }

    #[test]
    fn all_denial_conditions_true_requests_one_denial() {
        assert_eq!(
            classify_inbound(false, true, true, true),
            GateOutcome::DropAndDeny
        );
    }

    #[test]
    fn checking_limits_does_not_record_before_classifier_decision() {
        let channel_id = Uuid::new_v4();
        let mut limiter = DenialLimiter::new();

        assert!(limiter.allows(channel_id, "author"));
        assert!(limiter.allows(channel_id, "author"));
        limiter.record(channel_id, "author");
        assert!(!limiter.allows(channel_id, "author"));
    }

    #[test]
    fn per_author_cooldown_expires_after_window() {
        let channel_id = Uuid::new_v4();
        let started = Instant::now();
        let mut limiter = DenialLimiter::new();

        assert!(limiter.try_acquire_at(channel_id, "author", started));
        assert!(!limiter.try_acquire_at(
            channel_id,
            "author",
            started + DENIAL_WINDOW - Duration::from_secs(1)
        ));
        assert!(limiter.try_acquire_at(channel_id, "author", started + DENIAL_WINDOW));
    }

    #[test]
    fn per_channel_cap_is_independent_of_author_cooldown() {
        let channel_id = Uuid::new_v4();
        let started = Instant::now();
        let mut limiter = DenialLimiter::new();

        for author in ["one", "two", "three"] {
            assert!(limiter.try_acquire_at(channel_id, author, started));
        }
        assert!(
            !limiter.try_acquire_at(channel_id, "four", started),
            "a fourth distinct author must still be blocked by the per-channel cap"
        );
        assert!(limiter.try_acquire_at(channel_id, "four", started + DENIAL_WINDOW));
    }

    #[test]
    fn channel_capacity_never_repeals_an_existing_channel_cap() {
        let victim = Uuid::new_v4();
        let started = Instant::now();
        let mut limiter = DenialLimiter::new();

        limiter.by_channel.insert(
            victim,
            VecDeque::from(vec![started; DENIALS_PER_CHANNEL_PER_WINDOW]),
        );
        for filler_index in 0..(MAX_TRACKED_DENIAL_KEYS - 1) {
            limiter.by_channel.insert(
                Uuid::from_u128(filler_index as u128 + 1),
                VecDeque::from([started]),
            );
        }
        assert_eq!(limiter.by_channel.len(), MAX_TRACKED_DENIAL_KEYS);

        let overflow_channel = Uuid::from_u128(MAX_TRACKED_DENIAL_KEYS as u128 + 1);
        assert!(
            !limiter.try_acquire_at(overflow_channel, "overflow-author", started),
            "a channel beyond bookkeeping capacity must fail closed"
        );
        for attempt in 0..64 {
            assert!(
                !limiter.try_acquire_at(victim, &format!("retry-{attempt}"), started),
                "the victim channel cap must survive the capacity boundary"
            );
        }
        assert_eq!(limiter.by_channel.len(), MAX_TRACKED_DENIAL_KEYS);
        assert_eq!(
            limiter.by_channel.get(&victim).map(VecDeque::len),
            Some(DENIALS_PER_CHANNEL_PER_WINDOW)
        );
    }

    #[test]
    fn full_author_map_fails_closed_without_erasing_existing_cooldowns() {
        let started = Instant::now();
        let mut limiter = DenialLimiter::new();

        for index in 0..MAX_TRACKED_DENIAL_KEYS {
            limiter.by_author.insert(
                (
                    Uuid::from_u128(index as u128 + 1),
                    format!("author-{index}"),
                ),
                started,
            );
        }
        let existing_key = limiter.by_author.keys().next().cloned().unwrap();
        let new_channel = Uuid::new_v4();

        assert!(!limiter.allows_at(new_channel, "new-author", started));
        assert!(!limiter.try_acquire_at(new_channel, "new-author", started));
        assert_eq!(limiter.by_author.len(), MAX_TRACKED_DENIAL_KEYS);
        assert!(limiter.by_author.contains_key(&existing_key));
    }

    #[test]
    fn nobody_mode_suppresses_denial_for_an_otherwise_qualifying_message() {
        let event = signed_event(&Keys::generate(), 9, "@agent please answer", vec![]);
        assert!(
            !event_allows_denial(&event, 9, false),
            "respond-to=nobody must stay silent"
        );
    }

    #[tokio::test]
    async fn denial_publish_has_owner_only_copy_marker_no_p_tags_and_flat_threading() {
        let channel_id = Uuid::new_v4();
        let agent = Keys::generate();
        let requester = Keys::generate();
        let root = signed_event(&requester, 9, "root", vec![]);
        let nested = signed_event(
            &requester,
            9,
            "@agent please answer",
            vec![
                Tag::parse(["h", &channel_id.to_string()]).expect("h tag"),
                Tag::parse(["e", &root.id.to_hex(), "", "root"]).expect("root tag"),
                Tag::parse(["e", &Keys::generate().public_key().to_hex(), "", "reply"])
                    .expect("reply tag"),
                Tag::parse(["p", &agent.public_key().to_hex()]).expect("p tag"),
            ],
        );
        let (publisher, mut published_rx) = RelayEventPublisher::test_pair();

        publish_denial(&publisher, &agent, channel_id, &nested)
            .await
            .expect("publish denial");
        let denial = published_rx.recv().await.expect("one published denial");

        assert_eq!(denial.kind.as_u16() as u32, 9);
        assert_eq!(denial.content, DENIAL_BODY);
        assert!(denial.content.contains("only respond to my owner"));
        assert!(marker(&denial, "buzz:denial"));
        assert!(
            tag_values(&denial, "p").is_empty(),
            "denials must not p-tag the requester or agent"
        );
        assert_eq!(
            tag_values(&denial, "e"),
            vec![vec![
                "e".to_string(),
                root.id.to_hex(),
                "".to_string(),
                "reply".to_string()
            ]],
            "a nested trigger must receive a flat reply to its thread root"
        );
        assert_no_publish(
            &mut published_rx,
            "one trigger must publish exactly one denial",
        )
        .await;
    }

    #[tokio::test]
    async fn top_level_denial_replies_to_triggering_event() {
        let channel_id = Uuid::new_v4();
        let agent = Keys::generate();
        let requester = Keys::generate();
        let trigger = signed_event(&requester, 9, "@agent please answer", vec![]);
        let (publisher, mut published_rx) = RelayEventPublisher::test_pair();

        publish_denial(&publisher, &agent, channel_id, &trigger)
            .await
            .expect("publish denial");
        let denial = published_rx.recv().await.expect("one published denial");
        assert_eq!(
            tag_values(&denial, "e"),
            vec![vec![
                "e".to_string(),
                trigger.id.to_hex(),
                "".to_string(),
                "reply".to_string()
            ]]
        );
    }

    #[test]
    fn workflow_and_denial_markers_suppress_denial() {
        let author = Keys::generate();
        for marker_name in ["buzz:workflow", "buzz:denial"] {
            let event = signed_event(
                &author,
                9,
                "marked message",
                vec![Tag::parse([marker_name, "true"]).expect("marker tag")],
            );
            assert!(
                !event_allows_denial(&event, 9, true),
                "{marker_name} traffic must stay silent"
            );
        }
    }

    #[test]
    fn non_message_kind_suppresses_denial_even_when_mentioned() {
        let agent_hex = Keys::generate().public_key().to_hex();
        let event = signed_event(
            &Keys::generate(),
            buzz_core::kind::KIND_STREAM_REMINDER,
            "reminder",
            vec![Tag::parse(["p", &agent_hex]).expect("p tag")],
        );
        assert!(crate::event_mentions_agent(&event, &agent_hex));
        assert!(
            !event_allows_denial(&event, buzz_core::kind::KIND_STREAM_REMINDER, true),
            "a mentioned non-message kind must not produce a denial"
        );
    }
}
