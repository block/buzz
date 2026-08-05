use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProbeCacheKey {
    generation: u64,
    runtime: String,
    binary_path: PathBuf,
    args: Vec<String>,
    effective_path: Option<String>,
}

fn probe_cache_generation() -> &'static AtomicU64 {
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    &GENERATION
}

fn probe_cache() -> &'static Mutex<HashMap<ProbeCacheKey, (Instant, ProbeOutcome)>> {
    static CACHE: OnceLock<Mutex<HashMap<ProbeCacheKey, (Instant, ProbeOutcome)>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn clear_login_probe_cache() {
    probe_cache_generation().fetch_add(1, Ordering::AcqRel);
    // Invalidation must never wait behind a slow external probe, especially
    // when a config save currently owns a runtime-management lock.
    if let Ok(mut cache) = probe_cache().try_lock() {
        cache.clear();
    }
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
        generation: probe_cache_generation().load(Ordering::Acquire),
        runtime: probe_args.first().copied().unwrap_or_default().to_string(),
        binary_path: binary_path.to_path_buf(),
        args: probe_args
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        effective_path: augmented_path.map(str::to_owned),
    };
    let mut cache = probe_cache()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    cache.retain(|candidate, _| candidate.generation == key.generation);
    if let Some((created_at, outcome)) = cache.get(&key) {
        let ttl = if matches!(outcome, ProbeOutcome::ConfigInvalid { .. }) {
            CONFIG_ERROR_CACHE_TTL
        } else {
            LOGIN_PROBE_CACHE_TTL
        };
        if created_at.elapsed() < ttl {
            return outcome.clone();
        }
    }
    cache.remove(&key);

    let outcome = run_login_probe(binary_path, probe_args, augmented_path, timeout);
    // Config parse failures receive the shorter TTL above. This still
    // deduplicates a multi-row list call without hiding an external repair.
    cache.insert(key, (Instant::now(), outcome.clone()));
    outcome
}

fn run_login_probe(
    binary_path: &Path,
    probe_args: &[&str],
    augmented_path: Option<&str>,
    timeout: Duration,
) -> ProbeOutcome {
    let mut command = std::process::Command::new(binary_path);
    command.args(&probe_args[1..]);
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }
    crate::util::configure_no_window(&mut command);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let Ok(mut child) = command.spawn() else {
        return ProbeOutcome::LoggedOut;
    };
    let stdout_reader = child.stdout.take().map(spawn_bounded_reader);
    let stderr_reader = child.stderr.take().map(spawn_bounded_reader);
    let started_at = Instant::now();
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), false),
            Ok(None) if started_at.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                kill_probe_process_tree(&mut child);
                break (child.wait().ok(), true);
            }
            Err(_) => {
                kill_probe_process_tree(&mut child);
                let _ = child.wait();
                break (None, false);
            }
        }
    };
    let _stdout = join_reader(stdout_reader);
    let stderr = join_reader(stderr_reader);
    if timed_out {
        ProbeOutcome::TimedOut
    } else if let Some(status) = status {
        classify_probe_output(&stderr, status.success())
    } else {
        ProbeOutcome::LoggedOut
    }
}

fn spawn_bounded_reader<R>(mut reader: R) -> std::thread::JoinHandle<Vec<u8>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut retained = Vec::new();
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            let Ok(count) = reader.read(&mut chunk) else {
                break;
            };
            if count == 0 {
                break;
            }
            let remaining = MAX_PROBE_OUTPUT_BYTES.saturating_sub(retained.len());
            retained.extend_from_slice(&chunk[..count.min(remaining)]);
        }
        retained
    })
}

fn join_reader(reader: Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

fn kill_probe_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    // SAFETY: the child was placed in a fresh process group whose id is its
    // pid. A negative pid targets only that group.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}

/// Classify collected probe output into a `ProbeOutcome`.
///
/// Shared between `login_probe` (which has the full `Output`) and the
/// process-level timeout path in `probe_auth_status` (which drains stderr
/// on a background thread and collects it separately).
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
    use super::{ProbeOutcome, CONFIG_PARSE_SIGNALS};

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
        // SAFETY: signal 0 only checks whether this exact pid still exists.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "probe child survived");
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
    fn login_probe_cache_deduplicates_and_explicitly_invalidates() {
        use std::fs;

        super::clear_login_probe_cache();
        let (temp, script_path) = executable_script("#!/bin/sh\nprintf x >> \"$1\"\nexit 0\n");
        let marker = temp.path().join("calls");
        let marker_arg = marker.to_string_lossy().into_owned();
        let args = ["probe-script", marker_arg.as_str()];
        let mut stable_generation_observed = false;
        for _ in 0..20 {
            super::clear_login_probe_cache();
            let _ = fs::remove_file(&marker);
            let generation =
                super::probe_cache_generation().load(std::sync::atomic::Ordering::Acquire);
            assert_eq!(
                super::login_probe(&script_path, &args, None),
                ProbeOutcome::LoggedIn
            );
            assert_eq!(
                super::login_probe(&script_path, &args, None),
                ProbeOutcome::LoggedIn
            );
            if generation
                == super::probe_cache_generation().load(std::sync::atomic::Ordering::Acquire)
            {
                assert_eq!(fs::read_to_string(&marker).expect("call marker"), "x");
                stable_generation_observed = true;
                break;
            }
        }
        assert!(
            stable_generation_observed,
            "probe cache was invalidated continuously during the test"
        );

        super::clear_login_probe_cache();
        assert_eq!(
            super::login_probe(&script_path, &args, None),
            ProbeOutcome::LoggedIn
        );
        assert_eq!(fs::read_to_string(marker).expect("call marker"), "xx");
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
