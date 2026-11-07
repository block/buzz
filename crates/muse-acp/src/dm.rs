//! Direct Buzz DM handling for the Muse bridge.
//!
//! Buzz itself (buzz-acp) is the relay listener: it discovers DMs,
//! admits them under its author gate, and wakes an ACP agent. A DM in
//! Buzz is a channel whose metadata says `name: DM`, `t: dm` — it
//! carries no useful channel name, so a DM wake's session title is the
//! bare agent title, not `Agent · #channel`. This module is the
//! muse-acp side of that flow: given a DM event that Buzz delivered,
//! deduplicate it, route it to the correct Muse context, and produce
//! exactly one correlated reply.
//!
//! What this module is not: it does not speak Nostr, does not hold a
//! relay key, and does not poll a relay. The bridge's identity lives in
//! Buzz Desktop's agent config, not in muse-acp (a stdio ACP process),
//! so a relay-level DM listener inside muse-acp would need to export
//! that identity — the key-sharing path that was already declined.
//! Buzz remains the DM discoverer; this module is the dedup / routing /
//! correlation state machine for events Buzz delivers.
//!
//! Exactly-once, across restarts:
//!
//! * `ProcessedEvents` is a file-backed set of event IDs. It is loaded
//!   on construction and saved after every successful completion.
//! * `should_process` is false for an already-processed event ID, so a
//!   redelivery (or a restart that replays the same DM) produces no
//!   second reply.
//! * `complete` marks the event only after a reply is produced. A
//!   failure before completion leaves the event unmarked, so a restart
//!   retries it instead of dropping it — failure handling is retry,
//!   not silent loss.
//! * A reply is correlated by construction: `reply_to_event_id` is the
//!   original DM event, and `thread_root` is the DM's top-level event
//!   (the event itself for a top-level DM, the root for a reply in a
//!   DM thread, following Buzz's DM rule that top-level DM messages
//!   share one conversation).
//!
//! Routing: a DM has no channel name, so its Muse side chat is
//! deterministic from the DM conversation, not from a channel title:
//! `DM: <sender> (buzz)` when a sender label is known, otherwise
//! `DM <first-8-of-dm-channel-id> (buzz)`. Two different DM
//! conversations therefore never share a side chat, and the same DM
//! conversation always routes to the same one.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::BridgeError;

/// Environment variable naming the processed-DM-event file. When unset,
/// the listener keeps its set in memory only (a restart then replays
/// DMs); when set, it persists across restarts.
pub const DM_STATE_ENV: &str = "MUSE_ACP_DM_STATE_FILE";

/// A DM event delivered to the bridge by Buzz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmEvent {
    /// The Nostr event ID of this DM message (hex, 64 chars in
    /// production; the module does not enforce length, so tests can use
    /// short IDs).
    pub event_id: String,
    /// The DM conversation's channel UUID, as Buzz reports it.
    pub dm_channel_id: String,
    /// The sender's pubkey or display label, when known.
    pub sender: String,
    /// The DM thread's top-level event ID, if this message is a reply
    /// in a DM thread. `None` means this message is itself top-level.
    pub thread_root: Option<String>,
    /// The message text.
    pub text: String,
}

impl DmEvent {
    /// The thread this reply belongs to: the explicit root, or this
    /// event itself for a top-level DM.
    pub fn thread_root_or_self(&self) -> String {
        self.thread_root
            .clone()
            .filter(|r| !r.trim().is_empty())
            .unwrap_or_else(|| self.event_id.clone())
    }
}

/// Deterministic Muse side-chat title for a DM conversation.
pub fn dm_side_chat_title(dm_channel_id: &str, sender: &str) -> String {
    if !sender.trim().is_empty() {
        return format!("DM: {} (buzz)", sender.trim());
    }
    let short: String = dm_channel_id.chars().take(8).collect();
    if short.is_empty() {
        "DM (buzz)".to_string()
    } else {
        format!("DM {short} (buzz)")
    }
}

/// Parse a DM event out of prompt text, when the wake carries DM framing.
///
/// Accepted lines (any order, first non-empty wins per field):
///
/// ```text
/// DM channel: <uuid>
/// Event: <event id>
/// Sender: <label>
/// Thread root: <event id>
/// ```
///
/// Returns `None` when there is no `DM channel:` line — a plain channel
/// wake is not a DM, and must not be routed as one. This framing is
/// what a future buzz-acp DM wake would need to emit; Buzz's current
/// DM framing (bare session title, DM context section) does not yet
/// carry these lines, so live DM routing via this parser is gated on
/// that framing landing in buzz-acp. The dedup/routing/correlation
/// logic itself is fully tested here, independent of that framing.
pub fn parse_dm_from_text(text: &str) -> Option<DmEvent> {
    let mut dm_channel_id = None;
    let mut event_id = None;
    let mut sender = String::new();
    let mut thread_root = None;
    for line in text.lines() {
        let trimmed = line.trim();
        let field = |prefix: &str| trimmed.strip_prefix(prefix).map(str::trim);
        if let Some(value) = field("DM channel:") {
            if !value.is_empty() {
                dm_channel_id = Some(value.to_string());
            }
        } else if let Some(value) = field("Event:") {
            if !value.is_empty() {
                event_id = Some(value.to_string());
            }
        } else if let Some(value) = field("Sender:") {
            if !value.is_empty() {
                sender = value.to_string();
            }
        } else if let Some(value) = field("Thread root:") {
            if !value.is_empty() {
                thread_root = Some(value.to_string());
            }
        }
    }
    let (dm_channel_id, event_id) = match (dm_channel_id, event_id) {
        (Some(c), Some(e)) => (c, e),
        _ => return None,
    };
    Some(DmEvent {
        event_id,
        dm_channel_id,
        sender,
        thread_root,
        text: text.to_string(),
    })
}

/// A file-backed set of already-processed DM event IDs.
#[derive(Debug, Default)]
pub struct ProcessedEvents {
    ids: HashSet<String>,
}

impl ProcessedEvents {
    /// An empty, in-memory set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from `path`: a JSON array of event IDs. Missing file is
    /// empty; malformed file is an error (the caller falls back to
    /// empty, so a corrupt state file does not strand DMs).
    pub fn load(path: &Path) -> Result<Self, BridgeError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => return Err(BridgeError::Io(e)),
        };
        if text.trim().is_empty() {
            return Ok(Self::new());
        }
        let value: Value = serde_json::from_str(&text)?;
        let mut set = Self::new();
        if let Some(arr) = value.as_array() {
            for item in arr {
                if let Some(id) = item.as_str() {
                    set.ids.insert(id.to_string());
                }
            }
        }
        Ok(set)
    }

    /// Load, or empty on any failure.
    pub fn load_or_empty(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    /// Save atomically (temp file + rename).
    pub fn save(&self, path: &Path) -> Result<(), BridgeError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut ids: Vec<&String> = self.ids.iter().collect();
        ids.sort();
        let text = serde_json::to_string_pretty(&ids)?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// True when this event ID has already been completed.
    pub fn is_processed(&self, event_id: &str) -> bool {
        self.ids.contains(event_id)
    }

    /// Mark an event as processed. Returns true when it was newly
    /// marked, false when it was already processed.
    pub fn mark(&mut self, event_id: &str) -> bool {
        self.ids.insert(event_id.to_string())
    }

    /// Number of processed events (for tests / diagnostics).
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when no events have been processed yet.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// Exactly one correlated reply for a DM event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelatedReply {
    /// The DM event this reply answers.
    pub reply_to_event_id: String,
    /// The DM thread root this reply belongs to.
    pub thread_root: String,
    /// The DM conversation's channel UUID.
    pub dm_channel_id: String,
    /// The Muse side-chat title this DM routes to.
    pub side_chat_title: String,
    /// The reply text.
    pub reply_text: String,
}

/// The DM listener state machine: dedup + routing + correlation.
///
/// The caller drives it: `should_process` before running a turn,
/// `complete` after the turn's reply is ready (which marks the event),
/// and nothing on failure (so a restart retries). This ordering is
/// what makes "exactly one reply" hold: a duplicate is never run, and
/// a failed run is never marked as done.
#[derive(Debug)]
pub struct DmListener {
    processed: ProcessedEvents,
}

impl DmListener {
    /// A listener with an in-memory processed set.
    pub fn new() -> Self {
        Self {
            processed: ProcessedEvents::new(),
        }
    }

    /// A listener whose processed set is loaded from `path`.
    pub fn from_file(path: &Path) -> Self {
        Self {
            processed: ProcessedEvents::load_or_empty(path),
        }
    }

    /// True when this DM event has not been completed yet.
    pub fn should_process(&self, event: &DmEvent) -> bool {
        !self.processed.is_processed(&event.event_id)
    }

    /// Produce the correlated reply for a DM event, marking it as
    /// processed. Returns `None` (and marks nothing new) when the
    /// event was already processed — a duplicate must never produce a
    /// second reply.
    pub fn complete(&mut self, event: &DmEvent, reply_text: &str) -> Option<CorrelatedReply> {
        if self.processed.is_processed(&event.event_id) {
            return None;
        }
        self.processed.mark(&event.event_id);
        Some(CorrelatedReply {
            reply_to_event_id: event.event_id.clone(),
            thread_root: event.thread_root_or_self(),
            dm_channel_id: event.dm_channel_id.clone(),
            side_chat_title: dm_side_chat_title(&event.dm_channel_id, &event.sender),
            reply_text: reply_text.to_string(),
        })
    }

    /// Persist the processed set, if a state path is given. Failures
    /// are returned to the caller: losing the processed set on a
    /// restart would re-deliver DMs, so a save failure is not silent.
    pub fn save(&self, path: &Path) -> Result<(), BridgeError> {
        self.processed.save(path)
    }

    /// How many DM events have been completed.
    pub fn processed_count(&self) -> usize {
        self.processed.len()
    }
}

impl Default for DmListener {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn event(id: &str) -> DmEvent {
        DmEvent {
            event_id: id.to_string(),
            dm_channel_id: "3043c06d-6cf1-42f8-b1b0-cb2d61711ea1".to_string(),
            sender: "Seth".to_string(),
            thread_root: None,
            text: "hello".to_string(),
        }
    }

    #[test]
    fn dm_side_chat_title_uses_sender_then_channel_prefix() {
        assert_eq!(
            dm_side_chat_title("3043c06d-6cf1-42f8-b1b0-cb2d61711ea1", "Seth"),
            "DM: Seth (buzz)"
        );
        assert_eq!(
            dm_side_chat_title("3043c06d-6cf1-42f8-b1b0-cb2d61711ea1", ""),
            "DM 3043c06d (buzz)"
        );
        assert_eq!(dm_side_chat_title("", ""), "DM (buzz)");
    }

    #[test]
    fn thread_root_defaults_to_event_itself() {
        let top = event("aaa");
        assert_eq!(top.thread_root_or_self(), "aaa");
        let mut reply = event("bbb");
        reply.thread_root = Some("aaa".to_string());
        assert_eq!(reply.thread_root_or_self(), "aaa");
    }

    #[test]
    fn duplicate_dm_produces_no_second_reply() {
        let mut listener = DmListener::new();
        let ev = event("aaa");
        assert!(listener.should_process(&ev));
        let first = listener.complete(&ev, "reply one").unwrap();
        assert_eq!(first.reply_to_event_id, "aaa");
        assert_eq!(first.thread_root, "aaa");
        assert_eq!(first.side_chat_title, "DM: Seth (buzz)");
        // Duplicate: no second reply, and should_process is now false.
        assert!(!listener.should_process(&ev));
        assert!(listener.complete(&ev, "reply two").is_none());
        assert_eq!(listener.processed_count(), 1);
    }

    #[test]
    fn failure_before_complete_leaves_event_retryable() {
        let mut listener = DmListener::new();
        let ev = event("aaa");
        // A turn that fails calls neither complete nor save: the event
        // is still unprocessed, so a restart retries it.
        assert!(listener.should_process(&ev));
        assert_eq!(listener.processed_count(), 0);
        // A later successful turn completes it exactly once.
        assert!(listener.complete(&ev, "ok").is_some());
        assert!(!listener.should_process(&ev));
    }

    #[test]
    fn processed_set_survives_restart_via_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dm-state.json");
        let mut listener = DmListener::from_file(&path);
        listener.complete(&event("aaa"), "reply").unwrap();
        listener.save(&path).unwrap();
        // A fresh listener loading the same file must not reprocess.
        let reloaded = DmListener::from_file(&path);
        assert!(!reloaded.should_process(&event("aaa")));
        assert!(reloaded.should_process(&event("bbb")));
    }

    #[test]
    fn malformed_dm_state_file_falls_back_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dm-state.json");
        fs::write(&path, "not json").unwrap();
        assert!(ProcessedEvents::load(&path).is_err());
        let listener = DmListener::from_file(&path);
        assert!(listener.should_process(&event("aaa")));
    }

    #[test]
    fn parse_dm_from_text_requires_dm_channel_and_event() {
        let text = "DM channel: 3043c06d-6cf1-42f8-b1b0-cb2d61711ea1\nEvent: aaa\nSender: Seth\nThread root: root1\nMessage: hi";
        let ev = parse_dm_from_text(text).unwrap();
        assert_eq!(ev.dm_channel_id, "3043c06d-6cf1-42f8-b1b0-cb2d61711ea1");
        assert_eq!(ev.event_id, "aaa");
        assert_eq!(ev.sender, "Seth");
        assert_eq!(ev.thread_root, Some("root1".to_string()));
        // Plain channel text is not a DM.
        assert!(parse_dm_from_text("Channel: MASKY-CMO\nMessage: hi").is_none());
        assert!(parse_dm_from_text("DM channel: only-channel\nMessage: hi").is_none());
    }

    #[test]
    fn different_dm_conversations_route_to_different_side_chats() {
        let a = dm_side_chat_title("11111111-aaaa", "Seth");
        let b = dm_side_chat_title("22222222-bbbb", "");
        assert_ne!(a, b);
        // Same conversation, same title, every time.
        assert_eq!(
            dm_side_chat_title("11111111-aaaa", "Seth"),
            dm_side_chat_title("11111111-aaaa", "Seth")
        );
    }
}
