//! A shared vocabulary for local media references on reads and publications.
use nostr::Event;
use std::collections::BTreeSet;

/// Parse a local media URL, returning its parent hash (also for thumbnails).
/// Only the tenant's canonical base and root-relative paths count.
pub fn local_hash(value: &str, base: &str) -> Option<String> {
    let alternate = if let Some(rest) = base.strip_prefix("https://") {
        format!("http://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        base.to_owned()
    };
    let prefix = format!("{}/", base.trim_end_matches('/'));
    let alternate_prefix = format!("{}/", alternate.trim_end_matches('/'));
    let rest = value
        .strip_prefix("/media/")
        .or_else(|| value.strip_prefix(&prefix))
        .or_else(|| value.strip_prefix(&alternate_prefix))?;
    let mut parts = rest.split('.');
    let hash = parts.next()?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let safe_ext = |s: &str| {
        !s.is_empty()
            && s.len() <= 8
            && s.bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
    };
    match (parts.next(), parts.next(), parts.next()) {
        (None, None, None) => {}
        (Some(ext), None, None) if safe_ext(ext) => {}
        (Some("thumb"), Some("jpg" | "webp"), None) => {}
        _ => return None,
    }
    Some(hash.to_owned())
}

/// Decode the desktop's animated-avatar descriptor into its poster and animation.
/// Each URL still needs its own tenant check. Malformed escapes never become URLs.
pub fn animated_avatar_urls(value: &str) -> Option<(String, String)> {
    let (poster, encoded) = value.split_once("#buzz-anim=")?;
    if encoded.is_empty() {
        return None;
    }
    let bytes = encoded.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit())
        {
            return None;
        }
    }
    let animation = percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .ok()?
        .into_owned();
    for candidate in [poster, animation.as_str()] {
        let parsed = url::Url::parse(candidate).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return None;
        }
    }
    Some((poster.to_owned(), animation))
}

fn avatar_hashes(value: &str, base: &str, out: &mut BTreeSet<String>) {
    if let Some((poster, animation)) = animated_avatar_urls(value) {
        out.extend(local_hash(&poster, base));
        out.extend(local_hash(&animation, base));
    } else {
        out.extend(local_hash(value, base));
    }
}

/// Extract counted references. imeta URL/image fields cover attachments,
/// previews and posters. Profile pictures/banners, group pictures and emoji
/// are the explicitly supported non-message references. Free text grants nothing.
pub fn reference_hashes(event: &Event, base: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let preview_suppressed = event
        .tags
        .iter()
        .any(|tag| matches!(tag.as_slice(),[name,value] if name=="link-preview" && value=="none"));
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        match parts.first().map(String::as_str) {
            Some("imeta") => {
                for part in &parts[1..] {
                    if let Some(("url" | "image", value)) = part.split_once(' ') {
                        out.extend(local_hash(value, base));
                    }
                }
            }
            Some("link-preview")
                if !preview_suppressed
                    && parts.len() == 11
                    && parts[1] == "snapshot"
                    && parts[2] == "1" =>
            {
                for value in [&parts[7], &parts[9]] {
                    out.extend(local_hash(value, base));
                }
            }
            Some("picture") if event.kind.as_u16() == 39000 => {
                if let Some(value) = parts.get(1) {
                    out.extend(local_hash(value, base));
                }
            }
            Some("emoji") if matches!(event.kind.as_u16(), 10030 | 30030 | 30180) => {
                if let Some(value) = parts.get(2) {
                    out.extend(local_hash(value, base));
                }
            }
            _ => {}
        }
    }
    if matches!(event.kind.as_u16(), 0 | 30175) {
        if let Ok(profile) = serde_json::from_str::<serde_json::Value>(&event.content) {
            let fields = if event.kind.as_u16() == 30175 {
                [profile.get("avatar_url"), None]
            } else {
                [
                    profile.get("picture").or_else(|| profile.get("image")),
                    profile.get("banner"),
                ]
            };
            for value in fields.into_iter().flatten() {
                if let Some(value) = value.as_str() {
                    avatar_hashes(value, base, &mut out);
                }
            }
        }
    }
    out
}

/// All local URLs in structured command payloads as well as counted event
/// references must be authorized before dispatch. This also covers metadata
/// commands whose accepted payload later becomes relay-signed state.
pub fn publication_hashes(event: &Event, base: &str) -> BTreeSet<String> {
    fn visit(value: &serde_json::Value, base: &str, out: &mut BTreeSet<String>) {
        match value {
            serde_json::Value::String(value) => {
                avatar_hashes(value, base, out);
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    visit(value, base, out);
                }
            }
            serde_json::Value::Object(values) => {
                for value in values.values() {
                    visit(value, base, out);
                }
            }
            _ => {}
        }
    }
    let mut out = reference_hashes(event, base);
    if let Ok(value) = serde_json::from_str(&event.content) {
        visit(&value, base, &mut out);
    }
    for tag in event.tags.iter() {
        for part in tag.as_slice().iter().skip(1) {
            out.extend(local_hash(part, base));
        }
    }
    out
}

/// First valid edit target in signed tag order, accepting all ASCII hex casing.
pub fn first_edit_target(event: &nostr::Event) -> Option<Vec<u8>> {
    event.tags.iter().find_map(|tag| {
        let parts = tag.as_slice();
        let value = parts.get(1)?;
        if parts.first().map(String::as_str) == Some("e")
            && value.len() == 64
            && value.bytes().all(|c| c.is_ascii_hexdigit())
        {
            hex::decode(value).ok()
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    #[test]
    fn aliases_and_foreign_hosts() {
        let hash = "a".repeat(64);
        for suffix in ["", ".png", ".thumb.jpg", ".thumb.webp"] {
            assert_eq!(
                local_hash(&format!("/media/{hash}{suffix}"), "https://local/media"),
                Some(hash.clone())
            );
        }
        for value in [
            format!("https://foreign/media/{hash}.jpg"),
            format!("/media/{hash}0"),
            format!("/media/{hash}.png?x=1"),
        ] {
            assert_eq!(local_hash(&value, "https://local/media"), None);
        }
    }
    #[test]
    fn animated_profiles_check_each_url_and_reject_malformed_descriptors() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        for (poster, animation, expected) in [
            (
                format!("https://local/media/{a}.png"),
                format!("https://local/media/{b}.png"),
                BTreeSet::from([a.clone(), b.clone()]),
            ),
            (
                format!("https://foreign/media/{a}.png"),
                format!("https://local/media/{b}.png"),
                BTreeSet::from([b]),
            ),
            (
                format!("https://local/media/{a}.png"),
                format!("https://foreign/media/{}.png", "c".repeat(64)),
                BTreeSet::from([a.clone()]),
            ),
        ] {
            let descriptor = format!(
                "{poster}#buzz-anim={}",
                percent_encoding::utf8_percent_encode(
                    &animation,
                    percent_encoding::NON_ALPHANUMERIC
                )
            );
            let event = EventBuilder::new(
                Kind::Metadata,
                serde_json::json!({"picture":descriptor}).to_string(),
            )
            .sign_with_keys(&Keys::generate())
            .unwrap();
            assert_eq!(reference_hashes(&event, "https://local/media"), expected);
            assert_eq!(publication_hashes(&event, "https://local/media"), expected);
        }
        for value in [
            "https://local/a#buzz-anim=%xx",
            "https://local/a#buzz-anim=%ff",
            "https://local/a#buzz-anim=",
            "https://local/a#buzz-anim=javascript:foo",
        ] {
            assert!(animated_avatar_urls(value).is_none(), "{value}");
        }
    }
    #[test]
    fn snapshot_media_counts_only_image_and_favicon_urls() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let c = "c".repeat(64);
        for (image, favicon, expected) in [
            (
                format!("/media/{a}.png"),
                format!("https://local/media/{b}.png"),
                BTreeSet::from([a.clone(), b.clone()]),
            ),
            (
                format!("https://foreign/media/{a}.png"),
                String::new(),
                BTreeSet::new(),
            ),
        ] {
            let event = EventBuilder::new(Kind::Custom(9), "")
                .tags([Tag::parse([
                    "link-preview",
                    "snapshot",
                    "1",
                    &format!("https://local/media/{c}.png"),
                    "title",
                    "site",
                    "description",
                    &image,
                    &a,
                    &favicon,
                    &b,
                ])
                .unwrap()])
                .sign_with_keys(&Keys::generate())
                .unwrap();
            assert_eq!(reference_hashes(&event, "https://local/media"), expected);
        }
    }
    #[test]
    fn profile_image_fallback_matches_materialization() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        for (content, expected) in [
            (
                serde_json::json!({"image":format!("/media/{a}.png")}),
                BTreeSet::from([a.clone()]),
            ),
            (
                serde_json::json!({"picture":format!("/media/{b}.png"),"image":format!("/media/{a}.png")}),
                BTreeSet::from([b]),
            ),
            (
                serde_json::json!({"picture":null,"image":format!("/media/{a}.png")}),
                BTreeSet::new(),
            ),
        ] {
            let event = EventBuilder::new(Kind::Metadata, content.to_string())
                .sign_with_keys(&Keys::generate())
                .unwrap();
            assert_eq!(reference_hashes(&event, "https://local/media"), expected);
        }
    }
    #[test]
    fn profile_and_imeta_share_the_publication_extractor() {
        let hash = "b".repeat(64);
        let event = EventBuilder::new(
            Kind::Metadata,
            format!(r#"{{"picture":"https:\/\/local\/media\/{hash}.jpg"}}"#),
        )
        .tags([Tag::parse(["imeta", &format!("url /media/{hash}.jpg")]).unwrap()])
        .sign_with_keys(&Keys::generate())
        .unwrap();
        assert_eq!(
            reference_hashes(&event, "https://local/media"),
            BTreeSet::from([hash])
        );
        assert_eq!(
            publication_hashes(&event, "https://local/media"),
            reference_hashes(&event, "https://local/media")
        );
    }
}
