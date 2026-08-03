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

const STORE_VERSION: u32 = 1;

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
    events: BTreeMap<String, StoredPendingEvent>,
}

pub(crate) struct PendingStore {
    path: PathBuf,
    events: BTreeMap<String, StoredPendingEvent>,
}

impl PendingStore {
    pub(crate) fn open(agent_pubkey: &str) -> io::Result<Self> {
        let path = pending_store_path(agent_pubkey)?;
        Self::open_path(path)
    }

    pub(crate) fn open_path(path: PathBuf) -> io::Result<Self> {
        let events = match std::fs::read(&path) {
            Ok(bytes) => {
                let stored: StoreFile = serde_json::from_slice(&bytes).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid pending-work journal {}: {error}", path.display()),
                    )
                })?;
                if stored.version != STORE_VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "unsupported pending-work journal version {} in {}",
                            stored.version,
                            path.display()
                        ),
                    ));
                }
                stored.events
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { path, events })
    }

    pub(crate) fn restored(&self) -> Vec<StoredPendingEvent> {
        let mut events = self.events.values().cloned().collect::<Vec<_>>();
        events.sort_by_key(|event| event.accepted_at_nanos);
        events
    }

    pub(crate) fn record(&mut self, event: StoredPendingEvent) -> io::Result<()> {
        let id = event.event.id.to_hex();
        let previous = self.events.insert(id.clone(), event);
        if let Err(error) = self.persist() {
            match previous {
                Some(previous) => {
                    self.events.insert(id, previous);
                }
                None => {
                    self.events.remove(&id);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn remove<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) -> io::Result<()> {
        let removed: Vec<(String, StoredPendingEvent)> = ids
            .into_iter()
            .filter_map(|id| self.events.remove_entry(id))
            .collect();
        if removed.is_empty() {
            return Ok(());
        }
        if let Err(error) = self.persist() {
            self.events.extend(removed);
            return Err(error);
        }
        Ok(())
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
            events: self.events.clone(),
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
        std::fs::rename(temp, &self.path)
    }
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
}
