use std::{
    fs::{self, File, OpenOptions},
    io::{Read as _, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};

use crate::app_state::KEYRING_SERVICE;
use crate::managed_agents::ManagedAgentRecord;
use crate::secret_store::{KeyringProbe, SecretStore};

/// Keyring key name for an agent's nsec, namespaced from the human identity
/// key (`"identity"`) which shares the service.
fn agent_keyring_name(pubkey: &str) -> String {
    format!("agent:{pubkey}")
}

/// The agent secret store. `None` when the build has no keyring backend, in
/// which case agent keys stay inline in the `0o600` JSON file.
fn agent_secret_store() -> Option<SecretStore> {
    if cfg!(feature = "system-keyring") {
        Some(SecretStore::keyring(KEYRING_SERVICE))
    } else {
        None
    }
}

pub fn managed_agents_base_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("failed to resolve app data dir: {error}"))?
        .join("agents");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create agents dir: {error}"))?;
    Ok(dir)
}

fn managed_agents_store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("managed-agents.json"))
}

fn managed_agents_logs_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = managed_agents_base_dir(app)?.join("logs");
    fs::create_dir_all(&dir).map_err(|error| format!("failed to create logs dir: {error}"))?;
    Ok(dir)
}

pub fn managed_agent_log_path(app: &AppHandle, pubkey: &str) -> Result<PathBuf, String> {
    Ok(managed_agents_logs_dir(app)?.join(format!("{pubkey}.log")))
}

pub fn load_managed_agents(app: &AppHandle) -> Result<Vec<ManagedAgentRecord>, String> {
    let path = managed_agents_store_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read agent store: {error}"))?;
    let mut records: Vec<ManagedAgentRecord> = serde_json::from_str(&content)
        .map_err(|error| format!("failed to parse agent store: {error}"))?;

    hydrate_keys(&mut records);
    Ok(records)
}

/// Fill in each record's in-memory `private_key_nsec` from the keyring when it
/// was not serialized inline. A record loaded with a non-empty key came from
/// the JSON file-fallback (keyring was unreachable when it was written) — leave
/// it as-is. A record with an empty key has its secret in the keyring.
fn hydrate_keys(records: &mut [ManagedAgentRecord]) {
    let Some(store) = agent_secret_store() else {
        return;
    };
    for record in records.iter_mut() {
        if !record.private_key_nsec.is_empty() {
            continue;
        }
        match store.load(&agent_keyring_name(&record.pubkey)) {
            Ok(Some(nsec)) => record.private_key_nsec = nsec,
            Ok(None) => {
                eprintln!(
                    "buzz-desktop: agent {} has no key in JSON or keyring",
                    record.pubkey
                );
            }
            Err(e) => {
                eprintln!(
                    "buzz-desktop: failed to read agent {} key from keyring: {e}",
                    record.pubkey
                );
            }
        }
    }
}

pub fn save_managed_agents(app: &AppHandle, records: &[ManagedAgentRecord]) -> Result<(), String> {
    let mut sorted = records.to_vec();
    sorted.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.pubkey.cmp(&right.pubkey))
    });

    // Persist each key to the keyring; on success blank the inline copy so it
    // is skipped from JSON (`skip_serializing_if = "String::is_empty"`). If the
    // keyring is unreachable, the key stays inline and the JSON gets 0o600.
    let mut any_inline_key = false;
    persist_agent_keys(&mut sorted, &mut any_inline_key);

    let path = managed_agents_store_path(app)?;
    let payload = serde_json::to_vec_pretty(&sorted)
        .map_err(|error| format!("failed to serialize agent store: {error}"))?;

    atomic_write_json(&path, &payload)?;

    // The JSON only carries plaintext keys in the keyringless fallback; lock it
    // down to owner-only in that case.
    if any_inline_key {
        restrict_json_permissions(&path);
    }
    Ok(())
}

/// Write each record's in-memory key to the keyring and blank the inline copy
/// on success. Sets `any_inline_key` if any key had to stay in the JSON because
/// the keyring was unreachable. Mutates `records` (a save-local clone) — the
/// caller's in-memory records keep their keys.
fn persist_agent_keys(records: &mut [ManagedAgentRecord], any_inline_key: &mut bool) {
    let Some(store) = agent_secret_store() else {
        // No keyring backend: keys stay inline.
        *any_inline_key = records.iter().any(|r| !r.private_key_nsec.is_empty());
        return;
    };
    for record in records.iter_mut() {
        if record.private_key_nsec.is_empty() {
            continue;
        }
        let name = agent_keyring_name(&record.pubkey);
        match store.probe(&name) {
            KeyringProbe::Unreachable => {
                // Keep the key inline (file fallback); do not migrate.
                *any_inline_key = true;
            }
            KeyringProbe::Present | KeyringProbe::ReachableButEmpty => {
                match write_and_verify(&store, &name, &record.private_key_nsec) {
                    Ok(()) => record.private_key_nsec.clear(),
                    Err(e) => {
                        eprintln!(
                            "buzz-desktop: keyring write for agent {} failed ({e}), keeping inline",
                            record.pubkey
                        );
                        *any_inline_key = true;
                    }
                }
            }
        }
    }
}

/// Write `value` to the keyring and read it back to confirm before the caller
/// strips the inline copy.
fn write_and_verify(store: &SecretStore, name: &str, value: &str) -> Result<(), String> {
    store.store(name, value)?;
    match store.load(name)? {
        Some(stored) if stored == value => Ok(()),
        _ => Err("keyring read-back verify failed".to_string()),
    }
}

#[cfg(unix)]
fn restrict_json_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Err(e) = std::fs::set_permissions(&resolved, std::fs::Permissions::from_mode(0o600)) {
        eprintln!(
            "buzz-desktop: failed to restrict {} permissions: {e}",
            resolved.display()
        );
    }
}

#[cfg(not(unix))]
fn restrict_json_permissions(_path: &Path) {}

/// Remove an agent's key from the keyring (best-effort). Called when an agent
/// is deleted so its secret does not linger in the OS store.
pub fn delete_agent_key(pubkey: &str) {
    if let Some(store) = agent_secret_store() {
        if let Err(e) = store.delete(&agent_keyring_name(pubkey)) {
            eprintln!("buzz-desktop: failed to delete agent {pubkey} key from keyring: {e}");
        }
    }
}

/// Atomic, symlink-preserving JSON write.
/// Resolves symlinks so the tmp+rename happens at the real target path,
/// preserving any symlink at `path`.
pub(crate) fn atomic_write_json(path: &Path, payload: &[u8]) -> Result<(), String> {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let tmp = resolved.with_extension("json.tmp");
    std::fs::write(&tmp, payload).map_err(|e| format!("failed to write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &resolved)
        .map_err(|e| format!("failed to rename {}: {e}", resolved.display()))
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

/// Write a PID file for a spawned agent. The PID equals the PGID since we
/// spawn with `process_group(0)`.
pub fn write_agent_pid_file(app: &AppHandle, pubkey: &str, pid: u32) -> Result<(), String> {
    let path = agent_pids_dir(app)?.join(format!("{pubkey}.pid"));
    fs::write(&path, pid.to_string())
        .map_err(|error| format!("failed to write PID file {}: {error}", path.display()))
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

pub fn meaningful_agent_error_from_log(path: &Path) -> Option<String> {
    let tail = read_log_tail(path, 200).ok()?;
    tail.lines().rev().map(str::trim).find_map(|line| {
        if line.starts_with("Agent reported error:") {
            return Some(line.to_string());
        }
        if line.starts_with("llm auth:") {
            return Some(format!("Agent reported error: {line}"));
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::NamedTempFile;

    fn write_log(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("temp log");
        file.write_all(content.as_bytes()).expect("write log");
        file
    }

    #[test]
    fn meaningful_agent_error_from_log_promotes_wrapped_llm_auth() {
        let file = write_log("noise\nAgent reported error: llm auth: denied\n");
        assert_eq!(
            super::meaningful_agent_error_from_log(file.path()).as_deref(),
            Some("Agent reported error: llm auth: denied")
        );
    }

    #[test]
    fn meaningful_agent_error_from_log_promotes_unwrapped_llm_auth() {
        let file = write_log("noise\nllm auth: denied\n");
        assert_eq!(
            super::meaningful_agent_error_from_log(file.path()).as_deref(),
            Some("Agent reported error: llm auth: denied")
        );
    }

    #[test]
    fn meaningful_agent_error_from_log_does_not_promote_midline_auth_text() {
        let file = write_log("noise before llm auth: denied\n");
        assert!(super::meaningful_agent_error_from_log(file.path()).is_none());
    }

    #[test]
    fn strips_ansi_from_typical_tracing_line() {
        let input = "\x1b[2m2026-05-27T15:16:32\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2mbuzz_acp\x1b[0m\x1b[2m:\x1b[0m starting";
        assert_eq!(
            strip_ansi_escapes::strip_str(input),
            "2026-05-27T15:16:32  INFO buzz_acp: starting"
        );
    }
}
