//! Durable channel → ACP session bindings for harness restarts.
//!
//! `SessionState` is in-memory only. Agents that advertise `loadSession` (e.g.
//! Hermes) can restore a prior ACP conversation after the harness respawns if
//! the channel→session mapping survives. This module persists that mapping as
//! a small JSON sidecar under the process data directory.
//!
//! Keyed by `(agent_command_identity, agent_args, channel_id)` so different
//! agent binaries / profiles do not share bindings. Heartbeats are never
//! stored — they stay ephemeral.
//!
//! Cross-process safety: the store is a shared file. Every read and mutation
//! takes a sibling lockfile, reloads the on-disk map under that lock, then
//! writes atomically. A process-local cache alone is unsafe when two
//! `buzz-acp` processes share the same agent command/args identity.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::normalize_agent_command_identity;

/// Environment override for the session store path (tests / operators).
pub const SESSION_STORE_ENV: &str = "BUZZ_ACP_SESSION_STORE";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
struct StoreFile {
    /// version for future migrations
    version: u32,
    /// map key → ACP session id
    sessions: HashMap<String, String>,
}

/// Durable session binding store shared across buzz-acp processes.
pub struct SessionStore {
    path: PathBuf,
    lock_path: PathBuf,
}

/// RAII wrapper that unlocks the OS file lock on drop.
struct StoreLock {
    file: File,
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

impl SessionStore {
    /// Open or create the store at the resolved path.
    ///
    /// Does not cache file contents; each operation reloads under lock.
    pub fn open(path: PathBuf) -> Self {
        let lock_path = sibling_lock_path(&path);
        Self { path, lock_path }
    }

    /// Resolve the default store path for this agent identity.
    pub fn default_path(agent_command: &str, agent_args: &[String]) -> PathBuf {
        if let Ok(override_path) = std::env::var(SESSION_STORE_ENV) {
            if !override_path.trim().is_empty() {
                return PathBuf::from(override_path);
            }
        }
        let identity = store_identity(agent_command, agent_args);
        let base = dirs::data_local_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("buzz-acp")
            .join("sessions")
            .join(format!("{identity}.json"))
    }

    /// Look up a stored ACP session id for a channel.
    pub fn get(
        &self,
        agent_command: &str,
        agent_args: &[String],
        channel_id: &Uuid,
    ) -> Option<String> {
        let key = binding_key(agent_command, agent_args, channel_id);
        let _lock = self.acquire_lock(false)?;
        match load_store(&self.path) {
            Ok(data) => data.sessions.get(&key).cloned(),
            Err(e) => {
                self.warn_io("failed to read ACP session bindings", &e);
                None
            }
        }
    }

    /// Persist a channel → session binding.
    pub fn put(
        &self,
        agent_command: &str,
        agent_args: &[String],
        channel_id: &Uuid,
        session_id: &str,
    ) {
        let key = binding_key(agent_command, agent_args, channel_id);
        let Some(_lock) = self.acquire_lock(true) else {
            return;
        };
        let mut data = match load_store(&self.path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                // Corrupt sidecar: log and recover empty rather than wedging puts forever.
                self.warn_io(
                    "corrupt ACP session store on update — rewriting from empty map",
                    &e,
                );
                StoreFile::default()
            }
            Err(e) => {
                self.warn_io("failed to read ACP session bindings before update", &e);
                return;
            }
        };
        data.version = 1;
        data.sessions.insert(key, session_id.to_owned());
        if let Err(e) = save_store(&self.path, &data) {
            self.warn_io("failed to persist ACP session binding", &e);
        }
    }

    /// Remove a binding (after invalidation / failed load).
    pub fn remove(&self, agent_command: &str, agent_args: &[String], channel_id: &Uuid) {
        let key = binding_key(agent_command, agent_args, channel_id);
        let Some(_lock) = self.acquire_lock(true) else {
            return;
        };
        match load_store(&self.path) {
            Ok(mut data) => {
                if data.sessions.remove(&key).is_some() {
                    if let Err(e) = save_store(&self.path, &data) {
                        self.warn_io("failed to persist ACP session binding removal", &e);
                    }
                }
            }
            Err(e) => self.warn_io("failed to read ACP session bindings before removal", &e),
        }
    }

    fn acquire_lock(&self, exclusive: bool) -> Option<StoreLock> {
        if let Some(parent) = self.lock_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                self.warn_io("failed to create ACP session store directory", &e);
                return None;
            }
        }
        let file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock_path)
        {
            Ok(file) => file,
            Err(e) => {
                self.warn_io("failed to open ACP session store lock", &e);
                return None;
            }
        };
        let result = if exclusive {
            FileExt::lock_exclusive(&file)
        } else {
            FileExt::lock_shared(&file)
        };
        if let Err(e) = result {
            self.warn_io("failed to lock ACP session store", &e);
            return None;
        }
        Some(StoreLock { file })
    }

    fn warn_io(&self, message: &'static str, error: &std::io::Error) {
        tracing::warn!(
            target: "session_store",
            path = %self.path.display(),
            lock_path = %self.lock_path.display(),
            error = %error,
            "{message}"
        );
    }
}

fn sibling_lock_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(OsString::from(".lock"));
    PathBuf::from(name)
}

fn store_identity(agent_command: &str, agent_args: &[String]) -> String {
    let cmd = normalize_agent_command_identity(agent_command);
    let args = agent_args.join(" ");
    let raw = if args.is_empty() {
        cmd
    } else {
        format!("{cmd} {args}")
    };
    // Keep the filename filesystem-safe and short.
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "agent".into()
    } else {
        out
    }
}

fn binding_key(agent_command: &str, agent_args: &[String], channel_id: &Uuid) -> String {
    format!(
        "{}|{}|{}",
        normalize_agent_command_identity(agent_command),
        agent_args.join("\u{1f}"),
        channel_id
    )
}

fn load_store(path: &Path) -> std::io::Result<StoreFile> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(StoreFile::default()),
        Err(e) => Err(e),
    }
}

fn save_store(path: &Path, data: &StoreFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_binding() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let store = SessionStore::open(path);
        let channel = Uuid::new_v4();
        assert!(store.get("hermes", &["acp".into()], &channel).is_none());
        store.put("hermes", &["acp".into()], &channel, "sess-1");
        assert_eq!(
            store.get("hermes", &["acp".into()], &channel).as_deref(),
            Some("sess-1")
        );
        // Re-open from disk.
        let store2 = SessionStore::open(store.path.clone());
        assert_eq!(
            store2.get("hermes", &["acp".into()], &channel).as_deref(),
            Some("sess-1")
        );
        store2.remove("hermes", &["acp".into()], &channel);
        assert!(store2.get("hermes", &["acp".into()], &channel).is_none());
    }

    #[test]
    fn different_args_are_isolated() {
        let dir = tempdir().unwrap();
        let store = SessionStore::open(dir.path().join("s.json"));
        let channel = Uuid::new_v4();
        store.put("hermes", &["acp".into()], &channel, "a");
        store.put(
            "hermes",
            &["-p".into(), "chad".into(), "acp".into()],
            &channel,
            "b",
        );
        assert_eq!(
            store.get("hermes", &["acp".into()], &channel).as_deref(),
            Some("a")
        );
        assert_eq!(
            store
                .get(
                    "hermes",
                    &["-p".into(), "chad".into(), "acp".into()],
                    &channel
                )
                .as_deref(),
            Some("b")
        );
    }

    #[test]
    fn independently_opened_stores_do_not_lose_updates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let store_a = SessionStore::open(path.clone());
        let store_b = SessionStore::open(path.clone());
        let channel_a = Uuid::new_v4();
        let channel_b = Uuid::new_v4();
        let channel_c = Uuid::new_v4();
        let args = ["acp".into()];

        store_a.put("hermes", &args, &channel_a, "session-a");
        store_b.put("hermes", &args, &channel_b, "session-b");

        let reopened = SessionStore::open(path.clone());
        assert_eq!(
            reopened.get("hermes", &args, &channel_a).as_deref(),
            Some("session-a")
        );
        assert_eq!(
            reopened.get("hermes", &args, &channel_b).as_deref(),
            Some("session-b")
        );

        // Open both before either mutation. A stale process-local snapshot would
        // resurrect channel A when the second store writes channel C.
        let remover = SessionStore::open(path.clone());
        let writer = SessionStore::open(path.clone());
        remover.remove("hermes", &args, &channel_a);
        writer.put("hermes", &args, &channel_c, "session-c");

        let final_store = SessionStore::open(path);
        assert!(final_store.get("hermes", &args, &channel_a).is_none());
        assert_eq!(
            final_store.get("hermes", &args, &channel_b).as_deref(),
            Some("session-b")
        );
        assert_eq!(
            final_store.get("hermes", &args, &channel_c).as_deref(),
            Some("session-c")
        );
    }

    #[test]
    fn put_recovers_from_corrupt_store() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        fs::write(&path, "{not-json").unwrap();
        let store = SessionStore::open(path.clone());
        let channel = Uuid::new_v4();
        store.put("hermes", &["acp".into()], &channel, "recovered");
        assert_eq!(
            store.get("hermes", &["acp".into()], &channel).as_deref(),
            Some("recovered")
        );
    }
}
