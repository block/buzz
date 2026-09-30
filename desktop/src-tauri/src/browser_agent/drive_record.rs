//! Drive browser viewport recording (WKWebView snapshots → MP4 → grant chat).
//!
//! Opt-in only: agents start/stop via MCP `record-request.json` when a runbook
//! step or the user asks for a section clip. Never auto-starts on Drive grant.
//! Desktop captures the Drive WKWebView with the same snapshot path as Drive
//! stills, encodes H.264 MP4 (no audio), uploads, and posts as the grant agent
//! into the bound channel/thread. `stop` ends early; max duration is a safety
//! cap only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use super::drive_screen;
use super::grant::{now_ms, BrowserAgentGrant, BrowserAgentMode};
use super::viewport_gate;
use super::BrowserAgentState;

/// Snapshot cadence while recording (~4 fps).
pub const RECORD_FRAME_INTERVAL_MS: u64 = 250;
/// Safety hard stop if the agent forgets `browser_record_stop_and_post`.
/// Sized for a multi-step “record this section” clip; stop ends early.
pub const RECORD_MAX_DURATION_MS: u64 = 180_000;
/// Cap frames independently of duration (250ms × 720 ≈ 180s).
pub const RECORD_MAX_FRAMES: u32 = 720;
/// Encode framerate passed to ffmpeg (matches capture cadence).
pub const RECORD_FPS: u32 = 4;

#[derive(Debug)]
struct ActiveRecording {
    request_id: String,
    frames_dir: PathBuf,
    stop: Arc<AtomicBool>,
    started_at_ms: u64,
    /// Set when the capture loop finishes (stop, max duration, or error).
    finished: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
pub struct DriveRecordTracker {
    active: Mutex<HashMap<String, ActiveRecording>>,
}

impl DriveRecordTracker {
    pub fn is_recording(&self, label: &str) -> bool {
        self.active
            .lock()
            .ok()
            .map(|m| m.contains_key(label))
            .unwrap_or(false)
    }

    pub fn clear(&self, label: &str) {
        let session = self.active.lock().ok().and_then(|mut m| m.remove(label));
        if let Some(session) = session {
            session.stop.store(true, Ordering::Relaxed);
            let _ = std::fs::remove_dir_all(&session.frames_dir);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecordAction {
    Start,
    Stop,
}

#[derive(Debug, Clone)]
struct RecordRequest {
    action: RecordAction,
    request_id: String,
    agent_pubkey: String,
    caption: Option<String>,
}

fn parse_record_request(raw: &str) -> Result<RecordRequest, String> {
    let value: Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid record-request: {e}"))?;
    let action = value
        .get("action")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_ascii_lowercase();
    let action = match action.as_str() {
        "start" => RecordAction::Start,
        "stop" => RecordAction::Stop,
        other => {
            return Err(format!(
                "record action must be start|stop, got {other:?}"
            ))
        }
    };
    let request_id = value
        .get("requestId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "record-request missing requestId".to_string())?
        .to_string();
    let agent_pubkey = value
        .get("agentPubkey")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_ascii_lowercase();
    let caption = value
        .get("caption")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Ok(RecordRequest {
        action,
        request_id,
        agent_pubkey,
        caption,
    })
}

fn frame_path(dir: &Path, index: u32) -> PathBuf {
    dir.join(format!("frame_{index:04}.png"))
}

fn build_record_caption(url: &str, title: Option<&str>, agent_caption: Option<&str>) -> String {
    let title = title.map(str::trim).filter(|s| !s.is_empty());
    let mut lines: Vec<String> = Vec::new();
    match title {
        Some(t) => {
            let short = if t.len() > 64 { &t[..64] } else { t };
            lines.push(format!("**Browser recording** — {short}"));
        }
        None => lines.push("**Browser recording**".into()),
    }
    let url = url.trim();
    if !url.is_empty() {
        lines.push(url.to_string());
    }
    if let Some(c) = agent_caption.map(str::trim).filter(|s| !s.is_empty()) {
        let short = if c.len() > 280 { &c[..280] } else { c };
        lines.push(short.to_string());
    }
    lines.join("\n")
}

fn capture_label_png(app: &AppHandle, label: &str) -> Result<Vec<u8>, String> {
    let webview = app
        .get_webview(label)
        .ok_or_else(|| "webview not open".to_string())?;
    if label.starts_with("playground-") {
        if webview.label() != label {
            return Err("recording must target the playground webview".into());
        }
        if !crate::playground_webview::inspect_target_is_safe(label) {
            return Err("recording must target the playground webview".into());
        }
    }
    crate::playground_webview::capture::snapshot_viewport_png_for_drive(&webview)
}

fn push_record_event(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    kind: &str,
    payload: Value,
) {
    state
        .observe
        .push(label, kind, Some(payload.clone()), now_ms());
    let _ = app.emit(
        "browser-agent-observe",
        json!({
            "webviewLabel": label,
            "kind": kind,
            "payload": payload,
        }),
    );
}

fn mirror_recording_status(root: &Path, label: &str, status: Option<&Value>) {
    let dir = root.join(label);
    let path = dir.join("recording.json");
    match status {
        Some(v) => {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&path, v.to_string());
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn require_drive_grant(grant: &BrowserAgentGrant, agent_pubkey: &str) -> Result<(), String> {
    if !matches!(grant.mode, BrowserAgentMode::Drive) {
        return Err("recording requires Drive mode grant".into());
    }
    if !agent_pubkey.is_empty()
        && grant.agent_pubkey.to_ascii_lowercase() != agent_pubkey.to_ascii_lowercase()
    {
        return Err("record-request agentPubkey mismatch".into());
    }
    Ok(())
}

/// Process one `record-request.json` for a grant label. Returns 1 when handled.
pub async fn process_record_request(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    root: &PathBuf,
) -> u32 {
    let req_path = root.join(label).join("record-request.json");
    let Ok(raw) = std::fs::read_to_string(&req_path) else {
        return 0;
    };
    let _ = std::fs::remove_file(&req_path);
    let request = match parse_record_request(&raw) {
        Ok(r) => r,
        Err(e) => {
            push_record_event(
                app,
                state,
                label,
                "record_error",
                json!({ "ok": false, "error": e }),
            );
            return 1;
        }
    };
    let Some(grant) = state.grants.get(label) else {
        push_record_event(
            app,
            state,
            label,
            "record_error",
            json!({
                "ok": false,
                "requestId": request.request_id,
                "error": "no grant for webview",
            }),
        );
        return 1;
    };
    if let Err(e) = require_drive_grant(&grant, &request.agent_pubkey) {
        push_record_event(
            app,
            state,
            label,
            "record_error",
            json!({
                "ok": false,
                "requestId": request.request_id,
                "error": e,
            }),
        );
        return 1;
    }

    match request.action {
        RecordAction::Start => {
            start_recording(app, state, label, root, &grant, &request).await;
        }
        RecordAction::Stop => {
            stop_and_post(app, state, label, root, &grant, &request).await;
        }
    }
    1
}

async fn start_recording(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    root: &Path,
    grant: &BrowserAgentGrant,
    request: &RecordRequest,
) {
    if state.drive_records.is_recording(label) {
        push_record_event(
            app,
            state,
            label,
            "record_error",
            json!({
                "ok": false,
                "requestId": request.request_id,
                "error": "recording already active for this browser",
            }),
        );
        return;
    }

    if let Err(e) = viewport_gate::ensure_drive_viewport_usable(app, state, label).await {
        push_record_event(
            app,
            state,
            label,
            "record_error",
            json!({
                "ok": false,
                "requestId": request.request_id,
                "error": e,
            }),
        );
        return;
    }

    let frames_dir = std::env::temp_dir().join(format!(
        "buzz-drive-record-{}-{}",
        label.replace('/', "_"),
        uuid::Uuid::new_v4()
    ));
    if let Err(e) = std::fs::create_dir_all(&frames_dir) {
        push_record_event(
            app,
            state,
            label,
            "record_error",
            json!({
                "ok": false,
                "requestId": request.request_id,
                "error": format!("create frames dir: {e}"),
            }),
        );
        return;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let started_at_ms = now_ms();
    {
        let Ok(mut map) = state.drive_records.active.lock() else {
            let _ = std::fs::remove_dir_all(&frames_dir);
            return;
        };
        map.insert(
            label.to_string(),
            ActiveRecording {
                request_id: request.request_id.clone(),
                frames_dir: frames_dir.clone(),
                stop: stop.clone(),
                started_at_ms,
                finished: finished.clone(),
            },
        );
    }

    let status = json!({
        "active": true,
        "requestId": request.request_id,
        "startedAtMs": started_at_ms,
        "maxDurationMs": RECORD_MAX_DURATION_MS,
        "fps": RECORD_FPS,
    });
    mirror_recording_status(root, label, Some(&status));
    push_record_event(
        app,
        state,
        label,
        "record_started",
        json!({
            "ok": true,
            "requestId": request.request_id,
            "surfaceId": grant.surface_id,
            "webviewLabel": label,
            "maxDurationMs": RECORD_MAX_DURATION_MS,
            "fps": RECORD_FPS,
        }),
    );

    let app_capture = app.clone();
    let label_capture = label.to_string();
    let frames_capture = frames_dir.clone();
    let stop_capture = stop.clone();
    let finished_capture = finished.clone();
    tauri::async_runtime::spawn(async move {
        let mut frame_index: u32 = 0;
        let started = std::time::Instant::now();
        while !stop_capture.load(Ordering::Relaxed) {
            if started.elapsed().as_millis() as u64 >= RECORD_MAX_DURATION_MS {
                break;
            }
            if frame_index >= RECORD_MAX_FRAMES {
                break;
            }
            match capture_label_png(&app_capture, &label_capture) {
                Ok(png) => {
                    frame_index = frame_index.saturating_add(1);
                    let path = frame_path(&frames_capture, frame_index);
                    if let Err(e) = std::fs::write(&path, &png) {
                        eprintln!("buzz-desktop: drive record frame write failed: {e}");
                        break;
                    }
                }
                Err(e) => {
                    // Soft: skip a missed frame (hidden/parked webview) and continue.
                    eprintln!("buzz-desktop: drive record capture miss: {e}");
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(RECORD_FRAME_INTERVAL_MS)).await;
        }
        finished_capture.store(true, Ordering::Relaxed);
    });
}

async fn wait_capture_finished(finished: &AtomicBool, timeout_ms: u64) {
    let started = std::time::Instant::now();
    while !finished.load(Ordering::Relaxed) {
        if started.elapsed().as_millis() as u64 >= timeout_ms {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn stop_and_post(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    root: &Path,
    grant: &BrowserAgentGrant,
    request: &RecordRequest,
) {
    let session = {
        let Ok(mut map) = state.drive_records.active.lock() else {
            push_record_event(
                app,
                state,
                label,
                "record_error",
                json!({
                    "ok": false,
                    "requestId": request.request_id,
                    "error": "record tracker lock poisoned",
                }),
            );
            return;
        };
        map.remove(label)
    };
    let Some(session) = session else {
        push_record_event(
            app,
            state,
            label,
            "record_error",
            json!({
                "ok": false,
                "requestId": request.request_id,
                "error": "no active recording for this browser",
            }),
        );
        return;
    };

    session.stop.store(true, Ordering::Relaxed);
    wait_capture_finished(&session.finished, 5_000).await;

    let frame_count = count_frames(&session.frames_dir);
    if frame_count == 0 {
        let _ = std::fs::remove_dir_all(&session.frames_dir);
        mirror_recording_status(root, label, None);
        push_record_event(
            app,
            state,
            label,
            "record_error",
            json!({
                "ok": false,
                "requestId": request.request_id,
                "error": "recording produced no frames",
            }),
        );
        return;
    }

    let encode_result = tokio::task::spawn_blocking({
        let dir = session.frames_dir.clone();
        move || crate::commands::media_transcode::encode_png_sequence_to_mp4(&dir, RECORD_FPS)
    })
    .await;

    let mp4_path = match encode_result {
        Ok(Ok(path)) => path,
        Ok(Err(e)) => {
            let _ = std::fs::remove_dir_all(&session.frames_dir);
            mirror_recording_status(root, label, None);
            push_record_event(
                app,
                state,
                label,
                "record_error",
                json!({
                    "ok": false,
                    "requestId": request.request_id,
                    "error": e,
                    "frames": frame_count,
                }),
            );
            return;
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&session.frames_dir);
            mirror_recording_status(root, label, None);
            push_record_event(
                app,
                state,
                label,
                "record_error",
                json!({
                    "ok": false,
                    "requestId": request.request_id,
                    "error": format!("encode task failed: {e}"),
                    "frames": frame_count,
                }),
            );
            return;
        }
    };

    let body = match std::fs::read(&mp4_path) {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&mp4_path);
            let _ = std::fs::remove_dir_all(&session.frames_dir);
            mirror_recording_status(root, label, None);
            push_record_event(
                app,
                state,
                label,
                "record_error",
                json!({
                    "ok": false,
                    "requestId": request.request_id,
                    "error": format!("read mp4: {e}"),
                }),
            );
            return;
        }
    };
    let _ = std::fs::remove_file(&mp4_path);
    let _ = std::fs::remove_dir_all(&session.frames_dir);

    let Some(app_state) = app.try_state::<crate::app_state::AppState>() else {
        mirror_recording_status(root, label, None);
        push_record_event(
            app,
            state,
            label,
            "record_error",
            json!({
                "ok": false,
                "requestId": request.request_id,
                "error": "AppState unavailable",
            }),
        );
        return;
    };

    let filename = format!("drive-record-{}.mp4", &request.request_id);
    let blob = match crate::commands::media::upload_video_bytes(
        body,
        Some(filename),
        &app_state,
    )
    .await
    {
        Ok(b) => b,
        Err(e) => {
            mirror_recording_status(root, label, None);
            push_record_event(
                app,
                state,
                label,
                "record_error",
                json!({
                    "ok": false,
                    "requestId": request.request_id,
                    "error": format!("upload failed: {e}"),
                    "frames": frame_count,
                }),
            );
            return;
        }
    };

    let (url, title) = latest_nav(state, label);
    let caption = build_record_caption(
        &url,
        title.as_deref(),
        request.caption.as_deref(),
    );
    let content = format!("{caption}\n\n![video]({})", blob.url);
    let media = vec![drive_screen::imeta_tag_for_blob(&blob)];
    let duration_ms = now_ms().saturating_sub(session.started_at_ms);

    if let Err(e) = crate::commands::post_managed_agent_message_with_media(
        app,
        &app_state,
        &grant.agent_pubkey,
        &grant.channel_id,
        &content,
        grant.thread_root.as_deref(),
        &media,
        None,
    )
    .await
    {
        mirror_recording_status(root, label, None);
        push_record_event(
            app,
            state,
            label,
            "record_error",
            json!({
                "ok": false,
                "requestId": request.request_id,
                "error": format!("post failed: {e}"),
                "url": blob.url,
                "frames": frame_count,
            }),
        );
        return;
    }

    mirror_recording_status(root, label, None);
    push_record_event(
        app,
        state,
        label,
        "record_posted",
        json!({
            "ok": true,
            "requestId": request.request_id,
            "startRequestId": session.request_id,
            "surfaceId": grant.surface_id,
            "webviewLabel": label,
            "url": blob.url,
            "sha256": blob.sha256,
            "mime": blob.mime_type,
            "size": blob.size,
            "frames": frame_count,
            "durationMs": duration_ms,
            "fps": RECORD_FPS,
            "channelId": grant.channel_id,
            "threadRoot": grant.thread_root,
        }),
    );
}

fn count_frames(dir: &Path) -> u32 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("frame_") && n.ends_with(".png"))
        })
        .count() as u32
}

fn latest_nav(state: &BrowserAgentState, label: &str) -> (String, Option<String>) {
    let events = state.observe.poll(label, 0, 200);
    events
        .iter()
        .rev()
        .find(|e| e.kind == "nav")
        .and_then(|e| {
            let p = e.payload.as_ref()?;
            let url = p.get("url")?.as_str()?.to_string();
            let title = p
                .get("title")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Some((url, title))
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_start_and_stop_requests() {
        let start = parse_record_request(
            r#"{"action":"start","requestId":"r1","agentPubkey":"abc","atMs":1}"#,
        )
        .unwrap();
        assert_eq!(start.action, RecordAction::Start);
        assert_eq!(start.request_id, "r1");
        assert_eq!(start.agent_pubkey, "abc");

        let stop = parse_record_request(
            r#"{"action":"STOP","requestId":"r2","caption":" done ","agentPubkey":"ABC"}"#,
        )
        .unwrap();
        assert_eq!(stop.action, RecordAction::Stop);
        assert_eq!(stop.caption.as_deref(), Some("done"));
        assert_eq!(stop.agent_pubkey, "abc");
    }

    #[test]
    fn parse_rejects_bad_action() {
        let err = parse_record_request(r#"{"action":"pause","requestId":"r1"}"#).unwrap_err();
        assert!(err.contains("start|stop"), "{err}");
    }

    #[test]
    fn frame_paths_are_zero_padded() {
        assert_eq!(
            frame_path(Path::new("/tmp"), 1).file_name().unwrap(),
            "frame_0001.png"
        );
        assert_eq!(
            frame_path(Path::new("/tmp"), 12).file_name().unwrap(),
            "frame_0012.png"
        );
    }

    #[test]
    fn caption_includes_title_url_and_agent_note() {
        let c = build_record_caption(
            "https://ex.test/a",
            Some("Demo page"),
            Some("Walkthrough of checkout"),
        );
        assert!(c.contains("**Browser recording** — Demo page"));
        assert!(c.contains("https://ex.test/a"));
        assert!(c.contains("Walkthrough of checkout"));
    }

    #[test]
    fn record_limits_are_sane_for_v1() {
        assert!(RECORD_FRAME_INTERVAL_MS >= 100);
        assert_eq!(RECORD_MAX_DURATION_MS, 180_000);
        assert!(RECORD_MAX_DURATION_MS <= 300_000);
        assert_eq!(RECORD_MAX_FRAMES, 720);
        assert_eq!(RECORD_FPS, 4);
    }
}
