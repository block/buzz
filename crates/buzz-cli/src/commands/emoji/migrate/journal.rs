//! Append-only migration journal.
//!
//! Every state change is one JSON line. [`Journal::record`] writes the line
//! and calls `sync` before it returns, and the pipeline takes its next action
//! only after that, so a crash at any point replays to a state whose next
//! action is safe to repeat. The journal is the only source of truth: replay
//! rebuilds everything from it, and snapshots are for the operator.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::names::RejectReason;

/// Journal format version.
pub const JOURNAL_VERSION: u32 = 1;

/// A pipeline stage that can fail and be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Fetch,
    Upload,
    Publish,
    Verify,
}

impl Stage {
    fn index(self) -> usize {
        match self {
            Stage::Fetch => 0,
            Stage::Upload => 1,
            Stage::Publish => 2,
            Stage::Verify => 3,
        }
    }
}

/// One journal line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "rec", rename_all = "snake_case")]
pub enum Record {
    Header {
        version: u32,
        plan_sha256: String,
        input_sha256: String,
    },
    Planned {
        name: String,
    },
    Fetched {
        name: String,
        sha: String,
    },
    Sanitized {
        name: String,
        sha: String,
        ext: String,
        size: u64,
    },
    Uploaded {
        name: String,
        sha: String,
        url: String,
    },
    Published {
        name: String,
        event_id: String,
    },
    Verified {
        name: String,
    },
    /// Rollback deleted the published entry; the next run publishes it again.
    Unpublished {
        name: String,
    },
    Rejected {
        name: String,
        reason: RejectReason,
        detail: String,
    },
    Failed {
        name: String,
        stage: Stage,
        error: String,
    },
}

impl Record {
    fn name(&self) -> Option<&str> {
        match self {
            Record::Header { .. } => None,
            Record::Planned { name }
            | Record::Fetched { name, .. }
            | Record::Sanitized { name, .. }
            | Record::Uploaded { name, .. }
            | Record::Published { name, .. }
            | Record::Verified { name }
            | Record::Unpublished { name }
            | Record::Rejected { name, .. }
            | Record::Failed { name, .. } => Some(name),
        }
    }
}

/// How far an item got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Progress {
    Planned,
    Fetched,
    Sanitized,
    Uploaded,
    Published,
    Verified,
    Rejected,
}

/// Replayed state of one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemState {
    pub progress: Progress,
    pub raw_sha: Option<String>,
    pub sha: Option<String>,
    pub ext: Option<String>,
    pub size: Option<u64>,
    pub url: Option<String>,
    pub event_id: Option<String>,
    pub reject: Option<(RejectReason, String)>,
    failures: [u32; 4],
}

impl ItemState {
    fn planned() -> Self {
        Self {
            progress: Progress::Planned,
            raw_sha: None,
            sha: None,
            ext: None,
            size: None,
            url: None,
            event_id: None,
            reject: None,
            failures: [0; 4],
        }
    }

    /// Failed attempts recorded for `stage`.
    pub fn failures(&self, stage: Stage) -> u32 {
        self.failures[stage.index()]
    }
}

/// Everything replay rebuilds from the journal.
#[derive(Debug, Default, Clone)]
pub struct JournalState {
    pub header: Option<(String, String)>,
    pub items: BTreeMap<String, ItemState>,
    /// Uploaded blob URL for each sanitized sha.
    pub blobs: HashMap<String, String>,
    pub records: u64,
}

fn corrupt(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl JournalState {
    /// Validate `rec` against the current state without applying it.
    fn check(&self, rec: &Record) -> io::Result<()> {
        let Some(name) = rec.name() else {
            return if self.header.is_none() && self.records == 0 {
                Ok(())
            } else {
                Err(corrupt("header must be the first record"))
            };
        };
        if self.header.is_none() {
            return Err(corrupt("journal has no header"));
        }
        let cur = match (self.items.get(name), rec) {
            (None, Record::Planned { .. }) => return Ok(()),
            (None, _) => return Err(corrupt(format!("record for unplanned name '{name}'"))),
            (Some(_), Record::Planned { .. }) => {
                return Err(corrupt(format!("'{name}' planned twice")))
            }
            (Some(cur), _) => cur,
        };
        if cur.progress == Progress::Rejected {
            return Err(corrupt(format!("record after rejection for '{name}'")));
        }
        let ok = match rec {
            Record::Fetched { .. } => cur.progress <= Progress::Sanitized,
            Record::Sanitized { sha, .. } => {
                matches!(cur.progress, Progress::Fetched | Progress::Sanitized) && !sha.is_empty()
            }
            Record::Uploaded { sha, .. } => {
                cur.progress == Progress::Sanitized && cur.sha.as_deref() == Some(sha.as_str())
            }
            Record::Published { .. } => cur.progress == Progress::Uploaded,
            Record::Verified { .. } => {
                matches!(cur.progress, Progress::Published | Progress::Verified)
            }
            Record::Unpublished { .. } => {
                matches!(cur.progress, Progress::Published | Progress::Verified)
            }
            Record::Rejected { .. } | Record::Failed { .. } => true,
            Record::Header { .. } | Record::Planned { .. } => false,
        };
        if ok {
            Ok(())
        } else {
            Err(corrupt(format!(
                "record {rec:?} is out of order for '{name}' at {:?}",
                cur.progress
            )))
        }
    }

    fn apply(&mut self, rec: &Record) {
        self.records += 1;
        let Some(name) = rec.name() else {
            if let Record::Header {
                plan_sha256,
                input_sha256,
                ..
            } = rec
            {
                self.header = Some((plan_sha256.clone(), input_sha256.clone()));
            }
            return;
        };
        let item = self
            .items
            .entry(name.to_string())
            .or_insert_with(ItemState::planned);
        match rec {
            Record::Fetched { sha, .. } => {
                item.progress = Progress::Fetched;
                item.raw_sha = Some(sha.clone());
            }
            Record::Sanitized { sha, ext, size, .. } => {
                item.progress = Progress::Sanitized;
                item.sha = Some(sha.clone());
                item.ext = Some(ext.clone());
                item.size = Some(*size);
            }
            Record::Uploaded { sha, url, .. } => {
                item.progress = Progress::Uploaded;
                item.url = Some(url.clone());
                self.blobs.insert(sha.clone(), url.clone());
            }
            Record::Published { event_id, .. } => {
                item.progress = Progress::Published;
                item.event_id = Some(event_id.clone());
            }
            Record::Verified { .. } => item.progress = Progress::Verified,
            Record::Unpublished { .. } => {
                item.progress = Progress::Uploaded;
                item.event_id = None;
            }
            Record::Rejected { reason, detail, .. } => {
                item.progress = Progress::Rejected;
                item.reject = Some((*reason, detail.clone()));
            }
            Record::Failed { stage, .. } => item.failures[stage.index()] += 1,
            Record::Header { .. } | Record::Planned { .. } => {}
        }
    }
}

/// Replay `bytes`. Returns the state and the length of the valid prefix: a
/// final line with no newline is a torn write and is not part of it.
pub fn replay(bytes: &[u8]) -> io::Result<(JournalState, usize)> {
    let valid = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let mut state = JournalState::default();
    for (n, line) in bytes[..valid].split(|&b| b == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let rec: Record = serde_json::from_slice(line)
            .map_err(|e| corrupt(format!("journal line {} is not a record: {e}", n + 1)))?;
        state.check(&rec)?;
        state.apply(&rec);
    }
    Ok((state, valid))
}

/// Where journal lines go. `sync` must make every appended line durable.
pub trait JournalStorage: Send {
    fn append(&mut self, line: &[u8]) -> io::Result<()>;
    fn sync(&mut self) -> io::Result<()>;
}

/// File-backed storage opened for append.
pub struct FileStorage {
    file: std::fs::File,
}

impl JournalStorage for FileStorage {
    fn append(&mut self, line: &[u8]) -> io::Result<()> {
        self.file.write_all(line)
    }

    fn sync(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }
}

/// An open journal. Not shared: wrap it in a lock to use it from tasks.
pub struct Journal<S: JournalStorage> {
    storage: S,
    state: JournalState,
}

impl<S: JournalStorage> Journal<S> {
    pub fn new(storage: S, state: JournalState) -> Self {
        Self { storage, state }
    }

    pub fn state(&self) -> &JournalState {
        &self.state
    }

    /// Append `rec` and sync it. The in-memory state changes only after the
    /// sync succeeds, so a failed write never looks recorded.
    pub fn record(&mut self, rec: Record) -> io::Result<()> {
        self.record_batch(std::slice::from_ref(&rec))
    }

    /// Append several records and sync once. Use only for records that need
    /// no action between them (the initial `planned` lines).
    pub fn record_batch(&mut self, recs: &[Record]) -> io::Result<()> {
        let mut scratch = self.state.clone();
        let mut buf = Vec::new();
        for rec in recs {
            scratch.check(rec)?;
            scratch.apply(rec);
            serde_json::to_writer(&mut buf, rec).map_err(io::Error::other)?;
            buf.push(b'\n');
        }
        self.storage.append(&buf)?;
        self.storage.sync()?;
        self.state = scratch;
        Ok(())
    }
}

/// Counts for `snapshot.json`.
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub plan_sha256: String,
    pub records: u64,
    pub unique_blobs: u64,
    pub by_state: BTreeMap<String, u64>,
}

impl JournalState {
    pub fn snapshot(&self) -> Snapshot {
        let mut by_state: BTreeMap<String, u64> = BTreeMap::new();
        for item in self.items.values() {
            *by_state
                .entry(format!("{:?}", item.progress).to_lowercase())
                .or_default() += 1;
        }
        Snapshot {
            plan_sha256: self
                .header
                .as_ref()
                .map(|h| h.0.clone())
                .unwrap_or_default(),
            records: self.records,
            unique_blobs: self.blobs.len() as u64,
            by_state,
        }
    }
}

/// Write `bytes` to `path` through a temp file in the same directory, then
/// rename it over `path`, so a reader sees the old or the new file, never half.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    let dir = dir.unwrap_or_else(|| Path::new("."));
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}.{unique}.tmp", std::process::id()));
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)?;
    std::fs::File::open(dir)?.sync_all()
}

/// What `open_file` found.
pub struct Opened {
    pub journal: Journal<FileStorage>,
    pub resumed: bool,
}

/// Open or create the journal at `path` for a plan.
///
/// A journal whose header names another plan or input is refused unless
/// `force_new_plan`, which moves it aside (never deletes it) and starts fresh.
/// A torn final line is cut off. Plan names with no `planned` record get one.
pub fn open_file(
    path: &Path,
    plan_sha256: &str,
    input_sha256: &str,
    names: &[String],
    force_new_plan: bool,
) -> io::Result<Opened> {
    let mut resumed = false;
    let mut state = JournalState::default();
    if path.exists() {
        let bytes = std::fs::read(path)?;
        let (replayed, valid) = replay(&bytes)?;
        let same = replayed
            .header
            .as_ref()
            .is_none_or(|(p, i)| p == plan_sha256 && i == input_sha256);
        if !same {
            if !force_new_plan {
                return Err(corrupt(format!(
                    "{} belongs to a different plan or input; re-run with --force-new-plan to move it aside",
                    path.display()
                )));
            }
            let mut aside = path.to_path_buf();
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            aside.set_extension(format!("{stamp}.jsonl.old"));
            std::fs::rename(path, &aside)?;
        } else {
            if valid < bytes.len() {
                let f = std::fs::OpenOptions::new().write(true).open(path)?;
                f.set_len(valid as u64)?;
                f.sync_all()?;
            }
            resumed = replayed.header.is_some();
            state = replayed;
        }
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut journal = Journal::new(FileStorage { file }, state);
    init_records(&mut journal, plan_sha256, input_sha256, names)?;
    Ok(Opened { journal, resumed })
}

/// Append the header and any missing `planned` records in one synced batch.
pub fn init_records<S: JournalStorage>(
    journal: &mut Journal<S>,
    plan_sha256: &str,
    input_sha256: &str,
    names: &[String],
) -> io::Result<()> {
    let mut recs = Vec::new();
    if journal.state.header.is_none() {
        recs.push(Record::Header {
            version: JOURNAL_VERSION,
            plan_sha256: plan_sha256.to_string(),
            input_sha256: input_sha256.to_string(),
        });
    }
    let planned: HashSet<&str> = journal.state.items.keys().map(String::as_str).collect();
    for name in names {
        if !planned.contains(name.as_str()) {
            recs.push(Record::Planned { name: name.clone() });
        }
    }
    if recs.is_empty() {
        return Ok(());
    }
    journal.record_batch(&recs)
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Shared ordered log of journal appends, syncs and port actions.
    pub type EventLog = Arc<Mutex<Vec<String>>>;

    /// In-memory storage that logs `append:<line>` and `sync`.
    pub struct MemStorage {
        pub bytes: Arc<Mutex<Vec<u8>>>,
        pub log: EventLog,
    }

    impl MemStorage {
        pub fn new(log: EventLog) -> (Self, Arc<Mutex<Vec<u8>>>) {
            let bytes = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    bytes: bytes.clone(),
                    log,
                },
                bytes,
            )
        }
    }

    impl JournalStorage for MemStorage {
        fn append(&mut self, line: &[u8]) -> io::Result<()> {
            self.bytes.lock().unwrap().extend_from_slice(line);
            let text = String::from_utf8_lossy(line).trim_end().to_string();
            self.log.lock().unwrap().push(format!("append:{text}"));
            Ok(())
        }

        fn sync(&mut self) -> io::Result<()> {
            self.log.lock().unwrap().push("sync".into());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn mem_journal() -> (Journal<MemStorage>, Arc<Mutex<Vec<u8>>>, EventLog) {
        let log = EventLog::default();
        let (storage, bytes) = MemStorage::new(log.clone());
        (Journal::new(storage, JournalState::default()), bytes, log)
    }

    use std::sync::{Arc, Mutex};

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    fn full_item(j: &mut Journal<MemStorage>, name: &str) {
        j.record(Record::Fetched {
            name: name.into(),
            sha: "raw".into(),
        })
        .unwrap();
        j.record(Record::Sanitized {
            name: name.into(),
            sha: "abc".into(),
            ext: "png".into(),
            size: 9,
        })
        .unwrap();
        j.record(Record::Uploaded {
            name: name.into(),
            sha: "abc".into(),
            url: "https://r/abc.png".into(),
        })
        .unwrap();
        j.record(Record::Published {
            name: name.into(),
            event_id: "ev1".into(),
        })
        .unwrap();
        j.record(Record::Verified { name: name.into() }).unwrap();
    }

    #[test]
    fn every_record_is_synced_before_record_returns() {
        let (mut j, _, log) = mem_journal();
        init_records(&mut j, "plan", "input", &names(&["a"])).unwrap();
        j.record(Record::Fetched {
            name: "a".into(),
            sha: "raw".into(),
        })
        .unwrap();
        let log = log.lock().unwrap().clone();
        assert_eq!(log.len(), 4);
        assert!(log[0].starts_with("append:") && log[1] == "sync");
        assert!(log[2].starts_with("append:") && log[3] == "sync");
    }

    #[test]
    fn replay_rebuilds_state_and_blob_index() {
        let (mut j, bytes, _) = mem_journal();
        init_records(&mut j, "plan", "input", &names(&["a", "b"])).unwrap();
        full_item(&mut j, "a");
        j.record(Record::Failed {
            name: "b".into(),
            stage: Stage::Fetch,
            error: "boom".into(),
        })
        .unwrap();
        j.record(Record::Rejected {
            name: "b".into(),
            reason: RejectReason::RetriesExhausted,
            detail: "x".into(),
        })
        .unwrap();
        let (state, valid) = replay(&bytes.lock().unwrap()).unwrap();
        assert_eq!(valid, bytes.lock().unwrap().len());
        assert_eq!(state.header, Some(("plan".into(), "input".into())));
        assert_eq!(state.items["a"].progress, Progress::Verified);
        assert_eq!(state.items["a"].event_id.as_deref(), Some("ev1"));
        assert_eq!(state.items["b"].progress, Progress::Rejected);
        assert_eq!(state.items["b"].failures(Stage::Fetch), 1);
        assert_eq!(state.blobs["abc"], "https://r/abc.png");
    }

    #[test]
    fn torn_final_line_is_ignored_but_a_bad_inner_line_is_an_error() {
        let (mut j, bytes, _) = mem_journal();
        init_records(&mut j, "plan", "input", &names(&["a"])).unwrap();
        let mut torn = bytes.lock().unwrap().clone();
        let intact = torn.len();
        torn.extend_from_slice(br#"{"rec":"fetched","name":"a","sh"#);
        let (state, valid) = replay(&torn).unwrap();
        assert_eq!(valid, intact);
        assert_eq!(state.items["a"].progress, Progress::Planned);

        let mut bad = bytes.lock().unwrap().clone();
        bad.extend_from_slice(b"not json\n");
        bad.extend_from_slice(br#"{"rec":"verified","name":"a"}"#);
        bad.push(b'\n');
        assert!(replay(&bad).is_err());
    }

    #[test]
    fn out_of_order_and_unknown_records_are_refused() {
        let (mut j, _, _) = mem_journal();
        init_records(&mut j, "plan", "input", &names(&["a"])).unwrap();
        let bad = [
            Record::Published {
                name: "a".into(),
                event_id: "e".into(),
            },
            Record::Uploaded {
                name: "a".into(),
                sha: "s".into(),
                url: "u".into(),
            },
            Record::Fetched {
                name: "zzz".into(),
                sha: "s".into(),
            },
            Record::Planned { name: "a".into() },
        ];
        for rec in bad {
            assert!(j.record(rec).is_err());
        }
        j.record(Record::Fetched {
            name: "a".into(),
            sha: "raw".into(),
        })
        .unwrap();
        j.record(Record::Fetched {
            name: "a".into(),
            sha: "raw".into(),
        })
        .unwrap();
        assert!(
            j.record(Record::Uploaded {
                name: "a".into(),
                sha: "s".into(),
                url: "u".into()
            })
            .is_err(),
            "uploaded needs a sanitized record first"
        );
        j.record(Record::Rejected {
            name: "a".into(),
            reason: RejectReason::InvalidImage,
            detail: String::new(),
        })
        .unwrap();
        assert!(j.record(Record::Verified { name: "a".into() }).is_err());
    }

    #[test]
    fn unpublished_returns_an_item_to_uploaded() {
        let (mut j, _, _) = mem_journal();
        init_records(&mut j, "plan", "input", &names(&["a"])).unwrap();
        full_item(&mut j, "a");
        j.record(Record::Unpublished { name: "a".into() }).unwrap();
        let item = &j.state().items["a"];
        assert_eq!(item.progress, Progress::Uploaded);
        assert_eq!(item.event_id, None);
        j.record(Record::Published {
            name: "a".into(),
            event_id: "ev2".into(),
        })
        .unwrap();
    }

    #[test]
    fn open_file_resumes_cuts_torn_tail_and_refuses_a_changed_plan() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let n = names(&["a", "b"]);
        {
            let mut o = open_file(&path, "plan1", "in1", &n, false).unwrap();
            assert!(!o.resumed);
            o.journal
                .record(Record::Fetched {
                    name: "a".into(),
                    sha: "raw".into(),
                })
                .unwrap();
        }
        let mut raw = std::fs::read(&path).unwrap();
        raw.extend_from_slice(b"{\"rec\":\"fetch");
        std::fs::write(&path, &raw).unwrap();

        let mut o = open_file(&path, "plan1", "in1", &n, false).unwrap();
        assert!(o.resumed);
        assert_eq!(o.journal.state().items["a"].progress, Progress::Fetched);
        o.journal
            .record(Record::Sanitized {
                name: "a".into(),
                sha: "s".into(),
                ext: "png".into(),
                size: 1,
            })
            .unwrap();
        drop(o);
        assert!(
            replay(&std::fs::read(&path).unwrap()).is_ok(),
            "append after cut stays parseable"
        );

        for (plan, input) in [("plan2", "in1"), ("plan1", "in2")] {
            let err = open_file(&path, plan, input, &n, false).err().unwrap();
            assert!(err.to_string().contains("--force-new-plan"), "{err}");
        }
        let before = std::fs::read(&path).unwrap();
        let o = open_file(&path, "plan2", "in1", &n, true).unwrap();
        assert!(!o.resumed);
        let moved: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl.old"))
            .collect();
        assert_eq!(moved.len(), 1);
        assert_eq!(
            std::fs::read(moved[0].path()).unwrap(),
            before,
            "old journal kept intact"
        );
    }

    #[test]
    fn open_file_adds_planned_records_missing_after_a_crash_during_init() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        open_file(&path, "p", "i", &names(&["a"]), false).unwrap();
        let o = open_file(&path, "p", "i", &names(&["a", "b"]), false).unwrap();
        assert!(o.journal.state().items.contains_key("b"));
    }

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.json");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
