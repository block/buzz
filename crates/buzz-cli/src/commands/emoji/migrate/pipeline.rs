//! The resumable migration pipeline: fetch, sanitize, upload, publish, verify
//! and rollback over the [`ports`](super::ports) traits and the journal.
//!
//! Each step records its result in the journal (synced) before the next step
//! acts, so a restart replays to a state whose next action is safe to repeat.

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use rand::seq::IndexedRandom;
use serde::Serialize;

use super::bucket::{Clock, TokenBucket};
use super::journal::{atomic_write, ItemState, Journal, JournalStorage, Progress, Record, Stage};
use super::names::{PlanItem, Reject, RejectReason};
use super::plan::sha256_hex;
use super::ports::{CatalogRow, Fetcher, PortError, Relay, Uploader};

/// Names per `#d` lookup, the relay contract.
pub const NAMES_PER_LOOKUP: usize = 50;
const PROGRESS_EVERY: u64 = 1000;

/// Pacing for the three rate-limited actions, in requests per minute.
#[derive(Debug, Clone, Copy)]
pub struct Rates {
    pub uploads_per_minute: u32,
    pub events_per_minute: u32,
    pub queries_per_minute: u32,
}

/// Limits that bound a run.
#[derive(Debug, Clone)]
pub struct RunConfig {
    /// Items in flight at once; bounds concurrent fetches and cached bytes.
    pub concurrency: usize,
    /// Concurrent uploads; the relay allows 2 per key.
    pub upload_concurrency: usize,
    /// Failed attempts per item and stage before the item is rejected.
    pub max_attempts: u32,
    /// Where sanitized bytes wait between the `sanitized` and `uploaded` records.
    pub blob_dir: PathBuf,
    /// Written (temp file + rename) every [`PROGRESS_EVERY`] items and at the end.
    pub snapshot_path: Option<PathBuf>,
}

/// What a run leaves behind.
#[derive(Debug, Serialize)]
pub struct RunSummary {
    pub total: u64,
    pub by_state: BTreeMap<String, u64>,
    pub unique_blobs: u64,
    /// Entries this run published, counted only after `accepted: true`.
    pub published_now: u64,
}

/// One failed check.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Failure {
    pub name: String,
    pub detail: String,
}

/// Result of `verify`.
#[derive(Debug, Serialize)]
pub struct VerifyReport {
    pub names_checked: u64,
    pub verified: u64,
    pub blob_probes: u64,
    pub prefix_searches: u64,
    /// Searches whose result list was full without the name: not a failure.
    pub inconclusive_searches: u64,
    pub failures: Vec<Failure>,
}

/// Result of `rollback`.
#[derive(Debug, Serialize)]
pub struct RollbackReport {
    pub deleted: u64,
    pub failures: Vec<Failure>,
}

fn backoff(failures: u32) -> Duration {
    Duration::from_secs((1u64 << failures.saturating_sub(1).min(5)).min(30))
}

/// Fetch every distinct blob once for `plan --dry-run`; nothing is written to Buzz.
pub struct Measure {
    pub unique_sources: u64,
    pub unique_blobs: u64,
    pub unique_bytes: u64,
    pub names_resolved: u64,
    pub rejects: Vec<Reject>,
}

/// Fetch, sanitize and hash each distinct source among `items` (aliases share
/// one source) and count unique sanitized blobs and their bytes.
pub async fn measure<F: Fetcher>(fetcher: &F, items: &[PlanItem], concurrency: usize) -> Measure {
    let mut by_source: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for item in items {
        by_source.entry(&item.source).or_default().push(&item.name);
    }
    let results: Vec<_> = stream::iter(by_source)
        .map(|(source, names)| async move {
            let outcome = match fetcher.fetch(source).await {
                Ok(bytes) => sanitize(&bytes).map(|s| (sha256_hex(&s.bytes), s.bytes.len() as u64)),
                Err(e) => Err((RejectReason::FetchFailed, e.message().to_string())),
            };
            (names, outcome)
        })
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await;
    let mut blobs: HashMap<String, u64> = HashMap::new();
    let mut m = Measure {
        unique_sources: results.len() as u64,
        unique_blobs: 0,
        unique_bytes: 0,
        names_resolved: 0,
        rejects: Vec::new(),
    };
    for (names, outcome) in results {
        match outcome {
            Ok((sha, size)) => {
                m.names_resolved += names.len() as u64;
                blobs.insert(sha, size);
            }
            Err((reason, detail)) => m.rejects.extend(names.into_iter().map(|n| Reject {
                name: n.to_string(),
                reason,
                detail: detail.clone(),
            })),
        }
    }
    m.unique_blobs = blobs.len() as u64;
    m.unique_bytes = blobs.values().sum();
    m.rejects.sort_by(|a, b| a.name.cmp(&b.name));
    m
}

struct Sanitized {
    bytes: Vec<u8>,
    ext: &'static str,
}

fn sanitize(raw: &[u8]) -> Result<Sanitized, (RejectReason, String)> {
    let bytes = buzz_media::strip_image_metadata(raw)
        .map_err(|e| (RejectReason::InvalidImage, format!("sanitize: {e}")))?;
    let mime = buzz_media::validation::validate_content(
        &bytes,
        &buzz_media::MediaConfig::offline_validation(),
    )
    .map_err(|e| (RejectReason::InvalidImage, format!("validate: {e}")))?;
    Ok(Sanitized {
        bytes,
        ext: buzz_media::validation::mime_to_ext(&mime),
    })
}

/// `#d` lookup of `names`, 50 per query, paced by `bucket`, with bounded
/// retries. A lookup that cannot finish is an error, never an empty answer.
pub async fn lookup_all<R: Relay, C: Clock>(
    relay: &R,
    bucket: &TokenBucket<C>,
    clock: &C,
    names: &[String],
    max_attempts: u32,
) -> Result<Vec<CatalogRow>, String> {
    let mut rows = Vec::new();
    for chunk in names.chunks(NAMES_PER_LOOKUP) {
        let mut fails = 0u32;
        loop {
            bucket.acquire().await;
            match relay.lookup(chunk).await {
                Ok(found) => {
                    rows.extend(found);
                    break;
                }
                Err(PortError::Transient {
                    message,
                    retry_after,
                }) => {
                    fails += 1;
                    if fails >= max_attempts {
                        return Err(format!("lookup failed {fails} times: {message}"));
                    }
                    if let Some(d) = retry_after {
                        bucket.pause_for(d);
                    }
                    clock.sleep(backoff(fails)).await;
                }
                Err(e) => return Err(format!("lookup failed: {}", e.message())),
            }
        }
    }
    Ok(rows)
}

/// The run engine.
pub struct Migrator<'a, F, U, R, C: Clock + Clone, S: JournalStorage> {
    fetcher: &'a F,
    uploader: &'a U,
    relay: &'a R,
    clock: C,
    journal: Mutex<Journal<S>>,
    upload_bucket: TokenBucket<C>,
    event_bucket: TokenBucket<C>,
    query_bucket: TokenBucket<C>,
    upload_slots: tokio::sync::Semaphore,
    sha_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    fatal: Mutex<Option<String>>,
    cfg: RunConfig,
    published_now: AtomicU64,
}

impl<'a, F, U, R, C, S> Migrator<'a, F, U, R, C, S>
where
    F: Fetcher,
    U: Uploader,
    R: Relay,
    C: Clock + Clone,
    S: JournalStorage,
{
    pub fn new(
        fetcher: &'a F,
        uploader: &'a U,
        relay: &'a R,
        clock: C,
        journal: Journal<S>,
        rates: Rates,
        cfg: RunConfig,
    ) -> Self {
        Self {
            fetcher,
            uploader,
            relay,
            upload_bucket: TokenBucket::new(clock.clone(), rates.uploads_per_minute),
            event_bucket: TokenBucket::new(clock.clone(), rates.events_per_minute),
            query_bucket: TokenBucket::new(clock.clone(), rates.queries_per_minute),
            clock,
            journal: Mutex::new(journal),
            upload_slots: tokio::sync::Semaphore::new(cfg.upload_concurrency.max(1)),
            sha_locks: Mutex::default(),
            fatal: Mutex::default(),
            cfg,
            published_now: AtomicU64::new(0),
        }
    }

    fn set_fatal(&self, message: String) -> String {
        let mut slot = self.fatal.lock().unwrap_or_else(|p| p.into_inner());
        slot.get_or_insert_with(|| message.clone());
        message
    }

    fn fatal_message(&self) -> Option<String> {
        self.fatal.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn rec(&self, record: Record) -> Result<(), String> {
        let mut journal = self.journal.lock().unwrap_or_else(|p| p.into_inner());
        journal
            .record(record)
            .map_err(|e| self.set_fatal(format!("journal write failed: {e}")))
    }

    fn state_of(&self, name: &str) -> Result<ItemState, String> {
        let journal = self.journal.lock().unwrap_or_else(|p| p.into_inner());
        journal
            .state()
            .items
            .get(name)
            .cloned()
            .ok_or_else(|| self.set_fatal(format!("'{name}' is not in the journal")))
    }

    fn blob_url(&self, sha: &str) -> Option<String> {
        let journal = self.journal.lock().unwrap_or_else(|p| p.into_inner());
        journal.state().blobs.get(sha).cloned()
    }

    fn reject(&self, name: &str, reason: RejectReason, detail: String) -> Result<(), String> {
        self.rec(Record::Rejected {
            name: name.to_string(),
            reason,
            detail,
        })
    }

    async fn retrying<T, Fut>(
        &self,
        stage: Stage,
        name: &str,
        mut fails: u32,
        bucket: Option<&TokenBucket<C>>,
        permanent: RejectReason,
        mut op: impl FnMut() -> Fut,
    ) -> Result<Option<T>, String>
    where
        Fut: Future<Output = Result<T, PortError>>,
    {
        let mut last = String::from("earlier attempts");
        loop {
            if fails >= self.cfg.max_attempts {
                self.reject(
                    name,
                    RejectReason::RetriesExhausted,
                    format!("{stage:?} failed {fails} times; last error: {last}"),
                )?;
                return Ok(None);
            }
            match op().await {
                Ok(v) => return Ok(Some(v)),
                Err(PortError::Permanent(m)) => {
                    self.reject(name, permanent, m)?;
                    return Ok(None);
                }
                Err(PortError::Fatal(m)) => return Err(self.set_fatal(m)),
                Err(PortError::Transient {
                    message,
                    retry_after,
                }) => {
                    if let (Some(d), Some(b)) = (retry_after, bucket) {
                        b.pause_for(d);
                    }
                    self.rec(Record::Failed {
                        name: name.to_string(),
                        stage,
                        error: message.clone(),
                    })?;
                    last = message;
                    fails += 1;
                    if fails < self.cfg.max_attempts {
                        self.clock.sleep(backoff(fails)).await;
                    }
                }
            }
        }
    }

    async fn process(&self, item: &PlanItem) -> Result<(), String> {
        loop {
            if self.fatal_message().is_some() {
                return Ok(());
            }
            let st = self.state_of(&item.name)?;
            match st.progress {
                Progress::Planned | Progress::Fetched => self.acquire(item, &st).await?,
                Progress::Sanitized => self.upload(item, &st).await?,
                Progress::Uploaded => self.publish(item, &st).await?,
                Progress::Published | Progress::Verified | Progress::Rejected => return Ok(()),
            }
        }
    }

    fn cache_blob(&self, sha: &str, ext: &str, bytes: &[u8]) -> Result<(), String> {
        let path = self.cfg.blob_dir.join(format!("{sha}.{ext}"));
        if std::fs::read(&path).is_ok_and(|b| sha256_hex(&b) == sha) {
            return Ok(());
        }
        std::fs::create_dir_all(&self.cfg.blob_dir)
            .and_then(|()| atomic_write(&path, bytes))
            .map_err(|e| self.set_fatal(format!("cannot cache blob {}: {e}", path.display())))
    }

    async fn acquire(&self, item: &PlanItem, st: &ItemState) -> Result<(), String> {
        let source = item.source.as_str();
        let fetcher = self.fetcher;
        let Some(raw) = self
            .retrying(
                Stage::Fetch,
                &item.name,
                st.failures(Stage::Fetch),
                None,
                RejectReason::FetchFailed,
                || fetcher.fetch(source),
            )
            .await?
        else {
            return Ok(());
        };
        self.rec(Record::Fetched {
            name: item.name.clone(),
            sha: sha256_hex(&raw),
        })?;
        let clean = match sanitize(&raw) {
            Ok(c) => c,
            Err((reason, detail)) => return self.reject(&item.name, reason, detail),
        };
        let sha = sha256_hex(&clean.bytes);
        if self.blob_url(&sha).is_none() {
            self.cache_blob(&sha, clean.ext, &clean.bytes)?;
        }
        self.rec(Record::Sanitized {
            name: item.name.clone(),
            sha,
            ext: clean.ext.to_string(),
            size: clean.bytes.len() as u64,
        })
    }

    fn sha_lock(&self, sha: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.sha_locks.lock().unwrap_or_else(|p| p.into_inner());
        locks.entry(sha.to_string()).or_default().clone()
    }

    async fn upload(&self, item: &PlanItem, st: &ItemState) -> Result<(), String> {
        let (Some(sha), Some(ext)) = (st.sha.clone(), st.ext.clone()) else {
            return Err(self.set_fatal(format!("'{}' has no sanitized blob", item.name)));
        };
        let reuse = |url: String| {
            self.rec(Record::Uploaded {
                name: item.name.clone(),
                sha: sha.clone(),
                url,
            })
        };
        let lock = self.sha_lock(&sha);
        let _held = lock.lock().await;
        if let Some(url) = self.blob_url(&sha) {
            return reuse(url);
        }
        let path = self.cfg.blob_dir.join(format!("{sha}.{ext}"));
        let bytes = match std::fs::read(&path) {
            Ok(b) if sha256_hex(&b) == sha => b,
            _ => return self.acquire(item, st).await,
        };
        let _slot = self
            .upload_slots
            .acquire()
            .await
            .map_err(|e| self.set_fatal(format!("upload slots closed: {e}")))?;
        let (uploader, bucket, payload) = (self.uploader, &self.upload_bucket, &bytes);
        let Some(blob) = self
            .retrying(
                Stage::Upload,
                &item.name,
                st.failures(Stage::Upload),
                Some(bucket),
                RejectReason::UploadFailed,
                || async move {
                    bucket.acquire().await;
                    uploader.upload(payload.clone()).await
                },
            )
            .await?
        else {
            return Ok(());
        };
        if blob.sha256 != sha {
            return self.reject(
                &item.name,
                RejectReason::UploadFailed,
                format!("relay stored hash {} for bytes hashed {sha}", blob.sha256),
            );
        }
        reuse(blob.url)?;
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!(
                "warning: could not remove cached blob {}: {e}",
                path.display()
            );
        }
        Ok(())
    }

    async fn publish(&self, item: &PlanItem, st: &ItemState) -> Result<(), String> {
        let Some(url) = st.url.as_deref() else {
            return Err(self.set_fatal(format!("'{}' has no uploaded url", item.name)));
        };
        let (relay, bucket, name) = (self.relay, &self.event_bucket, item.name.as_str());
        let Some(event_id) = self
            .retrying(
                Stage::Publish,
                name,
                st.failures(Stage::Publish),
                Some(bucket),
                RejectReason::PublishFailed,
                || async move {
                    bucket.acquire().await;
                    relay.publish(name, url).await
                },
            )
            .await?
        else {
            return Ok(());
        };
        self.rec(Record::Published {
            name: item.name.clone(),
            event_id,
        })?;
        self.published_now.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn summary(&self) -> RunSummary {
        let journal = self.journal.lock().unwrap_or_else(|p| p.into_inner());
        let snap = journal.state().snapshot();
        RunSummary {
            total: journal.state().items.len() as u64,
            by_state: snap.by_state,
            unique_blobs: snap.unique_blobs,
            published_now: self.published_now.load(Ordering::Relaxed),
        }
    }

    fn write_snapshot(&self) -> Result<(), String> {
        let Some(path) = &self.cfg.snapshot_path else {
            return Ok(());
        };
        let snap = {
            let journal = self.journal.lock().unwrap_or_else(|p| p.into_inner());
            journal.state().snapshot()
        };
        let bytes = serde_json::to_vec_pretty(&snap).map_err(|e| e.to_string())?;
        atomic_write(path, &bytes)
            .map_err(|e| self.set_fatal(format!("snapshot write failed: {e}")))
    }

    fn sweep_cache(&self) {
        let Ok(dir) = std::fs::read_dir(&self.cfg.blob_dir) else {
            return;
        };
        let journal = self.journal.lock().unwrap_or_else(|p| p.into_inner());
        for entry in dir.flatten() {
            let path = entry.path();
            let uploaded = path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|sha| journal.state().blobs.contains_key(sha));
            if uploaded {
                if let Err(e) = std::fs::remove_file(&path) {
                    eprintln!(
                        "warning: could not remove cached blob {}: {e}",
                        path.display()
                    );
                }
            }
        }
    }

    /// Drive every item to a terminal state. Returns `Err` when a fatal
    /// condition (journal I/O, bad credentials) stopped the run; the journal
    /// stays valid and a re-run resumes.
    pub async fn run(&self, items: &[PlanItem]) -> Result<RunSummary, String> {
        let finished = AtomicU64::new(0);
        stream::iter(items)
            .map(|item| async {
                let _ = self.process(item).await;
                let n = finished.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(PROGRESS_EVERY) && self.write_snapshot().is_ok() {
                    eprintln!("progress: {n}/{} items settled", items.len());
                }
            })
            .buffer_unordered(self.cfg.concurrency.max(1))
            .collect::<Vec<()>>()
            .await;
        self.sweep_cache();
        let snapshot = self.write_snapshot();
        if let Some(message) = self.fatal_message() {
            return Err(message);
        }
        snapshot?;
        Ok(self.summary())
    }

    fn names_at(&self, wanted: &[Progress]) -> Vec<(String, Option<String>)> {
        let journal = self.journal.lock().unwrap_or_else(|p| p.into_inner());
        journal
            .state()
            .items
            .iter()
            .filter(|(_, st)| wanted.contains(&st.progress))
            .map(|(n, st)| (n.clone(), st.url.clone()))
            .collect()
    }

    /// Check every published name against the relay, probe `probe_blobs`
    /// random blob URLs and run `prefix_searches` random prefix searches.
    pub async fn verify(
        &self,
        probe_blobs: usize,
        prefix_searches: usize,
    ) -> Result<VerifyReport, String> {
        let published = self.names_at(&[Progress::Published, Progress::Verified]);
        let expected: HashMap<&str, &str> = published
            .iter()
            .filter_map(|(n, u)| Some((n.as_str(), u.as_deref()?)))
            .collect();
        let names: Vec<String> = published.iter().map(|(n, _)| n.clone()).collect();
        let rows = lookup_all(
            self.relay,
            &self.query_bucket,
            &self.clock,
            &names,
            self.cfg.max_attempts,
        )
        .await?;
        let found: HashMap<&str, &CatalogRow> = rows.iter().map(|r| (r.name.as_str(), r)).collect();
        let mut report = VerifyReport {
            names_checked: names.len() as u64,
            verified: 0,
            blob_probes: 0,
            prefix_searches: 0,
            inconclusive_searches: 0,
            failures: Vec::new(),
        };
        let mut good: Vec<&str> = Vec::new();
        for name in &names {
            let want = expected.get(name.as_str()).copied().unwrap_or_default();
            let failure = match found.get(name.as_str()) {
                None => Some("no catalog entry on the relay".to_string()),
                Some(row) if row.url != want => Some(format!(
                    "relay serves {} from author {}, journal has {want}",
                    row.url, row.author
                )),
                Some(_) => None,
            };
            match failure {
                None => good.push(name),
                Some(detail) => {
                    self.rec(Record::Failed {
                        name: name.clone(),
                        stage: Stage::Verify,
                        error: detail.clone(),
                    })?;
                    report.failures.push(Failure {
                        name: name.clone(),
                        detail,
                    });
                }
            }
        }
        for name in &good {
            if self.state_of(name)?.progress == Progress::Published {
                self.rec(Record::Verified {
                    name: (*name).to_string(),
                })?;
            }
            report.verified += 1;
        }
        let mut rng = rand::rng();
        for name in good.sample(&mut rng, probe_blobs) {
            report.blob_probes += 1;
            let url = expected.get(name).copied().unwrap_or_default();
            match self.relay.blob_exists(url).await {
                Ok(true) => {}
                Ok(false) => report.failures.push(Failure {
                    name: (*name).to_string(),
                    detail: format!("blob {url} answered not found"),
                }),
                Err(e) => report.failures.push(Failure {
                    name: (*name).to_string(),
                    detail: format!("blob probe of {url} failed: {}", e.message()),
                }),
            }
        }
        let searchable: Vec<&&str> = good.iter().filter(|n| n.len() >= 2).collect();
        for name in searchable.sample(&mut rng, prefix_searches) {
            report.prefix_searches += 1;
            self.query_bucket.acquire().await;
            let limit = 50;
            match self.relay.search(name, limit).await {
                Ok(hits) if hits.iter().any(|h| h.name == ***name) => {}
                Ok(hits) if hits.len() >= limit as usize => report.inconclusive_searches += 1,
                Ok(_) => report.failures.push(Failure {
                    name: (**name).to_string(),
                    detail: "prefix search did not return the entry".into(),
                }),
                Err(e) => report.failures.push(Failure {
                    name: (**name).to_string(),
                    detail: format!("prefix search failed: {}", e.message()),
                }),
            }
        }
        Ok(report)
    }

    /// Delete every entry the journal says this key published. Blobs stay.
    /// A name is marked `unpublished` only after the relay accepted its delete.
    pub async fn rollback(&self, dry_run: bool) -> Result<RollbackReport, String> {
        let targets: Vec<String> = self
            .names_at(&[Progress::Published, Progress::Verified])
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        if dry_run {
            return Ok(RollbackReport {
                deleted: targets.len() as u64,
                failures: Vec::new(),
            });
        }
        let results: Vec<Result<Option<Failure>, String>> = stream::iter(&targets)
            .map(|name| async move {
                let mut fails = 0u32;
                loop {
                    if self.fatal_message().is_some() {
                        return Ok(Some(Failure {
                            name: name.clone(),
                            detail: "run stopped".into(),
                        }));
                    }
                    self.event_bucket.acquire().await;
                    match self.relay.delete(name).await {
                        Ok(()) => {
                            self.rec(Record::Unpublished { name: name.clone() })?;
                            return Ok(None);
                        }
                        Err(PortError::Fatal(m)) => return Err(self.set_fatal(m)),
                        Err(PortError::Permanent(m)) => {
                            return Ok(Some(Failure {
                                name: name.clone(),
                                detail: m,
                            }))
                        }
                        Err(PortError::Transient {
                            message,
                            retry_after,
                        }) => {
                            fails += 1;
                            if fails >= self.cfg.max_attempts {
                                return Ok(Some(Failure {
                                    name: name.clone(),
                                    detail: format!("delete failed {fails} times: {message}"),
                                }));
                            }
                            if let Some(d) = retry_after {
                                self.event_bucket.pause_for(d);
                            }
                            self.clock.sleep(backoff(fails)).await;
                        }
                    }
                }
            })
            .buffer_unordered(self.cfg.concurrency.max(1))
            .collect()
            .await;
        let mut report = RollbackReport {
            deleted: 0,
            failures: Vec::new(),
        };
        for r in results {
            match r {
                Ok(None) => report.deleted += 1,
                Ok(Some(f)) => report.failures.push(f),
                Err(_) => {}
            }
        }
        if let Some(message) = self.fatal_message() {
            return Err(message);
        }
        report.failures.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(report)
    }

    /// Items the journal marks rejected, sorted by name.
    pub fn rejected(&self) -> Vec<Reject> {
        let journal = self.journal.lock().unwrap_or_else(|p| p.into_inner());
        journal
            .state()
            .items
            .iter()
            .filter_map(|(name, st)| {
                let (reason, detail) = st.reject.clone()?;
                Some(Reject {
                    name: name.clone(),
                    reason,
                    detail,
                })
            })
            .collect()
    }
}
