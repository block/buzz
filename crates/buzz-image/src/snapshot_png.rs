//! Preservation of Buzz snapshot tEXt chunks through the upload sanitizer.
//!
//! Agent/team sharing embeds a manifest in a PNG `tEXt` chunk
//! (`buzz_agent_snapshot` / `buzz_team_snapshot`); the sanitizer's re-encode
//! would destroy it and the relay would previously reject it. These helpers
//! extract the chunk before the re-encode and re-inject it afterwards. The
//! allowlist itself lives in `crate::validate` and is shared with the relay's
//! validator, so there is exactly one list.

/// Extract the raw bytes of the first Buzz snapshot tEXt chunk (length + type
/// + payload + CRC) from a PNG, or `None` when absent/not a PNG.
///
/// Walks the chunk structure directly instead of decoding the image so a
/// malformed file simply yields `None` and falls through to the normal
/// sanitize path.
pub fn extract_snapshot_text_chunk(bytes: &[u8]) -> Option<Vec<u8>> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIG) {
        return None;
    }
    let mut i = SIG.len();
    while i + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().ok()?) as usize;
        let end = i.checked_add(12)?.checked_add(len)?;
        if end > bytes.len() {
            return None;
        }
        let kind = &bytes[i + 4..i + 8];
        if kind == b"tEXt" {
            let payload = &bytes[i + 8..i + 8 + len];
            let is_snapshot = crate::validate::PNG_SNAPSHOT_KEYWORDS
                .iter()
                .any(|keyword| {
                    payload.len() > keyword.len()
                        && &payload[..keyword.len()] == *keyword
                        && payload[keyword.len()] == 0
                });
            if is_snapshot {
                return Some(bytes[i..end].to_vec());
            }
        }
        if kind == b"IEND" {
            return None;
        }
        i = end;
    }
    None
}

/// Re-insert a raw snapshot tEXt chunk into a sanitized PNG, immediately
/// after the IHDR chunk. Placement matters: the `png` crate decoder used by
/// the import path only exposes text chunks encountered before IDAT via
/// `read_info()`. The chunk bytes carry their own CRC, which remains valid
/// because the chunk content is unchanged.
pub fn inject_snapshot_text_chunk(png: Vec<u8>, chunk: &[u8]) -> Result<Vec<u8>, String> {
    const SIG_LEN: usize = 8;
    // IHDR is always the first chunk of a well-formed PNG.
    if png.len() < SIG_LEN + 12 || &png[SIG_LEN + 4..SIG_LEN + 8] != b"IHDR" {
        return Err("sanitized PNG is missing IHDR chunk".to_string());
    }
    let ihdr_len = u32::from_be_bytes(
        png[SIG_LEN..SIG_LEN + 4]
            .try_into()
            .map_err(|_| "sanitized PNG has malformed IHDR length".to_string())?,
    ) as usize;
    let ihdr_end = SIG_LEN
        .checked_add(12)
        .and_then(|v| v.checked_add(ihdr_len))
        .filter(|&v| v <= png.len())
        .ok_or_else(|| "sanitized PNG has malformed IHDR chunk".to_string())?;
    let mut out = Vec::with_capacity(png.len() + chunk.len());
    out.extend_from_slice(&png[..ihdr_end]);
    out.extend_from_slice(chunk);
    out.extend_from_slice(&png[ihdr_end..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::sanitize_image_for_upload;

    #[test]
    fn test_sanitizer_preserves_agent_snapshot_text_chunk() {
        // Build a real 2×2 PNG carrying an agent-snapshot manifest chunk plus
        // a mundane metadata chunk that must NOT survive.
        for keyword in ["buzz_agent_snapshot", "buzz_team_snapshot"] {
            let manifest = "eyJmb3JtYXQiOiJidXp6LWFnZW50LXNuYXBzaG90In0=";
            let mut source = Vec::new();
            {
                let mut enc = png::Encoder::new(std::io::Cursor::new(&mut source), 2, 2);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.add_text_chunk(keyword.to_string(), manifest.to_string())
                    .unwrap();
                enc.add_text_chunk("Comment".to_string(), "GPS=37.7,-122.4".to_string())
                    .unwrap();
                let mut writer = enc.write_header().unwrap();
                writer.write_image_data(&[0u8; 16]).unwrap();
            }

            let sanitized = sanitize_image_for_upload(source, "image/png").unwrap();

            // The snapshot manifest survives, readable by the same decoder the
            // import path uses.
            let decoder = png::Decoder::new(std::io::Cursor::new(&sanitized));
            let reader = decoder.read_info().unwrap();
            let texts = &reader.info().uncompressed_latin1_text;
            let snapshot = texts
                .iter()
                .find(|c| c.keyword == keyword)
                .unwrap_or_else(|| panic!("sanitized PNG lost the {keyword} tEXt chunk"));
            assert_eq!(snapshot.text, manifest);

            // The mundane metadata chunk is stripped.
            assert!(
                !texts.iter().any(|c| c.keyword == "Comment"),
                "sanitizer kept a non-snapshot tEXt chunk"
            );
        }
    }

    #[test]
    fn test_sanitizer_still_strips_all_text_from_regular_pngs() {
        let mut source = Vec::new();
        {
            let mut enc = png::Encoder::new(std::io::Cursor::new(&mut source), 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.add_text_chunk("Comment".to_string(), "GPS=37.7,-122.4".to_string())
                .unwrap();
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(&[0u8; 16]).unwrap();
        }

        let sanitized = sanitize_image_for_upload(source, "image/png").unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(&sanitized));
        let reader = decoder.read_info().unwrap();
        assert!(reader.info().uncompressed_latin1_text.is_empty());
    }
}
