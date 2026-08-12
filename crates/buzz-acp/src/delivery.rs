use crate::queue::{self, FlushBatch, PromptProfileLookup};
use crate::relay::RestClient;
use nostr::{Event, EventId, Filter, Keys};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::sync::Mutex;

const RECORD_VERSION: u8 = 1;

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error("outbox I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("outbox JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid outbox record: {0}")]
    InvalidRecord(String),
    #[error("reply build failed: {0}")]
    Build(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeliveryRecord {
    version: u8,
    delivery_key: String,
    event: Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordLocation {
    Pending,
    Delivered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryState {
    Confirmed { event_id: EventId },
    Pending { event_id: EventId },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RecoveryReport {
    pub confirmed: usize,
    pub pending: usize,
    pub quarantined: usize,
}

/// Durable, idempotent final-reply outbox.
///
/// A batch-derived delivery key owns one signed event for its lifetime. Pending
/// records are retried after ambiguous acknowledgements or restart; confirmed
/// records are retained as receipts so the same triggering batch can never sign
/// and publish a second event.
pub struct NativeDelivery {
    root: PathBuf,
    operation_lock: Mutex<()>,
}

impl NativeDelivery {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, DeliveryError> {
        let root = root.as_ref().to_path_buf();
        for path in [
            root.join("pending"),
            root.join("delivered"),
            root.join("corrupt"),
        ] {
            std::fs::create_dir_all(path)?;
        }
        Ok(Self {
            root,
            operation_lock: Mutex::new(()),
        })
    }

    pub(crate) fn pending_path(&self, key: &str) -> PathBuf {
        self.root.join("pending").join(format!("{key}.json"))
    }

    pub(crate) fn delivered_path(&self, key: &str) -> PathBuf {
        self.root.join("delivered").join(format!("{key}.json"))
    }

    pub(crate) fn corrupt_dir(&self) -> PathBuf {
        self.root.join("corrupt")
    }

    fn validate_key(key: &str) -> Result<(), DeliveryError> {
        if key.is_empty()
            || key.len() > 128
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(DeliveryError::InvalidRecord(
                "delivery key must contain only ASCII letters, digits, '-' or '_'".into(),
            ));
        }
        Ok(())
    }

    fn prepare<F>(
        &self,
        key: &str,
        build_event: F,
    ) -> Result<(DeliveryRecord, RecordLocation), DeliveryError>
    where
        F: FnOnce() -> Result<Event, DeliveryError>,
    {
        Self::validate_key(key)?;

        let delivered = self.delivered_path(key);
        if delivered.is_file() {
            return Ok((
                load_record_for_key(&delivered, key)?,
                RecordLocation::Delivered,
            ));
        }

        let pending = self.pending_path(key);
        if pending.is_file() {
            return Ok((load_record_for_key(&pending, key)?, RecordLocation::Pending));
        }

        let event = build_event()?;
        event.verify().map_err(|error| {
            DeliveryError::InvalidRecord(format!("new signed event failed verification: {error}"))
        })?;
        let record = DeliveryRecord {
            version: RECORD_VERSION,
            delivery_key: key.to_string(),
            event,
        };
        persist_record_atomically(&pending, &record)?;
        Ok((record, RecordLocation::Pending))
    }

    pub async fn deliver_with<B, S, SF, Q, QF>(
        &self,
        key: &str,
        build_event: B,
        submit: S,
        contains: Q,
    ) -> Result<DeliveryState, DeliveryError>
    where
        B: FnOnce() -> Result<Event, DeliveryError>,
        S: Fn(Event) -> SF,
        SF: Future<Output = Result<(), String>>,
        Q: Fn(Event) -> QF,
        QF: Future<Output = Result<bool, String>>,
    {
        let _guard = self.operation_lock.lock().await;
        let (record, location) = self.prepare(key, build_event)?;
        self.reconcile_record(key, record, location, &submit, &contains)
            .await
    }

    async fn reconcile_record<S, SF, Q, QF>(
        &self,
        key: &str,
        record: DeliveryRecord,
        mut location: RecordLocation,
        submit: &S,
        contains: &Q,
    ) -> Result<DeliveryState, DeliveryError>
    where
        S: Fn(Event) -> SF,
        SF: Future<Output = Result<(), String>>,
        Q: Fn(Event) -> QF,
        QF: Future<Output = Result<bool, String>>,
    {
        let event_id = record.event.id;

        if location == RecordLocation::Delivered {
            match contains(record.event.clone()).await {
                Ok(true) => return Ok(DeliveryState::Confirmed { event_id }),
                Ok(false) | Err(_) => {
                    self.move_record(key, RecordLocation::Delivered, RecordLocation::Pending)?;
                    location = RecordLocation::Pending;
                }
            }
        }

        debug_assert_eq!(location, RecordLocation::Pending);
        if let Err(error) = submit(record.event.clone()).await {
            tracing::warn!(delivery_key = key, %event_id, "native delivery submit was ambiguous: {error}");
        }

        match contains(record.event.clone()).await {
            Ok(true) => {
                self.move_record(key, RecordLocation::Pending, RecordLocation::Delivered)?;
                Ok(DeliveryState::Confirmed { event_id })
            }
            Ok(false) => Ok(DeliveryState::Pending { event_id }),
            Err(error) => {
                tracing::warn!(delivery_key = key, %event_id, "native delivery read-back failed: {error}");
                Ok(DeliveryState::Pending { event_id })
            }
        }
    }

    fn move_record(
        &self,
        key: &str,
        from: RecordLocation,
        to: RecordLocation,
    ) -> Result<(), DeliveryError> {
        let source = match from {
            RecordLocation::Pending => self.pending_path(key),
            RecordLocation::Delivered => self.delivered_path(key),
        };
        let target = match to {
            RecordLocation::Pending => self.pending_path(key),
            RecordLocation::Delivered => self.delivered_path(key),
        };

        if !source.exists() {
            if target.exists() {
                load_record_for_key(&target, key)?;
                return Ok(());
            }
            return Err(DeliveryError::InvalidRecord(format!(
                "record for {key} disappeared before state transition"
            )));
        }

        if target.exists() {
            let source_record = load_record_for_key(&source, key)?;
            let target_record = load_record_for_key(&target, key)?;
            if source_record.event.id != target_record.event.id {
                return Err(DeliveryError::InvalidRecord(format!(
                    "conflicting records for delivery key {key}"
                )));
            }
            std::fs::remove_file(&source)?;
            sync_parent_directory(&source)?;
            return Ok(());
        }

        std::fs::rename(&source, &target)?;
        sync_parent_directory(&source)?;
        sync_parent_directory(&target)?;
        Ok(())
    }

    pub async fn recover_with<S, SF, Q, QF>(&self, submit: S, contains: Q) -> RecoveryReport
    where
        S: Fn(Event) -> SF,
        SF: Future<Output = Result<(), String>>,
        Q: Fn(Event) -> QF,
        QF: Future<Output = Result<bool, String>>,
    {
        let _guard = self.operation_lock.lock().await;
        let mut report = RecoveryReport::default();
        let mut paths = match std::fs::read_dir(self.root.join("pending")) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
                .collect::<Vec<_>>(),
            Err(error) => {
                tracing::error!("native delivery recovery could not read outbox: {error}");
                return report;
            }
        };
        paths.sort();

        for path in paths {
            let key = match path.file_stem().and_then(|stem| stem.to_str()) {
                Some(key) => key.to_string(),
                None => {
                    if self.quarantine(&path).is_ok() {
                        report.quarantined += 1;
                    }
                    continue;
                }
            };
            let record = match load_record_for_key(&path, &key) {
                Ok(record) => record,
                Err(error) => {
                    tracing::error!(path = %path.display(), "quarantining invalid native delivery record: {error}");
                    if self.quarantine(&path).is_ok() {
                        report.quarantined += 1;
                    }
                    continue;
                }
            };

            match self
                .reconcile_record(&key, record, RecordLocation::Pending, &submit, &contains)
                .await
            {
                Ok(DeliveryState::Confirmed { .. }) => report.confirmed += 1,
                Ok(DeliveryState::Pending { .. }) => report.pending += 1,
                Err(error) => {
                    tracing::error!(
                        delivery_key = key,
                        "native delivery recovery failed: {error}"
                    );
                    report.pending += 1;
                }
            }
        }
        report
    }

    fn quarantine(&self, path: &Path) -> Result<(), DeliveryError> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("invalid-record");
        let target = self
            .corrupt_dir()
            .join(format!("{file_name}.{}.corrupt", uuid::Uuid::new_v4()));
        std::fs::rename(path, &target)?;
        sync_parent_directory(path)?;
        sync_parent_directory(&target)?;
        Ok(())
    }

    pub async fn deliver<B>(
        &self,
        key: &str,
        build_event: B,
        rest: &RestClient,
    ) -> Result<DeliveryState, DeliveryError>
    where
        B: FnOnce() -> Result<Event, DeliveryError>,
    {
        let submit_rest = rest.clone();
        let query_rest = rest.clone();
        self.deliver_with(
            key,
            build_event,
            move |event| {
                let rest = submit_rest.clone();
                async move {
                    rest.submit_event(&event)
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }
            },
            move |event| {
                let rest = query_rest.clone();
                async move { relay_contains_event(&rest, &event).await }
            },
        )
        .await
    }

    pub async fn recover(&self, rest: &RestClient) -> RecoveryReport {
        let submit_rest = rest.clone();
        let query_rest = rest.clone();
        self.recover_with(
            move |event| {
                let rest = submit_rest.clone();
                async move {
                    rest.submit_event(&event)
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }
            },
            move |event| {
                let rest = query_rest.clone();
                async move { relay_contains_event(&rest, &event).await }
            },
        )
        .await
    }
}

fn persist_record_atomically(path: &Path, record: &DeliveryRecord) -> Result<(), DeliveryError> {
    let parent = path.parent().ok_or_else(|| {
        DeliveryError::InvalidRecord(format!("outbox path has no parent: {}", path.display()))
    })?;
    let bytes = serde_json::to_vec(record)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("record.json");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));

    let write_result = (|| -> Result<(), DeliveryError> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);

        // A hard link is an atomic create-if-absent operation on both Unix and
        // Windows. Unlike `rename`, it cannot overwrite another harness's
        // record on Unix if twin processes race on the same delivery key.
        match std::fs::hard_link(&temporary, path) {
            Ok(()) => {
                sync_parent_directory(path)?;
                if let Err(error) = std::fs::remove_file(&temporary) {
                    tracing::warn!(path = %temporary.display(), "native delivery could not remove staged temporary file: {error}");
                } else {
                    sync_parent_directory(&temporary)?;
                }
                Ok(())
            }
            Err(_error) if path.exists() => {
                let existing = load_record(path)?;
                if existing.delivery_key == record.delivery_key
                    && existing.event.id == record.event.id
                {
                    std::fs::remove_file(&temporary)?;
                    Ok(())
                } else {
                    Err(DeliveryError::InvalidRecord(format!(
                        "outbox destination {} already contains a different event",
                        path.display()
                    )))
                }
            }
            Err(error) => Err(error.into()),
        }
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    write_result
}

#[cfg(unix)]
fn sync_parent_directory(path: &Path) -> Result<(), DeliveryError> {
    let parent = path.parent().ok_or_else(|| {
        DeliveryError::InvalidRecord(format!("outbox path has no parent: {}", path.display()))
    })?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_path: &Path) -> Result<(), DeliveryError> {
    Ok(())
}

fn load_record(path: &Path) -> Result<DeliveryRecord, DeliveryError> {
    let bytes = std::fs::read(path)?;
    let record: DeliveryRecord = serde_json::from_slice(&bytes)?;
    if record.version != RECORD_VERSION {
        return Err(DeliveryError::InvalidRecord(format!(
            "unsupported record version {}",
            record.version
        )));
    }
    record.event.verify().map_err(|error| {
        DeliveryError::InvalidRecord(format!("signed event verification failed: {error}"))
    })?;
    Ok(record)
}

fn load_record_for_key(path: &Path, expected_key: &str) -> Result<DeliveryRecord, DeliveryError> {
    let record = load_record(path)?;
    if record.delivery_key != expected_key {
        return Err(DeliveryError::InvalidRecord(format!(
            "record key {} does not match filename key {expected_key}",
            record.delivery_key
        )));
    }
    Ok(record)
}

async fn relay_contains_event(rest: &RestClient, expected: &Event) -> Result<bool, String> {
    let response = rest
        .query(&[Filter::new().id(expected.id).limit(1)])
        .await
        .map_err(|error| error.to_string())?;
    let Some(events) = response.as_array() else {
        return Err("relay query response was not an array".into());
    };

    Ok(events.iter().any(|raw| {
        serde_json::from_value::<Event>(raw.clone())
            .is_ok_and(|event| event.id == expected.id && event.verify().is_ok())
    }))
}

/// Derive a stable delivery identity from the publishing agent and immutable
/// triggering batch. Including the agent avoids collisions if two harnesses
/// are accidentally configured with the same outbox root.
pub fn delivery_key(agent_pubkey: &nostr::PublicKey, batch: &FlushBatch) -> String {
    let mut digest = Sha256::new();
    digest.update(b"buzz-acp-native-delivery-v1\0");
    digest.update(agent_pubkey.to_hex().as_bytes());
    digest.update(batch.channel_id.as_bytes());
    for event in &batch.cancelled_events {
        digest.update(b"\0cancelled\0");
        digest.update(event.event.id.to_hex().as_bytes());
    }
    for event in &batch.events {
        digest.update(b"\0event\0");
        digest.update(event.event.id.to_hex().as_bytes());
    }
    hex::encode(digest.finalize())
}

/// Build and sign the one native reply event for a completed batch.
pub fn build_reply_event(
    keys: &Keys,
    batch: &FlushBatch,
    content: &str,
    profile_lookup: Option<&PromptProfileLookup>,
) -> Result<Event, DeliveryError> {
    let triggering = batch
        .events
        .last()
        .ok_or_else(|| DeliveryError::Build("cannot build a reply for an empty batch".into()))?;
    let triggering_id = triggering.event.id.to_hex();
    let sender = triggering.event.pubkey.to_hex();
    let thread_tags = queue::parse_thread_tags(&triggering.event);
    let human_anchor =
        queue::resolve_reply_anchor(&sender, &thread_tags, &triggering_id, profile_lookup);

    let (root, parent) = if let Some(anchor) = human_anchor {
        let id = EventId::from_hex(&anchor)
            .map_err(|error| DeliveryError::Build(format!("invalid reply anchor: {error}")))?;
        (id, id)
    } else {
        let root = thread_tags
            .root_event_id
            .as_deref()
            .map(EventId::from_hex)
            .transpose()
            .map_err(|error| DeliveryError::Build(format!("invalid thread root: {error}")))?
            .unwrap_or(triggering.event.id);
        (root, triggering.event.id)
    };
    let thread_ref = buzz_sdk::ThreadRef {
        root_event_id: root,
        parent_event_id: parent,
    };
    let builder = buzz_sdk::build_message(
        batch.channel_id,
        content,
        Some(&thread_ref),
        &[sender.as_str()],
        false,
        &[],
    )
    .map_err(|error| DeliveryError::Build(error.to_string()))?;
    builder
        .sign_with_keys(keys)
        .map_err(|error| DeliveryError::Build(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{BatchEvent, FlushBatch, PromptProfile, PromptProfileLookup};
    use nostr::{Event, EventBuilder, Keys, Kind, Tag};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use uuid::Uuid;

    fn test_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("buzz-acp-delivery-test-{}", Uuid::new_v4()))
    }

    fn signed_event(keys: &Keys, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(9), content)
            .tags([])
            .sign_with_keys(keys)
            .expect("sign test event")
    }

    fn tagged_event(keys: &Keys, content: &str, tags: Vec<Vec<String>>) -> Event {
        let tags = tags
            .iter()
            .map(|tag| {
                Tag::parse(tag.iter().map(String::as_str).collect::<Vec<_>>())
                    .expect("parse test tag")
            })
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(9), content)
            .tags(tags)
            .sign_with_keys(keys)
            .expect("sign tagged test event")
    }

    fn batch(event: Event, channel_id: Uuid) -> FlushBatch {
        FlushBatch {
            channel_id,
            events: vec![BatchEvent {
                event,
                prompt_tag: "@mention".into(),
                received_at: Instant::now(),
            }],
            cancelled_events: Vec::new(),
            cancel_reason: None,
        }
    }

    fn tag_values(event: &Event, kind: &str, marker: Option<&str>) -> Vec<String> {
        event
            .tags
            .iter()
            .filter_map(|tag| {
                let parts = tag.as_slice();
                if parts.first().map(String::as_str) != Some(kind) {
                    return None;
                }
                if marker.is_some() && parts.get(3).map(String::as_str) != marker {
                    return None;
                }
                parts.get(1).cloned()
            })
            .collect()
    }

    #[tokio::test]
    async fn signed_event_is_durable_before_submit_and_readback_confirms_it() {
        let root = test_root();
        let delivery = NativeDelivery::open(&root).expect("open outbox");
        let keys = Keys::generate();
        let event = signed_event(&keys, "answer");
        let event_id = event.id;
        let key = "batch-a";
        let pending_path = delivery.pending_path(key);
        let observed = Arc::new(Mutex::new(Vec::new()));

        let submit_observed = Arc::clone(&observed);
        let state = delivery
            .deliver_with(
                key,
                || Ok(event.clone()),
                move |submitted| {
                    let submit_observed = Arc::clone(&submit_observed);
                    let pending_path = pending_path.clone();
                    async move {
                        assert!(pending_path.is_file(), "event must exist before submit");
                        let record = load_record(&pending_path).expect("load staged record");
                        assert_eq!(record.event.id, submitted.id);
                        record.event.verify().expect("staged event must verify");
                        submit_observed.lock().unwrap().push(submitted.id);
                        Err("landed but response was malformed".to_string())
                    }
                },
                move |candidate| async move { Ok(candidate.id == event_id) },
            )
            .await
            .expect("reconcile ambiguous acknowledgement");

        assert_eq!(state, DeliveryState::Confirmed { event_id });
        assert_eq!(observed.lock().unwrap().as_slice(), &[event_id]);
        assert!(!delivery.pending_path(key).exists());
        assert!(delivery.delivered_path(key).is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn restart_recovery_reuses_exact_event_without_rebuilding_or_rerunning() {
        let root = test_root();
        let keys = Keys::generate();
        let event = signed_event(&keys, "completed once");
        let event_id = event.id;
        let key = "batch-restart";

        let first = NativeDelivery::open(&root).expect("open first manager");
        let first_state = first
            .deliver_with(
                key,
                || Ok(event.clone()),
                |_submitted| async { Ok(()) },
                |_candidate| async { Ok(false) },
            )
            .await
            .expect("leave event pending");
        assert!(matches!(first_state, DeliveryState::Pending { .. }));
        drop(first);

        let second = NativeDelivery::open(&root).expect("open after restart");
        let submitted = Arc::new(Mutex::new(Vec::new()));
        let submitted_clone = Arc::clone(&submitted);
        let second_state = second
            .deliver_with(
                key,
                || panic!("existing delivery key must not rebuild or rerun the model"),
                move |candidate| {
                    let submitted_clone = Arc::clone(&submitted_clone);
                    async move {
                        submitted_clone.lock().unwrap().push(candidate.id);
                        Ok(())
                    }
                },
                move |candidate| async move { Ok(candidate.id == event_id) },
            )
            .await
            .expect("recover staged event");

        assert_eq!(second_state, DeliveryState::Confirmed { event_id });
        assert_eq!(submitted.lock().unwrap().as_slice(), &[event_id]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn absent_readback_keeps_exact_event_pending() {
        let root = test_root();
        let delivery = NativeDelivery::open(&root).expect("open outbox");
        let event = signed_event(&Keys::generate(), "not visible yet");
        let event_id = event.id;
        let state = delivery
            .deliver_with(
                "batch-pending",
                || Ok(event),
                |_submitted| async { Ok(()) },
                |_candidate| async { Ok(false) },
            )
            .await
            .expect("pending result");

        assert_eq!(state, DeliveryState::Pending { event_id });
        assert!(delivery.pending_path("batch-pending").is_file());
        assert!(!delivery.delivered_path("batch-pending").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn corrupt_pending_record_is_quarantined_and_never_submitted() {
        let root = test_root();
        let delivery = NativeDelivery::open(&root).expect("open outbox");
        std::fs::write(delivery.pending_path("broken"), b"{partial").expect("write corrupt record");
        let submits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let submits_clone = Arc::clone(&submits);

        let report = delivery
            .recover_with(
                move |_candidate| {
                    let submits_clone = Arc::clone(&submits_clone);
                    async move {
                        submits_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(())
                    }
                },
                |_candidate| async { Ok(false) },
            )
            .await;

        assert_eq!(report.quarantined, 1);
        assert_eq!(submits.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!delivery.pending_path("broken").exists());
        assert_eq!(
            std::fs::read_dir(delivery.corrupt_dir()).unwrap().count(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn delivery_key_is_stable_for_the_same_immutable_batch() {
        let agent_keys = Keys::generate();
        let channel_id = Uuid::new_v4();
        let inbound = signed_event(&Keys::generate(), "request");
        let one = batch(inbound.clone(), channel_id);
        let two = batch(inbound, channel_id);
        let key = delivery_key(&agent_keys.public_key(), &one);
        assert_eq!(key, delivery_key(&agent_keys.public_key(), &two));
        assert_eq!(key.len(), 64);
        assert_ne!(
            key,
            delivery_key(&Keys::generate().public_key(), &two),
            "different publishing agents must not collide"
        );
    }

    #[test]
    fn twin_writers_cannot_replace_the_first_signed_event() {
        let root = test_root();
        let pending = root.join("pending");
        std::fs::create_dir_all(&pending).expect("create pending directory");
        let path = pending.join("shared-batch.json");
        let first_event = signed_event(&Keys::generate(), "first answer");
        let second_event = signed_event(&Keys::generate(), "second answer");
        let first = DeliveryRecord {
            version: RECORD_VERSION,
            delivery_key: "shared-batch".into(),
            event: first_event.clone(),
        };
        let second = DeliveryRecord {
            version: RECORD_VERSION,
            delivery_key: "shared-batch".into(),
            event: second_event.clone(),
        };
        let barrier = Arc::new(std::sync::Barrier::new(2));

        let writer = |record: DeliveryRecord, barrier: Arc<std::sync::Barrier>| {
            let path = path.clone();
            std::thread::spawn(move || {
                barrier.wait();
                persist_record_atomically(&path, &record)
            })
        };
        let first_result = writer(first, Arc::clone(&barrier));
        let second_result = writer(second, barrier);
        let results = [
            first_result.join().expect("first writer joins"),
            second_result.join().expect("second writer joins"),
        ];

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(DeliveryError::InvalidRecord(_))))
                .count(),
            1
        );
        let stored = load_record_for_key(&path, "shared-batch").expect("load winning record");
        assert!(stored.event.id == first_event.id || stored.event.id == second_event.id);
        std::fs::remove_dir_all(root).expect("remove test outbox");
    }

    #[test]
    fn human_top_level_reply_opens_thread_and_notifies_triggering_author() {
        let agent_keys = Keys::generate();
        let human_keys = Keys::generate();
        let channel_id = Uuid::new_v4();
        let inbound = signed_event(&human_keys, "@agent help");
        let inbound_id = inbound.id.to_hex();
        let event = build_reply_event(&agent_keys, &batch(inbound, channel_id), "done", None)
            .expect("build native reply");

        assert_eq!(tag_values(&event, "h", None), vec![channel_id.to_string()]);
        assert_eq!(tag_values(&event, "e", Some("reply")), vec![inbound_id]);
        assert_eq!(
            tag_values(&event, "p", None),
            vec![human_keys.public_key().to_hex()]
        );
    }

    #[test]
    fn human_thread_reply_is_flat_at_root() {
        let agent_keys = Keys::generate();
        let human_keys = Keys::generate();
        let channel_id = Uuid::new_v4();
        let root_id = signed_event(&Keys::generate(), "root").id.to_hex();
        let parent_id = signed_event(&Keys::generate(), "parent").id.to_hex();
        let inbound = tagged_event(
            &human_keys,
            "@agent nested request",
            vec![
                vec!["e".into(), root_id.clone(), "".into(), "root".into()],
                vec!["e".into(), parent_id, "".into(), "reply".into()],
            ],
        );
        let event = build_reply_event(&agent_keys, &batch(inbound, channel_id), "done", None)
            .expect("build native reply");

        // A direct reply to the root uses the canonical single `reply` marker;
        // `parse_thread_tags` derives root == parent from that shape.
        assert!(tag_values(&event, "e", Some("root")).is_empty());
        assert_eq!(tag_values(&event, "e", Some("reply")), vec![root_id]);
    }

    #[test]
    fn agent_only_thread_reply_nests_under_triggering_event() {
        let agent_keys = Keys::generate();
        let sender_keys = Keys::generate();
        let channel_id = Uuid::new_v4();
        let root_id = signed_event(&Keys::generate(), "root").id.to_hex();
        let inbound = tagged_event(
            &sender_keys,
            "agent coordination",
            vec![vec!["e".into(), root_id.clone(), "".into(), "reply".into()]],
        );
        let inbound_id = inbound.id.to_hex();
        let profiles: PromptProfileLookup = HashMap::from([(
            sender_keys.public_key().to_hex(),
            PromptProfile {
                is_agent: true,
                ..Default::default()
            },
        )]);
        let event = build_reply_event(
            &agent_keys,
            &batch(inbound, channel_id),
            "done",
            Some(&profiles),
        )
        .expect("build native reply");

        assert_eq!(tag_values(&event, "e", Some("root")), vec![root_id]);
        assert_eq!(tag_values(&event, "e", Some("reply")), vec![inbound_id]);
    }
}
