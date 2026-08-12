//! Structural metadata validation for image uploads.
//!
//! The relay is the authority on what it will accept; this module is that
//! authority's implementation, shared so upload clients can check the same
//! rules before hashing rather than discovering them as a 422.
//!
//! Deliberately a structural allowlist rather than an EXIF-tag denylist:
//! location can also live in XMP, comments, PNG text, ICC descriptions, or
//! private chunks.
//!
//! Size caps and the megapixel ceiling are NOT here — they are server policy
//! driven by `MediaConfig` and map to a different HTTP status, so they stay in
//! `buzz-media`.

use crate::ImageError;

/// tEXt keyword carrying an agent snapshot manifest (`.agent.png`).
pub const AGENT_SNAPSHOT_KEYWORD: &str = "buzz_agent_snapshot";
/// tEXt keyword carrying a team snapshot manifest (`.team.png`).
pub const TEAM_SNAPSHOT_KEYWORD: &str = "buzz_team_snapshot";

/// tEXt keywords that carry Buzz snapshot manifests (`.agent.png` /
/// `.team.png`). These are deliberate product payloads — agent/team sharing
/// embeds a manifest in a single tEXt chunk — so they are exempt from the
/// metadata ban. Exactly one snapshot chunk is permitted per file; every
/// other textual/metadata chunk remains forbidden.
///
/// Single source of truth: the validator below, the sanitizer that preserves
/// these chunks, and the desktop producers that write them all read this list.
/// It used to exist as three separate hardcoded copies across two crates, so
/// adding a third snapshot kind produced a file the producer wrote, the
/// sanitizer preserved, and the relay silently rejected.
pub const PNG_SNAPSHOT_KEYWORDS: [&[u8]; 2] = [
    AGENT_SNAPSHOT_KEYWORD.as_bytes(),
    TEAM_SNAPSHOT_KEYWORD.as_bytes(),
];

/// Reject metadata-bearing image structures without decoding pixel data.
///
/// This is deliberately a structural allowlist rather than an EXIF-tag denylist:
/// location can also live in XMP, comments, PNG text, ICC descriptions, or
/// private chunks. Client encoders remove these before upload.
pub fn validate_image_metadata_free(bytes: &[u8], mime: &str) -> Result<(), ImageError> {
    match mime {
        "image/jpeg" => validate_jpeg_metadata_free(bytes),
        "image/png" => validate_png_metadata_free(bytes),
        "image/webp" => validate_webp_metadata_free(bytes),
        "image/gif" => validate_gif_metadata_free(bytes),
        _ => Ok(()),
    }
}

fn validate_jpeg_metadata_free(bytes: &[u8]) -> Result<(), ImageError> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(ImageError::InvalidImage);
    }
    let mut i = 2usize;
    let mut in_scan = false;
    while i < bytes.len() {
        if bytes[i] != 0xff {
            if in_scan {
                i += 1;
                continue;
            }
            return Err(ImageError::InvalidImage);
        }
        while i < bytes.len() && bytes[i] == 0xff {
            i += 1;
        }
        if i >= bytes.len() {
            return Err(ImageError::InvalidImage);
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
                .ok_or(ImageError::MetadataForbidden);
        }
        if marker == 0xd8 {
            return Err(ImageError::InvalidImage);
        }
        if i + 2 > bytes.len() {
            return Err(ImageError::InvalidImage);
        }
        let len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if len < 2 {
            return Err(ImageError::InvalidImage);
        }
        let end = i
            .checked_add(len)
            .filter(|&end| end <= bytes.len())
            .ok_or(ImageError::InvalidImage)?;
        // Only canonical JFIF/Adobe colour headers are allowed. Their lengths and
        // identifiers are fixed; accepting arbitrary APP0/APP14 payloads would
        // leave a metadata side channel.
        if marker == 0xe0 {
            let payload = &bytes[i + 2..end];
            let canonical_jfif = payload.len() >= 14
                && &payload[..5] == b"JFIF\0"
                && payload.len() == 14 + 3 * payload[12] as usize * payload[13] as usize;
            if !canonical_jfif {
                return Err(ImageError::MetadataForbidden);
            }
        } else if marker == 0xee {
            let payload = &bytes[i + 2..end];
            if payload.len() != 12 || &payload[..5] != b"Adobe" {
                return Err(ImageError::MetadataForbidden);
            }
        } else if (0xe1..=0xed).contains(&marker) || marker == 0xef || marker == 0xfe {
            return Err(ImageError::MetadataForbidden);
        }
        i = end;
        in_scan = marker == 0xda;
    }
    Err(ImageError::InvalidImage)
}

/// Returns true when a raw tEXt chunk payload is a Buzz snapshot manifest:
/// the payload must start with an allowlisted keyword followed by the
/// keyword/text NUL separator.
fn is_snapshot_text_chunk(payload: &[u8]) -> bool {
    PNG_SNAPSHOT_KEYWORDS.iter().any(|keyword| {
        payload.len() > keyword.len()
            && &payload[..keyword.len()] == *keyword
            && payload[keyword.len()] == 0
    })
}

fn validate_png_metadata_free(bytes: &[u8]) -> Result<(), ImageError> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIG) {
        return Err(ImageError::InvalidImage);
    }
    let mut i = SIG.len();
    let mut saw_iend = false;
    let mut saw_snapshot_chunk = false;
    while i < bytes.len() {
        if i + 12 > bytes.len() {
            return Err(ImageError::InvalidImage);
        }
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = bytes[i + 4..i + 8].try_into().unwrap();
        let end = i
            .checked_add(12)
            .and_then(|v| v.checked_add(len))
            .filter(|&v| v <= bytes.len())
            .ok_or(ImageError::InvalidImage)?;
        if &kind == b"tEXt" {
            // Buzz agent/team snapshot manifests ride in a single tEXt chunk
            // with an allowlisted keyword. Anything else — other keywords, or
            // a second snapshot chunk — is a forbidden metadata channel.
            let payload = &bytes[i + 8..end - 4];
            if saw_snapshot_chunk || !is_snapshot_text_chunk(payload) {
                return Err(ImageError::MetadataForbidden);
            }
            saw_snapshot_chunk = true;
            i = end;
            continue;
        }
        if matches!(&kind, b"eXIf" | b"zTXt" | b"iTXt" | b"iCCP") {
            return Err(ImageError::MetadataForbidden);
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
            return Err(ImageError::MetadataForbidden);
        }
        i = end;
        if &kind == b"IEND" {
            saw_iend = true;
            break;
        }
    }
    if !saw_iend || i != bytes.len() {
        return Err(ImageError::MetadataForbidden);
    }
    Ok(())
}

fn validate_webp_metadata_free(bytes: &[u8]) -> Result<(), ImageError> {
    fn validate_frame_payload(payload: &[u8]) -> Result<(), ImageError> {
        const FRAME_HEADER_LEN: usize = 16;
        if payload.len() < FRAME_HEADER_LEN {
            return Err(ImageError::InvalidImage);
        }

        let mut i = FRAME_HEADER_LEN;
        let mut saw_alpha = false;
        let mut saw_image = false;
        while i < payload.len() {
            if i + 8 > payload.len() {
                return Err(ImageError::InvalidImage);
            }
            let kind: [u8; 4] = payload[i..i + 4].try_into().unwrap();
            let len = u32::from_le_bytes(payload[i + 4..i + 8].try_into().unwrap()) as usize;
            let padded = len.checked_add(len & 1).ok_or(ImageError::InvalidImage)?;
            i = i
                .checked_add(8)
                .and_then(|start| start.checked_add(padded))
                .filter(|&end| end <= payload.len())
                .ok_or(ImageError::InvalidImage)?;

            match &kind {
                b"ALPH" if !saw_alpha && !saw_image => saw_alpha = true,
                b"VP8 " if !saw_image => saw_image = true,
                b"VP8L" if !saw_alpha && !saw_image => saw_image = true,
                b"ALPH" | b"VP8 " | b"VP8L" => return Err(ImageError::InvalidImage),
                _ => return Err(ImageError::MetadataForbidden),
            }
        }

        saw_image.then_some(()).ok_or(ImageError::InvalidImage)
    }

    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(ImageError::InvalidImage);
    }
    let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if declared.checked_add(8) != Some(bytes.len()) {
        return Err(ImageError::MetadataForbidden);
    }
    let mut i = 12usize;
    while i < bytes.len() {
        if i + 8 > bytes.len() {
            return Err(ImageError::InvalidImage);
        }
        let kind: [u8; 4] = bytes[i..i + 4].try_into().unwrap();
        let len = u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) as usize;
        let payload_start = i + 8;
        let padded = len.checked_add(len & 1).ok_or(ImageError::InvalidImage)?;
        i = payload_start
            .checked_add(padded)
            .filter(|&v| v <= bytes.len())
            .ok_or(ImageError::InvalidImage)?;
        if !matches!(
            &kind,
            b"VP8 " | b"VP8L" | b"VP8X" | b"ALPH" | b"ANIM" | b"ANMF"
        ) {
            return Err(ImageError::MetadataForbidden);
        }
        if &kind == b"VP8X" {
            let flags = *bytes.get(payload_start).ok_or(ImageError::InvalidImage)?;
            // ICC, EXIF, and XMP presence flags are metadata even if a malformed
            // file omits their corresponding chunks.
            if flags & (0x20 | 0x08 | 0x04) != 0 {
                return Err(ImageError::MetadataForbidden);
            }
        } else if &kind == b"ANMF" {
            validate_frame_payload(&bytes[payload_start..payload_start + len])?;
        }
    }
    Ok(())
}

fn validate_gif_metadata_free(bytes: &[u8]) -> Result<(), ImageError> {
    if !(bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) || bytes.len() < 13 {
        return Err(ImageError::InvalidImage);
    }

    fn skip_sub_blocks(bytes: &[u8], i: &mut usize) -> Result<(), ImageError> {
        loop {
            let len = *bytes.get(*i).ok_or(ImageError::InvalidImage)? as usize;
            *i += 1;
            if len == 0 {
                return Ok(());
            }
            *i = i
                .checked_add(len)
                .filter(|&end| end <= bytes.len())
                .ok_or(ImageError::InvalidImage)?;
        }
    }

    let packed = bytes[10];
    let mut i = 13usize;
    if packed & 0x80 != 0 {
        let table_len = 3usize << ((packed & 0x07) as usize + 1);
        i = i
            .checked_add(table_len)
            .filter(|&end| end <= bytes.len())
            .ok_or(ImageError::InvalidImage)?;
    }

    loop {
        match *bytes.get(i).ok_or(ImageError::InvalidImage)? {
            0x2c => {
                // Image descriptor, optional local colour table, LZW code size,
                // then length-prefixed image-data sub-blocks.
                if i + 10 > bytes.len() {
                    return Err(ImageError::InvalidImage);
                }
                let image_packed = bytes[i + 9];
                i += 10;
                if image_packed & 0x80 != 0 {
                    let table_len = 3usize << ((image_packed & 0x07) as usize + 1);
                    i = i
                        .checked_add(table_len)
                        .filter(|&end| end <= bytes.len())
                        .ok_or(ImageError::InvalidImage)?;
                }
                i = i
                    .checked_add(1)
                    .filter(|&v| v <= bytes.len())
                    .ok_or(ImageError::InvalidImage)?;
                skip_sub_blocks(bytes, &mut i)?;
            }
            0x21 => {
                let label = *bytes.get(i + 1).ok_or(ImageError::InvalidImage)?;
                i += 2;
                match label {
                    // Graphic Control Extension carries rendering/animation state,
                    // not descriptive metadata. Its shape is fixed by the spec.
                    0xf9 => {
                        if bytes.get(i) != Some(&4) || i + 6 > bytes.len() || bytes[i + 5] != 0 {
                            return Err(ImageError::InvalidImage);
                        }
                        i += 6;
                    }
                    // Preserve only the standard looping application extensions.
                    // Other application, comment, and plain-text extensions are
                    // unrestricted metadata channels.
                    0xff => {
                        if bytes.get(i) != Some(&11) || i + 12 > bytes.len() {
                            return Err(ImageError::InvalidImage);
                        }
                        let app = &bytes[i + 1..i + 12];
                        if app != b"NETSCAPE2.0" && app != b"ANIMEXTS1.0" {
                            return Err(ImageError::MetadataForbidden);
                        }
                        i += 12;
                        if bytes.get(i) != Some(&3)
                            || bytes.get(i + 1) != Some(&1)
                            || bytes.get(i + 4) != Some(&0)
                        {
                            return Err(ImageError::MetadataForbidden);
                        }
                        i += 5;
                    }
                    _ => return Err(ImageError::MetadataForbidden),
                }
            }
            0x3b => {
                return (i + 1 == bytes.len())
                    .then_some(())
                    .ok_or(ImageError::MetadataForbidden);
            }
            _ => return Err(ImageError::InvalidImage),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    /// PNG CRC-32 (IEEE), computed over chunk type + payload.
    fn crc32(data: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for (i, entry) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *entry = c;
        }
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in data {
            crc = table[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^ 0xFFFF_FFFF
    }

    /// Assemble a raw PNG chunk (length + type + payload + CRC).
    pub fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut body = kind.to_vec();
        body.extend_from_slice(payload);
        let mut out = (payload.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }

    /// A valid 2x2 RGBA PNG with `extra` chunks spliced in after IHDR.
    ///
    /// Built with the `png` encoder so IDAT is genuinely well-formed; the
    /// extra chunks are injected structurally so a fixture can carry exactly
    /// the metadata channel under test and nothing else.
    pub fn png_with_chunks(extra: &[Vec<u8>]) -> Vec<u8> {
        let mut base = Vec::new();
        {
            let mut enc = png::Encoder::new(std::io::Cursor::new(&mut base), 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(&[0u8; 16]).unwrap();
        }
        if extra.is_empty() {
            return base;
        }
        // IHDR is always first; splice immediately after it.
        const SIG: usize = 8;
        let ihdr_len = u32::from_be_bytes(base[SIG..SIG + 4].try_into().unwrap()) as usize;
        let ihdr_end = SIG + 12 + ihdr_len;
        let mut out = base[..ihdr_end].to_vec();
        for c in extra {
            out.extend_from_slice(c);
        }
        out.extend_from_slice(&base[ihdr_end..]);
        out
    }

    /// A PNG carrying a `tEXt` chunk with the given keyword and text.
    pub fn text_chunk(keyword: &str, text: &str) -> Vec<u8> {
        let mut payload = keyword.as_bytes().to_vec();
        payload.push(0);
        payload.extend_from_slice(text.as_bytes());
        chunk(b"tEXt", &payload)
    }
}
