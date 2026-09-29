//! Desktop IPC signer for Buzz Term / buzz-dev-mcp as the **signed-in user**.
//!
//! File-backed request/response under `{app_data}/user-signer/`:
//! - MCP (keyless) writes `{id}.request.json` into `inbox/`
//! - Desktop watcher authenticates reads with `AppState` keys and answers in `outbox/`
//! - Writes never auto-publish: `draft_message` emits a UI event so JM clicks Send
//!
//! The PTY never receives `BUZZ_PRIVATE_KEY` (see `buzz_terminal::env_fence`).

use std::fs::{create_dir_all, read_dir, remove_file, File};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::app_state::AppState;
use crate::relay::query_relay;

const INBOX: &str = "inbox";
const OUTBOX: &str = "outbox";

/// Env var MCP / Term config use to locate this IPC root.
pub const USER_SIGNER_DIR_ENV: &str = "BUZZ_USER_SIGNER_DIR";

/// Tauri event: frontend saves a composer draft and navigates for Send.
pub const DRAFT_EVENT: &str = "user-signer-draft";

/// Timeline kinds mirrored from `commands::messages` (channel/thread reads).
const TIMELINE_KINDS: [u32; 11] = [
    9,
    40002,
    40008,
    40099,
    43001,
    43002,
    43003,
    43004,
    43005,
    43006,
    buzz_core_pkg::kind::KIND_HUDDLE_STARTED,
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftMentionRef {
    display_name: String,
    pubkey: String,
    #[serde(default)]
    is_agent: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignerRequest {
    id: String,
    op: String,
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    mentions: Option<Vec<DraftMentionRef>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DraftEventMentionRef {
    display_name: String,
    pubkey: String,
    is_agent: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DraftEventPayload {
    channel_id: String,
    draft_key: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
    request_id: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mention_refs: Vec<DraftEventMentionRef>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Resolve and ensure `{app_data}/user-signer/{inbox,outbox}`.
pub fn ensure_root(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir unavailable: {e}"))?
        .join("user-signer");
    create_dir_all(root.join(INBOX)).map_err(|e| format!("user-signer inbox: {e}"))?;
    create_dir_all(root.join(OUTBOX)).map_err(|e| format!("user-signer outbox: {e}"))?;
    Ok(root)
}

fn write_response(root: &PathBuf, id: &str, body: serde_json::Value) {
    let path = root.join(OUTBOX).join(format!("{id}.response.json"));
    if let Ok(bytes) = serde_json::to_vec_pretty(&body) {
        if let Ok(mut f) = File::create(&path) {
            let _ = f.write_all(&bytes);
            let _ = f.write_all(b"\n");
        }
    }
}

fn summarize_events(events: &[nostr::Event]) -> Vec<serde_json::Value> {
    events
        .iter()
        .map(|ev| {
            json!({
                "id": ev.id.to_hex(),
                "pubkey": ev.pubkey.to_hex(),
                "createdAt": ev.created_at.as_secs() as i64,
                "kind": ev.kind.as_u16() as u32,
                "content": ev.content.clone(),
            })
        })
        .collect()
}

fn build_thread_filter(
    root_event_id: &str,
    channel_id: Option<&str>,
    cap: u32,
) -> serde_json::Map<String, serde_json::Value> {
    let mut filter = serde_json::Map::new();
    filter.insert("#e".into(), json!([root_event_id]));
    filter.insert("kinds".into(), json!(TIMELINE_KINDS));
    filter.insert("depth_limit".into(), json!(64));
    filter.insert("limit".into(), json!(cap));
    filter.insert("include_aux".into(), json!(true));
    if let Some(cid) = channel_id {
        filter.insert("#h".into(), json!([cid]));
    }
    filter
}

fn build_channel_filter(channel_id: &str, cap: u32) -> serde_json::Map<String, serde_json::Value> {
    let mut filter = serde_json::Map::new();
    filter.insert("#h".into(), json!([channel_id]));
    filter.insert("kinds".into(), json!(TIMELINE_KINDS));
    filter.insert("limit".into(), json!(cap));
    filter
}

async fn handle_request(app: &AppHandle, state: &AppState, root: &PathBuf, req: SignerRequest) {
    let id = req.id.trim().to_string();
    if id.is_empty() || id.len() > 128 {
        write_response(
            root,
            "invalid",
            json!({ "ok": false, "error": "request id invalid" }),
        );
        return;
    }
    let op = req.op.trim().to_ascii_lowercase();
    let cap = req.limit.unwrap_or(50).clamp(1, 200);

    match op.as_str() {
        "read_thread" => {
            let Some(thread_id) = req
                .thread_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                write_response(
                    root,
                    &id,
                    json!({ "id": id, "ok": false, "op": "read_thread", "error": "threadId required" }),
                );
                return;
            };
            let channel_id = req
                .channel_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let filter = build_thread_filter(thread_id, channel_id, cap);
            match query_relay(state, &[serde_json::Value::Object(filter)]).await {
                Ok(events) => {
                    let timeline: Vec<_> = events
                        .into_iter()
                        .filter(|e| TIMELINE_KINDS.contains(&(e.kind.as_u16() as u32)))
                        .collect();
                    write_response(
                        root,
                        &id,
                        json!({
                            "id": id,
                            "ok": true,
                            "op": "read_thread",
                            "threadId": thread_id,
                            "channelId": channel_id,
                            "events": summarize_events(&timeline),
                            "asUser": true,
                        }),
                    );
                }
                Err(e) => write_response(
                    root,
                    &id,
                    json!({ "id": id, "ok": false, "op": "read_thread", "error": e }),
                ),
            }
        }
        "read_channel" => {
            let Some(channel_id) = req
                .channel_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                write_response(
                    root,
                    &id,
                    json!({ "id": id, "ok": false, "op": "read_channel", "error": "channelId required" }),
                );
                return;
            };
            let filter = build_channel_filter(channel_id, cap);
            match query_relay(state, &[serde_json::Value::Object(filter)]).await {
                Ok(events) => write_response(
                    root,
                    &id,
                    json!({
                        "id": id,
                        "ok": true,
                        "op": "read_channel",
                        "channelId": channel_id,
                        "events": summarize_events(&events),
                        "asUser": true,
                    }),
                ),
                Err(e) => write_response(
                    root,
                    &id,
                    json!({ "id": id, "ok": false, "op": "read_channel", "error": e }),
                ),
            }
        }
        "draft_message" => {
            let Some(channel_id) = req
                .channel_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                write_response(
                    root,
                    &id,
                    json!({ "id": id, "ok": false, "op": "draft_message", "error": "channelId required" }),
                );
                return;
            };
            let content = req.content.unwrap_or_default();
            if content.trim().is_empty() {
                write_response(
                    root,
                    &id,
                    json!({ "id": id, "ok": false, "op": "draft_message", "error": "content required" }),
                );
                return;
            }
            if content.len() > 32_000 {
                write_response(
                    root,
                    &id,
                    json!({ "id": id, "ok": false, "op": "draft_message", "error": "content too long" }),
                );
                return;
            }
            let thread_id = req
                .thread_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let draft_key = match &thread_id {
                Some(tid) => format!("thread:{tid}"),
                None => channel_id.to_string(),
            };
            let mention_refs: Vec<DraftEventMentionRef> = req
                .mentions
                .unwrap_or_default()
                .into_iter()
                .filter_map(|m| {
                    let display_name = m.display_name.trim().to_string();
                    let pubkey = m.pubkey.trim().to_string();
                    if display_name.is_empty() || pubkey.is_empty() {
                        return None;
                    }
                    Some(DraftEventMentionRef {
                        display_name,
                        pubkey,
                        is_agent: m.is_agent.unwrap_or(true),
                    })
                })
                .collect();
            let payload = DraftEventPayload {
                channel_id: channel_id.to_string(),
                draft_key: draft_key.clone(),
                content,
                thread_id: thread_id.clone(),
                request_id: id.clone(),
                mention_refs,
            };
            if let Err(e) = app.emit(DRAFT_EVENT, &payload) {
                write_response(
                    root,
                    &id,
                    json!({ "id": id, "ok": false, "op": "draft_message", "error": format!("emit failed: {e}") }),
                );
                return;
            }
            write_response(
                root,
                &id,
                json!({
                    "id": id,
                    "ok": true,
                    "op": "draft_message",
                    "draftOnly": true,
                    "draftKey": draft_key,
                    "channelId": channel_id,
                    "threadId": thread_id,
                    "note": "Draft queued in Desktop. JM must click Send — nothing was published.",
                }),
            );
        }
        other => write_response(
            root,
            &id,
            json!({
                "id": id,
                "ok": false,
                "error": format!("unknown op '{other}' (read_thread|read_channel|draft_message)"),
            }),
        ),
    }
}

fn take_inbox_requests(root: &PathBuf) -> Vec<(PathBuf, SignerRequest)> {
    let inbox = root.join(INBOX);
    let Ok(entries) = read_dir(&inbox) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".request.json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Remove first so a crash mid-handle does not loop forever on poison.
        let _ = remove_file(&path);
        match serde_json::from_str::<SignerRequest>(&raw) {
            Ok(req) => out.push((path, req)),
            Err(e) => {
                let stem = name.trim_end_matches(".request.json");
                write_response(
                    root,
                    stem,
                    json!({
                        "id": stem,
                        "ok": false,
                        "error": format!("invalid request JSON: {e}"),
                        "ts": now_ms(),
                    }),
                );
            }
        }
    }
    out
}

/// Poll inbox every ~150ms and serve as the signed-in user.
pub fn spawn_user_signer_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let Ok(root) = ensure_root(&app) else {
                continue;
            };
            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            let requests = take_inbox_requests(&root);
            for (_path, req) in requests {
                handle_request(&app, state.inner(), &root, req).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_filter_includes_depth_and_kinds() {
        let f = build_thread_filter("abc", Some("chan"), 25);
        assert_eq!(f.get("depth_limit").and_then(|v| v.as_u64()), Some(64));
        assert!(f.get("kinds").is_some());
        assert_eq!(
            f.get("#e").and_then(|v| v.as_array()).map(|a| a.len()),
            Some(1)
        );
    }

    #[test]
    fn channel_filter_scopes_h_tag() {
        let f = build_channel_filter("chan-1", 10);
        assert_eq!(
            f.get("#h")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|v| v.as_str()),
            Some("chan-1")
        );
    }

    #[test]
    fn draft_key_prefers_thread() {
        let thread_id = Some("root-xyz".to_string());
        let draft_key = match &thread_id {
            Some(tid) => format!("thread:{tid}"),
            None => "chan".to_string(),
        };
        assert_eq!(draft_key, "thread:root-xyz");
    }

    #[test]
    fn signer_request_parses_mentions() {
        let raw = r#"{
            "id": "req-1",
            "op": "draft_message",
            "channelId": "ch",
            "content": "@Fable hi",
            "threadId": "th",
            "mentions": [{ "displayName": "Fable", "pubkey": "abc", "isAgent": true }]
        }"#;
        let req: SignerRequest = serde_json::from_str(raw).unwrap();
        let m = req.mentions.unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].display_name, "Fable");
        assert_eq!(m[0].pubkey, "abc");
        assert_eq!(m[0].is_agent, Some(true));
    }

    #[test]
    fn draft_event_payload_serializes_mention_refs() {
        let payload = DraftEventPayload {
            channel_id: "ch".into(),
            draft_key: "thread:th".into(),
            content: "@Fable hi".into(),
            thread_id: Some("th".into()),
            request_id: "r1".into(),
            mention_refs: vec![DraftEventMentionRef {
                display_name: "Fable".into(),
                pubkey: "abc".into(),
                is_agent: true,
            }],
        };
        let v = serde_json::to_value(&payload).unwrap();
        assert_eq!(v["mentionRefs"][0]["displayName"], "Fable");
        assert_eq!(v["mentionRefs"][0]["pubkey"], "abc");
        assert_eq!(v["mentionRefs"][0]["isAgent"], true);
    }
}
