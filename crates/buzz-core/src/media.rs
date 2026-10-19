//! Which media blobs an event shares, for channel-scoped media access.

use std::collections::BTreeSet;

use crate::kind::{
    event_is_shared, is_author_only_event_kind, is_shared_gated_kind, KIND_EMOJI_LIST,
    KIND_EMOJI_SET, KIND_PROFILE, P_GATED_KINDS,
};

/// Hashes of every blob `event` shares into its channel, or with the whole
/// community when it is channel-less: imeta `x` and `image-x`, link-preview
/// image and favicon, and any standalone 64-hex token in the content of a
/// profile or shared persona or team, or in an emoji set or list's tags.
///
/// Empty for an event only its author or tagged recipients can read, since
/// referencing a blob there shows it to no one new.
pub fn shared_blob_hashes(event: &nostr::Event) -> Vec<String> {
    let kind = u32::from(event.kind.as_u16());
    if is_author_only_event_kind(event)
        || P_GATED_KINDS.contains(&kind)
        || (is_shared_gated_kind(kind) && !event_is_shared(event))
    {
        return Vec::new();
    }
    let emoji = kind == KIND_EMOJI_SET || kind == KIND_EMOJI_LIST;
    let mut hashes = BTreeSet::new();
    if kind == KIND_PROFILE || is_shared_gated_kind(kind) {
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
    use crate::kind::{KIND_EVENT_REMINDER, KIND_PERSONA};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn media_url(sha256: &str) -> String {
        format!("https://relay.example/media/{sha256}.jpg")
    }

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
            shared_blob_hashes(&event),
            vec![video, poster, preview_image]
        );
    }

    #[test]
    fn covers_avatars_emoji_and_shared_personas_only() {
        let (avatar, emoji) = ("c".repeat(64), "d".repeat(64));
        let keys = Keys::generate();
        let content = format!(r#"{{"picture":"{}"}}"#, media_url(&avatar));
        let profile = EventBuilder::new(Kind::Metadata, &content)
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(shared_blob_hashes(&profile), vec![avatar.clone()]);

        let emoji_set = EventBuilder::new(Kind::from(30030), "")
            .tags([Tag::parse(["emoji", "party", &media_url(&emoji)]).unwrap()])
            .sign_with_keys(&keys)
            .unwrap();
        assert_eq!(shared_blob_hashes(&emoji_set), vec![emoji]);

        let persona = |shared: bool| {
            let mut tags = vec![Tag::parse(["d", "helper"]).unwrap()];
            if shared {
                tags.push(Tag::parse(["shared", "true"]).unwrap());
            }
            EventBuilder::new(Kind::from(KIND_PERSONA as u16), &content)
                .tags(tags)
                .sign_with_keys(&keys)
                .unwrap()
        };
        assert_eq!(shared_blob_hashes(&persona(true)), vec![avatar]);
        assert!(shared_blob_hashes(&persona(false)).is_empty());
    }

    #[test]
    fn author_only_events_share_nothing() {
        let sha = "a".repeat(64);
        let reminder = EventBuilder::new(Kind::from(KIND_EVENT_REMINDER as u16), "")
            .tags([Tag::parse(["imeta", &format!("x {sha}")]).unwrap()])
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert!(shared_blob_hashes(&reminder).is_empty());
    }
}
