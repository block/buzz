use crate::events;

/// A resolved NIP-10 thread reference plus the parent event's author.
///
/// The author is carried alongside the thread ref because a reply addresses
/// the person (or agent) being replied to: without their `p` tag the reply
/// notifies nobody and never wakes an agent harness.
pub(super) struct ResolvedThreadRef {
    pub(super) thread_ref: events::ThreadRef,
    pub(super) parent_pubkey: String,
}

/// Borrow the recipient list for the `events::build_*` tag builders, which
/// take `&[&str]`.
pub(super) fn mention_refs(mentions: &[String]) -> Vec<&str> {
    mentions.iter().map(String::as_str).collect()
}

/// Add the parent event's author to a reply's recipient list.
///
/// No-op when the pubkey is empty, when it is the sender's own (nobody
/// notifies themselves), or when an `@mention` already tagged them.
pub(super) fn add_reply_recipient(
    mentions: &mut Vec<String>,
    parent_pubkey: &str,
    sender_pubkey: &str,
) {
    let parent = parent_pubkey.trim().to_ascii_lowercase();
    let sender = sender_pubkey.trim().to_ascii_lowercase();
    if parent.is_empty()
        || parent == sender
        || mentions
            .iter()
            .any(|pubkey| pubkey.eq_ignore_ascii_case(&parent))
    {
        return;
    }
    mentions.push(parent);
}

#[cfg(test)]
mod tests {
    use super::add_reply_recipient;

    #[test]
    fn adds_parent_once_and_skips_sender() {
        let sender = "a".repeat(64);
        let parent = "b".repeat(64);
        let mut mentions = vec!["c".repeat(64)];

        add_reply_recipient(&mut mentions, &parent.to_uppercase(), &sender);
        add_reply_recipient(&mut mentions, &parent, &sender);
        add_reply_recipient(&mut mentions, &sender, &sender);

        assert_eq!(mentions, vec!["c".repeat(64), parent]);
    }

    #[test]
    fn skips_blank_parent_pubkey() {
        let mut mentions = Vec::new();
        add_reply_recipient(&mut mentions, "   ", &"a".repeat(64));
        assert!(mentions.is_empty());
    }
}
