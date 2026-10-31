//! Pipeline, plan and resume tests. Only the relay, the uploader and HTTP
//! fetch are faked; the journal, buckets, sanitizer and pipeline are real.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::bucket::testing::FakeClock;
use super::bucket::Clock;
use super::journal::testing::{EventLog, MemStorage};
use super::journal::{init_records, replay, Journal, Progress, Record, Stage};
use super::names::{PlanItem, RejectReason};
use super::pipeline::{measure, Migrator, Rates, RunConfig};
use super::plan::sha256_hex;
use super::ports::{CatalogRow, Fetcher, PortError, Relay, UploadedBlob, Uploader};
use super::*;

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut c = (data.len() as u32).to_be_bytes().to_vec();
    c.extend_from_slice(kind);
    c.extend_from_slice(data);
    c.extend_from_slice(&[0; 4]);
    c
}

/// A PNG the relay's validator reads as 4x4. `seed` picks the pixel payload;
/// `text` adds a tEXt chunk the validator rejects.
fn png(seed: u8, text: Option<&str>) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(png_chunk(b"IHDR", &[0, 0, 0, 4, 0, 0, 0, 4, 8, 6, 0, 0, 0]));
    if let Some(t) = text {
        out.extend(png_chunk(b"tEXt", format!("Comment\0{t}").as_bytes()));
    }
    out.extend(png_chunk(b"IDAT", &[seed; 8]));
    out.extend(png_chunk(b"IEND", &[]));
    out
}

fn item(name: &str, source: &str) -> PlanItem {
    PlanItem {
        name: name.into(),
        source: source.into(),
        alias_of: None,
    }
}

struct FakeFetcher {
    log: EventLog,
    images: HashMap<String, Vec<u8>>,
    script: Mutex<HashMap<String, VecDeque<PortError>>>,
    calls: Mutex<HashMap<String, u32>>,
}

impl FakeFetcher {
    fn calls(&self, source: &str) -> u32 {
        *self.calls.lock().unwrap().get(source).unwrap_or(&0)
    }

    fn total_calls(&self) -> u32 {
        self.calls.lock().unwrap().values().sum()
    }
}

impl Fetcher for FakeFetcher {
    async fn fetch(&self, source: &str) -> Result<Vec<u8>, PortError> {
        self.log.lock().unwrap().push(format!("fetch:{source}"));
        *self.calls.lock().unwrap().entry(source.into()).or_default() += 1;
        if let Some(e) = self
            .script
            .lock()
            .unwrap()
            .get_mut(source)
            .and_then(VecDeque::pop_front)
        {
            return Err(e);
        }
        self.images
            .get(source)
            .cloned()
            .ok_or_else(|| PortError::Permanent("HTTP 404".into()))
    }
}

struct FakeUploader {
    log: EventLog,
    clock: FakeClock,
    script: Mutex<VecDeque<PortError>>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    stored: Mutex<BTreeMap<String, Vec<u8>>>,
    attempts_at: Mutex<Vec<(String, Duration)>>,
    all_valid: Mutex<bool>,
    wrong_hash: AtomicBool,
}

impl FakeUploader {
    fn uploaded_shas(&self) -> Vec<String> {
        self.stored.lock().unwrap().keys().cloned().collect()
    }
}

impl Uploader for FakeUploader {
    async fn upload(&self, bytes: Vec<u8>) -> Result<UploadedBlob, PortError> {
        let sha = sha256_hex(&bytes);
        self.log.lock().unwrap().push(format!("upload:{sha}"));
        self.attempts_at
            .lock()
            .unwrap()
            .push((sha.clone(), self.clock.now()));
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        if let Some(e) = self.script.lock().unwrap().pop_front() {
            return Err(e);
        }
        let valid = buzz_media::validation::validate_content(
            &bytes,
            &buzz_media::MediaConfig::offline_validation(),
        )
        .is_ok();
        let mut all = self.all_valid.lock().unwrap();
        *all = *all && valid;
        self.stored.lock().unwrap().insert(sha.clone(), bytes);
        let reported = if self.wrong_hash.load(Ordering::SeqCst) {
            "0".repeat(64)
        } else {
            sha.clone()
        };
        Ok(UploadedBlob {
            url: format!("https://relay.test/media/{sha}.png"),
            sha256: reported,
        })
    }
}

#[derive(Default)]
struct RelayState {
    store: BTreeMap<String, CatalogRow>,
    publish_script: VecDeque<PortError>,
    delete_fail: HashSet<String>,
    missing_blobs: HashSet<String>,
    published: Vec<String>,
    deleted: Vec<String>,
    counter: u32,
}

struct FakeRelay {
    log: EventLog,
    state: Mutex<RelayState>,
    writes: AtomicU32,
    lookups: AtomicU32,
}

impl FakeRelay {
    fn put(&self, name: &str, url: &str, author: &str) {
        self.state.lock().unwrap().store.insert(
            name.into(),
            CatalogRow {
                name: name.into(),
                url: url.into(),
                author: author.into(),
                event_id: format!("seed-{name}"),
                other_authors: vec![],
            },
        );
    }

    fn names(&self) -> Vec<String> {
        self.state.lock().unwrap().store.keys().cloned().collect()
    }
}

impl Relay for FakeRelay {
    fn pubkey(&self) -> String {
        "me".into()
    }

    async fn publish(&self, name: &str, url: &str) -> Result<String, PortError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.log.lock().unwrap().push(format!("publish:{name}"));
        let mut st = self.state.lock().unwrap();
        if let Some(e) = st.publish_script.pop_front() {
            return Err(e);
        }
        st.counter += 1;
        let event_id = format!("ev{}", st.counter);
        st.store.insert(
            name.into(),
            CatalogRow {
                name: name.into(),
                url: url.into(),
                author: "me".into(),
                event_id: event_id.clone(),
                other_authors: vec![],
            },
        );
        st.published.push(name.into());
        Ok(event_id)
    }

    async fn lookup(&self, names: &[String]) -> Result<Vec<CatalogRow>, PortError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        assert!(names.len() <= 50, "lookups are batched at 50 names");
        let st = self.state.lock().unwrap();
        Ok(names
            .iter()
            .filter_map(|n| st.store.get(n).cloned())
            .collect())
    }

    async fn search(&self, prefix: &str, limit: u32) -> Result<Vec<CatalogRow>, PortError> {
        let want = prefix.replace(['_', '-'], " ");
        let st = self.state.lock().unwrap();
        Ok(st
            .store
            .values()
            .filter(|r| r.name.replace(['_', '-'], " ").starts_with(&want))
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn delete(&self, name: &str) -> Result<(), PortError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.log.lock().unwrap().push(format!("delete:{name}"));
        let mut st = self.state.lock().unwrap();
        if st.delete_fail.contains(name) {
            return Err(PortError::Permanent("relay refused the delete".into()));
        }
        st.store.remove(name);
        st.deleted.push(name.into());
        Ok(())
    }

    async fn blob_exists(&self, url: &str) -> Result<bool, PortError> {
        Ok(!self.state.lock().unwrap().missing_blobs.contains(url))
    }
}

struct World {
    log: EventLog,
    clock: FakeClock,
    fetcher: FakeFetcher,
    uploader: FakeUploader,
    relay: FakeRelay,
    dir: tempfile::TempDir,
}

impl World {
    fn new(images: &[(&str, Vec<u8>)]) -> Self {
        let log = EventLog::default();
        let clock = FakeClock::default();
        World {
            fetcher: FakeFetcher {
                log: log.clone(),
                images: images
                    .iter()
                    .map(|(s, b)| (s.to_string(), b.clone()))
                    .collect(),
                script: Mutex::default(),
                calls: Mutex::default(),
            },
            uploader: FakeUploader {
                log: log.clone(),
                clock: clock.clone(),
                script: Mutex::default(),
                in_flight: AtomicUsize::new(0),
                max_in_flight: AtomicUsize::new(0),
                stored: Mutex::default(),
                attempts_at: Mutex::default(),
                all_valid: Mutex::new(true),
                wrong_hash: AtomicBool::new(false),
            },
            relay: FakeRelay {
                log: log.clone(),
                state: Mutex::default(),
                writes: AtomicU32::new(0),
                lookups: AtomicU32::new(0),
            },
            log,
            clock,
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn blob_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("blobs")
    }

    fn journal(
        &self,
        prefix: &[u8],
        names: &[String],
    ) -> (Journal<MemStorage>, Arc<Mutex<Vec<u8>>>) {
        let (valid_state, valid_len) = replay(prefix).unwrap();
        let storage = MemStorage {
            bytes: Arc::new(Mutex::new(prefix[..valid_len].to_vec())),
            log: self.log.clone(),
        };
        let bytes = storage.bytes.clone();
        let mut j = Journal::new(storage, valid_state);
        init_records(&mut j, "plan", "input", names).unwrap();
        (j, bytes)
    }

    fn migrator(
        &self,
        journal: Journal<MemStorage>,
        rates: Rates,
    ) -> Migrator<'_, FakeFetcher, FakeUploader, FakeRelay, FakeClock, MemStorage> {
        Migrator::new(
            &self.fetcher,
            &self.uploader,
            &self.relay,
            self.clock.clone(),
            journal,
            rates,
            RunConfig {
                concurrency: 8,
                upload_concurrency: 2,
                max_attempts: 3,
                blob_dir: self.blob_dir(),
                snapshot_path: Some(self.dir.path().join("snapshot.json")),
            },
        )
    }
}

fn fast() -> Rates {
    Rates {
        uploads_per_minute: 60_000,
        events_per_minute: 60_000,
        queries_per_minute: 60_000,
    }
}

fn names_of(items: &[PlanItem]) -> Vec<String> {
    items.iter().map(|i| i.name.clone()).collect()
}

fn progress_map(bytes: &[u8]) -> BTreeMap<String, (Progress, Option<String>, Option<String>)> {
    let (state, _) = replay(bytes).unwrap();
    state
        .items
        .into_iter()
        .map(|(n, st)| (n, (st.progress, st.sha, st.url)))
        .collect()
}

fn catalog() -> (Vec<PlanItem>, Vec<(&'static str, Vec<u8>)>) {
    let images = vec![
        ("https://img/1.png", png(1, Some("secret exif-ish"))),
        ("https://img/2.png", png(2, None)),
        (
            "https://img/2b.png",
            png(2, Some("different metadata, same pixels")),
        ),
        ("https://img/bad.png", b"<html>not an image</html>".to_vec()),
    ];
    let items = vec![
        item("alpha", "https://img/1.png"),
        item("alpha_alias", "https://img/1.png"),
        item("beta", "https://img/2.png"),
        item("beta_twin", "https://img/2b.png"),
        item("broken", "https://img/bad.png"),
        item("gone", "https://img/missing.png"),
    ];
    (items, images)
}

#[tokio::test]
async fn run_dedupes_by_sanitized_hash_strips_metadata_and_publishes_each_name() {
    let (items, images) = catalog();
    let w = World::new(&images);
    let (j, bytes) = w.journal(b"", &names_of(&items));
    let summary = w.migrator(j, fast()).run(&items).await.unwrap();

    assert_eq!(
        w.uploader.uploaded_shas().len(),
        2,
        "four good names, two distinct blobs"
    );
    assert!(
        *w.uploader.all_valid.lock().unwrap(),
        "uploaded bytes pass the relay validator"
    );
    assert_eq!(summary.published_now, 4);
    assert_eq!(summary.unique_blobs, 2);
    let state = progress_map(&bytes.lock().unwrap());
    for name in ["alpha", "alpha_alias", "beta", "beta_twin"] {
        assert_eq!(state[name].0, Progress::Published, "{name}");
    }
    assert_eq!(
        state["alpha"].2, state["alpha_alias"].2,
        "aliases share one blob url"
    );
    assert_eq!(
        state["beta"].1, state["beta_twin"].1,
        "metadata-only differences dedupe"
    );
    assert_eq!(state["broken"].0, Progress::Rejected);
    assert_eq!(state["gone"].0, Progress::Rejected);
    let (journal, _) = replay(&bytes.lock().unwrap()).unwrap();
    assert_eq!(
        journal.items["broken"].reject.as_ref().unwrap().0,
        RejectReason::InvalidImage
    );
    assert_eq!(
        journal.items["gone"].reject.as_ref().unwrap().0,
        RejectReason::FetchFailed
    );
    assert_eq!(
        w.relay.names(),
        ["alpha", "alpha_alias", "beta", "beta_twin"]
    );
    assert!(w.dir.path().join("snapshot.json").exists());
    assert_eq!(
        std::fs::read_dir(w.blob_dir()).map_or(0, |d| d.count()),
        0,
        "cache swept"
    );
}

#[tokio::test]
async fn transient_failures_back_off_and_end_in_rejected_after_the_attempt_limit() {
    let w = World::new(&[("https://img/x.png", png(9, None))]);
    w.fetcher.script.lock().unwrap().insert(
        "https://img/x.png".into(),
        (0..5).map(|_| PortError::transient("HTTP 503")).collect(),
    );
    let items = vec![item("x", "https://img/x.png")];
    let (j, bytes) = w.journal(b"", &names_of(&items));
    let before = w.clock.now();
    let summary = w.migrator(j, fast()).run(&items).await.unwrap();

    assert_eq!(
        w.fetcher.calls("https://img/x.png"),
        3,
        "bounded by max_attempts"
    );
    assert_eq!(
        w.clock.now() - before,
        Duration::from_secs(3),
        "1s then 2s backoff"
    );
    assert_eq!(summary.published_now, 0);
    let (state, _) = replay(&bytes.lock().unwrap()).unwrap();
    assert_eq!(state.items["x"].failures(Stage::Fetch), 3);
    assert_eq!(
        state.items["x"].reject.as_ref().unwrap().0,
        RejectReason::RetriesExhausted
    );
}

#[tokio::test]
async fn failures_recorded_before_a_restart_count_toward_the_limit() {
    let w = World::new(&[("https://img/x.png", png(9, None))]);
    w.fetcher.script.lock().unwrap().insert(
        "https://img/x.png".into(),
        VecDeque::from([PortError::transient("HTTP 503")]),
    );
    let items = vec![item("x", "https://img/x.png")];
    let (mut j, bytes) = w.journal(b"", &names_of(&items));
    for _ in 0..2 {
        j.record(Record::Failed {
            name: "x".into(),
            stage: Stage::Fetch,
            error: "earlier".into(),
        })
        .unwrap();
    }
    w.migrator(j, fast()).run(&items).await.unwrap();
    assert_eq!(w.fetcher.calls("https://img/x.png"), 1);
    assert_eq!(
        progress_map(&bytes.lock().unwrap())["x"].0,
        Progress::Rejected
    );
}

#[tokio::test]
async fn a_fatal_error_stops_the_run_and_a_rerun_resumes_from_the_journal() {
    let (items, images) = catalog();
    let w = World::new(&images);
    w.uploader
        .script
        .lock()
        .unwrap()
        .push_back(PortError::Fatal("relay refused credentials (403)".into()));
    let (j, bytes) = w.journal(b"", &names_of(&items));
    let err = w.migrator(j, fast()).run(&items).await.err().unwrap();
    assert!(err.contains("403"), "{err}");
    let (state, _) = replay(&bytes.lock().unwrap()).unwrap();
    assert!(
        state.items.values().all(|s| s
            .reject
            .as_ref()
            .is_none_or(|r| r.0 != RejectReason::UploadFailed)),
        "a systemic failure must not reject items"
    );

    let w2 = World::new(&images);
    let snapshot = bytes.lock().unwrap().clone();
    let (j2, bytes2) = w2.journal(&snapshot, &names_of(&items));
    let summary = w2.migrator(j2, fast()).run(&items).await.unwrap();
    assert_eq!(summary.published_now, 4);
    assert_eq!(
        progress_map(&bytes2.lock().unwrap())["alpha"].0,
        Progress::Published
    );
}

#[tokio::test]
async fn a_relay_answer_other_than_accepted_true_is_never_counted_as_published() {
    let w = World::new(&[("https://img/x.png", png(3, None))]);
    w.relay
        .state
        .lock()
        .unwrap()
        .publish_script
        .push_back(PortError::Permanent(
            "relay rejected the event: invalid".into(),
        ));
    let items = vec![item("x", "https://img/x.png")];
    let (j, bytes) = w.journal(b"", &names_of(&items));
    let summary = w.migrator(j, fast()).run(&items).await.unwrap();
    assert_eq!(summary.published_now, 0);
    let (state, _) = replay(&bytes.lock().unwrap()).unwrap();
    assert_eq!(state.items["x"].progress, Progress::Rejected);
    assert_eq!(
        state.items["x"].reject.as_ref().unwrap().0,
        RejectReason::PublishFailed
    );
    assert_eq!(w.relay.names().len(), 0);
}

#[tokio::test]
async fn a_blob_the_relay_hashed_differently_is_rejected_and_never_published() {
    let w = World::new(&[("https://img/x.png", png(3, None))]);
    w.uploader.wrong_hash.store(true, Ordering::SeqCst);
    let items = vec![item("x", "https://img/x.png")];
    let (j, bytes) = w.journal(b"", &names_of(&items));
    w.migrator(j, fast()).run(&items).await.unwrap();
    let (state, _) = replay(&bytes.lock().unwrap()).unwrap();
    assert_eq!(state.items["x"].progress, Progress::Rejected);
    assert_eq!(
        state.items["x"].reject.as_ref().unwrap().0,
        RejectReason::UploadFailed
    );
    assert_eq!(w.relay.names().len(), 0);
}

#[tokio::test]
async fn a_crash_after_any_journal_record_resumes_without_repeating_recorded_work() {
    let (items, images) = catalog();
    let reference = World::new(&images);
    let (j, ref_bytes) = reference.journal(b"", &names_of(&items));
    reference.migrator(j, fast()).run(&items).await.unwrap();
    let full = ref_bytes.lock().unwrap().clone();
    let want = progress_map(&full);
    let blobs: BTreeMap<String, Vec<u8>> = reference.uploader.stored.lock().unwrap().clone();
    let lines: Vec<usize> = std::iter::once(0)
        .chain(
            full.iter()
                .enumerate()
                .filter(|(_, b)| **b == b'\n')
                .map(|(i, _)| i + 1),
        )
        .collect();
    assert!(lines.len() > 20, "the journal has records at every state");

    for (n, &cut) in lines.iter().enumerate() {
        for torn in [false, true] {
            let mut prefix = full[..cut].to_vec();
            if torn {
                let Some(next) = full
                    .get(cut..)
                    .and_then(|rest| rest.iter().position(|b| *b == b'\n'))
                else {
                    continue;
                };
                prefix.extend_from_slice(&full[cut..cut + next / 2]);
            }
            let (before, _) = replay(&prefix).unwrap();
            let w = World::new(&images);
            std::fs::create_dir_all(w.blob_dir()).unwrap();
            for st in before.items.values() {
                if let (Progress::Sanitized, Some(sha)) = (st.progress, &st.sha) {
                    if !before.blobs.contains_key(sha) {
                        std::fs::write(w.blob_dir().join(format!("{sha}.png")), &blobs[sha])
                            .unwrap();
                    }
                }
                if matches!(st.progress, Progress::Published | Progress::Verified) {
                    w.relay.put(
                        &before
                            .items
                            .iter()
                            .find(|(_, s)| std::ptr::eq(*s, st))
                            .unwrap()
                            .0
                            .clone(),
                        st.url.as_deref().unwrap(),
                        "me",
                    );
                }
            }
            let (j, bytes) = w.journal(&prefix, &names_of(&items));
            w.migrator(j, fast()).run(&items).await.unwrap();

            let ctx = format!("crash after record {n} (torn tail: {torn})");
            assert_eq!(progress_map(&bytes.lock().unwrap()), want, "{ctx}");
            let uploads = w.uploader.uploaded_shas().len();
            assert_eq!(
                uploads,
                blobs.len() - before.blobs.len(),
                "uploads repeat nothing: {ctx}"
            );
            let already_published = before
                .items
                .values()
                .filter(|s| matches!(s.progress, Progress::Published | Progress::Verified))
                .count();
            assert_eq!(
                w.relay.state.lock().unwrap().published.len(),
                4 - already_published,
                "publishes repeat nothing: {ctx}"
            );
            let settled = before
                .items
                .values()
                .filter(|s| s.progress >= Progress::Sanitized)
                .count();
            assert_eq!(
                w.fetcher.total_calls() as usize,
                items.len() - settled,
                "fetches repeat nothing: {ctx}"
            );
        }
    }
}

#[tokio::test]
async fn a_lost_blob_cache_falls_back_to_fetching_again() {
    let (items, images) = catalog();
    let reference = World::new(&images);
    let (j, ref_bytes) = reference.journal(b"", &names_of(&items));
    reference.migrator(j, fast()).run(&items).await.unwrap();
    let full = ref_bytes.lock().unwrap().clone();
    let last_sanitized = full
        .split(|b| *b == b'\n')
        .collect::<Vec<_>>()
        .iter()
        .rposition(|l| String::from_utf8_lossy(l).contains(r#""rec":"sanitized""#))
        .unwrap();
    let cut: usize = full
        .split(|b| *b == b'\n')
        .take(last_sanitized + 1)
        .map(|l| l.len() + 1)
        .sum();
    let w = World::new(&images);
    let (j, bytes) = w.journal(&full[..cut], &names_of(&items));
    w.migrator(j, fast()).run(&items).await.unwrap();
    assert_eq!(progress_map(&bytes.lock().unwrap()), progress_map(&full));
}

#[tokio::test]
async fn every_action_waits_for_its_preceding_record_to_be_synced() {
    let (items, images) = catalog();
    let w = World::new(&images);
    let (j, _) = w.journal(b"", &names_of(&items));
    w.migrator(j, fast()).run(&items).await.unwrap();
    let log = w.log.lock().unwrap().clone();

    let mut unsynced = false;
    let (mut actions, mut appends) = (0, 0);
    let mut seen: Vec<&str> = Vec::new();
    for event in &log {
        if let Some(line) = event.strip_prefix("append:") {
            unsynced = true;
            appends += 1;
            seen.push(line);
        } else if event == "sync" {
            unsynced = false;
        } else {
            actions += 1;
            assert!(
                !unsynced,
                "{event} ran while a journal record was not yet synced"
            );
            let (rec, key) = if let Some(sha) = event.strip_prefix("upload:") {
                ("sanitized", sha)
            } else if let Some(name) = event.strip_prefix("publish:") {
                ("uploaded", name)
            } else {
                continue;
            };
            let needle = format!(r#""rec":"{rec}""#);
            let by_sha = |l: &&str| l.contains(&format!(r#""sha":"{key}""#));
            let by_name = |l: &&str| l.contains(&format!(r#""name":"{key}""#));
            assert!(
                seen.iter()
                    .any(|l| l.contains(&needle) && (by_sha(l) || by_name(l))),
                "{event} ran before its {rec} record was written"
            );
        }
    }
    assert!(
        appends >= 15 && actions >= 10,
        "the log has {appends} appends and {actions} actions"
    );
}

#[tokio::test]
async fn uploads_pace_to_the_rate_wait_out_a_429_hint_and_never_exceed_two_at_once() {
    let images: Vec<(String, Vec<u8>)> = (0..6)
        .map(|i| (format!("https://img/{i}.png"), png(i as u8, None)))
        .collect();
    let refs: Vec<(&str, Vec<u8>)> = images
        .iter()
        .map(|(s, b)| (s.as_str(), b.clone()))
        .collect();
    let items: Vec<PlanItem> = images
        .iter()
        .enumerate()
        .map(|(i, (s, _))| item(&format!("e{i}"), s))
        .collect();
    let w = World::new(&refs);
    w.uploader
        .script
        .lock()
        .unwrap()
        .push_back(PortError::Transient {
            message: "relay rate limit".into(),
            retry_after: Some(Duration::from_secs(20)),
        });
    let (j, _) = w.journal(b"", &names_of(&items));
    let rates = Rates {
        uploads_per_minute: 30,
        events_per_minute: 240,
        queries_per_minute: 120,
    };
    w.migrator(j, rates).run(&items).await.unwrap();

    assert_eq!(w.uploader.max_in_flight.load(Ordering::SeqCst), 2);
    let times = w.uploader.attempts_at.lock().unwrap().clone();
    assert_eq!(times.len(), 7, "six uploads and one retried attempt");
    let retried = times
        .iter()
        .find(|(sha, _)| times.iter().filter(|(s, _)| s == sha).count() == 2)
        .map(|(sha, _)| sha.clone())
        .expect("the throttled upload is attempted twice");
    let at: Vec<Duration> = times
        .iter()
        .filter(|(s, _)| *s == retried)
        .map(|(_, t)| *t)
        .collect();
    assert!(
        at[1] - at[0] >= Duration::from_secs(20),
        "the 429 hint pauses the bucket"
    );
    assert!(
        w.clock.now() >= Duration::from_secs(20),
        "30 per minute plus the pause cannot finish sooner"
    );
}

async fn published_world() -> (World, Vec<PlanItem>, Arc<Mutex<Vec<u8>>>) {
    let (items, images) = catalog();
    let w = World::new(&images);
    let (j, bytes) = w.journal(b"", &names_of(&items));
    w.migrator(j, fast()).run(&items).await.unwrap();
    (w, items, bytes)
}

type TestMigrator<'a> = Migrator<'a, FakeFetcher, FakeUploader, FakeRelay, FakeClock, MemStorage>;

fn reopen<'a>(
    w: &'a World,
    bytes: &Arc<Mutex<Vec<u8>>>,
    items: &[PlanItem],
) -> (TestMigrator<'a>, Arc<Mutex<Vec<u8>>>) {
    let snapshot = bytes.lock().unwrap().clone();
    let (j, b) = w.journal(&snapshot, &names_of(items));
    (w.migrator(j, fast()), b)
}

#[tokio::test]
async fn verify_reports_missing_mismatched_and_unprobeable_entries() {
    let (w, items, bytes) = published_world().await;
    let (m, after) = reopen(&w, &bytes, &items);
    let clean = m.verify(10, 20).await.unwrap();
    assert!(clean.failures.is_empty(), "{:?}", clean.failures);
    assert_eq!(clean.names_checked, 4);
    assert_eq!(clean.verified, 4);
    assert_eq!(clean.blob_probes, 4);
    assert_eq!(clean.prefix_searches, 4);
    assert_eq!(
        progress_map(&after.lock().unwrap())["alpha"].0,
        Progress::Verified
    );

    {
        let mut st = w.relay.state.lock().unwrap();
        st.store.remove("beta");
        let url = st.store["alpha"].url.clone();
        st.missing_blobs.insert(url);
    }
    w.relay
        .put("beta_twin", "https://elsewhere/other.png", "someone-else");
    let (m, _) = reopen(&w, &after, &items);
    let report = m.verify(10, 0).await.unwrap();
    let by_name: BTreeMap<_, _> = report
        .failures
        .iter()
        .map(|f| (f.name.as_str(), f.detail.as_str()))
        .collect();
    assert!(by_name["beta"].contains("no catalog entry"));
    assert!(by_name["beta_twin"].contains("someone-else"), "{by_name:?}");
    assert!(by_name["alpha"].contains("not found"), "{by_name:?}");
    assert!(by_name["alpha_alias"].contains("not found"), "{by_name:?}");
    assert_eq!(report.verified, 2);
}

#[tokio::test]
async fn verify_treats_a_full_result_list_without_the_name_as_inconclusive_not_failed() {
    let (w, items, bytes) = published_world().await;
    for i in 0..60 {
        w.relay
            .put(&format!("alpha_{i:02}"), "https://x/y.png", "other");
    }
    let (m, _) = reopen(&w, &bytes, &items);
    let report = m.verify(0, 20).await.unwrap();
    assert!(report.failures.is_empty(), "{:?}", report.failures);
}

#[tokio::test]
async fn rollback_deletes_only_journaled_names_keeps_blobs_and_republishes_on_rerun() {
    let (w, items, bytes) = published_world().await;
    w.relay.put("someone_elses", "https://x/e.png", "other");
    w.relay
        .state
        .lock()
        .unwrap()
        .delete_fail
        .insert("beta".into());
    let uploads_before = w.uploader.uploaded_shas();
    let (m, after) = reopen(&w, &bytes, &items);

    assert_eq!(m.rollback(true).await.unwrap().deleted, 4);
    assert!(
        w.relay.state.lock().unwrap().deleted.is_empty(),
        "dry run deletes nothing"
    );
    let report = m.rollback(false).await.unwrap();
    assert_eq!(report.deleted, 3);
    assert_eq!(
        report
            .failures
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["beta"]
    );
    assert_eq!(w.relay.names(), ["beta", "someone_elses"]);
    assert_eq!(w.uploader.uploaded_shas(), uploads_before, "blobs stay");
    let state = progress_map(&after.lock().unwrap());
    assert_eq!(
        state["alpha"].0,
        Progress::Uploaded,
        "unpublished after an accepted delete"
    );
    assert_eq!(
        state["beta"].0,
        Progress::Published,
        "a failed delete is not recorded as done"
    );

    w.relay.state.lock().unwrap().delete_fail.clear();
    let (m, after2) = reopen(&w, &after, &items);
    assert_eq!(m.rollback(false).await.unwrap().deleted, 1);
    let (m, after3) = reopen(&w, &after2, &items);
    let summary = m.run(&items).await.unwrap();
    assert_eq!(
        summary.published_now, 4,
        "a rerun republishes from the recorded blob urls"
    );
    assert_eq!(
        w.uploader.uploaded_shas(),
        uploads_before,
        "without uploading again"
    );
    assert_eq!(
        progress_map(&after3.lock().unwrap())["beta"].0,
        Progress::Published
    );
}

#[tokio::test]
async fn measure_counts_unique_blobs_bytes_and_failures_once_per_source() {
    let (items, images) = catalog();
    let w = World::new(&images);
    let m = measure(&w.fetcher, &items, 8).await;
    assert_eq!(m.unique_sources, 5);
    assert_eq!(m.unique_blobs, 2);
    assert_eq!(m.names_resolved, 4);
    assert_eq!(
        m.unique_bytes,
        (png(1, None).len() + png(2, None).len()) as u64
    );
    let rejected: Vec<_> = m
        .rejects
        .iter()
        .map(|r| (r.name.as_str(), r.reason))
        .collect();
    assert_eq!(
        rejected,
        [
            ("broken", RejectReason::InvalidImage),
            ("gone", RejectReason::FetchFailed)
        ]
    );
    assert_eq!(
        w.fetcher.calls("https://img/1.png"),
        1,
        "aliases share one fetch"
    );
    assert_eq!(w.relay.writes.load(Ordering::SeqCst), 0);
}

fn write(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn plan_args(extra: &[&str]) -> PlanArgs {
    #[derive(clap::Parser)]
    struct W {
        #[command(flatten)]
        args: PlanArgs,
    }
    let mut argv = vec!["w"];
    argv.extend_from_slice(extra);
    <W as clap::Parser>::try_parse_from(argv).unwrap().args
}

const SLACK: &str = r#"{"ok":true,"emoji":{
  "alpha":"https://img/1.png","alpha_alias":"alias:alpha","beta":"https://img/2.png",
  "Bad Name":"https://img/2.png","cyc_a":"alias:cyc_b","cyc_b":"alias:cyc_a","uni":"alias:thumbsup"}}"#;

#[tokio::test]
async fn plan_never_writes_to_buzz_and_applies_the_conflict_policy() {
    let dir = tempfile::tempdir().unwrap();
    let slack = write(dir.path(), "emoji.json", SLACK);
    let slack = slack.to_str().unwrap();
    let (_, images) = catalog();
    let w = World::new(&images);
    w.relay.put("beta", "https://old/beta.png", "other");

    let skip = build_plan(&plan_args(&["--slack-json", slack]), &w.relay, &w.fetcher)
        .await
        .unwrap();
    assert_eq!(names_of(&skip.plan.items), ["alpha", "alpha_alias"]);
    let reasons: BTreeMap<_, _> = skip
        .rejects
        .iter()
        .map(|r| (r.name.as_str(), r.reason))
        .collect();
    assert_eq!(reasons["beta"], RejectReason::ConflictSkipped);
    assert_eq!(reasons["Bad Name"], RejectReason::InvalidName);
    assert_eq!(reasons["cyc_a"], RejectReason::AliasCycle);
    assert_eq!(reasons["uni"], RejectReason::AliasDangling);
    assert_eq!(skip.report.aliases_resolved, 1);
    assert_eq!(skip.report.existing_entries, 1);

    let over = build_plan(
        &plan_args(&["--slack-json", slack, "--on-conflict", "overwrite"]),
        &w.relay,
        &w.fetcher,
    )
    .await
    .unwrap();
    assert_eq!(names_of(&over.plan.items), ["alpha", "alpha_alias", "beta"]);
    assert_ne!(skip.report.plan_sha256, over.report.plan_sha256);

    let unicode = write(dir.path(), "unicode.txt", "thumbsup\n:smile:\n");
    let with_unicode = build_plan(
        &plan_args(&[
            "--slack-json",
            slack,
            "--unicode-names",
            unicode.to_str().unwrap(),
        ]),
        &w.relay,
        &w.fetcher,
    )
    .await
    .unwrap();
    assert!(with_unicode
        .rejects
        .iter()
        .any(|r| r.name == "uni" && r.reason == RejectReason::AliasToUnicode));

    let offline = w.relay.lookups.load(Ordering::SeqCst);
    build_plan(
        &plan_args(&["--slack-json", slack, "--no-conflict-check"]),
        &w.relay,
        &w.fetcher,
    )
    .await
    .unwrap();
    assert_eq!(
        w.relay.lookups.load(Ordering::SeqCst),
        offline,
        "no relay access without the check"
    );

    assert_eq!(
        w.relay.writes.load(Ordering::SeqCst),
        0,
        "plan makes no relay write"
    );
    assert!(w.uploader.uploaded_shas().is_empty());
    assert_eq!(
        w.fetcher.total_calls(),
        0,
        "plan without --dry-run fetches no image"
    );
}

#[tokio::test]
async fn plan_dry_run_reports_duplicates_bytes_and_the_time_estimate() {
    let dir = tempfile::tempdir().unwrap();
    let slack = write(dir.path(), "emoji.json", SLACK);
    let (_, images) = catalog();
    let w = World::new(&images);
    let out = build_plan(
        &plan_args(&[
            "--slack-json",
            slack.to_str().unwrap(),
            "--dry-run",
            "--on-conflict",
            "overwrite",
            "--uploads-per-minute",
            "60",
            "--events-per-minute",
            "120",
        ]),
        &w.relay,
        &w.fetcher,
    )
    .await
    .unwrap();
    let r = &out.report;
    assert_eq!(r.estimate_basis, "measured");
    assert_eq!(r.unique_blobs, 2);
    assert_eq!(r.items_planned, 3);
    assert!((r.duplicate_ratio - 1.0 / 3.0).abs() < 1e-9);
    assert!(r.bytes_to_upload.unwrap() > 0);
    assert_eq!(r.estimate.upload_secs, 2.0);
    assert_eq!(r.estimate.publish_secs, 1.5);
    assert_eq!(r.estimate.total_secs, 2.0);
    assert_eq!(w.relay.writes.load(Ordering::SeqCst), 0);

    let sampled = build_plan(
        &plan_args(&[
            "--slack-json",
            slack.to_str().unwrap(),
            "--dry-run",
            "--sample",
            "1",
            "--no-conflict-check",
        ]),
        &w.relay,
        &w.fetcher,
    )
    .await
    .unwrap();
    assert_eq!(sampled.report.estimate_basis, "sampled_extrapolation");
    assert!(sampled.report.measured.as_ref().unwrap().extrapolated);
}

#[test]
fn images_dir_input_maps_file_names_and_refuses_unsafe_map_entries() {
    let dir = tempfile::tempdir().unwrap();
    let images = dir.path().join("imgs");
    std::fs::create_dir(&images).unwrap();
    for f in ["Party.png", "ok_2.GIF", "notes.txt"] {
        std::fs::write(images.join(f), b"x").unwrap();
    }
    let input = read_images_dir(&images, None).unwrap();
    let names: Vec<_> = input.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Party", "ok_2"]);
    assert!(input.entries.iter().all(|e| e.value.starts_with("file://")));
    let first = input.sha256.clone();
    std::fs::write(images.join("new.webp"), b"x").unwrap();
    assert_ne!(
        read_images_dir(&images, None).unwrap().sha256,
        first,
        "listing changes the input hash"
    );

    let map = write(
        dir.path(),
        "map.json",
        r#"{"x":"Party.png","y":"../secret.png","z":"alias:x","w":"absent.png"}"#,
    );
    let input = read_images_dir(&images, Some(&map)).unwrap();
    let (items, rejects) = names::resolve_entries(&input.entries, input.policy, &HashSet::new());
    assert_eq!(names_of(&items), ["x", "z"]);
    assert!(rejects
        .iter()
        .all(|r| r.reason == RejectReason::InvalidSource));
    assert_eq!(rejects.len(), 2);
}

#[test]
fn sampling_is_evenly_spaced_and_bounded() {
    let items: Vec<PlanItem> = (0..10).map(|i| item(&format!("n{i}"), "s")).collect();
    assert_eq!(names_of(&sample_items(&items, Some(3))), ["n0", "n4", "n8"]);
    assert_eq!(sample_items(&items, Some(50)).len(), 10);
    assert_eq!(sample_items(&items, None).len(), 10);
}

#[tokio::test]
async fn lookups_split_into_batches_of_fifty_names_and_lose_no_row() {
    let w = World::new(&[]);
    let names: Vec<String> = (0..120).map(|i| format!("n{i}")).collect();
    for name in &names {
        w.relay.put(name, "https://img/x.png", "me");
    }
    let bucket = super::bucket::TokenBucket::new(w.clock.clone(), 60_000);
    let rows = lookup_all(&w.relay, &bucket, &w.clock, &names, 3)
        .await
        .unwrap();
    assert_eq!(rows.len(), 120);
    assert_eq!(w.relay.lookups.load(Ordering::SeqCst), 3);
}

#[test]
fn clap_rejects_a_zero_rate_and_both_input_kinds() {
    use clap::Parser;
    #[derive(Parser)]
    struct W {
        #[command(flatten)]
        args: PlanArgs,
    }
    for argv in [
        vec!["w", "--slack-json", "a", "--uploads-per-minute", "0"],
        vec!["w", "--slack-json", "a", "--images-dir", "b"],
        vec!["w"],
        vec!["w", "--slack-json", "a", "--name-map", "m"],
        vec!["w", "--slack-json", "a", "--sample", "3"],
    ] {
        assert!(W::try_parse_from(&argv).is_err(), "{argv:?}");
    }
}
