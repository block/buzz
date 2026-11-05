//! `buzz voice` — local speech-to-text and text-to-speech for voice notes and
//! audio files.
//!
//! Runs entirely on the local machine: no relay connection, no network, no
//! API key. The speech models are read from a local directory, so these
//! commands work offline and are usable as a building block by `buzz-acp`.
//!
//! A Buzz voice note is a `video/mp4` whose video track is a 16x16 black still
//! and whose audio track carries the speech (the relay has no audio allowlist,
//! so clients wrap speech in that envelope). Decoding one therefore means
//! reading the AAC track out of an MP4 container; producing one means muxing
//! the speech into that same envelope.

use std::path::{Path, PathBuf};

use buzz_voice::MAX_TRANSCRIBE_SECONDS;

use crate::error::CliError;

/// Dispatch `buzz voice ...`.
pub async fn dispatch(cmd: crate::VoiceCmd) -> Result<(), CliError> {
    match cmd {
        crate::VoiceCmd::Transcribe {
            input,
            model_dir,
            model_type,
        } => transcribe_command(&input, &model_dir, model_type),
        crate::VoiceCmd::Speak {
            text,
            output,
            model_dir,
            voice,
            wav,
        } => speak_command(&text, &output, &model_dir, &voice, wav),
    }
}

/// Transcribe a local audio or voice-note file to stdout.
///
/// The model directory is resolved from `--model-dir`, or from
/// `BUZZ_VOICE_MODEL_DIR` when the flag is omitted.
fn transcribe_command(
    input: &str,
    model_dir: &Option<PathBuf>,
    model_type: crate::VoiceModelType,
) -> Result<(), CliError> {
    let model_dir = match model_dir {
        Some(dir) => dir.clone(),
        None => std::env::var_os("BUZZ_VOICE_MODEL_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| {
                CliError::Usage(
                    "no speech model directory: pass --model-dir or set BUZZ_VOICE_MODEL_DIR"
                        .to_string(),
                )
            })?,
    };

    let config = buzz_voice::TranscribeConfig {
        model_dir: model_dir.clone(),
        model_type: model_type.as_model_type(),
        lm: None,
    };

    let samples = decode_input(input)?;
    let text = buzz_voice::transcribe(&samples.0, samples.1, &config)
        .map_err(|error| CliError::Other(error.to_string()))?;

    println!("{text}");
    Ok(())
}

/// Synthesize `text` into a speech file, offline.
///
/// The model directory is resolved from `--model-dir`, or from
/// `BUZZ_TTS_MODEL_DIR` when the flag is omitted. The reference voice is
/// resolved from `--voice` (a WAV path) or from `reference_sample.wav` inside
/// the model directory.
///
/// By default the output is a Buzz voice note: an mp4 whose video track is a
/// 16x16 black still and whose audio track carries the speech. Pass `--wav`
/// to write the raw PCM instead, which is useful for checking the synthesis
/// without the container.
fn speak_command(
    text: &str,
    output: &Option<PathBuf>,
    model_dir: &Option<PathBuf>,
    voice: &Option<PathBuf>,
    wav: bool,
) -> Result<(), CliError> {
    let model_dir = match model_dir {
        Some(dir) => dir.clone(),
        None => std::env::var_os("BUZZ_TTS_MODEL_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| {
                CliError::Usage(
                    "no TTS model directory: pass --model-dir or set BUZZ_TTS_MODEL_DIR"
                        .to_string(),
                )
            })?,
    };

    let tts = buzz_voice::load_text_to_speech(&model_dir.to_string_lossy())
        .map_err(|error| CliError::Other(error))?;

    let voice_path = match voice {
        Some(path) => path.clone(),
        None => model_dir.join(format!(
            "{}.{}",
            buzz_voice::DEFAULT_VOICE,
            buzz_voice::VOICE_FILE_EXT
        )),
    };
    let style = buzz_voice::load_voice_style(&voice_path).map_err(|error| {
        CliError::Other(format!("reference voice: {error} (looked in {})", voice_path.display()))
    })?;

    let samples = tts
        .synth_chunk(text, "en", &style, 0)
        .map_err(|error| CliError::Other(error))?;
    if samples.is_empty() {
        return Err(CliError::Other("synthesis produced no audio".to_string()));
    }

    let rate = buzz_voice::SAMPLE_RATE;
    let samples = trim_trailing_silence(&samples, rate);

    let out_path = match output {
        Some(path) => path.clone(),
        None => PathBuf::from("voice-note.mp4"),
    };

    if wav {
        write_wav(&out_path, &samples, rate)?;
    } else {
        // Mux into the Buzz voice-note envelope: a 16x16 black still at 1 fps
        // plus the speech as AAC, exactly as the desktop packages it.
        let wav_path = out_path.with_extension("voice-tmp.wav");
        write_wav(&wav_path, &samples, rate)?;
        let result = mux_voice_note(&wav_path, &out_path);
        let _ = std::fs::remove_file(&wav_path);
        result?;
    }

    println!("{}", out_path.display());
    Ok(())
}

/// Trim trailing silence from synthesized audio, keeping a short natural tail.
///
/// The relay validates a voice-note upload with `validate_video_file()` and
/// stores `VideoMeta::duration_secs`, then the imeta handler requires the
/// declared `duration` to match that stored value within ±0.1s
/// (`buzz-relay/src/handlers/imeta.rs`). The stored duration comes from the mp4
/// track header, and the video track is cut to the audio's length, so any
/// trailing silence in the synthesized PCM inflates the stored duration and
/// pushes the two apart.
///
/// Measured on a real synthesis: the engine emits ~0.11s of trailing silence,
/// which put the drift at 0.48s — over four times the tolerance. Trimming it
/// brings the drift to 0.008s.
///
/// The tail is kept short but non-zero so the note does not end abruptly.
fn trim_trailing_silence(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    const TAIL_MS: u32 = 20;
    const THRESHOLD: f32 = 0.01;

    let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
    if peak <= 0.0 {
        return samples.to_vec();
    }
    let threshold = peak * THRESHOLD;

    let last = samples
        .iter()
        .rposition(|s| s.abs() > threshold)
        .unwrap_or(0);
    let keep = (last + (sample_rate * TAIL_MS / 1000) as usize).min(samples.len());
    samples[..keep].to_vec()
}

/// Write mono 16-bit PCM samples as a WAV file.
fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), CliError> {
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        pcm.extend_from_slice(&((clamped * i16::MAX as f32) as i16).to_le_bytes());
    }

    let mut bytes = Vec::with_capacity(44 + pcm.len());
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&pcm);

    std::fs::write(path, &bytes)
        .map_err(|error| CliError::Other(format!("write {}: {error}", path.display())))
}

/// Wrap a speech WAV in the Buzz voice-note envelope (mp4, 16x16 still + AAC).
///
/// The video track is a 16x16 black still, exactly as the desktop packages it,
/// but at 25 fps rather than 1 fps. The relay validates an upload with
/// `validate_video_file()` and stores `VideoMeta::duration_secs`, then the
/// imeta handler requires the declared `duration` to match that stored value
/// within ±0.1s (`buzz-relay/src/handlers/imeta.rs`). The stored duration comes
/// from the mp4 track header, so the video track's length has to match the
/// audio's. At 1 fps the video quantizes to whole seconds and a 4.8s note
/// stores as 4.0s — an 0.8s drift, far outside the tolerance, which makes the
/// relay reject the upload. At 25 fps the track is quantized to 40ms, so the
/// drift stays under 0.02s across every duration measured (1s to 31.7s).
///
/// The still image is unchanged — only the frame rate differs, so the track
/// is still a single black 16x16 frame repeated.
fn mux_voice_note(wav: &Path, output: &Path) -> Result<(), CliError> {
    // The video track is cut to the audio's length with an explicit `-t`
    // rather than `-shortest`. `-shortest` cannot be used here: the `color`
    // lavfi source is infinite, and letting ffmpeg resolve "shortest" against
    // it fails with a misleading "No space left on device" (exit 228).
    let seconds = wav_duration_seconds(wav)?;
    let duration_arg = format!("{seconds:.3}");

    let status = std::process::Command::new("ffmpeg")
        .args([
            "-y",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=16x16:r=25",
        ])
        .arg("-i")
        .arg(wav)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-t",
            &duration_arg,
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-sn",
            "-dn",
            "-fflags",
            "+bitexact",
            "-flags:v",
            "+bitexact",
            "-flags:a",
            "+bitexact",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-tune",
            "stillimage",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-movflags",
            "+faststart",
            "-metadata",
            "encoder=",
        ])
        .arg(output)
        .status()
        .map_err(|error| {
            CliError::Other(format!(
                "ffmpeg is required to package a voice note: {error}"
            ))
        })?;

    if !status.success() {
        return Err(CliError::Other(format!(
            "ffmpeg failed to package the voice note (exit {status})"
        )));
    }
    Ok(())
}

/// Read a mono WAV's duration in seconds.
///
/// Parses the RIFF chunks rather than assuming the `data` chunk sits at a
/// fixed offset — ffmpeg writes a `LIST` chunk before it.
fn wav_duration_seconds(path: &Path) -> Result<f64, CliError> {
    let bytes = std::fs::read(path)
        .map_err(|error| CliError::Other(format!("read {}: {error}", path.display())))?;

    let mut sample_rate = 0u32;
    let mut data_len = 0u32;
    let mut pos = 12usize; // past "RIFF" + size + "WAVE"
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        if id == b"fmt " {
            sample_rate = u32::from_le_bytes([
                bytes[pos + 12],
                bytes[pos + 13],
                bytes[pos + 14],
                bytes[pos + 15],
            ]);
        }
        if id == b"data" {
            data_len = size as u32;
        }
        pos += 8 + size + (size & 1);
    }

    if sample_rate == 0 || data_len == 0 {
        return Err(CliError::Other(format!(
            "{} is not a usable mono 16-bit WAV",
            path.display()
        )));
    }
    Ok(data_len as f64 / 2.0 / sample_rate as f64)
}

/// Decode `input` into mono samples and its sample rate.
///
/// Accepts a WAV file directly, and anything else through `ffmpeg` (which is
/// what reads the AAC track out of a Buzz voice note's MP4 container).
fn decode_input(input: &str) -> Result<(Vec<f32>, u32), CliError> {
    let path = std::path::Path::new(input);
    if !path.exists() {
        return Err(CliError::Usage(format!("file not found: {input}")));
    }

    if is_wav(path) {
        let wave = sherpa_onnx::Wave::read(&path.display().to_string())
            .ok_or_else(|| CliError::Other(format!("could not read WAV file {input}")))?;
        let rate = wave.sample_rate() as u32;
        let samples = wave.samples().to_vec();
        return Ok((samples, rate));
    }

    decode_with_ffmpeg(path)
}

/// True when `path` is a RIFF/WAVE file.
///
/// Reads only the first 12 bytes — the RIFF magic, the chunk size, and the
/// `WAVE` form type. Reading the whole file to sniff a magic number made a
/// large input cost a full extra read before the real decode even started.
///
/// Note: the WAV branch of [`decode_input`] loads the file through
/// `sherpa_onnx::Wave::read` with no duration bound, so an over-long WAV is
/// fully materialised before the recognizer's cap rejects it — the same
/// shape as the ffmpeg decode bug this branch's sibling had. That path is
/// local and user-invoked (a person points the CLI at a file on their own
/// disk), not the remote ingest path, so it is left as-is deliberately.
fn is_wav(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 12];
    match file.read_exact(&mut header) {
        Ok(()) => &header[..4] == b"RIFF" && &header[8..12] == b"WAVE",
        Err(_) => false,
    }
}

/// Decode any media file to mono 16-bit PCM via `ffmpeg`.
///
/// `ffmpeg` is the only tool that reads the AAC track out of a Buzz voice
/// note's MP4 container, and it is already a dependency of the media pipeline.
///
/// The decode is bounded by `MAX_TRANSCRIBE_SECONDS` via `-t`. Without it,
/// ffmpeg decodes the whole file into `output.stdout` and the recognizer's
/// cap only rejects it afterwards — so the memory cost scales with the
/// input, not with the cap.
fn decode_with_ffmpeg(path: &std::path::Path) -> Result<(Vec<f32>, u32), CliError> {
    const SAMPLE_RATE: u32 = 16_000;

    let display = path.display().to_string();
    // `-t` bounds the decode. Without it, ffmpeg decodes the whole file into
    // `output.stdout` and the recognizer's cap only rejects it afterwards —
    // so the memory cost scales with the input, not with the cap.
    //
    // The bound is one second PAST the cap, not at it: ffmpeg's `-t` is
    // inclusive, so `-t 120` on a 900-second file yields exactly the cap's
    // worth of samples, and the recognizer's `samples.len() > max_samples`
    // check would no longer fire — an over-long file would silently
    // transcribe as an empty note instead of being rejected. Decoding one
    // second past the cap keeps the rejection intact while still bounding
    // the buffer to ~7.7 MB.
    let decode_limit = MAX_TRANSCRIBE_SECONDS + 1.0;
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error"])
        .arg("-i")
        .arg(path)
        .args(["-t", &decode_limit.to_string()])
        .args(["-f", "f32le", "-ar"])
        .arg(SAMPLE_RATE.to_string())
        .args(["-ac", "1"])
        .arg("pipe:1")
        .output()
        .map_err(|error| {
            CliError::Other(format!(
                "could not run ffmpeg to decode {display}: {error}. \
                 Install ffmpeg, or pass a WAV file."
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CliError::Other(format!(
            "ffmpeg could not decode {display}: {}",
            stderr.trim()
        )));
    }

    let bytes = output.stdout;
    if bytes.len() % 4 != 0 {
        return Err(CliError::Other(
            "ffmpeg returned a truncated sample buffer".to_string(),
        ));
    }
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    if samples.is_empty() {
        return Err(CliError::Other(format!(
            "{display} decoded to no audio samples"
        )));
    }
    Ok((samples, SAMPLE_RATE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_input_is_a_usage_error() {
        let error =
            decode_input("/nonexistent/audio/file.wav").expect_err("missing file must fail");
        assert!(
            matches!(error, CliError::Usage(_)),
            "expected a usage error, got {error:?}"
        );
    }

    #[test]
    fn a_non_wav_file_is_routed_through_ffmpeg() {
        // A file that is not RIFF/WAVE must not be read as a WAV.
        let path = std::env::temp_dir().join("buzz-cli-voice-not-a-wav.bin");
        std::fs::write(&path, b"\x00\x01\x02\x03not a wav").expect("write fixture");
        assert!(!is_wav(&path));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_wav_file_is_detected_by_its_magic_bytes() {
        let path = std::env::temp_dir().join("buzz-cli-voice-is-a-wav.wav");
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        std::fs::write(&path, bytes).expect("write fixture");
        assert!(is_wav(&path));
        let _ = std::fs::remove_file(&path);
    }

    /// A file too short to hold a WAV header is not a WAV, and must not be
    /// mistaken for one.
    ///
    /// The sniff reads exactly 12 bytes. A file shorter than that must
    /// return false rather than erroring or reading past the end.
    #[test]
    fn a_file_shorter_than_a_wav_header_is_not_a_wav() {
        for length in 0..12usize {
            let path = std::env::temp_dir().join(format!("buzz-cli-voice-too-short-{length}.bin"));
            std::fs::write(&path, vec![0u8; length]).expect("write fixture");
            assert!(
                !is_wav(&path),
                "a {length}-byte file must not be detected as a WAV"
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// A file that starts with `RIFF` but is not `WAVE` is not a WAV.
    #[test]
    fn a_riff_file_that_is_not_wave_is_not_a_wav() {
        let path = std::env::temp_dir().join("buzz-cli-voice-is-riff-not-wave.wav");
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"AVI ");
        std::fs::write(&path, bytes).expect("write fixture");
        assert!(!is_wav(&path));
        let _ = std::fs::remove_file(&path);
    }

    /// The ffmpeg decode must be BOUNDED by `MAX_TRANSCRIBE_SECONDS`.
    ///
    /// Without `-t`, ffmpeg decodes the whole input into `output.stdout`
    /// before the recognizer's cap rejects it — so the memory cost scales
    /// with the input rather than with the cap. The relay's media validation
    /// caps the video track's duration but not the audio track's, so an
    /// ordinary inbound attachment can carry hours of AAC.
    ///
    /// This asserts the bound on the real decoder: a file longer than the cap
    /// must decode to no more than the cap's worth of samples.
    #[test]
    fn the_ffmpeg_decode_is_bounded_by_the_transcribe_cap() {
        // A file with more than the cap's worth of audio. A 16-bit mono
        // 16 kHz WAV of silence is cheap to synthesize and unambiguous.
        let sample_rate = 16_000u32;
        let seconds = MAX_TRANSCRIBE_SECONDS as usize + 10;
        let samples = (sample_rate as usize) * seconds;
        let mut pcm = Vec::with_capacity(samples * 2);
        for _ in 0..samples {
            pcm.extend_from_slice(&0i16.to_le_bytes());
        }

        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&pcm);

        let path = std::env::temp_dir().join("buzz-cli-voice-too-long.wav");
        std::fs::write(&path, &bytes).expect("write fixture");

        // The WAV path is NOT ffmpeg's, so drive the ffmpeg decoder directly
        // to exercise the bound that matters.
        let result = decode_with_ffmpeg(&path);
        let _ = std::fs::remove_file(&path);

        let (decoded, rate) = result.expect("ffmpeg must decode the fixture");
        assert_eq!(rate, 16_000);

        // Two bounds, because there are two bugs.
        //
        // UPPER: the decode must not exceed the cap plus one second. Without
        // any `-t`, ffmpeg materialises the whole file, so the memory cost
        // scales with the input rather than with the cap.
        let max_samples = ((MAX_TRANSCRIBE_SECONDS + 1.0) * 16_000.0) as usize;
        assert!(
            decoded.len() <= max_samples,
            "the decode must be bounded to {max_samples} samples, got {}",
            decoded.len()
        );

        // LOWER: the decode must exceed the cap, so the recognizer's own
        // strict `>` guard still fires and rejects the note.
        //
        // ffmpeg's `-t` is INCLUSIVE, so `-t` at exactly the cap yields
        // exactly `max_samples` and `samples.len() > max_samples` is FALSE —
        // an over-long note would silently transcribe as a partial note
        // instead of being rejected. The one second of headroom is what
        // keeps the rejection intact, so the bound must be MORE than the
        // cap, not merely at most the cap.
        let cap_samples = (MAX_TRANSCRIBE_SECONDS * 16_000.0) as usize;
        assert!(
            decoded.len() > cap_samples,
            "the decode must exceed the cap ({cap_samples} samples) so the \
             recognizer still rejects it, got {}",
            decoded.len()
        );
    }
}
