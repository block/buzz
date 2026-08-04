//! Owner-only runtime receipt, runner specification, and terminal receipt files.

use crate::protocol::{
    JobId, JobStartRequest, LegacyRuntimeReceipt, RunnerReceiptHealth, RuntimeReceipt,
};
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(windows)]
use std::process::Command;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const RUNNER_RECEIPT_SCHEMA_VERSION: u8 = 1;
pub const JOB_SPEC_FILE: &str = "spec.json";
pub const RUNNER_RECEIPT_FILE: &str = "runner-receipt.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobSpec {
    pub runtime_id: String,
    pub job_id: JobId,
    pub attempt: u32,
    pub executable: PathBuf,
    pub request: JobStartRequest,
    pub argv_sha256: String,
    pub created_at: DateTime<Utc>,
}
impl JobSpec {
    pub fn validate(&self) -> Result<(), ArtifactError> {
        self.request
            .validate()
            .map_err(|error| ArtifactError::Invalid(error.to_string()))?;
        if self.attempt == 0 || !self.executable.is_absolute() {
            return Err(ArtifactError::Invalid(
                "invalid runner specification".into(),
            ));
        }
        if argv_sha256(&self.request.argv)? != self.argv_sha256 {
            return Err(ArtifactError::Invalid("argv hash mismatch".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerReceiptState {
    Ready,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerReceipt {
    pub schema_version: u8,
    pub job_id: JobId,
    pub attempt: u32,
    pub state: RunnerReceiptState,
    pub runner_pid: u32,
    pub runner_start_marker: String,
    pub process_group: String,
    pub argv_sha256: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub error_code: Option<String>,
}
impl RunnerReceipt {
    pub fn validate(
        &self,
        expected_job: JobId,
        expected_attempt: u32,
    ) -> Result<(), ArtifactError> {
        let terminal = self.state != RunnerReceiptState::Ready;
        if self.schema_version != RUNNER_RECEIPT_SCHEMA_VERSION
            || self.job_id != expected_job
            || self.attempt != expected_attempt
            || self.attempt == 0
            || self.runner_pid == 0
            || self.runner_start_marker.is_empty()
            || self.process_group.is_empty()
            || !is_lower_hex(&self.argv_sha256, 64)
            || terminal != self.finished_at.is_some()
        {
            return Err(ArtifactError::Invalid("invalid runner receipt".into()));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("artifact IO failed: {0}")]
    Io(#[from] io::Error),
    #[error("artifact JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid artifact: {0}")]
    Invalid(String),
    #[error("process {0} is not running or has no start marker")]
    ProcessUnavailable(u32),
}

pub fn job_attempt_dir(
    runtime_dir: &Path,
    job_id: JobId,
    attempt: u32,
) -> Result<PathBuf, ArtifactError> {
    if attempt == 0 {
        return Err(ArtifactError::Invalid("attempt must be positive".into()));
    }
    if !runtime_dir.is_absolute()
        || runtime_dir.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(ArtifactError::Invalid(
            "runtime directory is not a normalized absolute path".into(),
        ));
    }
    Ok(runtime_dir
        .join("jobs")
        .join(job_id.hyphenated().to_string())
        .join(attempt.to_string()))
}
/// Canonicalizes operator roots once and rejects empty, relative, or non-directory roots.
pub fn canonicalize_workspace_roots(
    roots: impl IntoIterator<Item = PathBuf>,
) -> Result<Vec<PathBuf>, ArtifactError> {
    let mut output = Vec::new();
    for root in roots {
        if !root.is_absolute() {
            return Err(ArtifactError::Invalid(
                "workspace root is not absolute".into(),
            ));
        }
        let canonical = fs::canonicalize(root)?;
        if !canonical.is_dir() {
            return Err(ArtifactError::Invalid(
                "workspace root is not a directory".into(),
            ));
        }
        if !output.contains(&canonical) {
            output.push(canonical)
        }
    }
    if output.is_empty() {
        return Err(ArtifactError::Invalid("no approved workspace roots".into()));
    }
    Ok(output)
}

/// Canonicalizes a requested cwd and proves containment beneath an approved canonical root.
pub fn canonicalize_workspace(
    cwd: &Path,
    approved_roots: &[PathBuf],
) -> Result<PathBuf, ArtifactError> {
    if !cwd.is_absolute() || approved_roots.is_empty() {
        return Err(ArtifactError::Invalid("workspace not allowed".into()));
    }
    let canonical = fs::canonicalize(cwd)?;
    if !canonical.is_dir()
        || !approved_roots
            .iter()
            .any(|root| canonical.starts_with(root))
    {
        return Err(ArtifactError::Invalid("workspace not allowed".into()));
    }
    Ok(canonical)
}

/// Canonicalizes the operator-owned driver and proves it is an absolute regular file.
pub fn canonicalize_executable(path: &Path) -> Result<PathBuf, ArtifactError> {
    if !path.is_absolute() {
        return Err(ArtifactError::Invalid("driver path is not absolute".into()));
    }
    let canonical = fs::canonicalize(path)?;
    if !canonical.is_file() {
        return Err(ArtifactError::Invalid(
            "driver is not a regular file".into(),
        ));
    }
    Ok(canonical)
}
pub fn write_legacy_runtime_receipt(
    path: &Path,
    receipt: &LegacyRuntimeReceipt,
) -> Result<(), ArtifactError> {
    receipt
        .validate()
        .map_err(|error| ArtifactError::Invalid(error.to_string()))?;
    write_owner_only_json(path, receipt)
}

pub fn read_legacy_runtime_receipt(path: &Path) -> Result<LegacyRuntimeReceipt, ArtifactError> {
    let receipt: LegacyRuntimeReceipt = read_owner_only_json(path)?;
    receipt
        .validate()
        .map_err(|error| ArtifactError::Invalid(error.to_string()))?;
    Ok(receipt)
}

pub fn write_runtime_receipt(path: &Path, receipt: &RuntimeReceipt) -> Result<(), ArtifactError> {
    receipt
        .validate()
        .map_err(|error| ArtifactError::Invalid(error.to_string()))?;
    write_owner_only_json(path, receipt)
}
pub fn read_runtime_receipt(path: &Path) -> Result<RuntimeReceipt, ArtifactError> {
    let receipt: RuntimeReceipt = read_owner_only_json(path)?;
    receipt
        .validate()
        .map_err(|error| ArtifactError::Invalid(error.to_string()))?;
    Ok(receipt)
}
pub fn write_job_spec(runtime_dir: &Path, spec: &JobSpec) -> Result<PathBuf, ArtifactError> {
    spec.validate()?;
    let path = job_attempt_dir(runtime_dir, spec.job_id, spec.attempt)?.join(JOB_SPEC_FILE);
    write_owner_only_json(&path, spec)?;
    Ok(fs::canonicalize(path)?)
}
pub fn read_job_spec(path: &Path) -> Result<JobSpec, ArtifactError> {
    let spec: JobSpec = read_owner_only_json(path)?;
    spec.validate()?;
    Ok(spec)
}
pub fn write_runner_receipt(
    runtime_dir: &Path,
    receipt: &RunnerReceipt,
) -> Result<PathBuf, ArtifactError> {
    receipt.validate(receipt.job_id, receipt.attempt)?;
    let path =
        job_attempt_dir(runtime_dir, receipt.job_id, receipt.attempt)?.join(RUNNER_RECEIPT_FILE);
    write_owner_only_json(&path, receipt)?;
    Ok(path)
}
pub fn read_runner_receipt(
    runtime_dir: &Path,
    job_id: JobId,
    attempt: u32,
) -> Result<RunnerReceipt, ArtifactError> {
    let path = job_attempt_dir(runtime_dir, job_id, attempt)?.join(RUNNER_RECEIPT_FILE);
    let receipt: RunnerReceipt = read_owner_only_json(&path)?;
    receipt.validate(job_id, attempt)?;
    Ok(receipt)
}
/// Inspects one receipt without disclosing its path or process identity.
pub fn runner_receipt_health(
    runtime_dir: &Path,
    job_id: JobId,
    attempt: u32,
) -> RunnerReceiptHealth {
    match read_runner_receipt(runtime_dir, job_id, attempt) {
        Ok(receipt) if receipt.state == RunnerReceiptState::Ready => {
            if process_matches_marker(receipt.runner_pid, &receipt.runner_start_marker) {
                RunnerReceiptHealth::Ready
            } else {
                RunnerReceiptHealth::IdentityMismatch
            }
        }
        Ok(_) => RunnerReceiptHealth::Terminal,
        Err(ArtifactError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
            RunnerReceiptHealth::Missing
        }
        Err(_) => RunnerReceiptHealth::Invalid,
    }
}
pub fn argv_sha256(argv: &[String]) -> Result<String, ArtifactError> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(argv)?)))
}
// Each platform arm ends in an explicit `return` so that adding an arm can never
// silently change which one is the function tail after `cfg` stripping.
#[allow(clippy::needless_return)]
pub fn process_start_marker(pid: u32) -> Result<String, ArtifactError> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat"))
            .map_err(|_| ArtifactError::ProcessUnavailable(pid))?;
        let start_ticks =
            linux_proc_start_ticks(pid, &stat).ok_or(ArtifactError::ProcessUnavailable(pid))?;
        let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        let boot_id = boot_id.trim();
        if boot_id.is_empty() {
            return Err(ArtifactError::Invalid(
                "Linux boot identity is empty".into(),
            ));
        }
        return Ok(format!("{pid}:linux:{boot_id}:{start_ticks:016x}"));
    }

    #[cfg(target_os = "macos")]
    {
        let native_pid = i32::try_from(pid).map_err(|_| ArtifactError::ProcessUnavailable(pid))?;
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let expected_size = std::mem::size_of::<libc::proc_bsdinfo>();
        // SAFETY: `info` points to writable storage sized exactly for the
        // requested PROC_PIDTBSDINFO structure. The value is read only when
        // proc_pidinfo reports that it initialized the entire structure.
        #[allow(unsafe_code)]
        let read_size = unsafe {
            libc::proc_pidinfo(
                native_pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                expected_size as libc::c_int,
            )
        };
        if read_size != expected_size as libc::c_int {
            return Err(ArtifactError::ProcessUnavailable(pid));
        }
        // SAFETY: the full-structure size check above proves initialization.
        #[allow(unsafe_code)]
        let info = unsafe { info.assume_init() };
        if info.pbi_pid != pid || info.pbi_start_tvsec == 0 || info.pbi_start_tvusec >= 1_000_000 {
            return Err(ArtifactError::ProcessUnavailable(pid));
        }
        return Ok(format!(
            "{pid}:macos:{:016x}:{:05x}",
            info.pbi_start_tvsec, info.pbi_start_tvusec
        ));
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, FILETIME},
            System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };

        // SAFETY: OpenProcess returns either a null handle or an owned process
        // handle. GetProcessTimes receives valid writable FILETIME pointers, and
        // every non-null handle is closed exactly once before this block exits.
        #[allow(unsafe_code)]
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return Err(ArtifactError::ProcessUnavailable(pid));
            }
            let mut creation = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut exit = creation;
            let mut kernel = creation;
            let mut user = creation;
            let read_ok =
                GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user);
            let _ = CloseHandle(process);
            if read_ok == 0 {
                return Err(ArtifactError::ProcessUnavailable(pid));
            }
            let ticks =
                (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
            if ticks == 0 {
                return Err(ArtifactError::ProcessUnavailable(pid));
            }
            return Ok(format!("{pid}:windows:{ticks:016x}"));
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = pid;
        Err(ArtifactError::Invalid(
            "process start markers are unsupported on this platform".into(),
        ))
    }
}
#[cfg(target_os = "linux")]
fn linux_proc_start_ticks(pid: u32, stat: &str) -> Option<u64> {
    let comm_start = stat.find('(')?;
    if stat[..comm_start].trim().parse::<u32>().ok()? != pid {
        return None;
    }
    let (_, fields) = stat.rsplit_once(')')?;
    fields.split_ascii_whitespace().nth(19)?.parse().ok()
}

pub fn current_process_start_marker() -> Result<String, ArtifactError> {
    process_start_marker(std::process::id())
}
/// Verifies a live PID still has the exact recorded anti-reuse marker.
pub fn process_matches_marker(pid: u32, expected: &str) -> bool {
    process_start_marker(pid).is_ok_and(|actual| actual == expected)
}

fn read_owner_only_json<T: DeserializeOwned>(path: &Path) -> Result<T, ArtifactError> {
    ensure_regular_owner_file(path)?;
    Ok(serde_json::from_reader(File::open(path)?)?)
}
fn write_owner_only_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ArtifactError> {
    let parent = path
        .parent()
        .ok_or_else(|| ArtifactError::Invalid("artifact path has no parent".into()))?;
    ensure_owner_dir(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| ArtifactError::Invalid("artifact path has no filename".into()))?
        .to_string_lossy();
    let temporary = parent.join(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec(value)?;
    let result = (|| -> Result<(), ArtifactError> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        #[cfg(windows)]
        harden_windows_acl(&temporary, false)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        atomic_replace(&temporary, path)?;
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
/// Creates or upgrades a runtime state directory to current-user-only access.
pub fn ensure_owner_only_runtime_dir(path: &Path) -> Result<(), ArtifactError> {
    ensure_owner_dir(path)
}
pub(crate) fn ensure_owner_dir(path: &Path) -> Result<(), ArtifactError> {
    create_missing_owner_dirs(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(ArtifactError::Invalid(
            "artifact directory is not a real directory".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(windows)]
    {
        harden_windows_acl(path, true)?;
    }
    Ok(())
}
fn create_missing_owner_dirs(path: &Path) -> Result<(), ArtifactError> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(ArtifactError::Invalid(
                "artifact ancestor is not a real directory".into(),
            ));
        }
        return Ok(());
    }
    if let Some(parent) = path.parent().filter(|parent| *parent != path) {
        create_missing_owner_dirs(parent)?
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)?;
        #[cfg(windows)]
        harden_windows_acl(path, true)?;
    }
    Ok(())
}
fn ensure_regular_owner_file(path: &Path) -> Result<(), ArtifactError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(ArtifactError::Invalid(
            "artifact path is not a regular file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(ArtifactError::Invalid(
                "artifact file is not owner-only".into(),
            ));
        }
    }
    #[cfg(windows)]
    verify_windows_acl(path)?;
    Ok(())
}
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), ArtifactError> {
    File::open(path)?.sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), ArtifactError> {
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), ArtifactError> {
    fs::rename(source, destination)?;
    Ok(())
}
#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<(), ArtifactError> {
    if !destination.exists() {
        fs::rename(source, destination)?;
        return Ok(());
    }
    let script = "[IO.File]::Replace($env:BUZZ_SOURCE_PATH,$env:BUZZ_DEST_PATH,$null)";
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("BUZZ_SOURCE_PATH", source)
        .env("BUZZ_DEST_PATH", destination)
        .status()?;
    if !status.success() {
        return Err(ArtifactError::Invalid(
            "atomic Windows file replacement failed".into(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn harden_windows_acl(path: &Path, directory: bool) -> Result<(), ArtifactError> {
    let script = if directory {
        r#"$p=$env:BUZZ_ACL_PATH;$id=[Security.Principal.WindowsIdentity]::GetCurrent().User;$a=New-Object Security.AccessControl.DirectorySecurity;$a.SetOwner($id);$r=New-Object Security.AccessControl.FileSystemAccessRule($id,'FullControl','ContainerInherit,ObjectInherit','None','Allow');$a.AddAccessRule($r);[IO.Directory]::SetAccessControl($p,$a)"#
    } else {
        r#"$p=$env:BUZZ_ACL_PATH;$id=[Security.Principal.WindowsIdentity]::GetCurrent().User;$a=New-Object Security.AccessControl.FileSecurity;$a.SetOwner($id);$r=New-Object Security.AccessControl.FileSystemAccessRule($id,'FullControl','None','None','Allow');$a.AddAccessRule($r);[IO.File]::SetAccessControl($p,$a)"#
    };
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("BUZZ_ACL_PATH", path)
        .status()?;
    if !status.success() {
        return Err(ArtifactError::Invalid(
            "failed to apply owner-only Windows ACL".into(),
        ));
    }
    verify_windows_acl(path)
}
#[cfg(windows)]
fn verify_windows_acl(path: &Path) -> Result<(), ArtifactError> {
    let script = r#"$p=$env:BUZZ_ACL_PATH;$id=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;$a=Get-Acl -LiteralPath $p;$bad=@($a.Access|?{($_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value-ne$id)-or($_.AccessControlType-ne'Allow')});if($bad.Count-ne0-or$a.Access.Count-ne1){exit 1}"#;
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("BUZZ_ACL_PATH", path)
        .status()?;
    if !status.success() {
        return Err(ArtifactError::Invalid(
            "artifact ACL is not current-user-only".into(),
        ));
    }
    Ok(())
}
fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod process_marker_tests {
    use super::{process_start_marker, ArtifactError};

    #[test]
    fn live_process_marker_is_stable() {
        let pid = std::process::id();
        let first = process_start_marker(pid).expect("current process marker");
        let second = process_start_marker(pid).expect("current process marker");

        assert_eq!(first, second);
        assert!(first.starts_with(&format!("{pid}:")));
    }

    #[test]
    fn nonexistent_process_is_unavailable() {
        assert!(matches!(
            process_start_marker(u32::MAX),
            Err(ArtifactError::ProcessUnavailable(pid)) if pid == u32::MAX
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_stat_parser_handles_spaces_and_closing_parentheses_in_comm() {
        let stat = "42 (worker name)) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 987654 20";
        assert_eq!(super::linux_proc_start_ticks(42, stat), Some(987_654));
        assert_eq!(super::linux_proc_start_ticks(41, stat), None);
    }
}
