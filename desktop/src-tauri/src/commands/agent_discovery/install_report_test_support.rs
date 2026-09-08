//! Shared harness for the install-report test modules.
//!
//! The tests are split by concern — redaction in
//! [`super::install_report_redaction_tests`], everything else in
//! [`super::install_report_tests`] — and both drive the reporter through the
//! same harness, so it lives here rather than in either of them.

use super::*;
use std::sync::Mutex;

/// Stands in for the real `app.package_info().version`, which needs a Tauri app.
pub(crate) const TEST_APP_VERSION: &str = "9.9.9";

/// A reporter with a started log session in a temp dir, and the emitted events
/// captured.
pub(crate) struct Harness {
    /// Kept alive so the log outlives the harness; a test that reuses the
    /// directory for a second run takes it.
    pub(crate) _dir: tempfile::TempDir,
    pub(crate) log: PathBuf,
    pub(crate) reporter: InstallReporter,
    pub(crate) events: Arc<Mutex<Vec<InstallOutputEvent>>>,
}

pub(crate) fn harness() -> Harness {
    harness_at(None)
}

/// A harness whose log lives in `dir`, or in a fresh temp dir when `dir` is
/// `None`. Passing a directory lets a test seed a previous run's file first.
pub(crate) fn harness_at(dir: Option<tempfile::TempDir>) -> Harness {
    harness_inner(dir, None)
}

/// A harness whose reporter scrubs exactly `secrets`, so a proxy or PAT
/// credential can be asserted without exporting it into the process.
pub(crate) fn harness_with_secrets(secrets: Vec<String>) -> Harness {
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
pub(crate) fn silent_reporter() -> InstallReporter {
    InstallReporter::new("goose", None, None)
}

pub(crate) fn step(name: &str, success: bool, stderr: &str) -> InstallStepResult {
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
pub(crate) fn outcome(name: &str, success: bool, log_stdout: &str) -> InstallOutcome {
    InstallOutcome {
        step: step(name, success, ""),
        log_stdout: log_stdout.to_string(),
        log_stderr: String::new(),
    }
}

impl Harness {
    pub(crate) fn log_contents(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// The emitted lines in order, with a clear signal rendered as `None`.
    pub(crate) fn lines(&self) -> Vec<Option<String>> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.line.clone())
            .collect()
    }

    pub(crate) fn events(&self) -> Vec<InstallOutputEvent> {
        self.events.lock().unwrap().clone()
    }
}
