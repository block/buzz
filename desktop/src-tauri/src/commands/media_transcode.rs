//! Video transcoding and poster-frame extraction via ffmpeg.
//!
//! Split out of `media.rs` to keep that file under the desktop line-size
//! limit. These helpers are used by the upload pipeline to normalize any
//! video to H.264/AAC/MP4/fast-start (guaranteed to pass the relay's
//! `validate_video_file()`) and to produce a JPEG poster frame.

use crate::managed_agents::resolve_command;

/// Build an ffmpeg command without inheriting user-controlled process knobs.
///
/// The binary path is resolved before this point, so a shell and `PATH` are not
/// needed. Windows keeps only the OS variables required for process/DLL lookup.
fn ffmpeg_command(path: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new(path);
    #[cfg(target_os = "windows")]
    let required_windows_env: Vec<(&'static str, std::ffi::OsString)> =
        ["SystemRoot", "WINDIR", "TEMP", "TMP"]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (name, value)))
            .collect();
    command.env_clear().env("LANG", "C");
    #[cfg(target_os = "windows")]
    for (name, value) in required_windows_env {
        command.env(name, value);
    }
    command
}

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

    match ffmpeg_command(&ffmpeg_path)
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

/// HEIC/HEIF compatible-brand codes that mark an ISO-BMFF file as a still
/// HEIF image. Mirrors mobile's `_heicBrands` set in
/// `mobile/lib/shared/relay/media_upload.dart` so detection stays consistent
/// across platforms — deliberately broader than the `infer` crate, which only
/// recognizes `heic`/`heix` majors (or `mif1`/`msf1` with a `heic` compatible
/// brand) and would miss `hevc`/`hevx`/`heim`/`heis`.
const HEIC_BRANDS: &[&[u8; 4]] = &[
    b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"mif1", b"msf1",
];

/// Detect a HEIC/HEIF still image by magic bytes.
///
/// HEIC/HEIF is an ISO base media file (ISO-BMFF): a `ftyp` box at offset 4
/// followed by a major brand and a list of compatible brands. We scan the
/// major brand plus the compatible-brand list for any of `HEIC_BRANDS`.
///
/// Mirrors mobile's `_looksLikeHeicOrHeif`: requires the `ftyp` marker at
/// offset 4 and scans 4-byte brand codes at offsets 8, 12, 16, ... up to the
/// first 32 bytes. The Tauri webview / Chromium cannot decode HEIC, so any
/// match here is transcoded to JPEG before upload.
pub(super) fn is_heic_file(buf: &[u8]) -> bool {
    // Need at least the 8-byte box header + 4-byte major brand.
    if buf.len() < 12 || &buf[4..8] != b"ftyp" {
        return false;
    }

    // Scan the major brand (offset 8) and each compatible brand, bounded to
    // the first 32 bytes (matches mobile's window).
    let upper = buf.len().min(32);
    let mut offset = 8;
    while offset + 4 <= upper {
        let brand: &[u8; 4] = buf[offset..offset + 4].try_into().expect("4-byte slice");
        if HEIC_BRANDS.contains(&brand) {
            return true;
        }
        offset += 4;
    }

    false
}

/// True if a filename ends in `.heic` or `.heif` (case-insensitive).
///
/// Mirrors mobile's `_hasHeicFileExtension`. Used on the file-picker path as a
/// secondary signal — some HEIC files from non-Apple tooling carry brands not
/// in `HEIC_BRANDS`, but the extension still tells us the webview can't render
/// them. The byte-based path (paste/drag) has no filename and relies solely on
/// `is_heic_file`.
pub(super) fn has_heic_extension(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("heic") || ext.eq_ignore_ascii_case("heif"))
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

/// Correlates ffmpeg progress with one renderer upload job.
#[derive(Clone)]
pub(super) struct TranscodeProgress {
    pub app: tauri::AppHandle,
    pub id: String,
    pub cancel: Option<tokio_util::sync::CancellationToken>,
}

fn probe_video_duration(source: &std::path::Path) -> Option<f64> {
    let ffprobe = resolve_command("ffprobe")?;
    let output = ffmpeg_command(&ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(source)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
}

fn emit_transcode_progress(progress: &TranscodeProgress, processed_us: u64, total_us: u64) {
    use tauri::Emitter;
    let _ = progress.app.emit(
        "media-upload-progress",
        serde_json::json!({
            "id": progress.id,
            "phase": "processing",
            "sent": processed_us.min(total_us),
            "total": total_us,
        }),
    );
}

/// Transcode any video file to a web-compatible, size-efficient MP4.
///
/// Video is capped at 1920×1080 without upscaling, encoded as H.264 CRF 24
/// with AAC audio, and stripped of source metadata. H.264/AAC is deliberately
/// preferred over smaller but less interoperable HEVC/AV1 output because the
/// result must play in every supported Buzz webview and browser.
///
/// Returns the path to a temp file. Caller must clean up.
pub(super) fn transcode_to_mp4(
    source: &std::path::Path,
    ffmpeg: &std::path::Path,
    progress: Option<TranscodeProgress>,
) -> Result<std::path::PathBuf, String> {
    let output = std::env::temp_dir().join(format!("buzz-transcode-{}.mp4", uuid::Uuid::new_v4()));
    let duration_us = probe_video_duration(source).map(|seconds| (seconds * 1_000_000.0) as u64);

    let mut command = ffmpeg_command(ffmpeg);
    command
        .args([
            "-y",
            "-nostdin",
            "-loglevel",
            "error",
            "-nostats",
            "-protocol_whitelist",
            "file,pipe",
        ])
        .arg("-i")
        .arg(source)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-sn",
            "-dn",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "24",
            "-pix_fmt",
            "yuv420p",
            "-vf",
            "scale=w=min(1920\\,iw):h=min(1080\\,ih):force_original_aspect_ratio=decrease:force_divisible_by=2,setsar=1",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-movflags",
            "+faststart",
            "-metadata",
            "encoder=",
            "-progress",
            "pipe:1",
        ])
        .arg(&output)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|e| format!("failed to spawn ffmpeg: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("failed to read ffmpeg progress")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("failed to read ffmpeg diagnostics")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if let Some(value) = line.strip_prefix("out_time_us=") {
                if let Ok(value) = value.parse::<u64>() {
                    let _ = tx.send(value);
                }
            }
        }
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut pipe = stderr;
        let _ = std::io::Read::read_to_end(&mut pipe, &mut bytes);
        bytes
    });

    let deadline = std::time::Instant::now() + FFMPEG_TIMEOUT;
    let mut last_percent = u64::MAX;
    let status = loop {
        while let Ok(processed_us) = rx.try_recv() {
            if let (Some(progress), Some(total_us)) = (&progress, duration_us) {
                let percent = processed_us.saturating_mul(100) / total_us.max(1);
                if percent != last_percent {
                    last_percent = percent;
                    emit_transcode_progress(progress, processed_us, total_us);
                }
            }
        }
        if progress
            .as_ref()
            .and_then(|progress| progress.cancel.as_ref())
            .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
        {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            let _ = stderr_reader.join();
            let _ = std::fs::remove_file(&output);
            return Err("Video conversion canceled".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() <= deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = stderr_reader.join();
                let _ = std::fs::remove_file(&output);
                return Err(format!(
                    "ffmpeg timed out after {}s",
                    FFMPEG_TIMEOUT.as_secs()
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = stderr_reader.join();
                let _ = std::fs::remove_file(&output);
                return Err(format!("failed to wait on ffmpeg: {error}"));
            }
        }
    };
    let _ = reader.join();
    let stderr = stderr_reader.join().unwrap_or_default();

    if !status.success() {
        let _ = std::fs::remove_file(&output);
        let stderr = String::from_utf8_lossy(&stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.is_empty() && !line.starts_with("  "))
            .unwrap_or("unknown error");
        return Err(format!("Video conversion failed: {detail}"));
    }
    if let (Some(progress), Some(total_us)) = (&progress, duration_us) {
        emit_transcode_progress(progress, total_us, total_us);
    }
    Ok(output)
}

/// Transcode a HEIC/HEIF still image to JPEG via ffmpeg.
///
/// The Tauri webview / Chromium cannot decode HEIC, so iPhone photos uploaded
/// as-is render blank in the composer and are unviewable for everyone. This
/// normalizes them to JPEG (the same fix mobile applies before upload).
///
/// Uses `-frames:v 1` so multi-image HEIF containers (Live Photos, bursts)
/// yield a single still, and `-q:v 2` for high JPEG quality. Returns the path
/// to a temp file. Caller must clean up.
pub(super) fn transcode_heic_to_jpeg(
    source: &std::path::Path,
    ffmpeg: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    // UUID-based temp path — unique across concurrent uploads.
    let output = std::env::temp_dir().join(format!("buzz-heic-{}.jpg", uuid::Uuid::new_v4()));

    // Single-frame image decode — 60s is generous even for large HEICs.
    let heic_timeout = std::time::Duration::from_secs(60);

    let result = run_ffmpeg_with_timeout(
        ffmpeg_command(ffmpeg)
            .args([
                "-y",
                "-nostdin",
                "-loglevel",
                "error",
                "-protocol_whitelist",
                "file,pipe",
            ]) // suppress progress spam — prevents stderr pipe deadlock
            .arg("-i")
            .arg(source) // OsStr — handles non-UTF-8 paths on Unix
            .args([
                "-map",
                "0:v:0",
                "-map_metadata",
                "-1",
                "-frames:v",
                "1",
                "-q:v",
                "2",
            ])
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped()),
        heic_timeout,
    )?;

    if !result.status.success() {
        let _ = std::fs::remove_file(&output);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|l| !l.is_empty() && !l.starts_with("  "))
            .unwrap_or("unknown error");
        return Err(format!("HEIC conversion failed: {detail}"));
    }

    Ok(output)
}

/// Transcode a HEIC/HEIF still image (from a path) to JPEG bytes.
///
/// Resolves ffmpeg, transcodes, reads the JPEG bytes, and cleans up the temp
/// file. Mirrors `transcode_and_extract_poster` but for images (no poster).
pub(super) fn transcode_heic_path_to_jpeg_bytes(
    source: &std::path::Path,
) -> Result<Vec<u8>, String> {
    let ffmpeg_path = find_ffmpeg()?;
    let jpeg_path = transcode_heic_to_jpeg(source, &ffmpeg_path)?;
    let bytes =
        std::fs::read(&jpeg_path).map_err(|e| format!("failed to read transcoded HEIC: {e}"));
    let _ = std::fs::remove_file(&jpeg_path);
    bytes
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
        ffmpeg_command(ffmpeg)
            .args([
                "-y",
                "-nostdin",
                "-loglevel",
                "error",
                "-protocol_whitelist",
                "file,pipe",
            ])
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
            ffmpeg_command(ffmpeg)
                .args([
                    "-y",
                    "-nostdin",
                    "-loglevel",
                    "error",
                    "-protocol_whitelist",
                    "file,pipe",
                ])
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

/// Transcode video and extract a poster frame without loading the video into RAM.
///
/// Returns the transcoded temp-file path plus optional poster bytes. The caller
/// owns the video path and must remove it after upload. Poster extraction is
/// best-effort because the video remains usable without a thumbnail.
pub(super) fn transcode_and_extract_poster_path(
    source: &std::path::Path,
    progress: Option<TranscodeProgress>,
) -> Result<(std::path::PathBuf, Option<Vec<u8>>), String> {
    let ffmpeg_path = find_ffmpeg()?;
    let transcoded = transcode_to_mp4(source, &ffmpeg_path, progress)?;

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

    Ok((transcoded, poster_bytes))
}

/// Compatibility helper for byte-based upload entry points.
pub(super) fn transcode_and_extract_poster(
    source: &std::path::Path,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let (transcoded, poster_bytes) = transcode_and_extract_poster_path(source, None)?;
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

    /// Build a minimal ISO-BMFF `ftyp` box header with the given major brand
    /// and optional compatible brands, suitable for `is_heic_file` testing.
    fn ftyp_box(major: &[u8; 4], compatible: &[&[u8; 4]]) -> Vec<u8> {
        let mut buf = vec![0x00, 0x00, 0x00, 0x00]; // box size (unused by detector)
        buf.extend_from_slice(b"ftyp");
        buf.extend_from_slice(major);
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // minor version
        for brand in compatible {
            buf.extend_from_slice(*brand);
        }
        buf
    }

    #[test]
    fn test_is_heic_file_major_brands() {
        // Every brand in HEIC_BRANDS should be detected as the major brand.
        for brand in HEIC_BRANDS {
            let buf = ftyp_box(brand, &[]);
            assert!(is_heic_file(&buf), "major brand {brand:?} not detected");
        }
    }

    #[test]
    fn test_is_heic_file_variants_infer_misses() {
        // These brands are detected by mobile but NOT by the `infer` crate's
        // HEIC heuristic — the whole reason we mirror mobile's full set.
        for brand in [b"hevc", b"hevx", b"heim", b"heis"] {
            let buf = ftyp_box(brand, &[]);
            assert!(is_heic_file(&buf), "variant brand {brand:?} not detected");
        }
    }

    #[test]
    fn test_is_heic_file_compatible_brand() {
        // Major brand is generic (mif1), HEIC signaled via compatible brand.
        let buf = ftyp_box(b"mif1", &[b"heic"]);
        assert!(is_heic_file(&buf));
    }

    #[test]
    fn test_is_heic_file_jpeg_is_not_heic() {
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00, 0x01,
        ];
        assert!(!is_heic_file(&jpeg));
    }

    #[test]
    fn test_is_heic_file_mp4_is_not_heic() {
        // An MP4 ftyp box (isom) must not be misdetected as HEIC.
        let mp4 = ftyp_box(b"isom", &[b"isom", b"iso2"]);
        assert!(!is_heic_file(&mp4));
    }

    #[test]
    fn test_is_heic_file_empty() {
        assert!(!is_heic_file(&[]));
    }

    #[test]
    fn test_is_heic_file_too_short() {
        // Has `ftyp` marker but fewer than 12 bytes — below mobile's threshold.
        let buf = [0x00, 0x00, 0x00, 0x00, b'f', b't', b'y', b'p'];
        assert!(!is_heic_file(&buf));
    }

    #[test]
    fn test_is_heic_file_no_ftyp_marker() {
        // 12+ bytes containing a HEIC brand but no `ftyp` at offset 4.
        let mut buf = vec![0u8; 16];
        buf[8..12].copy_from_slice(b"heic");
        assert!(!is_heic_file(&buf));
    }

    #[test]
    fn test_is_heic_file_brand_past_window() {
        // A HEIC brand sitting beyond the 32-byte scan window must not match,
        // matching mobile's bounded scan. Use non-HEIC major + filler brands
        // so the only HEIC brand present is the one pushed past offset 32.
        let mut buf = ftyp_box(b"isom", &[b"iso2", b"iso4", b"avc1", b"mp41", b"mp42"]);
        buf.extend_from_slice(b"heic"); // lands at offset 36, past the window
        assert!(!is_heic_file(&buf));
    }

    #[test]
    fn test_has_heic_extension() {
        use std::path::Path;
        assert!(has_heic_extension(Path::new("IMG_1234.HEIC")));
        assert!(has_heic_extension(Path::new("photo.heic")));
        assert!(has_heic_extension(Path::new("photo.heif")));
        assert!(has_heic_extension(Path::new("photo.HEIF")));
        assert!(!has_heic_extension(Path::new("photo.jpg")));
        assert!(!has_heic_extension(Path::new("photo.png")));
        assert!(!has_heic_extension(Path::new("noextension")));
    }

    #[test]
    fn test_transcode_to_mp4_caps_at_1080p_without_upscaling() {
        let Ok(ffmpeg) = find_ffmpeg() else {
            eprintln!("skipping dimension round-trip: ffmpeg not found");
            return;
        };
        let Some(ffprobe) = resolve_command("ffprobe") else {
            eprintln!("skipping dimension round-trip: ffprobe not found");
            return;
        };
        let source =
            std::env::temp_dir().join(format!("buzz-dimension-test-{}.mp4", uuid::Uuid::new_v4()));
        let generated = std::process::Command::new(&ffmpeg)
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("color=c=blue:s=2560x1440:r=1:d=0.1")
            .args(["-frames:v", "1", "-c:v", "libx264"])
            .arg(&source)
            .output()
            .expect("generate oversized fixture");
        if !generated.status.success() {
            eprintln!("skipping dimension round-trip: ffmpeg cannot encode H.264");
            let _ = std::fs::remove_file(&source);
            return;
        }

        let output = transcode_to_mp4(&source, &ffmpeg, None).expect("transcode fixture");
        let dimensions = std::process::Command::new(ffprobe)
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height",
                "-of",
                "csv=s=x:p=0",
            ])
            .arg(&output)
            .output()
            .expect("probe output dimensions");
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&output);
        assert!(dimensions.status.success());
        assert_eq!(
            String::from_utf8_lossy(&dimensions.stdout).trim(),
            "1920x1080"
        );
    }

    #[test]
    fn test_transcode_to_mp4_drops_source_metadata() {
        let Ok(ffmpeg) = find_ffmpeg() else {
            eprintln!("skipping metadata round-trip: ffmpeg not found");
            return;
        };
        let source =
            std::env::temp_dir().join(format!("buzz-metadata-test-{}.mp4", uuid::Uuid::new_v4()));
        let generated = std::process::Command::new(&ffmpeg)
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("testsrc2=size=64x64:rate=1")
            .args([
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-metadata",
                "location=+37.7-122.4/",
                "-metadata:s:v:0",
                "title=private camera stream",
            ])
            .arg(&source)
            .output()
            .expect("run ffmpeg fixture generation");
        if !generated.status.success() {
            eprintln!("skipping metadata round-trip: ffmpeg cannot encode H.264");
            let _ = std::fs::remove_file(&source);
            return;
        }

        let output = transcode_to_mp4(&source, &ffmpeg, None).expect("transcode fixture");
        let bytes = std::fs::read(&output).expect("read transcoded video");
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&output);
        for secret in [b"+37.7-122.4/".as_slice(), b"private camera stream"] {
            assert!(
                !bytes.windows(secret.len()).any(|window| window == secret),
                "source metadata survived transcode"
            );
        }
    }

    /// Round-trip transcode test, gated on ffmpeg being present so CI without
    /// ffmpeg doesn't fail. Generates a HEIC via ffmpeg, then transcodes it
    /// back to JPEG and asserts the output is a valid JPEG.
    #[test]
    fn test_transcode_heic_round_trip() {
        let Ok(ffmpeg) = find_ffmpeg() else {
            eprintln!("skipping HEIC round-trip: ffmpeg not found");
            return;
        };

        // Generate a small HEIC test image from a synthetic color source.
        let heic_path =
            std::env::temp_dir().join(format!("buzz-test-{}.heic", uuid::Uuid::new_v4()));
        let gen = std::process::Command::new(&ffmpeg)
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("color=c=red:s=64x64:d=1")
            .args(["-frames:v", "1"])
            .arg(&heic_path)
            .output();

        let gen = match gen {
            Ok(o) if o.status.success() && heic_path.exists() => o,
            other => {
                // This ffmpeg build can't encode HEIC — skip rather than fail.
                eprintln!("skipping HEIC round-trip: ffmpeg cannot encode HEIC: {other:?}");
                let _ = std::fs::remove_file(&heic_path);
                return;
            }
        };
        drop(gen);

        // Sanity: the generated file should be detected as HEIC.
        let heic_bytes = std::fs::read(&heic_path).expect("read generated heic");
        assert!(
            is_heic_file(&heic_bytes),
            "generated file not detected as HEIC"
        );

        // Transcode to JPEG bytes and verify the JPEG magic.
        let jpeg = transcode_heic_path_to_jpeg_bytes(&heic_path).expect("transcode to jpeg");
        let _ = std::fs::remove_file(&heic_path);
        assert!(jpeg.len() > 2, "empty jpeg output");
        assert_eq!(&jpeg[0..2], &[0xFF, 0xD8], "output is not a JPEG");
    }
}
