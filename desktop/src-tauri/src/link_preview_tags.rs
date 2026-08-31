use nostr::Tag;

pub fn append(
    preview_tags: &[Vec<String>],
    content: &str,
    relay_base: &str,
    tags: &mut Vec<Tag>,
) -> Result<(), String> {
    tags.extend(
        buzz_sdk_pkg::link_preview::parse_link_preview_tags(preview_tags, content, relay_base)
            .map_err(|error| error.to_string())?,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BASE: &str = "https://relay.example";
    const CONTENT: &str = "See https://linear.app/acme/issue/ABC-123/example";

    fn tag(image_url: &str, image_hash: &str) -> Vec<String> {
        [
            "link-preview",
            "snapshot",
            "1",
            "https://linear.app/acme/issue/ABC-123/example",
            "Example",
            "Linear",
            "Description",
            image_url,
            image_hash,
            "",
            "",
        ]
        .map(str::to_string)
        .to_vec()
    }

    #[test]
    fn append_accepts_complete_local_snapshot() {
        let mut tags = Vec::new();
        append(
            &[tag(&format!("{BASE}/media/{HASH}.png"), HASH)],
            CONTENT,
            BASE,
            &mut tags,
        )
        .unwrap();
        assert_eq!(tags.len(), 1);
    }

    #[test]
    fn append_accepts_blanket_suppression_by_itself() {
        let mut tags = Vec::new();
        append(
            &[vec!["link-preview".into(), "none".into()]],
            CONTENT,
            BASE,
            &mut tags,
        )
        .unwrap();
        assert_eq!(tags[0].as_slice(), ["link-preview", "none"]);
        assert!(append(
            &[vec!["link-preview".into(), "none".into()], tag("", ""),],
            CONTENT,
            BASE,
            &mut Vec::new(),
        )
        .is_err());
    }

    #[test]
    fn append_accepts_description_newlines() {
        let mut preview_tag = tag("", "");
        preview_tag[6] = "First paragraph\n\nSecond paragraph".into();
        assert!(append(&[preview_tag], CONTENT, BASE, &mut Vec::new()).is_ok());
    }

    #[test]
    fn append_rejects_other_control_characters() {
        let mut preview_tag = tag("", "");
        preview_tag[6] = "Unsafe\tdescription".into();
        assert!(append(&[preview_tag], CONTENT, BASE, &mut Vec::new()).is_err());
    }

    #[test]
    fn append_rejects_untrusted_or_malformed_snapshot_media() {
        for url in [
            format!("https://evil.example/media/{HASH}.png"),
            format!("{BASE}/media/{HASH}.png?token=leak"),
            format!("{BASE}/media/{HASH}.png#fragment"),
            format!("https://user@relay.example/media/{HASH}.png"),
            format!("{BASE}/media/{HASH}.svg"),
            format!("{BASE}/media/{HASH}.png/extra"),
        ] {
            assert!(
                append(&[tag(&url, HASH)], CONTENT, BASE, &mut Vec::new()).is_err(),
                "{url}"
            );
        }
        assert!(append(
            &[tag(&format!("{BASE}/media/{HASH}.png"), &"b".repeat(64))],
            CONTENT,
            BASE,
            &mut Vec::new(),
        )
        .is_err());
    }
}
