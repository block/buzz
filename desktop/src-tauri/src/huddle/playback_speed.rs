//! Pitch-preserving playback speed for generated speech.
//!
//! This stage sits between Pocket synthesis and rodio playback. It is
//! deliberately independent of Pocket's model parameters, and it does not use
//! rodio's speed control because rodio changes pitch and speed together.

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;

#[path = "playback_speed_dsp.rs"]
mod playback_speed_dsp;
#[cfg(test)]
pub(crate) use playback_speed_dsp::process_complete_chunk;
pub(crate) use playback_speed_dsp::process_complete_playback_chunk;
use playback_speed_dsp::{validate_speed, DEFAULT_PLAYBACK_SPEED};

const SETTINGS_FILE: &str = "tts-playback-settings.json";

/// Lock-free shared control read by the TTS worker before each synthesis chunk.
#[derive(Clone, Debug)]
pub struct PlaybackSpeedControl {
    speed: Arc<AtomicU32>,
    transition: Arc<tokio::sync::Mutex<()>>,
}

impl Default for PlaybackSpeedControl {
    fn default() -> Self {
        Self {
            speed: Arc::new(AtomicU32::new(DEFAULT_PLAYBACK_SPEED.to_bits())),
            transition: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

impl PlaybackSpeedControl {
    /// Return the current generated-speech playback speed.
    pub fn get(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Acquire))
    }

    /// Update the in-memory speed after validation.
    pub fn set(&self, speed: f32) -> Result<(), String> {
        validate_speed(speed)?;
        self.speed.store(speed.to_bits(), Ordering::Release);
        Ok(())
    }

    async fn persist_to_path(&self, path: &Path, speed: f32) -> Result<(), String> {
        validate_speed(speed)?;
        let _transition = self.transition.lock().await;
        save_to_path(path, speed)?;
        self.speed.store(speed.to_bits(), Ordering::Release);
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct PersistedPlaybackSettings {
    speed: f32,
}

/// Load the global playback speed during app setup. Logs and keeps the 1x
/// default if no valid persisted setting exists.
pub fn load_playback_speed(app: &AppHandle, control: &PlaybackSpeedControl) {
    let result = settings_path(app)
        .and_then(|path| load_from_path(&path))
        .and_then(|speed| control.set(speed));
    if let Err(error) = result {
        eprintln!("buzz-desktop: failed to load TTS playback speed; using 1x: {error}");
    }
}

/// Return the globally configured generated-speech playback speed.
#[tauri::command]
pub fn get_tts_playback_speed(state: State<'_, AppState>) -> f32 {
    state.tts_playback_speed.get()
}

/// Persist and apply the global generated-speech playback speed.
#[tauri::command]
pub async fn set_tts_playback_speed(
    speed: f32,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .tts_playback_speed
        .persist_to_path(&settings_path(&app)?, speed)
        .await
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join(SETTINGS_FILE))
        .map_err(|error| format!("resolve TTS playback settings directory: {error}"))
}

fn load_from_path(path: &Path) -> Result<f32, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DEFAULT_PLAYBACK_SPEED);
        }
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    let settings: PersistedPlaybackSettings = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    validate_speed(settings.speed)?;
    Ok(settings.speed)
}

fn save_to_path(path: &Path, speed: f32) -> Result<(), String> {
    validate_speed(speed)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let payload = serde_json::to_vec_pretty(&PersistedPlaybackSettings { speed })
        .map_err(|error| format!("serialize TTS playback settings: {error}"))?;
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut file = AtomicWriteFile::open(&resolved)
        .map_err(|error| format!("open {} for atomic write: {error}", resolved.display()))?;
    file.write_all(&payload)
        .map_err(|error| format!("write {}: {error}", resolved.display()))?;
    file.commit()
        .map_err(|error| format!("commit {}: {error}", resolved.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 24_000;

    #[test]
    fn unity_processing_is_a_bit_exact_bypass() {
        let input = vec![0.0, 0.125, -0.5, 1.0];
        let output = process_complete_chunk(&input, 1.0, SAMPLE_RATE).expect("unity output");
        assert_eq!(output, input);
    }

    #[test]
    fn complete_processing_preserves_length_pitch_and_order() {
        let input: Vec<f32> = (0..24_000)
            .map(|sample| {
                let frequency = if sample < 8_000 {
                    220.0
                } else if sample < 16_000 {
                    440.0
                } else {
                    660.0
                };
                (2.0 * std::f32::consts::PI * frequency * sample as f32 / SAMPLE_RATE as f32).sin()
            })
            .collect();
        let output = process_complete_chunk(&input, 1.25, SAMPLE_RATE).expect("complete output");

        assert_eq!(output.len(), 19_200);
        let first_frequency = zero_crossing_frequency(&output[1_500..5_000], SAMPLE_RATE);
        let second_frequency = zero_crossing_frequency(&output[7_500..11_000], SAMPLE_RATE);
        let third_frequency = zero_crossing_frequency(&output[14_000..18_000], SAMPLE_RATE);
        assert!(
            (first_frequency - 220.0).abs() < 8.0,
            "first segment measured {first_frequency} Hz"
        );
        assert!(
            (second_frequency - 440.0).abs() <= 10.0,
            "second segment measured {second_frequency} Hz"
        );
        assert!(
            (third_frequency - 660.0).abs() <= 12.0,
            "third segment measured {third_frequency} Hz"
        );
    }

    #[test]
    fn processor_preserves_sine_pitch() {
        let frequency = 220.0_f32;
        let input: Vec<f32> = (0..SAMPLE_RATE * 2)
            .map(|sample| {
                (2.0 * std::f32::consts::PI * frequency * sample as f32 / SAMPLE_RATE as f32).sin()
            })
            .collect();
        let output = process_complete_chunk(&input, 1.5, SAMPLE_RATE).expect("complete output");
        assert!(
            root_mean_square(&output[..480]) > 0.2,
            "full reset pre-roll was not removed"
        );
        assert!(
            root_mean_square(&output[output.len() - 480..]) > 0.2,
            "speech tail was truncated"
        );
        let measured = zero_crossing_frequency(&output[2_000..], SAMPLE_RATE);
        assert!(
            (measured - frequency).abs() < 3.0,
            "expected {frequency} Hz, measured {measured} Hz"
        );
    }

    #[test]
    fn complete_processing_preserves_onset_timing() {
        let input: Vec<f32> = (0..SAMPLE_RATE)
            .map(|sample| {
                if (4_800..12_000).contains(&sample) || sample >= 16_800 {
                    (2.0 * std::f32::consts::PI * 220.0 * sample as f32 / SAMPLE_RATE as f32).sin()
                } else {
                    0.0
                }
            })
            .collect();
        let output = process_complete_chunk(&input, 1.25, SAMPLE_RATE).expect("complete output");
        let active_windows = active_window_starts(&output, 240, 0.15);

        let first_onset = *active_windows.first().expect("first tone onset");
        let second_onset = active_windows
            .windows(2)
            .find_map(|pair| (pair[1] > pair[0] + 480).then_some(pair[1]))
            .expect("second tone onset");
        assert!(
            (3_200..=4_400).contains(&first_onset),
            "first onset shifted to sample {first_onset}"
        );
        assert!(
            (12_800..=14_000).contains(&second_onset),
            "second onset shifted to sample {second_onset}"
        );
    }

    #[test]
    fn non_unity_latency_stays_below_75_ms() {
        let mut stretch = ssstretch::Stretch::new();
        stretch.preset_default(1, SAMPLE_RATE as f32);
        let latency_ms = stretch.output_latency().max(0) as f64 * 1_000.0 / SAMPLE_RATE as f64;
        assert!(latency_ms <= 75.0, "algorithmic latency was {latency_ms}ms");
    }

    #[test]
    fn persisted_speed_round_trips_and_rejects_invalid_values() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join(SETTINGS_FILE);
        save_to_path(&path, playback_speed_dsp::MIN_PLAYBACK_SPEED).expect("save minimum");
        assert_eq!(
            load_from_path(&path).expect("load minimum"),
            playback_speed_dsp::MIN_PLAYBACK_SPEED
        );
        save_to_path(&path, playback_speed_dsp::MAX_PLAYBACK_SPEED).expect("save maximum");
        assert_eq!(
            load_from_path(&path).expect("load maximum"),
            playback_speed_dsp::MAX_PLAYBACK_SPEED
        );

        std::fs::write(&path, br#"{"speed":4.1}"#).expect("invalid fixture");
        assert!(load_from_path(&path).is_err());
    }

    #[tokio::test]
    async fn serialized_persistence_finishes_with_the_latest_speed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join(SETTINGS_FILE);
        let control = PlaybackSpeedControl::default();
        let transition = control.transition.lock().await;

        let (first_enqueued_tx, first_enqueued_rx) = tokio::sync::oneshot::channel();
        let first_control = control.clone();
        let first_path = path.clone();
        let first = tokio::spawn(async move {
            first_enqueued_tx.send(()).expect("signal first waiter");
            first_control.persist_to_path(&first_path, 1.25).await
        });
        first_enqueued_rx.await.expect("first waiter started");
        tokio::task::yield_now().await;

        let (second_enqueued_tx, second_enqueued_rx) = tokio::sync::oneshot::channel();
        let second_control = control.clone();
        let second_path = path.clone();
        let second = tokio::spawn(async move {
            second_enqueued_tx.send(()).expect("signal second waiter");
            second_control.persist_to_path(&second_path, 1.5).await
        });
        second_enqueued_rx.await.expect("second waiter started");
        tokio::task::yield_now().await;

        drop(transition);
        first.await.expect("join first save").expect("first save");
        second
            .await
            .expect("join second save")
            .expect("second save");

        assert_eq!(load_from_path(&path).expect("load final speed"), 1.5);
        assert_eq!(control.get(), 1.5);
    }

    fn zero_crossing_frequency(samples: &[f32], sample_rate: u32) -> f32 {
        let crossings = samples
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        crossings as f32 * sample_rate as f32 / samples.len() as f32
    }

    fn root_mean_square(samples: &[f32]) -> f32 {
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn active_window_starts(samples: &[f32], window: usize, threshold: f32) -> Vec<usize> {
        samples
            .chunks_exact(window)
            .enumerate()
            .filter_map(|(index, chunk)| {
                (root_mean_square(chunk) > threshold).then_some(index * window)
            })
            .collect()
    }
}
