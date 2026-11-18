//! `buzz emoji migrate`: bulk-load a Slack emoji catalog as kind 30180 entries.
//!
//! `plan` is read-only against Buzz. `run` is resumable from its journal.
//! `verify` and `rollback` work from the journal of what `run` published.

mod bucket;
mod journal;
mod live;
mod names;
mod pipeline;
mod plan;
mod ports;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde::Serialize;

use self::bucket::{SystemClock, TokenBucket};
use self::live::{BuzzPorts, FetchPolicy, HttpFetcher};
use self::names::{PlanItem, RawEntry, Reject, RejectReason, SourcePolicy};
use self::pipeline::{lookup_all, measure, Migrator, Rates, RunConfig};
use self::plan::{duplicate_ratio, estimate, sha256_hex, OnConflict, Plan, PLAN_VERSION};
use self::ports::{Fetcher, Relay};
use crate::client::BuzzClient;
use crate::error::CliError;

/// Largest input file read into memory.
const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;
const IMAGE_EXTENSIONS: [&str; 5] = ["png", "gif", "jpg", "jpeg", "webp"];

/// Bulk Slack catalog migration. Run `plan --dry-run` first.
#[derive(Subcommand)]
pub enum MigrateCmd {
    /// Parse and normalize the input, resolve aliases, apply the conflict policy and write plan.json (never writes to Buzz)
    Plan(PlanArgs),
    /// Fetch, sanitize, upload and publish the plan; resumes from the journal after any stop
    Run(RunArgs),
    /// Check every published name against the relay; exits non-zero on any failure
    Verify(VerifyArgs),
    /// Delete the entries the journal says this key published (blobs stay)
    Rollback(RollbackArgs),
}

#[derive(Args)]
pub struct WorkDir {
    /// Directory for plan.json, rejects.jsonl, journal.jsonl, snapshot.json and blobs/
    #[arg(long, default_value = "emoji-migration")]
    work_dir: PathBuf,
}

#[derive(Args)]
pub struct RateArgs {
    /// Blossom uploads per minute (the relay default limit is 30)
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u32).range(1..))]
    uploads_per_minute: u32,
    /// Event publishes and deletes per minute (the relay HTTP limit is 300)
    #[arg(long, default_value_t = 240, value_parser = clap::value_parser!(u32).range(1..))]
    events_per_minute: u32,
    /// Relay queries per minute
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(1..))]
    queries_per_minute: u32,
}

#[derive(Args)]
pub struct FetchArgs {
    /// Items in flight at once; bounds concurrent downloads
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(1..=64))]
    concurrency: u16,
    /// Concurrent downloads per host
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u16).range(1..=64))]
    per_host_concurrency: u16,
    /// Reject images larger than this many bytes
    #[arg(long, default_value_t = 1_048_576)]
    max_bytes: u64,
}

#[derive(Args)]
pub struct PlanArgs {
    #[command(flatten)]
    work: WorkDir,
    /// Slack emoji.list JSON file
    #[arg(
        long,
        conflicts_with = "images_dir",
        required_unless_present = "images_dir"
    )]
    slack_json: Option<PathBuf>,
    /// Directory of images; names come from file names unless --name-map is given
    #[arg(long)]
    images_dir: Option<PathBuf>,
    /// JSON of name to file name (or alias:name) inside --images-dir
    #[arg(long, conflicts_with = "slack_json")]
    name_map: Option<PathBuf>,
    /// File of standard Unicode shortcodes, one per line, to tell alias_to_unicode from alias_dangling
    #[arg(long)]
    unicode_names: Option<PathBuf>,
    /// What to do with a name that already has an entry
    #[arg(long, value_enum, default_value = "skip")]
    on_conflict: OnConflict,
    /// Do not query the relay for existing entries (conflicts go unchecked)
    #[arg(long)]
    no_conflict_check: bool,
    /// Fetch and sanitize the images to report unique blobs, bytes and the time estimate
    #[arg(long)]
    dry_run: bool,
    /// With --dry-run, measure only about this many evenly spaced names and extrapolate
    #[arg(long, requires = "dry_run")]
    sample: Option<usize>,
    #[command(flatten)]
    rates: RateArgs,
    #[command(flatten)]
    fetch: FetchArgs,
}

#[derive(Args)]
pub struct RunArgs {
    #[command(flatten)]
    work: WorkDir,
    /// Start a new journal for a changed plan; the old journal is moved aside, not deleted
    #[arg(long)]
    force_new_plan: bool,
    /// Failed attempts per item and stage before the item is rejected
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=20))]
    max_attempts: u32,
    #[command(flatten)]
    rates: RateArgs,
    #[command(flatten)]
    fetch: FetchArgs,
}

#[derive(Args)]
pub struct VerifyArgs {
    #[command(flatten)]
    work: WorkDir,
    /// HEAD this many random blobs
    #[arg(long, default_value_t = 0)]
    probe_blobs: usize,
    /// Run this many random prefix searches
    #[arg(long, default_value_t = 20)]
    prefix_searches: usize,
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=20))]
    max_attempts: u32,
    #[command(flatten)]
    rates: RateArgs,
}

#[derive(Args)]
pub struct RollbackArgs {
    #[command(flatten)]
    work: WorkDir,
    /// Count what would be deleted and delete nothing
    #[arg(long)]
    dry_run: bool,
    /// Confirm the deletes
    #[arg(long)]
    yes: bool,
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u32).range(1..=20))]
    max_attempts: u32,
    #[command(flatten)]
    rates: RateArgs,
}

pub async fn dispatch(cmd: MigrateCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        MigrateCmd::Plan(a) => cmd_plan(a, client).await,
        MigrateCmd::Run(a) => cmd_run(a, client).await,
        MigrateCmd::Verify(a) => cmd_verify(a, client).await,
        MigrateCmd::Rollback(a) => cmd_rollback(a, client).await,
    }
}

fn other(msg: impl std::fmt::Display) -> CliError {
    CliError::Other(msg.to_string())
}

fn print_json<T: Serialize>(value: &T) -> Result<(), CliError> {
    let out = serde_json::to_string_pretty(value).map_err(other)?;
    println!("{out}");
    Ok(())
}

fn read_capped(path: &Path) -> Result<Vec<u8>, CliError> {
    use std::io::Read;
    let file = std::fs::File::open(path)
        .map_err(|e| CliError::Usage(format!("cannot open {}: {e}", path.display())))?;
    let mut buf = Vec::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| CliError::Usage(format!("cannot read {}: {e}", path.display())))?;
    if buf.len() as u64 > MAX_INPUT_BYTES {
        return Err(CliError::Usage(format!(
            "{} is larger than {MAX_INPUT_BYTES} bytes",
            path.display()
        )));
    }
    Ok(buf)
}

struct Input {
    entries: Vec<RawEntry>,
    sha256: String,
    policy: SourcePolicy,
}

fn read_slack(path: &Path) -> Result<Input, CliError> {
    let bytes = read_capped(path)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| CliError::Usage(format!("{} is not UTF-8: {e}", path.display())))?;
    Ok(Input {
        entries: names::parse_slack_emoji_list(text).map_err(CliError::Usage)?,
        sha256: sha256_hex(&bytes),
        policy: SourcePolicy::Remote,
    })
}

fn read_images_dir(dir: &Path, name_map: Option<&Path>) -> Result<Input, CliError> {
    let dir = dir
        .canonicalize()
        .map_err(|e| CliError::Usage(format!("cannot open {}: {e}", dir.display())))?;
    let mut files: BTreeMap<String, u64> = BTreeMap::new();
    let listing = std::fs::read_dir(&dir).map_err(|e| CliError::Usage(e.to_string()))?;
    for entry in listing {
        let entry = entry.map_err(|e| CliError::Usage(e.to_string()))?;
        let meta = entry
            .metadata()
            .map_err(|e| CliError::Usage(e.to_string()))?;
        let path = entry.path();
        let image = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        if meta.is_file() && image {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                files.insert(file_name.to_string(), meta.len());
            }
        }
    }
    let file_url = |file_name: &str| {
        url::Url::from_file_path(dir.join(file_name))
            .map_or_else(|()| file_name.to_string(), |u| u.to_string())
    };
    let mut fingerprint: String = files.iter().map(|(n, s)| format!("{n}\t{s}\n")).collect();
    let entries = match name_map {
        None => files
            .keys()
            .map(|file_name| RawEntry {
                name: Path::new(file_name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string(),
                value: file_url(file_name),
            })
            .collect(),
        Some(map_path) => {
            let bytes = read_capped(map_path)?;
            fingerprint.push_str(&sha256_hex(&bytes));
            let text = std::str::from_utf8(&bytes).map_err(|e| {
                CliError::Usage(format!("{} is not UTF-8: {e}", map_path.display()))
            })?;
            names::parse_slack_emoji_list(text)
                .map_err(CliError::Usage)?
                .into_iter()
                .map(|e| {
                    let value = if e.value.starts_with("alias:") {
                        e.value
                    } else if files.contains_key(&e.value) {
                        file_url(&e.value)
                    } else {
                        format!("missing-or-unsafe-file:{}", e.value)
                    };
                    RawEntry {
                        name: e.name,
                        value,
                    }
                })
                .collect()
        }
    };
    Ok(Input {
        entries,
        sha256: sha256_hex(fingerprint.as_bytes()),
        policy: SourcePolicy::RemoteOrLocal,
    })
}

fn read_unicode_names(path: Option<&Path>) -> Result<HashSet<String>, CliError> {
    let Some(path) = path else {
        return Ok(HashSet::new());
    };
    let text = String::from_utf8(read_capped(path)?)
        .map_err(|e| CliError::Usage(format!("{} is not UTF-8: {e}", path.display())))?;
    Ok(text
        .lines()
        .map(|l| l.trim().trim_matches(':').to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

fn write_jsonl<T: Serialize>(path: &Path, rows: &[T]) -> Result<(), CliError> {
    let mut buf = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut buf, row).map_err(other)?;
        buf.push(b'\n');
    }
    journal::atomic_write(path, &buf)
        .map_err(|e| other(format!("cannot write {}: {e}", path.display())))
}

fn work_paths(dir: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    (
        dir.join("plan.json"),
        dir.join("rejects.jsonl"),
        dir.join("journal.jsonl"),
        dir.join("snapshot.json"),
        dir.join("blobs"),
    )
}

#[derive(Serialize)]
struct PlanReport {
    names_read: usize,
    items_planned: usize,
    aliases_resolved: usize,
    rejected: usize,
    rejected_by_reason: BTreeMap<String, usize>,
    conflicts_checked: bool,
    existing_entries: usize,
    on_conflict: OnConflict,
    #[serde(skip_serializing_if = "Option::is_none")]
    measured: Option<MeasureReport>,
    estimate_basis: &'static str,
    unique_blobs: u64,
    duplicate_ratio: f64,
    bytes_to_upload: Option<u64>,
    estimate: plan::Estimate,
    plan_sha256: String,
    work_dir: String,
}

#[derive(Serialize)]
struct MeasureReport {
    names_sampled: usize,
    unique_sources: u64,
    unique_blobs: u64,
    unique_bytes: u64,
    fetch_or_image_failures: usize,
    extrapolated: bool,
}

fn count_reasons(rejects: &[Reject]) -> BTreeMap<String, usize> {
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    for r in rejects {
        let key = serde_json::to_value(r.reason)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        *by.entry(key).or_default() += 1;
    }
    by
}

fn sample_items(items: &[PlanItem], sample: Option<usize>) -> Vec<PlanItem> {
    match sample {
        Some(n) if n > 0 && n < items.len() => {
            let stride = items.len().div_ceil(n);
            items.iter().step_by(stride).cloned().collect()
        }
        _ => items.to_vec(),
    }
}

struct PlanOutcome {
    plan: Plan,
    rejects: Vec<Reject>,
    measure_rejects: Vec<Reject>,
    report: PlanReport,
}

/// Build the plan. Reads the relay for conflicts and, with `--dry-run`, the
/// image sources; calls no relay write.
async fn build_plan<R: Relay, F: Fetcher>(
    a: &PlanArgs,
    relay: &R,
    fetcher: &F,
) -> Result<PlanOutcome, CliError> {
    let input = match (&a.slack_json, &a.images_dir) {
        (Some(json), None) => read_slack(json)?,
        (None, Some(dir)) => read_images_dir(dir, a.name_map.as_deref())?,
        _ => {
            return Err(CliError::Usage(
                "give exactly one of --slack-json and --images-dir".into(),
            ))
        }
    };
    let unicode = read_unicode_names(a.unicode_names.as_deref())?;
    let names_read = input.entries.len();
    let (mut items, mut rejects) = names::resolve_entries(&input.entries, input.policy, &unicode);

    let mut existing_entries = 0;
    if !a.no_conflict_check {
        let clock = SystemClock::new();
        let bucket = TokenBucket::new(clock.clone(), a.rates.queries_per_minute);
        let all: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
        let rows = lookup_all(relay, &bucket, &clock, &all, 3)
            .await
            .map_err(other)?;
        let existing: HashSet<String> = rows.into_iter().map(|r| r.name).collect();
        existing_entries = existing.len();
        if a.on_conflict == OnConflict::Skip {
            items.retain(|item| {
                let conflict = existing.contains(&item.name);
                if conflict {
                    rejects.push(Reject {
                        name: item.name.clone(),
                        reason: RejectReason::ConflictSkipped,
                        detail: "an entry with this name already exists".into(),
                    });
                }
                !conflict
            });
        }
    }
    rejects.sort_by(|x, y| x.name.cmp(&y.name));

    let measured = if a.dry_run {
        let sampled = sample_items(&items, a.sample);
        let m = measure(fetcher, &sampled, usize::from(a.fetch.concurrency)).await;
        Some((sampled.len(), m))
    } else {
        None
    };

    let plan = Plan {
        version: PLAN_VERSION,
        input_sha256: input.sha256,
        on_conflict: a.on_conflict,
        items,
    };
    let total = plan.items.len() as u64;
    let mut measure_rejects = Vec::new();
    let (unique_blobs, bytes, basis, measure_report) = match measured {
        None => (total, None, "upper_bound_no_fetch", None),
        Some((sampled, m)) => {
            let extrapolated = sampled < plan.items.len();
            let scale = if m.names_resolved == 0 {
                1.0
            } else {
                total as f64 / m.names_resolved as f64
            };
            let (blobs, bytes) = if extrapolated {
                (
                    (m.unique_blobs as f64 * scale).round() as u64,
                    (m.unique_bytes as f64 * scale).round() as u64,
                )
            } else {
                (m.unique_blobs, m.unique_bytes)
            };
            let report = MeasureReport {
                names_sampled: sampled,
                unique_sources: m.unique_sources,
                unique_blobs: m.unique_blobs,
                unique_bytes: m.unique_bytes,
                fetch_or_image_failures: m.rejects.len(),
                extrapolated,
            };
            measure_rejects = m.rejects;
            let basis = if extrapolated {
                "sampled_extrapolation"
            } else {
                "measured"
            };
            (blobs, Some(bytes), basis, Some(report))
        }
    };
    let report = PlanReport {
        names_read,
        items_planned: plan.items.len(),
        aliases_resolved: plan.items.iter().filter(|i| i.alias_of.is_some()).count(),
        rejected: rejects.len(),
        rejected_by_reason: count_reasons(&rejects),
        conflicts_checked: !a.no_conflict_check,
        existing_entries,
        on_conflict: a.on_conflict,
        measured: measure_report,
        estimate_basis: basis,
        unique_blobs,
        duplicate_ratio: duplicate_ratio(unique_blobs, total),
        bytes_to_upload: bytes,
        estimate: estimate(
            unique_blobs,
            total,
            a.rates.uploads_per_minute,
            a.rates.events_per_minute,
        ),
        plan_sha256: plan.sha256().map_err(other)?,
        work_dir: a.work.work_dir.display().to_string(),
    };
    Ok(PlanOutcome {
        plan,
        rejects,
        measure_rejects,
        report,
    })
}

async fn cmd_plan(a: PlanArgs, client: &BuzzClient) -> Result<(), CliError> {
    let fetcher = HttpFetcher::new(
        a.fetch.max_bytes,
        usize::from(a.fetch.per_host_concurrency),
        FetchPolicy::default(),
    )?;
    let out = build_plan(&a, &BuzzPorts { client }, &fetcher).await?;
    let (plan_path, rejects_path, ..) = work_paths(&a.work.work_dir);
    std::fs::create_dir_all(&a.work.work_dir).map_err(other)?;
    let plan_bytes = serde_json::to_vec_pretty(&out.plan).map_err(other)?;
    journal::atomic_write(&plan_path, &plan_bytes).map_err(other)?;
    write_jsonl(&rejects_path, &out.rejects)?;
    if !out.measure_rejects.is_empty() {
        write_jsonl(
            &a.work.work_dir.join("dry-run-rejects.jsonl"),
            &out.measure_rejects,
        )?;
    }
    print_json(&out.report)
}

fn load_plan(dir: &Path) -> Result<(Plan, String), CliError> {
    let path = dir.join("plan.json");
    let bytes = read_capped(&path)?;
    let plan: Plan = serde_json::from_slice(&bytes)
        .map_err(|e| CliError::Usage(format!("{} is not a plan: {e}", path.display())))?;
    if plan.version != PLAN_VERSION {
        return Err(CliError::Usage(format!(
            "{} has version {}, this build reads version {PLAN_VERSION}",
            path.display(),
            plan.version
        )));
    }
    let sha = plan.sha256().map_err(other)?;
    Ok((plan, sha))
}

fn rates_of(r: &RateArgs) -> Rates {
    Rates {
        uploads_per_minute: r.uploads_per_minute,
        events_per_minute: r.events_per_minute,
        queries_per_minute: r.queries_per_minute,
    }
}

fn run_config(dir: &Path, concurrency: usize, max_attempts: u32) -> RunConfig {
    let (_, _, _, snapshot, blobs) = work_paths(dir);
    RunConfig {
        concurrency,
        upload_concurrency: 2,
        max_attempts,
        blob_dir: blobs,
        snapshot_path: Some(snapshot),
    }
}

async fn cmd_run(a: RunArgs, client: &BuzzClient) -> Result<(), CliError> {
    let (plan, plan_sha) = load_plan(&a.work.work_dir)?;
    let (.., journal_path, _, _) = work_paths(&a.work.work_dir);
    let names: Vec<String> = plan.items.iter().map(|i| i.name.clone()).collect();
    let opened = journal::open_file(
        &journal_path,
        &plan_sha,
        &plan.input_sha256,
        &names,
        a.force_new_plan,
    )
    .map_err(other)?;
    if opened.resumed {
        eprintln!("resuming from {}", journal_path.display());
    }
    let fetcher = HttpFetcher::new(
        a.fetch.max_bytes,
        usize::from(a.fetch.per_host_concurrency),
        FetchPolicy::default(),
    )?;
    let ports = BuzzPorts { client };
    let migrator = Migrator::new(
        &fetcher,
        &ports,
        &ports,
        SystemClock::new(),
        opened.journal,
        rates_of(&a.rates),
        run_config(
            &a.work.work_dir,
            usize::from(a.fetch.concurrency),
            a.max_attempts,
        ),
    );
    let summary = migrator.run(&plan.items).await.map_err(other)?;
    let rejected = migrator.rejected();
    write_jsonl(&a.work.work_dir.join("run-rejects.jsonl"), &rejected)?;
    print_json(&summary)?;
    if rejected.is_empty() {
        Ok(())
    } else {
        Err(other(format!(
            "{} items were rejected; see {}",
            rejected.len(),
            a.work.work_dir.join("run-rejects.jsonl").display()
        )))
    }
}

fn open_existing<'a>(
    dir: &Path,
    plan: &Plan,
    plan_sha: &str,
    ports: &'a BuzzPorts<'a>,
    fetcher: &'a HttpFetcher,
    rates: Rates,
    max_attempts: u32,
) -> Result<
    Migrator<'a, HttpFetcher, BuzzPorts<'a>, BuzzPorts<'a>, SystemClock, journal::FileStorage>,
    CliError,
> {
    let (.., journal_path, _, _) = work_paths(dir);
    if !journal_path.exists() {
        return Err(CliError::Usage(format!(
            "{} does not exist; run `emoji migrate run` first",
            journal_path.display()
        )));
    }
    let names: Vec<String> = plan.items.iter().map(|i| i.name.clone()).collect();
    let opened = journal::open_file(&journal_path, plan_sha, &plan.input_sha256, &names, false)
        .map_err(other)?;
    Ok(Migrator::new(
        fetcher,
        ports,
        ports,
        SystemClock::new(),
        opened.journal,
        rates,
        run_config(dir, 4, max_attempts),
    ))
}

async fn cmd_verify(a: VerifyArgs, client: &BuzzClient) -> Result<(), CliError> {
    let (plan, plan_sha) = load_plan(&a.work.work_dir)?;
    let fetcher = HttpFetcher::new(1, 1, FetchPolicy::default())?;
    let ports = BuzzPorts { client };
    let migrator = open_existing(
        &a.work.work_dir,
        &plan,
        &plan_sha,
        &ports,
        &fetcher,
        rates_of(&a.rates),
        a.max_attempts,
    )?;
    let report = migrator
        .verify(a.probe_blobs, a.prefix_searches)
        .await
        .map_err(other)?;
    print_json(&report)?;
    if report.failures.is_empty() {
        Ok(())
    } else {
        Err(other(format!(
            "verify found {} failures",
            report.failures.len()
        )))
    }
}

async fn cmd_rollback(a: RollbackArgs, client: &BuzzClient) -> Result<(), CliError> {
    let (plan, plan_sha) = load_plan(&a.work.work_dir)?;
    let fetcher = HttpFetcher::new(1, 1, FetchPolicy::default())?;
    let ports = BuzzPorts { client };
    let migrator = open_existing(
        &a.work.work_dir,
        &plan,
        &plan_sha,
        &ports,
        &fetcher,
        rates_of(&a.rates),
        a.max_attempts,
    )?;
    let count = migrator.rollback(true).await.map_err(other)?.deleted;
    if a.dry_run {
        return print_json(&serde_json::json!({ "would_delete": count }));
    }
    if !a.yes {
        return Err(CliError::Usage(format!(
            "rollback would delete {count} entries; re-run with --yes to delete them, or --dry-run to only count"
        )));
    }
    let report = migrator.rollback(false).await.map_err(other)?;
    print_json(&report)?;
    if report.failures.is_empty() {
        Ok(())
    } else {
        Err(other(format!(
            "rollback failed for {} names",
            report.failures.len()
        )))
    }
}
