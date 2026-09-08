//! Structural GIF metadata stripping for the upload sanitizer.
//!
//! Split out of `media.rs` to keep that file under the desktop line-size
//! limit. The relay rejects media carrying metadata (`MetadataForbidden` in
//! buzz-media's `validate_gif_metadata_free`); these helpers drop the GIF
//! metadata channels it forbids without re-encoding, so animation timing,
//! disposal, and pixel data survive byte-identical.

/// Walk length-prefixed GIF data sub-blocks starting at `i`; return the index
/// just past the block terminator.
fn gif_sub_blocks_end(body: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *body.get(i)? as usize;
        i += 1;
        if len == 0 {
            return Some(i);
        }
        i = i.checked_add(len).filter(|&end| end <= body.len())?;
    }
}

/// Strip metadata channels from a GIF without re-encoding.
///
/// GIF carries three unrestricted metadata channels — comment extensions
/// (0xFE), plain-text extensions (0x01), and application extensions (0xFF)
/// other than the standard NETSCAPE2.0/ANIMEXTS1.0 looping ones. The relay
/// rejects all of them (`MetadataForbidden` in buzz-media's
/// `validate_gif_metadata_free`), and encoders like Photoshop and Giphy emit
/// them routinely, so uploads fail unless the client drops them first.
///
/// Everything the relay accepts — header, colour tables, graphic-control
/// extensions, image descriptors and frame data, standard looping extensions —
/// is copied verbatim, so animation timing, disposal, and pixel data stay
/// byte-identical. Trailing bytes after the trailer (another relay reject) are
/// truncated. Returns `None` when the payload isn't structurally parseable as
/// GIF; the caller then uploads the original bytes and the relay's validator
/// remains the authority.
pub(crate) fn strip_gif_metadata(body: &[u8]) -> Option<Vec<u8>> {
    if !(body.starts_with(b"GIF87a") || body.starts_with(b"GIF89a")) || body.len() < 13 {
        return None;
    }

    let packed = body[10];
    let mut i = 13usize;
    if packed & 0x80 != 0 {
        let table_len = 3usize << ((packed & 0x07) as usize + 1);
        i = i.checked_add(table_len).filter(|&end| end <= body.len())?;
    }

    let mut out = Vec::with_capacity(body.len());
    out.extend_from_slice(&body[..i]);

    loop {
        match *body.get(i)? {
            // Image descriptor: optional local colour table, LZW minimum code
            // size, then image-data sub-blocks. Copied verbatim.
            0x2c => {
                if i + 10 > body.len() {
                    return None;
                }
                let image_packed = body[i + 9];
                let mut end = i + 10;
                if image_packed & 0x80 != 0 {
                    let table_len = 3usize << ((image_packed & 0x07) as usize + 1);
                    end = end.checked_add(table_len).filter(|&e| e <= body.len())?;
                }
                end = end.checked_add(1).filter(|&e| e <= body.len())?;
                end = gif_sub_blocks_end(body, end)?;
                out.extend_from_slice(&body[i..end]);
                i = end;
            }
            0x21 => {
                let label = *body.get(i + 1)?;
                let start = i;
                i += 2;
                match label {
                    // Graphic Control Extension: fixed-shape rendering state
                    // (delay, disposal, transparency). Kept verbatim.
                    0xf9 => {
                        if body.get(i) != Some(&4) || i + 6 > body.len() || body[i + 5] != 0 {
                            return None;
                        }
                        i += 6;
                        out.extend_from_slice(&body[start..i]);
                    }
                    // Application extension: keep only the standard looping
                    // extensions; anything else (XMP, Photoshop, Giphy…) is a
                    // metadata channel and is dropped.
                    0xff => {
                        if body.get(i) != Some(&11) || i + 12 > body.len() {
                            return None;
                        }
                        let app = &body[i + 1..i + 12];
                        let keep = app == b"NETSCAPE2.0" || app == b"ANIMEXTS1.0";
                        let data_start = i + 12;
                        i = gif_sub_blocks_end(body, data_start)?;
                        if keep {
                            if body.get(data_start) != Some(&3)
                                || body.get(data_start + 1) != Some(&1)
                                || data_start.checked_add(5)? > body.len()
                            {
                                return None;
                            }
                            out.extend_from_slice(&body[start..data_start + 4]);
                            out.push(0);
                        }
                    }
                    // Comment (0xFE), plain-text (0x01), and unknown
                    // extensions: pure metadata channels, dropped. Their
                    // bodies are all length-prefixed sub-block sequences
                    // (plain-text's 12-byte header is itself a sub-block).
                    _ => {
                        i = gif_sub_blocks_end(body, i)?;
                    }
                }
            }
            // Trailer: emit and stop, truncating any trailing bytes.
            0x3b => {
                out.push(0x3b);
                return Some(out);
            }
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::media::sanitize_image_for_upload;

    /// Minimal single-frame GIF89a: header, 2×2 logical screen, 2-entry global
    /// colour table, NETSCAPE looping extension, graphic control extension,
    /// one image descriptor with LZW data, trailer. Structurally canonical —
    /// the relay's validator accepts exactly this shape.
    fn minimal_gif() -> Vec<u8> {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[
            0x02, 0x00, 0x02, 0x00, // logical screen 2×2
            0x80, 0x00, 0x00, // GCT flag, 2 entries
            0x00, 0x00, 0x00, 0xff, 0xff, 0xff, // colour table
        ]);
        // NETSCAPE2.0 looping application extension.
        gif.extend_from_slice(&[0x21, 0xff, 11]);
        gif.extend_from_slice(b"NETSCAPE2.0");
        gif.extend_from_slice(&[3, 0x01, 0x00, 0x00, 0x00]);
        // Graphic control extension.
        gif.extend_from_slice(&[0x21, 0xf9, 4, 0x00, 0x0a, 0x00, 0x00, 0x00]);
        // Image descriptor + 2-bit LZW data.
        gif.extend_from_slice(&[0x2c, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x02, 0x00, 0x00]);
        gif.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
        gif.push(0x3b);
        gif
    }

    /// Comment extension: `0x21 0xFE`, one data sub-block, terminator.
    fn gif_comment_ext() -> Vec<u8> {
        let mut ext = vec![0x21, 0xfe, 5];
        ext.extend_from_slice(b"hello");
        ext.push(0);
        ext
    }

    /// XMP-style application extension (what Photoshop/Giphy emit).
    fn gif_xmp_ext() -> Vec<u8> {
        let mut ext = vec![0x21, 0xff, 11];
        ext.extend_from_slice(b"XMP DataXMP");
        ext.extend_from_slice(&[4]);
        ext.extend_from_slice(b"<x/>");
        ext.push(0);
        ext
    }

    #[test]
    fn test_strip_gif_metadata_removes_comment_and_foreign_app_extensions() {
        let clean = minimal_gif();
        // Splice metadata extensions after the global colour table (offset 19).
        let mut dirty = clean[..19].to_vec();
        dirty.extend_from_slice(&gif_comment_ext());
        dirty.extend_from_slice(&gif_xmp_ext());
        dirty.extend_from_slice(&clean[19..]);

        let stripped = strip_gif_metadata(&dirty).unwrap();
        assert_eq!(stripped, clean);
    }

    #[test]
    fn test_strip_gif_metadata_preserves_clean_gif_byte_identical() {
        let clean = minimal_gif();
        assert_eq!(strip_gif_metadata(&clean).unwrap(), clean);
    }

    #[test]
    fn test_strip_gif_metadata_canonicalizes_loop_extension() {
        let clean = minimal_gif();
        let mut dirty = clean[..37].to_vec();
        dirty.extend_from_slice(&[8]);
        dirty.extend_from_slice(b"location");
        dirty.push(0);
        dirty.extend_from_slice(&clean[38..]);
        assert_eq!(strip_gif_metadata(&dirty).unwrap(), clean);
    }

    #[test]
    fn test_strip_gif_metadata_truncates_bytes_after_trailer() {
        let mut padded = minimal_gif();
        padded.extend_from_slice(b"junk after trailer");
        assert_eq!(strip_gif_metadata(&padded).unwrap(), minimal_gif());
    }

    #[test]
    fn test_strip_gif_metadata_rejects_unparseable_payloads() {
        // Not a GIF at all.
        assert!(strip_gif_metadata(b"GIF89a").is_none());
        assert!(strip_gif_metadata(b"\x89PNG\r\n\x1a\n").is_none());
        // Truncated mid-structure.
        let clean = minimal_gif();
        assert!(strip_gif_metadata(&clean[..clean.len() - 3]).is_none());
    }

    #[test]
    fn test_sanitize_gif_strips_metadata_and_passes_through_unparseable() {
        let clean = minimal_gif();
        let mut dirty = clean[..19].to_vec();
        dirty.extend_from_slice(&gif_comment_ext());
        dirty.extend_from_slice(&clean[19..]);
        assert_eq!(
            sanitize_image_for_upload(dirty, "image/gif").unwrap(),
            clean
        );

        // Unparseable GIF payloads pass through unchanged — the relay's
        // validator stays the authority.
        let junk = b"GIF89a\x00\x01".to_vec();
        assert_eq!(
            sanitize_image_for_upload(junk.clone(), "image/gif").unwrap(),
            junk
        );
    }
}
