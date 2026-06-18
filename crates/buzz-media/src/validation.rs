//! Content validation — magic bytes, allowlist, size, image bomb protection, video metadata.

use std::io::{BufReader, Seek, SeekFrom};
use std::path::Path;

use crate::config::MediaConfig;
use crate::error::MediaError;

/// Accepted MIME types for the image upload path.
///
/// `video/mp4` is intentionally excluded — video uploads use a separate pipeline
/// (`process_video_upload`) with its own magic-byte check. If an MP4 is uploaded
/// through the image path (Content-Type spoofing), `infer::get()` detects
/// `video/mp4` and `validate_content()` rejects it here.
const ALLOWED_MIME_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// MIME types blocked from the generic file-upload path.
///
/// These are the formats a browser (or the desktop webview) will *execute* or
/// *render as active content* if it ever reaches them with the wrong response
/// headers. We serve generic files with `Content-Disposition: attachment` +
/// `X-Content-Type-Options: nosniff` + `CSP: default-src 'none'`, which already
/// neutralises them — this allowlist-of-denials is defence in depth, so a future
/// header regression can't turn an uploaded blob into a stored-XSS vector.
///
/// HTML, JS, and SVG are the classic stored-XSS carriers. Native executables are
/// blocked because there's no legitimate reason to host them inline in chat and
/// they're a malware-distribution risk.
const BLOCKED_FILE_MIME_TYPES: &[&str] = &[
    // Active web content — stored-XSS vectors.
    "text/html",
    "application/xhtml+xml",
    "image/svg+xml",
    "application/javascript",
    "text/javascript",
    // Native executables / installers.
    "application/x-msdownload", // .exe / .dll
    "application/x-executable", // ELF
    "application/vnd.microsoft.portable-executable",
    "application/x-mach-binary", // Mach-O
    "application/x-sharedlib",
    "application/x-elf",
    "application/x-msi",
    "application/vnd.android.package-archive", // .apk
    "application/x-apple-diskimage",           // .dmg
];

/// Map a sniffed MIME type to a file extension for the generic file path.
///
/// Covers the common document, archive, audio, and data formats `infer`
/// recognises. Returns `None` for MIME types we don't have a canonical
/// extension for — the caller falls back to `bin`.
fn file_mime_to_ext(mime: &str) -> Option<&'static str> {
    let ext = match mime {
        // Documents
        "application/pdf" => "pdf",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.oasis.opendocument.text" => "odt",
        "application/vnd.oasis.opendocument.spreadsheet" => "ods",
        "application/vnd.oasis.opendocument.presentation" => "odp",
        "application/rtf" => "rtf",
        "application/epub+zip" => "epub",
        // Archives
        "application/zip" => "zip",
        "application/gzip" => "gz",
        "application/x-tar" => "tar",
        "application/x-7z-compressed" => "7z",
        "application/x-rar-compressed" | "application/vnd.rar" => "rar",
        "application/x-bzip2" => "bz2",
        "application/x-xz" => "xz",
        "application/zstd" => "zst",
        // Audio
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/ogg" => "ogg",
        "audio/aac" => "aac",
        "audio/opus" => "opus",
        // Other media containers (served as downloads, not transcoded)
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        // Data / text
        "application/json" => "json",
        "text/csv" => "csv",
        "text/plain" => "txt",
        _ => return None,
    };
    Some(ext)
}

/// Validate uploaded bytes for the **generic file** upload path.
///
/// This is the catch-all path for non-image, non-video attachments (documents,
/// archives, audio, text, data). It enforces three things:
///   1. A size cap (`config.max_file_bytes`).
///   2. A *deny* list — known active-content and executable MIME types are
///      rejected even though safe headers already neutralise them.
///   3. Magic-byte sniffing where possible.
///
/// Files with no detectable signature (plain text, CSV, source code, JSON —
/// none of which have magic bytes) are accepted as `application/octet-stream`.
/// They are always served as downloads, so an un-sniffable file can never
/// execute in the app.
///
/// Returns `(mime, ext)`.
pub fn validate_file_content(
    bytes: &[u8],
    config: &MediaConfig,
) -> Result<(String, String), MediaError> {
    // 1. Size cap.
    if bytes.len() as u64 > config.max_file_bytes {
        return Err(MediaError::FileTooLarge {
            size: bytes.len() as u64,
            max: config.max_file_bytes,
        });
    }

    // 2. Sniff. `None` means no magic signature (text/csv/json/source) — that's
    //    fine for the generic path; treat as opaque binary served as a download.
    match infer::get(bytes) {
        Some(kind) => {
            let mime = kind.mime_type().to_string();
            // 3. Deny dangerous active-content / executable types.
            if BLOCKED_FILE_MIME_TYPES.contains(&mime.as_str()) {
                return Err(MediaError::DisallowedContentType(mime));
            }
            let ext = file_mime_to_ext(&mime)
                .map(str::to_string)
                .unwrap_or_else(|| kind.extension().to_string());
            Ok((mime, ext))
        }
        None => Ok(("application/octet-stream".to_string(), "bin".to_string())),
    }
}

/// Whether a stored blob should be served inline (rendered in the client) or as
/// an attachment (forced download).
///
/// Images and video are previewed inline by the renderer; everything else is a
/// generic file card with a download action, so it serves as an attachment.
/// PDF is intentionally *not* inline yet — inline PDF preview is a planned
/// fast-follow; until the renderer handles it, force download like any other file.
pub fn serve_inline(mime: &str) -> bool {
    mime.starts_with("image/") || mime.starts_with("video/")
}

/// Metadata extracted from a validated MP4 file.
#[derive(Debug, Clone)]
pub struct VideoMeta {
    /// Duration in seconds (from mvhd timescale — not edit lists).
    pub duration_secs: f64,
    /// Width of the first video track in pixels.
    pub width: u32,
    /// Height of the first video track in pixels.
    pub height: u32,
    /// Whether the file contains at least one audio track.
    pub has_audio: bool,
}

/// Validate uploaded bytes for the **image** upload path.
///
/// Checks magic bytes, MIME allowlist (images only), size, and pixel dimensions.
/// Rejects `video/mp4` — video uploads must use [`process_video_upload`] which
/// has its own magic-byte check and full MP4 validation pipeline.
pub fn validate_content(bytes: &[u8], config: &MediaConfig) -> Result<String, MediaError> {
    // 1. Magic bytes — never trust Content-Type header
    let mime = infer::get(bytes)
        .map(|t| t.mime_type().to_string())
        .ok_or(MediaError::UnknownContentType)?;

    // 2. Allowlist (SVG, PDF, executables all rejected)
    if !ALLOWED_MIME_TYPES.contains(&mime.as_str()) {
        return Err(MediaError::DisallowedContentType(mime));
    }

    // 3. Size cap (images only — video uses its own size enforcement in the streaming pipeline)
    let max = if mime == "image/gif" {
        config.max_gif_bytes
    } else {
        config.max_image_bytes
    };
    if bytes.len() as u64 > max {
        return Err(MediaError::FileTooLarge {
            size: bytes.len() as u64,
            max,
        });
    }

    // 4. Image bomb — check pixel dimensions before full decode.
    //    Fail closed: imagesize supports JPEG, PNG, GIF, WebP. If dimensions
    //    can't be parsed, reject — don't let unknown-geometry images reach the
    //    full decoder in thumbnail generation.
    const MAX_PIXELS: u64 = 25_000_000; // 25 megapixels — 100MB max RGBA decode
    let size = imagesize::blob_size(bytes).map_err(|_| MediaError::InvalidImage)?;
    if (size.width as u64) * (size.height as u64) > MAX_PIXELS {
        return Err(MediaError::ImageTooLarge);
    }

    Ok(mime)
}

/// Validate an MP4 file on disk.
///
/// Checks:
/// - Container is MP4 (ftyp brand is not QuickTime `qt  `)
/// - First video track codec is `avc1` (H.264 only — rejects HEVC, VP9, AV1)
/// - Duration ≤ 600 seconds (from mvhd timescale, not edit lists)
/// - Resolution ≤ 3840×2160
/// - moov atom precedes mdat (fast-start / web-optimised)
///
/// Returns [`VideoMeta`] on success.
pub fn validate_video_file(path: &Path, config: &MediaConfig) -> Result<VideoMeta, MediaError> {
    // --- moov-before-mdat check (raw byte scan) ---
    // We scan the top-level atom sequence before handing off to the mp4 crate,
    // because the mp4 crate parses the whole file regardless of atom order.
    check_moov_before_mdat(path)?;

    let file = std::fs::File::open(path).map_err(|e| MediaError::Io(e.to_string()))?;
    let size = file
        .metadata()
        .map_err(|e| MediaError::Io(e.to_string()))?
        .len();

    // Size guard (belt-and-suspenders — the streaming layer also enforces this).
    if size > config.max_video_bytes {
        return Err(MediaError::FileTooLarge {
            size,
            max: config.max_video_bytes,
        });
    }

    let reader = BufReader::new(file);
    let mp4 = mp4::Mp4Reader::read_header(reader, size).map_err(|_| MediaError::InvalidVideo)?;

    // --- Container check ---
    // QuickTime (MOV) uses brand "qt  ". We reject it — only ISO-base MP4.
    // The mp4 crate exposes the ftyp major brand via mp4.major_brand().
    let brand = mp4.major_brand();
    let qt_brand = mp4::FourCC::from(*b"qt  ");
    if *brand == qt_brand {
        return Err(MediaError::UnsupportedContainer);
    }

    // --- Track inspection ---
    let mut video_meta: Option<VideoMeta> = None;
    let mut has_audio = false;

    for track in mp4.tracks().values() {
        match track.track_type().map_err(|_| MediaError::InvalidVideo)? {
            mp4::TrackType::Video => {
                if video_meta.is_some() {
                    // Already found a video track — use the first one only.
                    continue;
                }

                // Codec check: only H.264 (avc1).
                // media_type() reads the handler type and sample entry box type.
                let media_type = track.media_type().map_err(|_| MediaError::InvalidVideo)?;
                if media_type != mp4::MediaType::H264 {
                    return Err(MediaError::WrongCodec);
                }

                // Duration from mvhd timescale (track duration / timescale).
                // Reject zero/negative (malformed) and >600s (too long).
                // Must match imeta validation which requires duration > 0.0.
                // Guard: timescale=0 causes division-by-zero in the mp4 crate's
                // duration() method. Fail fast before it panics.
                if track.timescale() == 0 {
                    return Err(MediaError::InvalidVideo);
                }
                let duration_ms = track.duration().as_millis();
                let duration_secs = duration_ms as f64 / 1000.0;
                if duration_secs <= 0.0 {
                    return Err(MediaError::InvalidVideo);
                }
                if duration_secs > 600.0 {
                    return Err(MediaError::DurationTooLong);
                }

                // Resolution check.
                let width = track.width() as u32;
                let height = track.height() as u32;
                if width > 3840 || height > 2160 {
                    return Err(MediaError::ResolutionTooHigh);
                }

                video_meta = Some(VideoMeta {
                    duration_secs,
                    width,
                    height,
                    has_audio: false, // filled in after audio scan
                });
            }
            mp4::TrackType::Audio => {
                has_audio = true;
            }
            _ => {}
        }
    }

    let mut meta = video_meta.ok_or(MediaError::InvalidVideo)?;
    meta.has_audio = has_audio;
    Ok(meta)
}

/// Scan the top-level atom sequence to verify moov appears before mdat.
///
/// Reads only the 8-byte atom headers (size + fourcc) — never loads atom bodies.
/// Extended-size atoms (size==1) are handled by reading the 64-bit size field.
/// Iteration is capped to prevent DoS from crafted files with millions of tiny atoms.
fn check_moov_before_mdat(path: &Path) -> Result<(), MediaError> {
    use std::io::Read;

    /// Maximum top-level atoms to scan before giving up.
    /// A normal MP4 has < 20 top-level atoms. 1024 is generous but bounded.
    const MAX_ATOMS: u32 = 1024;

    let mut file = std::fs::File::open(path).map_err(|e| MediaError::Io(e.to_string()))?;
    let file_size = file
        .metadata()
        .map_err(|e| MediaError::Io(e.to_string()))?
        .len();

    let mut offset: u64 = 0;
    let mut moov_seen = false;
    let mut atoms_scanned: u32 = 0;

    while offset < file_size {
        atoms_scanned += 1;
        if atoms_scanned > MAX_ATOMS {
            // Fail closed: too many top-level atoms is abnormal. A crafted file
            // could hide mdat after 1025 junk atoms to bypass the moov check.
            return Err(MediaError::MoovNotAtFront);
        }

        file.seek(SeekFrom::Start(offset))
            .map_err(|e| MediaError::Io(e.to_string()))?;

        let mut header = [0u8; 8];
        match file.read_exact(&mut header) {
            Ok(_) => {}
            Err(_) => break, // truncated file — mp4 parser will catch it
        }

        let compact_size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
        let fourcc = &header[4..8];

        // Resolve actual atom size.
        let atom_size = if compact_size == 1 {
            // Extended size: next 8 bytes are the real 64-bit size (includes the 16-byte header).
            let mut ext = [0u8; 8];
            match file.read_exact(&mut ext) {
                Ok(_) => {}
                Err(_) => break, // truncated — mp4 parser will catch it
            }
            let extended = u64::from_be_bytes(ext);
            if extended < 16 {
                break; // malformed extended size — mp4 parser will reject
            }
            extended
        } else if compact_size == 0 {
            // atom_size == 0 means "extends to EOF" — this is the last atom.
            // Check fourcc before stopping: mdat-at-EOF without prior moov is an error.
            if fourcc == b"mdat" && !moov_seen {
                return Err(MediaError::MoovNotAtFront);
            }
            break;
        } else if compact_size < 8 {
            break; // malformed — mp4 parser will reject
        } else {
            compact_size
        };

        match fourcc {
            b"moov" => {
                moov_seen = true;
            }
            b"mdat" if !moov_seen => {
                return Err(MediaError::MoovNotAtFront);
            }
            _ => {}
        }

        offset += atom_size;
    }

    Ok(())
}

/// Map MIME type to file extension.
pub fn mime_to_ext(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        _ => "bin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> MediaConfig {
        MediaConfig {
            s3_endpoint: String::new(),
            s3_access_key: String::new(),
            s3_secret_key: String::new(),
            s3_bucket: String::new(),
            max_image_bytes: 50 * 1024 * 1024,
            max_gif_bytes: 10 * 1024 * 1024,
            max_video_bytes: 524_288_000,
            max_file_bytes: 104_857_600,
            public_base_url: String::new(),
            server_domain: None,
        }
    }

    // Minimal valid JPEG: SOI + APP0 + SOF0 (1x1px).
    // SOF0 is required for imagesize to parse dimensions (fail-closed check).
    const TINY_JPEG: &[u8] = &[
        // SOI
        0xFF, 0xD8, // APP0 (JFIF marker)
        0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01, 0x00,
        0x01, 0x00, 0x00, // SOF0: precision=8, height=1, width=1, components=1
        0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00, // EOI
        0xFF, 0xD9,
    ];

    // Minimal PNG header
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE,
    ];

    #[test]
    fn test_validate_jpeg() {
        let config = test_config();
        let result = validate_content(TINY_JPEG, &config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "image/jpeg");
    }

    #[test]
    fn test_validate_png() {
        let config = test_config();
        let result = validate_content(TINY_PNG, &config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "image/png");
    }

    #[test]
    fn test_validate_svg_rejected() {
        let config = test_config();
        // SVG starts with XML declaration — infer won't detect it as image
        let svg = b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"></svg>";
        let result = validate_content(svg, &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_oversized() {
        let mut config = test_config();
        config.max_image_bytes = 10; // 10 bytes max
        let result = validate_content(TINY_JPEG, &config);
        assert!(matches!(result, Err(MediaError::FileTooLarge { .. })));
    }

    // Minimal valid GIF89a (1x1 pixel) — full logical screen descriptor so imagesize can parse.
    const TINY_GIF: &[u8] = &[
        // Header
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61,
        // Logical Screen Descriptor: width=1, height=1, flags, bgcolor, aspect
        0x01, 0x00, 0x01, 0x00, 0x80, 0x00,
        0x00, // Global Color Table (2 colors: white, black)
        0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, // Image Descriptor
        0x2C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, // Image Data
        0x02, 0x02, 0x4C, 0x01, 0x00, // Trailer
        0x3B,
    ];

    #[test]
    fn test_validate_gif_cap() {
        let mut config = test_config();
        config.max_gif_bytes = 5; // tiny cap
        config.max_image_bytes = 50 * 1024 * 1024;
        let result = validate_content(TINY_GIF, &config);
        assert!(matches!(result, Err(MediaError::FileTooLarge { .. })));
    }

    #[test]
    fn test_validate_gif_ok() {
        let config = test_config();
        let result = validate_content(TINY_GIF, &config);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "image/gif");
    }

    #[test]
    fn test_mime_to_ext() {
        assert_eq!(mime_to_ext("image/jpeg"), "jpg");
        assert_eq!(mime_to_ext("image/png"), "png");
        assert_eq!(mime_to_ext("image/gif"), "gif");
        assert_eq!(mime_to_ext("image/webp"), "webp");
        assert_eq!(mime_to_ext("video/mp4"), "mp4");
        assert_eq!(mime_to_ext("application/pdf"), "bin");
    }

    // --- MP4 magic bytes test ---
    // A minimal ftyp box that infer recognises as video/mp4.
    // ftyp: size=20, 'ftyp', major_brand='isom', minor_version=0, compatible=['isom']
    const MP4_FTYP_MAGIC: &[u8] = &[
        0x00, 0x00, 0x00, 0x14, // size = 20
        0x66, 0x74, 0x79, 0x70, // 'ftyp'
        0x69, 0x73, 0x6F, 0x6D, // major brand: 'isom'
        0x00, 0x00, 0x00, 0x00, // minor version
        0x69, 0x73, 0x6F, 0x6D, // compatible brand: 'isom'
        // padding to ensure infer has enough bytes
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn test_validate_mp4_magic_bytes_rejected() {
        // MP4 uploaded through the image path must be rejected — video/mp4 is
        // not in ALLOWED_MIME_TYPES. This prevents Content-Type spoofing attacks
        // where an MP4 is uploaded as image/jpeg to bypass video validation.
        let config = test_config();
        let result = validate_content(MP4_FTYP_MAGIC, &config);
        match result {
            Err(MediaError::DisallowedContentType(mime)) => {
                assert_eq!(mime, "video/mp4");
            }
            Err(MediaError::UnknownContentType) => {
                // infer needs more bytes — acceptable, still rejected
            }
            Ok(mime) => panic!("MP4 should be rejected by image path, got Ok({mime})"),
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }

    // --- validate_video_file tests ---
    // These tests use real MP4 files written to a NamedTempFile.
    // We build minimal but structurally valid MP4 boxes by hand.

    /// Build a minimal fast-start MP4 with moov before mdat.
    /// Contains one H.264 video track (avc1), 1 second, 320x240.
    fn build_minimal_mp4_moov_first() -> Vec<u8> {
        build_mp4_bytes(true, b"avc1", 1_000, 320, 240, false)
    }

    /// Build an MP4 with mdat before moov (not fast-start).
    fn build_minimal_mp4_mdat_first() -> Vec<u8> {
        build_mp4_bytes(false, b"avc1", 1_000, 320, 240, false)
    }

    /// Build an MP4 with HEVC codec (hev1 — the box type the mp4 crate recognises).
    fn build_mp4_hevc() -> Vec<u8> {
        build_mp4_bytes(true, b"hev1", 1_000, 320, 240, false)
    }

    /// Build an MP4 with duration > 600s.
    fn build_mp4_too_long() -> Vec<u8> {
        build_mp4_bytes(true, b"avc1", 601_000, 320, 240, false)
    }

    /// Build an MP4 with resolution > 3840x2160.
    fn build_mp4_too_large() -> Vec<u8> {
        build_mp4_bytes(true, b"avc1", 1_000, 3841, 2161, false)
    }

    /// Build an MP4 with audio track.
    fn build_mp4_with_audio() -> Vec<u8> {
        build_mp4_bytes(true, b"avc1", 1_000, 320, 240, true)
    }

    /// Construct a minimal but parseable MP4 byte stream.
    ///
    /// Layout (fast-start): ftyp | moov | mdat
    /// Layout (non-fast-start): ftyp | mdat | moov
    ///
    /// The moov contains:
    ///   mvhd (duration_ms, timescale=1000)
    ///   trak (video: tkhd + mdia[mdhd+hdlr+minf[stbl[stsd[codec_box]]]])
    ///   optionally a second trak (audio: tkhd + mdia[mdhd+hdlr+minf[stbl[stsd[mp4a]]]])
    fn build_mp4_bytes(
        fast_start: bool,
        codec: &[u8; 4],
        duration_ms: u32,
        width: u16,
        height: u16,
        with_audio: bool,
    ) -> Vec<u8> {
        // ftyp box: size(4) + 'ftyp'(4) + major_brand(4) + minor_ver(4) + compat(4)
        let ftyp: Vec<u8> = {
            let mut b = Vec::new();
            b.extend_from_slice(&20u32.to_be_bytes()); // size
            b.extend_from_slice(b"ftyp");
            b.extend_from_slice(b"isom"); // major brand
            b.extend_from_slice(&0u32.to_be_bytes()); // minor version
            b.extend_from_slice(b"isom"); // compatible brand
            b
        };

        // mdat box: just an empty payload (no actual media samples needed for header parse)
        let mdat: Vec<u8> = {
            let mut b = Vec::new();
            b.extend_from_slice(&8u32.to_be_bytes()); // size = 8 (header only)
            b.extend_from_slice(b"mdat");
            b
        };

        // Build moov
        let moov = build_moov(duration_ms, codec, width, height, with_audio);

        let mut out = Vec::new();
        out.extend_from_slice(&ftyp);
        if fast_start {
            out.extend_from_slice(&moov);
            out.extend_from_slice(&mdat);
        } else {
            out.extend_from_slice(&mdat);
            out.extend_from_slice(&moov);
        }
        out
    }

    fn box_wrap(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(&size.to_be_bytes());
        b.extend_from_slice(fourcc);
        b.extend_from_slice(payload);
        b
    }

    fn build_moov(
        duration_ms: u32,
        codec: &[u8; 4],
        width: u16,
        height: u16,
        with_audio: bool,
    ) -> Vec<u8> {
        let timescale: u32 = 1000;
        let duration: u32 = duration_ms;

        // mvhd (version 0): flags(3) + creation(4) + modification(4) + timescale(4) +
        //                    duration(4) + rate(4) + volume(2) + reserved(10) +
        //                    matrix(36) + pre_defined(24) + next_track_id(4) = 100 bytes payload
        let mvhd_payload: Vec<u8> = {
            let mut b = vec![0u8; 4]; // version=0 + flags=0
            b.extend_from_slice(&0u32.to_be_bytes()); // creation_time
            b.extend_from_slice(&0u32.to_be_bytes()); // modification_time
            b.extend_from_slice(&timescale.to_be_bytes());
            b.extend_from_slice(&duration.to_be_bytes());
            b.extend_from_slice(&0x00010000u32.to_be_bytes()); // rate = 1.0
            b.extend_from_slice(&0x0100u16.to_be_bytes()); // volume = 1.0
            b.extend_from_slice(&[0u8; 10]); // reserved
                                             // identity matrix
            b.extend_from_slice(&0x00010000u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0x00010000u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0x40000000u32.to_be_bytes());
            b.extend_from_slice(&[0u8; 24]); // pre_defined
            b.extend_from_slice(&2u32.to_be_bytes()); // next_track_id
            b
        };
        let mvhd = box_wrap(b"mvhd", &mvhd_payload);

        let video_trak = build_video_trak(1, duration, timescale, codec, width, height);

        let mut moov_payload = Vec::new();
        moov_payload.extend_from_slice(&mvhd);
        moov_payload.extend_from_slice(&video_trak);

        if with_audio {
            let audio_trak = build_audio_trak(2, duration, timescale);
            moov_payload.extend_from_slice(&audio_trak);
        }

        box_wrap(b"moov", &moov_payload)
    }

    fn build_video_trak(
        track_id: u32,
        duration: u32,
        timescale: u32,
        codec: &[u8; 4],
        width: u16,
        height: u16,
    ) -> Vec<u8> {
        // tkhd (version 0, flags=3 = enabled+in-movie)
        let tkhd_payload: Vec<u8> = {
            let mut b = vec![0u8, 0u8, 0u8, 3u8]; // version=0, flags=3
            b.extend_from_slice(&0u32.to_be_bytes()); // creation_time
            b.extend_from_slice(&0u32.to_be_bytes()); // modification_time
            b.extend_from_slice(&track_id.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes()); // reserved
            b.extend_from_slice(&duration.to_be_bytes());
            b.extend_from_slice(&[0u8; 8]); // reserved
            b.extend_from_slice(&0i16.to_be_bytes()); // layer
            b.extend_from_slice(&0i16.to_be_bytes()); // alternate_group
            b.extend_from_slice(&0u16.to_be_bytes()); // volume
            b.extend_from_slice(&0u16.to_be_bytes()); // reserved
                                                      // identity matrix
            b.extend_from_slice(&0x00010000u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0x00010000u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0x40000000u32.to_be_bytes());
            // width and height as 16.16 fixed point
            b.extend_from_slice(&((width as u32) << 16).to_be_bytes());
            b.extend_from_slice(&((height as u32) << 16).to_be_bytes());
            b
        };
        let tkhd = box_wrap(b"tkhd", &tkhd_payload);

        let mdia = build_video_mdia(duration, timescale, codec, width, height);
        let trak_payload = {
            let mut b = Vec::new();
            b.extend_from_slice(&tkhd);
            b.extend_from_slice(&mdia);
            b
        };
        box_wrap(b"trak", &trak_payload)
    }

    fn build_video_mdia(
        duration: u32,
        timescale: u32,
        codec: &[u8; 4],
        width: u16,
        height: u16,
    ) -> Vec<u8> {
        // mdhd
        let mdhd_payload: Vec<u8> = {
            let mut b = vec![0u8; 4]; // version=0, flags=0
            b.extend_from_slice(&0u32.to_be_bytes()); // creation_time
            b.extend_from_slice(&0u32.to_be_bytes()); // modification_time
            b.extend_from_slice(&timescale.to_be_bytes());
            b.extend_from_slice(&duration.to_be_bytes());
            b.extend_from_slice(&0u16.to_be_bytes()); // language
            b.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
            b
        };
        let mdhd = box_wrap(b"mdhd", &mdhd_payload);

        // hdlr for video
        let hdlr = build_hdlr(b"vide", b"VideoHandler");

        // minf -> stbl -> stsd -> codec_box
        let minf = build_video_minf(codec, width, height);

        let mdia_payload = {
            let mut b = Vec::new();
            b.extend_from_slice(&mdhd);
            b.extend_from_slice(&hdlr);
            b.extend_from_slice(&minf);
            b
        };
        box_wrap(b"mdia", &mdia_payload)
    }

    fn build_hdlr(handler_type: &[u8; 4], name: &[u8]) -> Vec<u8> {
        let mut payload = vec![0u8; 4]; // version=0, flags=0
        payload.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
        payload.extend_from_slice(handler_type);
        payload.extend_from_slice(&[0u8; 12]); // reserved
        payload.extend_from_slice(name);
        payload.push(0); // null terminator
        box_wrap(b"hdlr", &payload)
    }

    fn build_video_minf(codec: &[u8; 4], width: u16, height: u16) -> Vec<u8> {
        // vmhd
        let vmhd_payload = {
            let mut b = vec![0u8, 0u8, 0u8, 1u8]; // version=0, flags=1
            b.extend_from_slice(&0u16.to_be_bytes()); // graphicsMode
            b.extend_from_slice(&[0u8; 6]); // opcolor
            b
        };
        let vmhd = box_wrap(b"vmhd", &vmhd_payload);

        // dinf -> dref
        let url_payload = vec![0u8, 0u8, 0u8, 1u8]; // version=0, flags=1 (self-contained)
        let url_box = box_wrap(b"url ", &url_payload);
        let dref_payload = {
            let mut b = vec![0u8; 4]; // version=0, flags=0
            b.extend_from_slice(&1u32.to_be_bytes()); // entry_count=1
            b.extend_from_slice(&url_box);
            b
        };
        let dref = box_wrap(b"dref", &dref_payload);
        let dinf = box_wrap(b"dinf", &dref);

        // stbl -> stsd -> codec sample entry
        let stsd = build_video_stsd(codec, width, height);
        // Minimal stts (time-to-sample): 1 entry, 1 sample, duration=1000
        let stts_payload = {
            let mut b = vec![0u8; 4]; // version=0, flags=0
            b.extend_from_slice(&1u32.to_be_bytes()); // entry_count
            b.extend_from_slice(&1u32.to_be_bytes()); // sample_count
            b.extend_from_slice(&1000u32.to_be_bytes()); // sample_delta
            b
        };
        let stts = box_wrap(b"stts", &stts_payload);
        // stsc: 1 chunk, 1 sample per chunk
        let stsc_payload = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&1u32.to_be_bytes()); // first_chunk
            b.extend_from_slice(&1u32.to_be_bytes()); // samples_per_chunk
            b.extend_from_slice(&1u32.to_be_bytes()); // sample_description_index
            b
        };
        let stsc = box_wrap(b"stsc", &stsc_payload);
        // stsz: 1 sample, size=0
        let stsz_payload = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&0u32.to_be_bytes()); // sample_size=0 (variable)
            b.extend_from_slice(&1u32.to_be_bytes()); // sample_count
            b.extend_from_slice(&0u32.to_be_bytes()); // entry_size[0]
            b
        };
        let stsz = box_wrap(b"stsz", &stsz_payload);
        // stco: 1 chunk offset
        let stco_payload = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&28u32.to_be_bytes()); // offset (after ftyp)
            b
        };
        let stco = box_wrap(b"stco", &stco_payload);

        let stbl_payload = {
            let mut b = Vec::new();
            b.extend_from_slice(&stsd);
            b.extend_from_slice(&stts);
            b.extend_from_slice(&stsc);
            b.extend_from_slice(&stsz);
            b.extend_from_slice(&stco);
            b
        };
        let stbl = box_wrap(b"stbl", &stbl_payload);

        let minf_payload = {
            let mut b = Vec::new();
            b.extend_from_slice(&vmhd);
            b.extend_from_slice(&dinf);
            b.extend_from_slice(&stbl);
            b
        };
        box_wrap(b"minf", &minf_payload)
    }

    fn build_video_stsd(codec: &[u8; 4], width: u16, height: u16) -> Vec<u8> {
        // Visual sample entry (avc1/hvc1/etc.)
        // VisualSampleEntry: reserved(6) + data_ref_idx(2) + pre_defined(2) + reserved(2) +
        //   pre_defined(12) + width(2) + height(2) + horiz_res(4) + vert_res(4) +
        //   reserved(4) + frame_count(2) + compressorname(32) + depth(2) + pre_defined(2)
        let mut entry_payload = Vec::new();
        entry_payload.extend_from_slice(&[0u8; 6]); // reserved
        entry_payload.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
        entry_payload.extend_from_slice(&[0u8; 2]); // pre_defined
        entry_payload.extend_from_slice(&[0u8; 2]); // reserved
        entry_payload.extend_from_slice(&[0u8; 12]); // pre_defined
        entry_payload.extend_from_slice(&width.to_be_bytes());
        entry_payload.extend_from_slice(&height.to_be_bytes());
        entry_payload.extend_from_slice(&0x00480000u32.to_be_bytes()); // horiz_res 72dpi
        entry_payload.extend_from_slice(&0x00480000u32.to_be_bytes()); // vert_res 72dpi
        entry_payload.extend_from_slice(&0u32.to_be_bytes()); // reserved
        entry_payload.extend_from_slice(&1u16.to_be_bytes()); // frame_count
        entry_payload.extend_from_slice(&[0u8; 32]); // compressorname
        entry_payload.extend_from_slice(&0x0018u16.to_be_bytes()); // depth
        entry_payload.extend_from_slice(&(-1i16).to_be_bytes()); // pre_defined

        // Append the codec-specific config box.
        if codec == b"avc1" {
            // avcC: minimal (version=1, profile=66/Baseline, compat=0, level=30)
            let avcc_payload = vec![
                0x01, 0x42, 0x00, 0x1E, // configurationVersion, AVCProfileIndication,
                // profile_compatibility, AVCLevelIndication
                0xFF, // lengthSizeMinusOne=3
                0xE1, // numSequenceParameterSets=1
                0x00, 0x00, // sequenceParameterSetLength=0 (empty SPS)
                0x01, // numPictureParameterSets=1
                0x00, 0x00, // pictureParameterSetLength=0 (empty PPS)
            ];
            entry_payload.extend_from_slice(&box_wrap(b"avcC", &avcc_payload));
        } else {
            // hvcC: minimal — configuration_version=1 (1 byte payload).
            // The mp4 crate's HvcCBox::read_box reads exactly 1 byte.
            entry_payload.extend_from_slice(&box_wrap(b"hvcC", &[0x01]));
        }

        let codec_box = box_wrap(codec, &entry_payload);

        let mut stsd_payload = vec![0u8; 4]; // version=0, flags=0
        stsd_payload.extend_from_slice(&1u32.to_be_bytes()); // entry_count
        stsd_payload.extend_from_slice(&codec_box);
        box_wrap(b"stsd", &stsd_payload)
    }

    fn build_audio_trak(track_id: u32, duration: u32, timescale: u32) -> Vec<u8> {
        let tkhd_payload: Vec<u8> = {
            let mut b = vec![0u8, 0u8, 0u8, 3u8];
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&track_id.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&duration.to_be_bytes());
            b.extend_from_slice(&[0u8; 8]);
            b.extend_from_slice(&0i16.to_be_bytes());
            b.extend_from_slice(&0i16.to_be_bytes());
            b.extend_from_slice(&0x0100u16.to_be_bytes()); // volume=1.0 for audio
            b.extend_from_slice(&0u16.to_be_bytes());
            b.extend_from_slice(&0x00010000u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0x00010000u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0x40000000u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes()); // width=0
            b.extend_from_slice(&0u32.to_be_bytes()); // height=0
            b
        };
        let tkhd = box_wrap(b"tkhd", &tkhd_payload);
        let mdia = build_audio_mdia(duration, timescale);
        let trak_payload = {
            let mut b = Vec::new();
            b.extend_from_slice(&tkhd);
            b.extend_from_slice(&mdia);
            b
        };
        box_wrap(b"trak", &trak_payload)
    }

    fn build_audio_mdia(duration: u32, timescale: u32) -> Vec<u8> {
        let mdhd_payload: Vec<u8> = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&timescale.to_be_bytes());
            b.extend_from_slice(&duration.to_be_bytes());
            b.extend_from_slice(&0u16.to_be_bytes());
            b.extend_from_slice(&0u16.to_be_bytes());
            b
        };
        let mdhd = box_wrap(b"mdhd", &mdhd_payload);
        let hdlr = build_hdlr(b"soun", b"SoundHandler");
        let minf = build_audio_minf();
        let mdia_payload = {
            let mut b = Vec::new();
            b.extend_from_slice(&mdhd);
            b.extend_from_slice(&hdlr);
            b.extend_from_slice(&minf);
            b
        };
        box_wrap(b"mdia", &mdia_payload)
    }

    fn build_audio_minf() -> Vec<u8> {
        // smhd
        let smhd_payload = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&0u16.to_be_bytes()); // balance
            b.extend_from_slice(&0u16.to_be_bytes()); // reserved
            b
        };
        let smhd = box_wrap(b"smhd", &smhd_payload);

        // dinf -> dref -> url
        let url_payload = vec![0u8, 0u8, 0u8, 1u8];
        let url_box = box_wrap(b"url ", &url_payload);
        let dref_payload = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&url_box);
            b
        };
        let dref = box_wrap(b"dref", &dref_payload);
        let dinf = box_wrap(b"dinf", &dref);

        // stbl: minimal stsd with mp4a
        // mp4a layout: 4 reserved + 2 reserved + 2 data_ref_idx + 8 reserved +
        //              2 channelcount + 2 samplesize + 4 pre_defined/reserved + 4 samplerate
        // esds is optional — omit it to avoid needing a valid ESDescriptor.
        let mp4a_payload = {
            let mut b = vec![0u8; 6]; // reserved
            b.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
            b.extend_from_slice(&[0u8; 8]); // reserved
            b.extend_from_slice(&2u16.to_be_bytes()); // channelcount
            b.extend_from_slice(&16u16.to_be_bytes()); // samplesize
            b.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
            b.extend_from_slice(&0u16.to_be_bytes()); // reserved
            b.extend_from_slice(&(44100u32 << 16).to_be_bytes()); // samplerate 44100.0
                                                                  // No esds box — mp4a.esds is Option<EsdsBox>, None is valid.
            b
        };
        let mp4a = box_wrap(b"mp4a", &mp4a_payload);
        let mut stsd_payload = vec![0u8; 4];
        stsd_payload.extend_from_slice(&1u32.to_be_bytes());
        stsd_payload.extend_from_slice(&mp4a);
        let stsd = box_wrap(b"stsd", &stsd_payload);

        let stts_payload = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&1024u32.to_be_bytes());
            b
        };
        let stts = box_wrap(b"stts", &stts_payload);
        let stsc_payload = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&1u32.to_be_bytes());
            b
        };
        let stsc = box_wrap(b"stsc", &stsc_payload);
        let stsz_payload = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&0u32.to_be_bytes());
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
            b
        };
        let stsz = box_wrap(b"stsz", &stsz_payload);
        let stco_payload = {
            let mut b = vec![0u8; 4];
            b.extend_from_slice(&1u32.to_be_bytes());
            b.extend_from_slice(&28u32.to_be_bytes());
            b
        };
        let stco = box_wrap(b"stco", &stco_payload);

        let stbl_payload = {
            let mut b = Vec::new();
            b.extend_from_slice(&stsd);
            b.extend_from_slice(&stts);
            b.extend_from_slice(&stsc);
            b.extend_from_slice(&stsz);
            b.extend_from_slice(&stco);
            b
        };
        let stbl = box_wrap(b"stbl", &stbl_payload);

        let minf_payload = {
            let mut b = Vec::new();
            b.extend_from_slice(&smhd);
            b.extend_from_slice(&dinf);
            b.extend_from_slice(&stbl);
            b
        };
        box_wrap(b"minf", &minf_payload)
    }

    // --- check_moov_before_mdat edge case tests ---

    /// Helper: write bytes to a temp file and run check_moov_before_mdat.
    fn check_moov_bytes(bytes: &[u8]) -> Result<(), MediaError> {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bytes).unwrap();
        check_moov_before_mdat(tmp.path())
    }

    #[test]
    fn test_moov_scanner_iteration_limit() {
        // Craft a file with 2000 minimal 8-byte "free" atoms followed by moov + mdat.
        // The scanner hits MAX_ATOMS (1024) and fails closed — it can't verify
        // moov-before-mdat, so it rejects the file rather than silently passing.
        let mut bytes = Vec::new();
        for _ in 0..2000 {
            bytes.extend_from_slice(&8u32.to_be_bytes()); // size = 8
            bytes.extend_from_slice(b"free");
        }
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(b"moov");
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(b"mdat");
        // Fail closed: too many atoms → reject
        let err = check_moov_bytes(&bytes);
        assert!(
            matches!(err, Err(MediaError::MoovNotAtFront)),
            "expected MoovNotAtFront, got {err:?}"
        );
    }

    #[test]
    fn test_moov_scanner_extended_atom_size() {
        // Build: ftyp(20) + moov(extended size, 24 bytes total) + mdat(8)
        // Extended size: compact_size=1, then 8-byte real size.
        let mut bytes = Vec::new();
        // ftyp
        bytes.extend_from_slice(&20u32.to_be_bytes());
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(b"isom");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(b"isom");
        // moov with extended size (compact_size=1, extended_size=24)
        // 24 = 16 byte header + 8 bytes payload
        bytes.extend_from_slice(&1u32.to_be_bytes()); // compact size = 1 (extended)
        bytes.extend_from_slice(b"moov");
        bytes.extend_from_slice(&24u64.to_be_bytes()); // extended size = 24
        bytes.extend_from_slice(&[0u8; 8]); // 8 bytes of moov payload
                                            // mdat
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(b"mdat");
        // moov is before mdat — should pass
        assert!(check_moov_bytes(&bytes).is_ok());
    }

    #[test]
    fn test_moov_scanner_extended_mdat_before_moov() {
        // Extended-size mdat before moov — must be rejected.
        let mut bytes = Vec::new();
        // ftyp
        bytes.extend_from_slice(&20u32.to_be_bytes());
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(b"isom");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(b"isom");
        // mdat with extended size (before moov)
        bytes.extend_from_slice(&1u32.to_be_bytes()); // compact size = 1 (extended)
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&24u64.to_be_bytes()); // extended size = 24
        bytes.extend_from_slice(&[0u8; 8]); // payload
                                            // moov after mdat
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(b"moov");
        let err = check_moov_bytes(&bytes);
        assert!(
            matches!(err, Err(MediaError::MoovNotAtFront)),
            "expected MoovNotAtFront, got {err:?}"
        );
    }

    #[test]
    fn test_moov_scanner_eof_atom_mdat_before_moov() {
        // atom_size==0 (extends to EOF) on mdat, with no moov seen — must be rejected.
        let mut bytes = Vec::new();
        // ftyp
        bytes.extend_from_slice(&20u32.to_be_bytes());
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(b"isom");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(b"isom");
        // mdat with size=0 (extends to EOF), no moov before it
        bytes.extend_from_slice(&0u32.to_be_bytes()); // size = 0 (EOF)
        bytes.extend_from_slice(b"mdat");
        let err = check_moov_bytes(&bytes);
        assert!(
            matches!(err, Err(MediaError::MoovNotAtFront)),
            "expected MoovNotAtFront, got {err:?}"
        );
    }

    // --- actual test cases ---

    #[test]
    fn test_validate_video_ok() {
        let config = test_config();
        let mp4_bytes = build_minimal_mp4_moov_first();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        let result = validate_video_file(tmp.path(), &config);
        match result {
            Ok(meta) => {
                assert_eq!(meta.width, 320);
                assert_eq!(meta.height, 240);
                assert!(!meta.has_audio);
                assert!(meta.duration_secs > 0.0 && meta.duration_secs <= 600.0);
            }
            Err(e) => panic!("expected Ok, got {e:?}"),
        }
    }

    #[test]
    fn test_validate_video_with_audio() {
        let config = test_config();
        let mp4_bytes = build_mp4_with_audio();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        let result = validate_video_file(tmp.path(), &config);
        match result {
            Ok(meta) => assert!(meta.has_audio),
            Err(e) => panic!("expected Ok, got {e:?}"),
        }
    }

    #[test]
    fn test_validate_video_mdat_first_rejected() {
        let config = test_config();
        let mp4_bytes = build_minimal_mp4_mdat_first();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        let result = validate_video_file(tmp.path(), &config);
        assert!(
            matches!(result, Err(MediaError::MoovNotAtFront)),
            "expected MoovNotAtFront, got {result:?}"
        );
    }

    #[test]
    fn test_validate_video_hevc_rejected() {
        let config = test_config();
        let mp4_bytes = build_mp4_hevc();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        let result = validate_video_file(tmp.path(), &config);
        assert!(
            matches!(result, Err(MediaError::WrongCodec)),
            "expected WrongCodec, got {result:?}"
        );
    }

    #[test]
    fn test_validate_video_too_long_rejected() {
        let config = test_config();
        let mp4_bytes = build_mp4_too_long();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        let result = validate_video_file(tmp.path(), &config);
        assert!(
            matches!(result, Err(MediaError::DurationTooLong)),
            "expected DurationTooLong, got {result:?}"
        );
    }

    #[test]
    fn test_validate_video_zero_duration_rejected() {
        let config = test_config();
        // duration_ms=0 → duration_secs=0.0 → rejected as InvalidVideo
        let mp4_bytes = build_mp4_bytes(true, b"avc1", 0, 320, 240, false);
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        let result = validate_video_file(tmp.path(), &config);
        assert!(
            matches!(result, Err(MediaError::InvalidVideo)),
            "expected InvalidVideo for zero-duration, got {result:?}"
        );
    }

    #[test]
    fn test_validate_video_resolution_too_high() {
        let config = test_config();
        let mp4_bytes = build_mp4_too_large();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        let result = validate_video_file(tmp.path(), &config);
        assert!(
            matches!(result, Err(MediaError::ResolutionTooHigh)),
            "expected ResolutionTooHigh, got {result:?}"
        );
    }

    // --- Generic file path tests ---

    /// Minimal PDF header — infer detects `application/pdf` from `%PDF`.
    const TINY_PDF: &[u8] = b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\ntrailer\n%%EOF";

    /// Minimal ZIP header — infer detects `application/zip` from `PK\x03\x04`.
    const TINY_ZIP: &[u8] = &[
        0x50, 0x4B, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn test_validate_file_pdf_accepted() {
        let config = test_config();
        let (mime, ext) = validate_file_content(TINY_PDF, &config).unwrap();
        assert_eq!(mime, "application/pdf");
        assert_eq!(ext, "pdf");
    }

    #[test]
    fn test_validate_file_zip_accepted() {
        let config = test_config();
        let (mime, ext) = validate_file_content(TINY_ZIP, &config).unwrap();
        assert_eq!(mime, "application/zip");
        assert_eq!(ext, "zip");
    }

    #[test]
    fn test_validate_file_plaintext_accepted_as_octet_stream() {
        // Plain text has no magic bytes — infer returns None. The generic path
        // accepts it as opaque binary served as a download (the common Slack
        // case: .txt, .csv, .md, source code).
        let config = test_config();
        let (mime, ext) = validate_file_content(b"hello, this is a text file\n", &config).unwrap();
        assert_eq!(mime, "application/octet-stream");
        assert_eq!(ext, "bin");
    }

    #[test]
    fn test_validate_file_html_rejected() {
        // HTML is a stored-XSS carrier — blocked even though headers neutralise it.
        let config = test_config();
        let html = b"<!DOCTYPE html><html><body><script>alert(1)</script></body></html>";
        let result = validate_file_content(html, &config);
        assert!(
            matches!(result, Err(MediaError::DisallowedContentType(ref m)) if m == "text/html"),
            "expected DisallowedContentType(text/html), got {result:?}"
        );
    }

    #[test]
    fn test_validate_file_too_large_rejected() {
        let mut config = test_config();
        config.max_file_bytes = 10;
        let result = validate_file_content(TINY_PDF, &config);
        assert!(matches!(result, Err(MediaError::FileTooLarge { .. })));
    }

    #[test]
    fn test_serve_inline() {
        assert!(serve_inline("image/jpeg"));
        assert!(serve_inline("image/png"));
        assert!(serve_inline("video/mp4"));
        // Generic files force download.
        assert!(!serve_inline("application/pdf"));
        assert!(!serve_inline("application/zip"));
        assert!(!serve_inline("application/octet-stream"));
        assert!(!serve_inline("audio/mpeg"));
        assert!(!serve_inline("text/plain"));
    }
}
