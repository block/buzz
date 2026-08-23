//! Reading the role marker a sender put on a `p` tag.
//!
//! NIP-10 addressing and a typed `@mention` are byte-identical as bare `p`
//! tags, so a reply marks each one with the role it plays. This module is the
//! single reader for that marker on the Tauri side; the writers live in
//! [`crate::events::message_tags`].
//!
//! Mirrors `pTagRoleFor` in `desktop/src/features/messages/lib/threading.ts`.

/// What the `p` tags naming a pubkey say this event is to them.
///
/// [`PTagRole::Unknown`] must never be collapsed into either answer: a sender
/// that predates these markers emits a bare `p` tag for both roles, so an absent
/// marker means "ask the parent", not "this is a mention".
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum PTagRole {
    Addressing,
    Mention,
    Unknown,
    None,
}

/// The role `pubkey` plays on this event, read from its `p` tags.
///
/// Takes the tags as string slices so both `nostr::Event` (whose tags are
/// `Tag`) and the flat `Vec<Vec<String>>` shape the native commands carry can
/// share one implementation.
pub(crate) fn p_tag_role<'a>(
    tags: impl IntoIterator<Item = &'a [String]>,
    pubkey: &str,
) -> PTagRole {
    let target = pubkey.to_ascii_lowercase();
    let mut saw_addressing = false;
    let mut saw_bare = false;
    for tag in tags {
        if tag.len() < 2 || tag[0] != "p" || !tag[1].eq_ignore_ascii_case(&target) {
            continue;
        }
        match tag.get(3).map(String::as_str) {
            // Mention wins outright: it is the only marker a sender emits when
            // the recipient is both the parent's author and typed in the body.
            Some(crate::events::P_TAG_MENTION_MARKER) => return PTagRole::Mention,
            Some(crate::events::P_TAG_ADDRESSING_MARKER) => saw_addressing = true,
            _ => saw_bare = true,
        }
    }
    if saw_bare {
        return PTagRole::Unknown;
    }
    if saw_addressing {
        PTagRole::Addressing
    } else {
        PTagRole::None
    }
}

/// [`p_tag_role`] for a signed event.
pub(crate) fn p_tag_role_for_event(ev: &nostr::Event, pubkey: &str) -> PTagRole {
    p_tag_role(ev.tags.iter().map(|tag| tag.as_slice()), pubkey)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(|part| (*part).to_string()).collect())
            .collect()
    }

    fn role(rows: &[&[&str]], pubkey: &str) -> PTagRole {
        let owned = tags(rows);
        p_tag_role(owned.iter().map(|tag| tag.as_slice()), pubkey)
    }

    const ME: &str = "aa";
    const OTHER: &str = "bb";

    #[test]
    fn a_marked_mention_is_a_mention() {
        assert_eq!(role(&[&["p", ME, "", "mention"]], ME), PTagRole::Mention);
    }

    #[test]
    fn a_marked_addressing_tag_is_addressing() {
        assert_eq!(role(&[&["p", ME, "", "reply"]], ME), PTagRole::Addressing);
    }

    #[test]
    fn a_bare_tag_is_unknown_not_a_mention() {
        // The whole point of the one-way read: absent means "ask the parent".
        assert_eq!(role(&[&["p", ME]], ME), PTagRole::Unknown);
    }

    #[test]
    fn mention_wins_over_addressing_on_the_same_pubkey() {
        assert_eq!(
            role(&[&["p", ME, "", "reply"], &["p", ME, "", "mention"]], ME),
            PTagRole::Mention,
        );
    }

    #[test]
    fn a_bare_tag_alongside_a_marked_one_still_reads_unknown() {
        // Two senders' shapes cannot be mixed into a confident answer.
        assert_eq!(
            role(&[&["p", ME, "", "reply"], &["p", ME]], ME),
            PTagRole::Unknown,
        );
    }

    #[test]
    fn tags_naming_someone_else_are_ignored() {
        assert_eq!(role(&[&["p", OTHER, "", "mention"]], ME), PTagRole::None);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(role(&[&["p", "AA", "", "mention"]], ME), PTagRole::Mention);
    }
}
