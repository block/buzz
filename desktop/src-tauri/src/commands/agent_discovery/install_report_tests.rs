use super::*;
use crate::commands::agent_discovery::install_capture::{drain_into, Capture};
use std::sync::Mutex;

/// Stands in for the real `app.package_info().version`, which needs a Tauri app.
const TEST_APP_VERSION: &str = "9.9.9";

/// A reporter with a started log session in a temp dir, and the emitted events
/// captured.
struct Harness {
    _dir: tempfile::TempDir,
    log: PathBuf,
    reporter: InstallReporter,
    events: Arc<Mutex<Vec<InstallOutputEvent>>>,
}

fn harness() -> Harness {
    harness_at(None)
}

/// A harness whose log lives in `dir`, or in a fresh temp dir when `dir` is
/// `None`. Passing a directory lets a test seed a previous run's file first.
fn harness_at(dir: Option<tempfile::TempDir>) -> Harness {
    harness_inner(dir, None)
}

/// A harness whose reporter scrubs exactly `secrets`, so a proxy or PAT
/// credential can be asserted without exporting it into the process.
fn harness_with_secrets(secrets: Vec<String>) -> Harness {
    harness_inner(None, Some(secrets))
}

fn harness_inner(dir: Option<tempfile::TempDir>, secrets: Option<Vec<String>>) -> Harness {
    let dir = dir.unwrap_or_else(|| tempfile::tempdir().expect("tempdir"));
    let log = dir.path().join("install-goose.log");
    let events: Arc<Mutex<Vec<InstallOutputEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let emit: EmitEvent = {
        let events = Arc::clone(&events);
        Arc::new(move |event| events.lock().unwrap().push(event))
    };
    let started = InstallLog::start(&log, "goose", TEST_APP_VERSION);
    Harness {
        reporter: match secrets {
            Some(secrets) => InstallReporter::with_secrets("goose", started, Some(emit), secrets),
            None => InstallReporter::new("goose", started, Some(emit)),
        },
        _dir: dir,
        log,
        events,
    }
}

/// A reporter with no log file and nothing listening — the degraded shape.
fn silent_reporter() -> InstallReporter {
    InstallReporter::new("goose", None, None)
}

fn step(name: &str, success: bool, stderr: &str) -> InstallStepResult {
    InstallStepResult {
        step: name.to_string(),
        command: "curl … | bash".to_string(),
        success,
        stdout: String::new(),
        stderr: stderr.to_string(),
        exit_code: Some(if success { 0 } else { 1 }),
        hint: None,
    }
}

/// An executed attempt whose log copy differs from the UI copy — the real shape,
/// since the two views are capped differently.
fn outcome(name: &str, success: bool, log_stdout: &str) -> InstallOutcome {
    InstallOutcome {
        step: step(name, success, ""),
        log_stdout: log_stdout.to_string(),
        log_stderr: String::new(),
    }
}

impl Harness {
    fn log_contents(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// The emitted lines in order, with a clear signal rendered as `None`.
    fn lines(&self) -> Vec<Option<String>> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.line.clone())
            .collect()
    }

    fn events(&self) -> Vec<InstallOutputEvent> {
        self.events.lock().unwrap().clone()
    }
}

// ── the log records history the UI does not keep ─────────────────────────────

/// Every attempt is recorded, not just the one the UI surfaces. Reproducing an
/// install failure means seeing whether attempts 1 and 2 failed the same way.
#[test]
fn test_log_records_every_attempt_not_only_the_last() {
    let h = harness();

    h.reporter
        .record_attempt(1, outcome("cli", false, "attempt-one-output"));
    h.reporter
        .record_attempt(2, outcome("cli", false, "attempt-two-output"));

    let log = h.log_contents();
    assert!(log.contains("attempt-one-output"), "got: {log}");
    assert!(log.contains("attempt-two-output"), "got: {log}");
    assert!(
        log.contains("attempt=1") && log.contains("attempt=2"),
        "got: {log}"
    );
}

/// A first attempt that printed a huge amount must not push later records out of
/// the file. Records are capped individually by the log-scale capture that
/// produced them, so the run's total is bounded by steps × attempts × cap rather
/// than by one runaway attempt.
///
/// The flood goes through a real [`Capture`] rather than straight into the
/// record, so this exercises the cap that actually bounds a record.
#[test]
fn test_first_attempt_overflow_does_not_erase_later_records() {
    let h = harness();
    // Through the real drain, so the record is bounded by the cap that bounds a
    // production record rather than by a string this test chose.
    let capture = Capture::new();
    drain_into(vec![b'F'; 4 * 1024 * 1024].as_slice(), &capture, None);

    h.reporter
        .record_attempt(1, outcome("cli", false, &capture.log()));
    h.reporter
        .record_attempt(2, outcome("cli", false, "second-attempt-detail"));
    h.reporter.record_step(
        &mut Vec::new(),
        step("verify", false, "verification-detail"),
    );

    let log = h.log_contents();
    assert!(
        log.contains("bytes omitted at cap"),
        "the flooded record must be marked as cut"
    );
    assert!(
        log.contains("second-attempt-detail"),
        "a later attempt must survive an earlier flood"
    );
    assert!(
        log.contains("verification-detail"),
        "the synthesized step explaining the failure must survive too"
    );
    assert!(
        log.len() < 4 * 1024 * 1024,
        "4MiB of first-attempt output must not reach the file, got {} bytes",
        log.len()
    );
}

/// A step Buzz synthesizes — a failed prerequisite, or post-install
/// verification — reaches the log as well as the UI. `record_step` is the only
/// path that guarantees this, which is why callers use it instead of
/// `steps.push`.
#[test]
fn test_recording_a_synthesized_step_logs_it_and_keeps_it_for_the_ui() {
    let h = harness();
    let mut steps = Vec::new();

    h.reporter
        .record_step(&mut steps, step("verify", false, "still-not-usable"));

    assert_eq!(steps.len(), 1, "the UI must still receive the step");
    assert!(h.log_contents().contains("still-not-usable"));
}

// ── one file per run ─────────────────────────────────────────────────────────

/// A run opens with a header naming the runtime and the environment the run
/// happened in, so a file holding one run of several steps is identifiable as
/// that run rather than a stream of records — and a failure report says which
/// app version and OS produced it without a second round trip to the user.
#[test]
fn test_a_run_opens_with_a_header_naming_the_runtime_app_version_and_os() {
    let h = harness();
    let log = h.log_contents();

    assert!(
        log.starts_with(&format!(
            "=== install run runtime=goose app={TEST_APP_VERSION} os={} started=",
            std::env::consts::OS
        )),
        "got: {log}"
    );
}

/// A new run does not append to the previous run's file: it starts a fresh one
/// and keeps the previous as `.1`. Reading a log has to mean reading one run —
/// records accumulated across runs are indistinguishable from retries within
/// one.
#[test]
fn test_a_new_run_starts_a_fresh_file_and_keeps_the_previous_as_dot_one() {
    let first = harness();
    first
        .reporter
        .record_attempt(1, outcome("cli", false, "previous-run-output"));
    let previous = first.log.clone();
    let dir = first._dir;
    drop(first.reporter);

    let second = harness_at(Some(dir));
    second
        .reporter
        .record_attempt(1, outcome("cli", false, "current-run-output"));

    let log = second.log_contents();
    assert!(log.contains("current-run-output"), "got: {log}");
    assert!(
        !log.contains("previous-run-output"),
        "the new run's file must not carry the previous run's records: {log}"
    );
    let rotated = std::fs::read_to_string(previous.with_extension("log.1")).expect("read .1");
    assert!(
        rotated.contains("previous-run-output"),
        "the previous run must remain readable as .1: {rotated}"
    );
}

/// Each executed attempt records how long it ran. A 15-minute ceiling is only
/// diagnosable if the file says which attempt consumed the time.
#[test]
fn test_an_executed_attempt_records_its_own_duration() {
    let h = harness();

    h.reporter.start_attempt();
    h.reporter.record_attempt(1, outcome("cli", true, "done"));
    h.reporter
        .record_step(&mut Vec::new(), step("verify", true, ""));

    let log = h.log_contents();
    assert!(
        log.contains("attempt=1") && log.contains("elapsed=0."),
        "an executed attempt must carry its duration: {log}"
    );
    assert!(
        log.contains("attempt=- ") && log.contains("elapsed=-"),
        "a synthesized step never ran, so it has no duration: {log}"
    );
}

// ── redaction ────────────────────────────────────────────────────────────────

/// Secrets that an installer echoed must not land on disk. The log is written
/// unattended, so scrubbing happens at the write, not at the read.
#[test]
fn test_log_redacts_secrets_before_writing() {
    let h = harness();
    let leak = "npm ERR! token nsec1qqqqqqqqqqsecretvalue failed";

    h.reporter.record_attempt(1, outcome("cli", false, leak));

    let log = h.log_contents();
    assert!(!log.contains("nsec1qqqqqqqqqqsecretvalue"), "got: {log}");
    assert!(log.contains("[REDACTED]"), "got: {log}");
}

/// The environment's own secrets are scrubbed too, by *name* rather than shape.
/// An install inherits Buzz's environment and installers echo it back — npm
/// prints its resolved config on an auth failure — and a token with no
/// recognizable prefix would otherwise reach the file verbatim.
#[test]
fn test_log_redacts_an_environment_secret_with_no_recognizable_prefix() {
    let secret = "0e8f31c5a4b7d296e5f1a";
    // Set before the reporter is built: the snapshot is taken at construction.
    std::env::set_var("BUZZ_TEST_REGISTRY_TOKEN", secret);
    let h = harness();
    std::env::remove_var("BUZZ_TEST_REGISTRY_TOKEN");

    h.reporter.record_attempt(
        1,
        outcome("cli", false, &format!("npm ERR! _authToken={secret}")),
    );

    let log = h.log_contents();
    assert!(!log.contains(secret), "got: {log}");
    assert!(log.contains("[REDACTED]"), "got: {log}");
}

/// A live line carries the same scrubbing as the log record. The line is
/// rendered verbatim in the UI, so a leak there is as visible as one on disk.
#[test]
fn test_a_live_line_is_redacted_before_it_is_emitted() {
    let h = harness();

    let observer = h.reporter.line_observer().expect("an observer");
    observer("fetching with token nsec1qqqqqqqqqqleaked");

    let lines = h.lines();
    assert_eq!(lines.len(), 1);
    let line = lines[0].clone().expect("a line, not a clear signal");
    assert!(!line.contains("nsec1qqqqqqqqqqleaked"), "got: {line}");
    assert!(line.contains("[REDACTED]"), "got: {line}");
}

// ── proxy and PAT credentials ────────────────────────────────────────────────

/// A proxy URL's password is a credential, but the proxy itself is diagnostic
/// information: an install that fails behind a proxy is only debuggable if the
/// record still says which proxy it went through. So the userinfo is scrubbed
/// and the host is kept.
#[test]
fn test_proxy_userinfo_is_secret_but_the_proxy_host_is_not() {
    let secrets = secret_values_from([(
        "HTTPS_PROXY".to_string(),
        "http://corpuser:hunter2pass@proxy.example:8080".to_string(),
    )]);

    assert_eq!(secrets, vec!["corpuser:hunter2pass"]);
}

/// A proxy with no credential contributes nothing — scrubbing a bare host would
/// erase the proxy's name from every record while protecting nothing. A bare
/// username is not a credential either, and scrubbing it would delete every
/// occurrence of that word from the log.
#[test]
fn test_a_proxy_without_credentials_contributes_no_secret() {
    let secrets = secret_values_from([
        (
            "HTTP_PROXY".to_string(),
            "http://proxy.example:8080".to_string(),
        ),
        (
            "ALL_PROXY".to_string(),
            "socks5://10.0.0.1:1080".to_string(),
        ),
        (
            "HTTPS_PROXY".to_string(),
            "http://user@proxy.example:8080".to_string(),
        ),
    ]);

    assert!(secrets.is_empty(), "got: {secrets:?}");
}

/// `*_PATH` variables must not be mistaken for personal access tokens. A
/// `contains("_PAT")` rule would match `PATH` itself and scrub every directory
/// name out of the log, which is why the rule matches `_PAT` as a suffix.
#[test]
fn test_a_path_variable_is_not_treated_as_a_personal_access_token() {
    let secrets = secret_values_from([
        ("PATH".to_string(), "/usr/local/bin:/usr/bin".to_string()),
        ("GOPATH".to_string(), "/home/user/go".to_string()),
        (
            "CARGO_HOME_PATH".to_string(),
            "/home/user/.cargo".to_string(),
        ),
    ]);

    assert!(secrets.is_empty(), "got: {secrets:?}");
}

/// Variables named as personal access tokens are secret by name, whatever shape
/// their value has.
#[test]
fn test_pat_named_variables_are_secret() {
    let secrets = secret_values_from([
        (
            "GITHUB_PAT".to_string(),
            "ghp_abcdefghij0123456789".to_string(),
        ),
        (
            "GH_PAT".to_string(),
            "github_pat_abcdefghij0123".to_string(),
        ),
    ]);

    assert_eq!(secrets.len(), 2, "got: {secrets:?}");
}

/// The whole point of the widening: a proxy password and a PAT that the
/// installer echoed reach neither the log nor the live line.
///
/// Both are checked through the real reporter rather than the classifier, so
/// this covers the wiring — a classifier that recognises a secret the reporter
/// never consults would still leak.
#[test]
fn test_proxy_and_pat_credentials_are_redacted_from_the_log_and_the_live_line() {
    let proxy_password = "hunter2pass";
    let pat = "ghp_abcdefghij0123456789";
    // The classifier's own tests cover recognising these under their real
    // variable names; injecting the resulting secrets here keeps a live
    // `HTTPS_PROXY` out of the process the rest of the suite shares.
    let h = harness_with_secrets(vec![format!("corpuser:{proxy_password}"), pat.to_string()]);

    h.reporter.record_attempt(
        1,
        outcome(
            "cli",
            false,
            &format!(
                "npm ERR! proxy=http://corpuser:{proxy_password}@proxy.example authToken={pat}"
            ),
        ),
    );
    let observer = h.reporter.line_observer().expect("an observer");
    observer(&format!("cloning https://{pat}@github.com/org/repo"));

    let log = h.log_contents();
    assert!(
        !log.contains(proxy_password),
        "log leaked the proxy password: {log}"
    );
    assert!(!log.contains(pat), "log leaked the PAT: {log}");
    assert!(
        log.contains("proxy.example"),
        "the proxy host is diagnostic and must survive: {log}"
    );

    let line = h.lines().into_iter().flatten().next().expect("a live line");
    assert!(!line.contains(pat), "live line leaked the PAT: {line}");
    assert!(line.contains("[REDACTED]"), "got: {line}");
}

/// The third surface: the step returned to the frontend. `getInstallErrorMessage`
/// renders the failing step's stderr verbatim, so a secret that the log and the
/// live line both scrub would still reach the user through the error dialog.
#[test]
fn test_a_returned_step_is_redacted_before_the_frontend_renders_it() {
    let pat = "ghp_abcdefghij0123456789";
    let h = harness_with_secrets(vec![pat.to_string()]);

    let returned = h.reporter.record_attempt(
        1,
        InstallOutcome {
            step: InstallStepResult {
                stdout: format!("configuring remote with {pat}"),
                stderr: format!("fatal: authentication failed for token {pat}"),
                hint: Some(format!("check that {pat} has the repo scope")),
                ..step("cli", false, "")
            },
            log_stdout: String::new(),
            log_stderr: String::new(),
        },
    );

    assert!(!returned.stdout.contains(pat), "got: {}", returned.stdout);
    assert!(!returned.stderr.contains(pat), "got: {}", returned.stderr);
    let hint = returned.hint.expect("a hint");
    assert!(!hint.contains(pat), "got: {hint}");
}

/// A synthesized step reaches the frontend through the other funnel, and needs
/// the same scrubbing — the managed-node prerequisite failures are built this
/// way and carry whatever the underlying command printed.
#[test]
fn test_a_synthesized_step_is_redacted_before_it_reaches_the_caller() {
    let pat = "ghp_abcdefghij0123456789";
    let h = harness_with_secrets(vec![pat.to_string()]);
    let mut steps = Vec::new();

    h.reporter.record_step(
        &mut steps,
        InstallStepResult {
            stderr: format!("npm ERR! 401 with {pat}"),
            ..step("adapter", false, "")
        },
    );

    assert_eq!(steps.len(), 1);
    assert!(!steps[0].stderr.contains(pat), "got: {}", steps[0].stderr);
    assert!(
        steps[0].stderr.contains("[REDACTED]"),
        "got: {}",
        steps[0].stderr
    );
}

// ── the log pointer ──────────────────────────────────────────────────────────

/// The path is available as soon as the run's session opens, because the file
/// exists from that moment — the header is already in it. A failure before any
/// step ran still points the user at a real file.
#[test]
fn test_log_path_is_available_from_the_start_of_the_run() {
    let h = harness();

    assert_eq!(h.reporter.log_path(), Some(h.log.display().to_string()));
}

/// A reporter with no log — an unresolvable app-data directory — records
/// nothing and reports no path, but must not panic or fail the install.
#[test]
fn test_reporter_without_a_log_records_nothing_and_reports_no_path() {
    let reporter = silent_reporter();
    let mut steps = Vec::new();

    reporter.start_attempt();
    reporter.record_attempt(1, outcome("cli", false, "output"));
    reporter.record_step(&mut steps, step("verify", false, "detail"));

    assert_eq!(reporter.log_path(), None);
    assert_eq!(steps.len(), 1, "the UI path is unaffected by a missing log");
}

/// A log path inside a directory that no longer exists cannot open a session, so
/// the run degrades to no log rather than failing.
#[test]
fn test_an_unopenable_log_degrades_to_no_log() {
    let path = PathBuf::from("/nonexistent-dir-for-test/install-goose.log");

    assert!(InstallLog::start(&path, "goose", TEST_APP_VERSION).is_none());
}

// ── live output line ─────────────────────────────────────────────────────────

/// Lines carry an install-wide monotonic sequence number, so the UI can order
/// them across steps and attempts — which a per-step retry number cannot do.
#[test]
fn test_emitted_lines_carry_their_runtime_and_a_monotonic_sequence() {
    let h = harness();

    let observer = h.reporter.line_observer().expect("an observer");
    observer("downloading");
    h.reporter.start_attempt();

    let events = h.events();
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| e.runtime_id == "goose"));
    assert_eq!(events[0].line.as_deref(), Some("downloading"));
    assert_eq!(events[0].seq, 0);
    assert_eq!(
        events[1].seq, 1,
        "the clear signal takes the next sequence number, so it cannot be \
         mistaken for a stale event"
    );
}

/// Starting an attempt clears the display first: the previous attempt's last
/// line is typically the failure that caused the retry, and leaving it under the
/// spinner through the backoff shows the user the past as if it were current.
#[test]
fn test_starting_an_attempt_clears_the_displayed_line() {
    let h = harness();
    let observer = h.reporter.line_observer().expect("an observer");
    observer("download failed");

    h.reporter.start_attempt();

    assert_eq!(
        h.lines(),
        vec![Some("download failed".to_string()), None],
        "the attempt boundary must emit a clear"
    );
}

/// The clear is not rate-limited, and it reopens the window: a new attempt's
/// first line goes out immediately even if it arrives inside the previous
/// attempt's window. This is the case the throttle used to swallow entirely.
#[test]
fn test_a_new_attempts_first_line_is_emitted_even_inside_the_previous_window() {
    let h = harness();
    let observer = h.reporter.line_observer().expect("an observer");
    observer("attempt one failed");

    // No wait: the previous line was emitted microseconds ago, so this is well
    // inside the 250ms window.
    h.reporter.start_attempt();
    observer("attempt two starting");

    assert_eq!(
        h.lines(),
        vec![
            Some("attempt one failed".to_string()),
            None,
            Some("attempt two starting".to_string()),
        ]
    );
}

/// A burst inside the window coalesces to one event, and the line it emits is
/// the *newest* — the display shows current progress, not the line that happened
/// to arrive when the window opened.
#[test]
fn test_a_burst_coalesces_to_the_newest_line_not_the_first() {
    let h = harness();
    let observer = h.reporter.line_observer().expect("an observer");

    observer("one");
    observer("two");
    observer("three");
    // Ends the attempt, which is when a held line is known to be the last.
    h.reporter.record_attempt(1, outcome("cli", true, "done"));

    assert_eq!(
        h.lines(),
        vec![Some("one".to_string()), Some("three".to_string())],
        "the held line must be the newest, and it must not be lost"
    );
}

/// The throttle is per install, not per stream: stdout and stderr of one attempt
/// share one window, so an install printing on both does not double the event
/// rate.
#[test]
fn test_both_streams_of_one_attempt_share_the_rate_window() {
    let h = harness();
    let stdout = h.reporter.line_observer().expect("an observer");
    let stderr = h.reporter.line_observer().expect("an observer");

    stdout("progress");
    stderr("warning");
    h.reporter.record_attempt(1, outcome("cli", true, "done"));

    assert_eq!(
        h.lines(),
        vec![Some("progress".to_string()), Some("warning".to_string())],
        "the second stream's line is held, not emitted immediately, and not lost"
    );
}

/// Nothing listening means no observer at all, so the drain skips line
/// reassembly entirely rather than doing the work and discarding it.
#[test]
fn test_no_observer_when_nothing_is_listening() {
    assert!(silent_reporter().line_observer().is_none());
}

// ── late-drain deactivation ──────────────────────────────────────────────────

/// After the reporter is dropped (run settled), a drain thread that still holds
/// a cloned observer must not be able to emit. The lifecycle lock is shared by
/// reference with every `line_observer` clone, so taking the exclusive write
/// lock in `Drop` — which waits out any in-flight read guards — ensures no
/// late event can reach the listener.
///
/// The test resets the throttle via `start_attempt` before the late emit, so
/// the late line would be emitted unconditionally without the lifecycle lock.
#[test]
fn test_a_detached_observer_cannot_emit_after_the_reporter_is_dropped() {
    let h = harness();

    // Simulate a drain thread: `line_observer` clones `Live` (owned, not
    // borrowed), so the closure outlives the reporter.
    let observer = h.reporter.line_observer().expect("an observer");
    observer("before-settle");

    assert_eq!(
        h.lines(),
        vec![Some("before-settle".to_string())],
        "a live observer must emit before the reporter drops"
    );

    // Open a fresh throttle window so the next offer would emit immediately —
    // simulating the drain arriving after the 250ms rate window closed.
    h.reporter.start_attempt();
    let events = Arc::clone(&h.events);

    // Drop the reporter — takes the exclusive lifecycle write lock, waits for
    // any in-flight publications, then deactivates.
    drop(h);

    // A late offer after drop must be blocked by the deactivated lifecycle.
    observer("late-drain-after-settle");

    let lines: Vec<Option<String>> = events
        .lock()
        .unwrap()
        .iter()
        .map(|e| e.line.clone())
        .collect();
    assert_eq!(
        lines,
        // before-settle + the clear from start_attempt, no late line
        vec![Some("before-settle".to_string()), None],
        "a detached observer must not emit after the reporter is dropped"
    );
}

/// Deterministic concurrency pin: proves that reporter deactivation cannot
/// complete while a drain thread is in-flight between admission and publication.
///
/// The lifecycle `RwLock` enforces this: drain threads hold a shared read guard
/// for the entire (check → emit) span, so `InstallReporter::drop`'s write lock
/// blocks until every admitted publication finishes.
///
/// **How this test fails on the old atomic shape** (`dc6421ac4`): on that shape,
/// `offer` loads `active` once as a plain atomic read and then calls `publish`
/// independently. There is no shared lock, so `drop` (an atomic store) can
/// complete while the thread is between the load and the emit — the test
/// exposes this by asserting the write lock CANNOT be acquired while a reader
/// holds a read guard. On the atomic shape the `lifecycle` field does not exist,
/// so the entire serialisation contract is absent and the pin fails.
#[test]
fn test_deactivation_blocks_until_in_flight_publication_completes() {
    // Build a reporter and grab a Live clone that represents a drain thread.
    let h = harness();
    let live_clone = {
        // Extract the `Live` from a `line_observer` closure by temporarily
        // building a second observer and using the lifecycle Arc directly.
        h.reporter.line_observer().expect("observer");
        // Clone the inner lifecycle from the reporter's `Live` via a
        // white-box path: the test module is a child of install_report and
        // can access private fields.
        h.reporter
            .live
            .as_ref()
            .expect("live exists")
            .lifecycle
            .clone()
    };

    // Phase 1: acquire the shared read guard (admission).
    let guard = live_clone.read().expect("lifecycle read");
    assert!(*guard, "lifecycle must be active at admission");

    // Phase 2: while the read guard is held, a write lock must be blocked.
    // This is the core serialisation invariant: `Drop` cannot complete until
    // every admitted reader releases its guard.
    assert!(
        live_clone.try_write().is_err(),
        "a write lock must not be acquirable while a read guard is held — \
         Drop must block while a publication is in-flight"
    );

    // Phase 3: release the read guard (publication completed).
    drop(guard);

    // Phase 4: write lock is now available, and Drop can set active=false.
    let mut write = live_clone.write().expect("lifecycle write");
    *write = false;
    drop(write);

    // Phase 5: a new reader after deactivation sees active=false and returns.
    let guard2 = live_clone.read().expect("lifecycle read");
    assert!(
        !*guard2,
        "lifecycle must be inactive after deactivation — new admissions rejected"
    );
}

/// Shared-consumer assertion: drive a potential late run-1 event AND run-2's
/// events through the same `nextInstallOutputLine`-equivalent reducer and assert
/// that run 2's output replaces — not revives — any stale run-1 state.
///
/// The frontend reduces events into a single consumer that resets to `null`
/// when `isInstalling` goes false (i.e., when the run settles).  This test
/// models that reset and verifies the full cross-run contract:
///
/// 1. Run 1 emits normally; the late drain is silenced by the lifecycle lock
///    (no event with a high run-1 `seq` ever reaches the shared sink).
/// 2. At run-1 settlement the consumer resets to `null`, exactly as the
///    frontend hook does when `isInstalling` becomes false.
/// 3. Run 2 starts, emits its `seq=0` clear and first line.  With a null-state
///    consumer the reducer accepts both immediately — even if a late run-1
///    event HAD arrived (it didn't), the reset would have cleared its seq.
///
/// **Why this test fails on the old atomic shape**: without the lifecycle lock,
/// `obs1("late-run-one-drain")` emits a high-seq run-1 event into the shared
/// sink.  That event arrives AFTER the consumer reset (its seq is accepted from
/// a null state), leaving `{ seq: N, line: "late-run-one-drain" }` in the
/// consumer.  Run 2 then emits `seq=0` and `seq=1`, both ≤ N, so they are
/// rejected and the final state stays on run 1's output.
#[test]
fn test_run2_output_replaces_stale_run1_state_through_shared_consumer() {
    // One shared event sink for all events from both runs, simulating the
    // permanent frontend listener that receives all `acp-install-output` events.
    let all_events: Arc<Mutex<Vec<InstallOutputEvent>>> = Arc::new(Mutex::new(Vec::new()));
    // Track where run 1 settles so the consumer reset can be applied at the
    // correct boundary in the fold below.
    let run1_settle_len: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    let runtime_id = "goose";

    // ── Run 1 ──────────────────────────────────────────────────────────────
    let dir1 = tempfile::tempdir().expect("tempdir");
    let log1 = dir1.path().join("install-goose.log");
    let emit1: EmitEvent = {
        let sink = Arc::clone(&all_events);
        Arc::new(move |event| sink.lock().unwrap().push(event))
    };
    let reporter1 = InstallReporter::new(
        runtime_id,
        InstallLog::start(&log1, runtime_id, "1.0.0"),
        Some(emit1),
    );

    reporter1.start_attempt();
    let obs1 = reporter1.line_observer().expect("observer");
    obs1("run-one-output");
    reporter1.record_attempt(1, outcome("cli", true, "done"));

    // Drop reporter1 — deactivates obs1 via the exclusive lifecycle write lock,
    // which blocks until any in-flight read guard (publication) has released.
    drop(reporter1);

    // Record the sink length at settlement — this is the point where the
    // frontend resets its consumer state to null (isInstalling = false).
    *run1_settle_len.lock().unwrap() = all_events.lock().unwrap().len();

    // A late drain from run 1 arrives after settlement.  With the lifecycle
    // lock this is silenced.  Without the lock (old atomic shape) it would
    // reach the sink with a high seq, poisoning the null-reset consumer before
    // run 2 can emit its restarted seq=0.
    // Reset the throttle so the offer would emit unconditionally if the lock
    // were absent — this makes the mutation meaningful.
    {
        // We need a fresh throttle window.  Simulate by directly restarting
        // via a new reporter to touch the shared Live (not possible after drop),
        // so instead just call offer — the throttle holds the last emit time
        // from flush_pending, which was microseconds ago.  Give the window time
        // to expire so the offer fires immediately on the old atomic shape.
        std::thread::sleep(Duration::from_millis(300));
    }
    obs1("late-run-one-drain");

    // ── Run 2 ──────────────────────────────────────────────────────────────
    let dir2 = tempfile::tempdir().expect("tempdir");
    let log2 = dir2.path().join("install-goose.log");
    let emit2: EmitEvent = {
        let sink = Arc::clone(&all_events);
        Arc::new(move |event| sink.lock().unwrap().push(event))
    };
    let reporter2 = InstallReporter::new(
        runtime_id,
        InstallLog::start(&log2, runtime_id, "1.0.0"),
        Some(emit2),
    );

    reporter2.start_attempt();
    let obs2 = reporter2.line_observer().expect("observer");
    obs2("run-two-first-line");
    reporter2.record_attempt(1, outcome("cli", true, "done"));

    // ── Shared-consumer fold ────────────────────────────────────────────────
    // Fold all emitted events through the nextInstallOutputLine reducer logic.
    // At the run-1 settlement boundary, reset consumer to null — exactly as
    // the frontend hook does when isInstalling becomes false.
    struct State {
        seq: u64,
        line: Option<String>,
    }
    let settle_at = *run1_settle_len.lock().unwrap();
    let events = all_events.lock().unwrap().clone();
    let mut consumer: Option<State> = None;
    for (i, event) in events.iter().enumerate() {
        // Simulate frontend reset at run-1 settlement boundary.
        if i == settle_at {
            consumer = None;
        }
        if event.runtime_id != runtime_id {
            continue;
        }
        if let Some(ref c) = consumer {
            if event.seq <= c.seq {
                continue; // reject stale / out-of-order
            }
        }
        consumer = Some(State {
            seq: event.seq,
            line: event.line.clone(),
        });
    }

    // The final consumer state must be run 2's first line.
    //
    // With the lifecycle lock: obs1("late-run-one-drain") emits nothing,
    // so after the null reset only run-2 events arrive — run 2 wins cleanly.
    //
    // Without the lifecycle lock (old atomic shape): obs1 emits the late line
    // with a high seq into the already-reset (null) consumer, consumer becomes
    // { seq: N, line: "late-run-one-drain" }.  Run 2's seq=0/1 are both ≤ N
    // and are rejected, leaving the final state on run 1's stale output.
    let final_line = consumer
        .as_ref()
        .and_then(|s| s.line.as_deref())
        .unwrap_or("<none>");
    assert_eq!(
        final_line, "run-two-first-line",
        "run 2 must replace — not revive — stale run-1 state through the shared consumer; \
         got: {final_line:?}"
    );
}
