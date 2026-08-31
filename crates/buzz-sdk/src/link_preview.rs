//! Validation and construction of signed link-preview snapshot tags.
//!
//! This mirrors the relay's wire contract so producers fail before signing.

use std::collections::HashSet;

use nostr::Tag;

use crate::SdkError;

const MAX_SNAPSHOTS: usize = 8;
const MAX_TITLE: usize = 300;
const MAX_SITE: usize = 100;
const MAX_DESCRIPTION: usize = 1000;

fn invalid(message: impl Into<String>) -> SdkError {
    SdkError::InvalidInput(message.into())
}

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
    let Some((path_hash, extension)) = filename.split_once('.') else {
        return false;
    };
    path_hash == hash
        && valid_sha256(path_hash)
        && matches!(extension, "jpg" | "png" | "gif" | "webp")
}

/// Validate exact link-preview wire arrays and convert them to Nostr tags.
///
/// An empty slice means that the producer has no link-preview opinion. Snapshot
/// canonical URLs must occur in `content`, and media URLs must identify local
/// relay image blobs with matching lowercase SHA-256 hashes.
pub fn parse_link_preview_tags(
    preview_tags: &[Vec<String>],
    content: &str,
    media_base_url: &str,
) -> Result<Vec<Tag>, SdkError> {
    if preview_tags.len() > MAX_SNAPSHOTS {
        return Err(invalid(format!(
            "too many link preview snapshots (max {MAX_SNAPSHOTS})"
        )));
    }
    let base =
        url::Url::parse(media_base_url).map_err(|_| invalid("invalid relay media base URL"))?;
    let mut seen = HashSet::new();
    let mut tags = Vec::with_capacity(preview_tags.len());

    for preview_tag in preview_tags {
        if preview_tag.as_slice() == ["link-preview", "none"] {
            if preview_tags.len() != 1 {
                return Err(invalid("link-preview suppression cannot include snapshots"));
            }
            tags.push(
                Tag::parse(["link-preview", "none"])
                    .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
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
            && content.contains(&preview_tag[3])
            && valid_text(&preview_tag[4], MAX_TITLE, false)
            && valid_text(&preview_tag[5], MAX_SITE, false)
            && valid_text(&preview_tag[6], MAX_DESCRIPTION, true)
            && valid_media_pair(&preview_tag[7], &preview_tag[8], &base)
            && valid_media_pair(&preview_tag[9], &preview_tag[10], &base);
        if !valid {
            return Err(invalid("invalid link-preview snapshot tag"));
        }
        let parts: Vec<&str> = preview_tag.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|error| SdkError::InvalidTag(error.to_string()))?);
    }

    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "https://example.com/item";
    const BASE: &str = "https://relay.example";
    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn snapshot() -> Vec<String> {
        [
            "link-preview",
            "snapshot",
            "1",
            URL,
            "Title",
            "Site",
            "Description",
            "",
            "",
            "",
            "",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    #[test]
    fn accepts_snapshot_and_suppression_contracts() {
        let tags = parse_link_preview_tags(&[snapshot()], URL, BASE).unwrap();
        assert_eq!(tags[0].as_slice(), snapshot());
        let tags = parse_link_preview_tags(&[vec!["link-preview".into(), "none".into()]], "", BASE)
            .unwrap();
        assert_eq!(tags[0].as_slice(), ["link-preview", "none"]);
    }

    #[test]
    fn rejects_missing_content_duplicate_and_mixed_suppression() {
        assert!(parse_link_preview_tags(&[snapshot()], "not the URL", BASE).is_err());
        assert!(parse_link_preview_tags(&[snapshot(), snapshot()], URL, BASE).is_err());
        assert!(parse_link_preview_tags(
            &[vec!["link-preview".into(), "none".into()], snapshot()],
            URL,
            BASE,
        )
        .is_err());
    }

    #[test]
    fn rejects_nonlocal_or_mismatched_media() {
        let mut value = snapshot();
        value[7] = format!("{BASE}/media/{HASH}.png");
        value[8] = HASH.into();
        assert!(parse_link_preview_tags(&[value.clone()], URL, BASE).is_ok());

        for invalid_url in [
            format!("https://evil.example/media/{HASH}.png"),
            format!("{BASE}/media/{HASH}.png?token=secret"),
            format!("{BASE}/media/{HASH}.png#fragment"),
            format!("{BASE}/media/{HASH}.svg"),
            format!("{BASE}/media/{HASH}.png/extra"),
        ] {
            let mut malformed = snapshot();
            malformed[7] = invalid_url;
            malformed[8] = HASH.into();
            assert!(parse_link_preview_tags(&[malformed], URL, BASE).is_err());
        }

        value[8] = "b".repeat(64);
        assert!(parse_link_preview_tags(&[value], URL, BASE).is_err());
    }

    #[test]
    fn rejects_over_limit_wrong_shape_and_unsafe_text() {
        assert!(parse_link_preview_tags(&vec![snapshot(); MAX_SNAPSHOTS + 1], URL, BASE).is_err());

        let mut wrong_shape = snapshot();
        wrong_shape.pop();
        assert!(parse_link_preview_tags(&[wrong_shape], URL, BASE).is_err());

        for (index, value) in [
            (4, "x".repeat(MAX_TITLE + 1)),
            (5, "x".repeat(MAX_SITE + 1)),
            (6, "unsafe\tdescription".into()),
        ] {
            let mut malformed = snapshot();
            malformed[index] = value;
            assert!(parse_link_preview_tags(&[malformed], URL, BASE).is_err());
        }
    }

    #[test]
    fn empty_input_is_a_no_op_and_invalid_base_fails_closed() {
        assert!(parse_link_preview_tags(&[], "", BASE).unwrap().is_empty());
        assert!(parse_link_preview_tags(&[snapshot()], URL, "not a URL").is_err());
    }
}
