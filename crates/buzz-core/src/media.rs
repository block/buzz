//! Which media blobs an event references, for channel-scoped media access.

use std::collections::BTreeSet;

use crate::kind::{KIND_EMOJI_LIST, KIND_EMOJI_SET, KIND_PROFILE};

/// Hashes of every blob `event` references, each of which shares the blob into
/// the event's channel (or with the whole community for a channel-less event):
/// imeta `x` and `image-x`, link-preview image and favicon, and any standalone
/// 64-hex token in a profile's content or an emoji set or list's tags.
pub fn referenced_blob_hashes(event: &nostr::Event) -> Vec<String> {
    let kind = u32::from(event.kind.as_u16());
    let emoji = kind == KIND_EMOJI_SET || kind == KIND_EMOJI_LIST;
    let mut hashes = BTreeSet::new();
    if kind == KIND_PROFILE {
        hashes.extend(hex64_tokens(&event.content));
    }
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        if emoji {
            hashes.extend(parts.iter().flat_map(|part| hex64_tokens(part)));
        }
        match parts.first().map(String::as_str) {
            Some("imeta") => hashes.extend(parts.iter().skip(1).filter_map(|part| {
                let (key, value) = part.split_once(' ')?;
                matches!(key, "x" | "image-x").then(|| value.to_owned())
            })),
            Some("link-preview") => {
                hashes.extend([8, 10].iter().filter_map(|&i| parts.get(i).cloned()));
            }
            _ => {}
        }
    }
    hashes
        .into_iter()
        .filter(|hash| is_sha256_hex(hash))
        .collect()
}

/// Whether `value` is a 64-char lowercase hex SHA-256.
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

/// Maximal runs of exactly 64 lowercase hex characters in `text`.
fn hex64_tokens(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !matches!(c, '0'..='9' | 'a'..='f'))
        .filter(|token| token.len() == 64)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    #[test]
    fn reads_only_attachment_positions() {
        let (video, poster, preview_image) = ("a".repeat(64), "c".repeat(64), "d".repeat(64));
        let mut preview = vec!["link-preview".to_owned(); 11];
        preview[4] = "f".repeat(64);
        preview[8] = preview_image.clone();
        preview[10] = String::new();
        let event = EventBuilder::new(Kind::TextNote, "")
            .tags([
                Tag::parse([
                    "imeta",
                    &format!("url /media/{video}.mp4"),
                    &format!("x {video}"),
                    &format!("image-x {poster}"),
                ])
                .unwrap(),
                Tag::parse(preview).unwrap(),
                Tag::parse(["x", &"e".repeat(64)]).unwrap(),
            ])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(
            referenced_blob_hashes(&event),
            vec![video, poster, preview_image]
        );
    }

    #[test]
    fn covers_avatars_and_emoji() {
        let (avatar, emoji) = ("c".repeat(64), "d".repeat(64));
        let keys = Keys::generate();
        let profile = EventBuilder::new(
            Kind::Metadata,
            format!(r#"{{"picture":"https://relay.example/media/{avatar}.jpg"}}"#),
        )
        .sign_with_keys(&keys)
        .unwrap();
        assert_eq!(referenced_blob_hashes(&profile), vec![avatar]);

        let emoji_set = EventBuilder::new(Kind::from(30030), "")
            .tags([Tag::parse([
                "emoji",
                "party",
                &format!("https://relay.example/media/{emoji}.png"),
            ])
            .unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(referenced_blob_hashes(&emoji_set), vec![emoji]);
    }
}
