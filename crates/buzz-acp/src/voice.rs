//! Local speech-to-text for voice notes arriving in `buzz-acp`.
//!
//! A Buzz voice note is a `video/mp4` whose video track is a 16x16 black still
//! and whose audio track carries the speech: the relay's media allowlist has
//! no audio entry, so clients wrap speech in the canonical video envelope.
//! The `imeta` tag records the blob's URL, its declared MIME (`video/mp4`), its
//! SHA-256, its size, and its dimensions (`dim 16x16`).
//!
//! An agent that sees only the event sees a URL, not the words. This module
//! resolves that URL, downloads the blob with the relay's NIP-98 auth, decodes
//! the audio track, and produces a transcript that the prompt can carry.
//!
//! Everything here is best-effort: a voice note whose blob cannot be fetched,
//! decoded, or transcribed still reaches the agent as an ordinary attachment.
//! A missing speech model must never cost the agent the message.

use std::path::PathBuf;

use buzz_voice::MAX_TRANSCRIBE_SECONDS;
use nostr::Event;

/// The MIME the relay reports for a packaged voice note.
const VOICE_NOTE_MIME: &str = "video/mp4";

/// The video dimensions of a Buzz voice note's still track.
///
/// A voice note's video track is a 16x16 black still, so the relay records
/// `dim 16x16`. This is the reliable voice-note signal: Buzz attachments
/// carry no `filename` field at all.
const VOICE_NOTE_DIMENSIONS: &str = "16x16";

/// A voice note's media reference, resolved from an event's `imeta` tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VoiceNoteMedia {
    /// The relay's Blossom URL for the blob.
    pub url: String,
    /// The blob's SHA-256, as declared by the relay.
    pub sha256: String,
    /// The blob's size in bytes, as declared by the relay.
    pub size: u64,
}

/// Extract the voice-note media reference from an event, if it has one.
///
/// Returns `None` for an event with no `imeta` tag, and for an `imeta` that
/// does not describe a voice note (an ordinary image or video attachment).
pub fn voice_note_media(event: &Event) -> Option<VoiceNoteMedia> {
    for tag in event.tags.iter() {
        let tag_slice = tag.as_slice();
        if tag_slice.first().map(String::as_str) != Some("imeta") {
            continue;
        }
        let mut url = None;
        let mut sha256 = None;
        let mut size = None;
        let mut mime = None;
        let mut dimensions = None;

        // imeta is a single tag whose remaining values are `key value` pairs
        // packed into one string per pair (`"url https://…"`, `"m video/mp4"`).
        for value in tag_slice.iter().skip(1) {
            let Some((key, val)) = value.split_once(' ') else {
                continue;
            };
            match key {
                "url" => url = Some(val.to_string()),
                "x" => sha256 = Some(val.to_string()),
                "size" => size = val.parse::<u64>().ok(),
                "m" => mime = Some(val.to_string()),
                "dim" => dimensions = Some(val.to_string()),
                _ => {}
            }
        }

        let is_voice_note = mime.as_deref() == Some(VOICE_NOTE_MIME)
            && dimensions
                .as_deref()
                .is_some_and(|dim| dim.eq_ignore_ascii_case(VOICE_NOTE_DIMENSIONS));
        if !is_voice_note {
            continue;
        }

        return Some(VoiceNoteMedia {
            url: url?,
            sha256: sha256?,
            size: size?,
        });
    }
    None
}

/// Where the local speech model lives, and how to load it.
///
/// The ACP harness resolves this from the environment so an operator can
/// point it at a model directory without recompiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriberConfig {
    /// Directory holding the speech model files.
    pub model_dir: PathBuf,
    /// The model family.
    pub model_type: buzz_voice::ModelType,
}

impl TranscriberConfig {
    /// Resolve the transcriber config from the environment.
    ///
    /// Returns `None` when no model directory is configured — the caller then
    /// skips transcription entirely rather than failing the turn.
    pub fn from_env() -> Option<Self> {
        let model_dir = std::env::var_os("BUZZ_VOICE_MODEL_DIR")
            .map(PathBuf::from)
            .filter(|dir| !dir.as_os_str().is_empty())?;

        let model_type = model_type_from_env(
            std::env::var("BUZZ_VOICE_MODEL_TYPE")
                .unwrap_or_default()
                .as_str(),
        );

        Some(Self {
            model_dir,
            model_type,
        })
    }

    /// The `buzz-voice` config for this transcriber.
    fn voice_config(&self) -> buzz_voice::TranscribeConfig {
        buzz_voice::TranscribeConfig {
            model_dir: self.model_dir.clone(),
            model_type: self.model_type,
            lm: None,
        }
    }
}

/// Map a `BUZZ_VOICE_MODEL_TYPE` value to a model family.
///
/// Parakeet TDT-CTC is the default: it is the layout the Buzz desktop ships
/// (`parakeet-tdt-ctc-110m-en`, a single `model.int8.onnx` + `tokens.txt`), so
/// the same model directory works on both sides without a second download. It
/// also decodes with CTC blank tokens, which removes the silence/cut-audio
/// hallucination class that hurts encoder-decoder models on short notes.
///
/// The older TDT triple (encoder/decoder/joiner) is still selectable as
/// "parakeet" for a 0.6b release; it is not the default because it cannot load
/// the 110M CTC model.
fn model_type_from_env(raw: &str) -> buzz_voice::ModelType {
    match raw.to_ascii_lowercase().as_str() {
        "paraformer" => buzz_voice::ModelType::Paraformer,
        "transducer" | "zipformer" => buzz_voice::ModelType::Transducer,
        "whisper" => buzz_voice::ModelType::Whisper,
        "parakeet" | "parakeet_tdt" => buzz_voice::ModelType::Parakeet,
        _ => buzz_voice::ModelType::ParakeetCtc,
    }
}

/// Transcribe raw media bytes into text.
///
/// `bytes` are the blob as stored by the relay — for a voice note, a
/// `video/mp4` whose audio track carries the speech.
///
/// Returns `None` when the bytes cannot be decoded or the model is missing;
/// the caller treats that as "no transcript" rather than as an error. Every
/// failure logs, so a transcript that never appears is diagnosable.
pub fn transcribe_bytes(bytes: &[u8], config: &TranscriberConfig) -> Option<String> {
    let samples = decode_voice_note(bytes)?;
    let text = match buzz_voice::transcribe(&samples.0, samples.1, &config.voice_config()) {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!(
                target: "voice",
                model_dir = %config.model_dir.display(),
                error = %error,
                "could not transcribe the voice note; skipping the transcript"
            );
            return None;
        }
    };
    if text.is_empty() {
        // An empty transcript is a NORMAL outcome: the recognizer heard no
        // speech. `buzz_voice` documents it as such — a silent or non-speech
        // note. Logging this at WARN would fill the log with warnings for
        // people who tap record and say nothing.
        tracing::debug!(
            target: "voice",
            model_dir = %config.model_dir.display(),
            "the voice note contained no speech; skipping the transcript"
        );
        None
    } else {
        Some(text)
    }
}

/// Resolve the transcript for a voice-note event, if there is one.
///
/// Returns `None` when the event is not a voice note, when no speech model is
/// configured, or when the blob cannot be fetched or transcribed. Every
/// failure is silent by design: a voice note whose words cannot be recovered
/// still reaches the agent as an ordinary attachment.
pub async fn resolve_transcript(
    event: &Event,
    rest_client: &crate::relay::RestClient,
) -> Option<String> {
    let media = voice_note_media(event)?;
    let config = TranscriberConfig::from_env()?;

    let bytes = match rest_client.download_media(&media.url).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                target: "voice",
                url = %media.url,
                error = %error,
                "could not download the voice note blob; skipping transcription"
            );
            return None;
        }
    };
    if bytes.len() as u64 != media.size {
        // The blob does not match what the relay declared. Transcribing it
        // anyway would be acting on bytes the relay did not vouch for.
        tracing::warn!(
            target: "voice",
            expected = media.size,
            actual = bytes.len(),
            "voice note blob size mismatch; skipping transcription"
        );
        return None;
    }

    transcribe_bytes(&bytes, &config)
}

/// Stage `bytes` at `path` with 0600 permissions set at creation time, so
/// there is no window where the note is world-readable.
///
/// `create_new` makes the creation exclusive, so the name cannot be
/// pre-planted by another process.
///
/// Non-Unix: a plain write. The workspace's `windows-rust` CI job compiles
/// this crate for `x86_64-pc-windows-msvc`, and `OpenOptionsExt` does not
/// exist there — so the unix import must stay inside a gated function rather
/// than at module scope.
#[cfg(unix)]
fn stage_voice_note(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn stage_voice_note(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// Decode the audio track of a canonical Buzz voice note into mono samples.
///
/// The file is a `video/mp4`; `ffmpeg` reads the AAC track out of the
/// container. Decoding to 16 kHz mono here means the recognizer never has to
/// resample a 24 kHz voice note, which is where the resampling artifacts
/// that cost accuracy come from.
///
/// Every failure path logs. Transcription is best-effort by design — a note
/// the agent cannot hear still reaches it as an ordinary attachment — but a
/// silent `None` here is indistinguishable from "no speech model configured",
/// so a missing `ffmpeg` or a failed decode would be invisible in production.
fn decode_voice_note(bytes: &[u8]) -> Option<(Vec<f32>, u32)> {
    const SAMPLE_RATE: u32 = 16_000;

    // `ffmpeg` reads from a path, so the bytes have to land on disk first.
    //
    // The file holds a private voice note, so it is created with owner-only
    // permissions rather than the default umask-derived ones. `create_new`
    // makes the creation exclusive, so the name cannot be pre-planted.
    let temp_path = std::env::temp_dir().join(format!(
        "buzz-voice-note-{}-{}.mp4",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    let write_result = stage_voice_note(&temp_path, bytes);
    if let Err(error) = write_result {
        tracing::warn!(
            target: "voice",
            path = %temp_path.display(),
            error = %error,
            "could not stage the voice note for decoding; skipping transcription"
        );
        let _ = std::fs::remove_file(&temp_path);
        return None;
    }

    // `-t` bounds the decode. Without it ffmpeg decodes the WHOLE file into
    // `output.stdout` before `buzz_voice::transcribe` applies
    // `MAX_TRANSCRIBE_SECONDS` — so a long audio track is fully materialised
    // in memory before the cap rejects it. The relay's media validation caps
    // the VIDEO track's duration but not the audio track's, so an ordinary
    // inbound attachment can carry hours of AAC. The recognizer's cap stays
    // as defence in depth; this bounds the memory the decode itself costs.
    //
    // The bound is one second PAST the cap, not at it: ffmpeg's `-t` is
    // inclusive, so `-t 120` on a long file yields exactly the cap's worth of
    // samples and the recognizer's `samples.len() > max_samples` check would
    // no longer fire — an over-long note would silently transcribe as empty
    // instead of being rejected. Decoding one second past the cap keeps the
    // rejection intact while still bounding the buffer to ~7.7 MB.
    let decode_limit = MAX_TRANSCRIBE_SECONDS + 1.0;
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&temp_path)
        .args(["-t", &decode_limit.to_string()])
        .args(["-f", "f32le", "-ar"])
        .arg(SAMPLE_RATE.to_string())
        .args(["-ac", "1", "pipe:1"])
        .output();
    let _ = std::fs::remove_file(&temp_path);

    let output = match output {
        Ok(output) => output,
        Err(error) => {
            // Most often a missing `ffmpeg` binary. Say so, loudly enough to
            // find, but do not fail the turn — the attachment still arrives.
            tracing::warn!(
                target: "voice",
                error = %error,
                "could not run ffmpeg to decode the voice note; skipping transcription"
            );
            return None;
        }
    };
    if !output.status.success() || output.stdout.len() < 4 {
        tracing::warn!(
            target: "voice",
            status = ?output.status,
            stdout_bytes = output.stdout.len(),
            "ffmpeg did not produce usable audio for the voice note; skipping transcription"
        );
        return None;
    }
    let samples: Vec<f32> = output
        .stdout
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    if samples.is_empty() {
        tracing::warn!(
            target: "voice",
            "the voice note decoded to no audio samples; skipping transcription"
        );
        return None;
    }
    Some((samples, SAMPLE_RATE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};

    fn signed_event_with_imeta(values: &[&str]) -> Event {
        let mut builder = EventBuilder::new(Kind::TextNote, "hello");
        builder = builder.tag(Tag::parse(["h", "channel"]).expect("tag"));
        let mut tag_values: Vec<&str> = vec!["imeta"];
        tag_values.extend_from_slice(values);
        builder = builder.tag(Tag::parse(tag_values).expect("imeta tag"));
        builder
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign")
    }

    #[test]
    fn a_voice_note_imeta_is_recognized() {
        // The imeta tag is ONE tag carrying `key value` pairs.
        let signed = signed_event_with_imeta(&[
            "url https://relay.example/media/abc.mp4",
            "m video/mp4",
            "x abc123",
            "size 1234",
            "dim 16x16",
        ]);

        let media = voice_note_media(&signed).expect("voice note must be recognized");
        assert_eq!(media.url, "https://relay.example/media/abc.mp4");
        assert_eq!(media.sha256, "abc123");
        assert_eq!(media.size, 1234);
    }

    #[test]
    fn an_ordinary_video_attachment_is_not_a_voice_note() {
        let signed = signed_event_with_imeta(&[
            "url https://relay.example/media/clip.mp4",
            "m video/mp4",
            "x abc123",
            "size 1234",
            "dim 1920x1080",
        ]);
        assert!(voice_note_media(&signed).is_none());
    }

    #[test]
    fn an_event_without_imeta_is_not_a_voice_note() {
        let builder = EventBuilder::new(Kind::TextNote, "hello");
        let signed = builder
            .sign_with_keys(&nostr::Keys::generate())
            .expect("sign");
        assert!(voice_note_media(&signed).is_none());
    }

    #[test]
    fn the_model_type_env_var_selects_the_family() {
        let previous_dir = std::env::var_os("BUZZ_VOICE_MODEL_DIR");
        let previous_type = std::env::var_os("BUZZ_VOICE_MODEL_TYPE");
        std::env::set_var("BUZZ_VOICE_MODEL_DIR", "/tmp/model");

        for (name, expected) in [
            ("whisper", buzz_voice::ModelType::Whisper),
            ("paraformer", buzz_voice::ModelType::Paraformer),
            ("transducer", buzz_voice::ModelType::Transducer),
            ("zipformer", buzz_voice::ModelType::Transducer),
            ("parakeet", buzz_voice::ModelType::Parakeet),
            ("parakeet_tdt", buzz_voice::ModelType::Parakeet),
            ("parakeet_ctc", buzz_voice::ModelType::ParakeetCtc),
            ("ctc", buzz_voice::ModelType::ParakeetCtc),
        ] {
            std::env::set_var("BUZZ_VOICE_MODEL_TYPE", name);
            let config = TranscriberConfig::from_env().expect("model dir is set");
            assert_eq!(config.model_type, expected, "BUZZ_VOICE_MODEL_TYPE={name}");
        }

        // Unset means the measured default.
        std::env::remove_var("BUZZ_VOICE_MODEL_TYPE");
        let config = TranscriberConfig::from_env().expect("model dir is set");
        assert_eq!(config.model_type, buzz_voice::ModelType::ParakeetCtc);

        if let Some(value) = previous_dir {
            std::env::set_var("BUZZ_VOICE_MODEL_DIR", value);
        } else {
            std::env::remove_var("BUZZ_VOICE_MODEL_DIR");
        }
        if let Some(value) = previous_type {
            std::env::set_var("BUZZ_VOICE_MODEL_TYPE", value);
        } else {
            std::env::remove_var("BUZZ_VOICE_MODEL_TYPE");
        }
    }

    /// A decode failure must not be silent.
    ///
    /// Transcription is best-effort by design — a note the agent cannot hear
    /// still reaches it as an ordinary attachment — but a swallowed failure
    /// is indistinguishable from "no speech model configured". A missing
    /// `ffmpeg` is the common case, and it must be visible in the log.
    ///
    /// This binds to the production seam: it captures what the real
    /// `decode_voice_note` emits through `tracing`, not a test-only helper.
    #[test]
    fn a_decode_failure_is_logged_not_swallowed() {
        use std::sync::{Arc, Mutex};

        /// A `MakeWriter` that keeps every line it is handed.
        #[derive(Clone, Default)]
        struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

        impl std::io::Write for CaptureWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0
                    .lock()
                    .expect("capture buffer poisoned")
                    .extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
            type Writer = CaptureWriter;
            fn make_writer(&'a self) -> Self::Writer {
                self.clone()
            }
        }

        let captured = CaptureWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_max_level(tracing::Level::WARN)
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);

        // Bytes that are not a decodable container: ffmpeg exits non-zero.
        let result = decode_voice_note(b"\x00\x01\x02\x03not a voice note");
        drop(guard);

        assert!(
            result.is_none(),
            "undecodable bytes must not produce samples"
        );

        let logged = String::from_utf8(captured.0.lock().expect("capture poisoned").clone())
            .expect("captured log must be utf-8");
        // Two different failures reach this point depending on the box:
        // a missing `ffmpeg` binary fails earlier ("could not run ffmpeg"),
        // and a present ffmpeg that cannot decode the bytes fails here
        // ("did not produce usable audio"). Either satisfies the property
        // this test protects — the failure is LOGGED, not swallowed — so
        // accept both.
        assert!(
            logged.contains("could not run ffmpeg to decode the voice note")
                || logged.contains("ffmpeg did not produce usable audio"),
            "a decode failure must be logged, got: {logged}"
        );
    }

    /// End-to-end proof against a REAL canonical voice note and a REAL model.
    ///
    /// This binds to the production path (`decode_voice_note` +
    /// `buzz_voice::transcribe`) — the same code the `Transcript:` injection
    /// depends on. It needs two environment variables pointing at real files;
    /// without them it is skipped rather than failing, so the suite still runs
    /// on a machine that has no speech model staged.
    #[test]
    fn a_real_voice_note_transcribes_through_the_production_path() {
        let Ok(model_dir) = std::env::var("BUZZ_TEST_VOICE_MODEL_DIR") else {
            eprintln!("skipped: BUZZ_TEST_VOICE_MODEL_DIR not set");
            return;
        };
        let Ok(note_path) = std::env::var("BUZZ_TEST_VOICE_NOTE") else {
            eprintln!("skipped: BUZZ_TEST_VOICE_NOTE not set");
            return;
        };
        let model_dir = std::path::PathBuf::from(model_dir);
        if !model_dir.is_dir() {
            eprintln!("skipped: model dir {} does not exist", model_dir.display());
            return;
        }

        let bytes = std::fs::read(&note_path).expect("read the voice note");
        assert!(bytes.len() > 1000, "the fixture must be a real note");

        // The production decode path.
        let (samples, rate) = decode_voice_note(&bytes).expect("the canonical MP4 must decode");
        assert!(!samples.is_empty(), "decoded samples must not be empty");
        assert_eq!(rate, 16_000, "the recognizer runs at 16 kHz");

        // The production recognizer. The model family is resolved the same
        // way production resolves it, so this test cannot drift away from the
        // default the harness actually uses.
        let model_type = model_type_from_env(
            std::env::var("BUZZ_VOICE_MODEL_TYPE")
                .unwrap_or_default()
                .as_str(),
        );
        let config = buzz_voice::TranscribeConfig {
            model_dir: model_dir.clone(),
            model_type,
            lm: None,
        };
        let text = buzz_voice::transcribe(&samples, rate, &config).expect("transcription succeeds");
        let lower = text.to_lowercase();
        assert!(
            lower.contains("voice note"),
            "the transcript should contain the spoken words, got: {text}"
        );
        println!("transcript: {text}");
    }

    #[test]
    fn the_default_model_family_is_the_ctc_layout_the_desktop_ships() {
        // The Buzz desktop ships `parakeet-tdt-ctc-110m-en`: a single
        // `model.int8.onnx` + `tokens.txt`. The default must therefore be the
        // CTC family, and an unset `BUZZ_VOICE_MODEL_TYPE` must resolve to it.
        assert_eq!(
            model_type_from_env(""),
            buzz_voice::ModelType::ParakeetCtc,
            "an unset model type must resolve to the CTC layout"
        );
        assert_eq!(
            model_type_from_env("parakeet-ctc"),
            buzz_voice::ModelType::ParakeetCtc,
            "the explicit CTC spelling must resolve to the CTC layout"
        );
        // The TDT triple is opt-in for the 0.6b release only.
        assert_eq!(
            model_type_from_env("parakeet"),
            buzz_voice::ModelType::Parakeet
        );
        assert_eq!(
            model_type_from_env("PARAKEET-TDT"),
            buzz_voice::ModelType::ParakeetCtc,
            "the model type match is case-insensitive"
        );
    }

    #[test]
    fn a_missing_model_directory_disables_transcription() {
        // With no BUZZ_VOICE_MODEL_DIR the harness must skip transcription
        // rather than fail the turn.
        let previous = std::env::var_os("BUZZ_VOICE_MODEL_DIR");
        std::env::remove_var("BUZZ_VOICE_MODEL_DIR");
        let resolved = TranscriberConfig::from_env();
        if let Some(value) = previous {
            std::env::set_var("BUZZ_VOICE_MODEL_DIR", value);
        }
        assert!(resolved.is_none());
    }
}
