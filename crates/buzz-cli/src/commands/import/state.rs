//! Import ledger — idempotency and resume.
//!
//! The ledger records every write the importer has completed, keyed by
//! Slack-side identifiers, so a re-run (after an interruption or on a
//! refreshed export) skips work already done. It also doubles as the
//! Slack-ts → Nostr-event-id map that thread replies are resolved from.
//!
//! Imported messages are stamped with the import time, so their event ids are
//! not reproducible: signing a message again after the relay already stored it
//! would publish it twice. The ledger therefore has to be exact, and lives in
//! two files:
//!
//! - the **snapshot** (`<state>`), rewritten atomically at channel boundaries;
//! - the **journal** (`<state>.journal`), one synced line appended per write.
//!
//! Before a message is submitted, the id of its signed event is journaled as
//! an *intent*. When a run dies, or the relay's answer is lost, before the
//! matching *message* record lands, the intent stays pending. The next run
//! asks the relay for that id and either adopts the stored event or signs the
//! message afresh. Compacting the journal into the snapshot keeps pending
//! intents, so an unresolved one is never dropped.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

const STATE_VERSION: u32 = 1;

/// Per-channel state: the Buzz UUID it maps to and which one-off channel
/// writes have been accepted.
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
    /// Whether a private Slack conversation is known to be private in Buzz.
    /// False for a channel adopted through the crosswalk until the importer
    /// has narrowed it, so private history is never published into a channel
    /// that is still open.
    #[serde(default)]
    pub private_visibility_done: bool,
    /// Set before the importer reopens a channel it found archived, and
    /// cleared once the channel is archived again, so an interrupted run still
    /// restores it.
    #[serde(default)]
    pub restore_archived: bool,
    /// Buzz public keys already added from mapped Slack conversation members.
    #[serde(default)]
    pub members_added: HashSet<String>,
}

/// The ledger snapshot.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportState {
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
    /// Message and reaction keys that can never be imported (for example, a
    /// message over the size limit), so later runs do not treat them as
    /// outstanding work.
    #[serde(default)]
    pub skipped: HashSet<String>,
    /// Message key → id of an event that was signed and handed to the relay
    /// without a confirmed outcome.
    #[serde(default)]
    pub pending: HashMap<String, String>,
}

impl Default for ImportState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            team_id: None,
            channels: HashMap::new(),
            messages: HashMap::new(),
            reactions: HashSet::new(),
            skipped: HashSet::new(),
            pending: HashMap::new(),
        }
    }
}

/// One journaled write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalRecord {
    /// A message was signed as event `id` and is about to be submitted.
    Intent { key: String, id: String },
    /// The relay stored the message as event `id`.
    Message { key: String, id: String },
    /// The relay stored (or already held) the reaction with this dedupe key.
    Reaction { key: String },
    /// A mapped member was added to the conversation with this Slack id.
    Member { channel: String, pubkey: String },
    /// The message or reaction with this key can never be imported.
    Skipped { key: String },
}

impl ImportState {
    /// Load the ledger at `path` — the snapshot plus any journal left by an
    /// interrupted run — and bind it to one Slack workspace.
    ///
    /// A missing snapshot yields a fresh ledger. Loading never writes, so a
    /// dry run can read a ledger without changing it.
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

        if state.version != STATE_VERSION {
            return Err(CliError::Usage(format!(
                "state file {} uses schema version {}; this CLI supports version {STATE_VERSION}",
                path.display(),
                state.version,
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
        state.team_id = Some(team_id.to_string());

        for record in read_journal(&journal_path(path))? {
            state.apply(record);
        }
        Ok(state)
    }

    /// Fold one journaled write into the ledger. Applying a record twice is
    /// harmless, so a journal that outlived its compaction replays cleanly.
    pub fn apply(&mut self, record: JournalRecord) {
        match record {
            JournalRecord::Intent { key, id } => {
                if !self.messages.contains_key(&key) {
                    self.pending.insert(key, id);
                }
            }
            JournalRecord::Message { key, id } => {
                self.pending.remove(&key);
                self.messages.insert(key, id);
            }
            JournalRecord::Reaction { key } => {
                self.reactions.insert(key);
            }
            JournalRecord::Member { channel, pubkey } => {
                if let Some(state) = self.channels.get_mut(&channel) {
                    state.members_added.insert(pubkey);
                }
            }
            JournalRecord::Skipped { key } => {
                self.skipped.insert(key);
            }
        }
    }

    /// Whether the message with this key needs no further work: it was
    /// imported, or it never can be.
    pub fn message_settled(&self, key: &str) -> bool {
        self.messages.contains_key(key) || self.skipped.contains(key)
    }

    /// Whether the reaction with this dedupe key needs no further work.
    pub fn reaction_settled(&self, key: &str) -> bool {
        self.reactions.contains(key) || self.skipped.contains(key)
    }

    /// Persist the snapshot to `path`: write a temp file, sync it, then
    /// rename it into place. The snapshot is durable before this returns, so
    /// the caller may drop the journal entries it replaces.
    pub fn save(&self, path: &Path) -> Result<(), CliError> {
        let raw = serde_json::to_vec(self)
            .map_err(|e| CliError::Other(format!("state serialization failed: {e}")))?;
        let tmp = sibling(path, ".tmp");
        File::create(&tmp)
            .and_then(|mut file| {
                file.write_all(&raw)?;
                file.sync_all()
            })
            .map_err(|e| CliError::Other(format!("cannot write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| CliError::Other(format!("cannot rename state file into place: {e}")))?;
        // Make the rename itself durable where the platform allows syncing a
        // directory; elsewhere the rename is already the best available.
        if let Some(parent) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    }

    /// Ledger key for a message.
    pub fn message_key(channel_id: &str, ts: &str) -> String {
        format!("{channel_id}:{ts}")
    }
}

/// Append-only journal of the writes made since the last snapshot.
pub struct Journal {
    file: File,
    path: PathBuf,
}

impl Journal {
    /// Open (creating if needed) the journal that accompanies `state_path`,
    /// positioned to append.
    ///
    /// A final line cut short by a crash is removed first, so the next append
    /// starts on a line of its own instead of extending the fragment.
    pub fn open(state_path: &Path) -> Result<Self, CliError> {
        let path = journal_path(state_path);
        let io_error =
            |e: std::io::Error| CliError::Other(format!("cannot open {}: {e}", path.display()));
        // Plain read/write, not append mode: an append-only handle cannot be
        // truncated on every platform.
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(io_error)?;
        let existing = std::fs::read(&path).map_err(io_error)?;
        if existing.last().is_some_and(|byte| *byte != b'\n') {
            let complete = existing
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |newline| newline + 1);
            file.set_len(complete as u64)
                .and_then(|()| file.sync_data())
                .map_err(io_error)?;
        }
        file.seek(SeekFrom::End(0)).map_err(io_error)?;
        Ok(Self { file, path })
    }

    /// Append one record and sync it to disk before returning.
    pub fn append(&mut self, record: &JournalRecord) -> Result<(), CliError> {
        let mut line = serde_json::to_string(record)
            .map_err(|e| CliError::Other(format!("journal serialization failed: {e}")))?;
        line.push('\n');
        self.file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.sync_data())
            .map_err(|e| CliError::Other(format!("cannot write {}: {e}", self.path.display())))
    }

    /// Drop every record. Call only after a snapshot that already holds them
    /// has been saved.
    pub fn clear(&mut self) -> Result<(), CliError> {
        self.file
            .set_len(0)
            .and_then(|()| self.file.seek(SeekFrom::Start(0)).map(|_| ()))
            .and_then(|()| self.file.sync_data())
            .map_err(|e| CliError::Other(format!("cannot clear {}: {e}", self.path.display())))
    }
}

/// The journal file that accompanies the snapshot at `state_path`.
pub fn journal_path(state_path: &Path) -> PathBuf {
    sibling(state_path, ".journal")
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// Read every record from the journal at `path`; a missing journal is empty.
///
/// A crash can cut the final append short, so an unterminated last line that
/// does not parse is ignored: the write it described was never acknowledged.
/// Any other unreadable line means the file was damaged, which is an error.
fn read_journal(path: &Path) -> Result<Vec<JournalRecord>, CliError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(CliError::Other(format!(
                "cannot read journal {}: {e}",
                path.display()
            )))
        }
    };
    let torn_tail = !raw.is_empty() && !raw.ends_with('\n');
    let lines: Vec<&str> = raw.lines().collect();
    let mut records = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        match serde_json::from_str(line) {
            Ok(record) => records.push(record),
            Err(_) if torn_tail && index + 1 == lines.len() => {}
            Err(e) => {
                return Err(CliError::Usage(format!(
                    "journal {} is corrupt at line {}: {e} — move it aside to restart the import",
                    path.display(),
                    index + 1
                )))
            }
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_state_path(label: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "buzz-import-state-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("state.json");
        (dir, path)
    }

    fn channel_state() -> ChannelState {
        ChannelState {
            uuid: "u-u-i-d".into(),
            metadata_done: true,
            archived_done: true,
            private_visibility_done: true,
            restore_archived: true,
            members_added: HashSet::from(["aa".repeat(32)]),
        }
    }

    #[test]
    fn roundtrips_through_disk() {
        let (dir, path) = temp_state_path("roundtrip");

        let mut state =
            ImportState::load_for_workspace(&path, "T1").expect("missing state is fresh");
        state.channels.insert("C1".into(), channel_state());
        state
            .messages
            .insert(ImportState::message_key("C1", "1.000"), "ff".repeat(32));
        state.reactions.insert("C1:1.000:👍".into());
        state.pending.insert("C1:2.000".into(), "ee".repeat(32));
        state.save(&path).expect("save");

        let loaded = ImportState::load_for_workspace(&path, "T1").expect("load");
        assert_eq!(loaded.channels["C1"].uuid, "u-u-i-d");
        assert!(loaded.channels["C1"].metadata_done);
        assert!(loaded.channels["C1"].archived_done);
        assert!(loaded.channels["C1"].private_visibility_done);
        assert!(loaded.channels["C1"].restore_archived);
        assert!(loaded.channels["C1"]
            .members_added
            .contains(&"aa".repeat(32)));
        assert_eq!(loaded.messages["C1:1.000"], "ff".repeat(32));
        assert!(loaded.reactions.contains("C1:1.000:👍"));
        assert_eq!(loaded.pending["C1:2.000"], "ee".repeat(32));

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
        let (dir, path) = temp_state_path("team");
        ImportState::load_for_workspace(&path, "T1")
            .expect("fresh")
            .save(&path)
            .expect("save");

        let error = ImportState::load_for_workspace(&path, "T2").expect_err("must reject");
        assert!(error.to_string().contains("belongs to Slack workspace T1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_an_unsupported_schema_version() {
        let (dir, path) = temp_state_path("version");
        std::fs::write(&path, r#"{"version":7,"team_id":"T1"}"#).expect("write");

        let error = ImportState::load_for_workspace(&path, "T1").expect_err("must reject");
        assert!(error.to_string().contains("schema version 7"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn journaled_writes_survive_a_run_that_never_saved_a_snapshot() {
        let (dir, path) = temp_state_path("journal");
        let mut seeded = ImportState::load_for_workspace(&path, "T1").expect("fresh");
        seeded.channels.insert("C1".into(), channel_state());
        seeded.save(&path).expect("save");

        let mut journal = Journal::open(&path).expect("open journal");
        for record in [
            JournalRecord::Intent {
                key: "C1:1.0".into(),
                id: "aa".repeat(32),
            },
            JournalRecord::Message {
                key: "C1:1.0".into(),
                id: "aa".repeat(32),
            },
            JournalRecord::Reaction {
                key: "C1:1.0:👍".into(),
            },
            JournalRecord::Member {
                channel: "C1".into(),
                pubkey: "bb".repeat(32),
            },
            JournalRecord::Intent {
                key: "C1:2.0".into(),
                id: "cc".repeat(32),
            },
            JournalRecord::Skipped {
                key: "C1:3.0".into(),
            },
        ] {
            journal.append(&record).expect("append");
        }
        drop(journal);

        let loaded = ImportState::load_for_workspace(&path, "T1").expect("replay");
        assert_eq!(loaded.messages["C1:1.0"], "aa".repeat(32));
        assert!(loaded.message_settled("C1:1.0") && loaded.message_settled("C1:3.0"));
        assert!(!loaded.message_settled("C1:2.0"));
        assert!(loaded.reactions.contains("C1:1.0:👍"));
        assert!(loaded.channels["C1"]
            .members_added
            .contains(&"bb".repeat(32)));
        // The confirmed message is no longer pending; the unconfirmed one is.
        assert_eq!(
            loaded.pending,
            HashMap::from([("C1:2.0".to_string(), "cc".repeat(32))])
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn compaction_keeps_pending_intents_and_replays_idempotently() {
        let (dir, path) = temp_state_path("compact");
        let mut journal = Journal::open(&path).expect("open journal");
        let intent = JournalRecord::Intent {
            key: "C1:1.0".into(),
            id: "aa".repeat(32),
        };
        journal.append(&intent).expect("append");

        let state = ImportState::load_for_workspace(&path, "T1").expect("replay");
        state.save(&path).expect("snapshot");
        // A crash between the snapshot and the journal clear leaves both.
        let replayed = ImportState::load_for_workspace(&path, "T1").expect("replay twice");
        assert_eq!(replayed.pending.len(), 1);

        journal.clear().expect("clear");
        let compacted = ImportState::load_for_workspace(&path, "T1").expect("after clear");
        assert_eq!(compacted.pending["C1:1.0"], "aa".repeat(32));
        assert!(std::fs::read_to_string(journal_path(&path))
            .expect("journal exists")
            .is_empty());

        // A cleared journal keeps accepting records from the start of the file.
        let later = JournalRecord::Reaction {
            key: "C1:1.0:👍".into(),
        };
        journal.append(&later).expect("append after clear");
        assert_eq!(
            std::fs::read_to_string(journal_path(&path)).expect("journal"),
            format!("{}\n", serde_json::to_string(&later).expect("serialize"))
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_torn_final_journal_line_is_ignored_but_other_damage_is_an_error() {
        let (dir, path) = temp_state_path("torn");
        let complete = serde_json::to_string(&JournalRecord::Reaction {
            key: "C1:1.0:👍".into(),
        })
        .expect("serialize");

        std::fs::write(journal_path(&path), format!("{complete}\n{{\"reac")).expect("write");
        let loaded = ImportState::load_for_workspace(&path, "T1").expect("torn tail tolerated");
        assert!(loaded.reactions.contains("C1:1.0:👍"));

        // Reopening drops the fragment, so the next record gets its own line.
        let mut journal = Journal::open(&path).expect("reopen");
        journal
            .append(&JournalRecord::Reaction {
                key: "C1:2.0:👍".into(),
            })
            .expect("append after repair");
        let repaired = ImportState::load_for_workspace(&path, "T1").expect("repaired journal");
        assert!(repaired.reactions.contains("C1:1.0:👍"));
        assert!(repaired.reactions.contains("C1:2.0:👍"));

        std::fs::write(journal_path(&path), format!("not json\n{complete}\n")).expect("write");
        let error = ImportState::load_for_workspace(&path, "T1").expect_err("damage rejected");
        assert!(error.to_string().contains("corrupt at line 1"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
