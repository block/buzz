//! Append-only JSONL sink for channel-switch perf traces.
//!
//! The desktop's `[switch-perf]` console traces vanish with the session; this
//! sink persists one JSON line per settled switch to
//! `{app_log_dir}/switch-perf.jsonl` so before/after builds can be compared
//! offline. Every line is stamped with the build's git revision (baked by
//! build.rs) and, when set at launch, the `BUZZ_PERF_LOG_LABEL` run label —
//! e.g. `BUZZ_PERF_LOG_LABEL=before just production`.

use std::io::Write;

use tauri::Manager;

const PERF_LOG_FILENAME: &str = "switch-perf.jsonl";

/// Defensive cap: one record is a small trace object; anything larger is a
/// caller bug and must not grow the log unbounded.
const MAX_RECORD_BYTES: usize = 4 * 1024;

/// Rotation threshold. The sink is always on, so without a cap the JSONL
/// grows for the life of the install; one rotated generation preserves
/// enough history for before/after comparisons.
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;

/// Validates and shapes one JSONL line: the record must be a JSON object
/// (which also guarantees the stored line is newline-free), then the build
/// revision and optional run label are folded in. Pure for unit testing.
fn shape_perf_log_line(
    record_json: &str,
    git_sha: Option<&str>,
    label: Option<&str>,
) -> Result<String, String> {
    if record_json.len() > MAX_RECORD_BYTES {
        return Err("perf log record too large".to_string());
    }
    let mut value: serde_json::Value =
        serde_json::from_str(record_json).map_err(|e| format!("invalid perf log record: {e}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "perf log record must be a JSON object".to_string())?;
    object.insert(
        "gitSha".to_string(),
        match git_sha {
            Some(sha) => serde_json::Value::String(sha.to_string()),
            None => serde_json::Value::Null,
        },
    );
    if let Some(label) = label {
        object.insert(
            "label".to_string(),
            serde_json::Value::String(label.to_string()),
        );
    }
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

/// Appends one line, rotating the file to `<name>.1` (replacing the previous
/// generation) once it exceeds `max_bytes`. Factored for unit testing.
fn append_line_rotating(path: &std::path::Path, line: &str, max_bytes: u64) -> Result<(), String> {
    if let Ok(metadata) = std::fs::metadata(path) {
        if metadata.len() >= max_bytes {
            let mut rotated = path.as_os_str().to_owned();
            rotated.push(".1");
            std::fs::rename(path, std::path::Path::new(&rotated)).map_err(|e| e.to_string())?;
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{line}").map_err(|e| e.to_string())
}

/// Appends one switch-perf record to the app-log-dir JSONL file and returns
/// the file's path so the frontend can announce where the log lives.
///
/// Async so Tauri runs it on the async runtime rather than the main thread:
/// a perf sink must not add main-thread filesystem stalls to the switches it
/// measures.
#[tauri::command]
pub async fn append_switch_perf_log(
    app: tauri::AppHandle,
    record_json: String,
) -> Result<String, String> {
    let label = std::env::var("BUZZ_PERF_LOG_LABEL").ok();
    let line = shape_perf_log_line(
        &record_json,
        option_env!("BUZZ_DESKTOP_BUILD_GIT_SHA"),
        label.as_deref(),
    )?;
    let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    let path = dir.join(PERF_LOG_FILENAME);
    let result = tauri::async_runtime::spawn_blocking(move || {
        std::fs::create_dir_all(path.parent().unwrap_or(&path)).map_err(|e| e.to_string())?;
        append_line_rotating(&path, &line, MAX_LOG_BYTES)?;
        Ok::<String, String>(path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_folds_in_git_sha_and_label() {
        let line = shape_perf_log_line(r#"{"totalMs":412}"#, Some("abc123-dirty"), Some("before"))
            .expect("shape");
        let value: serde_json::Value = serde_json::from_str(&line).expect("parse");
        assert_eq!(value["totalMs"], 412);
        assert_eq!(value["gitSha"], "abc123-dirty");
        assert_eq!(value["label"], "before");
        assert!(!line.contains('\n'));
    }

    #[test]
    fn shape_without_label_or_sha_keeps_record_and_null_sha() {
        let line = shape_perf_log_line(r#"{"totalMs":1}"#, None, None).expect("shape");
        let value: serde_json::Value = serde_json::from_str(&line).expect("parse");
        assert_eq!(value["gitSha"], serde_json::Value::Null);
        assert!(value.get("label").is_none());
    }

    #[test]
    fn shape_rejects_non_objects_and_oversized_records() {
        assert!(shape_perf_log_line("[1,2]", None, None).is_err());
        assert!(shape_perf_log_line("not json", None, None).is_err());
        let oversized = format!(r#"{{"pad":"{}"}}"#, "x".repeat(MAX_RECORD_BYTES));
        assert!(shape_perf_log_line(&oversized, None, None).is_err());
    }

    #[test]
    fn append_rotates_once_over_the_cap_and_keeps_one_generation() {
        let dir = std::env::temp_dir().join(format!("perf-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        let path = dir.join("switch-perf.jsonl");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(dir.join("switch-perf.jsonl.1"));

        append_line_rotating(&path, "first", 16).expect("append");
        append_line_rotating(&path, "second", 16).expect("append");
        // 12 bytes so far — under the cap, same file.
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "first\nsecond\n"
        );

        // Push past the cap; the next append must rotate.
        append_line_rotating(&path, "third-is-long", 16).expect("append");
        append_line_rotating(&path, "fresh", 16).expect("append");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "fresh\n");
        assert_eq!(
            std::fs::read_to_string(dir.join("switch-perf.jsonl.1")).expect("read rotated"),
            "first\nsecond\nthird-is-long\n"
        );

        // A second rotation replaces the previous generation, never a third file.
        append_line_rotating(&path, "overflow-the-cap!", 16).expect("append");
        append_line_rotating(&path, "newest", 16).expect("append");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "newest\n");
        assert_eq!(
            std::fs::read_to_string(dir.join("switch-perf.jsonl.1")).expect("read rotated"),
            "fresh\noverflow-the-cap!\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
