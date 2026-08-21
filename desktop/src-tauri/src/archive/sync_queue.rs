//! Crash-resilient write-ahead queue for native archive sync.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nostr::{Event, JsonUtil};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager};

use super::FLUSH_BATCH_SIZE;
use crate::archive::{ArchiveCandidate, MatchedScope, ScopeType};

/// A queue is deliberately finite. Relay backpressure stops acceptance once
/// this file cannot grow safely; silently dropping the tail is never allowed.
pub(super) const MAX_DURABLE_QUEUE_ENTRIES: usize = 2_048;
const MAX_DURABLE_QUEUE_BYTES: usize = 8 * 1024 * 1024;
const MAX_ACKED_DEDUPE_KEYS: usize = 2_048;
const DURABLE_QUEUE_VERSION: u8 = 1;

pub(super) const ARCHIVE_RETRY_ATTEMPTS: usize = 4;
#[cfg(not(test))]
const ARCHIVE_RETRY_BASE_DELAY: Duration = Duration::from_millis(100);
#[cfg(test)]
const ARCHIVE_RETRY_BASE_DELAY: Duration = Duration::from_millis(1);
#[cfg(not(test))]
pub(super) const ARCHIVE_RETRY_MAX_DELAY: Duration = Duration::from_millis(800);
#[cfg(test)]
pub(super) const ARCHIVE_RETRY_MAX_DELAY: Duration = Duration::from_millis(8);
#[cfg(not(test))]
pub(super) const ARCHIVE_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
pub(super) const ARCHIVE_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(50);
// ── Durable write-ahead queue ────────────────────────────────────────────────

/// Queue representation intentionally contains only the public signed Nostr
/// envelope and its asserted archive scope. Private keys, auth tokens, and
/// decrypted observer payloads never enter this file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DurableCandidate {
    pub(super) event_id: String,
    raw_event_json: String,
    scope_type: ScopeType,
    scope_value: String,
}

impl DurableCandidate {
    pub(super) fn from_event(event: &Event, scope: &MatchedScope) -> Self {
        Self {
            event_id: event.id.to_hex(),
            raw_event_json: event.as_json(),
            scope_type: scope.scope_type.clone(),
            scope_value: scope.scope_value.clone(),
        }
    }

    fn dedupe_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.event_id,
            self.scope_type.as_str(),
            self.scope_value
        )
    }

    fn archive_candidate(&self) -> ArchiveCandidate {
        ArchiveCandidate {
            raw_event_json: self.raw_event_json.clone(),
            matched_scope: MatchedScope {
                scope_type: self.scope_type.clone(),
                scope_value: self.scope_value.clone(),
            },
        }
    }

    fn validate(&self) -> Result<(), String> {
        let event = Event::from_json(&self.raw_event_json)
            .map_err(|error| format!("queued event is not valid Nostr JSON: {error}"))?;
        event
            .verify()
            .map_err(|error| format!("queued event signature is invalid: {error}"))?;
        if event.id.to_hex() != self.event_id {
            return Err("queued event id does not match its signed envelope".into());
        }
        if self.scope_value.is_empty() {
            return Err("queued archive scope is empty".into());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct DurableQueueFile {
    version: u8,
    entries: Vec<DurableCandidate>,
    #[serde(default)]
    acked_keys: Vec<String>,
}

#[derive(Debug)]
pub(super) struct DurableQueue {
    path: PathBuf,
    entries: Vec<DurableCandidate>,
    /// A bounded durable tombstone window suppresses relay duplicates after an
    /// acknowledged item has left the pending queue. A crash after archive
    /// acknowledgment but before this tombstone commits can still replay once;
    /// the archive database's signed-id/scope uniqueness is the final guard.
    acked_keys: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EnqueueResult {
    Accepted,
    Duplicate,
}

impl DurableQueue {
    pub(super) fn open(path: PathBuf) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "archive sync queue path has no parent".to_string())?;
        ensure_owner_only_dir(parent)?;

        let blocked_path = blocked_diagnostic_path(&path);
        if blocked_path.exists() {
            let diagnostic = fs::read_to_string(&blocked_path)
                .unwrap_or_else(|_| "archive sync queue is blocked".to_string());
            return Err(format!(
                "archive sync durable queue is fail-closed: {}",
                diagnostic.trim()
            ));
        }

        if !path.exists() {
            return Ok(Self {
                path,
                entries: Vec::new(),
                acked_keys: Vec::new(),
            });
        }

        let bytes = fs::read(&path)
            .map_err(|error| format!("read durable archive queue {}: {error}", path.display()))?;
        if bytes.len() > MAX_DURABLE_QUEUE_BYTES {
            return Err(quarantine_corrupt_queue(
                &path,
                format!(
                    "queue file is {} bytes; maximum is {MAX_DURABLE_QUEUE_BYTES}",
                    bytes.len()
                ),
            ));
        }
        let persisted: DurableQueueFile = serde_json::from_slice(&bytes).map_err(|error| {
            quarantine_corrupt_queue(&path, format!("invalid queue JSON: {error}"))
        })?;
        if persisted.version != DURABLE_QUEUE_VERSION {
            return Err(quarantine_corrupt_queue(
                &path,
                format!(
                    "unsupported queue version {}; expected {DURABLE_QUEUE_VERSION}",
                    persisted.version
                ),
            ));
        }
        if persisted.entries.len() > MAX_DURABLE_QUEUE_ENTRIES {
            return Err(quarantine_corrupt_queue(
                &path,
                format!(
                    "queue contains {} entries; maximum is {MAX_DURABLE_QUEUE_ENTRIES}",
                    persisted.entries.len()
                ),
            ));
        }
        for entry in &persisted.entries {
            if let Err(error) = entry.validate() {
                return Err(quarantine_corrupt_queue(&path, error));
            }
        }

        let mut changed = persisted.acked_keys.len() > MAX_ACKED_DEDUPE_KEYS;
        let mut acked_seen = HashSet::new();
        let mut acked_keys = Vec::new();
        for key in persisted
            .acked_keys
            .into_iter()
            .rev()
            .take(MAX_ACKED_DEDUPE_KEYS)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if acked_seen.insert(key.clone()) {
                acked_keys.push(key);
            } else {
                changed = true;
            }
        }
        let acked: HashSet<_> = acked_keys.iter().cloned().collect();
        let mut pending_seen = HashSet::new();
        let mut entries = Vec::new();
        for entry in persisted.entries {
            let key = entry.dedupe_key();
            if acked.contains(&key) || !pending_seen.insert(key) {
                changed = true;
                continue;
            }
            entries.push(entry);
        }

        let queue = Self {
            path,
            entries,
            acked_keys,
        };
        if changed {
            queue.persist()?;
        }
        Ok(queue)
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn enqueue(&mut self, candidate: DurableCandidate) -> Result<EnqueueResult, String> {
        candidate.validate()?;
        let key = candidate.dedupe_key();
        if self.acked_keys.iter().any(|existing| existing == &key)
            || self
                .entries
                .iter()
                .any(|existing| existing.dedupe_key() == key)
        {
            return Ok(EnqueueResult::Duplicate);
        }
        if self.entries.len() >= MAX_DURABLE_QUEUE_ENTRIES {
            return Err(format!(
                "archive sync durable queue limit reached ({MAX_DURABLE_QUEUE_ENTRIES} entries); event was not accepted"
            ));
        }

        self.entries.push(candidate);
        if let Err(error) = self.persist() {
            self.entries.pop();
            return Err(format!(
                "archive sync durable queue write failed; event was not accepted: {error}"
            ));
        }
        Ok(EnqueueResult::Accepted)
    }

    pub(super) fn head(&self) -> Vec<ArchiveCandidate> {
        self.entries
            .iter()
            .take(FLUSH_BATCH_SIZE)
            .map(DurableCandidate::archive_candidate)
            .collect()
    }

    pub(super) fn head_len(&self) -> usize {
        self.entries.len().min(FLUSH_BATCH_SIZE)
    }

    /// Atomically records both removal and the recent-ack tombstones. Memory is
    /// changed only after the replacement file commits, so a failed write
    /// leaves the durable head intact for another attempt or restart.
    pub(super) fn acknowledge_head(&mut self, count: usize) -> Result<(), String> {
        if count == 0 || count > self.entries.len() {
            return Err("invalid durable archive acknowledgment count".into());
        }
        let mut next_entries = self.entries[count..].to_vec();
        let mut next_acked = self.acked_keys.clone();
        next_acked.extend(
            self.entries[..count]
                .iter()
                .map(DurableCandidate::dedupe_key),
        );
        if next_acked.len() > MAX_ACKED_DEDUPE_KEYS {
            next_acked.drain(..next_acked.len() - MAX_ACKED_DEDUPE_KEYS);
        }
        persist_queue_file(&self.path, &next_entries, &next_acked)?;
        self.entries = std::mem::take(&mut next_entries);
        self.acked_keys = next_acked;
        Ok(())
    }

    fn persist(&self) -> Result<(), String> {
        persist_queue_file(&self.path, &self.entries, &self.acked_keys)
    }

    #[cfg(test)]
    pub(super) fn fill_to_capacity_for_test(&mut self, candidate: DurableCandidate) {
        self.entries = vec![candidate; MAX_DURABLE_QUEUE_ENTRIES];
    }
}

fn persist_queue_file(
    path: &Path,
    entries: &[DurableCandidate],
    acked_keys: &[String],
) -> Result<(), String> {
    let payload = serde_json::to_vec(&DurableQueueFile {
        version: DURABLE_QUEUE_VERSION,
        entries: entries.to_vec(),
        acked_keys: acked_keys.to_vec(),
    })
    .map_err(|error| format!("serialize durable archive queue: {error}"))?;
    if payload.len() > MAX_DURABLE_QUEUE_BYTES {
        return Err(format!(
            "durable archive queue would be {} bytes; maximum is {MAX_DURABLE_QUEUE_BYTES}",
            payload.len()
        ));
    }
    crate::managed_agents::storage::atomic_write_json_restricted(path, &payload)
}

fn ensure_owner_only_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("create archive sync queue dir {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!(
                "set archive sync queue dir {} permissions: {error}",
                path.display()
            )
        })?;
    }
    Ok(())
}

pub(super) fn blocked_diagnostic_path(path: &Path) -> PathBuf {
    path.with_extension("blocked")
}

fn quarantine_corrupt_queue(path: &Path, reason: String) -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive-sync-pending.json");
    let quarantine = path.with_file_name(format!("{file_name}.corrupt-{stamp}"));
    let move_result = fs::rename(path, &quarantine);
    let diagnostic = match move_result {
        Ok(()) => format!(
            "corrupt pending state quarantined at {}: {reason}",
            quarantine.display()
        ),
        Err(error) => format!(
            "corrupt pending state could not be quarantined from {} ({error}): {reason}",
            path.display()
        ),
    };
    if let Ok(payload) = serde_json::to_vec(&json!({
        "blocked": true,
        "diagnostic": diagnostic,
    })) {
        let _ = crate::managed_agents::storage::atomic_write_json_restricted(
            &blocked_diagnostic_path(path),
            &payload,
        );
    }
    diagnostic
}

pub(super) fn retry_delay(failed_attempt: usize) -> Duration {
    let multiplier = 1u32 << failed_attempt.min(16);
    ARCHIVE_RETRY_BASE_DELAY
        .saturating_mul(multiplier)
        .min(ARCHIVE_RETRY_MAX_DELAY)
}

#[derive(Clone, Copy)]
pub(super) struct AcknowledgedHead {
    pub(super) count: usize,
    pub(super) persisted_agent_metrics: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FlushOutcome {
    Empty,
    Committed,
    Retained,
    Cancelled,
}

fn stable_scope_hash(value: &str) -> u64 {
    // FNV-1a is intentionally fixed rather than DefaultHasher, whose output is
    // not a persistence contract across Rust releases.
    value
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

pub(super) fn durable_queue_path(
    app: &AppHandle,
    owner_pubkey: &str,
    relay_url: &str,
) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("resolve app data dir for archive sync queue: {error}"))?
        .join("archive")
        .join("sync-pending");
    Ok(base.join(format!(
        "{owner_pubkey}-{:016x}.json",
        stable_scope_hash(relay_url)
    )))
}
