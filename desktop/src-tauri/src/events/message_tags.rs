use nostr::{EventId, Tag};

use super::check_pubkey;

const MAX_THREAD_ROOT_EXCERPT_CHARS: usize = 64;
const SENT_FROM_THREAD_TAG: &str = "buzz:sent-from-thread";
const FORWARDED_FROM_TAG: &str = "buzz:forwarded-from";
const AGENT_ADDRESS_MENTION_MARKER: &str = "agent-address";

pub(super) fn mention_reference_tags(
    mentions: &[Vec<String>],
    tags: &mut Vec<Tag>,
) -> Result<(), String> {
    for mention in mentions {
        if mention.first().map(String::as_str) != Some("mention") {
            return Err(format!(
                "mention reference tags must use 'mention' prefix (got {:?})",
                mention.first()
            ));
        }
        let Some(pubkey) = mention.get(1) else {
            return Err("mention reference tag missing pubkey".into());
        };
        if mention.len() > 3
            || (mention.len() == 3
                && mention.get(2).map(String::as_str) != Some(AGENT_ADDRESS_MENTION_MARKER))
        {
            return Err("mention reference tag has invalid display metadata".into());
        }
        check_pubkey(pubkey)?;
        let normalized_pubkey = pubkey.to_ascii_lowercase();
        let mut parts = vec!["mention", normalized_pubkey.as_str()];
        if mention.len() == 3 {
            parts.push(AGENT_ADDRESS_MENTION_MARKER);
        }
        tags.push(
            Tag::parse(parts).map_err(|error| format!("invalid mention reference tag: {error}"))?,
        );
    }
    Ok(())
}

pub(super) fn append_sent_from_thread_tag(
    source_tag: Option<&[String]>,
    tags: &mut Vec<Tag>,
) -> Result<(), String> {
    let Some(source_tag) = source_tag else {
        return Ok(());
    };
    if !matches!(source_tag.len(), 2 | 3)
        || source_tag.first().map(String::as_str) != Some(SENT_FROM_THREAD_TAG)
    {
        return Err("invalid sent-from-thread tag shape".into());
    }

    EventId::from_hex(source_tag[1].trim())
        .map_err(|_| "sent-from-thread tag has invalid root event ID")?;

    if let Some(excerpt) = source_tag.get(2) {
        if excerpt.trim().is_empty()
            || excerpt.chars().count() > MAX_THREAD_ROOT_EXCERPT_CHARS
            || excerpt.chars().any(char::is_control)
        {
            return Err("sent-from-thread tag has invalid root excerpt".into());
        }
    }

    let parts: Vec<&str> = source_tag.iter().map(String::as_str).collect();
    tags.push(Tag::parse(parts).map_err(|e| format!("invalid sent-from-thread tag: {e}"))?);
    Ok(())
}

/// Validate and append the forward provenance tag
/// `["buzz:forwarded-from", <source event id>, <source channel id>,
///   <original author pubkey>, <thread root id>?]`.
///
/// Mirrors `append_sent_from_thread_tag`: the renderer supplies the parts, so
/// every identifier is re-parsed here and re-emitted in canonical lowercase
/// form. A tag whose name is anything else is rejected outright, which is what
/// keeps this slot from smuggling a forged `h`, `e`, or `p` tag onto the
/// signed event.
pub(super) fn append_forwarded_from_tag(
    source_tag: Option<&[String]>,
    tags: &mut Vec<Tag>,
) -> Result<(), String> {
    let Some(source_tag) = source_tag else {
        return Ok(());
    };
    if !matches!(source_tag.len(), 4 | 5)
        || source_tag.first().map(String::as_str) != Some(FORWARDED_FROM_TAG)
    {
        return Err("invalid forwarded-from tag shape".into());
    }

    let source_event_id = EventId::from_hex(source_tag[1].trim())
        .map_err(|_| "forwarded-from tag has invalid source event ID")?;
    let source_channel_id = uuid::Uuid::parse_str(source_tag[2].trim())
        .map_err(|_| "forwarded-from tag has invalid source channel ID")?;
    let author_pubkey = source_tag[3].trim();
    check_pubkey(author_pubkey)
        .map_err(|_| "forwarded-from tag has invalid author pubkey".to_string())?;

    let mut parts = vec![
        FORWARDED_FROM_TAG.to_string(),
        source_event_id.to_hex(),
        source_channel_id.to_string(),
        author_pubkey.to_ascii_lowercase(),
    ];
    if let Some(thread_root) = source_tag.get(4) {
        let thread_root = EventId::from_hex(thread_root.trim())
            .map_err(|_| "forwarded-from tag has invalid thread root event ID")?;
        parts.push(thread_root.to_hex());
    }

    tags.push(Tag::parse(parts).map_err(|e| format!("invalid forwarded-from tag: {e}"))?);
    Ok(())
}

/// Validate and append imeta tags. Rejects any tag whose first element is not "imeta"
/// to prevent injection of arbitrary tags (e.g., forged "h", "e", or "p" tags).
pub(super) fn imeta_tags(media_tags: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), String> {
    for media_tag in media_tags {
        if media_tag.first().map(String::as_str) != Some("imeta") {
            return Err(format!(
                "media tags must use 'imeta' prefix (got {:?})",
                media_tag.first()
            ));
        }
        let parts: Vec<&str> = media_tag.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid imeta tag: {e}"))?);
    }
    Ok(())
}

/// Validate and append NIP-30 custom-emoji tags. Mirrors `imeta_tags`: rejects
/// any tag whose first element is not "emoji" so this path can't be used to
/// smuggle forged "h"/"e"/"p" tags. Each tag is `["emoji", shortcode, url]`.
pub(super) fn emoji_tags(emoji_tags: &[Vec<String>], tags: &mut Vec<Tag>) -> Result<(), String> {
    for emoji_tag in emoji_tags {
        if emoji_tag.first().map(String::as_str) != Some("emoji") {
            return Err(format!(
                "emoji tags must use 'emoji' prefix (got {:?})",
                emoji_tag.first()
            ));
        }
        let parts: Vec<&str> = emoji_tag.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid emoji tag: {e}"))?);
    }
    Ok(())
}

pub(super) fn append_client_tags(
    client_tags: &[Vec<String>],
    tags: &mut Vec<Tag>,
) -> Result<(), String> {
    for client_tag in client_tags {
        if client_tag.first().map(String::as_str) != Some("client") {
            return Err(format!(
                "client tags must use 'client' prefix (got {:?})",
                client_tag.first()
            ));
        }
        if client_tag.len() < 2 {
            return Err("client tag missing marker".into());
        }
        let parts: Vec<&str> = client_tag.iter().map(String::as_str).collect();
        tags.push(Tag::parse(parts).map_err(|e| format!("invalid client tag: {e}"))?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBKEY: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const ROOT_HEX: &str = "d24da132115ca0a46233cf4c2ad8338fbf914250cbcaa9181a6dd59533cb5ac1";
    const SOURCE_HEX: &str = "4a1b2c3d4e5f60718293a4b5c6d7e8f901234567890abcdef1234567890abcde";
    const CHANNEL_UUID: &str = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";

    fn forwarded_from(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_string()).collect()
    }

    #[test]
    fn mention_reference_preserves_agent_address_display_metadata() {
        let mut tags = Vec::new();
        mention_reference_tags(
            &[vec![
                "mention".into(),
                PUBKEY.to_ascii_uppercase(),
                AGENT_ADDRESS_MENTION_MARKER.into(),
            ]],
            &mut tags,
        )
        .unwrap();

        assert_eq!(
            tags[0].as_slice(),
            &["mention", PUBKEY, AGENT_ADDRESS_MENTION_MARKER]
        );
    }

    #[test]
    fn mention_reference_rejects_unknown_display_metadata() {
        let mut tags = Vec::new();
        let result = mention_reference_tags(
            &[vec!["mention".into(), PUBKEY.into(), "unknown".into()]],
            &mut tags,
        );

        assert!(result.is_err());
    }

    #[test]
    fn message_accepts_only_valid_sent_from_thread_provenance() {
        let source_tag = vec![
            SENT_FROM_THREAD_TAG.to_string(),
            ROOT_HEX.to_string(),
            "Root message excerpt".to_string(),
        ];
        let mut tags = Vec::new();
        append_sent_from_thread_tag(Some(&source_tag), &mut tags).unwrap();
        assert_eq!(tags[0].as_slice(), source_tag);

        let forged_channel_tag = vec!["h".to_string(), "channel-id".to_string()];
        assert!(append_sent_from_thread_tag(Some(&forged_channel_tag), &mut Vec::new()).is_err());

        let invalid_root_tag = vec![
            SENT_FROM_THREAD_TAG.to_string(),
            "not-an-event-id".to_string(),
        ];
        assert!(append_sent_from_thread_tag(Some(&invalid_root_tag), &mut Vec::new()).is_err());
    }

    #[test]
    fn forwarded_from_normalizes_a_well_formed_provenance_tag() {
        let mut tags = Vec::new();
        append_forwarded_from_tag(
            Some(&forwarded_from(&[
                FORWARDED_FROM_TAG,
                &SOURCE_HEX.to_ascii_uppercase(),
                &format!("  {CHANNEL_UUID}  "),
                &PUBKEY.to_ascii_uppercase(),
                ROOT_HEX,
            ])),
            &mut tags,
        )
        .unwrap();

        assert_eq!(
            tags[0].as_slice(),
            &[
                FORWARDED_FROM_TAG,
                SOURCE_HEX,
                CHANNEL_UUID,
                PUBKEY,
                ROOT_HEX
            ]
        );

        // The thread root is optional: a forwarded top-level message omits it.
        let mut tags = Vec::new();
        append_forwarded_from_tag(
            Some(&forwarded_from(&[
                FORWARDED_FROM_TAG,
                SOURCE_HEX,
                CHANNEL_UUID,
                PUBKEY,
            ])),
            &mut tags,
        )
        .unwrap();
        assert_eq!(
            tags[0].as_slice(),
            &[FORWARDED_FROM_TAG, SOURCE_HEX, CHANNEL_UUID, PUBKEY]
        );

        // Absent provenance stays absent.
        let mut tags = Vec::new();
        append_forwarded_from_tag(None, &mut tags).unwrap();
        assert!(tags.is_empty());
    }

    #[test]
    fn forwarded_from_rejects_forged_or_malformed_provenance() {
        let cases: Vec<(&str, Vec<String>)> = vec![
            (
                "forged channel tag",
                forwarded_from(&["h", CHANNEL_UUID, PUBKEY, SOURCE_HEX]),
            ),
            (
                "forged mention tag",
                forwarded_from(&["p", PUBKEY, CHANNEL_UUID, SOURCE_HEX]),
            ),
            (
                "missing author",
                forwarded_from(&[FORWARDED_FROM_TAG, SOURCE_HEX, CHANNEL_UUID]),
            ),
            (
                "too many parts",
                forwarded_from(&[
                    FORWARDED_FROM_TAG,
                    SOURCE_HEX,
                    CHANNEL_UUID,
                    PUBKEY,
                    ROOT_HEX,
                    "extra",
                ]),
            ),
            (
                "bad source event id",
                forwarded_from(&[FORWARDED_FROM_TAG, "not-an-event-id", CHANNEL_UUID, PUBKEY]),
            ),
            (
                "bad channel id",
                forwarded_from(&[FORWARDED_FROM_TAG, SOURCE_HEX, "not-a-uuid", PUBKEY]),
            ),
            (
                "bad author pubkey",
                forwarded_from(&[FORWARDED_FROM_TAG, SOURCE_HEX, CHANNEL_UUID, "nope"]),
            ),
            (
                "bad thread root",
                forwarded_from(&[
                    FORWARDED_FROM_TAG,
                    SOURCE_HEX,
                    CHANNEL_UUID,
                    PUBKEY,
                    "not-an-event-id",
                ]),
            ),
        ];

        for (label, tag) in cases {
            let mut tags = Vec::new();
            assert!(
                append_forwarded_from_tag(Some(&tag), &mut tags).is_err(),
                "expected {label} to be rejected"
            );
            assert!(tags.is_empty(), "{label} must not append a tag");
        }
    }
}
