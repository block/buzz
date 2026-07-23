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

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

/// Process-wide durable session binding store.
pub struct SessionStore {
    path: PathBuf,
    inner: Mutex<StoreFile>,
}

impl SessionStore {
    /// Open or create the store at the resolved path.
    pub fn open(path: PathBuf) -> Self {
        let data = load_store(&path);
        Self {
            path,
            inner: Mutex::new(data),
        }
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
    pub fn get(&self, agent_command: &str, agent_args: &[String], channel_id: &Uuid) -> Option<String> {
        let key = binding_key(agent_command, agent_args, channel_id);
        self.inner
            .lock()
            .ok()
            .and_then(|guard| guard.sessions.get(&key).cloned())
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
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        guard.version = 1;
        guard.sessions.insert(key, session_id.to_owned());
        if let Err(e) = save_store(&self.path, &guard) {
            tracing::warn!(
                target: "session_store",
                path = %self.path.display(),
                error = %e,
                "failed to persist ACP session binding"
            );
        }
    }

    /// Remove a binding (after invalidation / failed load).
    pub fn remove(&self, agent_command: &str, agent_args: &[String], channel_id: &Uuid) {
        let key = binding_key(agent_command, agent_args, channel_id);
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        if guard.sessions.remove(&key).is_some() {
            if let Err(e) = save_store(&self.path, &guard) {
                tracing::warn!(
                    target: "session_store",
                    path = %self.path.display(),
                    error = %e,
                    "failed to persist ACP session binding removal"
                );
            }
        }
    }
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

fn load_store(path: &Path) -> StoreFile {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!(
                    target: "session_store",
                    path = %path.display(),
                    error = %e,
                    "corrupt session store — starting empty"
                );
                StoreFile::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => StoreFile::default(),
        Err(e) => {
            tracing::warn!(
                target: "session_store",
                path = %path.display(),
                error = %e,
                "could not read session store — starting empty"
            );
            StoreFile::default()
        }
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
                .get("hermes", &["-p".into(), "chad".into(), "acp".into()], &channel)
                .as_deref(),
            Some("b")
        );
    }
}
