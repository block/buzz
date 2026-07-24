use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;

#[cfg(target_os = "macos")]
const HELPER_NAME: &str = "buzz-apple-inputs";
const MAXIMUM_STDOUT_BYTES: usize = 1024 * 1024;
const MAXIMUM_STDERR_BYTES: usize = 64 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AppleInputSource {
    Calendar,
    Reminders,
    Notes,
    Files,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventKitSource {
    Calendar,
    Reminders,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PermissionArguments {
    source: AppleInputSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestPermissionArguments {
    source: EventKitSource,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CalendarArguments {
    calendar_ids: Vec<String>,
    start: String,
    end: String,
    maximum: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReminderArguments {
    list_ids: Vec<String>,
    start: String,
    end: String,
    maximum: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NotesArguments {
    folder_ids: Vec<String>,
    maximum: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FilesArguments {
    paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "operation",
    content = "arguments",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(crate) enum AppleInputRequest {
    PermissionStatus(PermissionArguments),
    RequestPermission(RequestPermissionArguments),
    ReadCalendar(CalendarArguments),
    ReadReminders(ReminderArguments),
    ReadNotes(NotesArguments),
    ReadFiles(FilesArguments),
}

impl AppleInputRequest {
    fn source(&self) -> AppleInputSource {
        match self {
            Self::PermissionStatus(arguments) => arguments.source,
            Self::RequestPermission(arguments) => match arguments.source {
                EventKitSource::Calendar => AppleInputSource::Calendar,
                EventKitSource::Reminders => AppleInputSource::Reminders,
            },
            Self::ReadCalendar(_) => AppleInputSource::Calendar,
            Self::ReadReminders(_) => AppleInputSource::Reminders,
            Self::ReadNotes(_) => AppleInputSource::Notes,
            Self::ReadFiles(_) => AppleInputSource::Files,
        }
    }

    fn timeout(&self) -> Duration {
        match self {
            Self::RequestPermission(_) => PERMISSION_TIMEOUT,
            _ => DEFAULT_TIMEOUT,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AppleInputPermission {
    NotDetermined,
    Denied,
    Authorized,
    Restricted,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppleInputRecord {
    fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AppleInputResponse {
    source: AppleInputSource,
    permission: AppleInputPermission,
    observed_at: String,
    records: Vec<AppleInputRecord>,
    truncated: bool,
    error: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum SupervisionError {
    #[cfg(not(target_os = "macos"))]
    UnsupportedPlatform,
    HelperPath,
    #[cfg(target_os = "macos")]
    HelperNotSibling,
    #[cfg(target_os = "macos")]
    HelperNotExecutable,
    RequestEncoding,
    Spawn,
    Stdin,
    Stdout,
    StdoutLimit,
    Stderr,
    StderrLimit,
    Timeout,
    Exit,
    Protocol,
    Teardown,
    Task,
}

impl SupervisionError {
    fn code(&self) -> &'static str {
        match self {
            #[cfg(not(target_os = "macos"))]
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::HelperPath => "helper_path",
            #[cfg(target_os = "macos")]
            Self::HelperNotSibling => "helper_not_sibling",
            #[cfg(target_os = "macos")]
            Self::HelperNotExecutable => "helper_not_executable",
            Self::RequestEncoding => "request_encoding",
            Self::Spawn => "spawn",
            Self::Stdin => "stdin",
            Self::Stdout => "stdout",
            Self::StdoutLimit => "stdout_limit",
            Self::Stderr => "stderr",
            Self::StderrLimit => "stderr_limit",
            Self::Timeout => "timeout",
            Self::Exit => "exit",
            Self::Protocol => "protocol",
            Self::Teardown => "teardown",
            Self::Task => "task",
        }
    }
}

#[cfg(target_os = "macos")]
fn bundled_helper_path() -> Result<PathBuf, SupervisionError> {
    let current_executable = std::env::current_exe().map_err(|_| SupervisionError::HelperPath)?;
    let sibling_directory = current_executable
        .parent()
        .ok_or(SupervisionError::HelperPath)?;
    let helper_path = sibling_directory.join(HELPER_NAME);
    let metadata =
        std::fs::symlink_metadata(&helper_path).map_err(|_| SupervisionError::HelperPath)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SupervisionError::HelperNotSibling);
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(SupervisionError::HelperNotExecutable);
    }
    let canonical_directory = sibling_directory
        .canonicalize()
        .map_err(|_| SupervisionError::HelperPath)?;
    let canonical_helper = helper_path
        .canonicalize()
        .map_err(|_| SupervisionError::HelperPath)?;
    if canonical_helper.parent() != Some(canonical_directory.as_path()) {
        return Err(SupervisionError::HelperNotSibling);
    }
    Ok(canonical_helper)
}

#[cfg(not(target_os = "macos"))]
fn bundled_helper_path() -> Result<PathBuf, SupervisionError> {
    Err(SupervisionError::UnsupportedPlatform)
}

fn terminate(child: &mut Child) -> Result<(), SupervisionError> {
    match child.try_wait() {
        Ok(Some(_)) => Ok(()),
        Ok(None) => {
            child.kill().map_err(|_| SupervisionError::Teardown)?;
            child.wait().map_err(|_| SupervisionError::Teardown)?;
            Ok(())
        }
        Err(_) => Err(SupervisionError::Teardown),
    }
}

fn read_bounded<R: Read>(
    reader: R,
    maximum: usize,
    read_error: SupervisionError,
    limit_error: SupervisionError,
) -> Result<Vec<u8>, SupervisionError> {
    let mut bytes = Vec::with_capacity(maximum.min(8192));
    reader
        .take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| read_error)?;
    if bytes.len() > maximum {
        return Err(limit_error);
    }
    Ok(bytes)
}

fn receive_pipe(
    receiver: &Receiver<Result<Vec<u8>, SupervisionError>>,
    cached: &mut Option<Vec<u8>>,
) -> Result<(), SupervisionError> {
    if cached.is_some() {
        return Ok(());
    }
    match receiver.try_recv() {
        Ok(Ok(bytes)) => {
            *cached = Some(bytes);
            Ok(())
        }
        Ok(Err(error)) => Err(error),
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(SupervisionError::Task),
    }
}

fn validate_response(
    stdout: &[u8],
    expected_source: AppleInputSource,
) -> Result<AppleInputResponse, SupervisionError> {
    let line = stdout
        .strip_suffix(b"\n")
        .ok_or(SupervisionError::Protocol)?;
    if line.is_empty() || line.contains(&b'\n') || line.contains(&b'\r') {
        return Err(SupervisionError::Protocol);
    }
    let response: AppleInputResponse =
        serde_json::from_slice(line).map_err(|_| SupervisionError::Protocol)?;
    if response.source != expected_source
        || chrono::DateTime::parse_from_rfc3339(&response.observed_at).is_err()
        || response.records.len() > 100
        || response
            .error
            .as_ref()
            .is_some_and(|error| error.len() > 4096)
    {
        return Err(SupervisionError::Protocol);
    }
    for record in &response.records {
        if record.fields.is_empty()
            || record.fields.len() > 32
            || record.fields.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > 128
                    || value.len() > MAXIMUM_STDOUT_BYTES
                    || key.contains('\0')
                    || value.contains('\0')
            })
        {
            return Err(SupervisionError::Protocol);
        }
    }
    Ok(response)
}

fn supervise_helper(
    helper_path: &Path,
    request: &AppleInputRequest,
    timeout: Duration,
) -> Result<AppleInputResponse, SupervisionError> {
    let working_directory = helper_path.parent().ok_or(SupervisionError::HelperPath)?;
    let mut request_line =
        serde_json::to_vec(request).map_err(|_| SupervisionError::RequestEncoding)?;
    request_line.push(b'\n');
    if request_line.len() > 64 * 1024 {
        return Err(SupervisionError::RequestEncoding);
    }

    let mut child = Command::new(helper_path)
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("PATH", "/usr/bin:/bin")
        .current_dir(working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| SupervisionError::Spawn)?;
    let mut stdin = child.stdin.take().ok_or(SupervisionError::Spawn)?;
    let stdout = child.stdout.take().ok_or(SupervisionError::Spawn)?;
    let stderr = child.stderr.take().ok_or(SupervisionError::Spawn)?;

    let (stdin_sender, stdin_receiver) = mpsc::sync_channel(1);
    let stdin_thread = std::thread::spawn(move || {
        let result = stdin
            .write_all(&request_line)
            .and_then(|_| stdin.flush())
            .map_err(|_| SupervisionError::Stdin);
        drop(stdin);
        let _ = stdin_sender.send(result);
    });
    let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
    let stdout_thread = std::thread::spawn(move || {
        let result = read_bounded(
            stdout,
            MAXIMUM_STDOUT_BYTES,
            SupervisionError::Stdout,
            SupervisionError::StdoutLimit,
        );
        let _ = stdout_sender.send(result);
    });
    let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
    let stderr_thread = std::thread::spawn(move || {
        let result = read_bounded(
            stderr,
            MAXIMUM_STDERR_BYTES,
            SupervisionError::Stderr,
            SupervisionError::StderrLimit,
        );
        let _ = stderr_sender.send(result);
    });

    let started = std::time::Instant::now();
    let mut stdout_bytes = None;
    let mut stderr_bytes = None;
    let status = loop {
        if let Err(error) = receive_pipe(&stdout_receiver, &mut stdout_bytes) {
            break match terminate(&mut child) {
                Ok(()) => Err(error),
                Err(teardown) => Err(teardown),
            };
        }
        if let Err(error) = receive_pipe(&stderr_receiver, &mut stderr_bytes) {
            break match terminate(&mut child) {
                Ok(()) => Err(error),
                Err(teardown) => Err(teardown),
            };
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                let teardown = terminate(&mut child);
                break teardown.and(Err(SupervisionError::Timeout));
            }
            Err(_) => {
                let _ = terminate(&mut child);
                break Err(SupervisionError::Teardown);
            }
        }
    };

    let stdin_joined = stdin_thread.join().map_err(|_| SupervisionError::Task);
    let stdout_joined = stdout_thread.join().map_err(|_| SupervisionError::Task);
    let stderr_joined = stderr_thread.join().map_err(|_| SupervisionError::Task);
    stdin_joined?;
    stdout_joined?;
    stderr_joined?;
    stdin_receiver
        .recv()
        .map_err(|_| SupervisionError::Task)??;
    let status = status?;
    if !status.success() {
        return Err(SupervisionError::Exit);
    }
    if stdout_bytes.is_none() {
        stdout_bytes = Some(
            stdout_receiver
                .recv()
                .map_err(|_| SupervisionError::Task)??,
        );
    }
    if stderr_bytes.is_none() {
        stderr_bytes = Some(
            stderr_receiver
                .recv()
                .map_err(|_| SupervisionError::Task)??,
        );
    }
    let _bounded_stderr = stderr_bytes.ok_or(SupervisionError::Task)?;
    validate_response(
        &stdout_bytes.ok_or(SupervisionError::Task)?,
        request.source(),
    )
}

fn fail_soft(source: AppleInputSource, error: &SupervisionError) -> AppleInputResponse {
    AppleInputResponse {
        source,
        permission: AppleInputPermission::Unavailable,
        observed_at: chrono::Utc::now().to_rfc3339(),
        records: Vec::new(),
        truncated: false,
        error: Some(format!("apple input helper unavailable: {}", error.code())),
    }
}

#[tauri::command]
pub(crate) async fn read_apple_inputs(request: AppleInputRequest) -> AppleInputResponse {
    let source = request.source();
    let timeout = request.timeout();
    let helper_path = match bundled_helper_path() {
        Ok(path) => path,
        Err(error) => return fail_soft(source, &error),
    };
    let task = tauri::async_runtime::spawn_blocking(move || {
        supervise_helper(&helper_path, &request, timeout)
    });
    match task.await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => fail_soft(source, &error),
        Err(_) => fail_soft(source, &SupervisionError::Task),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn permission_request() -> AppleInputRequest {
        AppleInputRequest::PermissionStatus(PermissionArguments {
            source: AppleInputSource::Calendar,
        })
    }

    fn response(extra: &str) -> String {
        format!(
            r#"{{"source":"calendar","permission":"not_determined","observedAt":"2026-07-24T00:00:00Z","records":[],"truncated":false,"error":null{extra}}}"#
        )
    }

    fn fixture_script(body: &str) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("temporary fixture directory");
        let path = directory.path().join("helper");
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).expect("write fixture");
        let mut permissions = fs::metadata(&path).expect("fixture metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("make fixture executable");
        (directory, path)
    }

    #[test]
    fn supervises_with_cleared_environment_fixed_workdir_and_exact_request() {
        std::env::set_var("BUZZ_APPLE_INPUTS_INHERITED_SENTINEL", "secret");
        let expected = serde_json::to_string(&permission_request()).expect("serialize request");
        let body = format!(
            r#"
test -z "${{BUZZ_APPLE_INPUTS_INHERITED_SENTINEL+x}}"
actual_workdir=$(pwd -P)
expected_workdir=$(cd "$(dirname "$0")" && pwd -P)
test "$actual_workdir" = "$expected_workdir"
IFS= read -r request
test "$request" = '{expected}'
printf '%s\n' '{}'
"#,
            response("")
        );
        let (_directory, helper) = fixture_script(&body);

        let result = supervise_helper(&helper, &permission_request(), Duration::from_secs(2))
            .expect("valid helper response");

        std::env::remove_var("BUZZ_APPLE_INPUTS_INHERITED_SENTINEL");
        assert!(matches!(
            result.permission,
            AppleInputPermission::NotDetermined
        ));
        assert!(result.records.is_empty());
    }

    #[test]
    fn rejects_unknown_response_keys() {
        let body = format!(
            "IFS= read -r _request\nprintf '%s\\n' '{}'",
            response(r#","extra":true"#)
        );
        let (_directory, helper) = fixture_script(&body);

        let error = supervise_helper(&helper, &permission_request(), Duration::from_millis(500))
            .expect_err("unknown response key must fail");

        assert_ne!(error.code(), "unimplemented");
    }

    #[test]
    fn rejects_source_mismatch_and_multiple_response_lines() {
        let wrong_source = response("").replace(r#""source":"calendar""#, r#""source":"notes""#);
        let body = format!("IFS= read -r _request\nprintf '%s\\n' '{wrong_source}'");
        let (_directory, helper) = fixture_script(&body);
        assert!(
            supervise_helper(&helper, &permission_request(), Duration::from_millis(500)).is_err()
        );

        let body = format!(
            "IFS= read -r _request\nprintf '%s\\n%s\\n' '{}' '{}'",
            response(""),
            response("")
        );
        let (_directory, helper) = fixture_script(&body);
        assert!(
            supervise_helper(&helper, &permission_request(), Duration::from_millis(500)).is_err()
        );
    }

    #[test]
    fn rejects_oversized_stdout_and_stderr() {
        let body = format!(
            "IFS= read -r _request\nhead -c {} /dev/zero | tr '\\0' x",
            MAXIMUM_STDOUT_BYTES + 1
        );
        let (_directory, helper) = fixture_script(&body);
        assert!(supervise_helper(&helper, &permission_request(), Duration::from_secs(2)).is_err());

        let body = format!(
            "IFS= read -r _request\nhead -c {} /dev/zero | tr '\\0' x >&2\nprintf '%s\\n' '{}'",
            MAXIMUM_STDERR_BYTES + 1,
            response("")
        );
        let (_directory, helper) = fixture_script(&body);
        assert!(supervise_helper(&helper, &permission_request(), Duration::from_secs(2)).is_err());
    }

    #[test]
    fn kills_helper_at_deadline() {
        let (_directory, helper) =
            fixture_script("IFS= read -r _request\nsleep 5\nprintf '%s\\n' '{}'");
        let started = std::time::Instant::now();

        let error = supervise_helper(&helper, &permission_request(), Duration::from_millis(100))
            .expect_err("hung helper must time out");

        assert_ne!(error.code(), "unimplemented");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn fail_soft_result_is_source_scoped_and_redacted() {
        let result = fail_soft(AppleInputSource::Files, &SupervisionError::Task);
        assert!(matches!(result.source, AppleInputSource::Files));
        assert!(matches!(
            result.permission,
            AppleInputPermission::Unavailable
        ));
        assert_eq!(
            result.error.as_deref(),
            Some("apple input helper unavailable: task")
        );
        assert!(result.records.is_empty());
    }

    #[test]
    fn explicit_permission_request_rejects_non_eventkit_sources() {
        let decoded = serde_json::from_str::<AppleInputRequest>(
            r#"{"operation":"request_permission","arguments":{"source":"notes"}}"#,
        );
        assert!(decoded.is_err());
    }
}
