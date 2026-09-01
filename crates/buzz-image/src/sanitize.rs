//! Metadata stripping for images on their way to the relay.
//!
//! The relay rejects images carrying metadata (see [`crate::validate`]), so
//! every upload client must strip it first. This is not optional politeness:
//! macOS screenshots carry `iCCP`/`eXIf`/`iTXt` and matplotlib writes
//! `tEXt`/`pHYs` by default, so an unsanitized client cannot upload a
//! screenshot or a chart at all.
//!
//! Stripping happens client-side rather than on the relay because Blossom is
//! content-addressed: the upload auth event commits to the SHA-256 of the
//! bytes actually sent, so the server cannot rewrite them.

/// Return true when a PNG/WebP payload declares animation.
///
/// Animated payloads use structural sanitizers so frame timing, looping, and
/// disposal semantics are preserved without flattening the image. The relay's
/// validator remains the final authority for the sanitized container.
fn is_animated_image(body: &[u8], mime: &str) -> bool {
    match mime {
        "image/png" if body.starts_with(b"\x89PNG\r\n\x1a\n") => {
            let mut offset = 8usize;
            while offset.checked_add(12).is_some_and(|end| end <= body.len()) {
                let length = u32::from_be_bytes([
                    body[offset],
                    body[offset + 1],
                    body[offset + 2],
                    body[offset + 3],
                ]) as usize;
                let Some(end) = offset.checked_add(12).and_then(|v| v.checked_add(length)) else {
                    return false;
                };
                if end > body.len() {
                    return false;
                }
                if &body[offset + 4..offset + 8] == b"acTL" {
                    return true;
                }
                offset = end;
            }
            false
        }
        "image/webp"
            if body.len() >= 12 && body.starts_with(b"RIFF") && &body[8..12] == b"WEBP" =>
        {
            let mut offset = 12usize;
            while offset.checked_add(8).is_some_and(|end| end <= body.len()) {
                let chunk = &body[offset..offset + 4];
                if chunk == b"ANIM" || chunk == b"ANMF" {
                    return true;
                }
                let length = u32::from_le_bytes([
                    body[offset + 4],
                    body[offset + 5],
                    body[offset + 6],
                    body[offset + 7],
                ]) as usize;
                let padded = length.checked_add(length & 1);
                let Some(end) = padded.and_then(|v| offset.checked_add(8 + v)) else {
                    return false;
                };
                if end > body.len() {
                    return false;
                }
                offset = end;
            }
            false
        }
        _ => false,
    }
}

/// Why an animated payload will be refused rather than stripped, if it will be.
///
/// These are deliberate refusals, not failures: an animated PNG/WebP whose
/// frames carry EXIF orientation or an ICC profile cannot have that metadata
/// removed without visibly changing what the viewer sees, so we decline and
/// say why. Callers must keep this reason rather than substituting a generic
/// fallback — it is the only place the user is told what is actually wrong.
fn animation_refusal(body: &[u8], mime: &str) -> Option<String> {
    let oriented_format = match mime {
        "image/png" if crate::animated::animated_png_uses_exif_orientation(body) => Some("PNG"),
        "image/webp" if crate::animated::animated_webp_uses_exif_orientation(body) => Some("WebP"),
        _ => None,
    };
    if let Some(format) = oriented_format {
        return Some(format!(
            "animated {format} with EXIF orientation cannot be uploaded without changing its appearance"
        ));
    }
    let color_profile_format = match mime {
        "image/png" if crate::animated::animated_png_uses_icc_profile(body) => Some("PNG"),
        "image/webp" if crate::animated::animated_webp_uses_icc_profile(body) => Some("WebP"),
        _ => None,
    };
    color_profile_format.map(|format| {
        format!(
            "animated {format} with an ICC profile cannot be uploaded without changing its colors"
        )
    })
}

/// Strip metadata from an image so the relay's validator will accept it.
///
/// Still images are decoded and re-encoded, which drops every ancillary chunk
/// as a side effect. Animated PNG/WebP and GIF are stripped structurally
/// instead — re-encoding those would keep only the first frame or destroy
/// animation timing.
pub fn sanitize_image_for_upload(body: Vec<u8>, mime: &str) -> Result<Vec<u8>, String> {
    let format = match mime {
        "image/jpeg" => image::ImageFormat::Jpeg,
        "image/png" => image::ImageFormat::Png,
        "image/webp" => image::ImageFormat::WebP,
        // GIF is never re-encoded (that would destroy animation timing);
        // metadata extensions are stripped structurally instead. Unparseable
        // payloads pass through — the relay's validator is the authority.
        "image/gif" => {
            let stripped = crate::gif::strip_gif_metadata(&body);
            return Ok(stripped.unwrap_or(body));
        }
        _ => return Ok(body),
    };

    if is_animated_image(&body, mime) {
        if let Some(reason) = animation_refusal(&body, mime) {
            return Err(reason);
        }
        let stripped = match mime {
            "image/png" => crate::animated::strip_animated_png_metadata(&body),
            "image/webp" => crate::animated::strip_animated_webp_metadata(&body),
            _ => None,
        };
        return Ok(stripped.unwrap_or(body));
    }

    // Agent/team snapshot PNGs carry their manifest in a tEXt chunk that the
    // re-encode below would destroy. Pull it out first and re-inject it after
    // sanitizing — all other metadata is still stripped, and the relay
    // allowlists exactly this chunk.
    let snapshot_chunk = if format == image::ImageFormat::Png {
        crate::snapshot_png::extract_snapshot_text_chunk(&body)
    } else {
        None
    };

    use image::ImageDecoder;
    let reader = image::ImageReader::with_format(std::io::Cursor::new(&body), format);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "failed to decode image for metadata removal".to_string())?;
    decoder
        .set_limits(image::Limits::default())
        .map_err(|_| "image exceeds safe decoding limits".to_string())?;
    let orientation = decoder
        .orientation()
        .map_err(|_| "failed to read image orientation".to_string())?;
    let mut image = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| "failed to decode image for metadata removal".to_string())?;
    image.apply_orientation(orientation);
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, format)
        .map_err(|_| "failed to encode image without metadata".to_string())?;
    let sanitized = output.into_inner();
    match snapshot_chunk {
        Some(chunk) => crate::snapshot_png::inject_snapshot_text_chunk(sanitized, &chunk),
        None => Ok(sanitized),
    }
}

/// Sniff the MIME type of a payload from its magic bytes.
///
/// Returns `None` when the bytes are not a recognised type; callers treat that
/// as "not an image" and leave the payload untouched.
pub fn detect_mime(body: &[u8]) -> Option<String> {
    infer::get(body).map(|t| t.mime_type().to_string())
}

/// Prepare an upload payload: leave it alone when the relay would already
/// accept it, otherwise strip its metadata.
///
/// Validating first matters for two reasons. Files that are already clean keep
/// their on-disk SHA-256, so the content address a caller computed locally
/// still matches what is uploaded. And nothing is re-encoded that did not need
/// it, so good input never picks up an incidental bit-depth or compression
/// change. Non-images pass through untouched — the relay validates those on a
/// separate path with its own rules.
pub fn prepare_for_upload(body: Vec<u8>) -> Result<Vec<u8>, String> {
    let Some(mime) = detect_mime(&body) else {
        return Ok(body);
    };
    if !mime.starts_with("image/") {
        return Ok(body);
    }
    if crate::validate::validate_image_metadata_free(&body, &mime).is_ok() {
        return Ok(body);
    }
    // A deliberate refusal is a real answer and must reach the user. Stripping
    // these would visibly change the image, and the message names exactly what
    // is wrong — far better than the relay's generic "contains metadata" 422.
    //
    // Only animated payloads can be refused: a *static* PNG carrying an ICC
    // profile is stripped losslessly by the re-encode below, so it must not
    // take this branch.
    if is_animated_image(&body, &mime) {
        if let Some(reason) = animation_refusal(&body, &mime) {
            return Err(reason);
        }
    }
    // Everything else is best-effort: if the payload sniffs as an image but
    // this pipeline cannot re-encode it — a truncated file, or a format variant
    // the decoder does not handle — upload the original bytes and let the relay
    // decide. Failing locally instead would turn "the relay rejects this" into
    // "this client refuses to try", which can only ever reject uploads that
    // used to work. Same principle the GIF path already follows: the relay's
    // validator is the authority.
    let original = body.clone();
    Ok(sanitize_image_for_upload(body, &mime).unwrap_or(original))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG with no ancillary chunks must survive `prepare_for_upload`
    /// byte-for-byte. This is the negative control: clean input keeps its
    /// on-disk hash, so the content address never moves under a caller.
    #[test]
    fn clean_png_passes_through_unchanged() {
        let clean = crate::validate::tests_support::png_with_chunks(&[]);
        let prepared = prepare_for_upload(clean.clone()).unwrap();
        assert_eq!(prepared, clean, "clean PNG was re-encoded");
    }

    /// Non-images are not the image pipeline's business — the relay validates
    /// them on a separate path.
    #[test]
    fn non_image_passes_through_unchanged() {
        let pdf = b"%PDF-1.4\n trailing bytes".to_vec();
        assert_eq!(prepare_for_upload(pdf.clone()).unwrap(), pdf);
    }

    /// A payload that sniffs as an image but cannot be decoded must still be
    /// uploaded, not rejected locally — the relay is the authority on what it
    /// accepts, and refusing here could only break uploads that used to work.
    #[test]
    fn undecodable_image_falls_back_to_original_bytes() {
        // Valid JPEG magic bytes, nothing decodable behind them.
        let fake = vec![
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0x00,
        ];
        assert_eq!(detect_mime(&fake).as_deref(), Some("image/jpeg"));
        assert_eq!(prepare_for_upload(fake.clone()).unwrap(), fake);
    }

    /// Unrecognised bytes must not be mistaken for an image.
    #[test]
    fn unknown_bytes_pass_through_unchanged() {
        let junk = vec![0x00, 0x01, 0x02, 0x03];
        assert_eq!(prepare_for_upload(junk.clone()).unwrap(), junk);
    }

    /// Animated payloads must take the structural path, not the re-encode
    /// path — re-encoding an APNG through `DynamicImage` keeps only frame one.
    #[test]
    fn animated_png_and_webp_are_not_flattened() {
        let mut apng = b"\x89PNG\r\n\x1a\n".to_vec();
        apng.extend_from_slice(&8u32.to_be_bytes());
        apng.extend_from_slice(b"acTL");
        apng.extend_from_slice(&[0; 8]);
        apng.extend_from_slice(&[0; 4]);
        assert!(is_animated_image(&apng, "image/png"));
        assert!(sanitize_image_for_upload(apng, "image/png").is_ok());

        let mut webp = b"RIFF\x0c\0\0\0WEBPANIM".to_vec();
        webp.extend_from_slice(&0u32.to_le_bytes());
        assert!(is_animated_image(&webp, "image/webp"));
        assert!(sanitize_image_for_upload(webp, "image/webp").is_ok());
    }
}
