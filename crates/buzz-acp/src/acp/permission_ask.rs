//! Opt-in interactive permission prompts.
//!
//! By default `session/request_permission` is auto-approved (`allow_once`). When the
//! harness is launched with `BUZZ_ACP_PERMISSION_MODE=ask` and a
//! `BUZZ_ACP_PERMISSION_DIR`, the request is written to that directory as
//! `<id>.req.json` and we wait for the owning desktop to drop `<id>.ans.json`
//! (`{"optionId": "..."}`), which it does when you answer, locally or from the web over
//! Direct. No answer within the timeout (`BUZZ_ACP_PERMISSION_TIMEOUT_S`, default
//! 120 s) means reject. The directory is private to the agent (0700 from the desktop).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

pub(crate) const MAX_TEXT: usize = 4000;

pub(crate) struct AskConfig {
    pub dir: PathBuf,
    pub timeout: Duration,
}

pub(crate) fn ask_config() -> Option<AskConfig> {
    if std::env::var("BUZZ_ACP_PERMISSION_MODE").ok()?.trim() != "ask" {
        return None;
    }
    let dir = PathBuf::from(std::env::var_os("BUZZ_ACP_PERMISSION_DIR")?);
    if !dir.is_absolute() {
        return None;
    }
    let secs = std::env::var("BUZZ_ACP_PERMISSION_TIMEOUT_S")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(120)
        .clamp(5, 3600);
    Some(AskConfig {
        dir,
        timeout: Duration::from_secs(secs),
    })
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_TEXT).collect()
}

/// The prompt record the desktop lists. Only display fields, no raw tool input beyond a
/// clipped text rendering (the desktop redacts secrets again before anything leaves it).
pub(crate) fn request_record(id: &str, params: &Value, now_s: u64) -> Value {
    let tc = &params["toolCall"];
    let title = tc["title"].as_str().unwrap_or("Permission requested");
    let detail = tc
        .get("rawInput")
        .map(|v| {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                v.to_string()
            }
        })
        .unwrap_or_default();
    let options: Vec<Value> = params["options"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|o| {
                    Some(serde_json::json!({
                        "optionId": o.get("optionId")?.as_str()?,
                        "name": clip(o.get("name").and_then(Value::as_str).unwrap_or("")),
                        "kind": o.get("kind").and_then(Value::as_str).unwrap_or(""),
                    }))
                })
                .take(10)
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({
        "id": id,
        "title": clip(title),
        "kind": tc["kind"].as_str().unwrap_or(""),
        "detail": clip(&detail),
        "options": options,
        "createdAt": now_s,
    })
}

/// The chosen option if the answer names one of the offered option ids.
pub(crate) fn parse_answer(raw: &str, record: &Value) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let chosen = v.get("optionId")?.as_str()?;
    record["options"]
        .as_array()?
        .iter()
        .any(|o| o["optionId"].as_str() == Some(chosen))
        .then(|| chosen.to_string())
}

fn safe_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(tmp, path)
}

/// Publish the request and wait for an answer. `None` on timeout or I/O failure.
pub(crate) async fn ask(cfg: &AskConfig, params: &Value) -> Option<String> {
    std::fs::create_dir_all(&cfg.dir).ok()?;
    let id = safe_id();
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let record = request_record(&id, params, now_s);
    let req = cfg.dir.join(format!("{id}.req.json"));
    let ans = cfg.dir.join(format!("{id}.ans.json"));
    write_atomic(&req, record.to_string().as_bytes()).ok()?;
    let deadline = tokio::time::Instant::now() + cfg.timeout;
    let mut out = None;
    while tokio::time::Instant::now() < deadline {
        if let Ok(raw) = std::fs::read_to_string(&ans) {
            out = parse_answer(&raw, &record);
            break;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let _ = std::fs::remove_file(&req);
    let _ = std::fs::remove_file(&ans);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Value {
        serde_json::json!({
            "toolCall": { "title": "Run ls", "kind": "execute", "rawInput": { "command": "ls" } },
            "options": [
                { "optionId": "a1", "name": "Allow once", "kind": "allow_once" },
                { "optionId": "r1", "name": "Reject", "kind": "reject_once" }
            ]
        })
    }

    #[test]
    fn record_has_display_fields_only() {
        let r = request_record("x", &params(), 7);
        assert_eq!(r["title"], "Run ls");
        assert_eq!(r["options"].as_array().unwrap().len(), 2);
        assert!(r["detail"].as_str().unwrap().contains("ls"));
    }

    #[test]
    fn answer_must_name_an_offered_option() {
        let r = request_record("x", &params(), 7);
        assert_eq!(
            parse_answer(r#"{"optionId":"r1"}"#, &r).as_deref(),
            Some("r1")
        );
        assert_eq!(parse_answer(r#"{"optionId":"zz"}"#, &r), None);
        assert_eq!(parse_answer("nope", &r), None);
    }

    #[tokio::test]
    async fn ask_round_trip_and_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = AskConfig {
            dir: dir.path().to_path_buf(),
            timeout: Duration::from_secs(5),
        };
        let d = dir.path().to_path_buf();
        let answerer = tokio::spawn(async move {
            loop {
                if let Some(e) = std::fs::read_dir(&d)
                    .unwrap()
                    .flatten()
                    .find(|e| e.file_name().to_string_lossy().ends_with(".req.json"))
                {
                    let id = e
                        .file_name()
                        .to_string_lossy()
                        .trim_end_matches(".req.json")
                        .to_string();
                    std::fs::write(d.join(format!("{id}.ans.json")), r#"{"optionId":"a1"}"#)
                        .unwrap();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        assert_eq!(ask(&cfg, &params()).await.as_deref(), Some("a1"));
        answerer.await.unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let short = AskConfig {
            dir: dir.path().to_path_buf(),
            timeout: Duration::from_millis(300),
        };
        assert_eq!(ask(&short, &params()).await, None);
    }
}
