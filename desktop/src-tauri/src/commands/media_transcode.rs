//! Video transcoding and poster-frame extraction via ffmpeg.
//!
//! Split out of `media.rs` to keep that file under the desktop line-size
//! limit. These helpers are used by the upload pipeline to normalize any
//! video to H.264/AAC/MP4/fast-start (guaranteed to pass the relay's
//! `validate_video_file()`) and to produce a JPEG poster frame.

use crate::managed_agents::resolve_command;

/// Locate ffmpeg using the same discovery logic as managed agents
/// (login shell PATH, /opt/homebrew/bin, /usr/local/bin, etc.).
/// Returns the resolved absolute path on success.
pub(super) fn find_ffmpeg() -> Result<std::path::PathBuf, String> {
    let ffmpeg_path = resolve_command("ffmpeg").ok_or_else(|| {
        "ffmpeg is required for video uploads but was not found.\n\n\
         Install it:\n  \
         macOS:   brew install ffmpeg\n  \
         Linux:   sudo apt install ffmpeg\n  \
         Windows: winget install ffmpeg"
            .to_string()
    })?;

    match std::process::Command::new(&ffmpeg_path)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(s) if s.success() => Ok(ffmpeg_path),
        Ok(_) => Err(
            "ffmpeg was found but returned an error — it may be broken or misconfigured"
                .to_string(),
        ),
        Err(e) => Err(format!("failed to check for ffmpeg: {e}")),
    }
}

/// Detect if a file is a video based on magic bytes.
pub(super) fn is_video_file(buf: &[u8]) -> bool {
    infer::get(buf).is_some_and(|t| t.mime_type().starts_with("video/"))
}

/// Maximum wall-clock time for an ffmpeg transcode before we kill it.
/// 10 minutes is generous for any reasonable video; pathological inputs
/// (crafted to cause exponential decode time) get killed instead of
/// blocking a Tokio worker thread indefinitely.
const FFMPEG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Run an ffmpeg command with a wall-clock timeout.
///
/// Spawns the child process, polls `try_wait()` every 500ms, and kills it
/// if the deadline is exceeded. Returns the same `Output` as `Command::output()`.
///
/// **IMPORTANT**: callers MUST pass `-loglevel error` (or `quiet`) to ffmpeg.
/// This function reads stderr only after the child exits. If ffmpeg writes
/// enough progress/diagnostic output to fill the OS pipe buffer (~64 KiB),
/// the child blocks on write() and never exits — causing a false timeout.
/// `-loglevel error` suppresses progress spam, keeping stderr small.
pub(super) fn run_ffmpeg_with_timeout(
    cmd: &mut std::process::Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, String> {
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn ffmpeg: {e}"))?;

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // Process exited — collect output.
                let stdout = child.stdout.take().map_or_else(Vec::new, |mut s| {
                    let mut buf = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut s, &mut buf);
                    buf
                });
                let stderr = child.stderr.take().map_or_else(Vec::new, |mut s| {
                    let mut buf = Vec::new();
                    let _ = std::io::Read::read_to_end(&mut s, &mut buf);
                    buf
                });
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                // Still running — check deadline.
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // reap zombie
                    return Err(format!("ffmpeg timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(e) => return Err(format!("failed to wait on ffmpeg: {e}")),
        }
    }
}

/// Transcode any video file to H.264/AAC/MP4/fast-start via ffmpeg.
///
/// Always re-encodes — handles HEVC, VP9, ProRes, non-faststart MP4, 10-bit,
/// wrong pixel format, MOV containers, etc. Output is guaranteed to pass the
/// relay's `validate_video_file()`.
///
/// Returns the path to a temp file. Caller must clean up.
pub(super) fn transcode_to_mp4(
    source: &std::path::Path,
    ffmpeg: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    // UUID-based temp path — unique across concurrent uploads.
    let output = std::env::temp_dir().join(format!("buzz-transcode-{}.mp4", uuid::Uuid::new_v4()));

    let result = run_ffmpeg_with_timeout(
        std::process::Command::new(ffmpeg)
            .args(["-y", "-loglevel", "error"]) // suppress progress spam — prevents stderr pipe deadlock
            .arg("-i")
            .arg(source) // OsStr — handles non-UTF-8 paths on Unix
            .args([
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
            ])
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped()),
        FFMPEG_TIMEOUT,
    )?;

    if !result.status.success() {
        let _ = std::fs::remove_file(&output);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.is_empty() && !l.starts_with("  "))
            .unwrap_or("unknown error");
        return Err(format!("Video conversion failed: {detail}"));
    }

    Ok(output)
}

/// Extract a single JPEG poster frame from a transcoded MP4 via ffmpeg.
///
/// Seeks to 1 second (avoids black leader frames), falls back to first frame
/// for videos shorter than 1 second. Output is scaled to 640px wide with even
/// dimensions. Returns the path to a temp JPEG. Caller must clean up.
///
/// Best-effort: returns `Err` on failure — callers should log and continue
/// without a poster rather than failing the entire video upload.
pub(super) fn extract_poster_frame(
    mp4_path: &std::path::Path,
    ffmpeg: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let output = std::env::temp_dir().join(format!("buzz-poster-{}.jpg", uuid::Uuid::new_v4()));

    // Poster extraction is a single-frame decode — 30s is generous.
    let poster_timeout = std::time::Duration::from_secs(30);

    // Try seeking to 1s first (avoids black first frames from fade-ins).
    let result = run_ffmpeg_with_timeout(
        std::process::Command::new(ffmpeg)
            .args(["-y", "-loglevel", "error"])
            .arg("-ss")
            .arg("1")
            .arg("-i")
            .arg(mp4_path)
            .args(["-vframes", "1", "-vf", "scale=640:-2", "-q:v", "2"])
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped()),
        poster_timeout,
    )?;

    // If seek to 1s failed (video shorter than 1s), retry from first frame.
    if !result.status.success()
        || !output.exists()
        || std::fs::metadata(&output).map_or(true, |m| m.len() == 0)
    {
        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            eprintln!("buzz-desktop: poster seek-to-1s failed, trying first frame: {stderr}");
        }
        let _ = std::fs::remove_file(&output);
        let fallback = run_ffmpeg_with_timeout(
            std::process::Command::new(ffmpeg)
                .args(["-y", "-loglevel", "error"])
                .arg("-i")
                .arg(mp4_path)
                .args(["-vframes", "1", "-vf", "scale=640:-2", "-q:v", "2"])
                .arg(&output)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped()),
            poster_timeout,
        )?;

        if !fallback.status.success() || !output.exists() {
            let stderr = String::from_utf8_lossy(&fallback.stderr);
            eprintln!("buzz-desktop: poster frame extraction failed: {stderr}");
            let _ = std::fs::remove_file(&output);
            return Err("ffmpeg could not extract a poster frame".to_string());
        }
    }

    Ok(output)
}

/// Transcode video and extract poster frame. Returns (video_bytes, Option<poster_bytes>).
///
/// Poster extraction is best-effort — if it fails, returns `None` for the poster
/// and the video bytes are still valid. All temp files are cleaned up.
pub(super) fn transcode_and_extract_poster(
    source: &std::path::Path,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let ffmpeg_path = find_ffmpeg()?;
    let transcoded = transcode_to_mp4(source, &ffmpeg_path)?;

    // Extract poster from the transcoded file (not the original — guarantees decodability).
    let poster_bytes = match extract_poster_frame(&transcoded, &ffmpeg_path) {
        Ok(poster_path) => {
            let bytes = std::fs::read(&poster_path).ok();
            let _ = std::fs::remove_file(&poster_path);
            bytes
        }
        Err(e) => {
            eprintln!("buzz-desktop: poster extraction failed (non-fatal): {e}");
            None
        }
    };

    let video_bytes =
        std::fs::read(&transcoded).map_err(|e| format!("failed to read transcoded file: {e}"));
    let _ = std::fs::remove_file(&transcoded);

    Ok((video_bytes?, poster_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_video_file_mp4() {
        // Minimal ftyp box (MP4 magic bytes)
        let ftyp: &[u8] = &[
            0x00, 0x00, 0x00, 0x14, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0x00, 0x00,
            0x00, 0x00, b'i', b's', b'o', b'm',
        ];
        assert!(is_video_file(ftyp));
    }

    #[test]
    fn test_is_video_file_jpeg_is_not_video() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0];
        assert!(!is_video_file(&jpeg));
    }

    #[test]
    fn test_is_video_file_empty() {
        assert!(!is_video_file(&[]));
    }

    #[test]
    fn test_find_ffmpeg_runs() {
        // This test verifies the function doesn't panic.
        // It may pass or fail depending on whether ffmpeg is installed.
        let _ = find_ffmpeg();
    }
}
