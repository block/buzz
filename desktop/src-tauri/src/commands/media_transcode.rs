//! Video transcoding and poster-frame extraction via ffmpeg.
//!
//! Split out of `media.rs` to keep that file under the desktop line-size
//! limit. These helpers are used by the upload pipeline to normalize any
//! video to H.264/AAC/MP4/fast-start (guaranteed to pass the relay's
//! `validate_video_file()`) and to produce a JPEG poster frame.

use crate::managed_agents::resolve_command;
use tokio_util::sync::CancellationToken;

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
    crate::util::configure_no_window(&mut command);
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

/// First ffmpeg release whose CLI assembles HEIF tile grids on its own
/// (`Stream #0:N -> xstack`) when no explicit `-map` is given. Older builds
/// expose the tiles as independent video streams and pick one of them.
const HEIF_TILE_GRID_MIN_FFMPEG: (u32, u32) = (8, 1);

/// Upper bound on the `meta` box we are willing to read when probing a HEIF
/// for a tile grid. iPhone files carry a few KiB; anything larger is treated
/// as "no grid found" rather than buffered.
const HEIF_META_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Split an ISO-BMFF byte range into `(box_type, payload)` pairs.
///
/// Handles 64-bit `largesize` and the "extends to end" `size == 0` form;
/// stops at the first truncated or malformed header.
fn iso_bmff_boxes(mut buf: &[u8]) -> impl Iterator<Item = (&[u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        let header: &[u8; 8] = buf.get(..8)?.try_into().ok()?;
        let box_type: &[u8; 4] = header[4..8].try_into().ok()?;
        let (size, header_len) =
            match u32::from_be_bytes([header[0], header[1], header[2], header[3]]) {
                0 => (buf.len(), 8),
                1 => {
                    let large: &[u8; 8] = buf.get(8..16)?.try_into().ok()?;
                    (usize::try_from(u64::from_be_bytes(*large)).ok()?, 16)
                }
                size => (usize::try_from(size).ok()?, 8),
            };
        if size < header_len {
            return None;
        }
        let payload = buf.get(header_len..size)?;
        buf = &buf[size..];
        Some((box_type, payload))
    })
}

/// Read a big-endian HEIF item ID at `offset`: u16, or u32 when `wide`.
fn heif_item_id(buf: &[u8], offset: usize, wide: bool) -> Option<u32> {
    if wide {
        let id = buf.get(offset..offset + 4)?;
        Some(u32::from_be_bytes([id[0], id[1], id[2], id[3]]))
    } else {
        let id = buf.get(offset..offset + 2)?;
        Some(u32::from(u16::from_be_bytes([id[0], id[1]])))
    }
}

/// True if a HEIF `meta` box payload declares its primary image as a `grid`
/// derived item — the layout iPhones use for every photo.
///
/// Walks `meta` → `pitm` for the primary item ID, then `meta` → `iinf` →
/// `infe` for that item's `item_type`. Only `infe` version 2+ carries an
/// item type, which is also the minimum version HEIF requires for `grid`
/// items. A grid that is *not* the primary item is ignored: ffmpeg's stream
/// selection follows `pitm`, so such files decode the same on every release.
/// Without a `pitm` box any `grid` item counts.
fn heif_meta_primary_item_is_grid(meta_payload: &[u8]) -> bool {
    // `meta` is a FullBox: skip 4 bytes of version/flags.
    let Some(children) = meta_payload.get(4..) else {
        return false;
    };
    let mut primary_item: Option<u32> = None;
    let mut iinf: Option<&[u8]> = None;
    for (box_type, payload) in iso_bmff_boxes(children) {
        match box_type {
            // FullBox version 0 stores a u16 item_ID, version 1 a u32.
            b"pitm" => {
                primary_item = payload
                    .first()
                    .and_then(|version| heif_item_id(payload, 4, *version != 0));
            }
            b"iinf" => iinf = Some(payload),
            _ => {}
        }
    }
    let Some(iinf) = iinf else {
        return false;
    };
    // `iinf` FullBox: version 0 uses a u16 entry count, later versions u32.
    let entries_offset = match iinf.first() {
        Some(0) => 6,
        Some(_) => 8,
        None => return false,
    };
    let Some(entries) = iinf.get(entries_offset..) else {
        return false;
    };
    iso_bmff_boxes(entries).any(|(t, infe)| {
        if t != b"infe" {
            return false;
        }
        // FullBox version, then item_ID (u16 for v2, u32 for v3),
        // item_protection_index (u16), item_type (4cc).
        let (item_id, item_type_offset) = match infe.first() {
            Some(2) => (heif_item_id(infe, 4, false), 8),
            Some(3) => (heif_item_id(infe, 4, true), 10),
            _ => return false,
        };
        primary_item.is_none_or(|primary| item_id == Some(primary))
            && infe.get(item_type_offset..item_type_offset + 4) == Some(b"grid")
    })
}

/// True if the HEIF at `path` stores its primary image as a tile grid.
///
/// Reads only top-level box headers plus the `meta` box payload (bounded by
/// `HEIF_META_MAX_BYTES`), so this stays cheap even for multi-MB photos.
/// Returns `Ok(false)` for files without a readable `meta` box — the
/// transcode itself surfaces decode errors with a better message.
fn heif_has_tile_grid(path: &std::path::Path) -> Result<bool, String> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).map_err(|e| format!("failed to open HEIC: {e}"))?;
    let mut header = [0u8; 8];
    loop {
        if file.read_exact(&mut header).is_err() {
            return Ok(false);
        }
        let mut header_len = 8u64;
        let size = match u32::from_be_bytes([header[0], header[1], header[2], header[3]]) {
            0 => return Ok(false), // box extends to EOF; only valid for a trailing `mdat`
            1 => {
                let mut large = [0u8; 8];
                if file.read_exact(&mut large).is_err() {
                    return Ok(false);
                }
                header_len = 16;
                u64::from_be_bytes(large)
            }
            size => u64::from(size),
        };
        if size < header_len {
            return Ok(false);
        }
        let payload_len = size - header_len;
        if &header[4..8] == b"meta" {
            if payload_len > HEIF_META_MAX_BYTES {
                return Ok(false);
            }
            let mut payload = Vec::new();
            if (&mut file)
                .take(payload_len)
                .read_to_end(&mut payload)
                .is_err()
            {
                return Ok(false);
            }
            return Ok(heif_meta_primary_item_is_grid(&payload));
        }
        let Ok(skip) = i64::try_from(payload_len) else {
            return Ok(false);
        };
        if file.seek(SeekFrom::Current(skip)).is_err() {
            return Ok(false);
        }
    }
}

/// Parse `(major, minor)` from the first line of `ffmpeg -version`.
///
/// Accepts release tags (`ffmpeg version 8.1.3-static`, `n7.1.5-12-g…`,
/// `7.1.1-1ubuntu1`) and returns `None` for git snapshots (`N-121557-g…`,
/// `2026-01-15-git-…`) whose feature set cannot be inferred from the string.
fn parse_ffmpeg_version(version_output: &str) -> Option<(u32, u32)> {
    let first_line = version_output.lines().next()?;
    let token = first_line
        .strip_prefix("ffmpeg version ")?
        .split_whitespace()
        .next()?;
    let token = token.strip_prefix('n').unwrap_or(token);
    // Require `<digits>.<digits>` up front so date-stamped snapshots such as
    // `2026-01-15-git-…` are not read as release 2026.1.
    let (major, rest) = token.split_once('.')?;
    let major = major.parse().ok()?;
    let minor_len = rest.bytes().take_while(u8::is_ascii_digit).count();
    let minor = rest[..minor_len].parse().ok()?;
    Some((major, minor))
}

/// Query the ffmpeg binary's `(major, minor)` release version.
fn ffmpeg_version(ffmpeg: &std::path::Path) -> Option<(u32, u32)> {
    let output = ffmpeg_command(ffmpeg)
        .arg("-version")
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    parse_ffmpeg_version(&String::from_utf8_lossy(&output.stdout))
}

/// Output options for the HEIC → JPEG transcode. Must not contain `-map`:
/// see `transcode_heic_to_jpeg`.
const HEIC_OUTPUT_ARGS: &[&str] = &["-map_metadata", "-1", "-frames:v", "1", "-q:v", "2"];

/// Refuse to transcode a tile-grid HEIF with an ffmpeg that would silently
/// emit a single tile.
///
/// `version` is `None` when the release could not be determined; that is
/// treated as unsupported so a mis-detected build never produces a cropped
/// upload without telling the user why.
fn check_heif_tile_grid_support(version: Option<(u32, u32)>) -> Result<(), String> {
    let (min_major, min_minor) = HEIF_TILE_GRID_MIN_FFMPEG;
    let found = match version {
        Some(v) if v >= HEIF_TILE_GRID_MIN_FFMPEG => return Ok(()),
        Some((major, minor)) => format!("found {major}.{minor}"),
        None => "could not determine the installed version".to_string(),
    };
    Err(format!(
        "This HEIC is stored as a tile grid (typical for iPhone photos), which needs \
         ffmpeg {min_major}.{min_minor} or newer to convert without cropping it to a \
         single tile ({found}).\n\n\
         Upgrade it:\n  \
         macOS:   brew upgrade ffmpeg\n  \
         Linux:   https://ffmpeg.org/download.html\n  \
         Windows: winget upgrade ffmpeg"
    ))
}

/// Maximum wall-clock time for an ffmpeg transcode before we kill it.
/// 10 minutes is generous for any reasonable video; pathological inputs
/// (crafted to cause exponential decode time) get killed instead of
/// blocking a Tokio worker thread indefinitely.
const FFMPEG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Run an ffmpeg command with a wall-clock timeout and optional cancellation.
///
/// Spawns the child process, polls `try_wait()` every 500ms, and kills it
/// if the deadline is exceeded. Returns the same `Output` as `Command::output()`.
///
/// **IMPORTANT**: callers MUST pass `-loglevel error` (or `quiet`) to ffmpeg.
/// This function reads stderr only after the child exits. If ffmpeg writes
/// enough progress/diagnostic output to fill the OS pipe buffer (~64 KiB),
/// the child blocks on write() and never exits — causing a false timeout.
/// `-loglevel error` suppresses progress spam, keeping stderr small.
fn run_ffmpeg_with_cancellation(
    cmd: &mut std::process::Command,
    timeout: std::time::Duration,
    cancellation: Option<&CancellationToken>,
) -> Result<std::process::Output, String> {
    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        return Err("upload cancelled".to_string());
    }
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
                if cancellation.is_some_and(CancellationToken::is_cancelled) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("upload cancelled".to_string());
                }
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
fn transcode_to_mp4_with_cancellation(
    source: &std::path::Path,
    ffmpeg: &std::path::Path,
    cancellation: Option<&CancellationToken>,
) -> Result<std::path::PathBuf, String> {
    // UUID-based temp path — unique across concurrent uploads.
    let output = std::env::temp_dir().join(format!("buzz-transcode-{}.mp4", uuid::Uuid::new_v4()));

    let result = run_ffmpeg_with_cancellation(
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
                "-map",
                "0:a:0?",
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
                "fast",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
                "-vf",
                "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-movflags",
                "+faststart",
                "-metadata",
                "encoder=",
            ])
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped()),
        FFMPEG_TIMEOUT,
        cancellation,
    )
    .inspect_err(|_| {
        let _ = std::fs::remove_file(&output);
    })?;

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

/// Package a voice-note audio file in the relay's existing canonical video
/// envelope. The tiny H.264 track satisfies the deployed video validator while
/// the AAC track remains the only user-facing content in the voice-note player.
///
/// Returns the path to a temp MP4. Caller must clean up.
pub(super) fn transcode_voice_note_to_mp4_with_cancellation(
    source: &std::path::Path,
    cancellation: Option<&CancellationToken>,
) -> Result<std::path::PathBuf, String> {
    let ffmpeg = find_ffmpeg()?;
    let output = std::env::temp_dir().join(format!("buzz-voice-note-{}.mp4", uuid::Uuid::new_v4()));

    let result = run_ffmpeg_with_cancellation(
        ffmpeg_command(&ffmpeg)
            .args([
                "-y",
                "-nostdin",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=16x16:r=1",
            ])
            .arg("-i")
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
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped()),
        FFMPEG_TIMEOUT,
        cancellation,
    )
    .inspect_err(|_| {
        let _ = std::fs::remove_file(&output);
    })?;

    if !result.status.success() {
        let _ = std::fs::remove_file(&output);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.is_empty() && !line.starts_with("  "))
            .unwrap_or("unknown error");
        return Err(format!("Voice note conversion failed: {detail}"));
    }

    Ok(output)
}

/// Transcode a HEIC/HEIF still image to JPEG via ffmpeg.
///
/// The Tauri webview / Chromium cannot decode HEIC, so iPhone photos uploaded
/// as-is render blank in the composer and are unviewable for everyone. This
/// normalizes them to JPEG (the same fix mobile applies before upload).
///
/// Deliberately passes no `-map`: iPhone HEICs store the photo as a grid of
/// 512×512 HEVC tiles, each exposed by ffmpeg as its own video stream, and
/// `-map 0:v:0` would select only the first tile. Left to its own stream
/// selection ffmpeg (≥ 8.1) picks the tile-grid group and assembles the full
/// image; older builds are refused up front for tiled inputs rather than
/// uploading one tile. Uses `-frames:v 1` so multi-image HEIF containers
/// (Live Photos, bursts) yield a single still, and `-q:v 2` for high JPEG
/// quality. Returns the path to a temp file. Caller must clean up.
fn transcode_heic_to_jpeg(
    source: &std::path::Path,
    ffmpeg: &std::path::Path,
    cancellation: Option<&CancellationToken>,
) -> Result<std::path::PathBuf, String> {
    if heif_has_tile_grid(source)? {
        check_heif_tile_grid_support(ffmpeg_version(ffmpeg))?;
    }

    // UUID-based temp path — unique across concurrent uploads.
    let output = std::env::temp_dir().join(format!("buzz-heic-{}.jpg", uuid::Uuid::new_v4()));

    // Single-frame image decode — 60s is generous even for large HEICs.
    let heic_timeout = std::time::Duration::from_secs(60);

    let result = run_ffmpeg_with_cancellation(
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
            .args(HEIC_OUTPUT_ARGS)
            .arg(&output)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped()),
        heic_timeout,
        cancellation,
    )
    .inspect_err(|_| {
        let _ = std::fs::remove_file(&output);
    })?;

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
    transcode_heic_path_to_jpeg_bytes_with_cancellation(source, None)
}

pub(super) fn transcode_heic_path_to_jpeg_bytes_with_cancellation(
    source: &std::path::Path,
    cancellation: Option<&CancellationToken>,
) -> Result<Vec<u8>, String> {
    let ffmpeg_path = find_ffmpeg()?;
    let jpeg_path = transcode_heic_to_jpeg(source, &ffmpeg_path, cancellation)?;
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
fn extract_poster_frame_with_cancellation(
    mp4_path: &std::path::Path,
    ffmpeg: &std::path::Path,
    cancellation: Option<&CancellationToken>,
) -> Result<std::path::PathBuf, String> {
    let output = std::env::temp_dir().join(format!("buzz-poster-{}.jpg", uuid::Uuid::new_v4()));

    // Poster extraction is a single-frame decode — 30s is generous.
    let poster_timeout = std::time::Duration::from_secs(30);

    // Try seeking to 1s first (avoids black first frames from fade-ins).
    let result = run_ffmpeg_with_cancellation(
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
        cancellation,
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
        let fallback = run_ffmpeg_with_cancellation(
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
            cancellation,
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
    transcode_and_extract_poster_with_cancellation(source, None)
}

pub(super) fn transcode_and_extract_poster_with_cancellation(
    source: &std::path::Path,
    cancellation: Option<&CancellationToken>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let ffmpeg_path = find_ffmpeg()?;
    let transcoded = transcode_to_mp4_with_cancellation(source, &ffmpeg_path, cancellation)?;

    // Extract poster from the transcoded file (not the original — guarantees decodability).
    let poster_bytes =
        match extract_poster_frame_with_cancellation(&transcoded, &ffmpeg_path, cancellation) {
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

    if cancellation.is_some_and(CancellationToken::is_cancelled) {
        let _ = std::fs::remove_file(&transcoded);
        return Err("upload cancelled".to_string());
    }

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

        let output =
            transcode_to_mp4_with_cancellation(&source, &ffmpeg, None).expect("transcode fixture");
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

    #[test]
    fn test_voice_note_envelope_passes_relay_video_validation() {
        if find_ffmpeg().is_err() {
            eprintln!("skipping voice-note round-trip: ffmpeg not found");
            return;
        }

        let source =
            std::env::temp_dir().join(format!("buzz-voice-test-{}.wav", uuid::Uuid::new_v4()));
        let sample_rate = 24_000u32;
        let sample_bytes = sample_rate as usize * 2;
        let mut wav = Vec::with_capacity(44 + sample_bytes);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + sample_bytes as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(sample_bytes as u32).to_le_bytes());
        wav.resize(44 + sample_bytes, 0);
        std::fs::write(&source, wav).expect("write voice-note fixture");

        let output = match transcode_voice_note_to_mp4_with_cancellation(&source, None) {
            Ok(output) => output,
            Err(error) => {
                eprintln!("skipping voice-note round-trip: {error}");
                let _ = std::fs::remove_file(&source);
                return;
            }
        };
        let relay_config = buzz_media_pkg::MediaConfig {
            s3_endpoint: String::new(),
            s3_access_key: String::new(),
            s3_secret_key: String::new(),
            s3_bucket: String::new(),
            s3_region: "us-east-1".to_string(),
            s3_addressing_style: buzz_media_pkg::S3AddressingStyle::Path,
            max_image_bytes: 50 * 1024 * 1024,
            max_gif_bytes: 10 * 1024 * 1024,
            max_video_bytes: 524_288_000,
            max_file_bytes: 104_857_600,
            public_base_url: String::new(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        };
        let metadata = buzz_media_pkg::validation::validate_video_file(&output, &relay_config)
            .expect("relay rejected the canonical voice-note envelope");
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&output);

        assert!(metadata.has_audio);
        assert_eq!((metadata.width, metadata.height), (16, 16));
        assert!(metadata.duration_secs > 0.0);
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

    /// Serialize an ISO-BMFF box with a 32-bit size header.
    fn bmff_box(box_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = u32::try_from(payload.len() + 8).expect("box fits u32");
        let mut buf = size.to_be_bytes().to_vec();
        buf.extend_from_slice(box_type);
        buf.extend_from_slice(payload);
        buf
    }

    /// Serialize an ISO-BMFF box with a 64-bit `largesize` header.
    fn bmff_large_box(box_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut buf = 1u32.to_be_bytes().to_vec();
        buf.extend_from_slice(box_type);
        buf.extend_from_slice(&(payload.len() as u64 + 16).to_be_bytes());
        buf.extend_from_slice(payload);
        buf
    }

    /// `infe` version 2 entry: item_ID, protection index, item_type, empty name.
    fn infe_v2(item_id: u16, item_type: &[u8; 4]) -> Vec<u8> {
        let mut payload = vec![2, 0, 0, 0];
        payload.extend_from_slice(&item_id.to_be_bytes());
        payload.extend_from_slice(&0u16.to_be_bytes());
        payload.extend_from_slice(item_type);
        payload.push(0);
        bmff_box(b"infe", &payload)
    }

    /// `meta` box (version 0) with `pitm` naming the 1-based `primary` entry
    /// of `item_types` (IDs are assigned 1, 2, 3, …) and an `iinf` listing them.
    fn meta_box(primary: Option<u16>, item_types: &[&[u8; 4]]) -> Vec<u8> {
        let mut iinf = vec![0, 0, 0, 0];
        iinf.extend_from_slice(&(item_types.len() as u16).to_be_bytes());
        for (i, item_type) in item_types.iter().enumerate() {
            iinf.extend(infe_v2(i as u16 + 1, item_type));
        }
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(bmff_box(b"hdlr", &[0; 20]));
        if let Some(primary) = primary {
            let mut pitm = vec![0, 0, 0, 0];
            pitm.extend_from_slice(&primary.to_be_bytes());
            meta.extend(bmff_box(b"pitm", &pitm));
        }
        meta.extend(bmff_box(b"iinf", &iinf));
        bmff_box(b"meta", &meta)
    }

    /// iPhone-style item layout: 48 HEVC tiles behind one `grid` primary item.
    fn iphone_grid_items() -> Vec<&'static [u8; 4]> {
        let mut items: Vec<&'static [u8; 4]> = vec![b"grid"];
        items.extend(std::iter::repeat_n(b"hvc1", 48));
        items.push(b"Exif");
        items
    }

    /// Synthetic iPhone-style HEIC: `ftyp`, `meta` with a primary `grid`, `mdat`.
    fn iphone_grid_heic() -> Vec<u8> {
        let mut heic = ftyp_box(b"heic", &[b"mif1"]);
        heic[3] = 20;
        heic.extend(meta_box(Some(1), &iphone_grid_items()));
        heic.extend(bmff_box(b"mdat", &[0xAB; 100]));
        heic
    }

    /// Synthetic single-image HEIC with no derived items.
    fn single_image_heic() -> Vec<u8> {
        let mut heic = ftyp_box(b"heic", &[b"mif1"]);
        heic[3] = 20;
        heic.extend(meta_box(Some(1), &[b"hvc1", b"Exif"]));
        heic.extend(bmff_box(b"mdat", &[0xAB; 100]));
        heic
    }

    #[test]
    fn test_heif_meta_primary_item_is_grid() {
        let grid = meta_box(Some(1), &iphone_grid_items());
        assert!(heif_meta_primary_item_is_grid(&grid[8..]));

        let single = meta_box(Some(1), &[b"hvc1", b"Exif"]);
        assert!(!heif_meta_primary_item_is_grid(&single[8..]));

        // A grid that is only an alternate (primary is a plain image) decodes
        // identically on every ffmpeg, so it must not trip the gate.
        let alternate = meta_box(Some(2), &[b"grid", b"hvc1", b"hvc1"]);
        assert!(!heif_meta_primary_item_is_grid(&alternate[8..]));

        // No `pitm`: fall back to "any grid item".
        let no_pitm = meta_box(None, &[b"hvc1", b"grid"]);
        assert!(heif_meta_primary_item_is_grid(&no_pitm[8..]));

        assert!(!heif_meta_primary_item_is_grid(&[]));
        assert!(!heif_meta_primary_item_is_grid(&grid[8..grid.len() / 2]));
    }

    #[test]
    fn test_heif_meta_primary_item_is_grid_wide_ids() {
        // `pitm` version 1 (u32 item_ID), `iinf` version 1 (u32 count),
        // `infe` version 3 (u32 item_ID).
        let mut infe = vec![3, 0, 0, 0];
        infe.extend_from_slice(&70_000u32.to_be_bytes());
        infe.extend_from_slice(&0u16.to_be_bytes());
        infe.extend_from_slice(b"grid");
        infe.push(0);
        let mut iinf = vec![1, 0, 0, 0];
        iinf.extend_from_slice(&1u32.to_be_bytes());
        iinf.extend(bmff_box(b"infe", &infe));
        let mut pitm = vec![1, 0, 0, 0];
        pitm.extend_from_slice(&70_000u32.to_be_bytes());
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(bmff_box(b"pitm", &pitm));
        meta.extend(bmff_box(b"iinf", &iinf));
        assert!(heif_meta_primary_item_is_grid(&meta));

        let mut other_primary = vec![1, 0, 0, 0];
        other_primary.extend_from_slice(&70_001u32.to_be_bytes());
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(bmff_box(b"pitm", &other_primary));
        meta.extend(bmff_box(b"iinf", &iinf));
        assert!(!heif_meta_primary_item_is_grid(&meta));
    }

    #[test]
    fn test_heif_has_tile_grid_walks_top_level_boxes() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("buzz-test-{}.heic", uuid::Uuid::new_v4()));

        // ftyp, then a largesize mdat *before* meta (exercises the seek path),
        // then meta with a grid, then a size-0 (to EOF) mdat.
        let mut file = ftyp_box(b"heic", &[b"mif1"]);
        file[3] = 20;
        file.extend(bmff_large_box(b"mdat", &[0xAB; 100]));
        file.extend(meta_box(Some(1), &iphone_grid_items()));
        file.extend([0, 0, 0, 0]);
        file.extend_from_slice(b"mdat");
        file.extend([0xCD; 50]);
        std::fs::write(&path, &file).expect("write grid fixture");
        assert!(is_heic_file(&file));
        assert_eq!(heif_has_tile_grid(&path), Ok(true));

        let grid = iphone_grid_heic();
        std::fs::write(&path, &grid).expect("write grid fixture");
        assert!(is_heic_file(&grid));
        assert_eq!(heif_has_tile_grid(&path), Ok(true));

        std::fs::write(&path, single_image_heic()).expect("write single fixture");
        assert_eq!(heif_has_tile_grid(&path), Ok(false));

        std::fs::write(&path, b"\xFF\xD8\xFF\xE0").expect("write junk");
        assert_eq!(heif_has_tile_grid(&path), Ok(false));
        let _ = std::fs::remove_file(&path);

        assert!(heif_has_tile_grid(&dir.join("buzz-test-missing.heic")).is_err());
    }

    #[test]
    fn test_parse_ffmpeg_version() {
        let cases = [
            (
                "ffmpeg version 8.1.3-static https://johnvansickle.com/ffmpeg/",
                Some((8, 1)),
            ),
            (
                "ffmpeg version n7.1.5-12-g1fdbca85aa-20260731 Copyright",
                Some((7, 1)),
            ),
            (
                "ffmpeg version 7.1.1-1ubuntu1 Copyright (c) 2000-2025",
                Some((7, 1)),
            ),
            (
                "ffmpeg version 8.0 Copyright (c) 2000-2025 the FFmpeg developers",
                Some((8, 0)),
            ),
            (
                "ffmpeg version 8.1-full_build-www.gyan.dev Copyright",
                Some((8, 1)),
            ),
            ("ffmpeg version 10.0.1 Copyright", Some((10, 0))),
            (
                "ffmpeg version 4.4.2-0ubuntu0.22.04.1 Copyright",
                Some((4, 4)),
            ),
            (
                "ffmpeg version N-121557-gc0f65ff9c3-20260115 Copyright",
                None,
            ),
            ("ffmpeg version 2026-01-15-git-abcdef1234-full_build", None),
            ("", None),
            ("ffprobe version 8.1.3", None),
        ];
        for (line, expected) in cases {
            assert_eq!(parse_ffmpeg_version(line), expected, "{line:?}");
        }
    }

    #[test]
    fn test_check_heif_tile_grid_support() {
        assert_eq!(check_heif_tile_grid_support(Some((8, 1))), Ok(()));
        assert_eq!(check_heif_tile_grid_support(Some((9, 0))), Ok(()));
        assert_eq!(check_heif_tile_grid_support(Some((10, 0))), Ok(()));
        for version in [Some((8, 0)), Some((7, 1)), Some((4, 4)), None] {
            let err =
                check_heif_tile_grid_support(version).expect_err("tile grids need ffmpeg 8.1+");
            assert!(err.contains("ffmpeg 8.1"), "{err}");
            assert!(err.contains("tile grid"), "{err}");
        }
        assert!(check_heif_tile_grid_support(Some((7, 1)))
            .expect_err("7.1 is too old")
            .contains("found 7.1"));
    }

    #[test]
    fn test_heic_output_args_do_not_pin_first_tile_stream() {
        // `-map 0:v:0` selects the first 512×512 tile of an iPhone HEIC instead
        // of the assembled grid; stream selection must be left to ffmpeg.
        assert!(!HEIC_OUTPUT_ARGS.contains(&"-map"), "{HEIC_OUTPUT_ARGS:?}");
        assert!(
            HEIC_OUTPUT_ARGS.contains(&"-frames:v"),
            "{HEIC_OUTPUT_ARGS:?}"
        );
    }

    /// Drive `transcode_heic_to_jpeg` against a stub ffmpeg that reports a
    /// configurable version and records the arguments of any transcode call.
    #[cfg(unix)]
    fn run_heic_transcode_with_stub_ffmpeg(
        version_line: &str,
        heic: &[u8],
    ) -> (Result<std::path::PathBuf, String>, Option<String>) {
        use std::os::unix::fs::PermissionsExt;

        let id = uuid::Uuid::new_v4();
        let dir = std::env::temp_dir();
        let stub = dir.join(format!("buzz-test-ffmpeg-{id}.sh"));
        let args_log = dir.join(format!("buzz-test-ffmpeg-{id}.args"));
        let heic_path = dir.join(format!("buzz-test-{id}.heic"));
        std::fs::write(
            &stub,
            format!(
                "#!/bin/sh\nif [ \"$1\" = -version ]; then echo '{version_line}'; exit 0; fi\n\
                 printf '%s\\n' \"$@\" > '{}'\nexit 1\n",
                args_log.display()
            ),
        )
        .expect("write stub ffmpeg");
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755))
            .expect("chmod stub ffmpeg");
        std::fs::write(&heic_path, heic).expect("write heic fixture");

        let result = transcode_heic_to_jpeg(&heic_path, &stub, None);
        let recorded = std::fs::read_to_string(&args_log).ok();

        let _ = std::fs::remove_file(&stub);
        let _ = std::fs::remove_file(&args_log);
        let _ = std::fs::remove_file(&heic_path);
        (result, recorded)
    }

    #[cfg(unix)]
    #[test]
    fn test_transcode_heic_refuses_tile_grid_on_old_ffmpeg() {
        let grid = iphone_grid_heic();

        let (result, recorded) =
            run_heic_transcode_with_stub_ffmpeg("ffmpeg version 7.1.1-1ubuntu1", &grid);
        let err = result.expect_err("tiled HEIC on ffmpeg 7.1 must not transcode");
        assert!(err.contains("ffmpeg 8.1"), "{err}");
        assert!(
            recorded.is_none(),
            "ffmpeg was invoked despite the version gate: {recorded:?}"
        );

        let (result, recorded) =
            run_heic_transcode_with_stub_ffmpeg("ffmpeg version N-121557-gc0f65ff9c3", &grid);
        assert!(
            result.is_err() && recorded.is_none(),
            "unknown version must be refused"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_transcode_heic_lets_ffmpeg_select_tile_grid_stream() {
        let grid = iphone_grid_heic();

        let (result, recorded) =
            run_heic_transcode_with_stub_ffmpeg("ffmpeg version 8.1.3-static", &grid);
        // The stub exits non-zero, so the transcode reports failure — what
        // matters is that it ran, and with which arguments.
        assert!(result.is_err());
        let recorded = recorded.expect("ffmpeg 8.1 should be invoked for a tiled HEIC");
        let args: Vec<&str> = recorded.lines().collect();
        assert!(
            !args.contains(&"-map"),
            "explicit -map pins one tile: {args:?}"
        );
        assert!(args.contains(&"-frames:v"), "{args:?}");
        assert!(args.contains(&"-map_metadata"), "{args:?}");

        // Non-tiled HEICs skip the version gate entirely.
        let (_, recorded) =
            run_heic_transcode_with_stub_ffmpeg("ffmpeg version 7.1.1", &single_image_heic());
        assert!(
            recorded.is_some(),
            "single-image HEIC must still transcode on ffmpeg 7.1"
        );
    }
}
