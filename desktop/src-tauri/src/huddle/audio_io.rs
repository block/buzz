//! Pluggable huddle audio capture (`AudioSource`) and playback (`AudioSink`).
//!
//! Today huddle audio has exactly one capture path and one playback path,
//! both hardwired:
//!   - Capture: the webview's `getUserMedia` + AudioWorklet posts raw PCM
//!     bytes to the `push_audio_pcm` Tauri command, which calls
//!     `SttPipeline::push_audio` directly (see `mod.rs`).
//!   - Playback: the TTS worker thread builds its own `rodio::Player` from
//!     `audio_output::open_output_sink_by_name` (see `tts.rs`).
//!
//! Neither of those call sites changes here. These two traits describe the
//! same frame shape as a seam so an alternative capture/playback backend can
//! be installed instead — the immediate, demonstrated use is driving the
//! resample → VAD → recognize path in `stt.rs` from a Rust test with no
//! webview involved (see the tests below).
//!
//! `AppState::huddle_audio_source` / `huddle_audio_sink` are the installation
//! points. Neither is consulted by `push_audio_pcm` or the TTS worker in this
//! change — wiring a live installed backend into those call sites needs its
//! own polling/shutdown lifecycle tied to huddle teardown, which is a bigger
//! change than this seam. Leaving that out keeps default behaviour exactly
//! as it is today: with nothing installed, both slots are inert.
//!
//! Not yet wired to a production call site — exercised by this module's own
//! tests (`pump_audio_source`, `RodioAudioSink`) and by `AppState`'s
//! `huddle_audio_source` / `huddle_audio_sink` slots.
#![allow(dead_code)]

use std::num::NonZero;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

/// Captured frames, 48 kHz f32 mono — the format `push_audio_pcm` already receives.
pub trait AudioSource: Send {
    fn try_recv(&mut self) -> Option<Vec<f32>>;
}

/// Synthesized audio for playback.
pub trait AudioSink: Send {
    fn push(&mut self, samples: &[f32]);
}

// ── Default AudioSource: the webview capture path ──────────────────────────────

/// Default `AudioSource`. Represents the frame shape of today's only capture
/// path — the webview's AudioWorklet, arriving over Tauri IPC.
///
/// `push_audio_pcm` does not construct one of these; it keeps calling
/// `SttPipeline::push_audio` directly, unchanged. This type exists so that
/// same frame shape can be pulled by something other than a Tauri command —
/// a test, or a future native capture thread — via [`WebviewAudioSourceHandle::send`].
pub struct WebviewAudioSource {
    rx: Receiver<Vec<f32>>,
}

/// The write side of a [`WebviewAudioSource`].
#[derive(Clone)]
pub struct WebviewAudioSourceHandle {
    tx: Sender<Vec<f32>>,
}

impl WebviewAudioSource {
    /// Create a linked handle/source pair.
    pub fn channel() -> (WebviewAudioSourceHandle, Self) {
        let (tx, rx) = mpsc::channel();
        (WebviewAudioSourceHandle { tx }, Self { rx })
    }
}

impl WebviewAudioSourceHandle {
    pub fn send(&self, frame: Vec<f32>) -> Result<(), mpsc::SendError<Vec<f32>>> {
        self.tx.send(frame)
    }
}

impl AudioSource for WebviewAudioSource {
    fn try_recv(&mut self) -> Option<Vec<f32>> {
        match self.rx.try_recv() {
            Ok(frame) => Some(frame),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => None,
        }
    }
}

// ── Default AudioSink: the rodio playback path ──────────────────────────────────

/// Default `AudioSink`. Wraps a `rodio::Player` exactly as the TTS worker
/// builds one today (see `tts_worker` in `tts.rs`, around the
/// `Player::connect_new` call).
pub struct RodioAudioSink {
    player: Arc<rodio::Player>,
    channels: NonZero<u16>,
    rate: NonZero<u32>,
}

impl RodioAudioSink {
    pub fn new(player: Arc<rodio::Player>, channels: NonZero<u16>, rate: NonZero<u32>) -> Self {
        Self {
            player,
            channels,
            rate,
        }
    }
}

impl AudioSink for RodioAudioSink {
    fn push(&mut self, samples: &[f32]) {
        self.player.append(rodio::buffer::SamplesBuffer::new(
            self.channels,
            self.rate,
            samples.to_vec(),
        ));
    }
}

// ── Wiring: drive any AudioSource into a byte-based sink ────────────────────────

/// Pump every frame currently available from `source` into `sink`, encoding
/// each frame as little-endian f32 bytes — the same wire layout
/// `push_audio_pcm` already hands to `SttPipeline::push_audio`. Returns once
/// `source.try_recv()` returns `None`.
///
/// `sink` is generic rather than a concrete `&SttPipeline` so it can be a
/// closure over a real pipeline in production/tests, or a plain recorder in
/// a unit test that has no model on disk. Going through the same byte
/// encoding `push_audio_pcm` uses means this can never diverge from the STT
/// gating logic that lives downstream of that call.
pub fn pump_audio_source(
    source: &mut dyn AudioSource,
    mut sink: impl FnMut(Vec<u8>) -> Result<(), String>,
) {
    while let Some(frame) = source.try_recv() {
        let bytes: Vec<u8> = frame.iter().flat_map(|s| s.to_le_bytes()).collect();
        if let Err(e) = sink(bytes) {
            eprintln!("buzz-desktop: pump_audio_source: sink rejected frame: {e}");
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::time::Duration;

    /// A finite, deterministic `AudioSource` backed by a queue of frames.
    /// Used by every test below — a WAV split into fixed-size chunks, or a
    /// handful of hand-written frames.
    struct FrameQueueSource(VecDeque<Vec<f32>>);

    impl FrameQueueSource {
        fn new(frames: impl IntoIterator<Item = Vec<f32>>) -> Self {
            Self(frames.into_iter().collect())
        }

        fn chunks(samples: &[f32], frame_len: usize) -> Self {
            Self(samples.chunks(frame_len).map(|c| c.to_vec()).collect())
        }
    }

    impl AudioSource for FrameQueueSource {
        fn try_recv(&mut self) -> Option<Vec<f32>> {
            self.0.pop_front()
        }
    }

    /// Proves the seam itself: frames in via `AudioSource::try_recv`, frames
    /// delivered to the sink in order, correctly byte-encoded — with no STT
    /// model and no `SttPipeline` involved at all.
    #[test]
    fn pump_audio_source_delivers_every_frame_in_order() {
        let frames = vec![vec![0.1_f32, 0.2, 0.3], vec![-0.4_f32, -0.5], vec![1.0_f32]];
        let mut source = FrameQueueSource::new(frames.clone());

        let mut delivered: Vec<Vec<u8>> = Vec::new();
        pump_audio_source(&mut source, |bytes| {
            delivered.push(bytes);
            Ok(())
        });

        assert_eq!(delivered.len(), frames.len());
        for (frame, bytes) in frames.iter().zip(delivered.iter()) {
            let expected: Vec<u8> = frame.iter().flat_map(|s| s.to_le_bytes()).collect();
            assert_eq!(bytes, &expected);
        }
        // Source is exhausted — a second pump delivers nothing further.
        let mut second_pass = 0;
        pump_audio_source(&mut source, |_| {
            second_pass += 1;
            Ok(())
        });
        assert_eq!(second_pass, 0);
    }

    /// `RodioAudioSink::push` forwards to the wrapped `Player`. Uses an
    /// in-memory `rodio::mixer` (no real audio device), the same device-free
    /// pattern as `tests/rodio_mixer_diagnostic.rs`.
    #[test]
    fn rodio_audio_sink_forwards_pushed_samples_to_the_player() {
        let channels = NonZero::new(1u16).expect("channel count");
        let rate = NonZero::new(48_000u32).expect("sample rate");
        let (mixer_in, mixer_out) = rodio::mixer::mixer(channels, rate);
        let player = Arc::new(rodio::Player::connect_new(&mixer_in));
        let mut sink = RodioAudioSink::new(Arc::clone(&player), channels, rate);

        sink.push(&[1.0, -1.0, 0.5]);

        let drained: Vec<f32> = mixer_out.take(3).collect();
        assert_eq!(drained, vec![1.0, -1.0, 0.5]);
    }

    // ── Full pipeline: WAV → AudioSource → real SttPipeline ─────────────────

    /// Minimal RIFF/WAVE reader for 16-bit PCM mono fixtures. Returns
    /// (sample_rate, samples normalized to [-1.0, 1.0]).
    fn read_wav_pcm16_mono(path: &std::path::Path) -> (u32, Vec<f32>) {
        let bytes = std::fs::read(path).expect("read wav fixture");
        assert_eq!(&bytes[0..4], b"RIFF", "not a RIFF file");
        assert_eq!(&bytes[8..12], b"WAVE", "not a WAVE file");

        let mut pos = 12;
        let mut sample_rate = 0u32;
        let mut data: &[u8] = &[];
        while pos + 8 <= bytes.len() {
            let chunk_id = &bytes[pos..pos + 4];
            let chunk_len =
                u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().expect("chunk len")) as usize;
            let chunk_start = pos + 8;
            if chunk_id == b"fmt " {
                sample_rate = u32::from_le_bytes(
                    bytes[chunk_start + 4..chunk_start + 8]
                        .try_into()
                        .expect("sample rate field"),
                );
            } else if chunk_id == b"data" {
                data = &bytes[chunk_start..chunk_start + chunk_len];
            }
            // RIFF chunks are word-aligned; an odd-length chunk has a pad byte.
            pos = chunk_start + chunk_len + (chunk_len % 2);
        }
        assert!(sample_rate > 0, "no fmt chunk found");

        let samples = data
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32)
            .collect();
        (sample_rate, samples)
    }

    /// Linear-interpolation resample. Coarse — good enough to feed a VAD +
    /// recognizer that only needs correctly paced 48 kHz input, not
    /// broadcast-quality resampling.
    fn resample_linear(input: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
        if input.is_empty() || from_rate == to_rate {
            return input.to_vec();
        }
        let ratio = to_rate as f64 / from_rate as f64;
        let out_len = (input.len() as f64 * ratio) as usize;
        (0..out_len)
            .map(|i| {
                let src_pos = i as f64 / ratio;
                let idx = src_pos.floor() as usize;
                let frac = (src_pos - idx as f64) as f32;
                let a = input[idx.min(input.len() - 1)];
                let b = input[(idx + 1).min(input.len() - 1)];
                a + (b - a) * frac
            })
            .collect()
    }

    /// Drives the real STT pipeline — resample → VAD → Parakeet — from a WAV
    /// fixture through `AudioSource`/`pump_audio_source`, with no webview.
    ///
    /// Needs the Parakeet model on disk (`~/.buzz/models/parakeet-tdt-ctc-110m-en/`),
    /// which a fresh checkout does not have. Ignored so CI stays green; run
    /// locally after `download_voice_models` has completed once with:
    ///   cargo test --package buzz-desktop wav_through_audio_source -- --ignored
    #[test]
    #[ignore = "requires the Parakeet STT model on disk — run download_voice_models first"]
    fn wav_through_audio_source_produces_a_transcript() {
        let model_dir = crate::huddle::models::stt_model_dir()
            .expect("STT model not downloaded — launch the app once or call download_voice_models");

        // A Pocket TTS voice-cloning reference clip — real spoken English,
        // already shipped with the app, so no new binary fixture is needed.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources/pocket-voices/eve.wav");
        let (source_rate, samples) = read_wav_pcm16_mono(&fixture);
        let samples_48k = resample_linear(&samples, source_rate, 48_000);
        // ~100 ms frames, matching the AudioWorklet batch size push_audio_pcm expects.
        // The STT worker only decodes after ~300 ms of trailing silence (see
        // SILENCE_FLUSH_FRAMES in stt.rs). Real capture supplies that silence
        // naturally when the speaker stops; a finite WAV does not, so append
        // one second of it or the utterance is never flushed and this test
        // waits forever for a transcript that cannot arrive.
        let mut samples_48k = samples_48k;
        samples_48k.resize(samples_48k.len() + 48_000, 0.0);
        let mut source = FrameQueueSource::chunks(&samples_48k, 4_800);

        let tts_active = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (pipeline, mut text_rx) =
            crate::huddle::stt::SttPipeline::new(model_dir, tts_active, None, None)
                .expect("spawn STT pipeline");

        // SttPipeline::push_audio drops frames when its bounded queue is full
        // (deliberately — better to lose audio than stall the UI thread), so
        // feeding a whole file as fast as the loop can run would discard most
        // of it. Pace the pump so the worker can drain; 10 ms per 100 ms frame
        // is ten times real time and still finishes quickly.
        while let Some(frame) = source.try_recv() {
            let mut bytes = Vec::with_capacity(frame.len() * 4);
            for sample in &frame {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
            let _ = pipeline.push_audio(bytes);
            std::thread::sleep(Duration::from_millis(10));
        }

        let transcript = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build tokio runtime")
            .block_on(async { tokio::time::timeout(Duration::from_secs(15), text_rx.recv()).await })
            .expect("STT pipeline produced no transcript within 15s")
            .expect("text channel closed before producing a transcript");

        pipeline.shutdown();

        assert!(
            !transcript.trim().is_empty(),
            "expected a non-empty transcript from real speech audio"
        );
    }
}
