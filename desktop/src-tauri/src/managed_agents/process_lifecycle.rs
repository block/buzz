//! Safe Windows managed-process containment.
//!
//! The desktop executable is a no-console launcher. `processkit::ProcessGroup`
//! creates that launcher suspended, assigns it to a kill-on-close Job Object,
//! and only then resumes it. The launcher reconstructs and starts the real
//! harness with `CREATE_NO_WINDOW`, so no descendant can execute outside the
//! owned Job and no console window is shown.

use super::{AcpAvailabilityStatus, ManagedAgentChild, ManagedAgentProcess};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MANAGED_LAUNCHER_ARG: &str = "--buzz-managed-agent-launcher";
const MANAGED_LAUNCH_PROGRAM_ENV: &str = "BUZZ_MANAGED_LAUNCH_PROGRAM_WIDE";
const MANAGED_LAUNCH_ARGS_ENV: &str = "BUZZ_MANAGED_LAUNCH_ARGS_WIDE";

#[cfg_attr(test, allow(dead_code))]
fn wrap_managed_agent_command_with_launcher(
    command: std::process::Command,
    launcher_exe: &std::path::Path,
    environment_cleared: bool,
) -> Result<std::process::Command, String> {
    use std::os::windows::ffi::OsStrExt;

    let program: Vec<u16> = command.get_program().encode_wide().collect();
    let args: Vec<Vec<u16>> = command
        .get_args()
        .map(|argument| argument.encode_wide().collect())
        .collect();
    let current_dir = command.get_current_dir().map(std::path::Path::to_path_buf);
    let env_changes: Vec<(std::ffi::OsString, Option<std::ffi::OsString>)> = command
        .get_envs()
        .map(|(key, value)| (key.to_os_string(), value.map(std::ffi::OsStr::to_os_string)))
        .collect();

    let mut wrapped = std::process::Command::new(launcher_exe);
    if environment_cleared {
        wrapped.env_clear();
    }
    wrapped.arg(MANAGED_LAUNCHER_ARG);
    if let Some(current_dir) = current_dir {
        wrapped.current_dir(current_dir);
    }
    for (key, value) in env_changes {
        match value {
            Some(value) => {
                wrapped.env(key, value);
            }
            None => {
                wrapped.env_remove(key);
            }
        }
    }
    // The internal envelope is applied last so inherited/user command edits
    // cannot remove or replace desktop launcher authority.
    wrapped
        .env(
            MANAGED_LAUNCH_PROGRAM_ENV,
            serde_json::to_string(&program)
                .map_err(|error| format!("failed to encode managed-agent program: {error}"))?,
        )
        .env(
            MANAGED_LAUNCH_ARGS_ENV,
            serde_json::to_string(&args)
                .map_err(|error| format!("failed to encode managed-agent arguments: {error}"))?,
        );
    Ok(wrapped)
}

#[cfg_attr(test, allow(dead_code))]
fn wrap_managed_agent_command(
    command: std::process::Command,
) -> Result<std::process::Command, String> {
    let launcher_exe = std::env::current_exe()
        .map_err(|error| format!("failed to locate managed-agent launcher: {error}"))?;
    wrap_managed_agent_command_with_launcher(command, &launcher_exe, false)
}

pub(crate) fn run_managed_agent_launcher_if_requested() -> bool {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(MANAGED_LAUNCHER_ARG)) {
        return false;
    }
    let exit_code = match run_managed_agent_launcher() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("buzz-desktop managed-agent launcher: {error}");
            1
        }
    };
    std::process::exit(exit_code);
}

fn run_managed_agent_launcher() -> Result<i32, String> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::process::CommandExt;

    let encoded_program = std::env::var(MANAGED_LAUNCH_PROGRAM_ENV).map_err(|error| {
        format!("missing launcher environment {MANAGED_LAUNCH_PROGRAM_ENV}: {error}")
    })?;
    let encoded_args = std::env::var(MANAGED_LAUNCH_ARGS_ENV).map_err(|error| {
        format!("missing launcher environment {MANAGED_LAUNCH_ARGS_ENV}: {error}")
    })?;
    let program: Vec<u16> = serde_json::from_str(&encoded_program).map_err(|error| {
        format!("invalid launcher environment {MANAGED_LAUNCH_PROGRAM_ENV}: {error}")
    })?;
    if program.is_empty() {
        return Err("managed-agent launcher program is empty".to_string());
    }
    let args: Vec<Vec<u16>> = serde_json::from_str(&encoded_args).map_err(|error| {
        format!("invalid launcher environment {MANAGED_LAUNCH_ARGS_ENV}: {error}")
    })?;

    let mut command = std::process::Command::new(std::ffi::OsString::from_wide(&program));
    command.args(
        args.iter()
            .map(|argument| std::ffi::OsString::from_wide(argument)),
    );
    command
        .env_remove(MANAGED_LAUNCH_PROGRAM_ENV)
        .env_remove(MANAGED_LAUNCH_ARGS_ENV)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
    let status = command
        .status()
        .map_err(|error| format!("failed to start managed-agent harness: {error}"))?;
    status
        .code()
        .ok_or_else(|| "managed-agent harness exited without a numeric status".to_string())
}

/// Spawn the managed command through the no-console desktop launcher directly
/// inside a race-free Windows Job Object.
pub(crate) fn spawn_managed_agent_process(
    command: std::process::Command,
    log_path: PathBuf,
    spawn_config_hash: u64,
    setup_mode: bool,
    adapter_availability: Option<AcpAvailabilityStatus>,
    start_nonce: String,
) -> Result<ManagedAgentProcess, String> {
    #[cfg(not(test))]
    let command = wrap_managed_agent_command(command)?;
    #[cfg(test)]
    let command = command;
    spawn_prepared_managed_agent_process(
        command,
        log_path,
        spawn_config_hash,
        setup_mode,
        adapter_availability,
        start_nonce,
    )
}

pub(crate) fn spawn_managed_agent_process_with_launcher(
    command: std::process::Command,
    launcher_exe: &std::path::Path,
    log_path: PathBuf,
    environment_cleared: bool,
) -> Result<ManagedAgentProcess, String> {
    let command =
        wrap_managed_agent_command_with_launcher(command, launcher_exe, environment_cleared)?;
    spawn_prepared_managed_agent_process(
        command,
        log_path,
        0,
        false,
        None,
        "native-production-boundary-proof".to_string(),
    )
}

fn spawn_prepared_managed_agent_process(
    mut command: std::process::Command,
    log_path: PathBuf,
    spawn_config_hash: u64,
    setup_mode: bool,
    adapter_availability: Option<AcpAvailabilityStatus>,
    start_nonce: String,
) -> Result<ManagedAgentProcess, String> {
    command.stdin(std::process::Stdio::null());
    if log_path.as_os_str().is_empty() {
        command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
    } else {
        let stdout = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|error| format!("failed to open managed-agent log: {error}"))?;
        let stderr = stdout
            .try_clone()
            .map_err(|error| format!("failed to clone managed-agent log handle: {error}"))?;
        command
            .stdout(std::process::Stdio::from(stdout))
            .stderr(std::process::Stdio::from(stderr));
    }

    let group = Arc::new(
        processkit::ProcessGroup::new()
            .map_err(|error| format!("failed to create managed-agent Job Object: {error}"))?,
    );
    let child = group
        .spawn(tokio::process::Command::from(command))
        .map_err(|error| format!("failed to spawn managed agent inside Job Object: {error}"))?;
    let child = ManagedAgentChild::new(child)?;
    Ok(ManagedAgentProcess {
        child,
        log_path,
        spawn_config_hash,
        setup_mode,
        adapter_availability,
        start_nonce,
        job: Some(group),
    })
}

/// Terminate the complete Job tree, prove membership is empty, and reap the
/// exact owned launcher. Every failure leaves both authorities attached.
pub(crate) fn terminate_managed_agent_process(
    process: &mut ManagedAgentProcess,
) -> Result<std::process::ExitStatus, String> {
    let group = process
        .job
        .as_ref()
        .ok_or_else(|| "managed Windows runtime is missing its required Job Object".to_string())?;
    group
        .kill_all()
        .map_err(|error| format!("failed to terminate managed-agent Job Object: {error}"))?;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let members = group
            .members()
            .map_err(|error| format!("failed to query managed-agent Job membership: {error}"))?;
        let launcher_status = process
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect managed launcher exit: {error}"))?;
        if members.is_empty() {
            if let Some(status) = launcher_status {
                return Ok(status);
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "managed-agent Job cleanup remained incomplete after the bounded wait (members: {}, launcher exited: {})",
                members.len(),
                launcher_status.is_some()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Release the empty Job authority only after durable receipt deletion commits.
/// Callers retain the complete process value on every error.
pub(crate) fn release_finalized_managed_agent_process(
    process: &mut ManagedAgentProcess,
) -> Result<(), String> {
    let group = process
        .job
        .as_ref()
        .ok_or_else(|| "managed Windows runtime is missing its required Job Object".to_string())?;
    let members = group
        .members()
        .map_err(|error| format!("failed to recheck managed-agent Job membership: {error}"))?;
    if !members.is_empty() {
        return Err(format!(
            "cannot release managed-agent Job authority while {} member(s) remain",
            members.len()
        ));
    }
    if process
        .child
        .try_wait()
        .map_err(|error| format!("failed to recheck managed launcher exit: {error}"))?
        .is_none()
    {
        return Err("cannot release managed-agent Job authority before launcher exit".to_string());
    }
    process.job.take();
    Ok(())
}

pub(crate) fn finalize_tracked_runtime_with(
    runtime: &mut ManagedAgentProcess,
    remove_receipt: impl FnOnce() -> Result<(), String>,
) -> Result<std::process::ExitStatus, String> {
    if runtime.job.is_none() {
        let status = runtime
            .child
            .try_wait()
            .map_err(|error| format!("failed to recheck finalized managed launcher: {error}"))?
            .ok_or_else(|| {
                "finalized managed runtime lost Job authority before launcher exit".to_string()
            })?;
        remove_receipt()?;
        return Ok(status);
    }
    let status = terminate_managed_agent_process(runtime)?;
    remove_receipt()?;
    // Receipt retirement is the commit point for releasing the empty Job.
    // The reaped Child handle remains an idempotent terminal-proof token if a
    // later recovery-authority persistence step requires an in-memory retry.
    runtime.job.take();
    Ok(status)
}

/// Complete checked process teardown, commit receipt deletion, then retain the
/// reaped Child handle as an idempotent terminal-proof token for the caller.
pub(crate) fn finalize_tracked_runtime(
    app: &tauri::AppHandle,
    key: &super::ManagedAgentRuntimeKey,
    runtime: &mut super::ManagedAgentPairRuntime,
) -> Result<std::process::ExitStatus, String> {
    finalize_tracked_runtime_with(runtime, || super::remove_agent_runtime_receipt(app, key))
}

/// Numeric-PID tree termination is retained only for unrelated cleanup paths;
/// managed runtimes always use their owned child and Job authority instead.
pub(crate) fn taskkill_tree(pid: u32) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut child = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("failed to run taskkill for pid {pid}: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "taskkill exited with status {status} for pid {pid}"
                ));
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = child.kill();
                return Err(format!("taskkill timed out for pid {pid}"));
            }
            Err(error) => {
                return Err(format!("failed waiting for taskkill pid {pid}: {error}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    fn launcher_envelope_preserves_unpaired_wide_argument() {
        use std::os::windows::ffi::OsStringExt;

        let raw = vec![0xd800, 0x0061];
        let mut command = Command::new("target.exe");
        command.arg(std::ffi::OsString::from_wide(&raw));
        let wrapped = wrap_managed_agent_command_with_launcher(
            command,
            std::path::Path::new("launcher.exe"),
            false,
        )
        .unwrap();
        let encoded = wrapped
            .get_envs()
            .find(|(key, _)| key.eq_ignore_ascii_case(MANAGED_LAUNCH_ARGS_ENV))
            .and_then(|(_, value)| value)
            .and_then(|value| value.to_str())
            .unwrap();
        let arguments: Vec<Vec<u16>> = serde_json::from_str(encoded).unwrap();
        assert_eq!(arguments, vec![raw]);
    }

    #[test]
    fn launcher_wrapper_encodes_wide_empty_and_spaced_arguments_last() {
        use std::os::windows::ffi::OsStringExt;

        let dir = tempfile::tempdir().expect("wrapper directory");
        let program = std::ffi::OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            0x96ea,
            b'.' as u16,
            b'e' as u16,
            b'x' as u16,
            b'e' as u16,
        ]);
        let mut original = Command::new(&program);
        original
            .arg("two words")
            .arg("")
            .arg("wide-雪")
            .current_dir(dir.path())
            .env_remove(MANAGED_LAUNCH_PROGRAM_ENV)
            .env_remove(MANAGED_LAUNCH_ARGS_ENV);
        let wrapped = wrap_managed_agent_command(original).expect("wrap managed command");
        assert_eq!(
            wrapped.get_program(),
            std::env::current_exe().expect("current exe")
        );
        assert_eq!(
            wrapped.get_args().collect::<Vec<_>>(),
            vec![std::ffi::OsStr::new(MANAGED_LAUNCHER_ARG)]
        );
        assert_eq!(wrapped.get_current_dir(), Some(dir.path()));
        let envs = wrapped
            .get_envs()
            .collect::<std::collections::BTreeMap<_, _>>();
        let encoded_program = envs[std::ffi::OsStr::new(MANAGED_LAUNCH_PROGRAM_ENV)]
            .expect("program envelope")
            .to_string_lossy();
        let encoded_args = envs[std::ffi::OsStr::new(MANAGED_LAUNCH_ARGS_ENV)]
            .expect("argument envelope")
            .to_string_lossy();
        let decoded_program: Vec<u16> =
            serde_json::from_str(&encoded_program).expect("decode program envelope");
        let decoded_args: Vec<Vec<u16>> =
            serde_json::from_str(&encoded_args).expect("decode argument envelope");
        assert_eq!(std::ffi::OsString::from_wide(&decoded_program), program);
        let decoded_args: Vec<std::ffi::OsString> = decoded_args
            .iter()
            .map(|argument| std::ffi::OsString::from_wide(argument))
            .collect();
        assert_eq!(
            decoded_args,
            vec![
                std::ffi::OsString::from("two words"),
                std::ffi::OsString::from(""),
                std::ffi::OsString::from("wide-雪"),
            ]
        );
    }

    #[test]
    fn safe_job_spawn_preserves_wide_args_cwd_and_environment() {
        let dir = tempfile::tempdir().expect("safe Job spawn directory");
        let output = dir.path().join("spawn.json");
        let script = dir.path().join("probe.ps1");
        std::fs::write(
            &script,
            r#"$payload=[ordered]@{cwd=(Get-Location).Path;value=$env:BUZZ_JOB_TEST;args=@($args)}
$payload|ConvertTo-Json -Compress|Set-Content -Encoding utf8 -LiteralPath $env:BUZZ_JOB_OUTPUT"#,
        )
        .expect("write safe Job spawn probe");
        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&script)
            .arg("two words")
            .arg("")
            .current_dir(dir.path())
            .env("BUZZ_JOB_TEST", "kept")
            .env("BUZZ_JOB_OUTPUT", &output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process = spawn_managed_agent_process(
            command,
            PathBuf::new(),
            0,
            false,
            None,
            "safe-job-test".into(),
        )
        .expect("spawn safe Job probe");
        let deadline = Instant::now() + Duration::from_secs(5);
        while process
            .child
            .try_wait()
            .expect("inspect safe Job probe")
            .is_none()
        {
            assert!(Instant::now() < deadline, "safe Job probe timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(process
            .job
            .as_ref()
            .expect("owned Job")
            .members()
            .expect("query Job")
            .is_empty());
        let bytes = std::fs::read(&output).expect("safe Job probe output");
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let payload: serde_json::Value = serde_json::from_slice(bytes).expect("probe JSON");
        assert_eq!(payload["cwd"], dir.path().display().to_string());
        assert_eq!(payload["value"], "kept");
        assert_eq!(payload["args"][0], "two words");
        assert_eq!(payload["args"][1], "");
        process.job.take();
    }

    #[test]
    fn receipt_deletion_failure_retains_empty_job_and_reaped_child_authority() {
        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "while ($true) { Start-Sleep -Seconds 1 }",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process = spawn_managed_agent_process(
            command,
            PathBuf::new(),
            0,
            false,
            None,
            "receipt-delete-failure".into(),
        )
        .expect("spawn receipt deletion fixture");

        let error = finalize_tracked_runtime_with(&mut process, || {
            Err("synthetic receipt deletion failure".to_string())
        })
        .expect_err("receipt deletion must fail finalization");

        assert!(error.contains("receipt deletion failure"));
        let job = process.job.as_ref().expect("Job authority retained");
        assert!(job.members().expect("query retained Job").is_empty());
        assert!(process
            .child
            .try_wait()
            .expect("inspect retained launcher")
            .is_some());
        release_finalized_managed_agent_process(&mut process)
            .expect("release retained empty Job authority");
    }

    #[test]
    fn terminal_proof_retains_reaped_child_for_idempotent_recovery_clear_retry() {
        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "while ($true) { Start-Sleep -Seconds 1 }",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process = spawn_managed_agent_process(
            command,
            PathBuf::new(),
            0,
            false,
            None,
            "terminal-proof-retry".into(),
        )
        .expect("spawn terminal-proof retry fixture");

        finalize_tracked_runtime_with(&mut process, || Ok(())).expect("first terminal proof");
        assert!(process.job.is_none());
        finalize_tracked_runtime_with(&mut process, || Ok(())).expect("idempotent terminal proof");
    }
}
