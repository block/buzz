//! Host-side Drive "new screen" screenshots → grant channel/thread chat.
//!
//! On each meaningful Drive navigation (URL without hash), capture the WKWebView,
//! build a short path caption from Drive actions since the last post, upload the
//! PNG, and post as the grant's managed agent into the bound channel/thread.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use super::grant::{now_ms, BrowserAgentMode};
use super::observe::ObserveEvent;
use super::BrowserAgentState;

/// Final paint settle after the page reports ready + network quiet.
pub const DRIVE_SCREEN_SETTLE_MS: u64 = 400;
/// How often to poll document.readyState / network quiet.
pub const DRIVE_SCREEN_READY_POLL_MS: u64 = 200;
/// Max wait for document complete + quiet before giving up on this nav.
pub const DRIVE_SCREEN_READY_TIMEOUT_MS: u64 = 12_000;
/// Consecutive quiet+complete probes required before capture.
pub const DRIVE_SCREEN_READY_STREAK: u32 = 2;
/// Ignore rapid re-posts for the same grant (defense in depth vs settle).
pub const DRIVE_SCREEN_MIN_INTERVAL_MS: u64 = 2_500;
/// Max path bullets in the chat caption.
const MAX_PATH_BULLETS: usize = 8;

#[derive(Debug, Default)]
pub struct DriveScreenTracker {
    /// Per-label generation so a newer nav cancels an in-flight settle.
    generation: Mutex<HashMap<String, u64>>,
    /// Last posted screen key (url without hash) per label.
    last_posted_key: Mutex<HashMap<String, String>>,
    /// Observe ring id watermark after last successful post.
    after_event_id: Mutex<HashMap<String, u64>>,
    /// Last successful post time (ms) per label.
    last_posted_at_ms: Mutex<HashMap<String, u64>>,
}

impl DriveScreenTracker {
    pub fn bump_generation(&self, label: &str) -> u64 {
        let Ok(mut map) = self.generation.lock() else {
            return 0;
        };
        let entry = map.entry(label.to_string()).or_insert(0);
        *entry = entry.saturating_add(1);
        *entry
    }

    pub fn generation(&self, label: &str) -> u64 {
        self.generation
            .lock()
            .ok()
            .and_then(|m| m.get(label).copied())
            .unwrap_or(0)
    }

    pub fn clear(&self, label: &str) {
        if let Ok(mut m) = self.generation.lock() {
            m.remove(label);
        }
        if let Ok(mut m) = self.last_posted_key.lock() {
            m.remove(label);
        }
        if let Ok(mut m) = self.after_event_id.lock() {
            m.remove(label);
        }
        if let Ok(mut m) = self.last_posted_at_ms.lock() {
            m.remove(label);
        }
    }

    fn last_posted_key(&self, label: &str) -> Option<String> {
        self.last_posted_key
            .lock()
            .ok()
            .and_then(|m| m.get(label).cloned())
    }

    fn after_event_id(&self, label: &str) -> u64 {
        self.after_event_id
            .lock()
            .ok()
            .and_then(|m| m.get(label).copied())
            .unwrap_or(0)
    }

    fn mark_posted(&self, label: &str, key: &str, after_id: u64, at_ms: u64) {
        if let Ok(mut m) = self.last_posted_key.lock() {
            m.insert(label.to_string(), key.to_string());
        }
        if let Ok(mut m) = self.after_event_id.lock() {
            m.insert(label.to_string(), after_id);
        }
        if let Ok(mut m) = self.last_posted_at_ms.lock() {
            m.insert(label.to_string(), at_ms);
        }
    }

    fn last_posted_at_ms(&self, label: &str) -> u64 {
        self.last_posted_at_ms
            .lock()
            .ok()
            .and_then(|m| m.get(label).copied())
            .unwrap_or(0)
    }
}

/// Strip fragment for "same screen" debounce (hash-only churn ignored).
pub fn screen_url_key(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match url::Url::parse(trimmed) {
        Ok(mut parsed) => {
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => match trimmed.split_once('#') {
            Some((before, _)) => before.to_string(),
            None => trimmed.to_string(),
        },
    }
}

fn truncate(value: &str, max: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn short_url(url: &str) -> String {
    if url.trim().is_empty() {
        return "page".into();
    }
    if let Ok(parsed) = url::Url::parse(url) {
        let host = parsed.host_str().unwrap_or("");
        let path = parsed.path();
        if !host.is_empty() {
            if path.is_empty() || path == "/" {
                return host.to_string();
            }
            return truncate(&format!("{host}{path}"), 48);
        }
    }
    truncate(url, 48)
}

fn hit_label(payload: &Value) -> String {
    let hit = payload.get("hit");
    if let Some(name) = hit
        .and_then(|h| h.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return truncate(name, 32);
    }
    if let Some(role) = hit
        .and_then(|h| h.get("role"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return role.to_string();
    }
    if let Some(tag) = hit
        .and_then(|h| h.get("tag"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!("<{tag}>");
    }
    let x = payload.get("x").and_then(|v| v.as_f64());
    let y = payload.get("y").and_then(|v| v.as_f64());
    if let (Some(x), Some(y)) = (x, y) {
        return format!("({}, {})", x.round() as i64, y.round() as i64);
    }
    "target".into()
}

/// Past-tense path bullet from a Drive observe event payload.
pub fn drive_path_bullet(payload: &Value) -> Option<String> {
    let action = payload
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match action.as_str() {
        "navigate" => {
            let url = payload
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!("Opened {}", short_url(url)))
        }
        "click" | "clickat" => Some(format!("Clicked {}", hit_label(payload))),
        "hover" => Some(format!("Moved to {}", hit_label(payload))),
        "type" => {
            // Never echo typed text — may be passwords / secrets.
            Some("Typed text".into())
        }
        "key" => {
            let key = payload
                .get("key")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            Some(match key {
                Some(k) => format!("Pressed {k}"),
                None => "Pressed a key".into(),
            })
        }
        "scroll" => Some("Scrolled".into()),
        "waitfor" => None, // noise
        _ => None,
    }
}

/// Build chat caption: header + bullets for the path to this screen.
pub fn build_drive_screen_caption(
    url: &str,
    title: Option<&str>,
    drive_events: &[ObserveEvent],
) -> String {
    let mut bullets: Vec<String> = Vec::new();
    for event in drive_events {
        if event.kind != "drive" {
            continue;
        }
        let Some(payload) = event.payload.as_ref() else {
            continue;
        };
        if payload.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        if let Some(bullet) = drive_path_bullet(payload) {
            if bullets.last() == Some(&bullet) {
                continue; // collapse exact repeats
            }
            bullets.push(bullet);
            if bullets.len() >= MAX_PATH_BULLETS {
                break;
            }
        }
    }

    let title = title.map(str::trim).filter(|s| !s.is_empty());
    let mut lines: Vec<String> = Vec::new();
    match title {
        Some(t) => lines.push(format!("**Browser progress** — {}", truncate(t, 64))),
        None => lines.push("**Browser progress**".into()),
    }
    lines.push(short_url(url));
    if bullets.is_empty() {
        lines.push("- Navigated to this screen".into());
    } else {
        for b in bullets {
            lines.push(format!("- {b}"));
        }
    }
    lines.join("\n")
}

fn imeta_tag_for_blob(blob: &crate::commands::media::BlobDescriptor) -> Vec<String> {
    let mut tag = vec![
        "imeta".into(),
        format!("url {}", blob.url),
        format!("m {}", blob.mime_type),
        format!("x {}", blob.sha256),
    ];
    if blob.size > 0 {
        tag.push(format!("size {}", blob.size));
    }
    if let Some(dim) = blob.dim.as_deref().filter(|s| !s.is_empty()) {
        tag.push(format!("dim {dim}"));
    }
    if let Some(blur) = blob.blurhash.as_deref().filter(|s| !s.is_empty()) {
        tag.push(format!("blurhash {blur}"));
    }
    if let Some(name) = blob.filename.as_deref().filter(|s| !s.is_empty()) {
        tag.push(format!("filename {name}"));
    }
    tag
}

/// Schedule a Drive screen post after settle, cancelling prior pending work.
pub fn schedule_drive_screen_post(app: &AppHandle, webview_label: &str, url: &str) {
    let Some(state) = app.try_state::<BrowserAgentState>() else {
        return;
    };
    let Some(grant) = state.grants.get(webview_label) else {
        return;
    };
    if !matches!(grant.mode, BrowserAgentMode::Drive) || grant.user_has_control {
        return;
    }
    let key = screen_url_key(url);
    if key.is_empty() || key == "about:blank" {
        return;
    }
    if state.drive_screens.last_posted_key(webview_label).as_deref() == Some(key.as_str()) {
        return;
    }
    let gen = state.drive_screens.bump_generation(webview_label);
    let label = webview_label.to_string();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<BrowserAgentState>() else {
            return;
        };
        if !wait_for_drive_screen_ready(&app, &state, &label, gen).await {
            return; // superseded, timed out unfinished, or webview gone
        }
        if let Err(e) = post_drive_screen(&app, &state, &label, &key).await {
            eprintln!("buzz-desktop: drive screen post {label}: {e}");
        }
    });
}

/// Poll until document complete + network quiet (streak), then a short paint
/// settle. Cancels when a newer nav bumps generation.
async fn wait_for_drive_screen_ready(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    gen: u64,
) -> bool {
    let started = std::time::Instant::now();
    let mut streak = 0u32;
    let mut saw_complete = false;
    loop {
        if state.drive_screens.generation(label) != gen {
            return false;
        }
        let elapsed_ms = started.elapsed().as_millis() as u64;
        if elapsed_ms >= DRIVE_SCREEN_READY_TIMEOUT_MS {
            // Do not post a half-loaded crop. Only proceed if we saw complete
            // at least once and the last probe was settled enough to try.
            return saw_complete && streak >= 1 && state.drive_screens.generation(label) == gen;
        }

        let label_owned = label.to_string();
        let app_probe = app.clone();
        let probe = match tokio::task::spawn_blocking(move || {
            let webview = app_probe.get_webview(&label_owned)?;
            crate::playground_webview::capture::probe_page_ready(&webview).ok()
        })
        .await
        {
            Ok(Some(p)) => Some(p),
            Ok(None) => {
                // Webview gone or eval failed this tick — keep waiting unless
                // the label disappeared entirely.
                if app.get_webview(label).is_none() {
                    return false;
                }
                None
            }
            Err(_) => None,
        };

        match probe {
            Some(p) if p.is_settled() => {
                saw_complete = true;
                streak = streak.saturating_add(1);
                if streak >= DRIVE_SCREEN_READY_STREAK {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        DRIVE_SCREEN_SETTLE_MS,
                    ))
                    .await;
                    return state.drive_screens.generation(label) == gen;
                }
            }
            Some(p) => {
                if p.ready == "complete" {
                    saw_complete = true;
                }
                streak = 0;
            }
            None => {
                streak = 0;
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(DRIVE_SCREEN_READY_POLL_MS))
            .await;
    }
}

async fn post_drive_screen(
    app: &AppHandle,
    state: &BrowserAgentState,
    label: &str,
    key: &str,
) -> Result<(), String> {
    let Some(grant) = state.grants.get(label) else {
        return Ok(());
    };
    if !matches!(grant.mode, BrowserAgentMode::Drive) || grant.user_has_control {
        return Ok(());
    }
    if state.drive_screens.last_posted_key(label).as_deref() == Some(key) {
        return Ok(());
    }
    let now = now_ms();
    let last_at = state.drive_screens.last_posted_at_ms(label);
    if last_at > 0 && now.saturating_sub(last_at) < DRIVE_SCREEN_MIN_INTERVAL_MS {
        return Ok(());
    }

    let after_id = state.drive_screens.after_event_id(label);
    let events = state.observe.poll(label, after_id, 200);
    let max_id = events.iter().map(|e| e.id).max().unwrap_or(after_id);
    let (url, title) = events
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
        .unwrap_or_else(|| (key.to_string(), None));

    let drive_events: Vec<ObserveEvent> = events
        .into_iter()
        .filter(|e| e.kind == "drive" || e.kind == "drive_error")
        .collect();
    let caption = build_drive_screen_caption(&url, title.as_deref(), &drive_events);

    let png = capture_label_png(app, label)?;
    let Some(app_state) = app.try_state::<crate::app_state::AppState>() else {
        return Err("AppState unavailable".into());
    };
    let blob = crate::commands::media::upload_image_bytes(png, &app_state).await?;

    let content = format!("{caption}\n\n![image]({})", blob.url);
    let media = vec![imeta_tag_for_blob(&blob)];
    crate::commands::post_managed_agent_message_with_media(
        app,
        &app_state,
        &grant.agent_pubkey,
        &grant.channel_id,
        &content,
        grant.thread_root.as_deref(),
        &media,
        None,
    )
    .await?;

    state
        .drive_screens
        .mark_posted(label, key, max_id.max(after_id), now_ms());
    let _ = app.emit(
        "browser-agent-observe",
        serde_json::json!({
            "webviewLabel": label,
            "kind": "drive_screen_posted",
            "payload": { "url": url, "title": title },
        }),
    );
    Ok(())
}

fn capture_label_png(app: &AppHandle, label: &str) -> Result<Vec<u8>, String> {
    let webview = app
        .get_webview(label)
        .ok_or_else(|| "webview not open".to_string())?;
    if label.starts_with("playground-") {
        // Same label guard as capture_playground_png (reject non-playground).
        if webview.label() != label {
            return Err("screenshot must target the playground webview".into());
        }
        if !crate::playground_webview::inspect_target_is_safe(label) {
            return Err("screenshot must target the playground webview".into());
        }
    }
    // Full WKWebView viewport after zoom reset + scroll-to-top — not a
    // device-bezel crop or mid-load fragment.
    crate::playground_webview::capture::snapshot_viewport_png_for_drive(&webview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ready_wait_constants_prefer_probe_over_fixed_delay() {
        assert!(DRIVE_SCREEN_READY_TIMEOUT_MS > DRIVE_SCREEN_SETTLE_MS);
        assert!(DRIVE_SCREEN_READY_POLL_MS >= 100);
        assert!(DRIVE_SCREEN_READY_STREAK >= 2);
        // Old fixed 1.2s alone was too early for many pages.
        assert!(DRIVE_SCREEN_SETTLE_MS <= 600);
    }

    #[test]
    fn screen_url_key_strips_hash() {
        assert_eq!(
            screen_url_key("https://ex.test/a#section"),
            "https://ex.test/a"
        );
        assert_eq!(
            screen_url_key("https://ex.test/a"),
            "https://ex.test/a"
        );
        assert_eq!(screen_url_key("https://ex.test/a#"), "https://ex.test/a");
    }

    #[test]
    fn path_bullets_are_past_tense_and_hide_typed_text() {
        assert_eq!(
            drive_path_bullet(&json!({"kind":"click","hit":{"name":"Submit"}})).as_deref(),
            Some("Clicked Submit")
        );
        assert_eq!(
            drive_path_bullet(&json!({"kind":"type","text":"secret"})).as_deref(),
            Some("Typed text")
        );
        assert_eq!(
            drive_path_bullet(&json!({"kind":"key","key":"Enter"})).as_deref(),
            Some("Pressed Enter")
        );
        assert_eq!(drive_path_bullet(&json!({"kind":"waitFor"})), None);
    }

    #[test]
    fn caption_includes_bullets_and_url() {
        let events = vec![ObserveEvent {
            id: 1,
            webview_label: "playground-a".into(),
            kind: "drive".into(),
            at_ms: 1,
            payload: Some(json!({"ok":true,"kind":"click","hit":{"name":"Next"}})),
        }];
        let caption = build_drive_screen_caption(
            "https://ex.test/step-2",
            Some("Step two"),
            &events,
        );
        assert!(caption.contains("**Browser progress** — Step two"));
        assert!(caption.contains("ex.test/step-2"));
        assert!(caption.contains("- Clicked Next"));
    }
}
