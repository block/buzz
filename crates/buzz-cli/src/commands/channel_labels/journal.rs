//! Append-only, fsynced exact-event recovery. A file lock excludes concurrent sends.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use buzz_core::channel_labels::CommandOutcome;
use nostr::{Event, PublicKey};
use serde::{Deserialize, Serialize};

use crate::error::CliError;

const MAX_JOURNAL_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub(super) struct CommandRecord {
    pub version: u8,
    pub relay_url: String,
    pub relay_key: PublicKey,
    pub event: Event,
}

pub(super) struct Journal {
    file: File,
    pub record: CommandRecord,
    pub outcome: CommandOutcome,
}

fn storage(error: impl std::fmt::Display) -> CliError {
    CliError::Other(format!("command recovery journal: {error}"))
}

impl Journal {
    pub fn create(path: &Path, record: CommandRecord) -> Result<Self, CliError> {
        let mut options = OpenOptions::new();
        options.read(true).append(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(storage)?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(storage)?;
        let mut journal = Self {
            file,
            record,
            outcome: CommandOutcome::Prepared,
        };
        journal.append(&serde_json::to_vec(&journal.record).map_err(storage)?)?;
        // Persist the directory entry before transport. Never create parents or
        // overwrite an existing file: the caller chooses the recovery location.
        #[cfg(unix)]
        File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )
        .and_then(|dir| dir.sync_all())
        .map_err(storage)?;
        Ok(journal)
    }

    pub fn open(path: &Path) -> Result<Self, CliError> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(path)
            .map_err(storage)?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(storage)?;
        if file.metadata().map_err(storage)?.len() > MAX_JOURNAL_BYTES {
            return Err(storage("file exceeds 1 MiB"));
        }
        file.seek(SeekFrom::Start(0)).map_err(storage)?;
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_JOURNAL_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(storage)?;
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(storage("file exceeds 1 MiB"));
        }
        let complete = bytes
            .iter()
            .rposition(|b| *b == b'\n')
            .ok_or_else(|| storage("incomplete command record; no recoverable signed event"))?;
        let mut lines = bytes[..complete].split(|b| *b == b'\n');
        let record: CommandRecord =
            serde_json::from_slice(lines.next().ok_or_else(|| storage("empty journal"))?)
                .map_err(storage)?;
        if record.version != 1 {
            return Err(storage("unsupported version"));
        }
        let mut outcome = CommandOutcome::Prepared;
        for line in lines {
            let next: CommandOutcome = serde_json::from_slice(line).map_err(storage)?;
            // The pre-send Unknown entry remains authoritative if the process
            // crashes before the next complete acknowledgement record.
            if outcome == CommandOutcome::Committed && next != outcome {
                return Err(storage("committed outcome cannot be downgraded"));
            }
            outcome = next;
        }
        let mut journal = Self {
            file,
            record,
            outcome,
        };
        if complete + 1 != bytes.len() {
            // Only the final partial append is disposable. The complete signed
            // event is immutable. A torn acknowledgement is not proof of rejection.
            journal
                .file
                .set_len((complete + 1) as u64)
                .map_err(storage)?;
            journal.file.sync_all().map_err(storage)?;
            journal.persist(outcome.before_send())?;
        }
        Ok(journal)
    }

    pub fn persist(&mut self, outcome: CommandOutcome) -> Result<(), CliError> {
        self.append(&serde_json::to_vec(&outcome).map_err(storage)?)?;
        self.outcome = outcome;
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> Result<(), CliError> {
        if self.file.metadata().map_err(storage)?.len() + bytes.len() as u64 + 1 > MAX_JOURNAL_BYTES
        {
            return Err(storage("retry journal is full; no request was sent"));
        }
        self.file
            .write_all(bytes)
            .and_then(|_| self.file.write_all(b"\n"))
            .and_then(|_| self.file.sync_all())
            .map_err(storage)
    }
}
