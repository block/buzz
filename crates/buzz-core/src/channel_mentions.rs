//! Channel-wide mention tag parsing shared by relay and desktop validation.

use nostr::Tag;

/// Marker stored in the fourth field of an audience `p` tag.
pub const CHANNEL_MENTION_RECIPIENT_MARKER_PREFIX: &str = "buzz:audience:";
/// Non-notifying tag used to preserve reserved mention rendering metadata.
pub const CHANNEL_MENTION_REFERENCE_TAG: &str = "buzz-audience-ref";
/// Maximum exact recipients that fit comfortably below Buzz's default frame limit.
pub const MAX_CHANNEL_MENTION_RECIPIENTS: usize = 4_000;

/// Return the exact recipient encoded by an audience tag.
///
/// Valid tags are `['p', '<hex-pubkey>', '', 'buzz:audience:everyone|here']`.
/// Keeping the recipient in a standard `p` tag preserves NIP-01 `#p`
/// subscriptions and notification interoperability for clients that ignore
/// Buzz's trailing marker.
pub fn channel_mention_recipient(tag: &Tag) -> Option<&str> {
    let parts = tag.as_slice();
    if parts.len() != 4
        || parts[0] != "p"
        || !parts[2].is_empty()
        || channel_mention_recipient_mode(tag).is_none()
    {
        return None;
    }
    Some(parts[1].as_str())
}

/// Return the audience mode encoded in a recipient `p` tag.
pub fn channel_mention_recipient_mode(tag: &Tag) -> Option<&str> {
    let parts = tag.as_slice();
    if parts.len() != 4 || parts[0] != "p" || !parts[2].is_empty() {
        return None;
    }
    match parts[3].as_str() {
        "buzz:audience:everyone" => Some("everyone"),
        "buzz:audience:here" => Some("here"),
        _ => None,
    }
}

/// Return the reserved mention mode encoded by a rendering reference tag.
pub fn channel_mention_reference_mode(tag: &Tag) -> Option<&str> {
    let parts = tag.as_slice();
    if parts.len() != 2
        || parts[0] != CHANNEL_MENTION_REFERENCE_TAG
        || (parts[1] != "everyone" && parts[1] != "here")
    {
        return None;
    }
    Some(parts[1].as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_recipient_tags() {
        let pubkey = "a".repeat(64);
        let tag = Tag::parse(["p", pubkey.as_str(), "", "buzz:audience:here"]).expect("tag");
        assert_eq!(channel_mention_recipient(&tag), Some(pubkey.as_str()));
        assert_eq!(channel_mention_recipient_mode(&tag), Some("here"));
    }

    #[test]
    fn rejects_unknown_modes_and_shapes() {
        let pubkey = "a".repeat(64);
        let unknown = Tag::parse(["p", pubkey.as_str(), "", "buzz:audience:away"]).expect("tag");
        let missing = Tag::parse(["p", pubkey.as_str()]).expect("tag");
        assert_eq!(channel_mention_recipient(&unknown), None);
        assert_eq!(channel_mention_recipient(&missing), None);
    }
}
