//! Local sink for the temporary renderer profiling harness (never merges).
//!
//! The renderer ring-buffers timing records and flushes JSONL lines here every
//! ~30s and on `pagehide`. Records are appended to
//! `<app-data>/profiling/<file_stem>.jsonl`. Nothing is transmitted; this file
//! is the only output. When a file crosses `MAX_FILE_BYTES` it is rotated aside
//! with a millisecond suffix so a long session cannot grow unbounded -- the
//! analyzer globs `<file_stem>*.jsonl` and concatenates.
use std::fs::{self, OpenOptions};
use std::io::Write;

use tauri::{AppHandle, Manager};

/// Rotate the active file aside once it reaches ~50 MiB.
const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// Append profiling records (one JSONL string per element) to the session file.
///
/// `file_stem` is caller-supplied but strictly validated to
/// `session-<digits>` so the argument can never escape the profiling dir.
#[tauri::command]
pub async fn append_profiling_log(
    file_stem: String,
    lines: Vec<String>,
    app: AppHandle,
) -> Result<(), String> {
    if lines.is_empty() {
        return Ok(());
    }
    if !is_valid_stem(&file_stem) {
        return Err(format!("invalid profiling file stem: {file_stem}"));
    }

    tokio::task::spawn_blocking(move || {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("profiling");
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;

        let path = dir.join(format!("{file_stem}.jsonl"));
        if let Ok(meta) = fs::metadata(&path) {
            if meta.len() >= MAX_FILE_BYTES {
                let rolled = dir.join(format!(
                    "{file_stem}.{}.jsonl",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                ));
                fs::rename(&path, rolled).map_err(|error| error.to_string())?;
            }
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| error.to_string())?;
        let mut buffer = String::new();
        for line in &lines {
            buffer.push_str(line);
            buffer.push('\n');
        }
        file.write_all(buffer.as_bytes())
            .map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
    .map_err(|error| format!("spawn_blocking failed: {error}"))?
}

/// Accept only `session-<digits>` to keep the argument inside the sink dir.
fn is_valid_stem(stem: &str) -> bool {
    stem.strip_prefix("session-")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::is_valid_stem;

    #[test]
    fn accepts_session_timestamp_stem() {
        assert!(is_valid_stem("session-1700000000000"));
    }

    #[test]
    fn rejects_traversal_and_malformed_stems() {
        assert!(!is_valid_stem("session-../etc/passwd"));
        assert!(!is_valid_stem("session-"));
        assert!(!is_valid_stem("other-123"));
        assert!(!is_valid_stem("session-12a"));
    }
}
