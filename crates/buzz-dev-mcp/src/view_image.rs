//! `view_image` MCP tool — load an image from a path, http(s) URL, or
//! `data:` URL and return it as an MCP `image` content block that any
//! multimodal-capable host (Anthropic, OpenAI-compatible, etc.) can forward
//! to its model.
//!
//! Design goals: tiny surface, no protocol-specific branching, and a
//! "reasonable resolution" that fits comfortably inside both Anthropic's
//! recommended ≤1568px / ≤5 MiB image budget and OpenAI's high-detail tile
//! size sweet spot. The MCP host translates `Content::image(data, mime)`
//! into the right provider-native shape on our behalf (see Goose's
//! `providers::utils::convert_image` for a reference implementation).

use crate::paths::resolve_path;
use crate::shell::SharedState;
use base64::Engine;
use image::{
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
    DynamicImage, ExtendedColorType, ImageEncoder, ImageReader, Limits,
};
use rmcp::{
    model::{CallToolResult, Content},
    ErrorData,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::Duration;

/// Hard cap on bytes we will read from disk / URL / data: URL.
pub(crate) const MAX_SOURCE_BYTES: usize = 20 * 1024 * 1024;
/// Hard cap on the raw (pre-base64) bytes we emit. base64 expands by 4/3, so
/// a 3 MiB raw payload becomes ~4 MiB on the wire — comfortably below
/// Anthropic's 5 MiB-per-image limit.
pub(crate) const MAX_FINAL_RAW_BYTES: usize = 3 * 1024 * 1024;
/// Default longest-edge cap. Matches Anthropic's published recommendation
/// (≤1568px) and lands well inside OpenAI's high-detail tile budget.
pub(crate) const DEFAULT_MAX_DIM: u32 = 1568;
pub(crate) const MIN_MAX_DIM: u32 = 64;
pub(crate) const MAX_MAX_DIM: u32 = 2048;
/// Hard cap on decoded pixel count. A ≤20 MiB compressed source can decode
/// to hundreds of megabytes; we reject anything above this budget *before*
/// touching the decoder. 64 megapixels is generous (e.g. 8000×8000) yet
/// keeps worst-case allocation well under a gigabyte.
pub(crate) const MAX_PIXELS: u64 = 64 * 1024 * 1024;
/// Defence-in-depth for the `image` decoder: bound any single allocation it
/// performs to 256 MiB (the default is 512 MiB and skews high for a dev MCP).
pub(crate) const MAX_DECODER_ALLOC: u64 = 256 * 1024 * 1024;
/// Connect + read timeout for URL fetches.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Build the decoder allocation cap. Centralised so the resize path uses the
/// same value tests can reason about.
fn decode_limits() -> Limits {
    let mut l = Limits::default();
    l.max_alloc = Some(MAX_DECODER_ALLOC);
    l
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ViewImageParams {
    /// Image source: an absolute or workspace-relative file path,
    /// an `http://` / `https://` URL, or a `data:image/<type>;base64,...` URL.
    pub source: String,
    /// Optional longest-edge cap in pixels. Clamped to [64, 2048].
    /// Defaults to 1568, which fits Anthropic's recommended budget and
    /// OpenAI's high-detail tile size.
    #[serde(default)]
    pub max_dim: Option<u32>,
    /// Workspace root for relative path resolution. Ignored for URL sources.
    /// Defaults to the server's cwd.
    #[serde(default)]
    pub workdir: Option<String>,
}

/// What `view_image` returns: the (mime, raw bytes) we will base64-encode
/// into an MCP image content block, plus a short human-readable summary.
#[derive(Debug)]
struct PreparedImage {
    mime: &'static str,
    bytes: Vec<u8>,
    summary: String,
}

pub async fn run(state: &SharedState, p: ViewImageParams) -> Result<CallToolResult, ErrorData> {
    let max_dim = p
        .max_dim
        .unwrap_or(DEFAULT_MAX_DIM)
        .clamp(MIN_MAX_DIM, MAX_MAX_DIM);

    let (raw, source_label) = load_source(state, &p).await?;
    let prepared = prepare(&raw, max_dim).map_err(invalid_params)?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(&prepared.bytes);
    let header = format!(
        "{} ({} from {source_label})",
        prepared.summary, prepared.mime
    );

    Ok(CallToolResult::success(vec![
        Content::text(header),
        Content::image(encoded, prepared.mime.to_string()),
    ]))
}

fn invalid_params(msg: String) -> ErrorData {
    ErrorData::invalid_params(msg, None)
}

/// Fetch the source bytes from path / http(s) / data URL.
async fn load_source(
    state: &SharedState,
    p: &ViewImageParams,
) -> Result<(Vec<u8>, String), ErrorData> {
    let src = p.source.trim();
    if src.starts_with("data:") {
        let bytes = decode_data_url(src).map_err(invalid_params)?;
        // `decode_data_url` enforces an encoded-length precheck so we never
        // allocate past the source cap. Re-verify the decoded length for
        // belt-and-braces.
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(invalid_params(format!(
                "data: URL decoded to {} bytes (limit {} bytes)",
                bytes.len(),
                MAX_SOURCE_BYTES
            )));
        }
        Ok((bytes, "data:URL".to_string()))
    } else if src.starts_with("http://") || src.starts_with("https://") {
        let bytes = fetch_url(src).await?;
        Ok((bytes, src.to_string()))
    } else if src.contains("://") {
        // Treat any other `scheme://...` form as an explicit reject so
        // `ftp://...` doesn't accidentally become a filesystem path.
        Err(invalid_params(format!(
            "unsupported URL scheme in `source`: {src}",
        )))
    } else {
        let workspace_root = match p.workdir.as_deref() {
            Some(w) => PathBuf::from(w),
            None => state.cwd.clone(),
        };
        let target = resolve_path(&workspace_root, src).map_err(invalid_params)?;
        let meta = std::fs::metadata(&target).map_err(|e| {
            ErrorData::internal_error(format!("cannot stat {}: {e}", target.display()), None)
        })?;
        if !meta.is_file() {
            return Err(invalid_params(format!(
                "not a regular file: {}",
                target.display()
            )));
        }
        if meta.len() as usize > MAX_SOURCE_BYTES {
            return Err(invalid_params(format!(
                "file too large: {} is {} bytes (limit {} bytes)",
                target.display(),
                meta.len(),
                MAX_SOURCE_BYTES
            )));
        }
        // Use `take(cap + 1)` so a file that grows between the metadata
        // check and the read still cannot exceed our budget. The +1
        // distinguishes "exactly at cap" from "grew past cap".
        let file = std::fs::File::open(&target).map_err(|e| {
            ErrorData::internal_error(format!("cannot open {}: {e}", target.display()), None)
        })?;
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        use std::io::Read;
        file.take(MAX_SOURCE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| {
                ErrorData::internal_error(format!("cannot read {}: {e}", target.display()), None)
            })?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(invalid_params(format!(
                "file {} grew past {} byte cap during read",
                target.display(),
                MAX_SOURCE_BYTES
            )));
        }
        Ok((bytes, target.display().to_string()))
    }
}

/// Parse `data:image/<subtype>[;base64],<payload>`. Only base64 payloads are
/// accepted — percent-encoded data URLs add surface area for no real benefit.
fn decode_data_url(src: &str) -> Result<Vec<u8>, String> {
    let rest = src
        .strip_prefix("data:")
        .ok_or_else(|| "not a data: URL".to_string())?;
    let (meta, payload) = rest
        .split_once(',')
        .ok_or_else(|| "malformed data: URL (no comma)".to_string())?;
    // meta is "<mime>[;param=value]*[;base64]"
    let mut parts = meta.split(';');
    let mime = parts.next().unwrap_or("");
    if !mime.starts_with("image/") {
        return Err(format!("data: URL is not an image (got `{mime}`)"));
    }
    let is_base64 = parts.any(|p| p.eq_ignore_ascii_case("base64"));
    if !is_base64 {
        return Err(
            "data: URL must be base64-encoded (non-base64 / percent-encoded forms are not supported)"
                .to_string(),
        );
    }
    let payload = payload.trim();
    // Pre-check encoded length so we never allocate past the source cap.
    // 4 base64 chars encode 3 raw bytes; ceil-divide MAX_SOURCE_BYTES.
    let max_encoded = MAX_SOURCE_BYTES.div_ceil(3) * 4 + 4; // +4 absorbs padding rounding
    if payload.len() > max_encoded {
        return Err(format!(
            "data: URL payload is {} base64 chars (limit ~{} = {} raw bytes)",
            payload.len(),
            max_encoded,
            MAX_SOURCE_BYTES
        ));
    }
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| format!("data: URL base64 decode failed: {e}"))
}

/// Fetch an http(s) URL with a streaming read and a hard byte cap.
/// Refuses up-front if `Content-Length` advertises more than the cap.
async fn fetch_url(url: &str) -> Result<Vec<u8>, ErrorData> {
    let client = reqwest::Client::builder()
        .connect_timeout(FETCH_TIMEOUT)
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| ErrorData::internal_error(format!("http client init failed: {e}"), None))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| ErrorData::internal_error(format!("fetch failed: {url} ({e})"), None))?;
    if !resp.status().is_success() {
        return Err(invalid_params(format!(
            "fetch {url} returned HTTP {}",
            resp.status()
        )));
    }
    if let Some(len) = resp.content_length() {
        if len as usize > MAX_SOURCE_BYTES {
            return Err(invalid_params(format!(
                "remote image too large: Content-Length {} bytes (limit {})",
                len, MAX_SOURCE_BYTES
            )));
        }
    }
    let mut buf: Vec<u8> = Vec::new();
    let mut stream = resp;
    loop {
        let chunk = stream
            .chunk()
            .await
            .map_err(|e| ErrorData::internal_error(format!("fetch read failed: {e}"), None))?;
        match chunk {
            Some(bytes) => {
                if buf.len() + bytes.len() > MAX_SOURCE_BYTES {
                    return Err(invalid_params(format!(
                        "remote image exceeded {} byte cap mid-stream",
                        MAX_SOURCE_BYTES
                    )));
                }
                buf.extend_from_slice(&bytes);
            }
            None => break,
        }
    }
    Ok(buf)
}

/// Sniff the image format from magic bytes alone (do not trust extensions
/// or `Content-Type`). Returns the canonical MIME type.
fn sniff_mime(bytes: &[u8]) -> Result<&'static str, String> {
    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok("image/png");
    }
    // JPEG: FF D8 FF
    if bytes.len() >= 3 && bytes[0..3] == [0xFF, 0xD8, 0xFF] {
        return Ok("image/jpeg");
    }
    // GIF87a / GIF89a
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok("image/gif");
    }
    // WebP: "RIFF" .... "WEBP"
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok("image/webp");
    }
    Err("unsupported image format (recognised: png, jpeg, gif, webp)".to_string())
}

/// Detect animated GIF (≥2 image descriptors) or animated WebP (VP8X chunk
/// with ANIM bit set). We refuse animated input outright rather than
/// silently emit a first-frame still.
///
/// **Important**: both branches must be allocation-free byte-level scans.
/// Using the `image` crate's `GifDecoder::into_frames()` here would let an
/// attacker-controlled logical-screen size trigger a multi-GB RGBA buffer
/// before our pixel-count cap fires.
fn is_animated(bytes: &[u8], mime: &str) -> bool {
    match mime {
        "image/gif" => gif_has_two_image_descriptors(bytes),
        "image/webp" => {
            // Animated WebP files always use the extended (VP8X) container.
            // The animation bit is bit 1 of the flags byte at offset 20.
            if bytes.len() < 21 {
                return false;
            }
            if &bytes[12..16] != b"VP8X" {
                return false;
            }
            (bytes[20] & 0x02) != 0
        }
        _ => false,
    }
}

/// Scan a GIF byte stream and report whether it contains ≥2 image descriptors
/// (frames). Does not allocate decode buffers — walks the block structure
/// described in the GIF89a spec and bails on the second `0x2C` separator.
fn gif_has_two_image_descriptors(bytes: &[u8]) -> bool {
    // 6-byte header ("GIF87a"/"GIF89a") + 7-byte logical screen descriptor.
    if bytes.len() < 13 {
        return false;
    }
    let packed = bytes[10];
    let has_gct = (packed & 0x80) != 0;
    let gct_size = if has_gct {
        3 * (1u32 << ((packed & 0x07) + 1))
    } else {
        0
    };
    let mut i = 13usize + gct_size as usize;
    let mut frames = 0u32;
    while let Some(&b) = bytes.get(i) {
        i += 1;
        match b {
            0x3B => return frames >= 2, // trailer
            0x21 => {
                // Extension introducer: <label><sub-block>*<0x00>
                if i >= bytes.len() {
                    return frames >= 2;
                }
                i += 1; // skip label
                i = match skip_subblocks(bytes, i) {
                    Some(n) => n,
                    None => return frames >= 2,
                };
            }
            0x2C => {
                // Image descriptor: 9 bytes (left/top/w/h/packed) then
                // optional local color table, then LZW min-code-size byte,
                // then sub-blocks.
                frames += 1;
                if frames >= 2 {
                    return true;
                }
                if i + 9 > bytes.len() {
                    return false;
                }
                let img_packed = bytes[i + 8];
                let has_lct = (img_packed & 0x80) != 0;
                let lct_size = if has_lct {
                    3u32 * (1u32 << ((img_packed & 0x07) + 1))
                } else {
                    0
                };
                i += 9 + lct_size as usize;
                if i >= bytes.len() {
                    return false;
                }
                i += 1; // LZW min-code-size
                i = match skip_subblocks(bytes, i) {
                    Some(n) => n,
                    None => return false,
                };
            }
            _ => {
                // Unknown / corrupt — bail rather than loop.
                return false;
            }
        }
    }
    false
}

/// Skip a GIF sub-block chain (length-prefixed runs terminated by a 0x00
/// length byte). Returns the index *after* the terminator, or `None` if the
/// stream is truncated.
fn skip_subblocks(bytes: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *bytes.get(i)? as usize;
        i += 1;
        if len == 0 {
            return Some(i);
        }
        i = i.checked_add(len)?;
        if i > bytes.len() {
            return None;
        }
    }
}

/// Either pass the bytes through verbatim (when they already fit) or
/// decode, resize, and re-encode them.
fn prepare(bytes: &[u8], max_dim: u32) -> Result<PreparedImage, String> {
    if bytes.is_empty() {
        return Err("empty image payload".to_string());
    }
    let mime = sniff_mime(bytes)?;
    if is_animated(bytes, mime) {
        return Err("animated images not supported; provide a still frame".to_string());
    }

    // Cheap header-only dimension read via ImageReader::with_guessed_format
    // — no pixel decoding yet.
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("image header read failed: {e}"))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| format!("image dimensions unreadable: {e}"))?;
    let longest = w.max(h);

    // Decompression-bomb guard: reject pathological dimensions *before* the
    // decoder allocates pixel buffers. A 20 MiB compressed source can
    // legitimately encode hundreds of megapixels; we cap at MAX_PIXELS.
    let pixels = u64::from(w) * u64::from(h);
    if pixels > MAX_PIXELS {
        return Err(format!(
            "image is {}×{} = {} pixels (limit {} pixels)",
            w, h, pixels, MAX_PIXELS
        ));
    }

    // Pass-through path: already small enough in both dims and bytes.
    if longest <= max_dim && bytes.len() <= MAX_FINAL_RAW_BYTES {
        return Ok(PreparedImage {
            mime,
            bytes: bytes.to_vec(),
            summary: format!("{w}×{h}, {}", human_bytes(bytes.len())),
        });
    }

    // Resize path: decode, resize, re-encode (PNG if alpha, JPEG otherwise).
    // The `Limits` cap is defence-in-depth for the case the dimension check
    // missed (e.g. some progressive JPEG re-allocations).
    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("image header read failed: {e}"))?;
    decoder.limits(decode_limits());
    let img = decoder
        .decode()
        .map_err(|e| format!("image decode failed: {e}"))?;

    let target = if longest > max_dim { max_dim } else { longest };
    let mut encoded = encode_resized(&img, target, w, h)?;

    // If still over budget after a normal resize (rare with JPEG-q85, more
    // common with PNG-alpha content), try once more at 75% scale before
    // giving up. Lanczos3 is good enough that one extra pass is plenty.
    if encoded.bytes.len() > MAX_FINAL_RAW_BYTES {
        let smaller = ((target as f32 * 0.75) as u32).max(MIN_MAX_DIM);
        encoded = encode_resized(&img, smaller, w, h)?;
        if encoded.bytes.len() > MAX_FINAL_RAW_BYTES {
            return Err(format!(
                "image still {} after resize; max allowed is {}",
                human_bytes(encoded.bytes.len()),
                human_bytes(MAX_FINAL_RAW_BYTES)
            ));
        }
    }
    encoded.summary = format!(
        "{}×{}, {} (resized from {}×{})",
        encoded.out_w,
        encoded.out_h,
        human_bytes(encoded.bytes.len()),
        w,
        h
    );
    Ok(PreparedImage {
        mime: encoded.mime,
        bytes: encoded.bytes,
        summary: encoded.summary,
    })
}

struct Encoded {
    mime: &'static str,
    bytes: Vec<u8>,
    out_w: u32,
    out_h: u32,
    summary: String,
}

fn encode_resized(
    img: &DynamicImage,
    target_longest: u32,
    orig_w: u32,
    orig_h: u32,
) -> Result<Encoded, String> {
    // Scale by longest edge, preserve aspect, round to nearest pixel.
    let (out_w, out_h) = if orig_w >= orig_h {
        let h = ((target_longest as u64 * orig_h as u64) / orig_w.max(1) as u64) as u32;
        (target_longest, h.max(1))
    } else {
        let w = ((target_longest as u64 * orig_w as u64) / orig_h.max(1) as u64) as u32;
        (w.max(1), target_longest)
    };
    let resized = img.resize_exact(out_w, out_h, image::imageops::FilterType::Lanczos3);

    // Decide output format from the *decoded* color type, not the original
    // MIME. A PNG that turns out fully opaque can be sent as JPEG; a JPEG
    // promoted to RGBA in transit still has no real alpha. We use
    // `color().has_alpha()` for the simple, correct decision.
    let has_alpha = resized.color().has_alpha();
    let mut out = Vec::new();
    let mime: &'static str = if has_alpha {
        let rgba = resized.to_rgba8();
        PngEncoder::new(&mut out)
            .write_image(&rgba, out_w, out_h, ExtendedColorType::Rgba8)
            .map_err(|e| format!("png encode failed: {e}"))?;
        "image/png"
    } else {
        let rgb = resized.to_rgb8();
        let mut enc = JpegEncoder::new_with_quality(&mut out, 85);
        enc.encode(&rgb, out_w, out_h, ExtendedColorType::Rgb8)
            .map_err(|e| format!("jpeg encode failed: {e}"))?;
        "image/jpeg"
    };

    Ok(Encoded {
        mime,
        bytes: out,
        out_w,
        out_h,
        summary: String::new(), // filled by caller
    })
}

fn human_bytes(n: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = KIB * 1024;
    if n >= MIB {
        format!("{:.2} MiB", n as f64 / MIB as f64)
    } else if n >= KIB {
        format!("{:.1} KiB", n as f64 / KIB as f64)
    } else {
        format!("{n} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, Rgba};
    use std::fs;
    use tempfile::tempdir;

    fn make_state(cwd: &std::path::Path) -> SharedState {
        let shim = crate::shim::Shim::install().expect("shim install");
        SharedState::new(cwd.to_path_buf(), shim).expect("state new")
    }

    fn write_png_rgba(path: &std::path::Path, w: u32, h: u32) -> Vec<u8> {
        let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = Rgba([(x % 256) as u8, (y % 256) as u8, 128, 200]);
        }
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&img, w, h, ExtendedColorType::Rgba8)
            .unwrap();
        fs::write(path, &bytes).unwrap();
        bytes
    }

    fn write_jpeg_rgb(path: &std::path::Path, w: u32, h: u32) -> Vec<u8> {
        let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = Rgb([(x % 256) as u8, (y % 256) as u8, 64]);
        }
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 85)
            .encode(&img, w, h, ExtendedColorType::Rgb8)
            .unwrap();
        fs::write(path, &bytes).unwrap();
        bytes
    }

    #[tokio::test]
    async fn small_png_passes_through_verbatim() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("tiny.png");
        let original = write_png_rgba(&p, 64, 48);
        let state = make_state(dir.path());
        let res = run(
            &state,
            ViewImageParams {
                source: "tiny.png".into(),
                max_dim: None,
                workdir: Some(dir.path().display().to_string()),
            },
        )
        .await
        .unwrap();
        // Last content block is the image; decode and compare bytes.
        let image_block = res
            .content
            .last()
            .and_then(|c| c.as_image())
            .expect("image content");
        assert_eq!(image_block.mime_type, "image/png");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&image_block.data)
            .unwrap();
        assert_eq!(decoded, original, "small PNG must pass through verbatim");
    }

    #[tokio::test]
    async fn oversize_png_with_alpha_resizes_to_png() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("big.png");
        write_png_rgba(&p, 4096, 2048);
        let state = make_state(dir.path());
        let res = run(
            &state,
            ViewImageParams {
                source: "big.png".into(),
                max_dim: Some(512),
                workdir: Some(dir.path().display().to_string()),
            },
        )
        .await
        .unwrap();
        let image_block = res
            .content
            .last()
            .and_then(|c| c.as_image())
            .expect("image");
        // Alpha preserved → PNG output.
        assert_eq!(image_block.mime_type, "image/png");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&image_block.data)
            .unwrap();
        // Resized version must be smaller in dims than the original.
        let dims = ImageReader::new(Cursor::new(&decoded))
            .with_guessed_format()
            .unwrap()
            .into_dimensions()
            .unwrap();
        assert!(dims.0.max(dims.1) <= 512, "got {:?}", dims);
    }

    #[tokio::test]
    async fn oversize_jpeg_resizes_to_jpeg() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("big.jpg");
        write_jpeg_rgb(&p, 3000, 2000);
        let state = make_state(dir.path());
        let res = run(
            &state,
            ViewImageParams {
                source: "big.jpg".into(),
                max_dim: Some(800),
                workdir: Some(dir.path().display().to_string()),
            },
        )
        .await
        .unwrap();
        let image_block = res
            .content
            .last()
            .and_then(|c| c.as_image())
            .expect("image");
        assert_eq!(image_block.mime_type, "image/jpeg");
    }

    #[tokio::test]
    async fn allows_path_outside_workspace() {
        let dir = tempdir().unwrap();
        // A real non-image file in a SECOND tempdir, genuinely outside the
        // workspace root — we expect a format error, not a path-escape error,
        // proving the traversal limit is gone. Avoids the Unix-only /etc/hosts
        // assumption (C:\etc\hosts does not exist on Windows).
        let outside = tempdir().unwrap();
        let target = outside.path().join("outside.txt");
        fs::write(&target, b"not an image").unwrap();
        let state = make_state(dir.path());
        let res = run(
            &state,
            ViewImageParams {
                source: target.display().to_string(),
                max_dim: None,
                workdir: Some(dir.path().display().to_string()),
            },
        )
        .await
        .unwrap_err();
        let msg = format!("{res:?}");
        assert!(
            msg.contains("unsupported image format") || msg.contains("empty image"),
            "{msg}"
        );
    }

    #[tokio::test]
    async fn rejects_unsupported_mime() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a.bmp");
        // BMP magic: "BM"
        fs::write(&p, b"BMfake-bmp-content-not-really").unwrap();
        let state = make_state(dir.path());
        let err = run(
            &state,
            ViewImageParams {
                source: "a.bmp".into(),
                max_dim: None,
                workdir: Some(dir.path().display().to_string()),
            },
        )
        .await
        .unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("unsupported image format"), "{msg}");
    }

    #[tokio::test]
    async fn rejects_animated_gif() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a.gif");
        // Build a 2-frame GIF.
        let mut bytes = Vec::new();
        {
            use image::codecs::gif::GifEncoder;
            use image::Frame;
            let mut enc = GifEncoder::new(&mut bytes);
            let f1: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_pixel(8, 8, Rgba([255, 0, 0, 255]));
            let f2: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_pixel(8, 8, Rgba([0, 255, 0, 255]));
            enc.encode_frame(Frame::new(f1)).unwrap();
            enc.encode_frame(Frame::new(f2)).unwrap();
        }
        fs::write(&p, &bytes).unwrap();
        let state = make_state(dir.path());
        let err = run(
            &state,
            ViewImageParams {
                source: "a.gif".into(),
                max_dim: None,
                workdir: Some(dir.path().display().to_string()),
            },
        )
        .await
        .unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("animated images not supported"), "{msg}");
    }

    #[test]
    fn data_url_round_trip() {
        // 1x1 transparent PNG
        let png_1x1: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let b64 = base64::engine::general_purpose::STANDARD.encode(png_1x1);
        let url = format!("data:image/png;base64,{b64}");
        let out = decode_data_url(&url).unwrap();
        assert_eq!(out, png_1x1);
    }

    #[test]
    fn data_url_rejects_non_base64() {
        let err = decode_data_url("data:image/png,raw-not-supported").unwrap_err();
        assert!(err.contains("base64"), "{err}");
    }

    #[test]
    fn data_url_rejects_non_image_mime() {
        let err = decode_data_url("data:text/plain;base64,aGVsbG8=").unwrap_err();
        assert!(err.contains("not an image"), "{err}");
    }

    #[test]
    fn rejects_unknown_url_scheme() {
        // `load_source` is async + needs SharedState; we hit the same branch
        // by constructing the runtime inline.
        let dir = tempdir().unwrap();
        let state = make_state(dir.path());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt
            .block_on(run(
                &state,
                ViewImageParams {
                    source: "ftp://example.com/foo.png".into(),
                    max_dim: None,
                    workdir: Some(dir.path().display().to_string()),
                },
            ))
            .unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("unsupported URL scheme"), "{msg}");
    }

    #[test]
    fn data_url_rejects_oversized_payload() {
        // 30 MiB of base64 = ~22.5 MiB raw — over MAX_SOURCE_BYTES.
        let huge = "A".repeat(30 * 1024 * 1024);
        let url = format!("data:image/png;base64,{huge}");
        let err = decode_data_url(&url).unwrap_err();
        assert!(err.contains("limit") && err.contains("raw bytes"), "{err}");
    }

    #[test]
    fn gif_scan_detects_single_frame_static() {
        // Build a real single-frame GIF and verify is_animated == false.
        let mut bytes = Vec::new();
        {
            use image::codecs::gif::GifEncoder;
            use image::Frame;
            let mut enc = GifEncoder::new(&mut bytes);
            let f1: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_pixel(8, 8, Rgba([255, 0, 0, 255]));
            enc.encode_frame(Frame::new(f1)).unwrap();
        }
        assert_eq!(sniff_mime(&bytes).unwrap(), "image/gif");
        assert!(
            !is_animated(&bytes, "image/gif"),
            "single-frame GIF must not register as animated"
        );
    }

    #[test]
    fn webp_scan_detects_animated_via_vp8x_flag() {
        // Synthesise a minimal VP8X WebP container with the ANIM flag bit
        // set. We don't need a valid VP8X payload — `is_animated` only
        // checks magic bytes + the ANIM bit.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&[0u8; 4]); // size — unchecked
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(b"VP8X");
        bytes.extend_from_slice(&[10, 0, 0, 0]); // chunk size
        bytes.push(0x02); // flags byte with ANIM bit (bit 1) set
        bytes.extend_from_slice(&[0u8; 9]); // padding to make slice long enough
        assert_eq!(sniff_mime(&bytes).unwrap(), "image/webp");
        assert!(is_animated(&bytes, "image/webp"));

        // And a static VP8X (no ANIM bit) is reported as not animated.
        bytes[20] = 0x00;
        assert!(!is_animated(&bytes, "image/webp"));
    }

    #[test]
    fn pixel_count_cap_rejects_decompression_bomb() {
        // Hand-roll a minimal valid PNG declaring 9000×9000 dimensions
        // (= 81M pixels, over MAX_PIXELS = 64M). The PNG decoder only needs
        // IHDR + a non-empty IDAT + IEND to surface dimensions via
        // `ImageReader::into_dimensions`. By constructing the file ourselves
        // we avoid allocating an 81M-element pixel buffer in the test.
        let png = synth_oversized_png(9000, 9000);
        let err = prepare(&png, 1568).unwrap_err();
        assert!(err.contains("pixels"), "{err}");
    }

    /// Synthesise a minimal IHDR + 1-byte-zlib IDAT + IEND PNG with the
    /// given dimensions. The image data is not meaningful — we only need
    /// the dim probe to succeed so `prepare`'s pixel-count cap fires.
    fn synth_oversized_png(w: u32, h: u32) -> Vec<u8> {
        fn crc(name: &[u8], data: &[u8]) -> u32 {
            // CRC32 with polynomial 0xEDB88320 (PNG standard).
            let mut c: u32 = 0xFFFF_FFFF;
            for &b in name.iter().chain(data) {
                c ^= b as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        (c >> 1) ^ 0xEDB8_8320
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&crc(kind, data).to_be_bytes());
        }
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 0, 0, 0, 0]); // bit-depth=8, grayscale, defaults
        chunk(&mut png, b"IHDR", &ihdr);
        // Minimal zlib-wrapped empty deflate stream: `78 9C` zlib header +
        // `03 00` empty stored block + Adler-32 of empty payload `00 00 00 01`.
        chunk(
            &mut png,
            b"IDAT",
            &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
        );
        chunk(&mut png, b"IEND", &[]);
        png
    }

    #[test]
    fn sniff_mime_recognises_all_four() {
        assert_eq!(sniff_mime(b"\x89PNG\r\n\x1a\nrest").unwrap(), "image/png");
        assert_eq!(
            sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]).unwrap(),
            "image/jpeg"
        );
        assert_eq!(sniff_mime(b"GIF89aXXX").unwrap(), "image/gif");
        let mut webp = Vec::new();
        webp.extend_from_slice(b"RIFF");
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBP");
        webp.extend_from_slice(b"VP8 ");
        assert_eq!(sniff_mime(&webp).unwrap(), "image/webp");
        sniff_mime(b"not-an-image").unwrap_err();
    }
}
