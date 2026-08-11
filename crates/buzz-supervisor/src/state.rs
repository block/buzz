//! Per-channel provisioning state, persisted to disk so a supervisor restart
//! doesn't lose track of (or re-provision) channels it already handled.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChannelStatus {
    /// Fully set up: every role has a published profile and a running (or
    /// last-known-pid) process.
    Provisioned,
    /// Provisioning started but did not finish — at least one role's
    /// identity/membership below was created before an error interrupted
    /// the rest. Not retried automatically (that's how a partial failure
    /// used to leak a fresh set of keys/memberships every poll cycle);
    /// an operator must inspect `roles` and either finish setup manually or
    /// delete this channel's state directory to start over.
    Provisioning,
    Rejected {
        reason: String,
    },
    Ignored,
    TornDown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleState {
    pub pubkey: String,
    pub privkey: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelState {
    pub status: ChannelStatus,
    pub workdir: Option<String>,
    #[serde(default)]
    pub roles: HashMap<String, RoleState>,
}

pub struct StateStore {
    root: PathBuf,
}

impl StateStore {
    pub fn new(root: PathBuf) -> anyhow::Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn path_for(&self, channel_id: &str) -> PathBuf {
        self.root.join(channel_id).join("state.json")
    }

    pub fn known_channels(&self) -> anyhow::Result<Vec<String>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    out.push(name.to_string());
                }
            }
        }
        Ok(out)
    }

    pub fn load(&self, channel_id: &str) -> anyhow::Result<Option<ChannelState>> {
        let path = self.path_for(channel_id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    pub fn save(&self, channel_id: &str, state: &ChannelState) -> anyhow::Result<()> {
        let dir = self.root.join(channel_id);
        fs::create_dir_all(&dir)?;
        let path = self.path_for(channel_id);
        let raw = serde_json::to_string_pretty(state)?;
        fs::write(&path, raw)?;
        // State contains generated private keys — lock it down like the
        // individual key files the earlier bash prototype used.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        Ok(())
    }

    pub fn log_path(&self, channel_id: &str, role: &str) -> PathBuf {
        self.root.join(channel_id).join(format!("{role}.log"))
    }
}
