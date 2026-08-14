use super::{decode_avatar_data_url, MAX_AVATAR_FETCH_BYTES};

/// Prefer the agent's published profile picture when it is non-blank.
pub(super) fn preferred_avatar_url(
    kind0_picture: Option<String>,
    record_avatar_url: Option<String>,
) -> Option<String> {
    kind0_picture
        .filter(|picture| !picture.trim().is_empty())
        .or(record_avatar_url)
}

/// Resolve a native-decodable raster from an inline avatar.
///
/// Buzz emoji avatars are deliberately stored as percent-encoded SVG data
/// URLs so WebKit can render them crisply. The image crate does not decode
/// SVG, so Export may provide a PNG rasterized from that exact URL. The
/// fallback is accepted only when the authoritative avatar is SVG; ordinary
/// raster data URLs must decode on their own and cannot be replaced by an
/// arbitrary caller-provided image.
pub(super) fn resolve_inline_avatar_bytes(
    source_url: &str,
    svg_raster_fallback: Option<&str>,
) -> Result<Vec<u8>, String> {
    if let Some(bytes) = decode_raster_data_url(source_url) {
        return Ok(bytes);
    }

    if source_url.split_once(',').is_some_and(|(header, _)| {
        header
            .strip_prefix("data:")
            .and_then(|metadata| metadata.split(';').next())
            .is_some_and(|mime| mime.eq_ignore_ascii_case("image/svg+xml"))
    }) {
        if let Some(bytes) = svg_raster_fallback.and_then(decode_raster_data_url) {
            return Ok(bytes);
        }
    }

    Err("Agent avatar data URL could not be decoded.".to_string())
}

fn decode_raster_data_url(url: &str) -> Option<Vec<u8>> {
    let bytes = decode_avatar_data_url(url)?;
    if bytes.len() > MAX_AVATAR_FETCH_BYTES || image::load_from_memory(&bytes).is_err() {
        return None;
    }
    Some(bytes)
}
