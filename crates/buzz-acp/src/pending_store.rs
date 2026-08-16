//! Durable journal for channel events accepted by the ACP queue.
//!
//! The relay subscription watermark intentionally starts at process startup, so
//! an event accepted immediately before a service restart is not replayed by
//! the relay. This journal closes that gap: an event is written atomically
//! before the harness publishes its "seen" reaction and is removed only after
//! the turn succeeds or the user receives a terminal failure.

use nostr::Event;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const STORE_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredPendingEvent {
    pub channel_id: Uuid,
    pub event: Event,
    pub prompt_tag: String,
    pub accepted_at_nanos: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    #[serde(default, alias = "events")]
    pending: BTreeMap<String, StoredPendingEvent>,
    #[serde(default)]
    dead_letters: BTreeMap<String, StoredDeadLetter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredDeadLetter {
    pending: StoredPendingEvent,
    reason: String,
    dead_lettered_at_nanos: u64,
}

pub(crate) struct PendingStore {
    path: PathBuf,
    pending: BTreeMap<String, StoredPendingEvent>,
    dead_letters: BTreeMap<String, StoredDeadLetter>,
}

impl PendingStore {
    pub(crate) fn open(agent_pubkey: &str) -> io::Result<Self> {
        let path = pending_store_path(agent_pubkey)?;
        Self::open_path(path)
    }

    pub(crate) fn open_path(path: PathBuf) -> io::Result<Self> {
        let (pending, dead_letters) = match std::fs::read(&path) {
            Ok(bytes) => {
                let stored: StoreFile = serde_json::from_slice(&bytes).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid pending-work journal {}: {error}", path.display()),
                    )
                })?;
                if !matches!(stored.version, 1 | STORE_VERSION) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "unsupported pending-work journal version {} in {}",
                            stored.version,
                            path.display()
                        ),
                    ));
                }
                let pending = stored
                    .pending
                    .into_iter()
                    .map(|(id, event)| validate_stored_event(&path, id, event))
                    .collect::<io::Result<BTreeMap<_, _>>>()?;
                let dead_letters = stored
                    .dead_letters
                    .into_iter()
                    .map(|(id, mut dead_letter)| {
                        let (id, pending) = validate_stored_event(&path, id, dead_letter.pending)?;
                        dead_letter.pending = pending;
                        Ok((id, dead_letter))
                    })
                    .collect::<io::Result<BTreeMap<_, _>>>()?;
                (pending, dead_letters)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                (BTreeMap::new(), BTreeMap::new())
            }
            Err(error) => return Err(error),
        };
        let mut store = Self {
            path,
            pending,
            dead_letters,
        };
        if replay_dead_letters_requested() && !store.dead_letters.is_empty() {
            let replayed = store.replay_all_dead_letters()?;
            tracing::warn!(
                replayed,
                "replayed durable dead letters because BUZZ_ACP_REPLAY_DEAD_LETTERS is enabled"
            );
        }
        Ok(store)
    }

    pub(crate) fn restored(&self) -> Vec<StoredPendingEvent> {
        let mut events = self.pending.values().cloned().collect::<Vec<_>>();
        events.sort_by_key(|event| event.accepted_at_nanos);
        events
    }

    pub(crate) fn record(&mut self, event: StoredPendingEvent) -> io::Result<()> {
        let id = event.event.id.to_hex();
        let previous = self.pending.insert(id.clone(), event);
        if let Err(error) = self.persist() {
            match previous {
                Some(previous) => {
                    self.pending.insert(id, previous);
                }
                None => {
                    self.pending.remove(&id);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn remove<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) -> io::Result<()> {
        let removed: Vec<(String, StoredPendingEvent)> = ids
            .into_iter()
            .filter_map(|id| self.pending.remove_entry(id))
            .collect();
        if removed.is_empty() {
            return Ok(());
        }
        if let Err(error) = self.persist() {
            self.pending.extend(removed);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn dead_letter<'a>(
        &mut self,
        ids: impl IntoIterator<Item = &'a str>,
        reason: &str,
    ) -> io::Result<usize> {
        let old_pending = self.pending.clone();
        let old_dead_letters = self.dead_letters.clone();
        let now = unix_time_nanos();
        let mut moved = 0;
        for id in ids {
            if let Some(pending) = self.pending.remove(id) {
                self.dead_letters.insert(
                    id.to_string(),
                    StoredDeadLetter {
                        pending,
                        reason: reason.to_string(),
                        dead_lettered_at_nanos: now,
                    },
                );
                moved += 1;
            }
        }
        if moved > 0 {
            if let Err(error) = self.persist() {
                self.pending = old_pending;
                self.dead_letters = old_dead_letters;
                return Err(error);
            }
        }
        Ok(moved)
    }

    fn replay_all_dead_letters(&mut self) -> io::Result<usize> {
        let old_pending = self.pending.clone();
        let old_dead_letters = self.dead_letters.clone();
        let replayed = self.dead_letters.len();
        for (id, dead_letter) in std::mem::take(&mut self.dead_letters) {
            self.pending.entry(id).or_insert(dead_letter.pending);
        }
        if let Err(error) = self.persist() {
            self.pending = old_pending;
            self.dead_letters = old_dead_letters;
            return Err(error);
        }
        Ok(replayed)
    }

    fn persist(&self) -> io::Result<()> {
        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "pending journal has no parent")
        })?;
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
        let bytes = serde_json::to_vec(&StoreFile {
            version: STORE_VERSION,
            pending: self.pending.clone(),
            dead_letters: self.dead_letters.clone(),
        })
        .map_err(io::Error::other)?;
        let temp = self.path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        std::io::Write::write_all(&mut file, &bytes)?;
        file.sync_all()?;
        std::fs::rename(temp, &self.path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}

fn validate_stored_event(
    path: &Path,
    id: String,
    event: StoredPendingEvent,
) -> io::Result<(String, StoredPendingEvent)> {
    if id != event.event.id.to_hex() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "pending-work journal {} contains mismatched event id",
                path.display()
            ),
        ));
    }
    event.event.verify().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "pending-work journal {} contains an invalid signed event: {error}",
                path.display()
            ),
        )
    })?;
    Ok((id, event))
}

fn unix_time_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

fn replay_dead_letters_requested() -> bool {
    std::env::var("BUZZ_ACP_REPLAY_DEAD_LETTERS").is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes"
        )
    })
}

fn pending_store_path(agent_pubkey: &str) -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("BUZZ_ACP_PENDING_STORE") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "HOME is unset and BUZZ_ACP_PENDING_STORE was not provided",
        )
    })?;
    Ok(Path::new(&home)
        .join(".local/state/buzz-acp")
        .join(format!("pending-{agent_pubkey}.json")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys};

    fn event(content: &str) -> Event {
        EventBuilder::text_note(content)
            .sign_with_keys(&Keys::generate())
            .expect("sign event")
    }

    #[test]
    fn journal_round_trips_and_removes_atomically() {
        let dir = std::env::temp_dir().join(format!("buzz-acp-pending-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let path = dir.join("pending.json");
        let mut store = PendingStore::open_path(path.clone()).expect("open");
        let event = event("survive restart");
        let id = event.id.to_hex();
        store
            .record(StoredPendingEvent {
                channel_id: Uuid::new_v4(),
                event,
                prompt_tag: "@mention".into(),
                accepted_at_nanos: 1,
            })
            .expect("record");
        drop(store);

        let mut restored = PendingStore::open_path(path.clone()).expect("reopen");
        assert_eq!(restored.restored().len(), 1);
        restored.remove([id.as_str()]).expect("remove");
        drop(restored);
        assert_eq!(
            PendingStore::open_path(path)
                .expect("final reopen")
                .restored()
                .len(),
            0
        );
        std::fs::remove_dir_all(dir).expect("remove tempdir");
    }

    #[test]
    fn dead_letters_are_retained_and_replayable() {
        let dir = std::env::temp_dir().join(format!("buzz-acp-dead-letters-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create tempdir");
        let path = dir.join("pending.json");
        let mut store = PendingStore::open_path(path.clone()).expect("open");
        let event = event("retry me later");
        let id = event.id.to_hex();
        store
            .record(StoredPendingEvent {
                channel_id: Uuid::new_v4(),
                event,
                prompt_tag: "@mention".into(),
                accepted_at_nanos: 1,
            })
            .expect("record");

        assert_eq!(
            store
                .dead_letter([id.as_str()], "retry budget exhausted")
                .expect("dead letter"),
            1
        );
        assert!(store.restored().is_empty());
        drop(store);

        let mut reopened = PendingStore::open_path(path.clone()).expect("reopen");
        assert!(reopened.restored().is_empty());
        assert_eq!(reopened.replay_all_dead_letters().expect("replay"), 1);
        assert_eq!(reopened.restored().len(), 1);
        drop(reopened);

        assert_eq!(
            PendingStore::open_path(path)
                .expect("final reopen")
                .restored()
                .len(),
            1
        );
        std::fs::remove_dir_all(dir).expect("remove tempdir");
    }
}
