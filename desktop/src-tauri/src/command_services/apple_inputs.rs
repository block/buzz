use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use crate::command_services::ssh::ProtectedFile;

#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;

#[cfg(target_os = "macos")]
const HELPER_NAME: &str = "buzz-apple-inputs";
const MAXIMUM_STDOUT_BYTES: usize = 1024 * 1024;
const MAXIMUM_STDERR_BYTES: usize = 64 * 1024;
const MAXIMUM_REQUEST_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(30);
#[path = "apple_inputs_pdf.rs"]
mod pdf;
pub(crate) use pdf::extract_planning_pdf;
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 loads the protected brief selection")
)]
const MAXIMUM_BRIEF_CONFIG_BYTES: u64 = 64 * 1024;

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
#[serde(deny_unknown_fields)]
pub(crate) struct PdfArguments {
    path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CalendarProjectionArguments {
    external_id: String,
    title: String,
    start: String,
    end: String,
    is_all_day: bool,
    location: Option<String>,
    notes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReconcileCalendarArguments {
    coverage_start: String,
    coverage_end: String,
    projections: Vec<CalendarProjectionArguments>,
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
    ExtractPdf(PdfArguments),
    ReconcileCalendar(ReconcileCalendarArguments),
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
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
            Self::ExtractPdf(_) => AppleInputSource::Files,
            Self::ReconcileCalendar(_) => AppleInputSource::Calendar,
        }
    }

    fn timeout(&self) -> Duration {
        match self {
            Self::RequestPermission(_) => PERMISSION_TIMEOUT,
            Self::ExtractPdf(_) => Duration::from_secs(60),
            Self::ReconcileCalendar(_) => PERMISSION_TIMEOUT,
            _ => DEFAULT_TIMEOUT,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

impl AppleInputRequest {
    pub(crate) fn source_name(&self) -> &'static str {
        match self.source() {
            AppleInputSource::Calendar => "calendar",
            AppleInputSource::Reminders => "reminders",
            AppleInputSource::Notes => "notes",
            AppleInputSource::Files => "files",
        }
    }

    pub(crate) fn read_window(&self) -> Option<(&str, &str)> {
        match self {
            Self::ReadCalendar(arguments) => Some((&arguments.start, &arguments.end)),
            Self::ReadReminders(arguments) => Some((&arguments.start, &arguments.end)),
            Self::ReconcileCalendar(arguments) => {
                Some((&arguments.coverage_start, &arguments.coverage_end))
            }
            _ => None,
        }
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
impl AppleInputPermission {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::NotDetermined => "not_determined",
            Self::Denied => "denied",
            Self::Authorized => "authorized",
            Self::Restricted => "restricted",
            Self::Unavailable => "unavailable",
        }
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
impl AppleInputRecord {
    pub(crate) fn fields(&self) -> &BTreeMap<String, String> {
        &self.fields
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
impl AppleInputResponse {
    pub(crate) fn source_name(&self) -> &'static str {
        match self.source {
            AppleInputSource::Calendar => "calendar",
            AppleInputSource::Reminders => "reminders",
            AppleInputSource::Notes => "notes",
            AppleInputSource::Files => "files",
        }
    }

    pub(crate) const fn permission(&self) -> AppleInputPermission {
        self.permission
    }

    pub(crate) fn observed_at(&self) -> &str {
        &self.observed_at
    }

    pub(crate) fn records(&self) -> &[AppleInputRecord] {
        &self.records
    }

    pub(crate) const fn truncated(&self) -> bool {
        self.truncated
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 loads the protected brief selection")
)]
struct RawAppleBriefSelection {
    schema_version: u32,
    calendar_ids: Vec<String>,
    reminder_list_ids: Vec<String>,
    note_folder_ids: Vec<String>,
    file_paths: Vec<String>,
    maximum_records_per_source: usize,
}

/// Protected, native-only allowlists for one Daily Command Brief Apple read.
#[derive(Clone, Debug)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 loads the protected brief selection")
)]
pub(crate) struct AppleBriefSelection {
    calendar_ids: Vec<String>,
    reminder_list_ids: Vec<String>,
    note_folder_ids: Vec<String>,
    file_paths: Vec<String>,
    maximum_records_per_source: usize,
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "Phase 5 wires the local source backend")
)]
impl AppleBriefSelection {
    fn parse_protected(value: Value) -> Result<Self, &'static str> {
        let raw: RawAppleBriefSelection =
            serde_json::from_value(value).map_err(|_| "invalid_apple_brief_config")?;
        let valid_list = |values: &[String]| {
            !values.is_empty()
                && values.len() <= 32
                && values.iter().all(|value| {
                    !value.is_empty()
                        && value.trim() == value
                        && value.len() <= 1024
                        && !value.chars().any(char::is_control)
                })
                && values
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == values.len()
        };
        if raw.schema_version != 1
            || !(1..=100).contains(&raw.maximum_records_per_source)
            || !valid_list(&raw.calendar_ids)
            || !valid_list(&raw.reminder_list_ids)
            || !valid_list(&raw.note_folder_ids)
            || !valid_list(&raw.file_paths)
            || raw
                .file_paths
                .iter()
                .any(|path| !Path::new(path).is_absolute())
        {
            return Err("invalid_apple_brief_config");
        }
        Ok(Self {
            calendar_ids: raw.calendar_ids,
            reminder_list_ids: raw.reminder_list_ids,
            note_folder_ids: raw.note_folder_ids,
            file_paths: raw.file_paths,
            maximum_records_per_source: raw.maximum_records_per_source,
        })
    }

    pub(crate) fn load_protected(path: &Path) -> Result<Self, &'static str> {
        let bytes = ProtectedFile::open(path, MAXIMUM_BRIEF_CONFIG_BYTES)
            .and_then(|file| file.read_all())
            .map_err(|_| "invalid_apple_brief_config")?;
        let value = serde_json::from_slice(&bytes).map_err(|_| "invalid_apple_brief_config")?;
        Self::parse_protected(value)
    }

    pub(crate) fn configuration_identity(&self) -> String {
        let mut basis = Vec::new();
        for values in [
            &self.calendar_ids,
            &self.reminder_list_ids,
            &self.note_folder_ids,
            &self.file_paths,
        ] {
            for value in values {
                basis.extend_from_slice(value.as_bytes());
                basis.push(0);
            }
            basis.push(0xff);
        }
        basis.extend_from_slice(&self.maximum_records_per_source.to_be_bytes());
        format!("sha256:{}", hex::encode(Sha256::digest(&basis)))
    }

    #[cfg(test)]
    pub(crate) fn for_test(value: Value) -> Result<Self, &'static str> {
        Self::parse_protected(value)
    }

    pub(crate) fn brief_requests(
        &self,
        observed_at: &str,
    ) -> Result<Vec<AppleInputRequest>, &'static str> {
        use chrono::{Datelike, FixedOffset, TimeZone};

        let observed =
            chrono::DateTime::parse_from_rfc3339(observed_at).map_err(|_| "invalid_brief_time")?;
        let offset: FixedOffset = *observed.offset();
        let start = offset
            .with_ymd_and_hms(observed.year(), observed.month(), observed.day(), 0, 0, 0)
            .single()
            .ok_or("invalid_brief_time")?;
        let end = start
            .checked_add_signed(chrono::Duration::days(1))
            .ok_or("invalid_brief_time")?;
        Ok(vec![
            AppleInputRequest::ReadCalendar(CalendarArguments {
                calendar_ids: self.calendar_ids.clone(),
                start: start.to_rfc3339(),
                end: end.to_rfc3339(),
                maximum: self.maximum_records_per_source,
            }),
            AppleInputRequest::ReadReminders(ReminderArguments {
                list_ids: self.reminder_list_ids.clone(),
                start: start.to_rfc3339(),
                end: end.to_rfc3339(),
                maximum: self.maximum_records_per_source,
            }),
            AppleInputRequest::ReadNotes(NotesArguments {
                folder_ids: self.note_folder_ids.clone(),
                maximum: self.maximum_records_per_source,
            }),
            AppleInputRequest::ReadFiles(FilesArguments {
                paths: self.file_paths.clone(),
            }),
        ])
    }

    pub(crate) fn permits_record(&self, source: &str, fields: &BTreeMap<String, String>) -> bool {
        match source {
            "calendar" => fields
                .get("calendar_identifier")
                .is_some_and(|id| self.calendar_ids.contains(id)),
            "reminders" => fields
                .get("list_identifier")
                .is_some_and(|id| self.reminder_list_ids.contains(id)),
            "notes" => fields
                .get("folder_identifier")
                .is_some_and(|id| self.note_folder_ids.contains(id)),
            "files" => fields
                .get("path")
                .is_some_and(|path| self.file_paths.contains(path)),
            _ => false,
        }
    }
}

/// Verify the packaged signed-helper boundary and return its content identity.
pub(crate) fn bundled_helper_identity() -> Result<String, &'static str> {
    let path = bundled_helper_path().map_err(|_| "apple_helper_unavailable")?;
    let metadata = std::fs::metadata(&path).map_err(|_| "apple_helper_unavailable")?;
    if metadata.len() == 0 || metadata.len() > 64 * 1024 * 1024 {
        return Err("apple_helper_unavailable");
    }
    let mut file = std::fs::File::open(path).map_err(|_| "apple_helper_unavailable")?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "apple_helper_unavailable")?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

/// Return the validated sibling helper path for native long-lived modes.
pub(crate) fn verified_bundled_helper_path() -> Result<PathBuf, &'static str> {
    bundled_helper_path().map_err(|_| "apple_helper_unavailable")
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
    let exited = child.try_wait().map_err(|_| SupervisionError::Teardown)?;
    #[cfg(unix)]
    {
        use nix::errno::Errno;
        use nix::sys::signal::{killpg, Signal};
        use nix::unistd::Pid;
        match killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL) {
            Ok(()) | Err(Errno::ESRCH) => {}
            Err(_) => return Err(SupervisionError::Teardown),
        }
    }
    match exited {
        Some(_) => Ok(()),
        None => {
            #[cfg(not(unix))]
            child.kill().map_err(|_| SupervisionError::Teardown)?;
            child.wait().map_err(|_| SupervisionError::Teardown)?;
            Ok(())
        }
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
        || response.records.len() > 2_000
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
    if request_line.len() > MAXIMUM_REQUEST_BYTES {
        return Err(SupervisionError::RequestEncoding);
    }

    let mut command = Command::new(helper_path);
    command
        .env_clear()
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("PATH", "/usr/bin:/bin")
        .current_dir(working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|_| SupervisionError::Spawn)?;
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
            Ok(Some(status)) => break terminate(&mut child).map(|()| status),
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
    let task = tauri::async_runtime::spawn_blocking(move || read_apple_inputs_blocking(request));
    match task.await {
        Ok(response) => response,
        Err(_) => fail_soft(source, &SupervisionError::Task),
    }
}

/// Runs the signed helper under the same bounded supervisor on a caller-owned
/// blocking thread.
pub(crate) fn read_apple_inputs_blocking(request: AppleInputRequest) -> AppleInputResponse {
    let source = request.source();
    let timeout = request.timeout();
    let helper_path = match bundled_helper_path() {
        Ok(path) => path,
        Err(error) => return fail_soft(source, &error),
    };
    match supervise_helper(&helper_path, &request, timeout) {
        Ok(response) => response,
        Err(error) => fail_soft(source, &error),
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

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_signed_helper_boundary_preserves_fail_soft_source_identity() {
        let response = read_apple_inputs_blocking(permission_request());
        assert_eq!(response.source_name(), "calendar");
        assert_eq!(response.permission(), AppleInputPermission::Unavailable);
        assert!(response.error().is_some());
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
    fn kills_helper_process_group_at_deadline() {
        let (_directory, helper) = fixture_script(
            "IFS= read -r _request\ntrap '' HUP TERM\n(trap '' HUP TERM; sleep 5) &\nwait",
        );
        let started = std::time::Instant::now();

        let error = supervise_helper(&helper, &permission_request(), Duration::from_millis(500))
            .expect_err("hung helper must time out");

        assert_ne!(error.code(), "unimplemented");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn kills_pipe_holding_descendants_after_helper_exits() {
        let (_directory, helper) =
            fixture_script("IFS= read -r _request\n(trap '' HUP TERM; sleep 5) &\nexit 0");
        let started = std::time::Instant::now();

        let error = supervise_helper(&helper, &permission_request(), Duration::from_secs(2))
            .expect_err("an exited helper without a response must fail");

        assert_eq!(error, SupervisionError::Protocol);
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

    #[test]
    fn calendar_reconciliation_request_is_closed_and_source_scoped() {
        let decoded = serde_json::from_str::<AppleInputRequest>(
            r#"{"operation":"reconcile_calendar","arguments":{"coverage_start":"2026-07-01T00:00:00Z","coverage_end":"2026-08-01T00:00:00Z","projections":[{"external_id":"battle-rhythm:brief","title":"Navigation brief","start":"2026-07-29T08:00:00Z","end":"2026-07-29T08:30:00Z","is_all_day":false,"location":"Bridge","notes":null}]}}"#,
        )
        .expect("closed reconciliation request");
        assert_eq!(decoded.source_name(), "calendar");
        assert_eq!(
            decoded.read_window(),
            Some(("2026-07-01T00:00:00Z", "2026-08-01T00:00:00Z"))
        );
        let unknown = serde_json::from_str::<AppleInputRequest>(
            r#"{"operation":"reconcile_calendar","arguments":{"coverage_start":"2026-07-01T00:00:00Z","coverage_end":"2026-08-01T00:00:00Z","projections":[],"calendar_name":"Personal"}}"#,
        );
        assert!(unknown.is_err());
    }
}
