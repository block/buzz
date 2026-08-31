//! Secret-free, durable per-placement execution journal. Callers authenticate and
//! revalidate authorization BEFORE opening it. No replay ever repeats a side
//! effect, including after a crash in the intent→spawn/stop window.
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

use buzz_core_pkg::host_execution::{hex_id, Action, Command, Outcome};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub command_id: String,
    pub request: Command,
    pub outcome: Outcome,
    pub observed_at: u64,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    entries: BTreeMap<String, Entry>,
    // A placement stays fenced even after a confirmed Stop. Only a new explicit
    // durable Start may release it; config edits/auto-reconcile may not resurrect.
    current: Option<String>,
}

pub(crate) enum Begin {
    Execute,
    Replay(Entry),
}

/// An OS file lock also serializes two controller processes sharing one store.
/// Never unlink the lock file: doing so would permit locks on different inodes.
pub(crate) struct Ledger {
    _lock: File,
    path: PathBuf,
    journal: Journal,
}

impl Ledger {
    pub(crate) fn open(directory: &Path, placement: &str) -> Result<Self, String> {
        if placement.is_empty()
            || !placement
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err("invalid execution placement".into());
        }
        fs::create_dir_all(directory).map_err(|_| "execution journal unavailable")?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(directory.join(format!("{placement}.lock")))
            .map_err(|_| "execution lock unavailable")?;
        lock.try_lock().map_err(|_| "execution placement busy")?;
        let path = directory.join(format!("{placement}.json"));
        let journal: Journal = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "execution journal corrupt")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Journal::default(),
            Err(_) => return Err("execution journal unreadable".into()),
        };
        if journal.entries.len() > 4096
            || journal
                .current
                .as_ref()
                .is_some_and(|id| !journal.entries.contains_key(id))
            || (journal.current.is_none() && !journal.entries.is_empty())
            || journal.entries.iter().any(|(id, entry)| {
                id != &entry.request.operation
                    || !hex_id(&entry.command_id, 64)
                    || entry.request.validate().is_err()
            })
        {
            return Err("execution journal invariants invalid".into());
        }
        Ok(Self {
            _lock: lock,
            path,
            journal,
        })
    }

    fn save(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec(&self.journal)
            .map_err(|_| "execution journal serialization failed")?;
        super::atomic_write_json_restricted(&self.path, &bytes)
            .map_err(|_| "execution journal write failed")?;
        // Atomic rename is not a durable intent unless directory metadata is
        // synced too. On unsupported filesystems fail before a side effect.
        #[cfg(unix)]
        {
            let parent = self.path.parent().ok_or("invalid journal directory")?;
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| "execution journal sync failed")?;
        }
        Ok(())
    }

    pub(crate) fn current(&self) -> Option<&Entry> {
        self.journal
            .current
            .as_ref()
            .and_then(|id| self.journal.entries.get(id))
    }

    pub(crate) fn operation(&self, id: &str) -> Option<&Entry> {
        self.journal.entries.get(id)
    }

    pub(crate) fn replay(
        &self,
        command_id: &str,
        request: &Command,
    ) -> Result<Option<Entry>, String> {
        if let Some(entry) = self.journal.entries.get(&request.operation) {
            if entry.command_id != command_id || entry.request != *request {
                return Err("operation ID already belongs to another command".into());
            }
            // Accepted is deliberately not resumed: creation/termination could
            // have happened before the result write. Reconciliation must prove
            // the old generation, not manufacture a replacement.
            let mut result = entry.clone();
            if result.outcome == Outcome::Accepted {
                result.outcome = Outcome::Unknown;
            }
            return Ok(Some(result));
        }
        Ok(None)
    }

    pub(crate) fn begin(&mut self, command_id: &str, request: &Command) -> Result<Begin, String> {
        request.validate()?;
        if !hex_id(command_id, 64) {
            return Err("invalid execution command ID".into());
        }
        if let Some(entry) = self.replay(command_id, request)? {
            return Ok(Begin::Replay(entry));
        }
        if self.journal.entries.len() >= 4096 {
            return Err("execution journal requires archival".into());
        }
        if let Some(current) = self
            .journal
            .current
            .as_ref()
            .and_then(|id| self.journal.entries.get(id))
        {
            if request.agent != current.request.agent || request.relay != current.request.relay {
                return Err("execution placement binding mismatch".into());
            }
            match &request.action {
                Action::Start { .. }
                    if current.outcome != Outcome::Stopped
                        && current.outcome != Outcome::Rejected =>
                {
                    return Err("previous execution is not proven stopped".into());
                }
                Action::Stop { run } if run != current.request.run() => {
                    return Err("stop generation does not match placement fence".into());
                }
                _ => {}
            }
        }
        self.journal.entries.insert(
            request.operation.clone(),
            Entry {
                command_id: command_id.into(),
                request: request.clone(),
                outcome: Outcome::Accepted,
                observed_at: nostr::Timestamp::now().as_secs(),
            },
        );
        self.journal.current = Some(request.operation.clone());
        self.save()?;
        Ok(Begin::Execute)
    }

    pub(crate) fn finish(&mut self, operation: &str, outcome: Outcome) -> Result<Entry, String> {
        if self.journal.current.as_deref() != Some(operation) {
            return Err("stale execution result".into());
        }
        let entry = self
            .journal
            .entries
            .get_mut(operation)
            .ok_or("unknown execution operation")?;
        // A terminal state cannot be overwritten by a late asynchronous result.
        if matches!(entry.outcome, Outcome::Stopped | Outcome::Rejected) && entry.outcome != outcome
        {
            return Err("execution result is terminal".into());
        }
        if outcome == Outcome::Rejected && entry.outcome != Outcome::Accepted {
            return Err("cannot reject after a possible side effect".into());
        }
        let allowed = match entry.request.action {
            Action::Start { .. } => matches!(
                outcome,
                Outcome::Spawned
                    | Outcome::Listening
                    | Outcome::Ready
                    | Outcome::Rejected
                    | Outcome::Unknown
            ),
            Action::Stop { .. } => matches!(
                outcome,
                Outcome::RootExited | Outcome::Stopped | Outcome::Unknown
            ),
        };
        if !allowed {
            return Err("invalid execution transition".into());
        }
        entry.outcome = outcome;
        entry.observed_at = nostr::Timestamp::now().as_secs();
        let result = entry.clone();
        self.save()?;
        Ok(result)
    }

    /// All legacy/config-driven starts must fail while a durable placement fence
    /// exists. The owning durable Start holds this lock through launch instead.
    pub(crate) fn is_fenced(&self) -> bool {
        self.journal.current.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(operation: &str, action: Action) -> Command {
        Command {
            v: 1,
            operation: operation.repeat(16),
            relay: "wss://relay.example".into(),
            agent: nostr::Keys::generate().public_key().to_hex(),
            expires_at: 200,
            action,
        }
    }
    fn start() -> Command {
        request(
            "aa",
            Action::Start {
                runtime: "goose".into(),
                revision: "bb".repeat(32),
            },
        )
    }
    #[test]
    fn crash_and_ack_loss_never_repeat_start() {
        let dir = tempfile::tempdir().unwrap();
        let req = start();
        let id = "cc".repeat(32);
        {
            let mut ledger = Ledger::open(dir.path(), "placement").unwrap();
            assert!(matches!(ledger.begin(&id, &req).unwrap(), Begin::Execute));
            assert!(Ledger::open(dir.path(), "placement").is_err());
        }
        let mut ledger = Ledger::open(dir.path(), "placement").unwrap();
        assert!(matches!(
            ledger.begin(&id, &req).unwrap(),
            Begin::Replay(Entry {
                outcome: Outcome::Unknown,
                ..
            })
        ));
        let mut replacement = req.clone();
        replacement.operation = "dd".repeat(16);
        assert!(ledger.begin(&"ee".repeat(32), &replacement).is_err());
        assert!(ledger.begin(&"ff".repeat(32), &req).is_err());
        ledger.finish(&req.operation, Outcome::Spawned).unwrap();
        assert!(matches!(
            ledger.begin(&id, &req).unwrap(),
            Begin::Replay(Entry {
                outcome: Outcome::Spawned,
                ..
            })
        ));
    }
    #[test]
    fn only_exact_confirmed_stop_allows_replacement_and_late_results_are_fenced() {
        let dir = tempfile::tempdir().unwrap();
        let req = start();
        let mut ledger = Ledger::open(dir.path(), "placement").unwrap();
        ledger.begin(&"cc".repeat(32), &req).unwrap();
        ledger.finish(&req.operation, Outcome::Spawned).unwrap();
        let mut stop = req.clone();
        stop.operation = "dd".repeat(16);
        stop.action = Action::Stop {
            run: "ee".repeat(16),
        };
        assert!(ledger.begin(&"ff".repeat(32), &stop).is_err());
        stop.action = Action::Stop {
            run: req.operation.clone(),
        };
        ledger.begin(&"ff".repeat(32), &stop).unwrap();
        ledger.finish(&stop.operation, Outcome::Unknown).unwrap();
        let mut next = req.clone();
        next.operation = "12".repeat(16);
        assert!(ledger.begin(&"34".repeat(32), &next).is_err());
        ledger.finish(&stop.operation, Outcome::RootExited).unwrap();
        assert!(
            ledger.begin(&"34".repeat(32), &next).is_err(),
            "root exit is not a stop certificate"
        );
        ledger.finish(&stop.operation, Outcome::Stopped).unwrap();
        assert!(ledger.is_fenced());
        ledger.begin(&"34".repeat(32), &next).unwrap();
        assert!(ledger.finish(&stop.operation, Outcome::Unknown).is_err());
        assert!(ledger.finish(&req.operation, Outcome::Ready).is_err());
    }
    #[test]
    fn corruption_and_traversal_fail_closed_and_ledger_contains_no_payload_secrets() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Ledger::open(dir.path(), "../escape").is_err());
        fs::write(dir.path().join("placement.json"), b"{").unwrap();
        assert!(Ledger::open(dir.path(), "placement").is_err());
        fs::write(
            dir.path().join("placement.json"),
            br#"{"entries":{},"current":"missing"}"#,
        )
        .unwrap();
        assert!(Ledger::open(dir.path(), "placement").is_err());
        let mut ledger = Ledger::open(dir.path(), "other").unwrap();
        ledger.begin(&"cc".repeat(32), &start()).unwrap();
        let text = fs::read_to_string(dir.path().join("other.json")).unwrap();
        assert!(!text.contains("private_key"));
        assert!(!text.contains("environment"));
    }
}
