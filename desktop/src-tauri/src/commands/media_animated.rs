//! Structural metadata stripping for animated PNG and WebP uploads.
//!
//! Re-encoding an animated image through `image::DynamicImage` keeps only its
//! first frame. These helpers instead copy rendering chunks byte-for-byte while
//! dropping the metadata channels rejected by the relay.

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const PNG_ALLOWED_ANCILLARY: &[[u8; 4]] = &[
    *b"cHRM", *b"gAMA", *b"sBIT", *b"sRGB", *b"bKGD", *b"hIST", *b"tRNS", *b"sPLT", *b"acTL",
    *b"fcTL", *b"fdAT",
];
const WEBP_ALLOWED_CHUNKS: &[[u8; 4]] =
    &[*b"VP8 ", *b"VP8L", *b"VP8X", *b"ALPH", *b"ANIM", *b"ANMF"];
const WEBP_METADATA_FLAGS: u8 = 0x20 | 0x08 | 0x04;

#[derive(Clone, Copy)]
enum TiffEndian {
    Little,
    Big,
}

fn tiff_u16(bytes: &[u8], offset: usize, endian: TiffEndian) -> Option<u16> {
    let value: [u8; 2] = bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(match endian {
        TiffEndian::Little => u16::from_le_bytes(value),
        TiffEndian::Big => u16::from_be_bytes(value),
    })
}

fn tiff_u32(bytes: &[u8], offset: usize, endian: TiffEndian) -> Option<u32> {
    let value: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(match endian {
        TiffEndian::Little => u32::from_le_bytes(value),
        TiffEndian::Big => u32::from_be_bytes(value),
    })
}

fn exif_orientation(payload: &[u8]) -> Option<u16> {
    let tiff = payload.strip_prefix(b"Exif\0\0").unwrap_or(payload);
    let endian = match tiff.get(..2)? {
        b"II" => TiffEndian::Little,
        b"MM" => TiffEndian::Big,
        _ => return None,
    };
    if tiff_u16(tiff, 2, endian)? != 42 {
        return None;
    }

    let ifd_offset = usize::try_from(tiff_u32(tiff, 4, endian)?).ok()?;
    let entry_count = usize::from(tiff_u16(tiff, ifd_offset, endian)?);
    let entries_start = ifd_offset.checked_add(2)?;
    for index in 0..entry_count {
        let entry = entries_start.checked_add(index.checked_mul(12)?)?;
        if tiff_u16(tiff, entry, endian)? == 0x0112
            && tiff_u16(tiff, entry.checked_add(2)?, endian)? == 3
            && tiff_u32(tiff, entry.checked_add(4)?, endian)? == 1
        {
            return tiff_u16(tiff, entry.checked_add(8)?, endian);
        }
    }
    None
}

fn png_contains_chunk(body: &[u8], target: &[u8; 4]) -> bool {
    if !body.starts_with(PNG_SIGNATURE) {
        return false;
    }

    let mut offset = PNG_SIGNATURE.len();
    while offset < body.len() {
        let Some(header_end) = offset.checked_add(8).filter(|&end| end <= body.len()) else {
            return false;
        };
        let Some(payload_len) = body
            .get(offset..offset + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
            .and_then(|length| usize::try_from(length).ok())
        else {
            return false;
        };
        let Some(kind) = body.get(offset + 4..header_end) else {
            return false;
        };
        let Some(chunk_end) = header_end
            .checked_add(payload_len)
            .and_then(|end| end.checked_add(4))
            .filter(|&end| end <= body.len())
        else {
            return false;
        };
        if kind == target {
            return true;
        }
        offset = chunk_end;
        if kind == b"IEND" {
            return false;
        }
    }
    false
}

fn webp_contains_top_level_chunk(body: &[u8], target: &[u8; 4]) -> bool {
    if body.len() < 12 || &body[..4] != b"RIFF" || &body[8..12] != b"WEBP" {
        return false;
    }
    let Some(declared) = body
        .get(4..8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .and_then(|length| usize::try_from(length).ok())
    else {
        return false;
    };
    let Some(input_end) = declared
        .checked_add(8)
        .filter(|&end| (12..=body.len()).contains(&end))
    else {
        return false;
    };

    let mut offset = 12usize;
    while offset < input_end {
        let Some(header_end) = offset.checked_add(8).filter(|&end| end <= input_end) else {
            return false;
        };
        let Some(kind) = body.get(offset..offset + 4) else {
            return false;
        };
        let Some(payload_len) = body
            .get(offset + 4..header_end)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .and_then(|length| usize::try_from(length).ok())
        else {
            return false;
        };
        let Some(chunk_end) = payload_len
            .checked_add(payload_len & 1)
            .and_then(|length| header_end.checked_add(length))
            .filter(|&end| end <= input_end)
        else {
            return false;
        };
        if kind == target {
            return true;
        }
        offset = chunk_end;
    }
    false
}

/// Return true when removing an APNG ICC profile would change color rendering.
pub(crate) fn animated_png_uses_icc_profile(body: &[u8]) -> bool {
    png_contains_chunk(body, b"iCCP")
}

/// Return true when removing an animated WebP ICC profile would change colors.
pub(crate) fn animated_webp_uses_icc_profile(body: &[u8]) -> bool {
    webp_contains_top_level_chunk(body, b"ICCP")
}

/// Return true when an animated PNG relies on eXIf to rotate or mirror frames.
pub(crate) fn animated_png_uses_exif_orientation(body: &[u8]) -> bool {
    if !body.starts_with(PNG_SIGNATURE) {
        return false;
    }

    let mut offset = PNG_SIGNATURE.len();
    while offset < body.len() {
        let Some(header_end) = offset.checked_add(8).filter(|&end| end <= body.len()) else {
            return false;
        };
        let Some(payload_len) = body
            .get(offset..offset + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_be_bytes)
            .and_then(|length| usize::try_from(length).ok())
        else {
            return false;
        };
        let Some(kind) = body.get(offset + 4..header_end) else {
            return false;
        };
        let payload_start = header_end;
        let Some(chunk_end) = payload_start
            .checked_add(payload_len)
            .and_then(|end| end.checked_add(4))
            .filter(|&end| end <= body.len())
        else {
            return false;
        };
        if kind == b"eXIf"
            && exif_orientation(&body[payload_start..payload_start + payload_len])
                .is_some_and(|orientation| (2..=8).contains(&orientation))
        {
            return true;
        }
        offset = chunk_end;
        if kind == b"IEND" {
            return false;
        }
    }
    false
}

/// Return true when a WebP relies on EXIF to rotate or mirror its frames.
///
/// Structural metadata removal cannot bake this transform without decoding
/// and re-encoding every frame, so callers must reject this uncommon case
/// rather than silently changing how the animation renders.
pub(crate) fn animated_webp_uses_exif_orientation(body: &[u8]) -> bool {
    if body.len() < 12 || &body[..4] != b"RIFF" || &body[8..12] != b"WEBP" {
        return false;
    }
    let Some(declared) = body
        .get(4..8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
        .and_then(|length| usize::try_from(length).ok())
    else {
        return false;
    };
    let Some(input_end) = declared
        .checked_add(8)
        .filter(|&end| (12..=body.len()).contains(&end))
    else {
        return false;
    };

    let mut offset = 12usize;
    while offset < input_end {
        let Some(header_end) = offset.checked_add(8).filter(|&end| end <= input_end) else {
            return false;
        };
        let Some(kind) = body.get(offset..offset + 4) else {
            return false;
        };
        let Some(payload_len) = body
            .get(offset + 4..header_end)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
            .and_then(|length| usize::try_from(length).ok())
        else {
            return false;
        };
        let payload_start = header_end;
        let Some(chunk_end) = payload_len
            .checked_add(payload_len & 1)
            .and_then(|length| payload_start.checked_add(length))
            .filter(|&end| end <= input_end)
        else {
            return false;
        };
        if kind == b"EXIF"
            && exif_orientation(&body[payload_start..payload_start + payload_len])
                .is_some_and(|orientation| (2..=8).contains(&orientation))
        {
            return true;
        }
        offset = chunk_end;
    }
    false
}

/// Strip metadata-bearing ancillary chunks from a PNG without touching frame
/// control or image data. Bytes after `IEND` are truncated.
pub(crate) fn strip_animated_png_metadata(body: &[u8]) -> Option<Vec<u8>> {
    if !body.starts_with(PNG_SIGNATURE) {
        return None;
    }

    let mut output = Vec::with_capacity(body.len());
    output.extend_from_slice(PNG_SIGNATURE);
    let mut offset = PNG_SIGNATURE.len();

    while offset < body.len() {
        let header_end = offset.checked_add(8)?;
        if header_end > body.len() {
            return None;
        }
        let payload_len = u32::from_be_bytes(body[offset..offset + 4].try_into().ok()?) as usize;
        let kind: [u8; 4] = body[offset + 4..offset + 8].try_into().ok()?;
        let chunk_end = offset
            .checked_add(12)?
            .checked_add(payload_len)
            .filter(|&end| end <= body.len())?;

        let ancillary = kind[0] & 0x20 != 0;
        if !ancillary || PNG_ALLOWED_ANCILLARY.contains(&kind) {
            output.extend_from_slice(&body[offset..chunk_end]);
        }

        offset = chunk_end;
        if kind == *b"IEND" {
            return Some(output);
        }
    }

    None
}

fn append_webp_chunk(output: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) -> Option<()> {
    output.extend_from_slice(kind);
    output.extend_from_slice(&u32::try_from(payload.len()).ok()?.to_le_bytes());
    output.extend_from_slice(payload);
    if payload.len() & 1 != 0 {
        output.push(0);
    }
    Some(())
}

fn strip_anmf_metadata(payload: &[u8]) -> Option<Vec<u8>> {
    const FRAME_HEADER_LEN: usize = 16;
    if payload.len() < FRAME_HEADER_LEN {
        return None;
    }

    let mut output = Vec::with_capacity(payload.len());
    output.extend_from_slice(&payload[..FRAME_HEADER_LEN]);
    let mut offset = FRAME_HEADER_LEN;
    let mut saw_alpha = false;
    let mut saw_image = false;

    while offset < payload.len() {
        let header_end = offset.checked_add(8).filter(|&end| end <= payload.len())?;
        let kind: [u8; 4] = payload[offset..offset + 4].try_into().ok()?;
        let chunk_len =
            u32::from_le_bytes(payload[offset + 4..header_end].try_into().ok()?) as usize;
        let chunk_start = header_end;
        let chunk_end = chunk_len
            .checked_add(chunk_len & 1)
            .and_then(|length| chunk_start.checked_add(length))
            .filter(|&end| end <= payload.len())?;
        let chunk_payload = &payload[chunk_start..chunk_start + chunk_len];

        match &kind {
            b"ALPH" if !saw_alpha && !saw_image => {
                append_webp_chunk(&mut output, &kind, chunk_payload)?;
                saw_alpha = true;
            }
            b"VP8 " if !saw_image => {
                append_webp_chunk(&mut output, &kind, chunk_payload)?;
                saw_image = true;
            }
            b"VP8L" if !saw_alpha && !saw_image => {
                append_webp_chunk(&mut output, &kind, chunk_payload)?;
                saw_image = true;
            }
            b"ALPH" | b"VP8 " | b"VP8L" => return None,
            _ => {}
        }

        offset = chunk_end;
    }

    saw_image.then_some(output)
}

/// Strip metadata chunks and flags from a WebP container while retaining all
/// still/animated rendering chunks. RIFF padding is canonicalized to zero and
/// the container length is rewritten after removals.
pub(crate) fn strip_animated_webp_metadata(body: &[u8]) -> Option<Vec<u8>> {
    if body.len() < 12 || &body[..4] != b"RIFF" || &body[8..12] != b"WEBP" {
        return None;
    }

    let declared = u32::from_le_bytes(body[4..8].try_into().ok()?) as usize;
    let input_end = declared
        .checked_add(8)
        .filter(|&end| (12..=body.len()).contains(&end))?;
    let mut output = Vec::with_capacity(input_end);
    output.extend_from_slice(b"RIFF\0\0\0\0WEBP");
    let mut offset = 12usize;

    while offset < input_end {
        if offset.checked_add(8)? > input_end {
            return None;
        }
        let kind: [u8; 4] = body[offset..offset + 4].try_into().ok()?;
        let payload_len =
            u32::from_le_bytes(body[offset + 4..offset + 8].try_into().ok()?) as usize;
        let payload_start = offset + 8;
        let padded_len = payload_len.checked_add(payload_len & 1)?;
        let chunk_end = payload_start
            .checked_add(padded_len)
            .filter(|&end| end <= input_end)?;

        if WEBP_ALLOWED_CHUNKS.contains(&kind) {
            if kind == *b"VP8X" {
                let (&flags, rest) =
                    body[payload_start..payload_start + payload_len].split_first()?;
                let mut payload = Vec::with_capacity(payload_len);
                payload.push(flags & !WEBP_METADATA_FLAGS);
                payload.extend_from_slice(rest);
                append_webp_chunk(&mut output, &kind, &payload)?;
            } else if kind == *b"ANMF" {
                let payload =
                    strip_anmf_metadata(&body[payload_start..payload_start + payload_len])?;
                append_webp_chunk(&mut output, &kind, &payload)?;
            } else {
                append_webp_chunk(
                    &mut output,
                    &kind,
                    &body[payload_start..payload_start + payload_len],
                )?;
            }
        }

        offset = chunk_end;
    }

    let riff_len = u32::try_from(output.len().checked_sub(8)?).ok()?;
    output[4..8].copy_from_slice(&riff_len.to_le_bytes());
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::media::sanitize_image_for_upload;

    fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(payload);
        // The structural sanitizer copies CRCs without interpreting them. A
        // zero placeholder keeps these focused tests dependency-free.
        chunk.extend_from_slice(&[0; 4]);
        chunk
    }

    fn animated_png(metadata: bool) -> Vec<u8> {
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend_from_slice(&png_chunk(b"IHDR", &[0; 13]));
        png.extend_from_slice(&png_chunk(b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]));
        if metadata {
            png.extend_from_slice(&png_chunk(b"tEXt", b"Location\0secret"));
            png.extend_from_slice(&png_chunk(b"pHYs", &[0; 9]));
        }
        png.extend_from_slice(&png_chunk(b"fcTL", &[0; 26]));
        png.extend_from_slice(&png_chunk(b"IDAT", &[1, 2, 3]));
        png.extend_from_slice(&png_chunk(b"fdAT", &[0, 0, 0, 1, 4, 5]));
        png.extend_from_slice(&png_chunk(b"IEND", &[]));
        png
    }

    fn exif_orientation_payload(
        orientation: u16,
        endian: TiffEndian,
        include_preamble: bool,
    ) -> Vec<u8> {
        let mut exif = if include_preamble {
            b"Exif\0\0".to_vec()
        } else {
            Vec::new()
        };
        match endian {
            TiffEndian::Little => {
                exif.extend_from_slice(b"II");
                exif.extend_from_slice(&42u16.to_le_bytes());
                exif.extend_from_slice(&8u32.to_le_bytes());
                exif.extend_from_slice(&1u16.to_le_bytes());
                exif.extend_from_slice(&0x0112u16.to_le_bytes());
                exif.extend_from_slice(&3u16.to_le_bytes());
                exif.extend_from_slice(&1u32.to_le_bytes());
                exif.extend_from_slice(&orientation.to_le_bytes());
                exif.extend_from_slice(&[0; 2]);
                exif.extend_from_slice(&0u32.to_le_bytes());
            }
            TiffEndian::Big => {
                exif.extend_from_slice(b"MM");
                exif.extend_from_slice(&42u16.to_be_bytes());
                exif.extend_from_slice(&8u32.to_be_bytes());
                exif.extend_from_slice(&1u16.to_be_bytes());
                exif.extend_from_slice(&0x0112u16.to_be_bytes());
                exif.extend_from_slice(&3u16.to_be_bytes());
                exif.extend_from_slice(&1u32.to_be_bytes());
                exif.extend_from_slice(&orientation.to_be_bytes());
                exif.extend_from_slice(&[0; 2]);
                exif.extend_from_slice(&0u32.to_be_bytes());
            }
        }
        exif
    }

    fn animated_png_with_orientation(orientation: u16, endian: TiffEndian) -> Vec<u8> {
        let clean = animated_png(false);
        let mut png = clean[..33].to_vec();
        png.extend_from_slice(&png_chunk(
            b"eXIf",
            &exif_orientation_payload(orientation, endian, false),
        ));
        png.extend_from_slice(&clean[33..]);
        png
    }

    fn webp_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        chunk.extend_from_slice(payload);
        if payload.len() & 1 != 0 {
            chunk.push(0);
        }
        chunk
    }

    fn animated_webp(metadata: bool) -> Vec<u8> {
        let metadata_flags = if metadata { WEBP_METADATA_FLAGS } else { 0 };
        let mut chunks = webp_chunk(b"VP8X", &[metadata_flags | 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        chunks.extend_from_slice(&webp_chunk(b"ANIM", &[0; 6]));
        if metadata {
            chunks.extend_from_slice(&webp_chunk(b"EXIF", b"location"));
            chunks.extend_from_slice(&webp_chunk(b"XMP ", b"<xmp/>"));
            chunks.extend_from_slice(&webp_chunk(b"JUNK", b"private"));
        }
        let mut frame = vec![0; 16];
        frame.extend_from_slice(&webp_chunk(b"VP8 ", &[1, 2, 3]));
        chunks.extend_from_slice(&webp_chunk(b"ANMF", &frame));

        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
        webp.extend_from_slice(b"WEBP");
        webp.extend_from_slice(&chunks);
        webp
    }

    fn animated_webp_with_orientation(orientation: u16, endian: TiffEndian) -> Vec<u8> {
        let exif = exif_orientation_payload(orientation, endian, true);
        let mut chunks = webp_chunk(
            b"VP8X",
            &[WEBP_METADATA_FLAGS | 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        chunks.extend_from_slice(&webp_chunk(b"ANIM", &[0; 6]));
        chunks.extend_from_slice(&webp_chunk(b"EXIF", &exif));
        let mut frame = vec![0; 16];
        frame.extend_from_slice(&webp_chunk(b"VP8 ", &[1, 2, 3]));
        chunks.extend_from_slice(&webp_chunk(b"ANMF", &frame));
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
        webp.extend_from_slice(b"WEBP");
        webp.extend_from_slice(&chunks);
        webp
    }

    #[test]
    fn test_strip_animated_png_metadata_preserves_animation_chunks() {
        assert_eq!(
            strip_animated_png_metadata(&animated_png(true)),
            Some(animated_png(false))
        );
    }

    #[test]
    fn test_strip_animated_png_metadata_is_byte_identical_for_clean_input() {
        let clean = animated_png(false);
        assert_eq!(strip_animated_png_metadata(&clean), Some(clean));
    }

    #[test]
    fn test_strip_animated_webp_metadata_preserves_animation_chunks() {
        assert_eq!(
            strip_animated_webp_metadata(&animated_webp(true)),
            Some(animated_webp(false))
        );
    }

    #[test]
    fn test_strip_animated_webp_metadata_is_byte_identical_for_clean_input() {
        let clean = animated_webp(false);
        assert_eq!(strip_animated_webp_metadata(&clean), Some(clean));
    }

    #[test]
    fn test_detects_non_identity_animated_webp_exif_orientation() {
        for endian in [TiffEndian::Little, TiffEndian::Big] {
            assert!(!animated_webp_uses_exif_orientation(
                &animated_webp_with_orientation(1, endian)
            ));
            assert!(animated_webp_uses_exif_orientation(
                &animated_webp_with_orientation(6, endian)
            ));
        }
    }

    #[test]
    fn test_detects_non_identity_animated_png_exif_orientation() {
        for endian in [TiffEndian::Little, TiffEndian::Big] {
            assert!(!animated_png_uses_exif_orientation(
                &animated_png_with_orientation(1, endian)
            ));
            assert!(animated_png_uses_exif_orientation(
                &animated_png_with_orientation(6, endian)
            ));
        }
    }

    #[test]
    fn test_detects_animated_icc_profiles() {
        let clean_png = animated_png(false);
        let mut png = clean_png[..33].to_vec();
        png.extend_from_slice(&png_chunk(b"iCCP", b"profile"));
        png.extend_from_slice(&clean_png[33..]);
        assert!(animated_png_uses_icc_profile(&png));
        assert!(sanitize_image_for_upload(png, "image/png").is_err());
        assert!(!animated_png_uses_icc_profile(&clean_png));

        let mut chunks = webp_chunk(
            b"VP8X",
            &[WEBP_METADATA_FLAGS | 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        chunks.extend_from_slice(&webp_chunk(b"ICCP", b"profile"));
        chunks.extend_from_slice(&animated_webp(false)[30..]);
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&((chunks.len() + 4) as u32).to_le_bytes());
        webp.extend_from_slice(b"WEBP");
        webp.extend_from_slice(&chunks);
        assert!(animated_webp_uses_icc_profile(&webp));
        assert!(sanitize_image_for_upload(webp, "image/webp").is_err());
        assert!(!animated_webp_uses_icc_profile(&animated_webp(false)));
    }

    #[test]
    fn test_strip_animated_webp_removes_nested_frame_metadata() {
        let mut dirty_frame = vec![0; 16];
        dirty_frame.extend_from_slice(&webp_chunk(b"VP8 ", &[1, 2, 3]));
        dirty_frame.extend_from_slice(&webp_chunk(b"JUNK", b"location"));
        let mut clean_frame = vec![0; 16];
        clean_frame.extend_from_slice(&webp_chunk(b"VP8 ", &[1, 2, 3]));

        assert_eq!(strip_anmf_metadata(&dirty_frame), Some(clean_frame.clone()));

        clean_frame.extend_from_slice(&webp_chunk(b"VP8L", &[4]));
        assert!(strip_anmf_metadata(&clean_frame).is_none());
    }

    #[test]
    fn test_animated_sanitizers_truncate_trailing_bytes() {
        let clean_png = animated_png(false);
        let mut padded_png = clean_png.clone();
        padded_png.extend_from_slice(b"trailing metadata");
        assert_eq!(strip_animated_png_metadata(&padded_png), Some(clean_png));

        let clean_webp = animated_webp(false);
        let mut padded_webp = clean_webp.clone();
        padded_webp.extend_from_slice(b"trailing metadata");
        assert_eq!(strip_animated_webp_metadata(&padded_webp), Some(clean_webp));
    }

    #[test]
    fn test_animated_sanitizers_reject_malformed_containers() {
        assert!(strip_animated_png_metadata(PNG_SIGNATURE).is_none());
        assert!(strip_animated_webp_metadata(b"RIFF\x20\0\0\0WEBP").is_none());
    }

    #[test]
    fn test_upload_sanitizer_uses_structural_animation_scrubbers() {
        assert_eq!(
            sanitize_image_for_upload(animated_png(true), "image/png"),
            Ok(animated_png(false))
        );
        assert_eq!(
            sanitize_image_for_upload(animated_webp(true), "image/webp"),
            Ok(animated_webp(false))
        );
        assert_eq!(
            sanitize_image_for_upload(
                animated_webp_with_orientation(1, TiffEndian::Little),
                "image/webp"
            ),
            Ok(animated_webp(false))
        );
        assert_eq!(
            sanitize_image_for_upload(
                animated_png_with_orientation(1, TiffEndian::Little),
                "image/png"
            ),
            Ok(animated_png(false))
        );
        assert!(sanitize_image_for_upload(
            animated_webp_with_orientation(6, TiffEndian::Little),
            "image/webp"
        )
        .is_err());
        assert!(sanitize_image_for_upload(
            animated_png_with_orientation(6, TiffEndian::Little),
            "image/png"
        )
        .is_err());
    }
}
