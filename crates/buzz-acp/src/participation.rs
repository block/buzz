//! Smart thread participation: continue conversations without a fresh @mention.
//!
//! When enabled under `subscribe=mentions`, the harness still only *acts* on:
//! 1. Explicit @mentions of this agent (always), or
//! 2. Bare human replies in a thread where this agent is already an active
//!    participant.
//!
//! Exclusive addressing of someone else (other `p` tags, this agent absent)
//! suppresses auto-continue so the human can hand the thread to another person
//! or agent without the prior participant barging in.
//!
//! Agent-authored events never auto-continue. That blocks A↔B ping-pong when
//! both agents share a thread. Explicit @ still works through the mention path.
//!
//! Active threads are persisted to disk (and rehydrated from recent relay
//! history on startup) so harness restarts do not wipe continuation state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nostr::Event;
use serde::{Deserialize, Serialize};

use crate::queue::{parse_thread_tags, ThreadTags};

/// Default how long a thread stays active without further participation.
pub const DEFAULT_THREAD_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Soft cap on tracked roots (LRU eviction by last activity).
pub const DEFAULT_MAX_ACTIVE_THREADS: usize = 256;

/// On-disk schema version.
const STATE_VERSION: u32 = 1;

/// Why an event was admitted under the participation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitReason {
    /// Event `p`-tags this agent.
    Mention,
    /// Bare (or non-exclusive) human reply in an active thread.
    ThreadContinuation,
}

/// Decision for a single inbound event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParticipationDecision {
    /// Forward to the agent. Optionally mark/refresh a thread root.
    Admit {
        reason: AdmitReason,
        /// Root to mark active. For top-level mentions this is the event id
        /// itself (it becomes the thread root when the agent replies).
        mark_root: Option<String>,
    },
    /// Drop silently.
    Drop,
}

/// Disk file shape for active-thread state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ActiveThreadsStateFile {
    version: u32,
    /// root_id (hex) → last activity as unix milliseconds.
    roots: HashMap<String, u64>,
}

/// Set of thread roots this agent is participating in.
///
/// Activity is stored as absolute unix milliseconds so state survives process
/// restarts (unlike [`std::time::Instant`]).
#[derive(Debug)]
pub struct ActiveThreads {
    /// root_id → last activity unix ms.
    roots: HashMap<String, u64>,
    ttl: Duration,
    max: usize,
    /// When set, every mark/gc that changes membership is written here.
    persist_path: Option<PathBuf>,
}

impl Default for ActiveThreads {
    fn default() -> Self {
        Self::new(DEFAULT_THREAD_TTL, DEFAULT_MAX_ACTIVE_THREADS)
    }
}

impl ActiveThreads {
    pub fn new(ttl: Duration, max: usize) -> Self {
        Self {
            roots: HashMap::new(),
            ttl,
            max: max.max(1),
            persist_path: None,
        }
    }

    /// Enable (or disable) automatic disk persistence after mutations.
    pub fn with_persist_path(mut self, path: Option<PathBuf>) -> Self {
        self.persist_path = path;
        self
    }

    /// Mark (or refresh) a thread root as active and persist if configured.
    pub fn mark(&mut self, root_id: &str) {
        let key = root_id.trim().to_ascii_lowercase();
        if key.is_empty() || !is_event_id_hex(&key) {
            return;
        }
        self.gc();
        self.roots.insert(key, now_unix_ms());
        self.evict_if_needed();
        self.persist();
    }

    /// Mark without writing disk (used during bulk rehydrate).
    fn mark_at(&mut self, root_id: &str, at_ms: u64) {
        let key = root_id.trim().to_ascii_lowercase();
        if key.is_empty() || !is_event_id_hex(&key) {
            return;
        }
        let existing = self.roots.get(&key).copied().unwrap_or(0);
        if at_ms >= existing {
            self.roots.insert(key, at_ms);
        }
    }

    /// True when `root_id` is tracked and not expired.
    pub fn is_active(&mut self, root_id: &str) -> bool {
        let key = root_id.trim().to_ascii_lowercase();
        self.gc();
        match self.roots.get(&key) {
            Some(&at_ms) if age_ms(at_ms) <= self.ttl_ms() => true,
            Some(_) => {
                self.roots.remove(&key);
                self.persist();
                false
            }
            None => false,
        }
    }

    pub fn len(&self) -> usize {
        self.roots.len()
    }

    /// Load roots from `path` (if it exists), dropping expired entries.
    pub fn load_from_path(&mut self, path: &Path) -> Result<usize, String> {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(format!("read {}: {e}", path.display())),
        };
        let file: ActiveThreadsStateFile = serde_json::from_str(&raw)
            .map_err(|e| format!("parse {}: {e}", path.display()))?;
        if file.version != STATE_VERSION && file.version != 0 {
            tracing::warn!(
                path = %path.display(),
                version = file.version,
                "active-threads state version mismatch — loading best-effort"
            );
        }
        for (root, at_ms) in file.roots {
            if age_ms(at_ms) <= self.ttl_ms() {
                self.mark_at(&root, at_ms);
            }
        }
        self.evict_if_needed();
        Ok(self.roots.len())
    }

    /// Load into a fresh store from disk.
    pub fn load(path: &Path, ttl: Duration, max: usize) -> Result<Self, String> {
        let mut store = Self::new(ttl, max).with_persist_path(Some(path.to_path_buf()));
        let _ = store.load_from_path(path)?;
        Ok(store)
    }

    /// Write current (non-expired) roots to disk.
    pub fn persist(&self) {
        let Some(path) = self.persist_path.as_ref() else {
            return;
        };
        if let Err(e) = self.persist_to(path) {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "failed to persist active-thread state"
            );
        }
    }

    fn persist_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        let mut roots = HashMap::new();
        let ttl_ms = self.ttl_ms();
        for (k, &at_ms) in &self.roots {
            if age_ms(at_ms) <= ttl_ms {
                roots.insert(k.clone(), at_ms);
            }
        }
        let file = ActiveThreadsStateFile {
            version: STATE_VERSION,
            roots,
        };
        let bytes = serde_json::to_vec_pretty(&file)
            .map_err(|e| format!("serialize active-threads: {e}"))?;
        atomic_write(path, &bytes)
    }

    /// Absorb participation roots from raw Nostr event JSON (array of events).
    ///
    /// Marks a root when:
    /// - this agent authored the event, or
    /// - this agent is `p`-tagged (mentioned).
    ///
    /// Uses each event's `created_at` (seconds) as the activity timestamp so
    /// rehydrate from history does not artificially extend TTL forever.
    pub fn absorb_events_json(&mut self, events: &serde_json::Value, agent_pubkey_hex: &str) -> usize {
        let agent = agent_pubkey_hex.trim().to_ascii_lowercase();
        let Some(arr) = events.as_array() else {
            return 0;
        };
        let before = self.roots.len();
        for ev in arr {
            let author = ev
                .get("pubkey")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let event_id = ev
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let created_at = ev.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
            let at_ms = created_at.saturating_mul(1000);

            let tags = ev.get("tags").and_then(|t| t.as_array());
            let (root, mentioned) = participation_hints_from_json_tags(tags, &agent);

            let authored = !agent.is_empty() && author == agent;
            if !authored && !mentioned {
                continue;
            }
            let mark = root
                .or_else(|| {
                    if is_event_id_hex(&event_id) {
                        Some(event_id)
                    } else {
                        None
                    }
                });
            if let Some(r) = mark {
                self.mark_at(&r, at_ms);
            }
        }
        self.gc();
        self.evict_if_needed();
        self.roots.len().saturating_sub(before)
    }

    fn ttl_ms(&self) -> u64 {
        self.ttl.as_millis().min(u128::from(u64::MAX)) as u64
    }

    fn gc(&mut self) {
        let ttl_ms = self.ttl_ms();
        self.roots.retain(|_, at_ms| age_ms(*at_ms) <= ttl_ms);
    }

    fn evict_if_needed(&mut self) {
        while self.roots.len() > self.max {
            let oldest = self
                .roots
                .iter()
                .min_by_key(|(_, at)| *at)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                self.roots.remove(&k);
            } else {
                break;
            }
        }
    }
}

/// Resolve the on-disk path for this agent's active-thread state.
///
/// Override with `BUZZ_ACP_STATE_DIR`. Default: `$HOME/.local/share/buzz-acp`
/// (or `%LOCALAPPDATA%/buzz-acp` via HOME-less fallbacks).
pub fn active_threads_state_path(agent_pubkey_hex: &str) -> PathBuf {
    let dir = state_dir();
    let pk = agent_pubkey_hex.trim().to_ascii_lowercase();
    let safe: String = pk
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(64)
        .collect();
    let name = if safe.is_empty() {
        "active-threads-unknown.json".into()
    } else {
        format!("active-threads-{safe}.json")
    };
    dir.join(name)
}

fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("BUZZ_ACP_STATE_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".local/share/buzz-acp");
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("buzz-acp");
    }
    PathBuf::from(".").join("buzz-acp-state")
}

/// Best-effort rehydrate from the relay: recent events authored by or
/// mentioning this agent within the participation TTL window.
pub async fn rehydrate_from_relay(
    active: &mut ActiveThreads,
    rest_client: &crate::relay::RestClient,
    agent_pubkey_hex: &str,
    ttl: Duration,
) -> usize {
    let Ok(agent_pk) = nostr::PublicKey::from_hex(agent_pubkey_hex) else {
        return 0;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let since = now.saturating_sub(ttl.as_secs());
    let since_ts = nostr::Timestamp::from_secs(since);

    // Mentions of this agent.
    let mentions = nostr::Filter::new()
        .kind(nostr::Kind::from(9u16))
        .pubkey(agent_pk)
        .since(since_ts)
        .limit(200);
    // Events this agent authored (replies that enrolled the thread).
    let authored = nostr::Filter::new()
        .kind(nostr::Kind::from(9u16))
        .author(agent_pk)
        .since(since_ts)
        .limit(200);

    let mut total = 0usize;
    for filter in [mentions, authored] {
        let resp = match tokio::time::timeout(
            Duration::from_secs(5),
            rest_client.query(std::slice::from_ref(&filter)),
        )
        .await
        {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "active-thread rehydrate query failed");
                continue;
            }
            Err(_) => {
                tracing::warn!("active-thread rehydrate query timed out");
                continue;
            }
        };
        total = total.saturating_add(active.absorb_events_json(&resp, agent_pubkey_hex));
    }
    active.persist();
    total
}

/// Inputs for the pure participation decision.
#[derive(Debug, Clone)]
pub struct ParticipationContext<'a> {
    pub agent_pubkey_hex: &'a str,
    pub event_id_hex: &'a str,
    pub author_pubkey_hex: &'a str,
    pub thread: &'a ThreadTags,
    /// True when the author is classified as an agent (NIP-OA profile, or a
    /// known same-owner sibling). Humans and unknowns are `false`.
    pub author_is_agent: bool,
    /// Whether this root is currently active for the agent.
    pub thread_is_active: bool,
}

/// Decide whether the harness should wake the agent for this event.
///
/// Call only when thread participation is enabled. The author gate
/// (`respond_to`) must already have passed.
pub fn decide(ctx: &ParticipationContext<'_>) -> ParticipationDecision {
    let agent = ctx.agent_pubkey_hex.trim().to_ascii_lowercase();
    let author = ctx.author_pubkey_hex.trim().to_ascii_lowercase();

    // Never auto-process our own events here; ignore_self handles that earlier,
    // but keep the pure function safe if called standalone.
    if !agent.is_empty() && author == agent {
        return ParticipationDecision::Drop;
    }

    let mentioned = ctx
        .thread
        .mentioned_pubkeys
        .iter()
        .any(|pk| pk.eq_ignore_ascii_case(&agent));

    if mentioned {
        let mark_root = participation_root(ctx.thread, ctx.event_id_hex);
        return ParticipationDecision::Admit {
            reason: AdmitReason::Mention,
            mark_root,
        };
    }

    // Exclusive handoff: someone *else* is p-tagged and we are not.
    // Ignore the event author — Buzz always p-tags the parent author on reply
    // (often the human's own prior message). That is not "talking to someone
    // else"; only additional p-tags count as handoff.
    if has_exclusive_other_mentions(ctx.thread, &agent, &author) {
        return ParticipationDecision::Drop;
    }

    // Thread continuation only for humans (loop guard).
    if ctx.author_is_agent {
        return ParticipationDecision::Drop;
    }

    let Some(root) = ctx.thread.root_event_id.as_deref() else {
        // Top-level, no mention, no thread → stay quiet.
        return ParticipationDecision::Drop;
    };

    if !ctx.thread_is_active {
        return ParticipationDecision::Drop;
    }

    ParticipationDecision::Admit {
        reason: AdmitReason::ThreadContinuation,
        mark_root: Some(root.to_ascii_lowercase()),
    }
}

/// Resolve the root id that should become active for this event.
///
/// Threaded events use the NIP-10 root. Top-level mentions use the event's own
/// id so subsequent replies (which will tag it as root/reply) continue.
pub fn participation_root(thread: &ThreadTags, event_id_hex: &str) -> Option<String> {
    if let Some(root) = thread.root_event_id.as_deref() {
        let key = root.trim().to_ascii_lowercase();
        if is_event_id_hex(&key) {
            return Some(key);
        }
    }
    let key = event_id_hex.trim().to_ascii_lowercase();
    if is_event_id_hex(&key) {
        Some(key)
    } else {
        None
    }
}

/// True when the event p-tags someone other than this agent and the author.
///
/// Parent-author `p` tags (NIP-10 / Buzz `buildReplyTags`) are not a handoff
/// when they equal the message author (self-reply). Explicit @ of another
/// person/agent still suppresses continuation.
fn has_exclusive_other_mentions(
    thread: &ThreadTags,
    agent_pubkey_hex: &str,
    author_pubkey_hex: &str,
) -> bool {
    thread.mentioned_pubkeys.iter().any(|pk| {
        let p = pk.trim();
        !p.is_empty()
            && !p.eq_ignore_ascii_case(agent_pubkey_hex)
            && !p.eq_ignore_ascii_case(author_pubkey_hex)
    })
}

/// Convenience: parse tags and decide in one step (tests + call sites).
pub fn decide_event(
    event: &Event,
    agent_pubkey_hex: &str,
    author_is_agent: bool,
    active: &mut ActiveThreads,
) -> ParticipationDecision {
    let thread = parse_thread_tags(event);
    let event_id = event.id.to_hex();
    let author = event.pubkey.to_hex();
    let root = thread.root_event_id.as_deref();
    let thread_is_active = root.map(|r| active.is_active(r)).unwrap_or(false);

    let decision = decide(&ParticipationContext {
        agent_pubkey_hex,
        event_id_hex: &event_id,
        author_pubkey_hex: &author,
        thread: &thread,
        author_is_agent,
        thread_is_active,
    });

    if let ParticipationDecision::Admit {
        mark_root: Some(ref root),
        ..
    } = decision
    {
        active.mark(root);
    }

    decision
}

fn participation_hints_from_json_tags(
    tags: Option<&Vec<serde_json::Value>>,
    agent_pubkey_hex: &str,
) -> (Option<String>, bool) {
    let Some(tags) = tags else {
        return (None, false);
    };
    let mut root = None;
    let mut reply = None;
    let mut mentioned = false;
    for tag in tags {
        let Some(parts) = tag.as_array() else {
            continue;
        };
        let kind = parts.first().and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "e" if parts.len() >= 4 => {
                let id = parts[1].as_str().unwrap_or("").to_ascii_lowercase();
                let marker = parts[3].as_str().unwrap_or("");
                if is_event_id_hex(&id) {
                    match marker {
                        "root" => root = Some(id),
                        "reply" => reply = Some(id),
                        _ => {}
                    }
                }
            }
            "p" if parts.len() >= 2 => {
                if parts[1]
                    .as_str()
                    .is_some_and(|pk| pk.eq_ignore_ascii_case(agent_pubkey_hex))
                {
                    mentioned = true;
                }
            }
            _ => {}
        }
    }
    let root = match (root, reply) {
        (Some(r), _) => Some(r),
        (None, Some(p)) => Some(p),
        (None, None) => None,
    };
    (root, mentioned)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn age_ms(at_ms: u64) -> u64 {
    now_unix_ms().saturating_sub(at_ms)
}

fn is_event_id_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", tmp.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn agent_pk() -> String {
        "a".repeat(64)
    }
    fn human_pk() -> String {
        "b".repeat(64)
    }
    fn other_pk() -> String {
        "c".repeat(64)
    }
    fn root_id() -> String {
        "d".repeat(64)
    }
    fn parent_id() -> String {
        "e".repeat(64)
    }

    fn keys_for(hex_seed_byte: u8) -> Keys {
        let mut sk = [hex_seed_byte; 32];
        sk[0] = hex_seed_byte;
        Keys::new(nostr::SecretKey::from_slice(&sk).expect("secret key"))
    }

    fn make_event(author: &Keys, content: &str, tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::from(9u16), content)
            .tags(tags)
            .sign_with_keys(author)
            .expect("sign")
    }

    fn p_tag(pk: &str) -> Tag {
        Tag::parse(["p", pk]).expect("p tag")
    }

    fn e_tag(id: &str, marker: &str) -> Tag {
        Tag::parse(["e", id, "", marker]).expect("e tag")
    }

    #[test]
    fn mention_admits_and_marks_top_level_as_root() {
        let author = keys_for(1);
        let agent = agent_pk();
        let event = make_event(&author, "hey", vec![p_tag(&agent)]);
        let mut active = ActiveThreads::default();

        let d = decide_event(&event, &agent, false, &mut active);
        assert!(matches!(
            d,
            ParticipationDecision::Admit {
                reason: AdmitReason::Mention,
                ..
            }
        ));
        assert!(active.is_active(&event.id.to_hex()));
    }

    #[test]
    fn mention_in_thread_marks_thread_root() {
        let author = keys_for(1);
        let agent = agent_pk();
        let root = root_id();
        let event = make_event(
            &author,
            "hey in thread",
            vec![e_tag(&root, "root"), e_tag(&root, "reply"), p_tag(&agent)],
        );
        let mut active = ActiveThreads::default();

        let d = decide_event(&event, &agent, false, &mut active);
        assert!(matches!(
            d,
            ParticipationDecision::Admit {
                reason: AdmitReason::Mention,
                mark_root: Some(ref r)
            } if r == &root
        ));
        assert!(active.is_active(&root));
    }

    #[test]
    fn bare_reply_in_active_thread_continues_for_human() {
        let author = keys_for(2);
        let agent = agent_pk();
        let root = root_id();
        let mut active = ActiveThreads::default();
        active.mark(&root);

        let event = make_event(
            &author,
            "follow up without alias",
            vec![e_tag(&root, "root"), e_tag(&parent_id(), "reply")],
        );
        let d = decide_event(&event, &agent, false, &mut active);
        assert_eq!(
            d,
            ParticipationDecision::Admit {
                reason: AdmitReason::ThreadContinuation,
                mark_root: Some(root.clone()),
            }
        );
        assert!(active.is_active(&root));
    }

    #[test]
    fn bare_reply_without_active_thread_drops() {
        let author = keys_for(2);
        let agent = agent_pk();
        let root = root_id();
        let mut active = ActiveThreads::default();

        let event = make_event(
            &author,
            "random reply",
            vec![e_tag(&root, "root"), e_tag(&root, "reply")],
        );
        let d = decide_event(&event, &agent, false, &mut active);
        assert_eq!(d, ParticipationDecision::Drop);
    }

    #[test]
    fn exclusive_other_mention_suppresses_continuation() {
        let author = keys_for(2);
        let agent = agent_pk();
        let other = other_pk();
        let root = root_id();
        let mut active = ActiveThreads::default();
        active.mark(&root);

        let event = make_event(
            &author,
            "talking to someone else",
            vec![e_tag(&root, "root"), e_tag(&root, "reply"), p_tag(&other)],
        );
        let d = decide_event(&event, &agent, false, &mut active);
        assert_eq!(d, ParticipationDecision::Drop);
    }

    #[test]
    fn self_parent_p_tag_does_not_suppress_continuation() {
        // Buzz buildReplyTags always p-tags the parent author. When the human
        // replies to their own prior message in an active agent thread, that
        // p-tag is themselves — not a handoff.
        let author = keys_for(2);
        let agent = agent_pk();
        let root = root_id();
        let mut active = ActiveThreads::default();
        active.mark(&root);

        let event = make_event(
            &author,
            "follow up to myself in the thread",
            vec![
                e_tag(&root, "root"),
                e_tag(&parent_id(), "reply"),
                p_tag(&author.public_key().to_hex()),
            ],
        );
        let d = decide_event(&event, &agent, false, &mut active);
        assert_eq!(
            d,
            ParticipationDecision::Admit {
                reason: AdmitReason::ThreadContinuation,
                mark_root: Some(root),
            }
        );
    }

    #[test]
    fn mention_with_others_still_admits() {
        let author = keys_for(2);
        let agent = agent_pk();
        let other = other_pk();
        let root = root_id();
        let mut active = ActiveThreads::default();

        let event = make_event(
            &author,
            "hey both of you",
            vec![
                e_tag(&root, "root"),
                e_tag(&root, "reply"),
                p_tag(&other),
                p_tag(&agent),
            ],
        );
        let d = decide_event(&event, &agent, false, &mut active);
        assert!(matches!(
            d,
            ParticipationDecision::Admit {
                reason: AdmitReason::Mention,
                ..
            }
        ));
    }

    #[test]
    fn agent_author_does_not_auto_continue() {
        let author = keys_for(3);
        let agent = agent_pk();
        let root = root_id();
        let mut active = ActiveThreads::default();
        active.mark(&root);

        let event = make_event(
            &author,
            "sibling agent chatter",
            vec![e_tag(&root, "root"), e_tag(&root, "reply")],
        );
        let d = decide_event(&event, &agent, true, &mut active);
        assert_eq!(d, ParticipationDecision::Drop);
    }

    #[test]
    fn top_level_without_mention_drops() {
        let author = keys_for(2);
        let agent = agent_pk();
        let mut active = ActiveThreads::default();
        let event = make_event(&author, "hello channel", vec![]);
        let d = decide_event(&event, &agent, false, &mut active);
        assert_eq!(d, ParticipationDecision::Drop);
    }

    #[test]
    fn lru_evicts_oldest_when_over_cap() {
        let mut active = ActiveThreads::new(Duration::from_secs(3600), 2);
        let r1 = "1".repeat(64);
        let r2 = "2".repeat(64);
        let r3 = "3".repeat(64);
        active.mark_at(&r1, 1_000);
        active.mark_at(&r2, 2_000);
        active.mark_at(&r3, 3_000);
        active.evict_if_needed();
        assert!(!active.roots.contains_key(&r1));
        assert!(active.roots.contains_key(&r2));
        assert!(active.roots.contains_key(&r3));
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn expired_root_is_inactive() {
        let mut active = ActiveThreads::new(Duration::from_millis(50), 16);
        let root = root_id();
        // Backdate activity so age > TTL without sleeping the process long.
        active.mark_at(&root, now_unix_ms().saturating_sub(200));
        assert!(!active.is_active(&root));
    }

    #[test]
    fn decide_pure_top_level_mention_marks_event_id() {
        let thread = ThreadTags {
            root_event_id: None,
            parent_event_id: None,
            mentioned_pubkeys: vec![agent_pk()],
        };
        let event_id = "f".repeat(64);
        let d = decide(&ParticipationContext {
            agent_pubkey_hex: &agent_pk(),
            event_id_hex: &event_id,
            author_pubkey_hex: &human_pk(),
            thread: &thread,
            author_is_agent: false,
            thread_is_active: false,
        });
        assert_eq!(
            d,
            ParticipationDecision::Admit {
                reason: AdmitReason::Mention,
                mark_root: Some(event_id),
            }
        );
    }

    #[test]
    fn persist_and_reload_survives_restart() {
        let dir = std::env::temp_dir().join(format!(
            "buzz-acp-participation-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("active-threads.json");
        let root = root_id();

        {
            let mut active =
                ActiveThreads::new(DEFAULT_THREAD_TTL, 16).with_persist_path(Some(path.clone()));
            active.mark(&root);
            assert!(path.exists());
        }

        // Fresh process: load from disk.
        let mut restored = ActiveThreads::load(&path, DEFAULT_THREAD_TTL, 16).unwrap();
        assert!(
            restored.is_active(&root),
            "root should survive restart via disk"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn absorb_events_json_marks_mentions_and_authored() {
        let agent = agent_pk();
        let root = root_id();
        let other_root = "e".repeat(64);
        let events = serde_json::json!([
            {
                "id": "1".repeat(64),
                "pubkey": "b".repeat(64),
                "created_at": now_unix_ms() / 1000,
                "tags": [
                    ["e", root, "", "root"],
                    ["e", root, "", "reply"],
                    ["p", agent]
                ]
            },
            {
                "id": "2".repeat(64),
                "pubkey": agent,
                "created_at": now_unix_ms() / 1000,
                "tags": [
                    ["e", other_root, "", "root"],
                    ["e", other_root, "", "reply"]
                ]
            },
            {
                // unrelated
                "id": "3".repeat(64),
                "pubkey": "c".repeat(64),
                "created_at": now_unix_ms() / 1000,
                "tags": [["e", "f".repeat(64), "", "root"]]
            }
        ]);
        let mut active = ActiveThreads::default();
        let n = active.absorb_events_json(&events, &agent);
        assert!(n >= 2);
        assert!(active.is_active(&root));
        assert!(active.is_active(&other_root));
        assert!(!active.is_active(&"f".repeat(64)));
    }
}
