use std::{
    collections::HashMap,
    ffi::OsString,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use crate::managed_agents::runtime::build_augmented_path;

/// Build the augmented PATH for CLI probes and other native child processes
/// (auth commands, `buzz-acp models` discovery), including nvm's default
/// Node.js bin directory so `#!/usr/bin/env node` shims (e.g. codex-acp)
/// resolve.
pub(crate) fn augmented_path() -> Option<String> {
    let home = dirs::home_dir();
    let nvm_bin = home
        .as_deref()
        .and_then(crate::managed_agents::find_nvm_default_bin);
    build_augmented_path(
        home,
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf)),
        crate::managed_agents::login_shell_path(),
        nvm_bin,
    )
}

/// Outcome of a CLI login-status probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProbeOutcome {
    /// The CLI reported a successful login (exit 0).
    LoggedIn,
    /// The CLI exited non-zero without a config-parse signal — treat as
    /// "not authenticated."
    LoggedOut,
    /// The CLI exited non-zero and its stderr contains a config-parse error
    /// (e.g. from `~/.codex/config.toml`). The user needs to fix their
    /// config, not re-run login.
    ConfigInvalid {
        /// A trimmed excerpt of the stderr message to surface in the nudge.
        stderr_excerpt: String,
    },
    /// The CLI did not finish within the hard process deadline. The child was
    /// killed and reaped before this result was returned.
    TimedOut,
}

const LOGIN_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const LOGIN_PROBE_CACHE_TTL: Duration = Duration::from_secs(45);
const CONFIG_ERROR_CACHE_TTL: Duration = Duration::from_secs(5);
const MAX_PROBE_OUTPUT_BYTES: usize = 64 * 1024;

trait ProbeTreeGuard {
    fn terminate_and_wait(&mut self) -> Result<(), String>;
}

#[cfg(unix)]
struct NativeProbeTreeGuard {
    process_group: i32,
}

#[cfg(unix)]
impl ProbeTreeGuard for NativeProbeTreeGuard {
    fn terminate_and_wait(&mut self) -> Result<(), String> {
        use nix::{
            errno::Errno,
            sys::signal::{killpg, Signal},
            unistd::Pid,
        };

        match killpg(Pid::from_raw(self.process_group), Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => Ok(()),
            Err(error) => Err(format!(
                "failed to terminate readiness probe process group {}: {error}",
                self.process_group
            )),
        }
    }
}

#[cfg(windows)]
struct NativeProbeTreeGuard {
    job: crate::managed_agents::JobHandle,
}

#[cfg(windows)]
impl ProbeTreeGuard for NativeProbeTreeGuard {
    fn terminate_and_wait(&mut self) -> Result<(), String> {
        self.job.terminate_and_wait(Duration::from_secs(1))
    }
}

#[cfg(not(any(unix, windows)))]
struct NativeProbeTreeGuard;

#[cfg(not(any(unix, windows)))]
impl ProbeTreeGuard for NativeProbeTreeGuard {
    fn terminate_and_wait(&mut self) -> Result<(), String> {
        Ok(())
    }
}

fn spawn_contained_probe(
    command: &mut std::process::Command,
) -> Result<(std::process::Child, NativeProbeTreeGuard), String> {
    #[cfg(windows)]
    {
        let (child, job) = crate::managed_agents::spawn_probe_in_job(command)?;
        return Ok((child, NativeProbeTreeGuard { job }));
    }
    #[cfg(not(windows))]
    {
        let child = command
            .spawn()
            .map_err(|error| format!("failed to spawn readiness probe: {error}"))?;
        #[cfg(unix)]
        let guard = NativeProbeTreeGuard {
            process_group: child.id() as i32,
        };
        #[cfg(not(any(unix, windows)))]
        let guard = NativeProbeTreeGuard;
        Ok((child, guard))
    }
}

fn terminate_contained_probe(
    child: &mut std::process::Child,
    guard: &mut impl ProbeTreeGuard,
) -> Result<(), String> {
    let tree_result = guard.terminate_and_wait();
    let _ = child.kill();
    tree_result
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProbeCacheKey {
    generation: u64,
    runtime: String,
    binary_path: PathBuf,
    args: Vec<String>,
    effective_path: Option<String>,
    effective_environment: Vec<(&'static str, Option<OsString>)>,
}

impl ProbeCacheKey {
    fn same_probe_identity(&self, other: &Self) -> bool {
        self.runtime == other.runtime
            && self.binary_path == other.binary_path
            && self.args == other.args
            && self.effective_path == other.effective_path
            && self.effective_environment == other.effective_environment
    }
}

#[derive(Debug)]
struct ProbeFlight {
    result: Mutex<Option<ProbeOutcome>>,
    ready: Condvar,
}

impl ProbeFlight {
    fn wait(&self) -> ProbeOutcome {
        let mut result = self
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while result.is_none() {
            result = self
                .ready
                .wait(result)
                .unwrap_or_else(|error| error.into_inner());
        }
        result.clone().unwrap_or(ProbeOutcome::LoggedOut)
    }

    fn publish(&self, outcome: ProbeOutcome) {
        let mut result = self
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *result = Some(outcome);
        self.ready.notify_all();
    }
}

#[derive(Debug)]
enum ProbeCacheEntry {
    InFlight(Arc<ProbeFlight>),
    Complete {
        completed_at: Instant,
        outcome: ProbeOutcome,
    },
}

#[derive(Default)]
struct LoginProbeCache {
    generation: AtomicU64,
    entries: Mutex<HashMap<ProbeCacheKey, ProbeCacheEntry>>,
}

enum ProbeDecision {
    Cached(ProbeOutcome),
    Wait(Arc<ProbeFlight>),
    Run(Arc<ProbeFlight>),
}

impl LoginProbeCache {
    fn probe<F, N>(&self, mut key: ProbeCacheKey, now: N, run: F) -> ProbeOutcome
    where
        F: FnOnce() -> ProbeOutcome,
        N: Fn() -> Instant,
    {
        key.generation = self.generation.load(Ordering::Acquire);
        let decision = {
            let observed_at = now();
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            // Invalidation expires completed values immediately, but an
            // already-running identical command remains the authoritative
            // flight until it settles. Clearing it here would allow the same
            // probe key to run twice during a configuration refresh.
            entries.retain(|candidate, entry| {
                candidate.generation == key.generation
                    || matches!(entry, ProbeCacheEntry::InFlight(_))
            });
            let cached = entries.get(&key).and_then(|entry| match entry {
                ProbeCacheEntry::Complete {
                    completed_at,
                    outcome,
                } => {
                    let ttl = if matches!(outcome, ProbeOutcome::ConfigInvalid { .. }) {
                        CONFIG_ERROR_CACHE_TTL
                    } else {
                        LOGIN_PROBE_CACHE_TTL
                    };
                    (observed_at.saturating_duration_since(*completed_at) < ttl)
                        .then(|| outcome.clone())
                }
                ProbeCacheEntry::InFlight(_) => None,
            });
            if let Some(outcome) = cached {
                ProbeDecision::Cached(outcome)
            } else if let Some(flight) = entries.iter().find_map(|(candidate, entry)| {
                if candidate.same_probe_identity(&key) {
                    if let ProbeCacheEntry::InFlight(flight) = entry {
                        return Some(Arc::clone(flight));
                    }
                }
                None
            }) {
                ProbeDecision::Wait(flight)
            } else {
                entries.remove(&key);
                let flight = Arc::new(ProbeFlight {
                    result: Mutex::new(None),
                    ready: Condvar::new(),
                });
                entries.insert(key.clone(), ProbeCacheEntry::InFlight(Arc::clone(&flight)));
                ProbeDecision::Run(flight)
            }
        };

        match decision {
            ProbeDecision::Cached(outcome) => outcome,
            ProbeDecision::Wait(flight) => flight.wait(),
            ProbeDecision::Run(flight) => {
                // The external command runs without the cache mutex. Identical
                // callers wait on this key's flight; different keys remain
                // independent and may probe concurrently.
                let outcome = run();
                flight.publish(outcome.clone());
                let completed_at = now();
                let mut entries = self
                    .entries
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if self.generation.load(Ordering::Acquire) == key.generation
                    && matches!(
                        entries.get(&key),
                        Some(ProbeCacheEntry::InFlight(current)) if Arc::ptr_eq(current, &flight)
                    )
                {
                    entries.insert(
                        key,
                        ProbeCacheEntry::Complete {
                            completed_at,
                            outcome: outcome.clone(),
                        },
                    );
                } else if matches!(
                    entries.get(&key),
                    Some(ProbeCacheEntry::InFlight(current)) if Arc::ptr_eq(current, &flight)
                ) {
                    // The result belongs to the pre-invalidation generation.
                    // Wake its waiters but do not leave a stale flight/value
                    // behind; the next call recomputes against current config.
                    entries.remove(&key);
                }
                outcome
            }
        }
    }

    fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .retain(|_, entry| matches!(entry, ProbeCacheEntry::InFlight(_)));
    }
}

fn login_probe_cache() -> &'static LoginProbeCache {
    static CACHE: OnceLock<LoginProbeCache> = OnceLock::new();
    CACHE.get_or_init(LoginProbeCache::default)
}

pub(crate) fn clear_login_probe_cache() {
    // The cache mutex is never held while an external process runs, so
    // configuration writes and manual discovery refreshes invalidate promptly.
    login_probe_cache().invalidate();
}

/// Signals emitted to stderr by codex (and related CLI tools) when they
/// fail to parse their config file. We check these to distinguish a
/// config-parse failure from a genuine "not authenticated" exit.
///
/// The real codex error reads:
///   `Error loading configuration: .../.codex/config.toml:... unknown variant ...`
/// So we require BOTH "error loading configuration" AND "unknown variant" to be
/// present, avoiding false positives from unrelated errors that mention only
/// one term.
const CONFIG_PARSE_SIGNALS: &[&str] = &["error loading configuration", "unknown variant"];

/// Run the probe at the resolved absolute path so the GUI-PATH gap is
/// bypassed. Injects the same augmented PATH used for launched agents so
/// script shims with `/usr/bin/env <interpreter>` shebangs can find runtimes
/// such as node/python when the app was launched with a bare GUI PATH.
pub(crate) fn login_probe(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
) -> ProbeOutcome {
    login_probe_with_timeout(binary_path, probe_args, augmented_path, LOGIN_PROBE_TIMEOUT)
}

fn login_probe_with_timeout(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
    timeout: Duration,
) -> ProbeOutcome {
    let key = ProbeCacheKey {
        generation: 0,
        runtime: probe_args.first().copied().unwrap_or_default().to_string(),
        binary_path: binary_path
            .canonicalize()
            .unwrap_or_else(|_| binary_path.to_path_buf()),
        args: probe_args
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        effective_path: augmented_path.map(str::to_owned),
        effective_environment: ["HOME", "XDG_CONFIG_HOME", "CODEX_HOME", "CLAUDE_CONFIG_DIR"]
            .into_iter()
            .map(|name| (name, std::env::var_os(name)))
            .collect(),
    };
    login_probe_cache().probe(key, Instant::now, || {
        run_login_probe(binary_path, probe_args, augmented_path, timeout)
    })
}

fn run_login_probe(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
    timeout: Duration,
) -> ProbeOutcome {
    // Regular files cannot be held at a blocking EOF by forked descendants.
    // This keeps output collection inside the same deadline as the child even
    // when a CLI exits after spawning a process that inherited stdout/stderr.
    let Ok(mut stdout) = tempfile::tempfile() else {
        return ProbeOutcome::LoggedOut;
    };
    let Ok(mut stderr) = tempfile::tempfile() else {
        return ProbeOutcome::LoggedOut;
    };
    let (Ok(stdout_writer), Ok(stderr_writer)) = (stdout.try_clone(), stderr.try_clone()) else {
        return ProbeOutcome::LoggedOut;
    };
    let mut command = std::process::Command::new(binary_path);
    command.args(&probe_args[1..]);
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    crate::util::configure_no_window(&mut command);
    command.stdout(stdout_writer).stderr(stderr_writer);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let (mut child, mut tree_guard) = match spawn_contained_probe(&mut command) {
        Ok(contained) => contained,
        Err(error) => {
            eprintln!("buzz-desktop: readiness probe containment failed: {error}");
            return ProbeOutcome::LoggedOut;
        }
    };
    let started_at = Instant::now();
    let (status, timed_out, mut containment_ok) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), false, true),
            Ok(None) if started_at.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let containment_ok =
                    cleanup_contained_probe(&mut child, &mut tree_guard, "after timeout");
                break (child.wait().ok(), true, containment_ok);
            }
            Err(_) => {
                let containment_ok =
                    cleanup_contained_probe(&mut child, &mut tree_guard, "after wait failure");
                let _ = child.wait();
                break (None, false, containment_ok);
            }
        }
    };
    // The direct child may have exited after forking a helper that inherited
    // its output descriptors. Terminate that probe-only process group before
    // returning so no authentication helper survives a completed probe.
    containment_ok &= cleanup_contained_probe(&mut child, &mut tree_guard, "after completion");
    let _stdout = read_bounded_output(&mut stdout);
    let stderr = read_bounded_output(&mut stderr);
    if !containment_ok {
        ProbeOutcome::LoggedOut
    } else if timed_out {
        ProbeOutcome::TimedOut
    } else if let Some(status) = status {
        classify_probe_output(&stderr, status.success())
    } else {
        ProbeOutcome::LoggedOut
    }
}

fn cleanup_contained_probe(
    child: &mut std::process::Child,
    guard: &mut impl ProbeTreeGuard,
    context: &str,
) -> bool {
    match terminate_contained_probe(child, guard) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("buzz-desktop: readiness probe cleanup failed {context}: {error}");
            false
        }
    }
}

fn read_bounded_output(file: &mut std::fs::File) -> Vec<u8> {
    if file.seek(SeekFrom::Start(0)).is_err() {
        return Vec::new();
    }
    let mut retained = Vec::new();
    let _ = file
        .take(MAX_PROBE_OUTPUT_BYTES as u64)
        .read_to_end(&mut retained);
    retained
}

/// Classify collected probe output into a `ProbeOutcome`.
///
/// Shared by the bounded process runner and classification-focused tests.
pub(crate) fn classify_probe_output(stderr_bytes: &[u8], exit_success: bool) -> ProbeOutcome {
    if exit_success {
        return ProbeOutcome::LoggedIn;
    }
    let stderr = String::from_utf8_lossy(stderr_bytes);
    let stderr_lower = stderr.to_lowercase();
    if CONFIG_PARSE_SIGNALS
        .iter()
        .all(|sig| stderr_lower.contains(sig))
    {
        let excerpt = stderr.trim().lines().next().unwrap_or("").to_string();
        ProbeOutcome::ConfigInvalid {
            stderr_excerpt: excerpt,
        }
    } else {
        ProbeOutcome::LoggedOut
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LoginProbeCache, ProbeCacheKey, ProbeOutcome, ProbeTreeGuard, CONFIG_PARSE_SIGNALS,
        LOGIN_PROBE_CACHE_TTL,
    };

    struct FakeProbeTreeGuard<'a> {
        descendants_alive: &'a std::cell::Cell<usize>,
        terminate_calls: &'a std::cell::Cell<usize>,
    }

    impl ProbeTreeGuard for FakeProbeTreeGuard<'_> {
        fn terminate_and_wait(&mut self) -> Result<(), String> {
            self.terminate_calls.set(self.terminate_calls.get() + 1);
            self.descendants_alive.set(0);
            Ok(())
        }
    }

    #[test]
    fn readiness_timeout_tree_guard_confirms_no_probe_descendants_remain() {
        let descendants_alive = std::cell::Cell::new(3);
        let terminate_calls = std::cell::Cell::new(0);
        let mut guard = FakeProbeTreeGuard {
            descendants_alive: &descendants_alive,
            terminate_calls: &terminate_calls,
        };
        guard
            .terminate_and_wait()
            .expect("terminate fake probe job");
        assert_eq!(descendants_alive.get(), 0);
        assert_eq!(terminate_calls.get(), 1);
    }

    #[test]
    fn windows_readiness_probe_is_suspended_until_job_assignment() {
        let lifecycle_source = include_str!("../process_lifecycle.rs");
        let probe_source = include_str!("cli_probe.rs");
        let managed_node_source = include_str!("../../commands/agent_discovery/managed_node.rs");
        assert!(lifecycle_source.contains("CREATE_SUSPENDED"));
        assert!(lifecycle_source.contains("AssignProcessToJobObject"));
        assert!(lifecycle_source.contains("ResumeThread"));
        assert!(lifecycle_source.contains("QueryInformationJobObject"));
        assert!(probe_source.contains("spawn_contained_probe"));
        assert!(probe_source.contains("terminate_contained_probe"));
        assert!(managed_node_source.contains("spawn_probe_in_job"));
        assert!(managed_node_source.contains("terminate_and_wait"));
    }

    fn cache_key(runtime: &str) -> ProbeCacheKey {
        ProbeCacheKey {
            generation: 0,
            runtime: runtime.to_string(),
            binary_path: std::path::PathBuf::from(format!("/test/{runtime}")),
            args: vec![runtime.to_string(), "login".into(), "status".into()],
            effective_path: Some("/test/bin".into()),
            effective_environment: Vec::new(),
        }
    }

    #[cfg(unix)]
    fn executable_script(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let script_path = temp.path().join("probe-script");
        fs::write(&script_path, contents).expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");
        (temp, script_path)
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_timeout_kills_and_reaps_child() {
        use std::fs;
        use std::time::{Duration, Instant};

        super::clear_login_probe_cache();
        let (temp, script_path) =
            executable_script("#!/bin/sh\nprintf '%s' \"$$\" > \"$1\"\nexec sleep 30\n");
        let marker = temp.path().join("pid");
        let marker_arg = marker.to_string_lossy().into_owned();
        let started_at = Instant::now();
        let outcome = super::login_probe_with_timeout(
            &script_path,
            &["probe-script", &marker_arg],
            None,
            Duration::from_millis(100),
        );
        assert_eq!(outcome, ProbeOutcome::TimedOut);
        assert!(started_at.elapsed() < Duration::from_secs(2));
        let pid: i32 = fs::read_to_string(marker)
            .expect("pid marker")
            .parse()
            .expect("numeric pid");
        assert!(
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err(),
            "probe child survived"
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_drains_but_bounds_large_output() {
        use std::time::{Duration, Instant};

        let (_temp, script_path) =
            executable_script("#!/bin/sh\nyes x | head -c 1048576 >&2\nexit 1\n");
        let started_at = Instant::now();
        let outcome = super::run_login_probe(
            &script_path,
            &["probe-script"],
            None,
            Duration::from_secs(2),
        );
        assert_eq!(outcome, ProbeOutcome::LoggedOut);
        assert!(started_at.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_does_not_wait_for_descendant_held_output() {
        use std::fs;
        use std::time::{Duration, Instant};

        let (temp, script_path) =
            executable_script("#!/bin/sh\n(sleep 30) >&2 &\nprintf '%s' \"$!\" > \"$1\"\nexit 0\n");
        let marker = temp.path().join("descendant-pid");
        let marker_arg = marker.to_string_lossy().into_owned();
        let started_at = Instant::now();
        let outcome = super::run_login_probe(
            &script_path,
            &["probe-script", &marker_arg],
            None,
            Duration::from_secs(1),
        );
        assert_eq!(outcome, ProbeOutcome::LoggedIn);
        assert!(started_at.elapsed() < Duration::from_secs(2));

        let pid: i32 = fs::read_to_string(marker)
            .expect("descendant pid marker")
            .parse()
            .expect("numeric descendant pid");
        let deadline = Instant::now() + Duration::from_secs(1);
        while nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_err(),
            "probe descendant survived"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ten_equivalent_agents_share_one_authentication_probe() {
        use std::sync::{atomic::AtomicUsize, Arc, Barrier};

        let cache = Arc::new(LoginProbeCache::default());
        let start = Arc::new(Barrier::new(11));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for _ in 0..10 {
            let cache = Arc::clone(&cache);
            let start = Arc::clone(&start);
            let calls = Arc::clone(&calls);
            threads.push(std::thread::spawn(move || {
                start.wait();
                cache.probe(cache_key("codex"), std::time::Instant::now, || {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(75));
                    ProbeOutcome::LoggedIn
                })
            }));
        }
        start.wait();
        for thread in threads {
            assert_eq!(thread.join().expect("probe caller"), ProbeOutcome::LoggedIn);
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn different_probe_keys_run_independently() {
        use std::sync::{atomic::AtomicUsize, Arc, Barrier};

        let cache = Arc::new(LoginProbeCache::default());
        let start = Arc::new(Barrier::new(3));
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for runtime in ["codex", "claude"] {
            let cache = Arc::clone(&cache);
            let start = Arc::clone(&start);
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            threads.push(std::thread::spawn(move || {
                start.wait();
                cache.probe(cache_key(runtime), std::time::Instant::now, || {
                    let count = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    max_active.fetch_max(count, std::sync::atomic::Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                    ProbeOutcome::LoggedIn
                })
            }));
        }
        start.wait();
        for thread in threads {
            assert_eq!(thread.join().expect("probe caller"), ProbeOutcome::LoggedIn);
        }
        assert_eq!(max_active.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn probe_cache_expiration_and_explicit_invalidation_use_fake_time() {
        use std::cell::Cell;

        let cache = LoginProbeCache::default();
        let started_at = std::time::Instant::now();
        let now = Cell::new(started_at);
        let calls = Cell::new(0_u32);
        let key = cache_key("codex");
        let mut run = || {
            calls.set(calls.get() + 1);
            ProbeOutcome::LoggedIn
        };

        assert_eq!(
            cache.probe(key.clone(), || now.get(), &mut run),
            ProbeOutcome::LoggedIn
        );
        assert_eq!(
            cache.probe(key.clone(), || now.get(), &mut run),
            ProbeOutcome::LoggedIn
        );
        assert_eq!(calls.get(), 1);

        now.set(started_at + LOGIN_PROBE_CACHE_TTL + std::time::Duration::from_millis(1));
        assert_eq!(
            cache.probe(key.clone(), || now.get(), &mut run),
            ProbeOutcome::LoggedIn
        );
        assert_eq!(calls.get(), 2);

        cache.invalidate();
        assert_eq!(
            cache.probe(key, || now.get(), &mut run),
            ProbeOutcome::LoggedIn
        );
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn invalidation_does_not_duplicate_an_identical_in_flight_probe() {
        use std::sync::{atomic::AtomicUsize, mpsc, Arc};

        let cache = Arc::new(LoginProbeCache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let first = {
            let cache = Arc::clone(&cache);
            let calls = Arc::clone(&calls);
            std::thread::spawn(move || {
                cache.probe(cache_key("codex"), std::time::Instant::now, || {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    started_tx.send(()).expect("announce probe start");
                    release_rx.recv().expect("release probe");
                    ProbeOutcome::LoggedIn
                })
            })
        };
        started_rx.recv().expect("first probe started");
        cache.invalidate();

        let second = {
            let cache = Arc::clone(&cache);
            let calls = Arc::clone(&calls);
            std::thread::spawn(move || {
                cache.probe(cache_key("codex"), std::time::Instant::now, || {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    ProbeOutcome::LoggedOut
                })
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(25));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        release_tx.send(()).expect("release first probe");
        assert_eq!(first.join().expect("first caller"), ProbeOutcome::LoggedIn);
        assert_eq!(
            second.join().expect("second caller"),
            ProbeOutcome::LoggedIn
        );

        assert_eq!(
            cache.probe(cache_key("codex"), std::time::Instant::now, || {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                ProbeOutcome::LoggedOut
            }),
            ProbeOutcome::LoggedOut
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn configuration_errors_expire_quickly_instead_of_being_cached_indefinitely() {
        use std::cell::Cell;

        let cache = LoginProbeCache::default();
        let started_at = std::time::Instant::now();
        let now = Cell::new(started_at);
        let calls = Cell::new(0_u32);
        let key = cache_key("codex-invalid-config");
        let mut run = || {
            calls.set(calls.get() + 1);
            ProbeOutcome::ConfigInvalid {
                stderr_excerpt: "invalid config".into(),
            }
        };

        assert!(matches!(
            cache.probe(key.clone(), || now.get(), &mut run),
            ProbeOutcome::ConfigInvalid { .. }
        ));
        now.set(started_at + super::CONFIG_ERROR_CACHE_TTL + std::time::Duration::from_millis(1));
        assert!(matches!(
            cache.probe(key, || now.get(), &mut run),
            ProbeOutcome::ConfigInvalid { .. }
        ));
        assert_eq!(calls.get(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_uses_augmented_path_for_env_shebang_interpreter() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let temp = tempfile::tempdir().expect("temp dir");
        let script_dir = temp.path().join("script-bin");
        let interpreter_dir = temp.path().join("interpreter-bin");
        let empty_path_dir = temp.path().join("empty-bin");
        fs::create_dir_all(&script_dir).expect("script dir");
        fs::create_dir_all(&interpreter_dir).expect("interpreter dir");
        fs::create_dir_all(&empty_path_dir).expect("empty path dir");

        let interpreter_path = interpreter_dir.join("node");
        let marker_path = temp.path().join("fake-node-ran");
        fs::write(
            &interpreter_path,
            format!(
                "#!/bin/sh\nprintf 'fake node ran\\n' > '{}' || exit 1\nexit 0\n",
                marker_path.display()
            ),
        )
        .expect("write interpreter");
        fs::set_permissions(&interpreter_path, fs::Permissions::from_mode(0o755))
            .expect("chmod interpreter");

        let script_path = script_dir.join("fake-codex");
        fs::write(&script_path, "#!/usr/bin/env node\n").expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let scrubbed_path = std::env::join_paths([empty_path_dir.as_path()])
            .expect("join scrubbed PATH")
            .to_string_lossy()
            .into_owned();
        let without_augmented_path = Command::new(&script_path)
            .args(["login", "status"])
            .env("PATH", &scrubbed_path)
            .output()
            .expect("run script with scrubbed PATH");
        assert!(
            !without_augmented_path.status.success(),
            "with a scrubbed PATH, /usr/bin/env should not find node"
        );

        let augmented_path =
            std::env::join_paths([interpreter_dir.as_path()]).expect("join augmented PATH");
        let augmented_path = augmented_path.to_string_lossy().into_owned();
        assert_eq!(
            super::login_probe(
                &script_path,
                &["fake-codex", "login", "status"],
                Some(&augmented_path),
            ),
            ProbeOutcome::LoggedIn,
            "the injected augmented PATH should allow /usr/bin/env to find the interpreter"
        );
        assert!(
            marker_path.exists(),
            "the fake node from the injected PATH should have run"
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_config_invalid_on_stderr_signal() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");

        // Script that exits 1 and writes a codex-style config-parse error to stderr.
        let script_path = bin_dir.join("fake-codex-bad-config");
        fs::write(
            &script_path,
            "#!/bin/sh\necho 'Error loading configuration: /home/user/.codex/config.toml: unknown variant `ultra`, expected one of none/minimal/low/medium/high/xhigh' >&2\nexit 1\n",
        )
        .expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let outcome = super::login_probe(
            &script_path,
            &["fake-codex-bad-config", "login", "status"],
            None,
        );
        assert!(
            matches!(outcome, ProbeOutcome::ConfigInvalid { .. }),
            "stderr with 'unknown variant' should produce ConfigInvalid; got {:?}",
            outcome
        );
        if let ProbeOutcome::ConfigInvalid { stderr_excerpt } = outcome {
            assert!(
                stderr_excerpt.contains("unknown variant")
                    || stderr_excerpt.contains("Error loading"),
                "stderr_excerpt should contain the parse error: {stderr_excerpt}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn login_probe_logged_out_on_nonzero_without_config_signal() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");

        // Script that exits 1 with a generic "not logged in" message.
        let script_path = bin_dir.join("fake-codex-logged-out");
        fs::write(
            &script_path,
            "#!/bin/sh\necho 'not authenticated' >&2\nexit 1\n",
        )
        .expect("write script");
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).expect("chmod script");

        let outcome = super::login_probe(
            &script_path,
            &["fake-codex-logged-out", "login", "status"],
            None,
        );
        assert_eq!(
            outcome,
            ProbeOutcome::LoggedOut,
            "non-config stderr should produce LoggedOut"
        );
    }

    /// Verify that every string in CONFIG_PARSE_SIGNALS is lowercased so the
    /// case-insensitive match works correctly.
    #[test]
    fn config_parse_signals_are_lowercase() {
        for sig in CONFIG_PARSE_SIGNALS {
            assert_eq!(
                *sig,
                sig.to_lowercase(),
                "CONFIG_PARSE_SIGNAL must be lowercase for case-insensitive matching: {sig}"
            );
        }
    }
}
