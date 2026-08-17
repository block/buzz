use super::*;
use crate::huddle::pocket::{normalize_german_text, KokoroGerman};
use crate::huddle::preprocessing::preprocess_for_tts;
use crate::huddle::speech_profile::german_voice_id;

pub(super) fn maybe_run_kokoro_worker(
    model_dir: PathBuf,
    voice_state: &WorkerVoiceState,
    text_rx: &mpsc::Receiver<QueuedText>,
    control_state: &WorkerControlState,
    output_device: Option<String>,
    startup_tx: &mpsc::SyncSender<Result<(), String>>,
) -> bool {
    let requested = voice_state
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    if !requested.starts_with("kokoro:") {
        return false;
    }

    let engine = match KokoroGerman::load(&model_dir) {
        Ok(engine) => engine,
        Err(error) => {
            let _ = startup_tx.send(Err(format!(
                "German TTS is unavailable. Download Kokoro models first. {error}"
            )));
            return true;
        }
    };

    let sink_handle = match super::super::audio_output::open_output_sink_by_name(output_device.as_deref())
    {
        Ok(handle) => handle,
        Err(error) => {
            let _ = startup_tx.send(Err(format!("TTS audio output initialization failed: {error}")));
            return true;
        }
    };
    let Some(channels) = NonZero::new(1u16) else {
        let _ = startup_tx.send(Err("TTS channel count invariant violated".into()));
        return true;
    };
    let Some(rate) = NonZero::new(24_000) else {
        let _ = startup_tx.send(Err("TTS sample rate invariant violated".into()));
        return true;
    };
    let player = rodio::Player::connect_new(sink_handle.mixer());
    let _ = startup_tx.send(Ok(()));

    let (tts_active, shutdown, cancel_signals, _, _, _, _) = control_state;
    let (cancel, voice_cancel) = cancel_signals;
    let mut voice = requested;
    loop {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        if cancel.load(Ordering::Acquire) || voice_cancel.load(Ordering::Acquire) {
            player.clear();
            tts_active.store(false, Ordering::Release);
            cancel.store(false, Ordering::Release);
            voice_cancel.store(false, Ordering::Release);
        }
        let item = match text_rx.recv_timeout(RECV_TIMEOUT) {
            Ok(item) => item,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if player.empty() {
                    tts_active.store(false, Ordering::Release);
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        if let Some(next) = item.voice_reference {
            voice = next;
        }
        let prepared = preprocess_for_tts(&item.text);
        let prepared = if prepared.is_empty() {
            continue;
        } else {
            normalize_german_text(&prepared)
        };
        tts_active.store(true, Ordering::Release);
        match engine.synthesize(&prepared, german_voice_id(&voice), 1.0) {
            Ok((samples, sample_rate)) if !samples.is_empty() => {
                let rate = NonZero::new(sample_rate.max(1)).unwrap_or(rate);
                player.append(rodio::buffer::SamplesBuffer::new(
                    channels,
                    rate,
                    samples,
                ));
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("buzz-desktop: Kokoro synthesis failed: {error}");
                tts_active.store(false, Ordering::Release);
            }
        }
    }
    true
}
