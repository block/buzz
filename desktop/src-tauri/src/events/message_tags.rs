use nostr::{EventId, Tag};

use super::{tag, MAX_MENTIONS};

/// Validate a hex pubkey is exactly 64 hex characters.
pub(super) fn check_pubkey(pubkey: &str) -> Result<(), String> {
    if pubkey.len() != 64 || !pubkey.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "pubkey must be a 64-character hex string (got {} chars)",
            pubkey.len()
        ));
    }
    Ok(())
}

const MAX_THREAD_ROOT_EXCERPT_CHARS: usize = 64;
const SENT_FROM_THREAD_TAG: &str = "buzz:sent-from-thread";
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
}

pub(super) fn mention_tags(mentions: &[&str]) -> Result<Vec<Tag>, String> {
    if mentions.len() > MAX_MENTIONS {
        return Err(format!("too many mentions (max {MAX_MENTIONS})"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut tags = Vec::new();
    for &hex in mentions {
        check_pubkey(hex)?;
        let lower = hex.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            tags.push(tag(vec!["p", &lower])?);
        }
    }
    Ok(tags)
}

/// Marker on a `p` tag naming the author this reply answers.
///
/// Mirrors `P_TAG_ADDRESSING_MARKER` in
/// `desktop/src/features/messages/lib/threading.ts`.
pub(crate) const P_TAG_ADDRESSING_MARKER: &str = "reply";

/// Marker on a `p` tag naming someone the author typed as `@name`.
///
/// Mirrors `P_TAG_MENTION_MARKER` in the same TypeScript module. Distinct from
/// the `["mention", pk]` reference tag built by `mention_reference_tags`, which
/// is a different tag kind meaning "render the chip, do not notify".
pub(crate) const P_TAG_MENTION_MARKER: &str = "mention";

/// Who an outgoing message `p`-tags, split by *why* it tags them.
///
/// The two lists are both `&[&str]` of pubkeys and mean different things on the
/// wire, so they are named rather than positional: folding one into the other is
/// exactly the mistake the role markers exist to prevent.
#[derive(Default, Clone, Copy)]
pub(crate) struct Recipients<'a> {
    /// Written as `@name` in the body. Marked `mention` on a reply.
    pub typed: &'a [&'a str],
    /// Addressed by the *channel* rather than by the message — every other
    /// participant in a DM, who is tagged whether or not anyone typed their
    /// name. Never marked, because neither role is true of it.
    pub addressed: &'a [&'a str],
}

/// Bare `p` tags for a top-level message.
///
/// Nothing to disambiguate without a parent, so a typed mention and a channel
/// recipient look the same here — which is what they both looked like before
/// markers existed.
pub(super) fn top_level_recipient_tags(recipients: Recipients<'_>) -> Result<Vec<Tag>, String> {
    let mut all: Vec<&str> =
        Vec::with_capacity(recipients.typed.len() + recipients.addressed.len());
    all.extend_from_slice(recipients.typed);
    all.extend_from_slice(recipients.addressed);
    mention_tags(&all)
}

/// `p` tags for a reply, each marked with the role it plays.
///
/// NIP-10 addressing and a typed `@mention` are byte-identical as bare `p`
/// tags, which forces every receiver to fetch the parent and check who wrote it
/// just to tell them apart. The markers record what the sender already knows.
///
/// Three roles, three shapes:
///
/// - `recipients.typed` → `mention`.
/// - `parent_author` → `reply`.
/// - `recipients.addressed` → left **bare**. Neither marker is true of a DM
///   counterpart who did not write the parent and was never typed, and under the
///   one-way read a bare tag means "ask the parent" — the same answer these tags
///   got before markers existed. Claiming `mention` here would let a DM thread
///   reply pierce a mute and outrank a real `@you` in the mention feed.
///
/// A pubkey that is both typed and the parent's author is emitted once, marked
/// as a mention — that is the case no amount of parent-fetching can decide, and
/// mention is the answer that preserves the stronger signal. Relay tag filters
/// match only the second element, so no marker affects `#p` delivery.
pub(super) fn reply_mention_tags(
    recipients: Recipients<'_>,
    parent_author: Option<&str>,
) -> Result<Vec<Tag>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut lowered = Vec::new();
    for &hex in recipients.typed {
        check_pubkey(hex)?;
        let lower = hex.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            lowered.push(lower);
        }
    }

    let addressing = match parent_author {
        Some(hex) => {
            check_pubkey(hex)?;
            let lower = hex.to_ascii_lowercase();
            // Already typed in the body, so it is a mention and not merely
            // addressing. Emitting both would double-tag the same pubkey.
            if seen.contains(&lower) {
                None
            } else {
                seen.insert(lower.clone());
                Some(lower)
            }
        }
        None => None,
    };

    // A channel recipient who is also the parent's author is already covered by
    // the addressing tag, and one who was typed is already a mention.
    let mut bare = Vec::new();
    for &hex in recipients.addressed {
        check_pubkey(hex)?;
        let lower = hex.to_ascii_lowercase();
        if seen.insert(lower.clone()) {
            bare.push(lower);
        }
    }

    // The addressing tag is the one that must survive the cap: without it an
    // agent's `require_mention` subscription never receives the reply at all,
    // while a dropped body mention only costs one notification.
    let room = MAX_MENTIONS - usize::from(addressing.is_some());
    if lowered.len() + bare.len() > room {
        return Err(format!("too many recipients (max {room} on a reply)"));
    }

    let mut tags = Vec::new();
    for lower in &lowered {
        tags.push(tag(vec!["p", lower, "", P_TAG_MENTION_MARKER])?);
    }
    for lower in &bare {
        tags.push(tag(vec!["p", lower])?);
    }
    if let Some(lower) = &addressing {
        tags.push(tag(vec!["p", lower, "", P_TAG_ADDRESSING_MARKER])?);
    }
    Ok(tags)
}

#[cfg(test)]
mod reply_tag_tests {
    use super::*;

    const PARENT_AUTHOR: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const TYPED: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

    const COUNTERPART: &str = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9";

    fn tag_rows(tags: &[Tag]) -> Vec<Vec<String>> {
        tags.iter().map(|t| t.as_slice().to_vec()).collect()
    }

    fn typed<'a>(pubkeys: &'a [&'a str]) -> Recipients<'a> {
        Recipients {
            typed: pubkeys,
            addressed: &[],
        }
    }

    fn row(pubkey: &str, marker: Option<&str>) -> Vec<String> {
        match marker {
            Some(marker) => vec![
                "p".to_string(),
                pubkey.to_string(),
                String::new(),
                marker.to_string(),
            ],
            None => vec!["p".to_string(), pubkey.to_string()],
        }
    }

    #[test]
    fn reply_marks_each_p_tag_with_the_role_it_plays() {
        let tags = reply_mention_tags(typed(&[TYPED]), Some(PARENT_AUTHOR)).unwrap();
        assert_eq!(
            tag_rows(&tags),
            vec![
                vec![
                    "p".to_string(),
                    TYPED.to_string(),
                    String::new(),
                    P_TAG_MENTION_MARKER.to_string()
                ],
                vec![
                    "p".to_string(),
                    PARENT_AUTHOR.to_string(),
                    String::new(),
                    P_TAG_ADDRESSING_MARKER.to_string()
                ],
            ]
        );
    }

    #[test]
    fn a_typed_parent_author_is_tagged_once_as_a_mention() {
        // The case no amount of parent-fetching can decide: the recipient is
        // both the author being answered and someone typed in the body. Mention
        // is the answer that keeps the stronger signal, and emitting an
        // addressing tag as well would double-tag the same pubkey.
        let tags = reply_mention_tags(typed(&[PARENT_AUTHOR]), Some(PARENT_AUTHOR)).unwrap();
        assert_eq!(
            tag_rows(&tags),
            vec![vec![
                "p".to_string(),
                PARENT_AUTHOR.to_string(),
                String::new(),
                P_TAG_MENTION_MARKER.to_string()
            ]]
        );
    }

    #[test]
    fn the_addressing_tag_keeps_its_slot_under_the_cap() {
        // Past the cap the relay rejects the whole event rather than trimming.
        // Losing the addressing tag costs an agent the reply entirely, so the
        // body list is what has to give.
        // Distinct, or dedup would collapse them and the cap would never trip.
        let full: Vec<String> = (1..=MAX_MENTIONS).map(|i| format!("{i:064x}")).collect();
        let refs: Vec<&str> = full.iter().map(String::as_str).collect();
        assert!(reply_mention_tags(typed(&refs), Some(PARENT_AUTHOR)).is_err());
        assert!(reply_mention_tags(typed(&refs[..MAX_MENTIONS - 1]), Some(PARENT_AUTHOR)).is_ok());
    }

    #[test]
    fn a_top_level_message_leaves_its_p_tags_bare() {
        // Nothing to disambiguate without a parent, so no marker is added and
        // older readers see exactly what they saw before.
        let tags = mention_tags(&[TYPED]).unwrap();
        assert_eq!(
            tag_rows(&tags),
            vec![vec!["p".to_string(), TYPED.to_string()]]
        );
    }

    #[test]
    fn a_channel_recipient_is_tagged_bare_not_as_a_mention() {
        // A DM tags every other participant whether or not anyone typed their
        // name. Marking that `mention` would be a lie the receivers act on: it
        // pierces a mute and takes a slot in the mention feed ahead of a real
        // `@you`. Bare means "ask the parent", which is the honest answer.
        let tags = reply_mention_tags(
            Recipients {
                typed: &[],
                addressed: &[COUNTERPART],
            },
            None,
        )
        .unwrap();
        assert_eq!(tag_rows(&tags), vec![row(COUNTERPART, None)]);
    }

    #[test]
    fn a_channel_recipient_who_wrote_the_parent_is_addressing() {
        // The usual DM reply: the counterpart is both the channel's other
        // participant and the author being answered. One tag, marked `reply`.
        let tags = reply_mention_tags(
            Recipients {
                typed: &[],
                addressed: &[COUNTERPART],
            },
            Some(COUNTERPART),
        )
        .unwrap();
        assert_eq!(
            tag_rows(&tags),
            vec![row(COUNTERPART, Some(P_TAG_ADDRESSING_MARKER))]
        );
    }

    #[test]
    fn a_channel_recipient_who_was_also_typed_is_a_mention() {
        let tags = reply_mention_tags(
            Recipients {
                typed: &[COUNTERPART],
                addressed: &[COUNTERPART],
            },
            None,
        )
        .unwrap();
        assert_eq!(
            tag_rows(&tags),
            vec![row(COUNTERPART, Some(P_TAG_MENTION_MARKER))]
        );
    }

    #[test]
    fn channel_recipients_count_against_the_cap_too() {
        let full: Vec<String> = (1..MAX_MENTIONS).map(|i| format!("{i:064x}")).collect();
        let refs: Vec<&str> = full.iter().map(String::as_str).collect();
        // MAX_MENTIONS - 1 typed + 1 bare + 1 addressing = one over.
        assert!(reply_mention_tags(
            Recipients {
                typed: &refs,
                addressed: &[COUNTERPART],
            },
            Some(PARENT_AUTHOR)
        )
        .is_err());
        assert!(reply_mention_tags(
            Recipients {
                typed: &refs,
                addressed: &[COUNTERPART],
            },
            None
        )
        .is_ok());
    }

    #[test]
    fn a_top_level_message_leaves_every_recipient_bare() {
        let tags = top_level_recipient_tags(Recipients {
            typed: &[TYPED],
            addressed: &[COUNTERPART],
        })
        .unwrap();
        assert_eq!(
            tag_rows(&tags),
            vec![row(TYPED, None), row(COUNTERPART, None)]
        );
    }
}
