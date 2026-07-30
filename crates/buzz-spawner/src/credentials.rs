//! Per-owner provider credentials, delivered over the encrypted kind:24201
//! channel and held only on this host.
//!
//! Kept in a separate file from `agents.json` so agent records stay
//! credential-free: the two files have different lifecycles (a credential
//! outlives any one agent) and different blast radii when read or logged.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Which env var a token should be injected as, by prefix.
///
/// Claude Code OAuth tokens are `sk-ant-oat…`; anything else is treated as an
/// Anthropic API key. Misclassifying is harmless-but-broken (the harness fails
/// to authenticate), never a leak.
pub fn credential_env_key(token: &str) -> &'static str {
    if token.starts_with("sk-ant-oat") {
        "CLAUDE_CODE_OAUTH_TOKEN"
    } else {
        "ANTHROPIC_API_KEY"
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CredentialFile {
    /// Keyed by owner pubkey, hex.
    #[serde(default)]
    credentials: HashMap<String, String>,
}

/// Persistent owner-pubkey → token store over `<state_dir>/credentials.json`.
pub struct CredentialStore {
    path: PathBuf,
    state: CredentialFile,
}

impl CredentialStore {
    /// Open the store, creating the directory if needed.
    pub fn open(state_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("failed to create state dir {}", state_dir.display()))?;
        let path = state_dir.join("credentials.json");
        let state = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => CredentialFile::default(),
            Err(e) => return Err(e).with_context(|| format!("failed to read {}", path.display())),
        };
        Ok(Self { path, state })
    }

    /// The stored token for an owner, if any.
    pub fn get(&self, owner_pubkey: &str) -> Option<&str> {
        self.state.credentials.get(owner_pubkey).map(String::as_str)
    }

    /// Store or replace an owner's token and persist.
    pub fn set(&mut self, owner_pubkey: &str, token: String) -> Result<()> {
        self.state
            .credentials
            .insert(owner_pubkey.to_string(), token);
        self.flush()
    }

    /// Remove an owner's token and persist. Returns whether one existed.
    pub fn remove(&mut self, owner_pubkey: &str) -> Result<bool> {
        let existed = self.state.credentials.remove(owner_pubkey).is_some();
        if existed {
            self.flush()?;
        }
        Ok(existed)
    }

    /// Owners that currently have a token.
    pub fn owners(&self) -> impl Iterator<Item = &String> {
        self.state.credentials.keys()
    }

    /// Atomic 0600 write, same crash-safety rationale as `Store::flush`.
    fn flush(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.state)
            .context("failed to serialize credential store")?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)
            .with_context(|| format!("failed to write {}", tmp.display()))?;
        crate::store::restrict_permissions(&tmp)?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("failed to rename into {}", self.path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_tokens_by_prefix() {
        assert_eq!(
            credential_env_key("sk-ant-oat01-xyz"),
            "CLAUDE_CODE_OAUTH_TOKEN"
        );
        assert_eq!(credential_env_key("sk-ant-api03-xyz"), "ANTHROPIC_API_KEY");
        assert_eq!(credential_env_key("something-else"), "ANTHROPIC_API_KEY");
    }

    #[test]
    fn round_trips_through_the_file_with_owner_only_permissions() {
        let dir = std::env::temp_dir().join(format!("buzz-cred-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let owner = "b".repeat(64);
        {
            let mut store = CredentialStore::open(&dir).unwrap();
            store.set(&owner, "sk-ant-oat01-abc".into()).unwrap();
        }
        let mut store = CredentialStore::open(&dir).unwrap();
        assert_eq!(store.get(&owner), Some("sk-ant-oat01-abc"));
        assert!(store.get("missing").is_none());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("credentials.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "credential file holds tokens");
        }

        assert!(store.remove(&owner).unwrap());
        assert!(!store.remove(&owner).unwrap());
        assert!(store.get(&owner).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
