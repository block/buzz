//! Preservation of Buzz snapshot tEXt chunks through the upload sanitizer.
//!
//! Split out of `media.rs` to keep that file under the desktop line-size
//! limit. Agent/team sharing embeds a manifest in a PNG `tEXt` chunk
//! (`buzz_agent_snapshot` / `buzz_team_snapshot`); the sanitizer's re-encode
//! would destroy it and the relay would previously reject it. These helpers
//! extract the chunk before the re-encode and re-inject it afterwards. The
//! relay allowlists exactly these keywords in `buzz-media::validation` — the
//! two lists must stay in sync.

/// tEXt keywords that carry Buzz snapshot manifests (`.agent.png` /
/// `.team.png`). These chunks are the product payload of agent/team sharing —
/// they must survive the metadata strip.
const PNG_SNAPSHOT_KEYWORDS: [&[u8]; 2] = [b"buzz_agent_snapshot", b"buzz_team_snapshot"];

/// Extract the raw bytes of the first Buzz snapshot tEXt chunk (length + type
/// + payload + CRC) from a PNG, or `None` when absent/not a PNG.
///
/// Walks the chunk structure directly instead of decoding the image so a
/// malformed file simply yields `None` and falls through to the normal
/// sanitize path.
pub(crate) fn extract_snapshot_text_chunk(bytes: &[u8]) -> Option<Vec<u8>> {
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
            let is_snapshot = PNG_SNAPSHOT_KEYWORDS.iter().any(|keyword| {
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
pub(crate) fn inject_snapshot_text_chunk(png: Vec<u8>, chunk: &[u8]) -> Result<Vec<u8>, String> {
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
    use super::super::media::sanitize_image_for_upload;

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

    #[test]
    fn test_agent_snapshot_survives_full_share_pipeline() {
        // Cross-contract regression for the production failure: a real
        // encoded .agent.png must survive export → client sanitize →
        // relay validation → import decode. Each seam is also covered by
        // unit tests, but this proves the exact pipeline that broke.
        use crate::managed_agents::agent_snapshot::{
            decode_snapshot_png, encode_snapshot_png, AgentSnapshot, AgentSnapshotDefinition,
            AgentSnapshotMemory, AgentSnapshotProfile, MemoryLevel,
        };

        let snapshot = AgentSnapshot {
            format: "buzz-agent-snapshot".to_string(),
            version: 1,
            definition: AgentSnapshotDefinition {
                name: "Tree Trunks".to_string(),
                source_is_builtin: false,
                system_prompt: Some("You are a helpful agent.".to_string()),
                runtime: Some("goose".to_string()),
                model: None,
                provider: None,
                parallelism: Some(1),
                respond_to: None,
                respond_to_allowlist: vec![],
                idle_timeout_seconds: None,
                max_turn_duration_seconds: None,
                name_pool: vec![],
            },
            profile: AgentSnapshotProfile {
                display_name: "Tree Trunks".to_string(),
                about: Some("Shared agent".to_string()),
                avatar_data_url: None,
                avatar_url: None,
            },
            memory: AgentSnapshotMemory {
                level: MemoryLevel::None,
                entries: vec![],
            },
        };

        // Export with a real avatar body (what the share flow uploads).
        let avatar = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([200, 120, 40, 255]),
        ));
        let mut avatar_png = std::io::Cursor::new(Vec::new());
        avatar
            .write_to(&mut avatar_png, image::ImageFormat::Png)
            .unwrap();
        let exported = encode_snapshot_png(&snapshot, Some(avatar_png.get_ref())).unwrap();

        // Client upload path.
        let sanitized = sanitize_image_for_upload(exported, "image/png").unwrap();

        // Relay ingest path.
        let relay_config = buzz_media_pkg::MediaConfig {
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
        };
        assert_eq!(
            buzz_media_pkg::validation::validate_content(&sanitized, &relay_config)
                .expect("relay rejected a sanitized agent snapshot PNG"),
            "image/png"
        );

        // Recipient import path.
        let imported = decode_snapshot_png(&sanitized)
            .expect("import failed on a PNG that passed sanitize + relay validation");
        assert_eq!(imported, snapshot);
    }
}
