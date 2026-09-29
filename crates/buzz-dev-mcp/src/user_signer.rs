//! Buzz Term / herdr: channel+thread tools via Desktop user-signer IPC.
//!
//! Desktop holds the signed-in user's keys. This process is keyless: it writes
//! request files under `BUZZ_USER_SIGNER_DIR` and polls for responses.
//! Reads (`buzz_read_*`) are fine for context. Writes are **draft-only**
//! (`buzz_draft_message`) — Desktop shows a draft JM must click Send on.
//! Use drafts only for content the human asked to post; never for agent
//! progress/status (that stays in the Term TUI). Never requires
//! `BUZZ_PRIVATE_KEY` in the PTY.

use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs::{create_dir_all, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use uuid::Uuid;

const DIR_ENV: &str = "BUZZ_USER_SIGNER_DIR";
const DEFAULT_WAIT_MS: u64 = 15_000;
const POLL_MS: u64 = 50;

#[cfg(test)]
thread_local! {
    static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn signer_root() -> Result<PathBuf, ErrorData> {
    #[cfg(test)]
    {
        if let Some(dir) = TEST_DIR.with(|c| c.borrow().clone()) {
            return Ok(dir);
        }
    }
    let raw = std::env::var(DIR_ENV).map_err(|_| {
        ErrorData::invalid_params(
            format!(
                "{DIR_ENV} is not set. Open a Buzz Term session from Desktop so prepare_term_session_launch wires buzz-dev-mcp with the user-signer IPC directory."
            ),
            None,
        )
    })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ErrorData::invalid_params(
            format!("{DIR_ENV} is empty"),
            None,
        ));
    }
    Ok(PathBuf::from(trimmed))
}

fn ensure_dirs(root: &Path) -> Result<(), ErrorData> {
    create_dir_all(root.join("inbox"))
        .map_err(|e| ErrorData::internal_error(format!("user-signer inbox: {e}"), None))?;
    create_dir_all(root.join("outbox"))
        .map_err(|e| ErrorData::internal_error(format!("user-signer outbox: {e}"), None))?;
    Ok(())
}

fn write_request(root: &Path, id: &str, body: &Value) -> Result<(), ErrorData> {
    ensure_dirs(root)?;
    let path = root.join("inbox").join(format!("{id}.request.json"));
    let bytes = serde_json::to_vec_pretty(body)
        .map_err(|e| ErrorData::internal_error(format!("serialize request: {e}"), None))?;
    let mut f = File::create(&path).map_err(|e| {
        ErrorData::internal_error(format!("write request {}: {e}", path.display()), None)
    })?;
    f.write_all(&bytes)
        .map_err(|e| ErrorData::internal_error(format!("write request: {e}"), None))?;
    f.write_all(b"\n")
        .map_err(|e| ErrorData::internal_error(format!("write request: {e}"), None))?;
    Ok(())
}

fn wait_response(root: &Path, id: &str, wait_ms: u64) -> Result<Value, ErrorData> {
    let path = root.join("outbox").join(format!("{id}.response.json"));
    let deadline = SystemTime::now() + Duration::from_millis(wait_ms.max(100));
    loop {
        if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| ErrorData::internal_error(format!("read response: {e}"), None))?;
            let _ = std::fs::remove_file(&path);
            let value: Value = serde_json::from_str(&raw)
                .map_err(|e| ErrorData::internal_error(format!("parse response: {e}"), None))?;
            return Ok(value);
        }
        if SystemTime::now() >= deadline {
            return Err(ErrorData::internal_error(
                format!(
                    "timed out waiting for Desktop user-signer response ({wait_ms}ms). Is Buzz Desktop running?"
                ),
                None,
            ));
        }
        std::thread::sleep(Duration::from_millis(POLL_MS));
    }
}

fn call_op(op: &str, mut fields: Value, wait_ms: u64) -> Result<CallToolResult, ErrorData> {
    let root = signer_root()?;
    let id = Uuid::new_v4().to_string();
    let obj = fields
        .as_object_mut()
        .ok_or_else(|| ErrorData::internal_error("request fields must be an object", None))?;
    obj.insert("id".into(), json!(id));
    obj.insert("op".into(), json!(op));
    write_request(&root, &id, &fields)?;
    let response = wait_response(&root, &id, wait_ms)?;
    let ok = response
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let text = serde_json::to_string_pretty(&response).unwrap_or_else(|_| response.to_string());
    if ok {
        Ok(CallToolResult::success(vec![Content::text(text)]))
    } else {
        let err = response
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("user-signer request failed");
        Err(ErrorData::invalid_params(err.to_string(), Some(response)))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadThreadParams {
    /// Thread root event id (hex).
    pub thread_id: String,
    /// Optional channel UUID to scope the query.
    #[serde(default)]
    pub channel_id: Option<String>,
    /// Max events (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadChannelParams {
    pub channel_id: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftMentionParam {
    /// Display name as it appears after `@` in content (no leading `@`).
    #[serde(alias = "display_name")]
    pub display_name: String,
    /// Hex pubkey for the mention chip.
    pub pubkey: String,
    /// True when the mention is an agent (default true for Term hand-back).
    #[serde(default, alias = "is_agent")]
    pub is_agent: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftMessageParams {
    pub channel_id: String,
    /// Message body the human asked to prepare for them to Send (not published;
    /// not for agent progress/status). Keep `@DisplayName` in content for rendering.
    pub content: String,
    /// When set, draft key is `thread:{thread_id}` and UI opens that thread.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Mention chips for the Desktop composer. Plain `@Name` in content alone is
    /// not enough — pass `{ displayName, pubkey, isAgent }` (camelCase on wire).
    #[serde(default)]
    pub mentions: Option<Vec<DraftMentionParam>>,
}

pub fn read_thread(p: ReadThreadParams) -> Result<CallToolResult, ErrorData> {
    let thread_id = p.thread_id.trim();
    if thread_id.is_empty() {
        return Err(ErrorData::invalid_params("thread_id required", None));
    }
    call_op(
        "read_thread",
        json!({
            "threadId": thread_id,
            "channelId": p.channel_id.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            "limit": p.limit.unwrap_or(50),
        }),
        DEFAULT_WAIT_MS,
    )
}

pub fn read_channel(p: ReadChannelParams) -> Result<CallToolResult, ErrorData> {
    let channel_id = p.channel_id.trim();
    if channel_id.is_empty() {
        return Err(ErrorData::invalid_params("channel_id required", None));
    }
    call_op(
        "read_channel",
        json!({
            "channelId": channel_id,
            "limit": p.limit.unwrap_or(50),
        }),
        DEFAULT_WAIT_MS,
    )
}

pub fn draft_message(p: DraftMessageParams) -> Result<CallToolResult, ErrorData> {
    let channel_id = p.channel_id.trim();
    if channel_id.is_empty() {
        return Err(ErrorData::invalid_params("channel_id required", None));
    }
    if p.content.trim().is_empty() {
        return Err(ErrorData::invalid_params("content required", None));
    }
    let mentions: Vec<serde_json::Value> = p
        .mentions
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            let display_name = m.display_name.trim();
            let pubkey = m.pubkey.trim();
            if display_name.is_empty() || pubkey.is_empty() {
                return None;
            }
            Some(json!({
                "displayName": display_name,
                "pubkey": pubkey,
                "isAgent": m.is_agent.unwrap_or(true),
            }))
        })
        .collect();
    let mut body = json!({
        "channelId": channel_id,
        "content": p.content,
        "threadId": p.thread_id.as_deref().map(str::trim).filter(|s| !s.is_empty()),
    });
    if !mentions.is_empty() {
        body.as_object_mut()
            .expect("object")
            .insert("mentions".into(), json!(mentions));
    }
    call_op("draft_message", body, DEFAULT_WAIT_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn draft_round_trip_via_files() {
        let _g = env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        create_dir_all(dir.path().join("inbox")).unwrap();
        create_dir_all(dir.path().join("outbox")).unwrap();
        TEST_DIR.with(|c| *c.borrow_mut() = Some(dir.path().to_path_buf()));

        // Simulate Desktop answering the next request.
        let root = dir.path().to_path_buf();
        std::thread::spawn(move || {
            let inbox = root.join("inbox");
            for _ in 0..200 {
                if let Ok(entries) = std::fs::read_dir(&inbox) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !name.ends_with(".request.json") {
                            continue;
                        }
                        let id = name.trim_end_matches(".request.json").to_string();
                        let _ = std::fs::remove_file(entry.path());
                        let resp = json!({
                            "id": id,
                            "ok": true,
                            "op": "draft_message",
                            "draftOnly": true,
                            "draftKey": "thread:abc",
                            "channelId": "chan-1",
                        });
                        let path = root.join("outbox").join(format!("{id}.response.json"));
                        std::fs::write(path, serde_json::to_string_pretty(&resp).unwrap()).unwrap();
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        let result = draft_message(DraftMessageParams {
            channel_id: "chan-1".into(),
            content: "hello from term".into(),
            thread_id: Some("abc".into()),
            mentions: None,
        });
        TEST_DIR.with(|c| *c.borrow_mut() = None);
        let ok = result.expect("draft ok");
        let text = ok.content[0].as_text().expect("text").text.clone();
        assert!(text.contains("draftOnly"));
        assert!(text.contains("thread:abc"));
    }

    #[test]
    fn missing_dir_env_errors() {
        let _g = env_lock().lock().unwrap();
        TEST_DIR.with(|c| *c.borrow_mut() = None);
        std::env::remove_var(DIR_ENV);
        let err = read_channel(ReadChannelParams {
            channel_id: "c".into(),
            limit: None,
        })
        .unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains(DIR_ENV) || msg.contains("BUZZ_USER_SIGNER"));
    }

    #[test]
    fn draft_params_accept_camel_case_mentions() {
        let p: DraftMessageParams = serde_json::from_str(
            r#"{
              "channel_id": "ch",
              "content": "@Fable done",
              "thread_id": "th",
              "mentions": [{
                "displayName": "Fable",
                "pubkey": "abc123",
                "isAgent": true
              }]
            }"#,
        )
        .unwrap();
        let m = p.mentions.unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].display_name, "Fable");
        assert_eq!(m[0].pubkey, "abc123");
        assert_eq!(m[0].is_agent, Some(true));
    }

    #[test]
    fn draft_writes_mentions_camel_case_on_wire() {
        let _g = env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        create_dir_all(dir.path().join("inbox")).unwrap();
        create_dir_all(dir.path().join("outbox")).unwrap();
        TEST_DIR.with(|c| *c.borrow_mut() = Some(dir.path().to_path_buf()));

        let root = dir.path().to_path_buf();
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let captured_bg = captured.clone();
        std::thread::spawn(move || {
            let inbox = root.join("inbox");
            for _ in 0..200 {
                if let Ok(entries) = std::fs::read_dir(&inbox) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !name.ends_with(".request.json") {
                            continue;
                        }
                        let raw = std::fs::read_to_string(entry.path()).unwrap();
                        *captured_bg.lock().unwrap() = Some(raw.clone());
                        let id = name.trim_end_matches(".request.json").to_string();
                        let _ = std::fs::remove_file(entry.path());
                        let resp = json!({
                            "id": id,
                            "ok": true,
                            "op": "draft_message",
                            "draftOnly": true,
                            "draftKey": "thread:abc",
                            "channelId": "chan-1",
                        });
                        let path = root.join("outbox").join(format!("{id}.response.json"));
                        std::fs::write(path, serde_json::to_string_pretty(&resp).unwrap()).unwrap();
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });

        let result = draft_message(DraftMessageParams {
            channel_id: "chan-1".into(),
            content: "@Fable shipped".into(),
            thread_id: Some("abc".into()),
            mentions: Some(vec![DraftMentionParam {
                display_name: "Fable".into(),
                pubkey: "deadbeef".into(),
                is_agent: Some(true),
            }]),
        });
        TEST_DIR.with(|c| *c.borrow_mut() = None);
        result.expect("draft ok");
        let raw = captured.lock().unwrap().clone().expect("captured request");
        assert!(
            raw.contains(r#""displayName": "Fable""#) || raw.contains(r#""displayName":"Fable""#)
        );
        assert!(raw.contains("deadbeef"));
        assert!(raw.contains(r#""isAgent""#));
    }
}
