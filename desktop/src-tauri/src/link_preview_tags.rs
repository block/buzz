use nostr::Tag;
use std::collections::HashSet;

const MAX_SNAPSHOTS: usize = 8;
const MAX_TITLE: usize = 300;
const MAX_SITE: usize = 100;
const MAX_DESCRIPTION: usize = 1000;

fn valid_text(value: &str, max: usize, allow_newlines: bool) -> bool {
    value.len() <= max
        && !value
            .chars()
            .any(|character| character.is_control() && !(allow_newlines && character == '\n'))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn valid_media_pair(url: &str, hash: &str, relay_base: &url::Url) -> bool {
    if url.is_empty() && hash.is_empty() {
        return true;
    }
    if url.is_empty() || !valid_sha256(hash) {
        return false;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if parsed.origin() != relay_base.origin()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return false;
    }
    let Some(filename) = parsed.path().strip_prefix("/media/") else {
        return false;
    };
    if filename.contains('/') || filename.contains('%') {
        return false;
    }
    let Some((path_hash, ext)) = filename.split_once('.') else {
        return false;
    };
    path_hash == hash && valid_sha256(path_hash) && matches!(ext, "jpg" | "png" | "gif" | "webp")
}

pub fn append(
    preview_tags: &[Vec<String>],
    relay_base: &str,
    tags: &mut Vec<Tag>,
) -> Result<(), String> {
    if preview_tags.len() > MAX_SNAPSHOTS {
        return Err(format!(
            "too many link preview snapshots (max {MAX_SNAPSHOTS})"
        ));
    }
    let base = url::Url::parse(relay_base).map_err(|_| "invalid relay base URL")?;
    let mut seen = HashSet::new();
    for preview_tag in preview_tags {
        if preview_tag.as_slice() == ["link-preview", "none"] {
            if preview_tags.len() != 1 {
                return Err("link-preview suppression cannot include snapshots".into());
            }
            tags.push(
                Tag::parse(["link-preview", "none"])
                    .map_err(|e| format!("invalid link-preview tag: {e}"))?,
            );
            continue;
        }
        let valid = preview_tag.len() == 11
            && preview_tag[0] == "link-preview"
            && preview_tag[1] == "snapshot"
            && preview_tag[2] == "1"
            && url::Url::parse(&preview_tag[3]).is_ok_and(|url| {
                url.scheme() == "https"
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.fragment().is_none()
            })
            && seen.insert(preview_tag[3].clone())
            && valid_text(&preview_tag[4], MAX_TITLE, false)
            && valid_text(&preview_tag[5], MAX_SITE, false)
            && valid_text(&preview_tag[6], MAX_DESCRIPTION, true)
            && valid_media_pair(&preview_tag[7], &preview_tag[8], &base)
            && valid_media_pair(&preview_tag[9], &preview_tag[10], &base);
        if !valid {
            return Err("invalid link-preview snapshot tag".into());
        }
        let parts: Vec<&str> = preview_tag.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid link-preview tag: {e}"))?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BASE: &str = "https://relay.example";

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
            BASE,
            &mut tags,
        )
        .unwrap();
        assert_eq!(tags[0].as_slice(), ["link-preview", "none"]);
        assert!(append(
            &[vec!["link-preview".into(), "none".into()], tag("", ""),],
            BASE,
            &mut Vec::new(),
        )
        .is_err());
    }

    #[test]
    fn append_accepts_description_newlines() {
        let mut preview_tag = tag("", "");
        preview_tag[6] = "First paragraph\n\nSecond paragraph".into();
        assert!(append(&[preview_tag], BASE, &mut Vec::new()).is_ok());
    }

    #[test]
    fn append_rejects_other_control_characters() {
        let mut preview_tag = tag("", "");
        preview_tag[6] = "Unsafe\tdescription".into();
        assert!(append(&[preview_tag], BASE, &mut Vec::new()).is_err());
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
                append(&[tag(&url, HASH)], BASE, &mut Vec::new()).is_err(),
                "{url}"
            );
        }
        assert!(append(
            &[tag(&format!("{BASE}/media/{HASH}.png"), &"b".repeat(64))],
            BASE,
            &mut Vec::new(),
        )
        .is_err());
    }
}
