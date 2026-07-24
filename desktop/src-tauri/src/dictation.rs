//! Standalone composer dictation backed by the local huddle STT engine.
//!
//! Dictation deliberately owns a separate pipeline and PCM command from
//! huddles. Microphone audio captured for a draft must never be fanned out to
//! the huddle relay, even when a huddle is active in the same process.

use std::sync::{atomic::AtomicBool, Arc, LazyLock, Mutex, MutexGuard};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::{
    app_state::AppState,
    huddle::{
        models::{self, ModelStatus},
        stt::SttPipeline,
    },
};

const MAX_AUDIO_BATCH_BYTES: usize = 100 * 1024;

/// Runtime state for the one process-wide composer dictation session.
#[derive(Default)]
struct DictationState {
    pipeline: Option<Arc<SttPipeline>>,
    generation: u64,
}

static DICTATION_STATE: LazyLock<Mutex<DictationState>> =
    LazyLock::new(|| Mutex::new(DictationState::default()));

fn dictation() -> Result<MutexGuard<'static, DictationState>, String> {
    DICTATION_STATE.lock().map_err(|error| error.to_string())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DictationTranscript {
    session_id: u64,
    text: String,
}

fn model_not_ready_message(status: ModelStatus) -> String {
    match status {
        ModelStatus::NotDownloaded => {
            "Voice input is getting ready. The speech model download has started; try again shortly."
                .to_string()
        }
        ModelStatus::Downloading { progress_percent } => format!(
            "Voice input is getting ready ({progress_percent}% downloaded). Try again shortly."
        ),
        ModelStatus::Ready => "Speech model is not available yet.".to_string(),
        ModelStatus::Error(error) => format!("Speech model download failed: {error}"),
    }
}

/// Start an isolated, continuous-VAD dictation session.
///
/// Returns a monotonically increasing session id. Transcript events include
/// the same id so composers can ignore results from replaced sessions.
#[tauri::command]
pub async fn start_dictation(app: AppHandle, state: State<'_, AppState>) -> Result<u64, String> {
    let manager = models::global_model_manager()
        .ok_or("model manager unavailable (home directory could not be resolved)")?;
    manager.start_stt_download(state.http_client.clone());
    let model_dir = manager
        .stt_model_dir()
        .ok_or_else(|| model_not_ready_message(manager.stt_status()))?;

    let (session_id, old_pipeline) = {
        let mut dictation = dictation()?;
        dictation.generation = dictation.generation.wrapping_add(1);
        let session_id = dictation.generation;
        let old_pipeline = dictation.pipeline.take();
        (session_id, old_pipeline)
    };

    if let Some(ref pipeline) = old_pipeline {
        pipeline.shutdown();
    }
    // SttPipeline::drop joins its worker, so never drop it while holding state.
    drop(old_pipeline);

    let constructed = tokio::task::spawn_blocking(move || {
        SttPipeline::new(model_dir, Arc::new(AtomicBool::new(false)), None, None)
    })
    .await;
    let (pipeline, mut text_rx) = match constructed {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => return Err(error),
        Err(error) => return Err(format!("failed to start dictation worker: {error}")),
    };
    let pipeline = Arc::new(pipeline);

    let installed = {
        let mut dictation = dictation()?;
        if dictation.generation != session_id {
            false
        } else {
            dictation.pipeline = Some(Arc::clone(&pipeline));
            true
        }
    };
    if !installed {
        pipeline.shutdown();
        drop(pipeline);
        return Err("dictation start was superseded".to_string());
    }

    tauri::async_runtime::spawn(async move {
        while let Some(text) = text_rx.recv().await {
            if text.trim().is_empty() {
                continue;
            }
            let is_active = dictation()
                .map(|dictation| dictation.generation == session_id && dictation.pipeline.is_some())
                .unwrap_or(false);
            if !is_active {
                break;
            }
            let _ = app.emit(
                "dictation-transcript",
                DictationTranscript { session_id, text },
            );
        }
    });

    Ok(session_id)
}

/// Stop a dictation session if it is still current.
///
/// Stale callers are ignored so cleanup from an old composer cannot stop a
/// newer session started in another channel or thread.
#[tauri::command]
pub fn stop_dictation(session_id: u64) -> Result<(), String> {
    let old_pipeline = {
        let mut dictation = dictation()?;
        if dictation.generation != session_id {
            return Ok(());
        }
        dictation.generation = dictation.generation.wrapping_add(1);
        dictation.pipeline.take()
    };

    if let Some(ref pipeline) = old_pipeline {
        pipeline.shutdown();
    }
    drop(old_pipeline);
    Ok(())
}

/// Feed raw microphone PCM to composer dictation only.
///
/// Expects f32 little-endian samples at 48 kHz mono. This command intentionally
/// does not touch huddle state or the huddle relay encoder.
#[tauri::command]
pub fn push_dictation_audio_pcm(request: tauri::ipc::Request<'_>) -> Result<(), String> {
    match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => {
            if bytes.len() > MAX_AUDIO_BATCH_BYTES {
                return Err(format!(
                    "audio batch too large: {} bytes (max {})",
                    bytes.len(),
                    MAX_AUDIO_BATCH_BYTES
                ));
            }
            if let Some(ref pipeline) = dictation()?.pipeline {
                pipeline.push_audio(bytes.to_vec())?;
            }
            Ok(())
        }
        _ => Err("expected raw binary body".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::model_not_ready_message;
    use crate::huddle::models::ModelStatus;

    #[test]
    fn download_progress_is_actionable() {
        assert_eq!(
            model_not_ready_message(ModelStatus::Downloading {
                progress_percent: 42
            }),
            "Voice input is getting ready (42% downloaded). Try again shortly."
        );
    }

    #[test]
    fn download_errors_are_preserved() {
        assert_eq!(
            model_not_ready_message(ModelStatus::Error("disk full".to_string())),
            "Speech model download failed: disk full"
        );
    }
}
