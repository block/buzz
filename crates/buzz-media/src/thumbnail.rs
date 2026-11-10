//! Synchronous thumbnail generation and blurhash encoding.

use std::io::Cursor;

use image::ImageFormat;

use crate::config::MediaConfig;
use crate::error::MediaError;
use crate::storage::BlobMeta;

/// A stored thumbnail variant: `{sha256}.thumb.{ext}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thumb {
    pub ext: &'static str,
    pub mime: &'static str,
}

/// Lossy WebP keeps alpha, so upload descriptors reference it.
pub const WEBP: Thumb = Thumb {
    ext: "webp",
    mime: "image/webp",
};

/// JPEG drops alpha. It stays for clients that derive `{sha256}.thumb.jpg`.
pub const JPEG: Thumb = Thumb {
    ext: "jpg",
    mime: "image/jpeg",
};

/// Every variant written for an image.
pub const THUMBS: [Thumb; 2] = [WEBP, JPEG];

/// One image's thumbnail bytes, in [`THUMBS`] order.
pub type ThumbBytes = [Vec<u8>; THUMBS.len()];

/// libwebp quality: ~13 KB for a 320px transparent avatar, about JPEG's size.
const WEBP_QUALITY: f32 = 80.0;

impl Thumb {
    /// Storage key and `/media/` path segment for `sha256`'s thumbnail.
    pub fn key(self, sha256: &str) -> String {
        format!("{sha256}.thumb.{}", self.ext)
    }

    /// Split `{sha256}.thumb.{ext}` into its (unvalidated) hash and variant.
    pub fn parse(name: &str) -> Option<(&str, Thumb)> {
        let (sha256, ext) = name.split_once(".thumb.")?;
        THUMBS
            .into_iter()
            .find(|t| t.ext == ext)
            .map(|t| (sha256, t))
    }
}

/// Generate thumbnails and blurhash from image bytes (CPU-bound, sync).
///
/// Returns `(metadata, thumbnails)`, with no thumbnails for non-images.
/// Caller handles S3 writes after `spawn_blocking` returns.
pub fn generate_image_metadata_sync(
    config: &MediaConfig,
    sha256: &str,
    bytes: &[u8],
    mime: &str,
    ext: &str,
) -> Result<(BlobMeta, Option<ThumbBytes>), MediaError> {
    if !mime.starts_with("image/") {
        return Ok((BlobMeta::default(), None));
    }

    let img = image::load_from_memory(bytes)?;
    let (w, h) = (img.width(), img.height());

    // Thumbnail: 320px max dimension, preserve aspect ratio
    let thumb = img.thumbnail(320, 320);
    let rgba = thumb.to_rgba8();
    let webp = webp::Encoder::from_rgba(&rgba, rgba.width(), rgba.height())
        .encode_simple(false, WEBP_QUALITY)
        .map_err(|_| MediaError::Internal)?
        .to_vec();
    let mut jpeg = Vec::new();
    thumb.write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)?;

    // Blurhash from thumbnail (faster than full image)
    let bh =
        blurhash::encode(4, 3, thumb.width(), thumb.height(), rgba.as_raw()).unwrap_or_default();

    Ok((
        BlobMeta {
            dim: format!("{w}x{h}"),
            blurhash: bh,
            thumb_url: format!("{}/{}", config.public_base_url, WEBP.key(sha256)),
            ext: ext.to_string(),
            mime_type: mime.to_string(),
            size: bytes.len() as u64,
            ..BlobMeta::default() // uploaded_at set by caller (process_upload)
        },
        Some([webp, jpeg]),
    ))
}
