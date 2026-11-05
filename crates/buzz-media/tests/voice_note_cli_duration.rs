//! Does the voice note that `buzz voice speak` produces satisfy the relay's
//! own duration check?
//!
//! The relay validates an upload with `validate_video_file()` (buzz-media) and
//! stores `VideoMeta::duration_secs`, then the imeta handler requires the
//! declared `duration` to match that stored value within ±0.1s
//! (`buzz-relay/src/handlers/imeta.rs`).
//!
//! The duration the relay stores comes from the mp4 crate's TRACK header
//! (`track.duration()`), which is NOT necessarily what ffprobe reports. So the
//! question — "does a packaged voice note's declared duration match what the
//! relay stored?" — has to be answered with the relay's own code, not ffprobe.
//!
//! This test runs the REAL `buzz voice speak` command, then runs the REAL
//! validator over its output, and compares the stored duration against the
//! spoken length.

use std::path::Path;
use std::process::Command;

use buzz_media::config::MediaConfig;
use buzz_media::validation::validate_video_file;

/// The relay's own media config, as `buzz-relay/src/config.rs` builds it.
fn relay_media_config() -> MediaConfig {
    MediaConfig {
        s3_endpoint: "http://localhost:9000".to_string(),
        s3_access_key: "buzz_dev".to_string(),
        s3_secret_key: "buzz_dev_secret".to_string(),
        s3_bucket: "buzz-media".to_string(),
        s3_region: "us-east-1".to_string(),
        s3_addressing_style: buzz_media::config::S3AddressingStyle::default(),
        max_image_bytes: 50 * 1024 * 1024,
        max_gif_bytes: 50 * 1024 * 1024,
        max_file_bytes: 100 * 1024 * 1024,
        max_video_bytes: 500 * 1024 * 1024,
        public_base_url: "http://localhost:3000/media".to_string(),
        upload_records_enabled: false,
        upload_ip_header: None,
        upload_port_header: None,
    }
}

/// Locate the built `buzz` binary.
fn buzz_bin() -> std::path::PathBuf {
    // The test runs from the crate root, so target/ is ../../target.
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // buzz-media
    path.pop(); // crates
    path.join("target/release/buzz")
}

/// Locate the installed Pocket TTS model directory.
fn tts_model_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("/home/hermes/.buzz/models/pocket-tts")
}

/// Read the AUDIO track's duration from an mp4, in seconds.
///
/// This is the number the uploading client declares in `imeta`: the duration
/// of the file it is uploading, not the length of some separately produced
/// sample.
fn mp4_audio_duration_seconds(path: &Path) -> Result<f64, String> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("ffprobe failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffprobe could not read the audio track: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let seconds: f64 = text
        .trim()
        .parse()
        .map_err(|_| format!("unexpected ffprobe output: {}", text.trim()))?;
    Ok(seconds)
}

#[test]
fn cli_produced_voice_note_satisfies_the_relay_duration_check() {
    let buzz = buzz_bin();
    if !buzz.exists() {
        eprintln!("skipping: {} not built", buzz.display());
        return;
    }
    let model_dir = tts_model_dir();
    if !model_dir.join("bundle.json").exists() {
        eprintln!("skipping: no Pocket TTS model at {}", model_dir.display());
        return;
    }

    let dir = std::env::temp_dir().join("buzz-cli-voice-note-duration");
    std::fs::create_dir_all(&dir).expect("create dir");
    let out = dir.join("voice-note.mp4");

    // A short sentence, so the test stays quick but still produces real audio.
    let text = "Zack, this is a Buzz voice note. The text to speech engine is working.";

    let status = Command::new(&buzz)
        .args(["voice", "speak", text])
        .arg("--model-dir")
        .arg(&model_dir)
        .arg("--output")
        .arg(&out)
        .status()
        .expect("run buzz voice speak");
    assert!(status.success(), "buzz voice speak failed: {status}");
    assert!(out.exists(), "voice note was not written");

    // The spoken length, as the UPLOADING CLIENT would see it: the mp4's own
    // audio track. The client declares `duration` from the file it uploads, so
    // that is the number the relay compares against its stored value.
    //
    // It must be read from the produced mp4, NOT from a second synthesis: the
    // Pocket TTS engine is a flow-matching LM and its output length is NOT
    // deterministic (measured: the same sentence produced 4.718s and 5.120s on
    // two runs), so a separately synthesised WAV is a different sample and
    // comparing across two runs measures the engine's randomness, not the
    // packaging.
    let audio_seconds = mp4_audio_duration_seconds(&out).expect("read the mp4's audio duration");
    let seconds = audio_seconds;

    // Run the relay's REAL validator over the produced file.
    let meta = validate_video_file(&out, &relay_media_config()).expect("relay accepts the file");

    let stored = meta.duration_secs;
    let declared = seconds;
    let drift = (declared - stored).abs();

    eprintln!(
        "source={seconds}s  relay_stored_duration={stored:.3}  drift={drift:.3}s  \
         dim={}x{}  has_audio={}",
        meta.width, meta.height, meta.has_audio
    );

    assert_eq!(meta.width, 16, "voice note video track must be 16px wide");
    assert_eq!(meta.height, 16, "voice note video track must be 16px tall");
    assert!(meta.has_audio, "voice note must carry an audio track");
    assert!(
        drift <= 0.1,
        "declared duration {declared} must match the relay's stored duration {stored} \
         within 0.1s (imeta.rs:271), but they differ by {drift:.3}s"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
