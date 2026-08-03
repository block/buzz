use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use buzz_runtime::{
    current_process_start_marker, read_job_spec, write_runner_receipt, JobSpec, RedactingWriter,
    RotatingLogWriter, RunnerReceipt, RunnerReceiptState, RUNNER_RECEIPT_SCHEMA_VERSION,
};
use chrono::Utc;
#[cfg(unix)]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[cfg(test)]
use uuid::Uuid;

pub(crate) const MAX_JOB_SPEC_BYTES: u64 = 64 * 1024;
const MAX_REDACTION_SECRETS: usize = 256;
const MAX_SECRET_VALUE_BYTES: usize = 64 * 1024;
const MAX_TOTAL_SECRET_BYTES: usize = 1024 * 1024;

// An LH child gets only process-discovery, home/config discovery, locale, and
// temporary-directory values. Credentials (including SSH agents) and all Buzz
// runtime/operator configuration are deliberately absent.
const LH_CHILD_ENV_ALLOWLIST: &[&str] = &[
    "HOME",
    "PATH",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TZ",
    "TERM",
    "NO_COLOR",
    // Windows needs these to launch an absolute executable and its descendants.
    "SystemRoot",
    "WINDIR",
    "ComSpec",
    "PATHEXT",
];

struct RunnerEnvironment {
    child: Vec<(OsString, OsString)>,
    secrets: Vec<Vec<u8>>,
}

pub(crate) fn run_from_process_args() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 3 || args.get(1).and_then(|value| value.to_str()) != Some("__job-runner") {
        bail!("invalid __job-runner invocation");
    }
    let environment = capture_runner_environment()?;
    remove_runtime_credentials();
    run(Path::new(&args[2]), environment)
}

fn capture_runner_environment() -> Result<RunnerEnvironment> {
    let mut child = Vec::new();
    let mut secrets = Vec::new();
    let mut total_secret_bytes = 0_usize;
    for (name, value) in std::env::vars_os() {
        if is_lh_child_environment_name(&name) {
            child.push((name.clone(), value.clone()));
        }
        if !is_sensitive_environment_name(&name) {
            continue;
        }
        let value = environment_value_bytes(&value);
        if value.is_empty() {
            continue;
        }
        if value.len() > MAX_SECRET_VALUE_BYTES
            || secrets.len() >= MAX_REDACTION_SECRETS
            || total_secret_bytes.saturating_add(value.len()) > MAX_TOTAL_SECRET_BYTES
        {
            bail!("secret environment exceeds bounded redaction capacity");
        }
        total_secret_bytes += value.len();
        secrets.push(value);
    }
    Ok(RunnerEnvironment { child, secrets })
}

fn remove_runtime_credentials() {
    let sensitive_names: Vec<OsString> = std::env::vars_os()
        .map(|(name, _)| name)
        .filter(|name| is_buzz_environment_name(name) || is_sensitive_environment_name(name))
        .collect();
    for name in sensitive_names {
        std::env::remove_var(name);
    }
}

fn is_lh_child_environment_name(name: &OsStr) -> bool {
    #[cfg(windows)]
    {
        let name = name.to_string_lossy();
        return LH_CHILD_ENV_ALLOWLIST
            .iter()
            .any(|allowed| name.eq_ignore_ascii_case(allowed));
    }
    #[cfg(not(windows))]
    LH_CHILD_ENV_ALLOWLIST
        .iter()
        .any(|allowed| name == OsStr::new(allowed))
}

fn is_buzz_environment_name(name: &OsStr) -> bool {
    name.to_string_lossy()
        .to_ascii_uppercase()
        .starts_with("BUZZ_")
}

fn is_sensitive_environment_name(name: &OsStr) -> bool {
    let name = name.to_string_lossy().to_ascii_uppercase();
    name == "BUZZ_RUNTIME_RECEIPT"
        || name == "BUZZ_RELAY_URL"
        || name == "DATABASE_URL"
        || name == "REDIS_URL"
        || name == "GH_TOKEN"
        || name == "GITHUB_TOKEN"
        || name == "GITLAB_TOKEN"
        || name == "NPM_TOKEN"
        || name == "NODE_AUTH_TOKEN"
        || name == "HF_TOKEN"
        || name == "AWS_ACCESS_KEY_ID"
        || name == "AWS_SECRET_ACCESS_KEY"
        || name == "AWS_SESSION_TOKEN"
        || [
            "_API_KEY",
            "_API_TOKEN",
            "_ACCESS_TOKEN",
            "_AUTH_TOKEN",
            "_SESSION_TOKEN",
            "_CONTROL_TOKEN",
            "_MODEL_TOKEN",
            "_PRIVATE_KEY",
            "_SIGNING_KEY",
            "_SECRET",
            "_SECRET_KEY",
            "_AUTH_TAG",
            "_PASSWORD",
            "_PASSWD",
            "_CREDENTIAL",
            "_CREDENTIALS",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

#[cfg(unix)]
fn environment_value_bytes(value: &OsStr) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().to_vec()
}

#[cfg(not(unix))]
fn environment_value_bytes(value: &OsStr) -> Vec<u8> {
    value.to_string_lossy().into_owned().into_bytes()
}

fn run(spec_path: &Path, environment: RunnerEnvironment) -> Result<()> {
    let spec = read_spec_bounded(spec_path)?;
    let attempt_dir = spec_path
        .parent()
        .context("job spec path has no attempt directory")?;
    validate_attempt_path(attempt_dir, &spec)?;
    let runtime_dir = attempt_dir
        .ancestors()
        .nth(3)
        .context("job spec is not below runtime/jobs/<job>/<attempt>")?;
    let canonical_spec = std::fs::canonicalize(spec_path).context("canonicalize job spec path")?;
    if !spec_path.is_absolute() || canonical_spec != spec_path {
        bail!("job spec path must be absolute and canonical");
    }
    for directory in attempt_dir.ancestors().take(4) {
        let metadata = std::fs::symlink_metadata(directory)
            .with_context(|| format!("inspect job artifact directory {}", directory.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("job artifact ancestors must be real directories");
        }
    }

    let runner_pid = std::process::id();
    #[cfg(unix)]
    let process_group = establish_process_group_identity(runner_pid)?;
    #[cfg(windows)]
    let job_object = {
        let name = crate::job_windows::job_name(&spec.runtime_id, spec.job_id, spec.attempt);
        crate::job_windows::NamedJobObject::create_for_current(name)
            .context("create and assign durable named Job Object")?
    };
    #[cfg(windows)]
    let process_group = job_object.name().to_string();
    let runner_start_marker =
        current_process_start_marker().context("read runner process identity")?;
    let started_at = Utc::now();

    #[cfg(unix)]
    let cancelled = {
        let cancelled = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&cancelled))
            .context("install runner cancellation handler")?;
        cancelled
    };

    let stdout_path = attempt_dir.join("stdout.log");
    let stderr_path = attempt_dir.join("stderr.log");
    let mut command = Command::new(&spec.executable);
    command
        .args(&spec.request.argv)
        .current_dir(&spec.request.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env_clear().envs(environment.child);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let terminal = RunnerReceipt {
                schema_version: RUNNER_RECEIPT_SCHEMA_VERSION,
                job_id: spec.job_id,
                attempt: spec.attempt,
                state: RunnerReceiptState::Failed,
                runner_pid,
                runner_start_marker,
                process_group,
                argv_sha256: spec.argv_sha256,
                started_at,
                finished_at: Some(Utc::now()),
                exit_code: None,
                error_code: Some("driver_spawn_failed".into()),
            };
            write_runner_receipt(runtime_dir, &terminal)?;
            return Err(error).context("spawn LH driver");
        }
    };

    let stdout = child.stdout.take().context("LH stdout pipe unavailable")?;
    let stderr = child.stderr.take().context("LH stderr pipe unavailable")?;
    let stdout_drain = spawn_drain(stdout, stdout_path, environment.secrets.clone());
    let stderr_drain = spawn_drain(stderr, stderr_path, environment.secrets);

    let ready = RunnerReceipt {
        schema_version: RUNNER_RECEIPT_SCHEMA_VERSION,
        job_id: spec.job_id,
        attempt: spec.attempt,
        state: RunnerReceiptState::Ready,
        runner_pid,
        runner_start_marker: runner_start_marker.clone(),
        process_group: process_group.clone(),
        argv_sha256: spec.argv_sha256.clone(),
        started_at,
        finished_at: None,
        exit_code: None,
        error_code: None,
    };
    write_runner_receipt(runtime_dir, &ready)?;

    let status = child.wait().context("wait for LH driver")?;
    #[cfg(unix)]
    let descendant_check = process_group_has_live_members(runner_pid, Some(runner_pid));
    #[cfg(windows)]
    let descendant_check = governed_tree_has_descendants(runner_pid, &job_object);
    let descendant_error_code = match descendant_check {
        Ok(false) => None,
        Ok(true) | Err(_) => Some("orphan_suspected"),
    };
    if let Some(error_code) = descendant_error_code {
        let terminal = RunnerReceipt {
            state: RunnerReceiptState::Failed,
            finished_at: Some(Utc::now()),
            exit_code: status.code(),
            error_code: Some(error_code.into()),
            ..ready
        };
        write_runner_receipt(runtime_dir, &terminal)?;
        #[cfg(unix)]
        terminate_current_governed_tree(runner_pid)?;
        #[cfg(windows)]
        terminate_current_governed_tree(runner_pid, &job_object)?;
        bail!("governed driver exited without proving its process tree empty");
    }
    let stdout_result = stdout_drain
        .join()
        .map_err(|_| anyhow::anyhow!("stdout drain thread panicked"))?;
    let stderr_result = stderr_drain
        .join()
        .map_err(|_| anyhow::anyhow!("stderr drain thread panicked"))?;
    stdout_result.context("drain LH stdout")?;
    stderr_result.context("drain LH stderr")?;

    #[cfg(unix)]
    let was_cancelled = cancelled.load(Ordering::SeqCst);
    #[cfg(not(unix))]
    let was_cancelled = false;
    let state = if was_cancelled {
        RunnerReceiptState::Cancelled
    } else if status.success() {
        RunnerReceiptState::Succeeded
    } else {
        RunnerReceiptState::Failed
    };
    let terminal = RunnerReceipt {
        state,
        finished_at: Some(Utc::now()),
        exit_code: status.code(),
        error_code: if was_cancelled {
            Some("cancelled".into())
        } else {
            (!status.success()).then(|| "driver_exit_nonzero".into())
        },
        ..ready
    };
    write_runner_receipt(runtime_dir, &terminal)?;
    #[cfg(windows)]
    job_object.disarm();
    Ok(())
}

fn read_spec_bounded(path: &Path) -> Result<JobSpec> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect job spec {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("job spec must be a regular non-symlink file");
    }
    if metadata.len() > MAX_JOB_SPEC_BYTES {
        bail!("job spec exceeds 64 KiB");
    }
    read_job_spec(path).context("parse and validate job spec")
}

fn validate_attempt_path(attempt_dir: &Path, spec: &JobSpec) -> Result<()> {
    let attempt_name = attempt_dir.file_name().and_then(|name| name.to_str());
    let job_name = attempt_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    let expected_attempt = spec.attempt.to_string();
    let expected_job = spec.job_id.to_string();
    if attempt_name != Some(expected_attempt.as_str()) || job_name != Some(expected_job.as_str()) {
        bail!("job spec identity does not match its durable path");
    }
    Ok(())
}

fn spawn_drain<R>(
    reader: R,
    path: PathBuf,
    secrets: Vec<Vec<u8>>,
) -> std::thread::JoinHandle<Result<()>>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || drain_stream(reader, &path, secrets))
}

fn drain_stream(mut reader: impl Read, path: &Path, secrets: Vec<Vec<u8>>) -> Result<()> {
    let rotating = RotatingLogWriter::open(path)?;
    let mut writer = RedactingWriter::new(rotating, secrets);
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).context("read process output")?;
        if read == 0 {
            writer.finish()?;
            return Ok(());
        }
        writer.write_all(&buffer[..read])?;
    }
}

#[cfg(unix)]
fn establish_process_group_identity(pid: u32) -> Result<String> {
    let pgid = nix::unistd::getpgrp().as_raw();
    if pgid != pid as i32 {
        bail!("runner is not its process-group leader");
    }
    Ok(pgid.to_string())
}
#[cfg(target_os = "linux")]
pub(crate) fn process_group_has_live_members(
    process_group: u32,
    excluded_pid: Option<u32>,
) -> Result<bool> {
    for entry in std::fs::read_dir("/proc").context("enumerate Linux processes")? {
        let entry = entry.context("read Linux process entry")?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        if excluded_pid == Some(pid) {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some(after_name) = stat.rsplit_once(')').map(|(_, suffix)| suffix) else {
            continue;
        };
        let mut fields = after_name.split_whitespace();
        let state = fields.next();
        let _parent_pid = fields.next();
        let member_group = fields.next().and_then(|value| value.parse::<u32>().ok());
        if member_group == Some(process_group) && state != Some("Z") {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub(crate) fn process_group_has_live_members(
    process_group: u32,
    excluded_pid: Option<u32>,
) -> Result<bool> {
    use nix::libc;

    const PROC_PGRP_ONLY: u32 = 2;
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_listpids(
            pid_type: u32,
            type_info: u32,
            buffer: *mut libc::c_void,
            buffer_size: libc::c_int,
        ) -> libc::c_int;
    }

    let required_bytes =
        unsafe { proc_listpids(PROC_PGRP_ONLY, process_group, std::ptr::null_mut(), 0) };
    if required_bytes < 0 {
        return Err(std::io::Error::last_os_error()).context("size process-group inventory");
    }
    if required_bytes == 0 {
        return Ok(false);
    }

    let pid_size = std::mem::size_of::<libc::pid_t>();
    let capacity = (required_bytes as usize)
        .checked_add(pid_size * 32)
        .context("process-group inventory size overflow")?
        / pid_size;
    let mut pids = vec![0 as libc::pid_t; capacity];
    let read_bytes = unsafe {
        proc_listpids(
            PROC_PGRP_ONLY,
            process_group,
            pids.as_mut_ptr().cast(),
            (pids.len() * pid_size)
                .try_into()
                .context("process-group inventory exceeds proc_listpids limit")?,
        )
    };
    if read_bytes < 0 {
        return Err(std::io::Error::last_os_error()).context("read process-group inventory");
    }
    pids.truncate(read_bytes as usize / pid_size);

    Ok(pids.into_iter().any(|pid| {
        if pid <= 0 || excluded_pid == Some(pid as u32) {
            return false;
        }
        if unsafe { libc::kill(pid, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
pub(crate) fn process_group_has_live_members(
    _process_group: u32,
    _excluded_pid: Option<u32>,
) -> Result<bool> {
    bail!("process-group descendant verification is unsupported on this Unix platform")
}

#[cfg(windows)]
fn governed_tree_has_descendants(
    _runner_pid: u32,
    job_object: &crate::job_windows::NamedJobObject,
) -> Result<bool> {
    job_object
        .has_other_active_processes()
        .context("query governed Job Object membership")
}

#[cfg(unix)]
fn terminate_current_governed_tree(runner_pid: u32) -> Result<()> {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;

    killpg(Pid::from_raw(runner_pid as i32), Signal::SIGKILL)
        .context("terminate driver process group")
}

#[cfg(windows)]
fn terminate_current_governed_tree(
    _runner_pid: u32,
    job_object: &crate::job_windows::NamedJobObject,
) -> Result<()> {
    job_object
        .terminate_all()
        .context("terminate driver Job Object")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_receipt_uses_fixed_camel_case_schema() {
        let receipt = RunnerReceipt {
            schema_version: 1,
            job_id: Uuid::nil(),
            attempt: 1,
            state: RunnerReceiptState::Ready,
            runner_pid: 7,
            runner_start_marker: "marker".into(),
            process_group: "7".into(),
            argv_sha256: "a".repeat(64),
            started_at: Utc::now(),
            finished_at: None,
            exit_code: None,
            error_code: None,
        };
        let value = serde_json::to_value(receipt).expect("serialize receipt");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["state"], "ready");
        assert!(value.get("runnerPid").is_some());
        assert!(value.get("runner_pid").is_none());
    }

    #[test]
    fn lh_child_environment_is_exact_and_excludes_credentials() {
        for safe in ["HOME", "PATH", "TMPDIR", "LANG", "SystemRoot"] {
            assert!(is_lh_child_environment_name(OsStr::new(safe)));
        }
        for denied in [
            "OPENAI_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "SSH_AUTH_SOCK",
            "BUZZ_RUNTIME_RECEIPT",
            "BUZZ_RUNTIME_CONTROL_TOKEN",
            "UNRELATED_CONFIG",
        ] {
            assert!(!is_lh_child_environment_name(OsStr::new(denied)));
        }
    }

    #[test]
    fn sensitive_environment_names_cover_runtime_and_provider_credentials() {
        for secret in [
            "BUZZ_PRIVATE_KEY",
            "BUZZ_RUNTIME_MODEL_TOKEN",
            "OPENAI_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "DATABASE_URL",
        ] {
            assert!(is_sensitive_environment_name(OsStr::new(secret)));
        }
        for public in ["HOME", "PATH", "LANG", "TOKENIZERS_PARALLELISM"] {
            assert!(!is_sensitive_environment_name(OsStr::new(public)));
        }
    }
}
