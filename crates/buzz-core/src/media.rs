//! Which media blobs an event shares, for channel-scoped media access.

use std::collections::BTreeSet;

use crate::kind::{
    event_is_shared, is_author_only_event_kind, is_shared_gated_kind, P_GATED_KINDS,
};

/// Hashes of every blob `event` shares into its channel, or with the whole
/// community when it is channel-less: each local media path `/media/<sha256>`
/// anywhere in its content or tags (attachments, posters, thumbnails, link
/// previews, avatars, custom emoji, pasted links).
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
    // JSON content may escape `/` as `\/`; scan the path as clients will see it.
    let content = event.content.replace("\\/", "/");
    let mut hashes: BTreeSet<String> = media_path_hashes(&content).collect();
    for tag in event.tags.iter() {
        hashes.extend(
            tag.as_slice()
                .iter()
                .flat_map(|part| media_path_hashes(part)),
        );
    }
    hashes.into_iter().collect()
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

/// The hash in each `/media/<sha256>` path in `text`, where the hash is not
/// followed by another hex digit.
fn media_path_hashes(text: &str) -> impl Iterator<Item = String> + '_ {
    text.match_indices("/media/").filter_map(|(at, path)| {
        let rest = &text[at + path.len()..];
        let hash = rest.get(..64)?;
        let bounded = !rest[64..].starts_with(|c: char| matches!(c, '0'..='9' | 'a'..='f'));
        (is_sha256_hex(hash) && bounded).then(|| hash.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kind::{KIND_EVENT_REMINDER, KIND_PERSONA};
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn media_url(sha256: &str, ext: &str) -> String {
        format!("https://relay.example/media/{sha256}.{ext}")
    }

    fn signed(builder: EventBuilder) -> nostr::Event {
        builder.sign_with_keys(&Keys::generate()).unwrap()
    }

    #[test]
    fn shares_every_local_media_path() {
        let (video, poster, embed) = ("a".repeat(64), "c".repeat(64), "d".repeat(64));
        let event = signed(
            EventBuilder::new(
                Kind::TextNote,
                format!("look ![image]({})", media_url(&embed, "png")),
            )
            .tags([
                Tag::parse([
                    "imeta",
                    &format!("url {}", media_url(&video, "mp4")),
                    &format!("x {video}"),
                    &format!("image /media/{poster}.jpg"),
                ])
                .unwrap(),
                Tag::parse([
                    "r",
                    &format!("https://elsewhere.example/{}", "e".repeat(64)),
                ])
                .unwrap(),
                Tag::parse(["r", &format!("/media/{}0", "f".repeat(64))]).unwrap(),
            ]),
        );
        assert_eq!(shared_blob_hashes(&event), vec![video, poster, embed]);
    }

    #[test]
    fn shares_avatars_emoji_and_shared_personas_only() {
        let (avatar, emoji) = ("c".repeat(64), "d".repeat(64));
        let content = format!(r#"{{"picture":"{}"}}"#, media_url(&avatar, "jpg"));
        let profile = signed(EventBuilder::new(Kind::Metadata, &content));
        assert_eq!(shared_blob_hashes(&profile), vec![avatar.clone()]);
        let escaped = signed(EventBuilder::new(
            Kind::Metadata,
            content.replace('/', "\\/"),
        ));
        assert_eq!(shared_blob_hashes(&escaped), vec![avatar.clone()]);

        let emoji_set = signed(
            EventBuilder::new(Kind::from(30030), "").tags([Tag::parse([
                "emoji",
                "party",
                &media_url(&emoji, "png"),
            ])
            .unwrap()]),
        );
        assert_eq!(shared_blob_hashes(&emoji_set), vec![emoji]);

        let persona = |shared: bool| {
            let mut tags = vec![Tag::parse(["d", "helper"]).unwrap()];
            if shared {
                tags.push(Tag::parse(["shared", "true"]).unwrap());
            }
            signed(EventBuilder::new(Kind::from(KIND_PERSONA as u16), &content).tags(tags))
        };
        assert_eq!(shared_blob_hashes(&persona(true)), vec![avatar]);
        assert!(shared_blob_hashes(&persona(false)).is_empty());
    }

    #[test]
    fn author_only_events_share_nothing() {
        let reminder = signed(
            EventBuilder::new(Kind::from(KIND_EVENT_REMINDER as u16), "").tags([Tag::parse([
                "imeta",
                &format!("url /media/{}.jpg", "a".repeat(64)),
            ])
            .unwrap()]),
        );
        assert!(shared_blob_hashes(&reminder).is_empty());
    }
}
