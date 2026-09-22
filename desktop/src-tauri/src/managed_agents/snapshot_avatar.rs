use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::ImageDecoder;
use std::io::Cursor;

const MAX_AVATAR_INLINE_BYTES: usize = 2 * 1024 * 1024;
const MAX_AVATAR_DIMENSION: u32 = 2048;
const MAX_AVATAR_DECODE_ALLOC: u64 = 32 * 1024 * 1024;

/// Materialize a snapshot PNG's visible pixels as a bounded portable avatar.
/// The exact transparent 1×1 no-avatar placeholder and images that cannot fit
/// the persisted inline-avatar budget leave the manifest fallback intact.
pub(crate) fn snapshot_png_avatar_data_url(png_bytes: &[u8]) -> Result<Option<String>, String> {
    let reader = image::ImageReader::with_format(Cursor::new(png_bytes), image::ImageFormat::Png);
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("Failed to decode snapshot avatar: {e}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_AVATAR_DIMENSION);
    limits.max_image_height = Some(MAX_AVATAR_DIMENSION);
    limits.max_alloc = Some(MAX_AVATAR_DECODE_ALLOC);
    decoder
        .set_limits(limits)
        .map_err(|e| format!("Snapshot avatar exceeds safe decoding limits: {e}"))?;
    let (width, height) = decoder.dimensions();
    let image = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| format!("Failed to decode snapshot avatar: {e}"))?;
    if width == 1 && height == 1 && image.to_rgba8().get_pixel(0, 0).0 == [0, 0, 0, 0] {
        return Ok(None);
    }

    let mut clean_png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut clean_png), image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode snapshot avatar: {e}"))?;
    if clean_png.len() > MAX_AVATAR_INLINE_BYTES {
        return Ok(None);
    }
    Ok(Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(clean_png)
    )))
}
