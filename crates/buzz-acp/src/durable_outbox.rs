//! Crash-durable signed notice and `/new` intent storage.
//!
//! Two alternating, generation-stamped snapshots avoid in-place replacement:
//! a crash while writing the next slot always leaves the prior generation
//! intact, including on Windows where replacing an open destination is not
//! uniformly atomic. An OS file lock prevents two harnesses for one agent and
//! relay from racing the snapshots.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use nostr::Event;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const SCHEMA_VERSION: u32 = 2;
const MAX_SNAPSHOT_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const MAX_PENDING_LIFECYCLE: usize = 256;
pub(crate) const MAX_PENDING_RESETS: usize = 256;
const MAX_DELIVERED_KEYS: usize = 1_024;
const MAX_CONSUMED_RESET_RECEIPTS: usize = 1_024;
const MAX_EVENT_CONTENT_BYTES: usize = 16 * 1024;
const SLOT_A: &str = "signed-outbox-a.json";
const SLOT_B: &str = "signed-outbox-b.json";
const LOCK_FILE: &str = "signed-outbox.lock";
const INITIALIZED_MARKER: &str = ".signed-outbox-initialized-v1";
const INITIALIZED_MARKER_TEMP: &str = ".signed-outbox-initialized-v1.tmp";
const INITIALIZED_MARKER_CONTENT: &[u8] = b"buzz-acp-signed-outbox-v1\n";

#[derive(Debug, Error)]
pub(crate) enum DurableOutboxError {
    #[error("durable outbox I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("durable outbox JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("durable outbox is already locked by another harness: {0}")]
    Locked(PathBuf),
    #[error("durable outbox is invalid: {0}")]
    Invalid(String),
    #[error("durable outbox capacity reached: {0}")]
    Capacity(&'static str),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DurableLifecycleEnvelope {
    pub(crate) dedupe_key: Option<String>,
    pub(crate) event: Event,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DurableResetPhase {
    Prepared,
    Committed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DurableResetRecord {
    pub(crate) channel_id: Uuid,
    pub(crate) root_event_id: String,
    pub(crate) conversation_id: String,
    pub(crate) reset_token: String,
    pub(crate) event: Event,
    pub(crate) phase: DurableResetPhase,
}

impl DurableResetRecord {
    pub(crate) fn identity(&self) -> (Uuid, String) {
        (self.channel_id, self.root_event_id.clone())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DurableResetReceipt {
    pub(crate) channel_id: Uuid,
    pub(crate) root_event_id: String,
    pub(crate) reset_token: String,
    pub(crate) event: Event,
}

impl From<DurableResetRecord> for DurableResetReceipt {
    fn from(record: DurableResetRecord) -> Self {
        Self {
            channel_id: record.channel_id,
            root_event_id: record.root_event_id,
            reset_token: record.reset_token,
            event: record.event,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DurableState {
    schema_version: u32,
    generation: u64,
    pending_lifecycle: Vec<DurableLifecycleEnvelope>,
    delivered_keys: Vec<String>,
    pending_resets: Vec<DurableResetRecord>,
    #[serde(default)]
    consumed_reset_receipts: Vec<DurableResetReceipt>,
}

impl Default for DurableState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            generation: 0,
            pending_lifecycle: Vec::new(),
            delivered_keys: Vec::new(),
            pending_resets: Vec::new(),
            consumed_reset_receipts: Vec::new(),
        }
    }
}

struct Inner {
    state_dir: PathBuf,
    expected_pubkey: String,
    state: Mutex<DurableState>,
    _lock: File,
}

#[derive(Clone)]
pub(crate) struct DurableOutbox {
    inner: Arc<Inner>,
}

impl DurableOutbox {
    pub(crate) fn open(
        state_dir: &Path,
        expected_pubkey: &str,
    ) -> Result<Self, DurableOutboxError> {
        if !state_dir.is_absolute() {
            return Err(DurableOutboxError::Invalid(format!(
                "state directory is not absolute: {}",
                state_dir.display()
            )));
        }
        std::fs::create_dir_all(state_dir)?;
        set_private_dir_permissions(state_dir)?;

        let lock_path = state_dir.join(LOCK_FILE);
        let lock = private_open(&lock_path, false)?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| DurableOutboxError::Locked(lock_path))?;

        let initialized = read_initialized_marker(&state_dir.join(INITIALIZED_MARKER))?;
        let slot_a = read_slot(&state_dir.join(SLOT_A), expected_pubkey);
        let slot_b = read_slot(&state_dir.join(SLOT_B), expected_pubkey);
        let mut valid = Vec::new();
        let mut errors = Vec::new();
        for result in [slot_a, slot_b] {
            match result {
                Ok(Some(state)) => valid.push(state),
                Ok(None) => {}
                Err(error) => errors.push(error),
            }
        }
        let had_valid_snapshot = !valid.is_empty();
        let state = valid
            .into_iter()
            .max_by_key(|state| state.generation)
            .unwrap_or_default();
        if !had_valid_snapshot {
            if !errors.is_empty() {
                return Err(errors.remove(0));
            }
            if initialized {
                return Err(DurableOutboxError::Invalid(
                    "initialized durable outbox has no valid snapshot".into(),
                ));
            }
        }
        for error in errors {
            tracing::warn!(%error, "ignored invalid older durable outbox slot");
        }
        // Establish a synced generation-0 predecessor before accepting the
        // first mutation. Without this bootstrap slot, a crash during the very
        // first generation write would leave no valid snapshot to fall back to.
        if !had_valid_snapshot {
            write_slot(state_dir, &state)?;
        }
        if !initialized {
            write_initialized_marker(state_dir)?;
        }

        Ok(Self {
            inner: Arc::new(Inner {
                state_dir: state_dir.to_path_buf(),
                expected_pubkey: expected_pubkey.to_owned(),
                state: Mutex::new(state),
                _lock: lock,
            }),
        })
    }

    pub(crate) fn pending_lifecycle(
        &self,
    ) -> Result<Vec<DurableLifecycleEnvelope>, DurableOutboxError> {
        Ok(self
            .inner
            .state
            .lock()
            .map_err(|_| DurableOutboxError::Invalid("state mutex was poisoned".into()))?
            .pending_lifecycle
            .clone())
    }

    pub(crate) fn pending_resets(&self) -> Result<Vec<DurableResetRecord>, DurableOutboxError> {
        Ok(self
            .inner
            .state
            .lock()
            .map_err(|_| DurableOutboxError::Invalid("state mutex was poisoned".into()))?
            .pending_resets
            .clone())
    }

    pub(crate) fn delivered_keys(&self) -> Result<Vec<String>, DurableOutboxError> {
        Ok(self
            .inner
            .state
            .lock()
            .map_err(|_| DurableOutboxError::Invalid("state mutex was poisoned".into()))?
            .delivered_keys
            .clone())
    }

    pub(crate) fn consumed_reset(
        &self,
        identity: &(Uuid, String),
        reset_token: &str,
    ) -> Result<Option<DurableResetReceipt>, DurableOutboxError> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| DurableOutboxError::Invalid("state mutex was poisoned".into()))?;
        Ok(state
            .consumed_reset_receipts
            .iter()
            .find(|receipt| {
                receipt.channel_id == identity.0
                    && receipt.root_event_id == identity.1
                    && receipt.reset_token == reset_token
            })
            .cloned())
    }

    /// Persist a signed lifecycle event before it can be acknowledged to the
    /// adapter or handed to the asynchronous relay worker. Returns `false` for
    /// an already-pending or already-delivered stable dedupe key.
    pub(crate) fn enqueue_lifecycle(
        &self,
        envelope: DurableLifecycleEnvelope,
    ) -> Result<bool, DurableOutboxError> {
        self.update(|state| {
            if let Some(key) = envelope.dedupe_key.as_deref() {
                if state.delivered_keys.iter().any(|item| item == key)
                    || state
                        .pending_lifecycle
                        .iter()
                        .any(|item| item.dedupe_key.as_deref() == Some(key))
                {
                    return Ok((false, false));
                }
            }
            if state
                .pending_lifecycle
                .iter()
                .any(|item| item.event.id == envelope.event.id)
            {
                return Ok((false, false));
            }
            if state.pending_lifecycle.len() >= MAX_PENDING_LIFECYCLE {
                return Err(DurableOutboxError::Capacity("lifecycle notices"));
            }
            state.pending_lifecycle.push(envelope);
            Ok((true, true))
        })
    }

    pub(crate) fn mark_lifecycle_delivered(
        &self,
        event_id: &str,
    ) -> Result<bool, DurableOutboxError> {
        self.update(|state| {
            let Some(index) = state
                .pending_lifecycle
                .iter()
                .position(|item| item.event.id.to_hex() == event_id)
            else {
                return Ok((false, false));
            };
            let envelope = state.pending_lifecycle.remove(index);
            if let Some(key) = envelope.dedupe_key {
                state.delivered_keys.retain(|item| item != &key);
                state.delivered_keys.push(key);
                if state.delivered_keys.len() > MAX_DELIVERED_KEYS {
                    let excess = state.delivered_keys.len() - MAX_DELIVERED_KEYS;
                    state.delivered_keys.drain(..excess);
                }
            }
            Ok((true, true))
        })
    }

    pub(crate) fn prepare_reset(
        &self,
        record: DurableResetRecord,
    ) -> Result<(), DurableOutboxError> {
        self.update(|state| {
            if let Some(existing) = state.pending_resets.iter_mut().find(|item| {
                item.channel_id == record.channel_id && item.root_event_id == record.root_event_id
            }) {
                if existing.reset_token == record.reset_token
                    && existing.event.id == record.event.id
                {
                    return Ok(((), false));
                }
                *existing = record;
                return Ok(((), true));
            }
            if state.pending_resets.len() >= MAX_PENDING_RESETS {
                return Err(DurableOutboxError::Capacity("reset acknowledgements"));
            }
            state.pending_resets.push(record);
            Ok(((), true))
        })
    }

    pub(crate) fn mark_reset_committed(
        &self,
        identity: &(Uuid, String),
        event_id: &str,
    ) -> Result<bool, DurableOutboxError> {
        self.update(|state| {
            let Some(record) = state.pending_resets.iter_mut().find(|item| {
                item.channel_id == identity.0
                    && item.root_event_id == identity.1
                    && item.event.id.to_hex() == event_id
            }) else {
                return Ok((false, false));
            };
            if record.phase == DurableResetPhase::Committed {
                return Ok((true, false));
            }
            record.phase = DurableResetPhase::Committed;
            Ok((true, true))
        })
    }

    pub(crate) fn discard_reset(
        &self,
        identity: &(Uuid, String),
        event_id: &str,
    ) -> Result<bool, DurableOutboxError> {
        self.update(|state| {
            let before = state.pending_resets.len();
            state.pending_resets.retain(|item| {
                !(item.channel_id == identity.0
                    && item.root_event_id == identity.1
                    && item.event.id.to_hex() == event_id)
            });
            let removed = state.pending_resets.len() != before;
            Ok((removed, removed))
        })
    }

    /// Atomically retire an accepted reset ACK and retain the signed command
    /// token as a bounded durable replay receipt. A replay can then resend the
    /// same acknowledgement without advancing the conversation generation.
    pub(crate) fn complete_reset(
        &self,
        identity: &(Uuid, String),
        event_id: &str,
    ) -> Result<bool, DurableOutboxError> {
        self.update(|state| {
            let Some(index) = state.pending_resets.iter().position(|item| {
                item.channel_id == identity.0
                    && item.root_event_id == identity.1
                    && item.event.id.to_hex() == event_id
                    && item.phase == DurableResetPhase::Committed
            }) else {
                return Ok((false, false));
            };
            let record = state.pending_resets.remove(index);
            state.consumed_reset_receipts.retain(|receipt| {
                !(receipt.channel_id == record.channel_id
                    && receipt.root_event_id == record.root_event_id
                    && receipt.reset_token == record.reset_token)
            });
            state.consumed_reset_receipts.push(record.into());
            if state.consumed_reset_receipts.len() > MAX_CONSUMED_RESET_RECEIPTS {
                let excess = state.consumed_reset_receipts.len() - MAX_CONSUMED_RESET_RECEIPTS;
                state.consumed_reset_receipts.drain(..excess);
            }
            Ok((true, true))
        })
    }

    fn update<T>(
        &self,
        mutate: impl FnOnce(&mut DurableState) -> Result<(T, bool), DurableOutboxError>,
    ) -> Result<T, DurableOutboxError> {
        let mut guard = self
            .inner
            .state
            .lock()
            .map_err(|_| DurableOutboxError::Invalid("state mutex was poisoned".into()))?;
        let mut candidate = guard.clone();
        let (result, changed) = mutate(&mut candidate)?;
        if !changed {
            return Ok(result);
        }
        candidate.generation = candidate
            .generation
            .checked_add(1)
            .ok_or_else(|| DurableOutboxError::Invalid("generation exhausted".into()))?;
        validate_state(&candidate, &self.inner.expected_pubkey)?;
        write_slot(&self.inner.state_dir, &candidate)?;
        *guard = candidate;
        Ok(result)
    }
}

fn validate_state(state: &DurableState, expected_pubkey: &str) -> Result<(), DurableOutboxError> {
    if state.schema_version != SCHEMA_VERSION {
        return Err(DurableOutboxError::Invalid(format!(
            "unsupported schema version {}",
            state.schema_version
        )));
    }
    if state.pending_lifecycle.len() > MAX_PENDING_LIFECYCLE
        || state.pending_resets.len() > MAX_PENDING_RESETS
        || state.delivered_keys.len() > MAX_DELIVERED_KEYS
        || state.consumed_reset_receipts.len() > MAX_CONSUMED_RESET_RECEIPTS
    {
        return Err(DurableOutboxError::Invalid(
            "entry count exceeds cap".into(),
        ));
    }
    let mut event_ids = HashSet::new();
    for envelope in &state.pending_lifecycle {
        validate_event(&envelope.event, expected_pubkey)?;
        if envelope
            .dedupe_key
            .as_ref()
            .is_some_and(|key| key.is_empty() || key.len() > 1_024)
        {
            return Err(DurableOutboxError::Invalid(
                "invalid lifecycle dedupe key".into(),
            ));
        }
        if !event_ids.insert(envelope.event.id.to_hex()) {
            return Err(DurableOutboxError::Invalid(
                "duplicate pending event".into(),
            ));
        }
    }
    if state
        .delivered_keys
        .iter()
        .any(|key| key.is_empty() || key.len() > 1_024)
    {
        return Err(DurableOutboxError::Invalid(
            "invalid delivered dedupe key".into(),
        ));
    }
    for reset in &state.pending_resets {
        validate_event(&reset.event, expected_pubkey)?;
        if reset.root_event_id.is_empty()
            || reset.root_event_id.len() > 512
            || reset.conversation_id.is_empty()
            || reset.conversation_id.len() > 1_024
            || reset.reset_token.is_empty()
            || reset.reset_token.len() > 256
        {
            return Err(DurableOutboxError::Invalid(
                "invalid reset record fields".into(),
            ));
        }
        if !event_ids.insert(reset.event.id.to_hex()) {
            return Err(DurableOutboxError::Invalid(
                "duplicate pending event".into(),
            ));
        }
    }
    for receipt in &state.consumed_reset_receipts {
        validate_event(&receipt.event, expected_pubkey)?;
        if receipt.root_event_id.is_empty()
            || receipt.root_event_id.len() > 512
            || receipt.reset_token.is_empty()
            || receipt.reset_token.len() > 256
        {
            return Err(DurableOutboxError::Invalid(
                "invalid consumed reset receipt".into(),
            ));
        }
        if !event_ids.insert(receipt.event.id.to_hex()) {
            return Err(DurableOutboxError::Invalid(
                "duplicate pending/receipt event".into(),
            ));
        }
    }
    Ok(())
}

fn validate_event(event: &Event, expected_pubkey: &str) -> Result<(), DurableOutboxError> {
    buzz_core::verify_event(event)
        .map_err(|error| DurableOutboxError::Invalid(format!("invalid signed event: {error}")))?;
    if event.pubkey.to_hex() != expected_pubkey {
        return Err(DurableOutboxError::Invalid(
            "signed event belongs to another agent".into(),
        ));
    }
    if event.kind.as_u16() as u32 != buzz_core::kind::KIND_STREAM_MESSAGE
        || event.content.len() > MAX_EVENT_CONTENT_BYTES
    {
        return Err(DurableOutboxError::Invalid(
            "signed event has invalid kind or content size".into(),
        ));
    }
    Ok(())
}

fn read_initialized_marker(path: &Path) -> Result<bool, DurableOutboxError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if metadata.len() != INITIALIZED_MARKER_CONTENT.len() as u64 {
        return Err(DurableOutboxError::Invalid(
            "durable outbox initialized marker has invalid size".into(),
        ));
    }
    let mut content = Vec::with_capacity(INITIALIZED_MARKER_CONTENT.len());
    file.read_to_end(&mut content)?;
    if content != INITIALIZED_MARKER_CONTENT {
        return Err(DurableOutboxError::Invalid(
            "durable outbox initialized marker has invalid content".into(),
        ));
    }
    Ok(true)
}

fn write_initialized_marker(state_dir: &Path) -> Result<(), DurableOutboxError> {
    let path = state_dir.join(INITIALIZED_MARKER);
    let temp_path = state_dir.join(INITIALIZED_MARKER_TEMP);
    match std::fs::remove_file(&temp_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut file = private_open(&temp_path, true)?;
    file.write_all(INITIALIZED_MARKER_CONTENT)?;
    file.sync_all()?;
    std::fs::rename(&temp_path, &path)?;
    sync_directory(state_dir)?;
    Ok(())
}

fn read_slot(
    path: &Path,
    expected_pubkey: &str,
) -> Result<Option<DurableState>, DurableOutboxError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if metadata.len() == 0 || metadata.len() > MAX_SNAPSHOT_BYTES {
        return Err(DurableOutboxError::Invalid(format!(
            "snapshot {} has invalid size {}",
            path.display(),
            metadata.len()
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_SNAPSHOT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err(DurableOutboxError::Invalid(
            "snapshot exceeds byte cap".into(),
        ));
    }
    let mut state: DurableState = serde_json::from_slice(&bytes)?;
    // Schema v2 only adds the bounded consumed-reset receipt collection, which
    // is serde-defaulted above. Upgrade v1 snapshots in memory and let the next
    // mutation rewrite them; rejecting the previous production format here
    // would strand otherwise valid pending lifecycle/reset records.
    if state.schema_version == 1 {
        state.schema_version = SCHEMA_VERSION;
    }
    validate_state(&state, expected_pubkey)?;
    Ok(Some(state))
}

fn write_slot(state_dir: &Path, state: &DurableState) -> Result<(), DurableOutboxError> {
    let bytes = serde_json::to_vec(state)?;
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err(DurableOutboxError::Capacity("snapshot bytes"));
    }
    let slot_name = if state.generation.is_multiple_of(2) {
        SLOT_A
    } else {
        SLOT_B
    };
    let path = state_dir.join(slot_name);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut file = private_open(&path, true)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    sync_directory(state_dir)?;
    Ok(())
}

fn private_open(path: &Path, create_new: bool) -> Result<File, std::io::Error> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), std::io::Error> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), std::io::Error> {
    // Windows does not provide a portable directory fsync through std. Each
    // complete slot file is still flushed before it becomes the newest valid
    // generation, and the previous slot remains intact.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};

    fn signed_notice(keys: &Keys, content: &str) -> Event {
        EventBuilder::new(
            Kind::Custom(buzz_core::kind::KIND_STREAM_MESSAGE as u16),
            content,
        )
        .sign_with_keys(keys)
        .expect("sign notice")
    }

    fn reset(keys: &Keys, channel_id: Uuid) -> DurableResetRecord {
        DurableResetRecord {
            channel_id,
            root_event_id: "aa".repeat(32),
            conversation_id: format!("{channel_id}:thread:{}", "aa".repeat(32)),
            reset_token: "bb".repeat(32),
            event: signed_notice(keys, "reset ready"),
            phase: DurableResetPhase::Prepared,
        }
    }

    #[test]
    fn lifecycle_and_reset_survive_restart_and_delivered_key_dedupes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let channel_id = Uuid::new_v4();
        let lifecycle = DurableLifecycleEnvelope {
            dedupe_key: Some("compaction:one".into()),
            event: signed_notice(&keys, "compacted"),
        };
        let lifecycle_id = lifecycle.event.id.to_hex();
        let reset = reset(&keys, channel_id);
        let reset_id = reset.event.id.to_hex();
        let identity = reset.identity();

        {
            let outbox = DurableOutbox::open(dir.path(), &pubkey).expect("open");
            assert!(outbox
                .enqueue_lifecycle(lifecycle.clone())
                .expect("enqueue"));
            outbox.prepare_reset(reset).expect("prepare reset");
            assert!(outbox
                .mark_reset_committed(&identity, &reset_id)
                .expect("commit reset"));
        }

        let outbox = DurableOutbox::open(dir.path(), &pubkey).expect("reopen");
        assert_eq!(outbox.pending_lifecycle().expect("read lifecycle").len(), 1);
        let pending_resets = outbox.pending_resets().expect("read resets");
        assert_eq!(pending_resets.len(), 1);
        assert_eq!(pending_resets[0].phase, DurableResetPhase::Committed);
        assert!(outbox
            .mark_lifecycle_delivered(&lifecycle_id)
            .expect("mark delivered"));
        assert!(!outbox
            .enqueue_lifecycle(lifecycle)
            .expect("dedupe delivered"));
        let reset_token = pending_resets[0].reset_token.clone();
        assert!(outbox
            .complete_reset(&identity, &reset_id)
            .expect("complete reset"));
        let receipt = outbox
            .consumed_reset(&identity, &reset_token)
            .expect("read receipt")
            .expect("consumed reset receipt");
        assert_eq!(receipt.event.id.to_hex(), reset_id);
        assert!(outbox
            .pending_lifecycle()
            .expect("read lifecycle")
            .is_empty());
        assert!(outbox.pending_resets().expect("read resets").is_empty());

        drop(outbox);
        let reopened = DurableOutbox::open(dir.path(), &pubkey).expect("reopen receipt");
        assert_eq!(
            reopened
                .consumed_reset(&identity, &reset_token)
                .expect("read receipt")
                .expect("receipt survives restart")
                .event
                .id
                .to_hex(),
            reset_id
        );
    }

    #[test]
    fn schema_one_snapshot_migrates_without_losing_pending_records() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let envelope = DurableLifecycleEnvelope {
            dedupe_key: Some("legacy-one".into()),
            event: signed_notice(&keys, "legacy"),
        };
        {
            let outbox = DurableOutbox::open(dir.path(), &pubkey).expect("open");
            outbox.enqueue_lifecycle(envelope).expect("enqueue");
        }

        let slot = dir.path().join(SLOT_B);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&slot).expect("read slot")).expect("decode slot");
        value["schemaVersion"] = serde_json::json!(1);
        value
            .as_object_mut()
            .expect("state object")
            .remove("consumedResetReceipts");
        std::fs::write(&slot, serde_json::to_vec(&value).expect("encode legacy"))
            .expect("write legacy");

        let reopened = DurableOutbox::open(dir.path(), &pubkey).expect("migrate v1");
        assert_eq!(
            reopened.pending_lifecycle().expect("read lifecycle").len(),
            1
        );
    }

    #[test]
    fn corrupt_newest_slot_falls_back_to_previous_generation() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        {
            let outbox = DurableOutbox::open(dir.path(), &pubkey).expect("open");
            outbox
                .enqueue_lifecycle(DurableLifecycleEnvelope {
                    dedupe_key: Some("one".into()),
                    event: signed_notice(&keys, "one"),
                })
                .expect("first generation");
            outbox
                .enqueue_lifecycle(DurableLifecycleEnvelope {
                    dedupe_key: Some("two".into()),
                    event: signed_notice(&keys, "two"),
                })
                .expect("second generation");
        }
        std::fs::write(dir.path().join(SLOT_A), b"truncated").expect("corrupt newest even slot");

        let recovered = DurableOutbox::open(dir.path(), &pubkey).expect("recover older slot");
        let pending = recovered.pending_lifecycle().expect("read lifecycle");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].dedupe_key.as_deref(), Some("one"));
    }

    #[test]
    fn first_generation_has_a_synced_empty_predecessor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        {
            let outbox = DurableOutbox::open(dir.path(), &pubkey).expect("open");
            outbox
                .enqueue_lifecycle(DurableLifecycleEnvelope {
                    dedupe_key: Some("first".into()),
                    event: signed_notice(&keys, "first"),
                })
                .expect("first generation");
        }
        std::fs::write(dir.path().join(SLOT_B), b"torn-first-generation")
            .expect("corrupt first mutation");

        let recovered = DurableOutbox::open(dir.path(), &pubkey)
            .expect("generation zero must remain recoverable");
        assert!(recovered
            .pending_lifecycle()
            .expect("read lifecycle")
            .is_empty());
    }

    #[test]
    fn second_harness_for_same_state_directory_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let _first = DurableOutbox::open(dir.path(), &pubkey).expect("first lock");
        assert!(matches!(
            DurableOutbox::open(dir.path(), &pubkey),
            Err(DurableOutboxError::Locked(_))
        ));
    }

    #[test]
    fn initialized_marker_makes_missing_both_snapshots_fail_closed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        {
            let outbox = DurableOutbox::open(dir.path(), &pubkey).expect("initialize");
            outbox
                .enqueue_lifecycle(DurableLifecycleEnvelope {
                    dedupe_key: Some("material-state".into()),
                    event: signed_notice(&keys, "material state"),
                })
                .expect("write second slot");
        }
        assert!(dir.path().join(INITIALIZED_MARKER).exists());
        std::fs::remove_file(dir.path().join(SLOT_A)).expect("delete bootstrap slot");
        std::fs::remove_file(dir.path().join(SLOT_B)).expect("delete current slot");

        assert!(matches!(
            DurableOutbox::open(dir.path(), &pubkey),
            Err(DurableOutboxError::Invalid(message))
                if message.contains("no valid snapshot")
        ));
    }

    #[test]
    fn initialized_marker_recovers_a_torn_temp_file_without_touching_valid_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = DurableState::default();
        write_slot(dir.path(), &state).expect("write valid bootstrap snapshot");
        std::fs::write(dir.path().join(INITIALIZED_MARKER_TEMP), b"torn")
            .expect("write torn marker temp");

        write_initialized_marker(dir.path()).expect("replace torn marker temp atomically");
        assert!(
            read_initialized_marker(&dir.path().join(INITIALIZED_MARKER)).expect("read marker")
        );
        assert!(!dir.path().join(INITIALIZED_MARKER_TEMP).exists());
    }

    #[test]
    fn poisoned_state_cannot_masquerade_as_an_unconsumed_reset() {
        let dir = tempfile::tempdir().expect("tempdir");
        let keys = Keys::generate();
        let outbox = DurableOutbox::open(dir.path(), &keys.public_key().to_hex()).expect("open");
        let inner = Arc::clone(&outbox.inner);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = inner.state.lock().expect("lock before poison");
            panic!("intentional durable state poison");
        }));

        let identity = (Uuid::new_v4(), "root".to_owned());
        assert!(matches!(
            outbox.consumed_reset(&identity, "token"),
            Err(DurableOutboxError::Invalid(message)) if message.contains("poisoned")
        ));
        assert!(matches!(
            outbox.pending_lifecycle(),
            Err(DurableOutboxError::Invalid(message)) if message.contains("poisoned")
        ));
        assert!(matches!(
            outbox.pending_resets(),
            Err(DurableOutboxError::Invalid(message)) if message.contains("poisoned")
        ));
        assert!(matches!(
            outbox.delivered_keys(),
            Err(DurableOutboxError::Invalid(message)) if message.contains("poisoned")
        ));
    }
}
