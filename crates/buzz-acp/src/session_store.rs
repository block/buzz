//! Durable ACP session affinity for managed Buzz agents.
//!
//! The live pool keeps a fast in-memory channel -> session map. This store is
//! the restart boundary: it survives sidecar/Desktop/ACP process replacement
//! and lets the next child call `session/load` before considering `session/new`.

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionRecord {
    pub work_key: String,
    pub channel_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_event_id: Option<String>,
    pub session_id: String,
    pub agent_pubkey: String,
    pub relay_url: String,
    pub harness_name: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_completed_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continued_from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionStoreFile {
    version: u32,
    entries: HashMap<String, SessionRecord>,
}

impl Default for SessionStoreFile {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            entries: HashMap::new(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct SessionStore {
    path: Arc<PathBuf>,
    agent_pubkey: Arc<String>,
    relay_url: Arc<String>,
    harness_name: Arc<String>,
    state: Arc<Mutex<SessionStoreFile>>,
}

impl SessionStore {
    pub(crate) fn open(agent_pubkey: &str, relay_url: &str, harness_name: &str) -> Self {
        let path = session_store_path(agent_pubkey, relay_url);
        let state = load_store(&path).unwrap_or_else(|error| {
            tracing::warn!(
                target: "pool::session_store",
                path = %path.display(),
                error = %error,
                "could not load durable session map; preserving the unreadable file and starting with an empty in-memory map"
            );
            SessionStoreFile::default()
        });
        Self {
            path: Arc::new(path),
            agent_pubkey: Arc::new(agent_pubkey.to_owned()),
            relay_url: Arc::new(relay_url.to_owned()),
            harness_name: Arc::new(harness_name.to_owned()),
            state: Arc::new(Mutex::new(state)),
        }
    }

    #[cfg(test)]
    pub(crate) fn isolated_for_test(
        agent_pubkey: &str,
        relay_url: &str,
        harness_name: &str,
    ) -> Self {
        Self {
            path: Arc::new(
                std::env::temp_dir()
                    .join("buzz-acp-session-store-tests")
                    .join(format!("{}.json", Uuid::new_v4())),
            ),
            agent_pubkey: Arc::new(agent_pubkey.to_owned()),
            relay_url: Arc::new(relay_url.to_owned()),
            harness_name: Arc::new(harness_name.to_owned()),
            state: Arc::new(Mutex::new(SessionStoreFile::default())),
        }
    }

    /// Stable per-project-channel work key. Buzz project channels are the
    /// isolation boundary; thread roots are retained as metadata so follow-up
    /// chats and replies continue the same session instead of fragmenting it.
    pub(crate) fn work_key(channel_id: Uuid) -> String {
        format!("channel:{channel_id}")
    }

    pub(crate) fn get(&self, channel_id: Uuid) -> Option<SessionRecord> {
        let key = Self::work_key(channel_id);
        self.state.lock().ok()?.entries.get(&key).cloned()
    }

    pub(crate) fn commit_session(
        &self,
        channel_id: Uuid,
        thread_root_event_id: Option<String>,
        session_id: String,
        continued_from: Option<String>,
    ) -> Result<SessionRecord, String> {
        let work_key = Self::work_key(channel_id);
        let mut guard = self.state.lock().map_err(|error| error.to_string())?;
        let last_completed_turn_id = guard
            .entries
            .get(&work_key)
            .and_then(|record| record.last_completed_turn_id.clone());
        let record = SessionRecord {
            work_key: work_key.clone(),
            channel_id,
            thread_root_event_id,
            session_id,
            agent_pubkey: (*self.agent_pubkey).clone(),
            relay_url: (*self.relay_url).clone(),
            harness_name: (*self.harness_name).clone(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            last_completed_turn_id,
            continued_from,
        };
        guard.entries.insert(work_key, record.clone());
        write_store(&self.path, &guard)?;
        Ok(record)
    }

    pub(crate) fn mark_turn_completed(
        &self,
        channel_id: Uuid,
        session_id: &str,
        turn_id: &str,
    ) -> Result<(), String> {
        let key = Self::work_key(channel_id);
        let mut guard = self.state.lock().map_err(|error| error.to_string())?;
        let Some(record) = guard.entries.get_mut(&key) else {
            return Err(format!("no durable session record for {key}"));
        };
        if record.session_id != session_id {
            return Err(format!(
                "session mismatch for {key}: stored {}, completed {session_id}",
                record.session_id
            ));
        }
        record.last_completed_turn_id = Some(turn_id.to_owned());
        record.updated_at = chrono::Utc::now().to_rfc3339();
        write_store(&self.path, &guard)
    }

    pub(crate) fn remove(&self, channel_id: Uuid) -> Result<bool, String> {
        let key = Self::work_key(channel_id);
        let mut guard = self.state.lock().map_err(|error| error.to_string())?;
        let removed = guard.entries.remove(&key).is_some();
        if removed {
            write_store(&self.path, &guard)?;
        }
        Ok(removed)
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

fn load_store(path: &Path) -> Result<SessionStoreFile, String> {
    if !path.exists() {
        return Ok(SessionStoreFile::default());
    }
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let store: SessionStoreFile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    if store.version != STORE_VERSION {
        return Err(format!(
            "unsupported session-store version {} in {}",
            store.version,
            path.display()
        ));
    }
    Ok(store)
}

fn write_store(path: &Path, store: &SessionStoreFile) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("session-store path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create {}: {error}", parent.display()))?;
    let payload = serde_json::to_vec_pretty(store).map_err(|error| error.to_string())?;
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut file = AtomicWriteFile::open(&resolved)
        .map_err(|error| format!("open {} for atomic write: {error}", resolved.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("restrict {}: {error}", resolved.display()))?;
    }
    file.write_all(&payload)
        .map_err(|error| format!("write {}: {error}", resolved.display()))?;
    file.commit()
        .map_err(|error| format!("commit {}: {error}", resolved.display()))
}

fn session_store_path(agent_pubkey: &str, relay_url: &str) -> PathBuf {
    let root = std::env::var_os("BUZZ_ACP_SESSION_STATE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("APPDATA").map(|appdata| {
                PathBuf::from(appdata)
                    .join("xyz.block.buzz.app")
                    .join("agents")
                    .join("session-state")
            })
        })
        .or_else(|| {
            std::env::var_os("XDG_STATE_HOME")
                .map(|state| PathBuf::from(state).join("buzz").join("agent-sessions"))
        })
        .or_else(|| {
            std::env::var_os("HOME").map(|home| {
                PathBuf::from(home)
                    .join(".local")
                    .join("state")
                    .join("buzz")
                    .join("agent-sessions")
            })
        })
        .unwrap_or_else(|| PathBuf::from(".buzz-agent-sessions"));
    let relay_hash = hex::encode(Sha256::digest(relay_url.as_bytes()));
    root.join(agent_pubkey)
        .join(format!("{}.json", &relay_hash[..16]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_key_is_stable_per_channel() {
        let channel = Uuid::parse_str("caab4104-e53f-4d37-9d8a-d2be97f2212e").unwrap();
        assert_eq!(
            SessionStore::work_key(channel),
            format!("channel:{channel}")
        );
    }

    #[test]
    fn commit_mark_and_remove_round_trip_to_disk() {
        let store = SessionStore::isolated_for_test(
            "1111111111111111111111111111111111111111111111111111111111111111",
            "wss://relay.example",
            "hermes-acp",
        );
        let channel = Uuid::parse_str("f748b533-c7b1-46d4-b788-322180370310").unwrap();
        let record = store
            .commit_session(
                channel,
                Some("root-event".into()),
                "session-1".into(),
                Some("session-0".into()),
            )
            .unwrap();
        assert_eq!(record.continued_from.as_deref(), Some("session-0"));
        assert_eq!(store.get(channel).unwrap().session_id, "session-1");

        store
            .mark_turn_completed(channel, "session-1", "turn-1")
            .unwrap();
        let persisted = load_store(store.path()).unwrap();
        let persisted_record = persisted
            .entries
            .get(&SessionStore::work_key(channel))
            .unwrap();
        assert_eq!(
            persisted_record.last_completed_turn_id.as_deref(),
            Some("turn-1")
        );

        assert!(store.remove(channel).unwrap());
        assert!(store.get(channel).is_none());
        assert!(load_store(store.path()).unwrap().entries.is_empty());
        if let Some(parent) = store.path().parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
}
