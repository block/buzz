//! Sonar sticker reference validation for message builders.

use nostr::Tag;

/// Validate and append exactly one Sonar message sticker reference. Keeping
/// this as a dedicated lane prevents renderer-controlled arbitrary tag
/// injection through the media/emoji inputs.
pub(crate) fn sticker_tags(
    sticker_tags: &[Vec<String>],
    tags: &mut Vec<Tag>,
) -> Result<(), String> {
    if sticker_tags.len() > 1 {
        return Err("a message may reference at most one sticker".into());
    }
    for sticker in sticker_tags {
        if sticker.len() != 4 || sticker.first().map(String::as_str) != Some("sticker") {
            return Err("sticker tag must contain coordinate, shortcode, and sha256".into());
        }
        let pack = sonar_stickers::PackAddress::parse(&sticker[1])
            .map_err(|_| "invalid sticker pack coordinate".to_string())?;
        let reference =
            sonar_stickers::StickerRef::new(pack, sticker[2].clone(), sticker[3].clone())
                .map_err(|_| "invalid sticker reference".to_string())?;
        if reference.pack.coordinate() != sticker[1]
            || reference.shortcode != sticker[2]
            || reference.plaintext_sha256 != sticker[3]
        {
            return Err("sticker reference must use canonical lowercase hex".to_string());
        }
        let parts: Vec<&str> = sticker.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid sticker tag: {e}"))?);
    }
    Ok(())
}
