//! Where an install's output goes: the install log file (complete history) and
//! the live output line in the UI (current progress).
//!
//! Both destinations hang off the same drain seam in
//! [`super::install_capture`], and both are best-effort: an install must never
//! fail because a log write or an event emit did.
//!
//! [`InstallReporter`] owns two explicit lifecycles, because both the log and
//! the live line are meaningless without a notion of "this run":
//!
//! * a **log session**, started once per run, which keeps the previous run's
//!   file as `.1` and writes this run's header; and
//! * a **live-event sequence**, monotonic across the whole install, which is
//!   what lets the UI drop a superseded line. A per-command retry number
//!   cannot do that job — it restarts at 1 for every step.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::install_capture::{LineObserver, Throttle};
use crate::managed_agents::{InstallRuntimeResult, InstallStepResult};

/// One install command's result: what the UI shows, and the log-scale copy of
/// the same output for the log file.
pub(super) struct InstallOutcome {
    pub(super) step: InstallStepResult,
    pub(super) log_stdout: String,
    pub(super) log_stderr: String,
}

impl InstallOutcome {
    /// A step Buzz synthesized rather than ran — a failed prerequisite, or the
    /// post-install verification. Its own message is the whole record.
    pub(super) fn synthesized(step: InstallStepResult) -> Self {
        Self {
            log_stdout: step.stdout.clone(),
            log_stderr: step.stderr.clone(),
            step,
        }
    }
}

/// Payload of the `acp-install-output` event.
///
/// `seq` is monotonic across the entire install, so the UI can drop a line a
/// later step or attempt has already superseded. The retry number cannot serve
/// as that key: it restarts at 1 for every step, so a step that succeeded on
/// attempt 2 would make the next step's attempt-1 output look stale and freeze
/// the display.
///
/// `line: None` is the *start signal*: an attempt is beginning and the displayed
/// line must clear now. It is emitted unthrottled, because the point is that
/// stale output stops being shown before the new work prints anything.
#[derive(Serialize, Clone, Debug)]
pub(super) struct InstallOutputEvent {
    pub(super) runtime_id: String,
    pub(super) seq: u64,
    pub(super) line: Option<String>,
}

/// Emits one live output event. Boxed rather than holding an `AppHandle` so the
/// reporter is constructible — and assertable — without a Tauri app.
type EmitEvent = Arc<dyn Fn(InstallOutputEvent) + Send + Sync>;

/// Literal secret values scrubbed out of everything this module publishes, in
/// addition to the shapes [`crate::managed_agents::redact_secrets_with`]
/// recognises on its own.
type Secrets = Arc<Vec<String>>;

/// At most four live-output events per second. Coalescing *holds* the newest
/// line rather than dropping it: a burst that ends just before the window
/// closes would otherwise leave the display showing a line the install had
/// already moved past.
const LIVE_LINE_INTERVAL: Duration = Duration::from_millis(250);

pub(super) struct InstallReporter {
    log: Option<InstallLog>,
    /// `None` when nothing is listening, which is also what makes
    /// [`InstallReporter::line_observer`] `None` — the drain then skips line
    /// reassembly entirely instead of doing it for no one.
    live: Option<Live>,
    secrets: Secrets,
}

impl Drop for InstallReporter {
    /// Serialise deactivation against in-flight publications: take the
    /// exclusive lifecycle write lock, mark the run as inactive, and return —
    /// all before the per-runtime concurrency guard releases.
    ///
    /// Every drain thread holds the shared read guard from its admission check
    /// through its `(self.emit)(...)` call, so the write lock here blocks until
    /// every in-flight publication has finished.  After this returns, `active`
    /// is `false` under an exclusive write, and any thread that attempts a new
    /// `offer` will read `false` under a read lock and return without emitting.
    ///
    /// The ordering guarantee: `reporter` is declared after `_guard` in
    /// `install_acp_runtime_blocking` (line 311 vs line 306), so Rust drops
    /// `reporter` first in reverse-declaration order — deactivation completes
    /// before the runtime guard releases and a new install can start.
    fn drop(&mut self) {
        if let Some(live) = &self.live {
            if let Ok(mut active) = live.lifecycle.write() {
                *active = false;
            }
        }
    }
}

impl InstallReporter {
    /// The reporter a real install run uses: it starts this run's log session
    /// and emits live output events through `app`.
    ///
    /// `runtime_id` must already be the canonical id from the runtime catalog —
    /// the log path is built from it, so resolving it first is what keeps a raw
    /// command argument out of a filename.
    ///
    /// A log that cannot be resolved or opened degrades to no log rather than
    /// failing the install: a user with a broken app-data directory still needs
    /// the install itself to work.
    pub(super) fn for_run(app: &tauri::AppHandle, runtime_id: &str) -> Self {
        // Read from the app's own package info rather than the frontend's
        // `getVersion` plugin call: the header is written on the Rust side, and
        // this cannot fail or be mocked out from under the log.
        let app_version = app.package_info().version.to_string();
        let log = crate::managed_agents::storage::install_log_path(app, runtime_id)
            .ok()
            .and_then(|path| InstallLog::start(&path, runtime_id, &app_version));
        let app = app.clone();
        let emit: EmitEvent = Arc::new(move |event| {
            use tauri::Emitter;
            let _ = app.emit("acp-install-output", event);
        });
        Self::new(runtime_id, log, Some(emit))
    }

    fn new(runtime_id: &str, log: Option<InstallLog>, emit: Option<EmitEvent>) -> Self {
        // Snapshot the environment's secrets once, at construction: the install
        // inherits this environment, so anything it echoes came from here.
        Self::with_secrets(runtime_id, log, emit, env_secret_values())
    }

    /// The reporter over an explicit secret set, which is what makes the
    /// scrubbing assertable: a test can name a proxy credential without
    /// exporting a real `HTTPS_PROXY` into the process every HTTP client in the
    /// suite would then read.
    fn with_secrets(
        runtime_id: &str,
        log: Option<InstallLog>,
        emit: Option<EmitEvent>,
        secrets: Vec<String>,
    ) -> Self {
        let secrets: Secrets = Arc::new(secrets);
        let live = emit.map(|emit| Live {
            runtime_id: Arc::from(runtime_id),
            emit,
            throttle: Arc::new(Throttle::new(LIVE_LINE_INTERVAL)),
            seq: Arc::new(AtomicU64::new(0)),
            lifecycle: Arc::new(RwLock::new(true)),
            secrets: Arc::clone(&secrets),
        });
        Self { log, live, secrets }
    }

    /// The log file to point the user at, or `None` when this run has no log —
    /// the failure message then omits the pointer rather than naming a file
    /// that does not exist.
    pub(super) fn log_path(&self) -> Option<String> {
        Some(self.log.as_ref()?.path.display().to_string())
    }

    /// A failed install carrying the steps recorded so far and the log holding
    /// their full history. Every early return in the install shapes its result
    /// here, so none can forget the log pointer the failure message needs.
    pub(super) fn failed(&self, steps: Vec<InstallStepResult>) -> InstallRuntimeResult {
        InstallRuntimeResult {
            success: false,
            steps,
            restarted_count: 0,
            failed_restart_count: 0,
            log_path: self.log_path(),
        }
    }

    /// Mark the start of one executed attempt: clear whatever line the previous
    /// attempt left on screen, and start this attempt's clock.
    ///
    /// The clear is emitted unthrottled and reopens the rate window, so the new
    /// attempt's first line cannot be swallowed by the previous attempt's.
    /// Without this signal the prior attempt's last line — typically the failure
    /// that caused the retry — sits under the spinner through the backoff and
    /// through a silent next attempt.
    pub(super) fn start_attempt(&self) {
        if let Some(log) = &self.log {
            log.mark_attempt_start();
        }
        if let Some(live) = &self.live {
            live.throttle.restart();
            live.publish(None);
        }
    }

    /// Observer for one attempt's drains, or `None` when nothing is listening.
    pub(super) fn line_observer(&self) -> Option<LineObserver> {
        let live = self.live.clone()?;
        Some(Arc::new(move |line: &str| live.offer(line)))
    }

    /// Record one executed attempt of a step, returning the step with secrets
    /// scrubbed out of the output the UI will render.
    ///
    /// The scrub happens here rather than at the construction sites because
    /// every executed step reaches the caller through this function — the
    /// timeout path, the status-check failure, and the ordinary exit all build
    /// their `InstallStepResult` straight from the captures.
    pub(super) fn record_attempt(
        &self,
        attempt: u32,
        outcome: InstallOutcome,
    ) -> InstallStepResult {
        // The drains are finished, so a line the throttle is still holding is
        // this attempt's last and nothing is coming to replace it.
        if let Some(live) = &self.live {
            live.flush_pending();
        }
        self.write_record(Some(attempt), &outcome);
        self.redacted_step(outcome.step)
    }

    /// Push a synthesized step onto `steps` and record it. Routing every step
    /// through here is what keeps the log complete: a step that reaches the UI
    /// without passing this function is invisible in the file.
    pub(super) fn record_step(&self, steps: &mut Vec<InstallStepResult>, step: InstallStepResult) {
        self.write_record(None, &InstallOutcome::synthesized(step.clone()));
        steps.push(self.redacted_step(step));
    }

    /// Scrub the frontend-visible fields of a step. The failure message the UI
    /// builds renders `stderr`/`stdout` and the hint verbatim, so they need the
    /// same scrubbing as the log record and the live line — the log is not the
    /// only place an install's output is read.
    fn redacted_step(&self, mut step: InstallStepResult) -> InstallStepResult {
        step.command = redact(&step.command, &self.secrets);
        step.stdout = redact(&step.stdout, &self.secrets);
        step.stderr = redact(&step.stderr, &self.secrets);
        step.hint = step.hint.map(|hint| redact(&hint, &self.secrets));
        step
    }

    /// Append one record. Best-effort by contract: a full disk or a revoked
    /// permission degrades the diagnostics, it does not fail the install.
    fn write_record(&self, attempt: Option<u32>, outcome: &InstallOutcome) {
        let Some(log) = &self.log else {
            return;
        };
        log.append(&render_record(
            attempt,
            log.take_attempt_elapsed(),
            outcome,
            &self.secrets,
        ));
    }
}

/// The shared half of the reporter — everything a drain thread's observer needs,
/// owned rather than borrowed so an observer can outlive the call that made it.
#[derive(Clone)]
struct Live {
    runtime_id: Arc<str>,
    emit: EmitEvent,
    throttle: Arc<Throttle>,
    seq: Arc<AtomicU64>,
    /// Lifecycle lock: `true` while the run is active, `false` once
    /// `InstallReporter` has been dropped.
    ///
    /// Drain threads hold a **shared read guard** from the admission check
    /// through the `(self.emit)(...)` call, making the admit-and-publish pair
    /// atomic with respect to deactivation. `InstallReporter::drop` takes the
    /// **exclusive write guard** and sets the value to `false`; this blocks
    /// until every in-flight publication finishes, then prevents any new
    /// publications from starting. The write lock is held only for the flag
    /// store and is released before the per-runtime concurrency guard drops,
    /// so its duration is bounded by the time a single `emit` call takes —
    /// microseconds to low milliseconds for the Tauri IPC broadcast.
    lifecycle: Arc<RwLock<bool>>,
    secrets: Secrets,
}

impl Live {
    /// Offer one drained line to the rate limiter, emitting it if the window is
    /// open and holding it as the newest pending line if not.
    ///
    /// The read guard is held from the admission check through the emit call so
    /// that `InstallReporter::drop`'s write lock must wait for any in-flight
    /// publication to complete before deactivating.  This makes the
    /// check-then-emit pair atomic with respect to shutdown.
    fn offer(&self, line: &str) {
        let Ok(guard) = self.lifecycle.read() else {
            return;
        };
        if !*guard {
            return;
        }
        if let Some(line) = self.throttle.offer(line, Instant::now()) {
            self.publish_under_guard(line);
        }
        // `guard` drops here, releasing the read lock after publication.
    }

    fn flush_pending(&self) {
        let Ok(guard) = self.lifecycle.read() else {
            return;
        };
        if !*guard {
            return;
        }
        if let Some(line) = self.throttle.take_pending() {
            self.publish_under_guard(line);
        }
        // `guard` drops here, releasing the read lock after publication.
    }

    /// Emit `line` now, bypassing the rate window and the lifecycle lock.
    ///
    /// Only called from `InstallReporter` methods that run on the reporter
    /// itself (never from detached drain threads), so no lifecycle guard is
    /// needed — the reporter is alive by definition when its own methods run.
    fn publish(&self, line: Option<String>) {
        (self.emit)(InstallOutputEvent {
            runtime_id: self.runtime_id.to_string(),
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            line: line.map(|line| redact(&line, &self.secrets)),
        });
    }

    /// Publish `line` while already holding a read guard on `lifecycle`.  The
    /// caller is responsible for checking `active` before calling this.
    fn publish_under_guard(&self, line: String) {
        (self.emit)(InstallOutputEvent {
            runtime_id: self.runtime_id.to_string(),
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            line: Some(redact(&line, &self.secrets)),
        });
    }
}

/// This run's log file: one session, opened once, appended to per record.
struct InstallLog {
    path: PathBuf,
    /// When the attempt currently running started, so its record can name its
    /// own duration. A 15-minute ceiling is only diagnosable if the file says
    /// how long each attempt actually took.
    attempt_start: Mutex<Option<Instant>>,
}

impl InstallLog {
    /// Start this run's session, or `None` if the file cannot be opened.
    ///
    /// Rotation happens here, once per run, rather than per record: a run either
    /// gets its own file or it gets no log at all, so two runs are never
    /// interleaved in one file.
    ///
    /// The header identifies the environment the run happened in, not just the
    /// run: a Windows install failure and a macOS one on the same runtime are
    /// different bugs, and a stale app version explains a failure that no longer
    /// reproduces.
    fn start(path: &Path, runtime_id: &str, app_version: &str) -> Option<Self> {
        let mut file = crate::managed_agents::storage::start_install_log_session(path).ok()?;
        let _ = file.write_all(
            format!(
                "=== install run runtime={runtime_id} app={app_version} os={} started={}\n",
                std::env::consts::OS,
                chrono::Utc::now().to_rfc3339()
            )
            .as_bytes(),
        );
        Some(Self {
            path: path.to_path_buf(),
            attempt_start: Mutex::new(None),
        })
    }

    fn mark_attempt_start(&self) {
        if let Ok(mut start) = self.attempt_start.lock() {
            *start = Some(Instant::now());
        }
    }

    /// How long the attempt being recorded ran, consumed so a later record
    /// cannot reuse it. `None` for a synthesized step, which never ran.
    fn take_attempt_elapsed(&self) -> Option<Duration> {
        Some(self.attempt_start.lock().ok()?.take()?.elapsed())
    }

    fn append(&self, record: &str) {
        if let Ok(mut file) = crate::managed_agents::storage::open_install_log_file(&self.path) {
            let _ = file.write_all(record.as_bytes());
        }
    }
}

/// One self-contained record. Each is capped independently by the log-scale
/// capture that produced it, so an early attempt that printed megabytes cannot
/// push a later attempt — or the verification step that explains the failure —
/// out of the file.
fn render_record(
    attempt: Option<u32>,
    elapsed: Option<Duration>,
    outcome: &InstallOutcome,
    secrets: &Secrets,
) -> String {
    let step = &outcome.step;
    let attempt = attempt.map_or_else(|| "-".to_string(), |n| n.to_string());
    let exit = step
        .exit_code
        .map_or_else(|| "none".to_string(), |code| code.to_string());
    let elapsed = elapsed.map_or_else(
        || "-".to_string(),
        |elapsed| format!("{:.1}s", elapsed.as_secs_f64()),
    );
    let mut record = format!(
        "=== {} step={} attempt={attempt} success={} exit={exit} elapsed={elapsed}\n$ {}\n",
        chrono::Utc::now().to_rfc3339(),
        step.step,
        step.success,
        redact(&step.command, secrets),
    );
    for (label, text) in [
        ("stdout", &outcome.log_stdout),
        ("stderr", &outcome.log_stderr),
    ] {
        if !text.trim().is_empty() {
            record.push_str(&format!("--- {label} ---\n{}\n", redact(text, secrets)));
        }
    }
    if let Some(hint) = &step.hint {
        record.push_str(&format!("--- hint ---\n{}\n", redact(hint, secrets)));
    }
    record
}

/// Scrub secrets before anything reaches disk or the UI. The log is written
/// unattended and the live line is rendered verbatim, so scrubbing happens at
/// the write, not at the read.
fn redact(text: &str, secrets: &Secrets) -> String {
    let extras: Vec<&str> = secrets.iter().map(String::as_str).collect();
    crate::managed_agents::redact_secrets_with(text, &extras)
}

/// Values of environment variables whose *name* marks them as secret.
///
/// An install inherits Buzz's environment and installers echo it back — npm
/// prints the resolved registry config on an auth failure, and a shell that
/// traces its commands prints every expansion. Without this, only the
/// hard-coded key shapes would be scrubbed, so a plain `NPM_TOKEN` or
/// `ANTHROPIC_API_KEY` would land in the file in clear text.
fn env_secret_values() -> Vec<String> {
    secret_values_from(std::env::vars())
}

/// The secret-bearing part of each variable that carries one.
///
/// Split from [`env_secret_values`] so the classification is assertable without
/// mutating the process environment — setting a real `HTTPS_PROXY` in a test
/// would be read by every HTTP client the rest of the suite builds.
///
/// Three kinds of variable are recognised, because they need different
/// treatment:
///
/// * **URL-valued variables**, where only the userinfo is the credential;
/// * **exactly named credentials**, whose whole value is the secret; and
/// * **name-marked secrets**, matched by a marker substring.
fn secret_values_from(vars: impl IntoIterator<Item = (String, String)>) -> Vec<String> {
    vars.into_iter()
        .filter_map(|(name, value)| {
            let name = name.to_ascii_uppercase();
            if URL_CREDENTIAL_VAR_NAMES.contains(&name.as_str()) {
                // Only the userinfo, so the endpoint itself stays named in the
                // record: an install that fails against a proxy or a private
                // registry is diagnosable only if the log still says which one
                // it went through, and the host and port are not the secret.
                // Redacting the whole value would erase that while protecting
                // nothing more.
                return url_userinfo(&value).map(str::to_string);
            }
            if SECRET_VAR_NAMES.contains(&name.as_str()) {
                // Deliberately not subject to the 8-byte floor below: an exact
                // name is a fact, not the guess the marker rule makes, so there
                // is nothing for a floor to protect against. npm's one-time
                // password is six digits and is a credential at that length —
                // short, but still above the four-byte minimum
                // [`crate::managed_agents::redact_secrets_with`] applies, so it
                // survives to be scrubbed.
                return (!value.is_empty()).then_some(value);
            }
            // A value under 8 bytes is more likely a flag like `true` or a
            // version than a credential, and scrubbing those makes ordinary
            // output unreadable.
            (value.len() >= 8 && name_marks_secret(&name)).then_some(value)
        })
        .collect()
}

/// Variables whose value is a URL that may embed a credential in its userinfo.
///
/// npm's own `npm_config_*` aliases are here too: npm resolves them ahead of
/// the conventional names and echoes the result from `npm config list`, and
/// none of these names carries a marker [`name_marks_secret`] would catch.
/// Matching is case-insensitive because the caller uppercases the name first,
/// which is what npm's lowercase spelling needs.
const URL_CREDENTIAL_VAR_NAMES: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NPM_CONFIG_PROXY",
    "NPM_CONFIG_HTTPS_PROXY",
    "NPM_CONFIG_REGISTRY",
];

/// Variables whose whole value is a credential, recognised by exact name.
///
/// These are npm's supported credential settings, whose names carry no marker
/// [`name_marks_secret`] would catch. They are listed exactly rather than
/// matched on `KEY` or `AUTH` substrings: those occur throughout an ordinary
/// environment, and scrubbing on them would delete unrelated values from the
/// whole log.
///
/// * `NPM_CONFIG_KEY` — the PEM client key used to reach a registry.
/// * `NPM_CONFIG__AUTH` — the base64 basic-auth blob (npm's own double
///   underscore, matching the `_auth` setting).
/// * `NPM_CONFIG_OTP` — the registry one-time password.
const SECRET_VAR_NAMES: &[&str] = &["NPM_CONFIG_KEY", "NPM_CONFIG__AUTH", "NPM_CONFIG_OTP"];

/// Whether an environment variable's name marks its value as a credential.
///
/// Keyed on the name because a secret's *value* has no reliable shape. The
/// markers avoid substrings that occur in non-secret names: `AUTH` is left out
/// because it matches `GIT_AUTHOR_NAME`, whose value is a person's name, and
/// personal access tokens match on `_PAT` as a *suffix* rather than a substring
/// — `contains("_PAT")` would match every `*_PATH` variable on the system and
/// scrub directory names out of the whole log.
fn name_marks_secret(name: &str) -> bool {
    const SECRET_NAME_MARKERS: &[&str] = &[
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "APIKEY",
        "API_KEY",
        "PRIVATE_KEY",
        "ACCESS_KEY",
        "CREDENTIAL",
    ];
    name.ends_with("_PAT")
        || SECRET_NAME_MARKERS
            .iter()
            .any(|marker| name.contains(marker))
}

/// The `user:password` credential embedded in a URL, if it has one.
///
/// Parsed rather than pattern-matched so a URL with no credential — the common
/// case — contributes nothing to scrub. The last `@` in the
/// authority separates userinfo from host, so a password containing an
/// encoded `@` still splits correctly.
///
/// A bare username with no password is not treated as a credential: it is not
/// secret on its own, and scrubbing it would erase every occurrence of a word
/// like `user` from the whole record.
fn url_userinfo(value: &str) -> Option<&str> {
    let authority = value
        .split_once("://")?
        .1
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let userinfo = authority.rsplit_once('@')?.0;
    userinfo.contains(':').then_some(userinfo)
}

#[cfg(test)]
#[path = "install_report_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "install_report_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "install_report_redaction_tests.rs"]
mod redaction_tests;
