//! Structural metadata strip for uploaded images.
//!
//! [`strip_image_metadata`] copies the rendering structure of a PNG, GIF, JPEG
//! or WebP and drops every metadata channel that
//! [`validate_content`](crate::validation::validate_content) rejects. It never
//! decodes or re-encodes pixel data, so the output stays byte-close to the
//! input. Its output is meant to pass the validator; callers must still run it.

use crate::error::MediaError;

const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
const PNG_KEEP_ANCILLARY: [&[u8; 4]; 11] = [
    b"cHRM", b"gAMA", b"sBIT", b"sRGB", b"bKGD", b"hIST", b"tRNS", b"sPLT", b"acTL", b"fcTL",
    b"fdAT",
];

/// Remove metadata from `bytes` and return the stripped image.
///
/// The format comes from the magic bytes. Dropped data: PNG text, EXIF, ICC and
/// every other non-rendering ancillary chunk; GIF comment, plain-text and
/// non-canonical application extensions; JPEG APPn and COM segments (a
/// canonical JFIF or Adobe segment stays); WebP ICC, EXIF, XMP and unknown
/// chunks. Bytes after the end-of-image marker are dropped.
///
/// # Errors
///
/// [`MediaError::UnknownContentType`] or [`MediaError::DisallowedContentType`]
/// when the format is not one of the four, and [`MediaError::InvalidImage`]
/// when the container is malformed or truncated.
pub fn strip_image_metadata(bytes: &[u8]) -> Result<Vec<u8>, MediaError> {
    let mime = infer::get(bytes)
        .map(|t| t.mime_type())
        .ok_or(MediaError::UnknownContentType)?;
    match mime {
        "image/png" => strip_png(bytes),
        "image/gif" => strip_gif(bytes),
        "image/jpeg" => strip_jpeg(bytes),
        "image/webp" => strip_webp(bytes),
        other => Err(MediaError::DisallowedContentType(other.to_string())),
    }
}

fn be32(bytes: &[u8], at: usize) -> Result<u32, MediaError> {
    let raw: [u8; 4] = bytes
        .get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or(MediaError::InvalidImage)?;
    Ok(u32::from_be_bytes(raw))
}

fn le32(bytes: &[u8], at: usize) -> Result<u32, MediaError> {
    let raw: [u8; 4] = bytes
        .get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or(MediaError::InvalidImage)?;
    Ok(u32::from_le_bytes(raw))
}

fn strip_png(bytes: &[u8]) -> Result<Vec<u8>, MediaError> {
    if !bytes.starts_with(PNG_SIG) {
        return Err(MediaError::InvalidImage);
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(PNG_SIG);
    let mut i = PNG_SIG.len();
    while i < bytes.len() {
        let len = be32(bytes, i)? as usize;
        let kind: [u8; 4] = bytes
            .get(i + 4..i + 8)
            .and_then(|s| s.try_into().ok())
            .ok_or(MediaError::InvalidImage)?;
        let end = i
            .checked_add(12)
            .and_then(|v| v.checked_add(len))
            .filter(|&v| v <= bytes.len())
            .ok_or(MediaError::InvalidImage)?;
        let critical = kind[0] & 0x20 == 0;
        if critical || PNG_KEEP_ANCILLARY.contains(&&kind) {
            out.extend_from_slice(&bytes[i..end]);
        }
        i = end;
        if &kind == b"IEND" {
            return Ok(out);
        }
    }
    Err(MediaError::InvalidImage)
}

fn skip_gif_sub_blocks(bytes: &[u8], i: &mut usize) -> Result<(), MediaError> {
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

fn gif_color_table_len(packed: u8) -> usize {
    3usize << ((packed & 0x07) as usize + 1)
}

fn strip_gif(bytes: &[u8]) -> Result<Vec<u8>, MediaError> {
    if !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) || bytes.len() < 13 {
        return Err(MediaError::InvalidImage);
    }
    let mut i = 13usize;
    if bytes[10] & 0x80 != 0 {
        i = i
            .checked_add(gif_color_table_len(bytes[10]))
            .filter(|&end| end <= bytes.len())
            .ok_or(MediaError::InvalidImage)?;
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..i]);
    loop {
        let block_start = i;
        match *bytes.get(i).ok_or(MediaError::InvalidImage)? {
            0x2c => {
                let packed = *bytes.get(i + 9).ok_or(MediaError::InvalidImage)?;
                i += 10;
                if packed & 0x80 != 0 {
                    i = i
                        .checked_add(gif_color_table_len(packed))
                        .filter(|&end| end <= bytes.len())
                        .ok_or(MediaError::InvalidImage)?;
                }
                i = i
                    .checked_add(1)
                    .filter(|&v| v <= bytes.len())
                    .ok_or(MediaError::InvalidImage)?;
                skip_gif_sub_blocks(bytes, &mut i)?;
                out.extend_from_slice(&bytes[block_start..i]);
            }
            0x21 => {
                let label = *bytes.get(i + 1).ok_or(MediaError::InvalidImage)?;
                i += 2;
                match label {
                    0xf9 => {
                        if bytes.get(i) != Some(&4) || i + 6 > bytes.len() || bytes[i + 5] != 0 {
                            return Err(MediaError::InvalidImage);
                        }
                        i += 6;
                        out.extend_from_slice(&bytes[block_start..i]);
                    }
                    0xff => {
                        let canonical_loop = bytes.get(i) == Some(&11)
                            && i + 12 <= bytes.len()
                            && matches!(&bytes[i + 1..i + 12], b"NETSCAPE2.0" | b"ANIMEXTS1.0")
                            && bytes.get(i + 12) == Some(&3)
                            && bytes.get(i + 13) == Some(&1)
                            && bytes.get(i + 16) == Some(&0);
                        if canonical_loop {
                            i += 17;
                            out.extend_from_slice(&bytes[block_start..i]);
                        } else {
                            skip_gif_sub_blocks(bytes, &mut i)?;
                        }
                    }
                    _ => skip_gif_sub_blocks(bytes, &mut i)?,
                }
            }
            0x3b => {
                out.push(0x3b);
                return Ok(out);
            }
            _ => return Err(MediaError::InvalidImage),
        }
    }
}

fn strip_jpeg(bytes: &[u8]) -> Result<Vec<u8>, MediaError> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(MediaError::InvalidImage);
    }
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&[0xff, 0xd8]);
    let mut i = 2usize;
    let mut in_scan = false;
    while i < bytes.len() {
        if bytes[i] != 0xff {
            if in_scan {
                out.push(bytes[i]);
                i += 1;
                continue;
            }
            return Err(MediaError::InvalidImage);
        }
        while i < bytes.len() && bytes[i] == 0xff {
            i += 1;
        }
        let marker = *bytes.get(i).ok_or(MediaError::InvalidImage)?;
        i += 1;
        if in_scan && marker == 0x00 {
            out.extend_from_slice(&[0xff, 0x00]);
            continue;
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            out.extend_from_slice(&[0xff, marker]);
            continue;
        }
        if marker == 0xd9 {
            out.extend_from_slice(&[0xff, 0xd9]);
            return Ok(out);
        }
        if marker == 0xd8 || i + 2 > bytes.len() {
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
        let payload = &bytes[i + 2..end];
        let keep = match marker {
            0xe0 => {
                payload.len() >= 14
                    && &payload[..5] == b"JFIF\0"
                    && payload.len() == 14 + 3 * payload[12] as usize * payload[13] as usize
            }
            0xee => payload.len() == 12 && &payload[..5] == b"Adobe",
            0xe1..=0xed | 0xef | 0xfe => false,
            _ => true,
        };
        if keep {
            out.extend_from_slice(&[0xff, marker]);
            out.extend_from_slice(&bytes[i..end]);
        }
        i = end;
        in_scan = marker == 0xda;
    }
    Err(MediaError::InvalidImage)
}

fn push_riff_chunk(out: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() & 1 == 1 {
        out.push(0);
    }
}

fn strip_webp_frame(payload: &[u8]) -> Result<Vec<u8>, MediaError> {
    const FRAME_HEADER_LEN: usize = 16;
    if payload.len() < FRAME_HEADER_LEN {
        return Err(MediaError::InvalidImage);
    }
    let mut out = payload[..FRAME_HEADER_LEN].to_vec();
    let mut i = FRAME_HEADER_LEN;
    while i < payload.len() {
        let (kind, body, next) = read_riff_chunk(payload, i)?;
        if matches!(&kind, b"ALPH" | b"VP8 " | b"VP8L") {
            push_riff_chunk(&mut out, &kind, body);
        }
        i = next;
    }
    Ok(out)
}

fn read_riff_chunk(bytes: &[u8], at: usize) -> Result<([u8; 4], &[u8], usize), MediaError> {
    let kind: [u8; 4] = bytes
        .get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .ok_or(MediaError::InvalidImage)?;
    let len = le32(bytes, at + 4)? as usize;
    let start = at + 8;
    let body_end = start
        .checked_add(len)
        .filter(|&v| v <= bytes.len())
        .ok_or(MediaError::InvalidImage)?;
    let next = body_end
        .checked_add(len & 1)
        .filter(|&v| v <= bytes.len())
        .ok_or(MediaError::InvalidImage)?;
    Ok((kind, &bytes[start..body_end], next))
}

fn strip_webp(bytes: &[u8]) -> Result<Vec<u8>, MediaError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(MediaError::InvalidImage);
    }
    let declared = le32(bytes, 4)? as usize;
    if declared
        .checked_add(8)
        .is_none_or(|total| total > bytes.len())
    {
        return Err(MediaError::InvalidImage);
    }
    let bytes = &bytes[..declared + 8];
    let mut body = b"WEBP".to_vec();
    let mut i = 12usize;
    while i < bytes.len() {
        let (kind, payload, next) = read_riff_chunk(bytes, i)?;
        match &kind {
            b"VP8X" => {
                let mut copy = payload.to_vec();
                let flags = copy.first_mut().ok_or(MediaError::InvalidImage)?;
                *flags &= !(0x20 | 0x08 | 0x04);
                push_riff_chunk(&mut body, &kind, &copy);
            }
            b"ANMF" => push_riff_chunk(&mut body, &kind, &strip_webp_frame(payload)?),
            b"VP8 " | b"VP8L" | b"ALPH" | b"ANIM" => push_riff_chunk(&mut body, &kind, payload),
            _ => {}
        }
        i = next;
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MediaConfig;
    use crate::validation::validate_content;
    use image::{ImageEncoder, RgbaImage};

    fn config() -> MediaConfig {
        MediaConfig::offline_validation()
    }

    fn rgba() -> RgbaImage {
        RgbaImage::from_fn(4, 4, |x, y| {
            image::Rgba([x as u8 * 60, y as u8 * 60, 7, 255])
        })
    }

    fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut c = (payload.len() as u32).to_be_bytes().to_vec();
        c.extend_from_slice(kind);
        c.extend_from_slice(payload);
        c.extend_from_slice(&[0, 0, 0, 0]);
        c
    }

    fn base_png() -> Vec<u8> {
        let mut buf = Vec::new();
        image::codecs::png::PngEncoder::new(&mut buf)
            .write_image(rgba().as_raw(), 4, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        buf
    }

    fn png_with_text() -> Vec<u8> {
        let base = base_png();
        let ihdr_end = 8 + 12 + 13;
        let mut out = base[..ihdr_end].to_vec();
        out.extend(png_chunk(b"tEXt", b"Comment\0secret"));
        out.extend(png_chunk(b"eXIf", b"Exif\0\0"));
        out.extend(png_chunk(b"iCCP", b"p\0\0x"));
        out.extend(png_chunk(b"tIME", &[7, 230, 1, 1, 0, 0, 0]));
        out.extend(png_chunk(b"gAMA", &[0, 1, 0, 0]));
        out.extend_from_slice(&base[ihdr_end..]);
        out.extend_from_slice(b"trailing");
        out
    }

    fn base_gif() -> Vec<u8> {
        let mut buf = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut buf)
            .encode(rgba().as_raw(), 4, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        buf
    }

    fn gif_with_metadata() -> Vec<u8> {
        let base = base_gif();
        let mut out = base[..13].to_vec();
        let mut i = 13;
        if base[10] & 0x80 != 0 {
            i += gif_color_table_len(base[10]);
            out.extend_from_slice(&base[13..i]);
        }
        out.extend_from_slice(&[0x21, 0xfe, 5, b'h', b'e', b'l', b'l', b'o', 0]);
        out.extend_from_slice(&[0x21, 0xff, 11]);
        out.extend_from_slice(b"XMP DataXMP");
        out.extend_from_slice(&[2, 9, 9, 0]);
        out.extend_from_slice(&[0x21, 0x01, 3, 1, 2, 3, 0]);
        out.extend_from_slice(&base[i..]);
        out
    }

    fn base_jpeg() -> Vec<u8> {
        let mut buf = Vec::new();
        let rgb = image::DynamicImage::ImageRgba8(rgba()).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new(&mut buf)
            .write_image(rgb.as_raw(), 4, 4, image::ExtendedColorType::Rgb8)
            .unwrap();
        buf
    }

    fn jpeg_segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut s = vec![0xff, marker];
        s.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        s.extend_from_slice(payload);
        s
    }

    fn jpeg_with_metadata() -> Vec<u8> {
        let base = base_jpeg();
        let mut out = base[..2].to_vec();
        out.extend(jpeg_segment(0xe1, b"Exif\0\0data"));
        out.extend(jpeg_segment(0xe2, b"ICC_PROFILE\0"));
        out.extend(jpeg_segment(0xfe, b"a comment"));
        out.extend_from_slice(&base[2..]);
        out.extend_from_slice(b"trailing");
        out
    }

    fn base_webp() -> Vec<u8> {
        let mut buf = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut buf)
            .write_image(rgba().as_raw(), 4, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        buf
    }

    fn webp_with_metadata() -> Vec<u8> {
        let base = base_webp();
        let mut body = b"WEBP".to_vec();
        let vp8x = vec![0x20 | 0x08 | 0x04 | 0x10, 0, 0, 0, 3, 0, 0, 3, 0, 0];
        push_riff_chunk(&mut body, b"VP8X", &vp8x);
        push_riff_chunk(&mut body, b"ICCP", b"profile!!");
        let mut i = 12;
        while i < base.len() {
            let (kind, payload, next) = read_riff_chunk(&base, i).unwrap();
            if &kind != b"VP8X" {
                push_riff_chunk(&mut body, &kind, payload);
            }
            i = next;
        }
        push_riff_chunk(&mut body, b"EXIF", b"Exif\0\0");
        push_riff_chunk(&mut body, b"XMP ", b"<x:xmpmeta/>");
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn metadata_bearing_images_are_rejected_before_and_accepted_after() {
        let cases = [
            ("png", png_with_text()),
            ("gif", gif_with_metadata()),
            ("jpeg", jpeg_with_metadata()),
            ("webp", webp_with_metadata()),
        ];
        for (name, input) in cases {
            assert!(
                matches!(
                    validate_content(&input, &config()),
                    Err(MediaError::MetadataForbidden)
                ),
                "{name} fixture must carry forbidden metadata"
            );
            let stripped = strip_image_metadata(&input).unwrap();
            validate_content(&stripped, &config())
                .unwrap_or_else(|e| panic!("{name} stripped output rejected: {e}"));
            assert!(stripped.len() < input.len(), "{name} should shrink");
            assert_eq!(
                strip_image_metadata(&stripped).unwrap(),
                stripped,
                "{name} strip is idempotent"
            );
        }
    }

    #[test]
    fn pixels_survive_the_strip() {
        for input in [png_with_text(), gif_with_metadata(), jpeg_with_metadata()] {
            let before = image::load_from_memory(&input).unwrap().to_rgba8();
            let stripped = strip_image_metadata(&input).unwrap();
            let after = image::load_from_memory(&stripped).unwrap().to_rgba8();
            assert_eq!(before, after);
        }
    }

    #[test]
    fn clean_images_pass_through_unchanged() {
        for input in [base_png(), base_gif(), base_jpeg(), base_webp()] {
            validate_content(&input, &config()).expect("fixture is clean");
            assert_eq!(strip_image_metadata(&input).unwrap(), input);
        }
    }

    #[test]
    fn png_keeps_rendering_chunks() {
        let stripped = strip_image_metadata(&png_with_text()).unwrap();
        assert!(stripped.windows(4).any(|w| w == b"gAMA"));
        for gone in [&b"tEXt"[..], b"eXIf", b"iCCP", b"tIME", b"trailing"] {
            assert!(!stripped.windows(gone.len()).any(|w| w == gone));
        }
    }

    #[test]
    fn webp_strip_clears_vp8x_metadata_flags_and_resizes_riff() {
        let stripped = strip_image_metadata(&webp_with_metadata()).unwrap();
        let declared = u32::from_le_bytes(stripped[4..8].try_into().unwrap()) as usize;
        assert_eq!(declared + 8, stripped.len());
        let vp8x = stripped.windows(4).position(|w| w == b"VP8X").unwrap();
        assert_eq!(stripped[vp8x + 8] & (0x20 | 0x08 | 0x04), 0);
        assert_eq!(stripped[vp8x + 8] & 0x10, 0x10, "alpha flag stays");
    }

    #[test]
    fn malformed_and_unsupported_inputs_are_errors() {
        let png = base_png();
        let jpeg = base_jpeg();
        let gif = base_gif();
        let webp = base_webp();
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", vec![]),
            ("text", b"hello world, not an image".to_vec()),
            ("png truncated", png[..png.len() - 5].to_vec()),
            ("gif truncated", gif[..gif.len() - 3].to_vec()),
            ("jpeg truncated", jpeg[..jpeg.len() - 2].to_vec()),
            ("webp truncated", webp[..webp.len() - 4].to_vec()),
        ];
        for (name, input) in cases {
            assert!(strip_image_metadata(&input).is_err(), "{name} must error");
        }
    }
}
