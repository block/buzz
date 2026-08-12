use image::{GenericImageView, ImageDecoder};

const MAX_IMAGE_PIXELS: u64 = 25_000_000;
const PNG_SNAPSHOT_KEYWORDS: [&[u8]; 2] = [b"buzz_agent_snapshot", b"buzz_team_snapshot"];

pub(crate) struct PreparedImageUpload {
    pub bytes: Vec<u8>,
    pub resized_from: Option<(u32, u32)>,
    pub dimensions: (u32, u32),
}

fn image_format(mime: &str) -> Option<image::ImageFormat> {
    match mime {
        "image/jpeg" => Some(image::ImageFormat::Jpeg),
        "image/png" => Some(image::ImageFormat::Png),
        "image/webp" => Some(image::ImageFormat::WebP),
        _ => None,
    }
}

fn is_animated_image(bytes: &[u8], mime: &str) -> bool {
    match mime {
        "image/png" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => {
            let mut offset = 8usize;
            while offset.checked_add(12).is_some_and(|end| end <= bytes.len()) {
                let length = u32::from_be_bytes([
                    bytes[offset],
                    bytes[offset + 1],
                    bytes[offset + 2],
                    bytes[offset + 3],
                ]) as usize;
                let Some(end) = offset
                    .checked_add(12)
                    .and_then(|value| value.checked_add(length))
                else {
                    return false;
                };
                if end > bytes.len() {
                    return false;
                }
                if &bytes[offset + 4..offset + 8] == b"acTL" {
                    return true;
                }
                offset = end;
            }
            false
        }
        "image/webp"
            if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" =>
        {
            let mut offset = 12usize;
            while offset.checked_add(8).is_some_and(|end| end <= bytes.len()) {
                let chunk = &bytes[offset..offset + 4];
                if chunk == b"ANIM" || chunk == b"ANMF" {
                    return true;
                }
                let length = u32::from_le_bytes([
                    bytes[offset + 4],
                    bytes[offset + 5],
                    bytes[offset + 6],
                    bytes[offset + 7],
                ]) as usize;
                let Some(end) = length
                    .checked_add(length & 1)
                    .and_then(|value| offset.checked_add(8 + value))
                else {
                    return false;
                };
                if end > bytes.len() {
                    return false;
                }
                offset = end;
            }
            false
        }
        _ => false,
    }
}

fn fit_dimensions_to_pixel_limit(width: u32, height: u32, max_pixels: u64) -> (u32, u32) {
    let pixels = u64::from(width) * u64::from(height);
    if pixels <= max_pixels || pixels == 0 {
        return (width, height);
    }
    let scale = (max_pixels as f64 / pixels as f64).sqrt();
    let mut next_width = ((f64::from(width) * scale).floor() as u32).max(1);
    let mut next_height = ((f64::from(height) * scale).floor() as u32).max(1);
    while u64::from(next_width) * u64::from(next_height) > max_pixels {
        if next_width >= next_height {
            next_width -= 1;
        } else {
            next_height -= 1;
        }
    }
    (next_width, next_height)
}

fn extract_snapshot_text_chunk(bytes: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return None;
    }
    let mut offset = SIGNATURE.len();
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().ok()?) as usize;
        let end = offset.checked_add(12)?.checked_add(length)?;
        if end > bytes.len() {
            return None;
        }
        let kind = &bytes[offset + 4..offset + 8];
        if kind == b"tEXt" {
            let payload = &bytes[offset + 8..offset + 8 + length];
            let is_snapshot = PNG_SNAPSHOT_KEYWORDS.iter().any(|keyword| {
                payload.len() > keyword.len()
                    && &payload[..keyword.len()] == *keyword
                    && payload[keyword.len()] == 0
            });
            if is_snapshot {
                return Some(bytes[offset..end].to_vec());
            }
        }
        if kind == b"IEND" {
            return None;
        }
        offset = end;
    }
    None
}

fn inject_snapshot_text_chunk(png: Vec<u8>, chunk: &[u8]) -> Result<Vec<u8>, String> {
    const SIGNATURE_LENGTH: usize = 8;
    if png.len() < SIGNATURE_LENGTH + 12
        || &png[SIGNATURE_LENGTH + 4..SIGNATURE_LENGTH + 8] != b"IHDR"
    {
        return Err("sanitized PNG is missing its IHDR chunk".to_string());
    }
    let ihdr_length = u32::from_be_bytes(
        png[SIGNATURE_LENGTH..SIGNATURE_LENGTH + 4]
            .try_into()
            .map_err(|_| "sanitized PNG has a malformed IHDR length".to_string())?,
    ) as usize;
    let ihdr_end = SIGNATURE_LENGTH
        .checked_add(12)
        .and_then(|value| value.checked_add(ihdr_length))
        .filter(|end| *end <= png.len())
        .ok_or_else(|| "sanitized PNG has a malformed IHDR chunk".to_string())?;
    let mut output = Vec::with_capacity(png.len() + chunk.len());
    output.extend_from_slice(&png[..ihdr_end]);
    output.extend_from_slice(chunk);
    output.extend_from_slice(&png[ihdr_end..]);
    Ok(output)
}

fn prepare_image_upload_with_limit(
    bytes: Vec<u8>,
    mime: &str,
    max_pixels: u64,
) -> Result<PreparedImageUpload, String> {
    let Some(format) = image_format(mime) else {
        let dimensions = imagesize::blob_size(&bytes)
            .map(|size| (size.width as u32, size.height as u32))
            .unwrap_or((0, 0));
        return Ok(PreparedImageUpload {
            bytes,
            resized_from: None,
            dimensions,
        });
    };
    // Re-encoding animated PNG/WebP would flatten the animation. Preserve it
    // byte-for-byte and leave the relay as the final structural validator.
    if is_animated_image(&bytes, mime) {
        let dimensions = imagesize::blob_size(&bytes)
            .map(|size| (size.width as u32, size.height as u32))
            .unwrap_or((0, 0));
        return Ok(PreparedImageUpload {
            bytes,
            resized_from: None,
            dimensions,
        });
    }

    let snapshot_chunk = (format == image::ImageFormat::Png)
        .then(|| extract_snapshot_text_chunk(&bytes))
        .flatten();
    let reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "failed to decode image for safe upload".to_string())?;
    decoder
        .set_limits(image::Limits::default())
        .map_err(|_| "image exceeds safe decoding limits".to_string())?;
    let orientation = decoder
        .orientation()
        .map_err(|_| "failed to read image orientation".to_string())?;
    let mut image = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| "failed to decode image for safe upload".to_string())?;
    image.apply_orientation(orientation);

    let original_dimensions = image.dimensions();
    let fitted =
        fit_dimensions_to_pixel_limit(original_dimensions.0, original_dimensions.1, max_pixels);
    let resized_from = (fitted != original_dimensions).then_some(original_dimensions);
    if resized_from.is_some() {
        image = image.resize_exact(fitted.0, fitted.1, image::imageops::FilterType::Lanczos3);
    }

    let mut output = std::io::Cursor::new(Vec::new());
    if format == image::ImageFormat::Jpeg {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 92)
            .encode_image(&image)
            .map_err(|_| "failed to encode JPEG without metadata".to_string())?;
    } else {
        image
            .write_to(&mut output, format)
            .map_err(|_| "failed to encode image without metadata".to_string())?;
    }
    let output = match snapshot_chunk {
        Some(chunk) => inject_snapshot_text_chunk(output.into_inner(), &chunk)?,
        None => output.into_inner(),
    };
    Ok(PreparedImageUpload {
        bytes: output,
        resized_from,
        dimensions: fitted,
    })
}

pub(crate) fn prepare_image_upload(
    bytes: Vec<u8>,
    mime: &str,
) -> Result<PreparedImageUpload, String> {
    prepare_image_upload_with_limit(bytes, mime, MAX_IMAGE_PIXELS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scientific_figure_dimensions_use_the_full_pixel_budget() {
        let fitted = fit_dimensions_to_pixel_limit(6_000, 5_000, MAX_IMAGE_PIXELS);
        assert!(u64::from(fitted.0) * u64::from(fitted.1) <= MAX_IMAGE_PIXELS);
        assert!(
            fitted.0 > 5_000,
            "wide figures should not use a 5000px edge cap"
        );
        assert!((f64::from(fitted.0) / f64::from(fitted.1) - 1.2).abs() < 0.001);
    }

    #[test]
    fn static_png_is_scrubbed_and_resized_without_changing_aspect_ratio() {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            100,
            80,
            image::Rgba([10, 20, 30, 255]),
        ));
        let mut source = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut source, image::ImageFormat::Png)
            .unwrap();

        let prepared =
            prepare_image_upload_with_limit(source.into_inner(), "image/png", 5_000).unwrap();
        assert_eq!(prepared.resized_from, Some((100, 80)));
        assert!(u64::from(prepared.dimensions.0) * u64::from(prepared.dimensions.1) <= 5_000);
        assert!(
            (f64::from(prepared.dimensions.0) / f64::from(prepared.dimensions.1) - 1.25).abs()
                < 0.01
        );
        assert_eq!(
            image::load_from_memory_with_format(&prepared.bytes, image::ImageFormat::Png)
                .unwrap()
                .dimensions(),
            prepared.dimensions
        );
    }

    #[test]
    fn static_jpeg_metadata_is_removed_before_upload() {
        let mut source = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut source, image::ImageFormat::Jpeg)
            .unwrap();
        let mut source = source.into_inner();
        source.splice(2..2, [0xff, 0xfe, 0x00, 0x06, b'B', b'U', b'Z', b'Z']);
        assert!(source.windows(4).any(|window| window == b"BUZZ"));

        let prepared = prepare_image_upload(source, "image/jpeg").unwrap();

        assert!(!prepared.bytes.windows(4).any(|window| window == b"BUZZ"));
        assert_eq!(
            image::load_from_memory_with_format(&prepared.bytes, image::ImageFormat::Jpeg)
                .unwrap()
                .dimensions(),
            (2, 2)
        );
    }
}
