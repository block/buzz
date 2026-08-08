//! Thread-follow support: a bounded set of thread roots the agent
//! participates in.
//!
//! When `--thread-follow` is enabled (mentions mode only), the harness
//! tracks the NIP-10 roots of threads where the agent was @mentioned or
//! has itself replied. Subsequent replies in a followed thread trigger
//! turns without requiring a fresh @mention.
//!
//! The set is in-memory and bounded: once `cap` roots are tracked, the
//! oldest-inserted root is evicted. A harness restart starts with an
//! empty set; the mention catch-up pass repopulates roots for threads
//! with recent agent mentions.

use std::collections::{HashSet, VecDeque};

/// Default capacity for the harness's follow set. Generous for a single
/// agent (a root is ~64 bytes, so the ceiling is a few hundred KB) while
/// still bounding memory over long uptimes.
pub const DEFAULT_CAP: usize = 1024;

/// Bounded, insertion-ordered set of thread-root event ids (hex).
///
/// Deliberately dependency-free: `HashSet` for membership plus a
/// `VecDeque` for FIFO eviction. Re-inserting a known root is a no-op
/// (it does not refresh eviction order); with a generous capacity the
/// simpler semantics are worth it.
pub struct ThreadFollowSet {
    set: HashSet<String>,
    order: VecDeque<String>,
    cap: usize,
}

impl ThreadFollowSet {
    /// Create a set that tracks at most `cap` roots (minimum 1).
    pub fn new(cap: usize) -> Self {
        let cap = cap.max(1);
        Self {
            set: HashSet::with_capacity(cap),
            order: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// Track a thread root. Evicts the oldest root when full.
    pub fn insert(&mut self, root: String) {
        if self.set.contains(&root) {
            return;
        }
        while self.set.len() >= self.cap {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            } else {
                break;
            }
        }
        self.order.push_back(root.clone());
        self.set.insert(root);
    }

    /// Whether `root` is currently followed.
    pub fn contains(&self, root: &str) -> bool {
        self.set.contains(root)
    }

    /// Number of tracked roots (for logging/tests).
    pub fn len(&self) -> usize {
        self.set.len()
    }

    /// Whether the set is empty.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }
}

/// Extract the NIP-10 thread root of `event`, if it is a thread reply.
///
/// Preference order:
/// 1. `["e", <id>, .., "root"]` — explicit root marker.
/// 2. `["e", <id>, .., "reply"]` — reply without a root marker; the
///    referenced parent is treated as the root (single-level thread).
/// 3. First bare `["e", <id>]` tag — deprecated positional style.
///
/// Returns `None` for top-level messages (no `e` tags).
pub fn thread_root_of(event: &nostr::Event) -> Option<String> {
    let mut root: Option<&str> = None;
    let mut reply: Option<&str> = None;
    let mut bare: Option<&str> = None;
    for tag in event.tags.iter() {
        let s = tag.as_slice();
        if s.first().map(|k| k.as_str()) != Some("e") {
            continue;
        }
        let Some(id) = s.get(1).map(|v| v.as_str()) else {
            continue;
        };
        match s.get(3).map(|m| m.as_str()) {
            Some("root") => root = root.or(Some(id)),
            Some("reply") => reply = reply.or(Some(id)),
            _ => bare = bare.or(Some(id)),
        }
    }
    root.or(reply).or(bare).map(str::to_owned)
}

/// The root under which `event`'s thread should be followed: its NIP-10
/// root when it is a reply, otherwise its own id (a top-level message
/// starts a thread rooted at itself).
pub fn followable_root(event: &nostr::Event) -> String {
    thread_root_of(event).unwrap_or_else(|| event.id.to_hex())
}

/// Whether `event` carries a `p` tag for `pubkey_hex`.
pub fn mentions_pubkey(event: &nostr::Event, pubkey_hex: &str) -> bool {
    event.tags.iter().any(|tag| {
        let s = tag.as_slice();
        s.first().map(|k| k.as_str()) == Some("p")
            && s.get(1).map(|v| v.as_str()) == Some(pubkey_hex)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn event_with_tags(tags: Vec<Tag>) -> nostr::Event {
        let keys = Keys::generate();
        EventBuilder::new(Kind::Custom(9), "hello")
            .tags(tags)
            .sign_with_keys(&keys)
            .unwrap()
    }

    fn e_tag(id: &str, marker: Option<&str>) -> Tag {
        let mut parts = vec!["e".to_string(), id.to_string()];
        if let Some(m) = marker {
            parts.push(String::new());
            parts.push(m.to_string());
        }
        Tag::parse(parts).unwrap()
    }

    const ROOT: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const PARENT: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    #[test]
    fn root_marker_wins() {
        let event = event_with_tags(vec![
            e_tag(ROOT, Some("root")),
            e_tag(PARENT, Some("reply")),
        ]);
        assert_eq!(thread_root_of(&event).as_deref(), Some(ROOT));
    }

    #[test]
    fn reply_marker_without_root() {
        let event = event_with_tags(vec![e_tag(PARENT, Some("reply"))]);
        assert_eq!(thread_root_of(&event).as_deref(), Some(PARENT));
    }

    #[test]
    fn bare_e_tag_fallback() {
        let event = event_with_tags(vec![e_tag(ROOT, None)]);
        assert_eq!(thread_root_of(&event).as_deref(), Some(ROOT));
    }

    #[test]
    fn top_level_has_no_root_and_follows_itself() {
        let event = event_with_tags(vec![]);
        assert_eq!(thread_root_of(&event), None);
        assert_eq!(followable_root(&event), event.id.to_hex());
    }

    #[test]
    fn mentions_pubkey_matches_p_tag() {
        let keys = Keys::generate();
        let pk = keys.public_key();
        let event = event_with_tags(vec![Tag::public_key(pk)]);
        assert!(mentions_pubkey(&event, &pk.to_hex()));
        assert!(!mentions_pubkey(&event, ROOT));
    }

    #[test]
    fn follow_set_inserts_and_contains() {
        let mut set = ThreadFollowSet::new(4);
        assert!(set.is_empty());
        set.insert(ROOT.into());
        assert!(!set.is_empty());
        assert!(set.contains(ROOT));
        assert!(!set.contains(PARENT));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn follow_set_reinsert_is_noop() {
        let mut set = ThreadFollowSet::new(4);
        set.insert(ROOT.into());
        set.insert(ROOT.into());
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn follow_set_evicts_oldest_at_cap() {
        let mut set = ThreadFollowSet::new(2);
        set.insert("a".into());
        set.insert("b".into());
        set.insert("c".into());
        assert_eq!(set.len(), 2);
        assert!(!set.contains("a"));
        assert!(set.contains("b"));
        assert!(set.contains("c"));
    }

    #[test]
    fn follow_set_zero_cap_clamps_to_one() {
        let mut set = ThreadFollowSet::new(0);
        set.insert("a".into());
        assert!(set.contains("a"));
        set.insert("b".into());
        assert!(!set.contains("a"));
        assert!(set.contains("b"));
    }
}
