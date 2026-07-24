//! Import state file — idempotency and resume.
//!
//! The state file records every write the importer has completed, keyed by
//! Slack-side identifiers, so a re-run (after an interruption or on a
//! refreshed export) skips work already done. It also doubles as the
//! Slack-ts → Nostr-event-id ledger that thread replies are resolved from.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::CliError;

/// Per-channel state: the Buzz UUID minted for it and whether metadata
/// (create + topic/purpose) has been published.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelState {
    /// Buzz channel UUID.
    pub uuid: String,
    /// Whether the create/topic/purpose events were accepted.
    #[serde(default)]
    pub metadata_done: bool,
    /// Whether an archived Slack channel was archived in Buzz.
    #[serde(default)]
    pub archived_done: bool,
}

/// The whole state file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ImportState {
    /// Slack channel ID → channel state.
    #[serde(default)]
    pub channels: HashMap<String, ChannelState>,
    /// `"<slack channel id>:<ts>"` → Nostr event ID (hex).
    #[serde(default)]
    pub messages: HashMap<String, String>,
    /// Reaction dedupe keys: `"<slack channel id>:<ts>:<emoji>"`.
    #[serde(default)]
    pub reactions: HashSet<String>,
}

impl ImportState {
    /// Load state from `path`; a missing file yields empty state.
    pub fn load(path: &Path) -> Result<Self, CliError> {
        match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
                CliError::Usage(format!(
                    "state file {} is corrupt: {e} — move it aside to restart the import",
                    path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(CliError::Other(format!(
                "cannot read state file {}: {e}",
                path.display()
            ))),
        }
    }

    /// Persist state to `path` (write-temp-then-rename so an interrupted
    /// save never truncates the previous state).
    pub fn save(&self, path: &Path) -> Result<(), CliError> {
        let raw = serde_json::to_string(self)
            .map_err(|e| CliError::Other(format!("state serialization failed: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, raw)
            .map_err(|e| CliError::Other(format!("cannot write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| CliError::Other(format!("cannot rename state file into place: {e}")))?;
        Ok(())
    }

    /// Ledger key for a message.
    pub fn message_key(channel_id: &str, ts: &str) -> String {
        format!("{channel_id}:{ts}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_disk() {
        let dir = std::env::temp_dir().join(format!("buzz-import-state-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("state.json");

        let mut state = ImportState::default();
        state.channels.insert(
            "C1".into(),
            ChannelState {
                uuid: "u-u-i-d".into(),
                metadata_done: true,
                archived_done: true,
            },
        );
        state
            .messages
            .insert(ImportState::message_key("C1", "1.000"), "ff".repeat(32));
        state.reactions.insert("C1:1.000:👍".into());
        state.save(&path).expect("save");

        let loaded = ImportState::load(&path).expect("load");
        assert_eq!(loaded.channels["C1"].uuid, "u-u-i-d");
        assert!(loaded.channels["C1"].metadata_done);
        assert!(loaded.channels["C1"].archived_done);
        assert_eq!(loaded.messages["C1:1.000"], "ff".repeat(32));
        assert!(loaded.reactions.contains("C1:1.000:👍"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_is_empty_state() {
        let path = std::path::PathBuf::from("/nonexistent/dir/state.json");
        let state = ImportState::load(&path).expect("missing file is fine");
        assert!(state.channels.is_empty());
        assert!(state.messages.is_empty());
    }
}
