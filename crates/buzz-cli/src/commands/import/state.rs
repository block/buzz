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

const CURRENT_STATE_VERSION: u32 = 1;

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
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportState {
    /// State schema version. Missing means the file predates workspace pinning.
    #[serde(default)]
    version: u32,
    /// Slack workspace this ledger belongs to.
    #[serde(default)]
    team_id: Option<String>,
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

impl Default for ImportState {
    fn default() -> Self {
        Self {
            version: CURRENT_STATE_VERSION,
            team_id: None,
            channels: HashMap::new(),
            messages: HashMap::new(),
            reactions: HashSet::new(),
        }
    }
}

impl ImportState {
    /// Load state from `path` and bind it to one Slack workspace.
    ///
    /// A missing file yields fresh state. A non-empty legacy file is rejected:
    /// it may contain unscoped `import_author` values, so silently adopting it
    /// could skip messages that can never match workspace-scoped bindings.
    pub fn load_for_workspace(path: &Path, team_id: &str) -> Result<Self, CliError> {
        let mut state: Self = match std::fs::read_to_string(path) {
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
        }?;

        if state.version == 0 && !state.is_empty() {
            return Err(CliError::Usage(format!(
                "state file {} predates workspace-scoped Slack imports and cannot be resumed \
                 safely; use a fresh target community, or add version/team_id only after \
                 verifying the existing events already use slack:<team>:<user> identities",
                path.display()
            )));
        }
        if state.version > CURRENT_STATE_VERSION {
            return Err(CliError::Usage(format!(
                "state file {} uses newer schema version {}; this CLI supports version {}",
                path.display(),
                state.version,
                CURRENT_STATE_VERSION
            )));
        }
        if let Some(saved_team_id) = state.team_id.as_deref() {
            if saved_team_id != team_id {
                return Err(CliError::Usage(format!(
                    "state file {} belongs to Slack workspace {saved_team_id}, not {team_id}",
                    path.display()
                )));
            }
        }
        state.version = CURRENT_STATE_VERSION;
        state.team_id = Some(team_id.to_string());
        Ok(state)
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

    fn is_empty(&self) -> bool {
        self.channels.is_empty() && self.messages.is_empty() && self.reactions.is_empty()
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

        let mut state =
            ImportState::load_for_workspace(&path, "T1").expect("missing state is fresh");
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

        let loaded = ImportState::load_for_workspace(&path, "T1").expect("load");
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
        let state = ImportState::load_for_workspace(&path, "T1").expect("missing file is fine");
        assert!(state.channels.is_empty());
        assert!(state.messages.is_empty());
    }

    #[test]
    fn rejects_state_from_a_different_workspace() {
        let dir =
            std::env::temp_dir().join(format!("buzz-import-state-team-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("state.json");
        ImportState::load_for_workspace(&path, "T1")
            .expect("fresh")
            .save(&path)
            .expect("save");

        let error = ImportState::load_for_workspace(&path, "T2").expect_err("must reject");
        assert!(error.to_string().contains("belongs to Slack workspace T1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_non_empty_legacy_state() {
        let dir =
            std::env::temp_dir().join(format!("buzz-import-state-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("state.json");
        std::fs::write(
            &path,
            r#"{"channels":{},"messages":{"C1:1.0":"abc"},"reactions":[]}"#,
        )
        .expect("write");

        let error = ImportState::load_for_workspace(&path, "T1").expect_err("must reject");
        assert!(error.to_string().contains("predates workspace-scoped"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
