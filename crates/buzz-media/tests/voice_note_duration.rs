//! Does the canonical voice-note packaging satisfy the relay's own duration check?
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
//! This test packages a tone exactly the way the desktop does (16x16 still at
//! r=1, `-shortest`, no `-t`), then runs the REAL validator over the result and
//! compares the stored duration against the spoken length.

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

/// Package an audio file the way the Buzz desktop does.
///
/// `transcode_voice_note_to_mp4_with_cancellation` in
/// `desktop/src-tauri/src/commands/media_transcode.rs`.
fn package_desktop_style(source: &Path, out: &Path) -> bool {
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=16x16:r=1",
            "-i",
        ])
        .arg(source)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-shortest",
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
        .arg(out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    status.map(|s| s.success()).unwrap_or(false)
}

fn make_tone(path: &Path, seconds: u32) -> bool {
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={seconds}:sample_rate=24000"),
            "-c:a",
            "pcm_s16le",
        ])
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    status.map(|s| s.success()).unwrap_or(false)
}

#[test]
fn desktop_style_packaging_yields_a_relay_accepted_duration() {
    let dir = std::env::temp_dir().join(format!("buzz-vn-dur-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    for seconds in [3u32, 10, 24] {
        let tone = dir.join(format!("tone-{seconds}.wav"));
        let packaged = dir.join(format!("packaged-{seconds}.mp4"));
        assert!(
            make_tone(&tone, seconds),
            "ffmpeg tone failed for {seconds}s"
        );
        assert!(
            package_desktop_style(&tone, &packaged),
            "ffmpeg packaging failed for {seconds}s"
        );

        // THE authority: the relay's own upload validator.
        let meta = validate_video_file(&packaged, &relay_media_config())
            .expect("relay must accept the packaged voice note");

        // The imeta handler compares the DECLARED duration against this stored
        // value with a ±0.1s tolerance.
        let stored = meta.duration_secs;
        let declared = seconds as f64;
        let drift = (declared - stored).abs();

        eprintln!(
            "source={seconds}s  relay_stored_duration={stored:.3}  drift={drift:.3}s  \
             dim={}x{}  has_audio={}",
            meta.width, meta.height, meta.has_audio
        );

        assert!(
            drift <= 0.1,
            "declared duration {declared} must match the relay's stored duration {stored} \
             within 0.1s (imeta.rs:271), but they differ by {drift:.3}s"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
