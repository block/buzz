//! Agent log files, install logs, and runtime receipt/PID persistence.
//!
//! Split out of `storage.rs` to keep that file under the size gate. These are
//! the filesystem helpers for the managed-agents `logs/` and `agent-pids/`
//! directories: log-path resolution, size- and run-based rotation, owner-only
//! install logs, runtime receipts, PID files, and log-tail error extraction.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use tauri::AppHandle;

use crate::managed_agents::{ManagedAgentRuntimeKey, ManagedAgentRuntimeReceipt};

use super::{atomic_write_json_restricted, managed_agents_base_dir};

fn managed_agents_logs_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = managed_agents_base_dir(app)?.join("logs");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create logs dir: {error}"))?;
    Ok(dir)
}

/// Install-log path for `runtime_id`, alongside the agent logs.
pub fn install_log_path(app: &AppHandle, runtime_id: &str) -> Result<PathBuf, String> {
    Ok(managed_agents_logs_dir(app)?.join(install_log_filename(runtime_id)?))
}

/// Filename for a runtime's install log, or an error for an id that must not
/// become one.
///
/// The id is validated rather than trusted: ids reach this from user-defined
/// custom harnesses as well as the catalog, and a `../` or a separator in one
/// would place the log outside the logs directory. Rejecting beats sanitizing —
/// a rejected id means no log, while a rewritten one could collide with another
/// runtime's.
fn install_log_filename(runtime_id: &str) -> Result<String, String> {
    if runtime_id.is_empty() || !runtime_id.chars().all(is_safe_id_char) {
        return Err(format!(
            "unsafe runtime id for a log filename: {runtime_id}"
        ));
    }
    Ok(format!("install-{runtime_id}.log"))
}

/// Characters allowed in a runtime id used as a filename. Excludes `/`, `\`,
/// `:` and `.`, so no id can traverse or escape the logs directory.
fn is_safe_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

pub fn managed_agent_log_path(app: &AppHandle, pubkey: &str) -> Result<PathBuf, String> {
    Ok(managed_agents_logs_dir(app)?.join(format!("{pubkey}.log")))
}

/// Pair-scoped log path for a managed runtime. The relay URL never appears in
/// the filename; the suffix is a hash of the canonical URL.
pub fn managed_agent_runtime_log_path(
    app: &AppHandle,
    key: &ManagedAgentRuntimeKey,
) -> Result<PathBuf, String> {
    Ok(managed_agents_logs_dir(app)?.join(format!("{}.log", key.runtime_id())))
}

/// Log path to surface for an agent whose runtime is not tracked in memory:
/// the most recently written of its pair-scoped logs, falling back to the
/// legacy single-runtime path when the agent has not run since harnesses
/// became per (agent, relay) pair.
pub fn latest_managed_agent_log_path(app: &AppHandle, pubkey: &str) -> Result<PathBuf, String> {
    match newest_agent_log_in_dir(&managed_agents_logs_dir(app)?, pubkey) {
        Some(path) => Ok(path),
        None => managed_agent_log_path(app, pubkey),
    }
}

/// Newest log in `dir` belonging to `pubkey` — either a pair-scoped
/// `{pubkey}__{relay_hash}.log` or the legacy `{pubkey}.log`. Ties break
/// toward the higher filename so the choice is deterministic.
fn newest_agent_log_in_dir(dir: &Path, pubkey: &str) -> Option<PathBuf> {
    let legacy_name = format!("{pubkey}.log");
    let pair_prefix = format!("{pubkey}__");
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let matches = name.to_str().is_some_and(|name| {
                name == legacy_name || (name.starts_with(&pair_prefix) && name.ends_with(".log"))
            });
            if !matches {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, name, entry.path()))
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
        .map(|(_, _, path)| path)
}

/// Maximum log file size before rotation (10 MB).
const MAX_LOG_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// If `path` exceeds [`MAX_LOG_FILE_SIZE`], rotate it to `<path>.1`.
fn maybe_rotate_log(path: &Path) {
    let size = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return,
    };
    if size <= MAX_LOG_FILE_SIZE {
        return;
    }
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    let _ = fs::rename(path, &rotated);
}

pub(crate) fn open_log_file(path: &Path) -> Result<File, String> {
    maybe_rotate_log(path);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to open log file {}: {error}", path.display()))
}

/// Start a new install-log session at `path`: keep the previous run as
/// `<path>.1` and return a freshly created, empty current file.
///
/// Rotating per *run* rather than by size is what bounds this file. A run
/// writes one record per executed attempt, each capped by the log-scale
/// capture, so one run's file is bounded by steps × attempts × cap and the
/// history on disk is bounded at two runs. Size-triggered rotation could not
/// promise either: it never replaced an existing `.1`, and on Windows —
/// where rename does not replace its destination — it stopped working
/// altogether once `.1` existed, leaving the current file to grow.
///
/// The old `.1` is therefore *removed* before the rename rather than renamed
/// over. Every step is best-effort: a rotation that fails must not cost the
/// user the install, so the session continues with a truncated current file.
pub(crate) fn start_install_log_session(path: &Path) -> Result<File, String> {
    if path.exists() {
        let mut previous = path.as_os_str().to_owned();
        previous.push(".1");
        let previous = PathBuf::from(previous);
        let _ = fs::remove_file(&previous);
        let _ = fs::rename(path, &previous);
    }
    open_install_log(path, /* truncate */ true)
}

/// Open an install log for appending one more record to the current session.
pub(crate) fn open_install_log_file(path: &Path) -> Result<File, String> {
    open_install_log(path, /* truncate */ false)
}

/// Open an install log owner-only.
///
/// The mode is set *in the create* rather than chmod'd afterwards, so the file
/// is never briefly group/world-readable. Install output can carry registry
/// tokens and proxy credentials echoed by a failing installer, so the window
/// matters even though it is short. An existing file's mode is left as-is —
/// `OpenOptions::mode` only applies on creation, and silently re-tightening a
/// file the user relaxed is not this function's call to make.
fn open_install_log(path: &Path, truncate: bool) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true);
    if truncate {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| format!("failed to open log file {}: {error}", path.display()))
}

pub(crate) fn append_log_marker(path: &Path, message: &str) -> Result<(), String> {
    let mut file = open_log_file(path)?;
    writeln!(file, "{message}").map_err(|error| format!("failed to write log marker: {error}"))
}

fn agent_pids_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = managed_agents_base_dir(app)?.join("agent-pids");
    fs::create_dir_all(&dir)
        .map_err(|error| format!("failed to create agent-pids dir: {error}"))?;
    Ok(dir)
}

/// Persist a pair-scoped runtime receipt atomically. Callers must register the
/// process in memory in the same runtime transition; on write failure they must
/// terminate the child before releasing that transition.
pub fn write_agent_runtime_receipt(
    app: &AppHandle,
    receipt: &ManagedAgentRuntimeReceipt,
) -> Result<(), String> {
    let path = agent_pids_dir(app)?.join(format!("{}.json", receipt.key.runtime_id()));
    let payload = serde_json::to_vec(receipt)
        .map_err(|error| format!("failed to serialize runtime receipt: {error}"))?;
    atomic_write_json_restricted(&path, &payload)
}

pub fn remove_agent_runtime_receipt(app: &AppHandle, key: &ManagedAgentRuntimeKey) {
    if let Ok(dir) = agent_pids_dir(app) {
        let _ = fs::remove_file(dir.join(format!("{}.json", key.runtime_id())));
    }
}

pub fn remove_agent_runtime_receipt_path(path: &Path) {
    let _ = fs::remove_file(path);
}

pub fn read_all_agent_runtime_receipts(
    app: &AppHandle,
) -> Vec<(PathBuf, ManagedAgentRuntimeReceipt)> {
    let Ok(dir) = agent_pids_dir(app) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| {
            let path = entry.path();
            let bytes = fs::read(&path).ok()?;
            serde_json::from_slice(&bytes)
                .ok()
                .map(|receipt| (path, receipt))
        })
        .collect()
}

/// Remove the PID file for an agent (e.g. on normal stop).
pub fn remove_agent_pid_file(app: &AppHandle, pubkey: &str) {
    if let Ok(dir) = agent_pids_dir(app) {
        let _ = fs::remove_file(dir.join(format!("{pubkey}.pid")));
    }
}

/// Read all PID files from `agent-pids/`, returning `(pubkey, pid)` pairs.
pub fn read_all_agent_pid_files(app: &AppHandle) -> Vec<(String, u32)> {
    let Ok(dir) = agent_pids_dir(app) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let pubkey = name.strip_suffix(".pid")?;
            let pid: u32 = fs::read_to_string(entry.path()).ok()?.trim().parse().ok()?;
            Some((pubkey.to_string(), pid))
        })
        .collect()
}

pub fn read_log_tail(path: &Path, max_lines: usize) -> Result<String, String> {
    if !path.exists() {
        return Ok(String::new());
    }

    let mut file = File::open(path)
        .map_err(|error| format!("failed to read log file {}: {error}", path.display()))?;

    let file_len = file
        .seek(SeekFrom::End(0))
        .map_err(|error| format!("failed to seek log file: {error}"))?;

    if file_len == 0 {
        return Ok(String::new());
    }

    // Read backward in chunks to find enough newlines.
    const CHUNK_SIZE: u64 = 8 * 1024;
    let mut buf = Vec::new();
    let mut remaining = file_len;
    let mut newline_count: usize = 0;
    // We need max_lines + 1 newlines to delimit max_lines lines (the trailing
    // newline of the last line counts as one).
    let target_newlines = max_lines + 1;

    while remaining > 0 && newline_count < target_newlines {
        let chunk = remaining.min(CHUNK_SIZE);
        remaining -= chunk;
        file.seek(SeekFrom::Start(remaining))
            .map_err(|error| format!("failed to seek log file: {error}"))?;

        let mut tmp = vec![0u8; chunk as usize];
        file.read_exact(&mut tmp)
            .map_err(|error| format!("failed to read log chunk: {error}"))?;

        // Prepend this chunk so buf always has the tail of the file.
        tmp.append(&mut buf);
        buf = tmp;

        newline_count = bytecount_newlines(&buf);
    }

    // Strip ANSI escapes here (not in the harness) so the desktop log view
    // renders cleanly while terminals and other tools still get the colors
    // buzz-acp emits.
    let cleaned = strip_ansi_escapes::strip_str(String::from_utf8_lossy(&buf));
    let lines: Vec<&str> = cleaned.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    Ok(lines[start..].join("\n"))
}

fn bytecount_newlines(buf: &[u8]) -> usize {
    buf.iter().filter(|&&b| b == b'\n').count()
}

/// A meaningful error recovered from an exited agent's log tail.
pub struct AgentLogError {
    /// The full log line, wrapped as `Agent reported error…` for display.
    pub message: String,
    /// JSON-RPC error code parsed from the line's `(code N)` marker, or a
    /// synthetic code for known bare prefixes. `None` for legacy-format
    /// lines that carry no code (or when the code fails to parse as i64).
    pub code: Option<i64>,
}

pub fn meaningful_agent_error_from_log(path: &Path) -> Option<AgentLogError> {
    let tail = read_log_tail(path, 200).ok()?;
    tail.lines().rev().map(str::trim).find_map(|line| {
        // New format: "Agent reported error (code -32002): ..."
        if let Some(rest) = line.strip_prefix("Agent reported error (code ") {
            if let Some(paren_end) = rest.find("): ") {
                let code = rest[..paren_end].parse::<i64>().ok();
                return Some(AgentLogError {
                    message: line.to_string(),
                    code,
                });
            }
        }
        // Legacy format (older buzz-acp builds): "Agent reported error: ..."
        if line.starts_with("Agent reported error:") {
            return Some(AgentLogError {
                message: line.to_string(),
                code: None,
            });
        }
        // Bare prefixes emitted by older agent binaries whose Display still leaks
        // unwrapped errors. Promote these so they surface instead of the generic
        // "harness exited with status N" fallback.
        if line.starts_with("llm auth:") {
            return Some(AgentLogError {
                message: format!("Agent reported error: {line}"),
                code: Some(-32001),
            });
        }
        if line.starts_with("llm model not found:") {
            return Some(AgentLogError {
                message: format!("Agent reported error: {line}"),
                code: Some(-32002),
            });
        }
        None
    })
}

#[cfg(test)]
#[path = "logs_tests.rs"]
mod tests;
