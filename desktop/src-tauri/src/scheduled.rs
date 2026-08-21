//! Durable scheduled-message queue for the desktop app.
//!
//! "Schedule for Later" messages are stored in a local JSON file at
//! `<app-data-dir>/scheduled/scheduled-messages.json` — the same store the
//! Buzz CLI writes with `buzz messages send --scheduled-at ...`
//! (`<platform-data-dir>/xyz.block.buzz.app/scheduled/scheduled-messages.json`,
//! which is exactly the production app-data dir). The desktop app owns the
//! file while it runs (the composer enqueues, the delivery loop delivers);
//! the CLI can list/cancel the same pending queue with
//! `buzz messages scheduled list` / `cancel`.
//!
//! Writes are atomic (temp file + rename) and the store is read/written in
//! full, mirroring the CLI's store semantics so both tools stay interoperable.

use std::{fs, io::Write, path::Path};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// A pending (not-yet-delivered) message in the local scheduling queue.
///
/// Field-for-field compatible with the CLI's `ScheduledMessage`
/// (`crates/buzz-cli/src/schedule.rs`) so the desktop app and CLI share one
/// store. `scheduled_at` is a Unix timestamp in seconds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduledMessage {
    /// Stable local id (UUID) used to list and cancel the delivery.
    pub id: String,
    /// Channel UUID the message will be delivered to.
    pub channel_id: String,
    /// Message content (markdown, @mentions) exactly as composed.
    pub content: String,
    /// Nostr event kind (defaults to the channel default on delivery).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<u16>,
    /// Parent event id to reply to (threads the delivered message).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    /// Whether to also publish to the Nostr network on delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<bool>,
    /// Explicit mention pubkeys (hex or npub) to tag on delivery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    /// Unix timestamp (seconds) at which the message becomes deliverable.
    pub scheduled_at: i64,
    /// Unix timestamp (seconds) when the delivery was scheduled.
    pub created_at: i64,
}

/// Payload accepted by `scheduled_enqueue` from the composer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleMessageRequest {
    /// Channel UUID the message will be delivered to.
    pub channel_id: String,
    /// Message content (markdown, @mentions).
    pub content: String,
    /// Parent event id to reply to (threads the delivered message).
    pub reply_to: Option<String>,
    /// Explicit mention pubkeys (hex or npub) to tag on delivery.
    #[serde(default)]
    pub mentions: Vec<String>,
    /// ISO8601 / RFC 3339 timestamp for future delivery.
    pub scheduled_at: String,
}

fn scheduled_store_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("app data dir: {error}"))?
        .join("scheduled");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create scheduled dir: {error}"))?;
    Ok(dir.join("scheduled-messages.json"))
}

/// Load the pending queue from `path`. A missing or empty store is an empty
/// queue (an interrupted atomic write can leave a zero-byte file behind).
/// Corrupt stores surface as errors rather than being wiped.
fn load_queue_from_path(path: &Path) -> Result<Vec<ScheduledMessage>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse scheduled store: {error}"))
}

/// Persist `queue` to `path` atomically (temp file + rename). Creates the
/// parent directory when missing.
fn save_queue_to_path(path: &Path, queue: &[ScheduledMessage]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(queue)
        .map_err(|error| format!("failed to serialize scheduled store: {error}"))?;
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp)
        .map_err(|error| format!("failed to write {}: {error}", tmp.display()))?;
    file.write_all(&json)
        .map_err(|error| format!("failed to write {}: {error}", tmp.display()))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", tmp.display()))?;
    fs::rename(&tmp, path)
        .map_err(|error| format!("failed to replace {}: {error}", path.display()))?;
    Ok(())
}

/// Load the pending queue from the app's store.
pub fn load_queue(app: &AppHandle) -> Result<Vec<ScheduledMessage>, String> {
    load_queue_from_path(&scheduled_store_path(app)?)
}

fn save_queue(app: &AppHandle, queue: &[ScheduledMessage]) -> Result<(), String> {
    save_queue_to_path(&scheduled_store_path(app)?, queue)
}

/// Append `msg` to the pending queue.
pub fn enqueue(app: &AppHandle, msg: ScheduledMessage) -> Result<(), String> {
    let mut queue = load_queue(app)?;
    queue.push(msg);
    save_queue(app, &queue)
}

/// Re-append an entry the delivery loop already took but failed to deliver.
///
/// Unlike `enqueue`, the entry is persisted verbatim — including its (now
/// past) `scheduled_at` — so the next sweep picks it up again and retries.
pub fn reenqueue(app: &AppHandle, msg: ScheduledMessage) -> Result<(), String> {
    enqueue(app, msg)
}

/// Remove the entry with `id`, returning it if present.
pub fn cancel_by_id(app: &AppHandle, id: &str) -> Result<Option<ScheduledMessage>, String> {
    let mut queue = load_queue(app)?;
    let mut removed = None;
    queue.retain(|msg| {
        if msg.id == id {
            removed = Some(msg.clone());
            false
        } else {
            true
        }
    });
    save_queue(app, &queue)?;
    Ok(removed)
}

/// Split the queue into (due, pending) at `now` and persist the pending half.
///
/// Due entries are removed from the store and returned so the delivery loop
/// can attempt delivery without racing the composer on the same record.
/// Failed deliveries may be re-enqueued by the caller.
pub fn take_due(app: &AppHandle, now: i64) -> Result<Vec<ScheduledMessage>, String> {
    let queue = load_queue(app)?;
    let mut due = Vec::new();
    let mut pending = Vec::new();
    for msg in queue {
        if msg.scheduled_at <= now {
            due.push(msg);
        } else {
            pending.push(msg);
        }
    }
    save_queue(app, &pending)?;
    Ok(due)
}

/// Earliest scheduled timestamp still pending, if any.
pub fn next_due(app: &AppHandle) -> Result<Option<i64>, String> {
    Ok(load_queue(app)?.into_iter().map(|msg| msg.scheduled_at).min())
}

/// Parse an ISO8601 / RFC 3339 timestamp into Unix seconds.
///
/// Rejects anything that is not parseable or is already in the past, so a
/// mistyped schedule fails at enqueue time instead of delivering immediately.
pub fn parse_scheduled_at(input: &str) -> Result<i64, String> {
    let timestamp = chrono::DateTime::parse_from_rfc3339(input)
        .map_err(|error| format!("invalid delivery time {input:?}: {error}"))?
        .timestamp();
    if timestamp <= chrono::Utc::now().timestamp() {
        return Err("delivery time must be in the future".into());
    }
    Ok(timestamp)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    fn sample(id: &str, scheduled_at: i64) -> ScheduledMessage {
        ScheduledMessage {
            id: id.to_string(),
            channel_id: "3f4c2d10-1b9e-4e1a-9f6e-6c6f3f0b0f0f".to_string(),
            content: "hello".to_string(),
            kind: None,
            reply_to: None,
            broadcast: Some(false),
            mentions: Vec::new(),
            scheduled_at,
            created_at: 1000,
        }
    }

    #[test]
    fn parse_scheduled_at_accepts_rfc3339_future() {
        let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let timestamp = parse_scheduled_at(&future).expect("future timestamp parses");
        assert!(timestamp > chrono::Utc::now().timestamp());
    }

    #[test]
    fn parse_scheduled_at_rejects_past() {
        let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        assert!(parse_scheduled_at(&past).is_err());
    }

    #[test]
    fn parse_scheduled_at_rejects_garbage() {
        assert!(parse_scheduled_at("not-a-timestamp").is_err());
    }

    #[test]
    fn load_queue_missing_store_is_empty() {
        let queue = load_queue_from_path(Path::new("/nonexistent/scheduled-messages.json")).unwrap();
        assert!(queue.is_empty());
    }

    #[test]
    fn enqueue_and_cancel_round_trip() {
        let file = NamedTempFile::new().expect("tempfile");
        let path = file.path();
        enqueue_at(path, sample("id-1", 2000)).expect("enqueue");
        enqueue_at(path, sample("id-2", 3000)).expect("enqueue");
        let queue = load_queue_from_path(path).expect("load");
        assert_eq!(queue.len(), 2);

        let removed = cancel_at(path, "id-1").expect("cancel");
        assert_eq!(removed.map(|msg| msg.id).as_deref(), Some("id-1"));
        let queue = load_queue_from_path(path).expect("load");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].id, "id-2");
    }

    #[test]
    fn cancel_unknown_id_is_none() {
        let file = NamedTempFile::new().expect("tempfile");
        enqueue_at(file.path(), sample("id-1", 2000)).expect("enqueue");
        let removed = cancel_at(file.path(), "missing").expect("cancel");
        assert!(removed.is_none());
        assert_eq!(load_queue_from_path(file.path()).unwrap().len(), 1);
    }

    #[test]
    fn take_due_removes_due_and_keeps_pending() {
        let file = NamedTempFile::new().expect("tempfile");
        let path = file.path();
        enqueue_at(path, sample("due-1", 1000)).expect("enqueue");
        enqueue_at(path, sample("due-2", 1000)).expect("enqueue");
        enqueue_at(path, sample("future", 9999)).expect("enqueue");

        let due = take_due_at(path, 1000).expect("take_due");
        assert_eq!(due.len(), 2);
        assert!(due.iter().all(|msg| msg.id.starts_with("due-")));
        let remaining = load_queue_from_path(path).expect("load");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "future");
    }

    #[test]
    fn next_due_reports_earliest_pending() {
        let file = NamedTempFile::new().expect("tempfile");
        let path = file.path();
        enqueue_at(path, sample("late", 5000)).expect("enqueue");
        enqueue_at(path, sample("early", 2000)).expect("enqueue");
        let next = load_queue_from_path(path)
            .expect("load")
            .into_iter()
            .map(|msg| msg.scheduled_at)
            .min();
        assert_eq!(next, Some(2000));
    }

    #[test]
    fn corrupted_store_surfaces_as_error() {
        let file = NamedTempFile::new().expect("tempfile");
        let mut handle = std::fs::File::create(file.path()).expect("create");
        handle.write_all(b"not json").expect("write");
        let err = load_queue_from_path(file.path()).unwrap_err();
        assert!(err.contains("failed to parse"));
    }

    #[test]
    fn save_queue_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a/b/c/scheduled-messages.json");
        enqueue_at(&path, sample("id-1", 2000)).expect("enqueue");
        assert_eq!(load_queue_from_path(&path).unwrap().len(), 1);
    }

    #[test]
    fn serialization_matches_cli_shape() {
        let msg = sample("id-1", 2000);
        let json = serde_json::to_string_pretty(&msg).unwrap();
        let parsed: ScheduledMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
        // Optional fields are omitted so the CLI's serde defaults still apply.
        assert!(!json.contains("\"kind\""));
        assert!(!json.contains("\"mentions\""));
    }

    fn enqueue_at(path: &Path, msg: ScheduledMessage) -> Result<(), String> {
        let mut queue = load_queue_from_path(path)?;
        queue.push(msg);
        save_queue_to_path(path, &queue)
    }

    fn cancel_at(path: &Path, id: &str) -> Result<Option<ScheduledMessage>, String> {
        let mut queue = load_queue_from_path(path)?;
        let mut removed = None;
        queue.retain(|msg| {
            if msg.id == id {
                removed = Some(msg.clone());
                false
            } else {
                true
            }
        });
        save_queue_to_path(path, &queue)?;
        Ok(removed)
    }

    fn take_due_at(path: &Path, now: i64) -> Result<Vec<ScheduledMessage>, String> {
        let queue = load_queue_from_path(path)?;
        let mut due = Vec::new();
        let mut pending = Vec::new();
        for msg in queue {
            if msg.scheduled_at <= now {
                due.push(msg);
            } else {
                pending.push(msg);
            }
        }
        save_queue_to_path(path, &pending)?;
        Ok(due)
    }
}