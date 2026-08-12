//! Image metadata sanitization and validation for Buzz uploads.
//!
//! The relay refuses images that carry metadata — EXIF, ICC profiles, PNG text
//! chunks, GIF comment extensions and the like — because those are covert
//! channels and a privacy leak. Every upload client therefore has to strip
//! metadata before hashing, and Blossom's content addressing means the relay
//! cannot do it on the client's behalf: the upload auth event commits to the
//! SHA-256 of the bytes actually sent.
//!
//! That makes the sanitizer and the validator two halves of one contract. They
//! previously lived in two different crates with no shared tests and drifted:
//! the desktop app stripped metadata, the CLI did not, so agents could not
//! upload a macOS screenshot or a matplotlib chart at all. They live together
//! here so that cannot recur.
//!
//! This is a leaf crate on purpose — `image`, `infer` and nothing else. It is
//! linked into the CLI binary and the desktop app, neither of which should be
//! pulling in a web framework or a storage client to get at a pure bytes-in,
//! bytes-out function.

mod animated;
mod error;
mod gif;
mod sanitize;
mod snapshot_png;
pub mod validate;

pub use error::ImageError;
pub use sanitize::{detect_mime, prepare_for_upload, sanitize_image_for_upload};
pub use snapshot_png::{extract_snapshot_text_chunk, inject_snapshot_text_chunk};
pub use validate::{
    validate_image_metadata_free, AGENT_SNAPSHOT_KEYWORD, PNG_SNAPSHOT_KEYWORDS,
    TEAM_SNAPSHOT_KEYWORD,
};

/// Round-trip contract: whatever the sanitizer emits, the validator accepts.
///
/// This is the test that would have caught the original bug. The two halves
/// are only correct relative to each other, so every fixture below is a real
/// metadata channel that some real producer emits — a macOS screenshot, a
/// matplotlib figure, a PIL save with `dpi=`.
#[cfg(test)]
mod roundtrip_tests {
    use super::validate::tests_support::{chunk, png_with_chunks, text_chunk};
    use super::*;

    /// Every fixture: (name, bytes) where bytes are a PNG the relay rejects.
    fn dirty_fixtures() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            // PIL `Image.save(dpi=...)`, ImageMagick — DPI metadata.
            (
                "pHYs (PIL dpi=)",
                png_with_chunks(&[chunk(b"pHYs", &[0, 0, 0x0B, 0x13, 0, 0, 0x0B, 0x13, 1])]),
            ),
            // matplotlib writes BOTH of these by default. This is the fixture
            // that proves the bug is about generated images, not screenshots.
            (
                "tEXt + pHYs (matplotlib)",
                png_with_chunks(&[
                    text_chunk(
                        "Software",
                        "Matplotlib version 3.9.4, https://matplotlib.org/",
                    ),
                    chunk(b"pHYs", &[0, 0, 0x0B, 0x13, 0, 0, 0x0B, 0x13, 1]),
                ]),
            ),
            // macOS screenshots carry all three.
            (
                "iCCP + eXIf + iTXt (macOS screenshot)",
                png_with_chunks(&[
                    chunk(b"iCCP", b"Display P3\0\0x\x9c\x03\x00\x00\x00\x00\x01"),
                    chunk(b"eXIf", b"II*\0\x08\0\0\0\0\0"),
                    chunk(b"iTXt", b"XML:com.adobe.xmp\0\0\0\0\0<x:xmpmeta/>"),
                ]),
            ),
        ]
    }

    #[test]
    fn dirty_fixtures_are_rejected_before_sanitizing() {
        for (name, bytes) in dirty_fixtures() {
            assert_eq!(
                validate_image_metadata_free(&bytes, "image/png"),
                Err(ImageError::MetadataForbidden),
                "{name}: fixture should be rejected before sanitizing, \
                 otherwise the round-trip test below proves nothing",
            );
        }
    }

    #[test]
    fn sanitized_output_always_passes_validation() {
        for (name, bytes) in dirty_fixtures() {
            let sanitized = sanitize_image_for_upload(bytes, "image/png")
                .unwrap_or_else(|e| panic!("{name}: sanitize failed: {e}"));
            assert_eq!(
                validate_image_metadata_free(&sanitized, "image/png"),
                Ok(()),
                "{name}: sanitizer emitted something the relay rejects",
            );
        }
    }

    /// `prepare_for_upload` is what clients actually call: it must fix the
    /// dirty cases and leave clean bytes alone.
    #[test]
    fn prepare_for_upload_fixes_every_dirty_fixture() {
        for (name, bytes) in dirty_fixtures() {
            let prepared =
                prepare_for_upload(bytes).unwrap_or_else(|e| panic!("{name}: prepare failed: {e}"));
            assert_eq!(
                validate_image_metadata_free(&prepared, "image/png"),
                Ok(()),
                "{name}: prepared bytes still rejected",
            );
        }
    }

    /// The snapshot manifest is a product payload, not metadata — it has to
    /// survive the strip, and it has to still validate afterwards.
    #[test]
    fn snapshot_text_chunk_survives_sanitizing() {
        for keyword in [AGENT_SNAPSHOT_KEYWORD, TEAM_SNAPSHOT_KEYWORD] {
            let manifest = "eyJmb3JtYXQiOiJidXp6LWFnZW50LXNuYXBzaG90In0=";
            let source = png_with_chunks(&[
                text_chunk(keyword, manifest),
                // A mundane text chunk alongside it, which must NOT survive.
                text_chunk("Comment", "GPS=37.7,-122.4"),
            ]);

            let sanitized = sanitize_image_for_upload(source, "image/png").unwrap();

            let decoder = png::Decoder::new(std::io::Cursor::new(&sanitized));
            let reader = decoder.read_info().unwrap();
            let texts = &reader.info().uncompressed_latin1_text;
            let snapshot = texts
                .iter()
                .find(|c| c.keyword == keyword)
                .unwrap_or_else(|| panic!("sanitizer lost the {keyword} tEXt chunk"));
            assert_eq!(snapshot.text, manifest);
            assert!(
                !texts.iter().any(|c| c.keyword == "Comment"),
                "sanitizer kept a non-snapshot tEXt chunk",
            );

            assert_eq!(
                validate_image_metadata_free(&sanitized, "image/png"),
                Ok(()),
                "relay would reject a sanitized snapshot PNG",
            );
        }
    }

    /// The keyword allowlist is the thing that used to be triplicated. Pin it
    /// so the validator and the producers cannot disagree again.
    #[test]
    fn snapshot_keywords_match_the_allowlist() {
        assert_eq!(
            PNG_SNAPSHOT_KEYWORDS,
            [
                AGENT_SNAPSHOT_KEYWORD.as_bytes(),
                TEAM_SNAPSHOT_KEYWORD.as_bytes()
            ],
        );
    }
}
