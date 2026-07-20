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

const MP4_BRANDS: &[[u8; 4]] = &[
    *b"isom", *b"iso2", *b"iso3", *b"iso4", *b"iso5", *b"iso6", *b"iso7", *b"iso8", *b"iso9",
    *b"mp41", *b"mp42", *b"avc1", *b"dash", *b"M4V ",
];

fn iso_bmff_ftyp_payload(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return None;
    }
    let compact = u32::from_be_bytes(bytes[..4].try_into().ok()?) as u64;
    let (declared_size, header_size) = if compact == 1 {
        if bytes.len() < 24 {
            return None;
        }
        (u64::from_be_bytes(bytes[8..16].try_into().ok()?), 16usize)
    } else if compact == 0 {
        (bytes.len() as u64, 8usize)
    } else {
        (compact, 8usize)
    };
    if declared_size < (header_size + 8) as u64 {
        return None;
    }
    let available_end = usize::try_from(declared_size)
        .unwrap_or(usize::MAX)
        .min(bytes.len());
    (available_end >= header_size + 8).then_some(&bytes[header_size..available_end])
}

/// Return whether the leading bytes contain a structurally valid ISO-BMFF
/// `ftyp` box, independent of the request MIME type or `infer`'s brand list.
pub fn looks_like_iso_bmff(bytes: &[u8]) -> bool {
    iso_bmff_ftyp_payload(bytes).is_some()
}

pub(crate) fn looks_like_mp4_iso_bmff(bytes: &[u8]) -> bool {
    let Some(payload) = iso_bmff_ftyp_payload(bytes) else {
        return false;
    };
    let major = payload[..4].try_into().ok();
    major.is_some_and(|brand| MP4_BRANDS.contains(&brand))
        || payload[8..]
            .chunks_exact(4)
            .any(|brand| MP4_BRANDS.iter().any(|candidate| brand == candidate))
}

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
/// This is the catch-all path for non-media attachments (documents, archives,
/// text, data). It enforces three things:
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

    // ISO-BMFF permits arbitrary major brands, so `infer` cannot enumerate all
    // valid MP4 signatures. Never let an `ftyp` container fall through as an
    // opaque attachment merely because its brand is unfamiliar.
    if looks_like_iso_bmff(bytes) {
        let mime = infer::get(bytes)
            .map(|kind| kind.mime_type().to_string())
            .unwrap_or_else(|| "application/iso-bmff".to_string());
        return Err(MediaError::DisallowedContentType(mime));
    }

    // 2. Sniff. `None` means no magic signature (text/csv/json/source) — that's
    //    fine for the generic path; treat as opaque binary served as a download.
    match infer::get(bytes) {
        Some(kind) => {
            let mime = kind.mime_type().to_string();
            // Recognized media must never fall through exact-byte attachment
            // storage. Images and video use their canonical media validators;
            // audio is rejected until Buzz has an explicit sanitizer and
            // location-metadata validator for its container.
            if mime.starts_with("image/")
                || mime.starts_with("video/")
                || mime.starts_with("audio/")
            {
                return Err(MediaError::DisallowedContentType(mime));
            }
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

    // 4. Reject metadata-bearing or non-canonical container structures.
    validate_image_metadata_free(bytes, &mime)?;

    // 5. Image bomb — check pixel dimensions before full decode.
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
/// - Exactly one video track using `avc1` (H.264 only — rejects HEVC, VP9, AV1)
/// - At most one audio track, using `mp4a` (AAC)
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

    validate_mp4_metadata_free(path)?;

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
                    // Alternate video tracks can carry telemetry or other content
                    // that the client did not intend to publish. Canonical uploads
                    // contain exactly one video track.
                    return Err(MediaError::MetadataForbidden);
                }

                // Codec check: only H.264 (avc1).
                // media_type() reads the handler type and sample entry box type.
                let media_type = track.media_type().map_err(|_| MediaError::WrongCodec)?;
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
                if has_audio {
                    // Reject alternate audio tracks for the same reason as
                    // alternate video tracks: only the canonical primary stream
                    // produced by the client sanitizer is permitted.
                    return Err(MediaError::MetadataForbidden);
                }
                let media_type = track.media_type().map_err(|_| MediaError::WrongCodec)?;
                if media_type != mp4::MediaType::AAC {
                    return Err(MediaError::WrongCodec);
                }
                has_audio = true;
            }
            _ => return Err(MediaError::MetadataForbidden),
        }
    }

    let mut meta = video_meta.ok_or_else(|| {
        if has_audio {
            MediaError::DisallowedContentType("audio/mp4".to_string())
        } else {
            MediaError::InvalidVideo
        }
    })?;
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

/// Reject metadata-bearing image structures without decoding pixel data.
///
/// This is deliberately a structural allowlist rather than an EXIF-tag denylist:
/// location can also live in XMP, comments, PNG text, ICC descriptions, or
/// private chunks. Client encoders remove these before upload.
fn validate_image_metadata_free(bytes: &[u8], mime: &str) -> Result<(), MediaError> {
    match mime {
        "image/jpeg" => validate_jpeg_metadata_free(bytes),
        "image/png" => validate_png_metadata_free(bytes),
        "image/webp" => validate_webp_metadata_free(bytes),
        "image/gif" => validate_gif_metadata_free(bytes),
        _ => Ok(()),
    }
}

fn validate_jpeg_metadata_free(bytes: &[u8]) -> Result<(), MediaError> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(MediaError::InvalidImage);
    }
    let mut i = 2usize;
    let mut in_scan = false;
    while i < bytes.len() {
        if bytes[i] != 0xff {
            if in_scan {
                i += 1;
                continue;
            }
            return Err(MediaError::InvalidImage);
        }
        while i < bytes.len() && bytes[i] == 0xff {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(MediaError::InvalidImage);
        }
        let marker = bytes[i];
        i += 1;
        if in_scan && marker == 0x00 {
            continue;
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            continue;
        }
        if marker == 0xd9 {
            return (i == bytes.len())
                .then_some(())
                .ok_or(MediaError::MetadataForbidden);
        }
        if marker == 0xd8 {
            return Err(MediaError::InvalidImage);
        }
        if i + 2 > bytes.len() {
            return Err(MediaError::InvalidImage);
        }
        let len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if len < 2 {
            return Err(MediaError::InvalidImage);
        }
        let end = i
            .checked_add(len)
            .filter(|&end| end <= bytes.len())
            .ok_or(MediaError::InvalidImage)?;
        // Only canonical JFIF/Adobe colour headers are allowed. Their lengths and
        // identifiers are fixed; accepting arbitrary APP0/APP14 payloads would
        // leave a metadata side channel.
        if marker == 0xe0 {
            let payload = &bytes[i + 2..end];
            let canonical_jfif = payload.len() >= 14
                && &payload[..5] == b"JFIF\0"
                && payload.len() == 14 + 3 * payload[12] as usize * payload[13] as usize;
            if !canonical_jfif {
                return Err(MediaError::MetadataForbidden);
            }
        } else if marker == 0xee {
            let payload = &bytes[i + 2..end];
            if payload.len() != 12 || &payload[..5] != b"Adobe" {
                return Err(MediaError::MetadataForbidden);
            }
        } else if (0xe1..=0xed).contains(&marker) || marker == 0xef || marker == 0xfe {
            return Err(MediaError::MetadataForbidden);
        }
        i = end;
        in_scan = marker == 0xda;
    }
    Err(MediaError::InvalidImage)
}

fn validate_png_metadata_free(bytes: &[u8]) -> Result<(), MediaError> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIG) {
        return Err(MediaError::InvalidImage);
    }
    let mut i = SIG.len();
    let mut saw_iend = false;
    while i < bytes.len() {
        if i + 12 > bytes.len() {
            return Err(MediaError::InvalidImage);
        }
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = bytes[i + 4..i + 8].try_into().unwrap();
        let end = i
            .checked_add(12)
            .and_then(|v| v.checked_add(len))
            .filter(|&v| v <= bytes.len())
            .ok_or(MediaError::InvalidImage)?;
        if matches!(&kind, b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"iCCP") {
            return Err(MediaError::MetadataForbidden);
        }
        // Unknown ancillary chunks are private metadata channels. Keep only
        // rendering chunks that client encoders may legitimately emit; pHYs is
        // deliberately excluded because arbitrary values are an identity channel.
        let ancillary = kind[0] & 0x20 != 0;
        let known_rendering = matches!(
            &kind,
            b"cHRM"
                | b"gAMA"
                | b"sBIT"
                | b"sRGB"
                | b"bKGD"
                | b"hIST"
                | b"tRNS"
                | b"sPLT"
                | b"acTL"
                | b"fcTL"
                | b"fdAT"
        );
        if ancillary && !known_rendering {
            return Err(MediaError::MetadataForbidden);
        }
        i = end;
        if &kind == b"IEND" {
            saw_iend = true;
            break;
        }
    }
    if !saw_iend || i != bytes.len() {
        return Err(MediaError::MetadataForbidden);
    }
    Ok(())
}

fn validate_webp_metadata_free(bytes: &[u8]) -> Result<(), MediaError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(MediaError::InvalidImage);
    }
    let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if declared.checked_add(8) != Some(bytes.len()) {
        return Err(MediaError::MetadataForbidden);
    }
    let mut i = 12usize;
    while i < bytes.len() {
        if i + 8 > bytes.len() {
            return Err(MediaError::InvalidImage);
        }
        let kind: [u8; 4] = bytes[i..i + 4].try_into().unwrap();
        let len = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
        let payload_start = i + 8;
        let padded = len.checked_add(len & 1).ok_or(MediaError::InvalidImage)?;
        i = payload_start
            .checked_add(padded)
            .filter(|&v| v <= bytes.len())
            .ok_or(MediaError::InvalidImage)?;
        if !matches!(
            &kind,
            b"VP8 " | b"VP8L" | b"VP8X" | b"ALPH" | b"ANIM" | b"ANMF"
        ) {
            return Err(MediaError::MetadataForbidden);
        }
        if &kind == b"VP8X" {
            let flags = *bytes.get(payload_start).ok_or(MediaError::InvalidImage)?;
            // ICC, EXIF, and XMP presence flags are metadata even if a malformed
            // file omits their corresponding chunks.
            if flags & (0x20 | 0x08 | 0x04) != 0 {
                return Err(MediaError::MetadataForbidden);
            }
        }
    }
    Ok(())
}

fn validate_gif_metadata_free(bytes: &[u8]) -> Result<(), MediaError> {
    if !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) || bytes.len() < 13 {
        return Err(MediaError::InvalidImage);
    }

    fn skip_sub_blocks(bytes: &[u8], i: &mut usize) -> Result<(), MediaError> {
        loop {
            let len = *bytes.get(*i).ok_or(MediaError::InvalidImage)? as usize;
            *i += 1;
            if len == 0 {
                return Ok(());
            }
            *i = i
                .checked_add(len)
                .filter(|&end| end <= bytes.len())
                .ok_or(MediaError::InvalidImage)?;
        }
    }

    let packed = bytes[10];
    let mut i = 13usize;
    if packed & 0x80 != 0 {
        let table_len = 3usize << ((packed & 0x07) as usize + 1);
        i = i
            .checked_add(table_len)
            .filter(|&end| end <= bytes.len())
            .ok_or(MediaError::InvalidImage)?;
    }

    loop {
        match *bytes.get(i).ok_or(MediaError::InvalidImage)? {
            0x2c => {
                // Image descriptor, optional local colour table, LZW code size,
                // then length-prefixed image-data sub-blocks.
                if i + 10 > bytes.len() {
                    return Err(MediaError::InvalidImage);
                }
                let image_packed = bytes[i + 9];
                i += 10;
                if image_packed & 0x80 != 0 {
                    let table_len = 3usize << ((image_packed & 0x07) as usize + 1);
                    i = i
                        .checked_add(table_len)
                        .filter(|&end| end <= bytes.len())
                        .ok_or(MediaError::InvalidImage)?;
                }
                i = i
                    .checked_add(1)
                    .filter(|&v| v <= bytes.len())
                    .ok_or(MediaError::InvalidImage)?;
                skip_sub_blocks(bytes, &mut i)?;
            }
            0x21 => {
                let label = *bytes.get(i + 1).ok_or(MediaError::InvalidImage)?;
                i += 2;
                match label {
                    // Graphic Control Extension carries rendering/animation state,
                    // not descriptive metadata. Its shape is fixed by the spec.
                    0xf9 => {
                        if bytes.get(i) != Some(&4) || i + 6 > bytes.len() || bytes[i + 5] != 0 {
                            return Err(MediaError::InvalidImage);
                        }
                        i += 6;
                    }
                    // Preserve only the standard looping application extensions.
                    // Other application, comment, and plain-text extensions are
                    // unrestricted metadata channels.
                    0xff => {
                        if bytes.get(i) != Some(&11) || i + 12 > bytes.len() {
                            return Err(MediaError::InvalidImage);
                        }
                        let app = &bytes[i + 1..i + 12];
                        if app != b"NETSCAPE2.0" && app != b"ANIMEXTS1.0" {
                            return Err(MediaError::MetadataForbidden);
                        }
                        i += 12;
                        skip_sub_blocks(bytes, &mut i)?;
                    }
                    _ => return Err(MediaError::MetadataForbidden),
                }
            }
            0x3b => {
                return (i + 1 == bytes.len())
                    .then_some(())
                    .ok_or(MediaError::MetadataForbidden);
            }
            _ => return Err(MediaError::InvalidImage),
        }
    }
}

fn validate_mp4_metadata_free(path: &Path) -> Result<(), MediaError> {
    const MAX_BOXES: usize = 100_000;
    const MAX_BOX_DEPTH: usize = 32;
    const EMPTY_FFMPEG_UDTA: &[u8] = &[
        0, 0, 0, 0x35, b'm', b'e', b't', b'a', 0, 0, 0, 0, 0, 0, 0, 0x21, b'h', b'd', b'l', b'r',
        0, 0, 0, 0, 0, 0, 0, 0, b'm', b'd', b'i', b'r', b'a', b'p', b'p', b'l', 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 8, b'i', b'l', b's', b't',
    ];
    const FORBIDDEN: &[[u8; 4]] = &[
        *b"meta",
        *b"ilst",
        *b"keys",
        *b"data",
        *b"uuid",
        *b"xml ",
        *b"bxml",
        *b"loci",
        *b"\xa9xyz",
        *b"name",
        *b"chap",
    ];
    const CONTAINERS: &[[u8; 4]] = &[
        *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"edts", *b"dinf", *b"sinf", *b"schi",
    ];
    // Constrained H.264/AAC MP4 produced by our client encoders. Unknown boxes
    // are rejected rather than guessed safe because private boxes can carry GPS.
    const ALLOWED: &[[u8; 4]] = &[
        *b"ftyp", *b"moov", *b"mdat", *b"free", *b"skip", *b"wide", *b"trak", *b"mdia", *b"minf",
        *b"stbl", *b"edts", *b"dinf", *b"sinf", *b"schi", *b"udta", *b"mvhd", *b"tkhd", *b"mdhd",
        *b"hdlr", *b"vmhd", *b"smhd", *b"dref", *b"url ", *b"urn ", *b"stsd", *b"stts", *b"stss",
        *b"ctts", *b"stsc", *b"stsz", *b"stco", *b"co64", *b"sgpd", *b"sbgp", *b"elst",
    ];
    fn walk(
        file: &mut std::fs::File,
        start: u64,
        end: u64,
        count: &mut usize,
        depth: usize,
    ) -> Result<(), MediaError> {
        use std::io::{Read, Seek, SeekFrom};
        if depth > MAX_BOX_DEPTH {
            return Err(MediaError::InvalidVideo);
        }
        let mut off = start;
        while off < end {
            *count += 1;
            if *count > MAX_BOXES || end - off < 8 {
                return Err(MediaError::InvalidVideo);
            }
            file.seek(SeekFrom::Start(off))
                .map_err(|e| MediaError::Io(e.to_string()))?;
            let mut h = [0u8; 8];
            file.read_exact(&mut h)
                .map_err(|_| MediaError::InvalidVideo)?;
            let compact = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
            let kind: [u8; 4] = h[4..8].try_into().unwrap();
            let (size, header) = if compact == 1 {
                let mut ext = [0u8; 8];
                file.read_exact(&mut ext)
                    .map_err(|_| MediaError::InvalidVideo)?;
                (u64::from_be_bytes(ext), 16)
            } else if compact == 0 {
                (end - off, 8)
            } else {
                (compact, 8)
            };
            if size < header || off.checked_add(size).filter(|&v| v <= end).is_none() {
                return Err(MediaError::InvalidVideo);
            }
            if FORBIDDEN.contains(&kind) || !ALLOWED.contains(&kind) {
                return Err(MediaError::MetadataForbidden);
            }
            if kind == *b"udta" {
                if size != header + EMPTY_FFMPEG_UDTA.len() as u64 {
                    return Err(MediaError::MetadataForbidden);
                }
                let mut body = vec![0; EMPTY_FFMPEG_UDTA.len()];
                file.read_exact(&mut body)
                    .map_err(|_| MediaError::InvalidVideo)?;
                if body != EMPTY_FFMPEG_UDTA {
                    return Err(MediaError::MetadataForbidden);
                }
            } else if CONTAINERS.contains(&kind) {
                walk(file, off + header, off + size, count, depth + 1)?;
            }
            off += size;
        }
        Ok(())
    }
    let mut file = std::fs::File::open(path).map_err(|e| MediaError::Io(e.to_string()))?;
    let end = file
        .metadata()
        .map_err(|e| MediaError::Io(e.to_string()))?
        .len();
    let mut count = 0;
    walk(&mut file, 0, end, &mut count, 0)
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
            s3_region: "us-east-1".to_string(),
            max_image_bytes: 50 * 1024 * 1024,
            max_gif_bytes: 10 * 1024 * 1024,
            max_video_bytes: 524_288_000,
            max_file_bytes: 104_857_600,
            public_base_url: String::new(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
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
        0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, // IEND chunk
        0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
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

    fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        // The policy parser is structural; the image decoder validates CRC later.
        out.extend_from_slice(&0u32.to_be_bytes());
        out
    }

    /// Deterministic little-endian TIFF payload with a real EXIF GPS IFD.
    /// Coordinates are synthetic: 37°46'30" N, 122°25'10" W.
    fn gps_exif_tiff() -> Vec<u8> {
        let mut tiff = b"II\x2a\0\x08\0\0\0".to_vec();
        // IFD0: one GPSInfo pointer (tag 0x8825) to offset 26.
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x8825u16.to_le_bytes());
        tiff.extend_from_slice(&4u16.to_le_bytes()); // LONG
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&26u32.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes()); // next IFD

        // GPS IFD: latitude ref/value and longitude ref/value.
        tiff.extend_from_slice(&4u16.to_le_bytes());
        for (tag, field_type, count, value) in [
            (1u16, 2u16, 2u32, u32::from_le_bytes(*b"N\0\0\0")),
            (2, 5, 3, 80),
            (3, 2, 2, u32::from_le_bytes(*b"W\0\0\0")),
            (4, 5, 3, 104),
        ] {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&field_type.to_le_bytes());
            tiff.extend_from_slice(&count.to_le_bytes());
            tiff.extend_from_slice(&value.to_le_bytes());
        }
        tiff.extend_from_slice(&0u32.to_le_bytes()); // next GPS IFD
        for value in [37u32, 46, 30, 122, 25, 10] {
            tiff.extend_from_slice(&value.to_le_bytes());
            tiff.extend_from_slice(&1u32.to_le_bytes());
        }
        tiff
    }

    /// Independent minimal EXIF parser used to prove the fixture really has a
    /// GPSInfo pointer and latitude/longitude entries before policy validation.
    fn assert_exif_fixture_has_gps(tiff: &[u8]) {
        assert_eq!(&tiff[..4], b"II\x2a\0");
        let read_u16 = |offset: usize| u16::from_le_bytes([tiff[offset], tiff[offset + 1]]);
        let read_u32 = |offset: usize| {
            u32::from_le_bytes([
                tiff[offset],
                tiff[offset + 1],
                tiff[offset + 2],
                tiff[offset + 3],
            ])
        };
        let ifd0 = read_u32(4) as usize;
        let ifd0_entries = read_u16(ifd0) as usize;
        let gps_offset = (0..ifd0_entries)
            .map(|index| ifd0 + 2 + index * 12)
            .find(|&entry| read_u16(entry) == 0x8825)
            .map(|entry| read_u32(entry + 8) as usize)
            .expect("fixture must contain an EXIF GPSInfo pointer");
        let gps_entries = read_u16(gps_offset) as usize;
        let tags: Vec<u16> = (0..gps_entries)
            .map(|index| read_u16(gps_offset + 2 + index * 12))
            .collect();
        assert!(tags.contains(&1) && tags.contains(&2));
        assert!(tags.contains(&3) && tags.contains(&4));
    }

    #[test]
    fn test_real_exif_gps_is_rejected_in_image_containers() {
        let tiff = gps_exif_tiff();
        assert_exif_fixture_has_gps(&tiff);

        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        let jpeg_payload_len = 6 + tiff.len();
        jpeg.extend_from_slice(&((jpeg_payload_len + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&tiff);
        jpeg.extend_from_slice(&TINY_JPEG[2..]);
        assert!(matches!(
            validate_jpeg_metadata_free(&jpeg),
            Err(MediaError::MetadataForbidden)
        ));

        let mut png = TINY_PNG[..TINY_PNG.len() - 12].to_vec();
        png.extend_from_slice(&png_chunk(b"eXIf", &tiff));
        png.extend_from_slice(&TINY_PNG[TINY_PNG.len() - 12..]);
        assert!(matches!(
            validate_png_metadata_free(&png),
            Err(MediaError::MetadataForbidden)
        ));

        let mut webp_body = b"WEBP".to_vec();
        let mut webp_exif = b"Exif\0\0".to_vec();
        webp_exif.extend_from_slice(&tiff);
        webp_body.extend_from_slice(b"EXIF");
        webp_body.extend_from_slice(&(webp_exif.len() as u32).to_le_bytes());
        webp_body.extend_from_slice(&webp_exif);
        if !webp_exif.len().is_multiple_of(2) {
            webp_body.push(0);
        }
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&(webp_body.len() as u32).to_le_bytes());
        webp.extend_from_slice(&webp_body);
        assert!(matches!(
            validate_webp_metadata_free(&webp),
            Err(MediaError::MetadataForbidden)
        ));
    }

    #[test]
    fn test_real_xmp_location_is_rejected_in_image_containers() {
        let xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
          <rdf:Description xmlns:exif="http://ns.adobe.com/exif/1.0/"
            exif:GPSLatitude="37,46.500N" exif:GPSLongitude="122,25.167W"/>
        </x:xmpmeta>"#;
        assert!(xmp
            .windows(b"GPSLatitude".len())
            .any(|window| window == b"GPSLatitude"));
        assert!(xmp
            .windows(b"GPSLongitude".len())
            .any(|window| window == b"GPSLongitude"));

        let namespace = b"http://ns.adobe.com/xap/1.0/\0";
        let mut jpeg_payload = namespace.to_vec();
        jpeg_payload.extend_from_slice(xmp);
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend_from_slice(&((jpeg_payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(&jpeg_payload);
        jpeg.extend_from_slice(&TINY_JPEG[2..]);
        assert!(matches!(
            validate_jpeg_metadata_free(&jpeg),
            Err(MediaError::MetadataForbidden)
        ));

        let mut itxt = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
        itxt.extend_from_slice(xmp);
        let mut png = TINY_PNG[..TINY_PNG.len() - 12].to_vec();
        png.extend_from_slice(&png_chunk(b"iTXt", &itxt));
        png.extend_from_slice(&TINY_PNG[TINY_PNG.len() - 12..]);
        assert!(matches!(
            validate_png_metadata_free(&png),
            Err(MediaError::MetadataForbidden)
        ));

        let mut webp_body = b"WEBP".to_vec();
        webp_body.extend_from_slice(b"XMP ");
        webp_body.extend_from_slice(&(xmp.len() as u32).to_le_bytes());
        webp_body.extend_from_slice(xmp);
        if !xmp.len().is_multiple_of(2) {
            webp_body.push(0);
        }
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&(webp_body.len() as u32).to_le_bytes());
        webp.extend_from_slice(&webp_body);
        assert!(matches!(
            validate_webp_metadata_free(&webp),
            Err(MediaError::MetadataForbidden)
        ));
    }

    #[test]
    fn test_android_image_processor_outputs_match_relay_contract() {
        let config = test_config();
        for (name, bytes, expected_mime) in [
            (
                "sRGB PNG",
                include_bytes!("../tests/fixtures/android/sanitized/bitmap-srgb-sanitized.png")
                    .as_slice(),
                "image/png",
            ),
            (
                "sRGB JPEG",
                include_bytes!("../tests/fixtures/android/sanitized/bitmap-srgb-sanitized.jpg")
                    .as_slice(),
                "image/jpeg",
            ),
            (
                "Display-P3 PNG",
                include_bytes!(
                    "../tests/fixtures/android/sanitized/bitmap-display-p3-sanitized.png"
                )
                .as_slice(),
                "image/png",
            ),
            (
                "Display-P3 JPEG",
                include_bytes!(
                    "../tests/fixtures/android/sanitized/bitmap-display-p3-sanitized.jpg"
                )
                .as_slice(),
                "image/jpeg",
            ),
        ] {
            let actual = validate_content(bytes, &config).unwrap_or_else(|error| {
                panic!("rejected Android-sanitized {name} fixture: {error}")
            });
            assert_eq!(actual, expected_mime);
        }
    }

    #[test]
    fn test_android_bitmap_encoder_outputs_require_sanitization() {
        let config = test_config();
        let srgb_png = include_bytes!("../tests/fixtures/android/bitmap-srgb.png").as_slice();
        assert_eq!(
            validate_content(srgb_png, &config).expect("rejected Android sRGB PNG fixture"),
            "image/png"
        );

        for (name, bytes) in [
            (
                "sRGB JPEG",
                include_bytes!("../tests/fixtures/android/bitmap-srgb.jpg").as_slice(),
            ),
            (
                "Display-P3 PNG",
                include_bytes!("../tests/fixtures/android/bitmap-display-p3.png").as_slice(),
            ),
            (
                "Display-P3 JPEG",
                include_bytes!("../tests/fixtures/android/bitmap-display-p3.jpg").as_slice(),
            ),
        ] {
            assert!(
                matches!(
                    validate_content(bytes, &config),
                    Err(MediaError::MetadataForbidden)
                ),
                "accepted unsanitized Android Bitmap.compress {name} fixture"
            );
        }
    }

    #[test]
    fn test_ios_uikit_sanitizer_outputs_match_relay_contract() {
        let config = test_config();
        for (name, bytes, expected_mime) in [
            (
                "PNG",
                include_bytes!("../tests/fixtures/ios/uikit-sanitized.png").as_slice(),
                "image/png",
            ),
            (
                "JPEG",
                include_bytes!("../tests/fixtures/ios/uikit-sanitized.jpg").as_slice(),
                "image/jpeg",
            ),
        ] {
            let actual = validate_content(bytes, &config).unwrap_or_else(|error| {
                panic!("rejected iOS UIKit-sanitized {name} fixture: {error}")
            });
            assert_eq!(actual, expected_mime);
        }
    }

    #[test]
    fn test_ios_uikit_encoder_outputs_require_sanitization() {
        let config = test_config();
        for (name, bytes) in [
            (
                "PNG",
                include_bytes!("../tests/fixtures/ios/uikit-encoded.png").as_slice(),
            ),
            (
                "JPEG",
                include_bytes!("../tests/fixtures/ios/uikit-encoded.jpg").as_slice(),
            ),
        ] {
            assert!(
                matches!(
                    validate_content(bytes, &config),
                    Err(MediaError::MetadataForbidden)
                ),
                "accepted unsanitized iOS UIKit {name} fixture"
            );
        }
    }

    #[test]
    fn test_rejects_png_metadata_and_trailing_payload() {
        let config = test_config();
        for kind in [
            b"eXIf", b"tEXt", b"zTXt", b"iTXt", b"iCCP", b"pHYs", b"vpAg",
        ] {
            let mut png = TINY_PNG[..TINY_PNG.len() - 12].to_vec();
            png.extend_from_slice(&png_chunk(kind, b"GPS=37.7,-122.4"));
            png.extend_from_slice(&TINY_PNG[TINY_PNG.len() - 12..]);
            assert!(
                matches!(
                    validate_content(&png, &config),
                    Err(MediaError::MetadataForbidden)
                ),
                "accepted {kind:?}"
            );
        }
        let mut trailing = TINY_PNG.to_vec();
        trailing.extend_from_slice(b"hidden location");
        assert!(matches!(
            validate_content(&trailing, &config),
            Err(MediaError::MetadataForbidden)
        ));
    }

    #[test]
    fn test_rejects_jpeg_app_metadata_comments_and_trailing_payload() {
        let config = test_config();
        for marker in [0xe1, 0xec, 0xed, 0xef, 0xfe] {
            let mut jpeg = vec![0xff, 0xd8, 0xff, marker, 0x00, 0x08];
            jpeg.extend_from_slice(b"secret");
            jpeg.extend_from_slice(&TINY_JPEG[2..]);
            assert!(
                matches!(
                    validate_content(&jpeg, &config),
                    Err(MediaError::MetadataForbidden)
                ),
                "accepted marker {marker:#x}"
            );
        }
        for marker in [0xe0, 0xee] {
            let mut jpeg = vec![0xff, 0xd8, 0xff, marker, 0x00, 0x08];
            jpeg.extend_from_slice(b"secret");
            jpeg.extend_from_slice(&TINY_JPEG[2..]);
            assert!(matches!(
                validate_content(&jpeg, &config),
                Err(MediaError::MetadataForbidden)
            ));
        }
        let mut trailing = TINY_JPEG.to_vec();
        trailing.extend_from_slice(b"motion photo payload");
        assert!(matches!(
            validate_content(&trailing, &config),
            Err(MediaError::MetadataForbidden)
        ));
    }

    #[test]
    fn test_rejects_webp_metadata_unknown_chunks_and_trailing_payload() {
        fn webp(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
            let mut body = b"WEBP".to_vec();
            for (kind, payload) in chunks {
                body.extend_from_slice(*kind);
                body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                body.extend_from_slice(payload);
                if payload.len() % 2 != 0 {
                    body.push(0);
                }
            }
            let mut out = b"RIFF".to_vec();
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&body);
            out
        }

        for kind in [b"EXIF", b"XMP ", b"ICCP", b"priv"] {
            let bytes = webp(&[(kind, b"GPS=37.7,-122.4")]);
            assert!(matches!(
                validate_webp_metadata_free(&bytes),
                Err(MediaError::MetadataForbidden)
            ));
        }
        for flag in [0x20, 0x08, 0x04] {
            let mut payload = vec![0; 10];
            payload[0] = flag;
            let bytes = webp(&[(b"VP8X", &payload)]);
            assert!(matches!(
                validate_webp_metadata_free(&bytes),
                Err(MediaError::MetadataForbidden)
            ));
        }
        let mut trailing = webp(&[(b"VP8 ", b"pixels")]);
        trailing.extend_from_slice(b"hidden");
        assert!(matches!(
            validate_webp_metadata_free(&trailing),
            Err(MediaError::MetadataForbidden)
        ));
    }

    #[test]
    fn test_rejects_gif_metadata_extensions_and_trailing_payload() {
        for extension in [
            &[0x21, 0xfe, 1, b'x', 0][..],
            &[0x21, 0x01, 0][..],
            &[
                0x21, 0xff, 11, b'P', b'R', b'I', b'V', b'A', b'T', b'E', b'A', b'P', b'P', b'0', 0,
            ][..],
        ] {
            let mut gif = TINY_GIF[..TINY_GIF.len() - 1].to_vec();
            gif.extend_from_slice(extension);
            gif.push(0x3b);
            assert!(matches!(
                validate_gif_metadata_free(&gif),
                Err(MediaError::MetadataForbidden)
            ));
        }
        let mut trailing = TINY_GIF.to_vec();
        trailing.extend_from_slice(b"hidden");
        assert!(matches!(
            validate_gif_metadata_free(&trailing),
            Err(MediaError::MetadataForbidden)
        ));
    }

    #[test]
    fn test_generic_file_path_cannot_bypass_media_validation() {
        let config = test_config();
        assert!(
            matches!(validate_file_content(TINY_JPEG, &config), Err(MediaError::DisallowedContentType(m)) if m == "image/jpeg")
        );
        assert!(
            matches!(validate_file_content(MP4_FTYP_MAGIC, &config), Err(MediaError::DisallowedContentType(m)) if m == "video/mp4")
        );

        let proprietary_major = b"\x00\x00\x00\x18ftypPRIV\x00\x00\x00\x00isommp42";
        assert!(infer::get(proprietary_major).is_none());
        assert!(looks_like_iso_bmff(proprietary_major));
        assert!(looks_like_mp4_iso_bmff(proprietary_major));
        assert!(
            matches!(validate_file_content(proprietary_major, &config), Err(MediaError::DisallowedContentType(m)) if m == "application/iso-bmff")
        );
    }

    #[test]
    fn test_generic_file_path_rejects_recognized_audio() {
        let config = test_config();
        let fixtures: &[(&str, &[u8])] = &[
            ("mp3", b"ID3\x04\x00\x00\x00\x00\x00\x00"),
            ("flac", b"fLaC\x00\x00\x00\x22"),
            ("wav", b"RIFF\x24\x00\x00\x00WAVEfmt "),
            ("ogg", b"OggS\x00\x02\x00\x00\x00\x00\x00\x00"),
            ("m4a", b"\x00\x00\x00\x18ftypM4A \x00\x00\x00\x00M4A "),
            ("aac", b"\xff\xf1\x50\x80\x00\x1f\xfc"),
        ];

        for (name, bytes) in fixtures {
            let detected = infer::get(bytes)
                .unwrap_or_else(|| panic!("{name} fixture must be recognized as audio"));
            assert!(
                detected.mime_type().starts_with("audio/"),
                "{name} fixture detected as {}",
                detected.mime_type()
            );
            assert!(
                matches!(
                    validate_file_content(bytes, &config),
                    Err(MediaError::DisallowedContentType(mime)) if mime.starts_with("audio/")
                ),
                "generic path accepted {name}"
            );
        }
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

    /// Insert a child box at the end of the top-level `moov` box.
    fn append_box_to_moov(mut bytes: Vec<u8>, child: &[u8]) -> Vec<u8> {
        const FTYP_SIZE: usize = 20;
        let moov_size = u32::from_be_bytes(
            bytes[FTYP_SIZE..FTYP_SIZE + 4]
                .try_into()
                .expect("moov size bytes"),
        ) as usize;
        let moov_end = FTYP_SIZE + moov_size;
        bytes.splice(moov_end..moov_end, child.iter().copied());
        let new_moov_size = (moov_size + child.len()) as u32;
        bytes[FTYP_SIZE..FTYP_SIZE + 4].copy_from_slice(&new_moov_size.to_be_bytes());
        bytes
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
    fn test_validate_video_accepts_proprietary_major_with_isom_compatibility() {
        let mut mp4_bytes = build_minimal_mp4_moov_first();
        mp4_bytes[8..12].copy_from_slice(b"PRIV");
        assert!(infer::get(&mp4_bytes).is_none());
        assert!(looks_like_mp4_iso_bmff(&mp4_bytes));

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        assert!(validate_video_file(tmp.path(), &test_config()).is_ok());
    }

    #[test]
    fn test_accepts_exact_empty_ffmpeg_udta() {
        let empty_ffmpeg_udta = hex::decode(
            "000000356d657461000000000000002168646c7200000000000000006d6469726170706c00000000000000000000000008696c7374",
        )
        .unwrap();
        let bytes = [
            box_wrap(b"ftyp", b"isom\0\0\0\0isom"),
            box_wrap(b"moov", &box_wrap(b"udta", &empty_ffmpeg_udta)),
            box_wrap(b"mdat", b""),
        ]
        .concat();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bytes).unwrap();
        assert!(validate_mp4_metadata_free(tmp.path()).is_ok());
    }

    #[test]
    fn test_rejects_excessive_mp4_box_nesting() {
        let mut nested = box_wrap(b"free", b"");
        // One level beyond the validator's intentionally generous bound.
        for _ in 0..=32 {
            nested = box_wrap(b"moov", &nested);
        }
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), nested).unwrap();
        assert!(matches!(
            validate_mp4_metadata_free(tmp.path()),
            Err(MediaError::InvalidVideo)
        ));
    }

    #[test]
    fn test_rejects_mp4_metadata_boxes_and_trailing_payload() {
        let config = test_config();
        for kind in [
            b"udta", b"meta", b"ilst", b"keys", b"data", b"uuid", b"xml ", b"\xa9xyz",
        ] {
            let mut bytes = build_minimal_mp4_moov_first();
            bytes.extend_from_slice(&box_wrap(kind, b"GPS=37.7,-122.4"));
            let tmp = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(tmp.path(), &bytes).unwrap();
            assert!(
                matches!(
                    validate_video_file(tmp.path(), &config),
                    Err(MediaError::MetadataForbidden)
                ),
                "accepted {kind:?}"
            );
        }
        let mut bytes = build_minimal_mp4_moov_first();
        bytes.extend_from_slice(b"trailing");
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &bytes).unwrap();
        assert!(validate_video_file(tmp.path(), &config).is_err());
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
    fn test_validate_video_rejects_alternate_video_track() {
        let config = test_config();
        let extra_video = build_video_trak(2, 1_000, 1_000, b"avc1", 320, 240);
        let mp4_bytes = append_box_to_moov(build_minimal_mp4_moov_first(), &extra_video);
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        assert!(matches!(
            validate_video_file(tmp.path(), &config),
            Err(MediaError::MetadataForbidden)
        ));
    }

    #[test]
    fn test_validate_video_rejects_alternate_audio_track() {
        let config = test_config();
        let extra_audio = build_audio_trak(3, 1_000, 1_000);
        let mp4_bytes = append_box_to_moov(build_mp4_with_audio(), &extra_audio);
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        assert!(matches!(
            validate_video_file(tmp.path(), &config),
            Err(MediaError::MetadataForbidden)
        ));
    }

    #[test]
    fn test_validate_video_rejects_real_quicktime_iso6709_location() {
        // QuickTime stores ISO-6709 coordinates in a ©xyz child of moov/udta.
        let coordinates = b"+37.7750-122.4183+015.000/";
        let location = box_wrap(b"\xa9xyz", coordinates);
        let udta = box_wrap(b"udta", &location);
        let mp4_bytes = append_box_to_moov(build_minimal_mp4_moov_first(), &udta);

        // Independently prove this source contains the expected location box
        // and coordinates before exercising the validator.
        assert!(mp4_bytes.windows(4).any(|window| window == b"\xa9xyz"));
        assert!(mp4_bytes
            .windows(coordinates.len())
            .any(|window| window == coordinates));

        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        assert!(matches!(
            validate_video_file(tmp.path(), &test_config()),
            Err(MediaError::MetadataForbidden)
        ));
    }

    #[test]
    fn test_validate_video_rejects_timed_metadata_track() {
        // Start from a parseable audio track and change its handler from
        // `soun` to `meta`, representing a timed telemetry/GPS track.
        let mut telemetry_track = build_audio_trak(2, 1_000, 1_000);
        let handler = telemetry_track
            .windows(4)
            .position(|window| window == b"soun")
            .expect("audio fixture must contain a soun handler");
        telemetry_track[handler..handler + 4].copy_from_slice(b"meta");
        assert!(telemetry_track.windows(4).any(|window| window == b"meta"));

        let mp4_bytes = append_box_to_moov(build_minimal_mp4_moov_first(), &telemetry_track);
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), &mp4_bytes).unwrap();
        assert!(validate_video_file(tmp.path(), &test_config()).is_err());
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
