//! Durable scheduled-message queue for the Buzz CLI.
//!
//! `buzz messages send --scheduled-at <ISO8601>` writes a pending delivery to a
//! JSON store instead of publishing immediately. `buzz messages scheduled run`
//! (optionally `--watch`) delivers due entries by re-running the normal send
//! path; `buzz messages scheduled list` / `cancel` inspect and mutate the
//! pending queue.
//!
//! The store lives at `<platform-data-dir>/xyz.block.buzz.app/scheduled/
//! scheduled-messages.json` — the same app-data root the desktop app uses (see
//! `channel_templates.rs`), so a future desktop "Schedule for later" composer
//! can share the queue. Writes are atomic (temp file + rename) and the store is
//! read/written in full, which keeps the shape simple for a single-user local
//! queue.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

/// Tauri bundle identifier for the production desktop app. `dirs::data_dir()`
/// joined with this segment matches `app.path().app_data_dir()` exactly
/// (Tauri resolves app-data as the platform data dir plus the identifier).
const PROD_BUNDLE_IDENTIFIER: &str = "xyz.block.buzz.app";

/// A pending (not-yet-delivered) message in the local scheduling queue.
///
/// Field-for-field this mirrors the flags accepted by
/// `buzz messages send`, minus file attachments (scheduling uploads is not
/// supported in the first cut). Delivered entries are removed from the store;
/// `scheduled_at` is a Unix timestamp in seconds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduledMessage {
    /// Stable local id (UUID) used to list and cancel the delivery.
    pub id: String,
    /// Channel UUID the message will be delivered to.
    pub channel_id: String,
    /// Message content (markdown, @mentions) exactly as passed to `send`.
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

/// Parse an ISO8601 / RFC 3339 timestamp into Unix seconds.
///
/// Rejects anything that is not parseable or is already in the past, so a
/// mistyped `--scheduled-at` fails at enqueue time instead of silently
/// delivering the message immediately.
pub fn parse_scheduled_at(input: &str) -> Result<i64, CliError> {
    let dt = chrono::DateTime::parse_from_rfc3339(input).map_err(|e| {
        CliError::Usage(format!(
            "--scheduled-at must be an ISO8601 / RFC 3339 timestamp (e.g. 2026-08-09T09:00:00Z): {e}"
        ))
    })?;
    let ts = dt.timestamp();
    if ts <= chrono::Utc::now().timestamp() {
        return Err(CliError::Usage(format!(
            "--scheduled-at must be in the future (got {input})"
        )));
    }
    Ok(ts)
}

/// Resolve the scheduled-messages store path.
///
/// `override_path` (from `--queue-file`) always wins — useful for tests and
/// ad-hoc debugging. Otherwise defaults to the prod bundle's app-data dir:
/// `<platform-data-dir>/xyz.block.buzz.app/scheduled/scheduled-messages.json`.
pub fn resolve_queue_path(override_path: Option<&str>) -> Result<PathBuf, CliError> {
    if let Some(p) = override_path {
        return Ok(PathBuf::from(p));
    }
    let data_dir = dirs::data_dir().ok_or_else(|| {
        CliError::Other("could not resolve platform app-data directory".to_string())
    })?;
    Ok(data_dir
        .join(PROD_BUNDLE_IDENTIFIER)
        .join("scheduled")
        .join("scheduled-messages.json"))
}

/// Load the pending queue from `path`. A missing or empty store is an empty
/// queue (an interrupted atomic write can leave a zero-byte file behind).
pub fn load_queue(path: &Path) -> Result<Vec<ScheduledMessage>, CliError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)
        .map_err(|e| CliError::Other(format!("failed to read {}: {e}", path.display())))?;
    if content.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&content)
        .map_err(|e| CliError::Other(format!("failed to parse {}: {e}", path.display())))
}

/// Persist `queue` to `path` atomically (temp file + rename).
///
/// Creates the parent directory when missing. Corrupt or unreadable existing
/// stores are surfaced as errors rather than silently overwritten.
fn save_queue(path: &Path, queue: &[ScheduledMessage]) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| CliError::Other(format!("failed to create {}: {e}", parent.display())))?;
    }
    let json = serde_json::to_string_pretty(queue)
        .map_err(|e| CliError::Other(format!("failed to serialize queue: {e}")))?;
    let tmp = path.with_extension("json.tmp");
    let mut f = fs::File::create(&tmp)
        .map_err(|e| CliError::Other(format!("failed to write {}: {e}", tmp.display())))?;
    f.write_all(json.as_bytes())
        .map_err(|e| CliError::Other(format!("failed to write {}: {e}", tmp.display())))?;
    f.sync_all()
        .map_err(|e| CliError::Other(format!("failed to sync {}: {e}", tmp.display())))?;
    fs::rename(&tmp, path)
        .map_err(|e| CliError::Other(format!("failed to replace {}: {e}", path.display())))?;
    Ok(())
}

/// Append `msg` to the queue at `path`.
pub fn enqueue(path: &Path, msg: ScheduledMessage) -> Result<(), CliError> {
    let mut queue = load_queue(path)?;
    queue.push(msg);
    save_queue(path, &queue)
}

/// Remove the entry with `id` from the queue, returning it if present.
pub fn cancel_by_id(path: &Path, id: &str) -> Result<Option<ScheduledMessage>, CliError> {
    let mut queue = load_queue(path)?;
    let mut removed = None;
    queue.retain(|m| {
        if m.id == id {
            removed = Some(m.clone());
            false
        } else {
            true
        }
    });
    save_queue(path, &queue)?;
    Ok(removed)
}

/// Split the queue into (due, pending) at `now` and persist the pending half.
///
/// Due entries are removed from the store and returned so the caller can
/// attempt delivery without racing the watcher on the same record. Failed
/// deliveries may be re-enqueued by the caller.
pub fn take_due(path: &Path, now: i64) -> Result<Vec<ScheduledMessage>, CliError> {
    let queue = load_queue(path)?;
    let mut due = Vec::new();
    let mut pending = Vec::new();
    for msg in queue {
        if msg.scheduled_at <= now {
            due.push(msg);
        } else {
            pending.push(msg);
        }
    }
    save_queue(path, &pending)?;
    Ok(due)
}

/// Earliest scheduled timestamp still pending, if any.
pub fn next_due(path: &Path) -> Result<Option<i64>, CliError> {
    Ok(load_queue(path)?.into_iter().map(|m| m.scheduled_at).min())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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

    fn write_store(json: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("tempfile");
        f.write_all(json.as_bytes()).expect("write");
        f
    }

    #[test]
    fn resolve_queue_path_honors_override() {
        let path = resolve_queue_path(Some("/tmp/custom.json")).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/custom.json"));
    }

    #[test]
    fn resolve_queue_path_defaults_to_prod_bundle() {
        let path = resolve_queue_path(None).unwrap();
        assert!(path.ends_with("xyz.block.buzz.app/scheduled/scheduled-messages.json"));
    }

    #[test]
    fn parse_scheduled_at_accepts_rfc3339_future() {
        let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let ts = parse_scheduled_at(&future).expect("future timestamp parses");
        assert!(ts > chrono::Utc::now().timestamp());
    }

    #[test]
    fn parse_scheduled_at_rejects_past() {
        let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let err = parse_scheduled_at(&past).unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
        assert!(err.to_string().contains("future"));
    }

    #[test]
    fn parse_scheduled_at_rejects_garbage() {
        let err = parse_scheduled_at("not-a-timestamp").unwrap_err();
        assert!(matches!(err, CliError::Usage(_)));
    }

    #[test]
    fn load_queue_missing_store_is_empty() {
        let queue = load_queue(Path::new("/nonexistent/scheduled-messages.json")).unwrap();
        assert!(queue.is_empty());
    }

    #[test]
    fn enqueue_and_cancel_round_trip() {
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        let path = f.path();
        enqueue(path, sample("id-1", 2000)).expect("enqueue");
        enqueue(path, sample("id-2", 3000)).expect("enqueue");
        let queue = load_queue(path).expect("load");
        assert_eq!(queue.len(), 2);

        let removed = cancel_by_id(path, "id-1").expect("cancel");
        assert_eq!(removed.map(|m| m.id).as_deref(), Some("id-1"));
        let queue = load_queue(path).expect("load");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue[0].id, "id-2");
    }

    #[test]
    fn cancel_unknown_id_is_none() {
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        enqueue(f.path(), sample("id-1", 2000)).expect("enqueue");
        let removed = cancel_by_id(f.path(), "missing").expect("cancel");
        assert!(removed.is_none());
        assert_eq!(load_queue(f.path()).unwrap().len(), 1);
    }

    #[test]
    fn take_due_removes_due_and_keeps_pending() {
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        let path = f.path();
        enqueue(path, sample("due-1", 1000)).expect("enqueue");
        enqueue(path, sample("due-2", 1000)).expect("enqueue");
        enqueue(path, sample("future", 9999)).expect("enqueue");

        let due = take_due(path, 1000).expect("take_due");
        assert_eq!(due.len(), 2);
        assert!(due.iter().all(|m| m.id.starts_with("due-")));
        let remaining = load_queue(path).expect("load");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "future");
    }

    #[test]
    fn next_due_reports_earliest_pending() {
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        let path = f.path();
        enqueue(path, sample("late", 5000)).expect("enqueue");
        enqueue(path, sample("early", 2000)).expect("enqueue");
        assert_eq!(next_due(path).unwrap(), Some(2000));
        assert_eq!(next_due(Path::new("/nonexistent/q.json")).unwrap(), None);
    }

    #[test]
    fn corrupted_store_surfaces_as_error() {
        let f = write_store("not json");
        let err = load_queue(f.path()).unwrap_err();
        assert!(err.to_string().contains("failed to parse"));
    }

    #[test]
    fn save_queue_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a/b/c/scheduled-messages.json");
        enqueue(&path, sample("id-1", 2000)).expect("enqueue");
        assert_eq!(load_queue(&path).unwrap().len(), 1);
    }
}
