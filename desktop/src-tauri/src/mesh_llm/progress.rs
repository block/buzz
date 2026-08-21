//! Bridges mesh-llm's process-global output sink onto Tauri events so the
//! frontend can render model-download progress (bytes, percent) instead of a
//! frozen, greyed-out toggle. Same pattern as mesh-console's ConsoleSink.

use std::io;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use mesh_llm_events::{ConsoleSessionMode, ModelProgressStatus, OutputEvent, OutputSink};

/// Tauri event name the frontend subscribes to.
pub const MESH_DOWNLOAD_PROGRESS_EVENT: &str = "mesh-download-progress";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshDownloadProgress {
    /// Model (or runtime) label being downloaded.
    pub label: String,
    /// Specific file within the download, when known.
    pub file: Option<String>,
    pub downloaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    /// preparing | downloading | done — mirrors ModelProgressStatus.
    pub status: &'static str,
    pub done: bool,
}

struct TauriProgressSink {
    app: AppHandle,
}

impl OutputSink for TauriProgressSink {
    fn emit_event(&self, event: OutputEvent) -> io::Result<()> {
        if let OutputEvent::ModelDownloadProgress {
            label,
            file,
            downloaded_bytes,
            total_bytes,
            status,
        } = event
        {
            let payload = MeshDownloadProgress {
                label,
                file,
                downloaded_bytes,
                total_bytes,
                status: match status {
                    ModelProgressStatus::Ensuring => "preparing",
                    ModelProgressStatus::Downloading => "downloading",
                    ModelProgressStatus::Ready => "done",
                },
                done: matches!(status, ModelProgressStatus::Ready),
            };
            let _ = self.app.emit(MESH_DOWNLOAD_PROGRESS_EVENT, payload);
        }
        Ok(())
    }

    /// Byte-level ModelDownloadProgress only flows through the sink when the
    /// host-runtime believes an interactive dashboard is attached; otherwise
    /// it draws ANSI progress bars on stderr. Our UI *is* the dashboard.
    fn console_session_mode(&self) -> Option<ConsoleSessionMode> {
        Some(ConsoleSessionMode::InteractiveDashboard)
    }
}

/// Install (or replace) the process-global progress sink pointed at this app.
/// Idempotent in effect — replacing with an equivalent sink is harmless.
pub fn install_progress_sink(app: &AppHandle) {
    mesh_llm_events::set_output_sink(Arc::new(TauriProgressSink { app: app.clone() }));
}
