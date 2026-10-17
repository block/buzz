//! Continuous native capture, H.264 encoding, and bounded live video batches.

pub(super) mod encoder;
#[cfg(target_os = "linux")]
mod portal;

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use buzz_core_pkg::huddle_screen::{ScreenMessage, ScreenShare, VIDEO_FPS};
use serde::Serialize;
use tauri::{Manager, State};
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::app_state::AppState;
use encoder::ScreenVideoEncoder;

/// A source in the native chooser. Portal selection is completed by the OS.
#[derive(Debug, Serialize)]
pub struct ScreenSource {
    /// Opaque stable source token, including a window's owning process.
    pub id: String,
    /// Human-readable source name.
    pub label: String,
    /// Display, window, or the platform screen-cast portal.
    pub source_type: &'static str,
}

#[derive(Debug)]
enum Source {
    Display(xcap::Monitor),
    Window {
        window: xcap::Window,
        pid: u32,
    },
    #[cfg(target_os = "linux")]
    PortalPicker,
    #[cfg(target_os = "linux")]
    PipeWire(Box<portal::PortalCapture>),
}

impl Source {
    fn frame(&self) -> Result<Option<image::RgbaImage>, String> {
        #[cfg(target_os = "linux")]
        if let Self::PipeWire(source) = self {
            return source.frame();
        }
        let (width, height) = match self {
            Self::Display(display) => (
                display.width().map_err(|e| e.to_string())?,
                display.height().map_err(|e| e.to_string())?,
            ),
            Self::Window { window, pid } => {
                if window.pid().map_err(|e| e.to_string())? != *pid
                    || window.is_minimized().map_err(|e| e.to_string())?
                {
                    return Err("The selected window is no longer available".into());
                }
                (
                    window.width().map_err(|e| e.to_string())?,
                    window.height().map_err(|e| e.to_string())?,
                )
            }
            #[cfg(target_os = "linux")]
            _ => return Err("Screen capture has not been started".into()),
        };
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 40_000_000 {
            return Err("Selected source exceeds the capture size limit".into());
        }
        let image = match self {
            Self::Display(display) => display.capture_image(),
            Self::Window { window, .. } => window.capture_image(),
            #[cfg(target_os = "linux")]
            _ => return Err("Screen capture has not been started".into()),
        }
        .map_err(|e| format!("Screen capture failed: {e}"))?;
        Ok(Some(image))
    }

    async fn close(self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        if let Self::PipeWire(source) = self {
            return source.close().await;
        }
        Ok(())
    }
}

/// Screen capture uses the same bounded consent lease as camera video.
pub type ScreenCapture = super::video::VideoCapture;

fn available_sources() -> Result<Vec<(ScreenSource, Source)>, String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE")
            .is_ok_and(|value| value.eq_ignore_ascii_case("wayland"))
    {
        return Ok(vec![(
            ScreenSource {
                id: "portal".into(),
                label: "Choose a screen or window".into(),
                source_type: "portal",
            },
            Source::PortalPicker,
        )]);
    }
    let mut sources = Vec::new();
    for (index, monitor) in xcap::Monitor::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .take(16)
        .enumerate()
    {
        let id = monitor.id().map_err(|e| e.to_string())?;
        let primary = monitor.is_primary().map_err(|e| e.to_string())?;
        sources.push((
            ScreenSource {
                id: format!("display:{id}"),
                label: format!(
                    "Display {}{}: {}",
                    index + 1,
                    if primary { " (primary)" } else { "" },
                    monitor.name().map_err(|e| e.to_string())?
                )
                .chars()
                .take(160)
                .collect(),
                source_type: "display",
            },
            Source::Display(monitor),
        ));
    }
    for window in xcap::Window::all()
        .map_err(|e| e.to_string())?
        .into_iter()
        .take(128)
    {
        let Ok(pid) = window.pid() else { continue };
        if !window.is_minimized().is_ok_and(|minimized| !minimized) {
            continue;
        }
        let Ok(title) = window.title() else { continue };
        if title.trim().is_empty() {
            continue;
        }
        let Ok(id) = window.id() else { continue };
        let Ok(app) = window.app_name() else { continue };
        sources.push((
            ScreenSource {
                id: format!("window:{id}:{pid}"),
                label: format!("{app}: {title}").chars().take(160).collect(),
                source_type: "window",
            },
            Source::Window { pid, window },
        ));
        if sources.len() >= 64 {
            break;
        }
    }
    Ok(sources)
}

/// List sources without beginning a capture or opening a portal session.
#[tauri::command]
pub async fn list_huddle_screen_sources(
    state: State<'_, AppState>,
) -> Result<Vec<ScreenSource>, String> {
    {
        let hs = state.huddle()?;
        if !matches!(
            hs.phase,
            super::HuddlePhase::Connected | super::HuddlePhase::Active
        ) {
            return Err("Join a huddle before sharing your screen".into());
        }
    }
    tokio::task::spawn_blocking(|| {
        available_sources().map(|sources| sources.into_iter().map(|(entry, _)| entry).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn capture_video(
    source: &Source,
    capture: &ScreenCapture,
    sender: mpsc::Sender<ScreenMessage>,
) -> Result<(), String> {
    let mut encoder = ScreenVideoEncoder::new(capture.share.clone())?;
    let epoch = Instant::now();
    let cadence = Duration::from_nanos(1_000_000_000 / u64::from(VIDEO_FPS));
    let mut next = epoch;
    let mut last_frame = epoch;
    while !capture.cancel.is_cancelled() {
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        if capture.cancel.is_cancelled() {
            break;
        }
        let Some(frame) = source.frame()? else {
            if last_frame.elapsed() > Duration::from_secs(5) {
                return Err("Screen capture stopped producing video".into());
            }
            next = Instant::now() + cadence;
            continue;
        };
        last_frame = Instant::now();
        let timestamp = u64::try_from(epoch.elapsed().as_micros()).map_err(|e| e.to_string())?;
        let batches = encoder.push(frame, timestamp)?;
        if capture.cancel.is_cancelled() {
            break;
        }
        for batch in batches {
            match sender.try_send(batch) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => encoder.discontinuity(),
                Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
            }
        }
        // No catch-up bursts after a slow capture or encode.
        next = std::cmp::max(next + cadence, Instant::now());
    }
    Ok(())
}

/// Begin continuous capture only after explicit source/portal consent.
#[tauri::command]
pub async fn start_huddle_screen_share(
    channel_id: String,
    source_id: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if source_id.len() > 64 {
        return Err("Invalid screen source".into());
    }
    let cancel = CancellationToken::new();
    let (lifetime, revision) = {
        let mut hs = state.huddle()?;
        if !hs.is_current_huddle(&channel_id, hs.huddle_generation) {
            return Err("The source chooser belongs to a different or inactive huddle".into());
        }
        if hs.screen_share.is_some() {
            return Err("Stop the current share before starting another".into());
        }
        if let Some(previous) = hs.screen_share_start_cancel.replace(cancel.clone()) {
            previous.cancel();
        }
        hs.screen_share_revision = hs.screen_share_revision.wrapping_add(1);
        (hs.huddle_generation, hs.screen_share_revision)
    };
    let pubkey = state.keys.lock().map_err(|e| e.to_string())?.public_key();
    let (entry, source) = tokio::task::spawn_blocking(move || {
        available_sources()?
            .into_iter()
            .find(|(entry, _)| entry.id == source_id)
            .ok_or_else(|| "The selected source is no longer available".to_owned())
    })
    .await
    .map_err(|e| e.to_string())??;
    #[cfg(target_os = "linux")]
    let source = if matches!(source, Source::PortalPicker) {
        Source::PipeWire(Box::new(portal::PortalCapture::open(&cancel).await?))
    } else {
        source
    };
    let share = ScreenShare {
        session_id: Uuid::new_v4(),
        started_at: u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis(),
        )
        .map_err(|e| e.to_string())?,
        label: entry.label,
    };
    let (sender, receiver) = mpsc::channel(2);
    let capture = Arc::new(ScreenCapture {
        share: share.clone(),
        pubkey,
        cancel: cancel.clone(),
        receiver: Mutex::new(receiver),
    });
    let committed = {
        let mut hs = state.huddle()?;
        if hs.is_current_huddle(&channel_id, lifetime)
            && hs.screen_share_revision == revision
            && !cancel.is_cancelled()
        {
            hs.screen_share_start_cancel = None;
            hs.screen_capture = Some(Arc::clone(&capture));
            hs.screen_share = Some(share);
            hs.screen_share_error = None;
            true
        } else {
            false
        }
    };
    if !committed {
        source.close().await?;
        return Err("Screen sharing was cancelled".into());
    }
    state.emit_huddle_state_changed();
    tokio::spawn(async move {
        let worker_capture = Arc::clone(&capture);
        let result = tokio::task::spawn_blocking(move || {
            let result = capture_video(&source, &worker_capture, sender);
            (source, result)
        })
        .await;
        let error = match result {
            Ok((source, result)) => source.close().await.and(result).err(),
            Err(error) => Some(format!("Video capture worker failed: {error}")),
        };
        capture.cancel();
        let state = app.state::<AppState>();
        if let Ok(mut hs) = state.huddle() {
            if hs.is_current_huddle(&channel_id, lifetime)
                && hs.screen_share.as_ref().map(|share| share.session_id)
                    == Some(capture.share.session_id)
            {
                hs.stop_screen_capture();
                hs.screen_share_error = error.map(|error| error.chars().take(300).collect());
                if let Some(error) = &hs.screen_share_error {
                    eprintln!("buzz-desktop: screen capture stopped: {error}");
                }
            }
        }
        state.emit_huddle_state_changed();
    });
    Ok(())
}

/// Stop only this share, including capture/encode and pending video reads.
#[tauri::command]
pub fn stop_huddle_screen_share(
    session_id: String,
    error: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let session = Uuid::parse_str(&session_id).map_err(|e| e.to_string())?;
    {
        let mut hs = state.huddle()?;
        if hs.screen_share.as_ref().map(|share| share.session_id) != Some(session) {
            return Ok(());
        }
        hs.stop_screen_capture();
        hs.screen_share_error = error.map(|value| value.chars().take(300).collect());
        if let Some(error) = &hs.screen_share_error {
            eprintln!("buzz-desktop: screen publication stopped: {error}");
        }
    }
    state.emit_huddle_state_changed();
    Ok(())
}

/// Consume the next bounded encoded-video batch, never a fresh snapshot.
#[tauri::command]
pub async fn read_huddle_screen_video_chunk(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    super::video::read_video_chunk(session_id, &state, buzz_core_pkg::kind::KIND_HUDDLE_SCREEN)
        .await
}
