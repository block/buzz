//! Durable channel → ACP session map, so a harness restart resumes instead of
//! forgetting.
//!
//! `AgentPool` keeps `SessionState.sessions` in memory. That is correct while
//! the process lives, but every restart — a desktop relaunch, a config-change
//! restart, or a hosted harness being moved between nodes on a deploy — emptied
//! it, and the next mention in every channel got a fresh `session/new`. For an
//! agent whose session *is* its workspace (a sandbox per session, as with the
//! `fountain acp` gateway) that discards the channel's memory and files each
//! time.
//!
//! The store is a small JSON file: `{ "<channel uuid>": "<session id>", ... }`.
//! Writes are atomic (temp file + rename). Every failure is logged and
//! swallowed — the store is an optimisation on top of the in-memory map, never
//! a reason to refuse a turn.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use uuid::Uuid;

#[derive(Debug)]
pub struct SessionStore {
    /// `None` disables persistence: `get` is always empty, `put`/`remove` no-op.
    path: Option<PathBuf>,
    map: Mutex<HashMap<Uuid, String>>,
    /// Channels whose owner asked (`!rotate`) that the *next* session start
    /// from nothing — see [`request_fresh`](Self::request_fresh). In-memory
    /// only and independent of `path`: it is a signal about the next
    /// `session/new`, not a memory of a past one.
    fresh: Mutex<HashSet<Uuid>>,
}

impl SessionStore {
    /// A store that remembers nothing across restarts.
    pub fn disabled() -> Self {
        Self {
            path: None,
            map: Mutex::new(HashMap::new()),
            fresh: Mutex::new(HashSet::new()),
        }
    }

    /// Open (or create on first write) the store at `path`, loading whatever is
    /// there. An unreadable or malformed file is treated as empty and logged.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let map = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<HashMap<Uuid, String>>(&bytes) {
                Ok(map) => map,
                Err(e) => {
                    tracing::warn!(
                        target: "session_store",
                        "ignoring malformed session store {}: {e}",
                        path.display()
                    );
                    HashMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => {
                tracing::warn!(
                    target: "session_store",
                    "cannot read session store {}: {e}",
                    path.display()
                );
                HashMap::new()
            }
        };
        tracing::info!(
            target: "session_store",
            "session store {}: {} channel session(s) remembered",
            path.display(),
            map.len()
        );
        Self {
            path: Some(path),
            map: Mutex::new(map),
            fresh: Mutex::new(HashSet::new()),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.path.is_some()
    }

    /// The session last recorded for `channel_id`, if any.
    pub fn get(&self, channel_id: Uuid) -> Option<String> {
        self.map.lock().ok()?.get(&channel_id).cloned()
    }

    /// Remember `session_id` for `channel_id` and flush.
    pub fn put(&self, channel_id: Uuid, session_id: &str) {
        if self.path.is_none() {
            return;
        }
        if let Ok(mut map) = self.map.lock() {
            map.insert(channel_id, session_id.to_owned());
            self.flush(&map);
        }
    }

    /// Forget `channel_id` (the session was invalidated, or a resume of it
    /// failed) and flush.
    pub fn remove(&self, channel_id: Uuid) {
        if self.path.is_none() {
            return;
        }
        if let Ok(mut map) = self.map.lock() {
            if map.remove(&channel_id).is_some() {
                self.flush(&map);
            }
        }
    }

    /// Record that the owner rotated `channel_id`: the next `session/new` for
    /// it must tell the agent to start fresh (`_meta.freshSession`), even if
    /// the agent keys its own state by channel and would otherwise resume.
    ///
    /// Dropping the harness's ACP session is not enough on its own. An agent
    /// that resumes by `_meta.channelId` (`fountain acp` and its channel-bound
    /// conversations) hands the same conversation back on the very next
    /// `session/new`, which turns `!rotate` into a no-op. This flag is how the
    /// harness relays the owner's intent through.
    pub fn request_fresh(&self, channel_id: Uuid) {
        if let Ok(mut fresh) = self.fresh.lock() {
            fresh.insert(channel_id);
        }
    }

    /// Consume a pending fresh-session request for `channel_id`. Returns
    /// `true` at most once per [`request_fresh`](Self::request_fresh).
    pub fn take_fresh(&self, channel_id: Uuid) -> bool {
        self.fresh
            .lock()
            .map(|mut fresh| fresh.remove(&channel_id))
            .unwrap_or(false)
    }

    fn flush(&self, map: &HashMap<Uuid, String>) {
        let Some(path) = &self.path else { return };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            let bytes = serde_json::to_vec_pretty(map).map_err(std::io::Error::other)?;
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            tracing::warn!(
                target: "session_store",
                "cannot write session store {}: {e}",
                path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "buzz-acp-session-store-{}-{name}.json",
            std::process::id()
        ))
    }

    #[test]
    fn fresh_request_is_consumed_once_and_works_when_disabled() {
        let store = SessionStore::disabled();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert!(!store.take_fresh(a));
        store.request_fresh(a);
        assert!(!store.take_fresh(b), "other channels are unaffected");
        assert!(store.take_fresh(a));
        assert!(!store.take_fresh(a), "consumed by the first take");
    }

    #[test]
    fn disabled_store_remembers_nothing() {
        let store = SessionStore::disabled();
        let ch = Uuid::new_v4();
        store.put(ch, "sess");
        assert_eq!(store.get(ch), None);
        assert!(!store.is_enabled());
    }

    #[test]
    fn put_survives_reopen_and_remove_forgets() {
        let path = tmp_path("roundtrip");
        let _ = std::fs::remove_file(&path);
        let ch_a = Uuid::new_v4();
        let ch_b = Uuid::new_v4();

        let store = SessionStore::open(&path);
        store.put(ch_a, "sess-a");
        store.put(ch_b, "sess-b");
        drop(store);

        // "The process restarted."
        let store = SessionStore::open(&path);
        assert_eq!(store.get(ch_a).as_deref(), Some("sess-a"));
        assert_eq!(store.get(ch_b).as_deref(), Some("sess-b"));

        store.remove(ch_a);
        drop(store);
        let store = SessionStore::open(&path);
        assert_eq!(store.get(ch_a), None);
        assert_eq!(store.get(ch_b).as_deref(), Some("sess-b"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn malformed_file_is_treated_as_empty() {
        let path = tmp_path("malformed");
        std::fs::write(&path, b"{ not json").unwrap();
        let store = SessionStore::open(&path);
        assert_eq!(store.get(Uuid::new_v4()), None);
        // And it is writable afterwards.
        let ch = Uuid::new_v4();
        store.put(ch, "sess");
        assert_eq!(SessionStore::open(&path).get(ch).as_deref(), Some("sess"));
        let _ = std::fs::remove_file(&path);
    }
}
