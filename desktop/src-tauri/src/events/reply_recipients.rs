use super::mention_tags;
use nostr::{EventId, Tag};

/// NIP-10 thread reference plus the immediate parent author who must be notified.
pub struct ThreadRef {
    pub root_event_id: EventId,
    pub parent_event_id: EventId,
    pub parent_author_pubkey: String,
}

pub(super) fn tags(thread_ref: Option<&ThreadRef>, mentions: &[&str]) -> Result<Vec<Tag>, String> {
    let mut tags = mention_tags(mentions)?;
    if let Some(thread_ref) = thread_ref {
        let parent_author = thread_ref.parent_author_pubkey.as_str();
        if !tags
            .iter()
            .filter_map(|tag| tag.as_slice().get(1))
            .any(|pubkey| pubkey.eq_ignore_ascii_case(parent_author))
        {
            tags.extend(mention_tags(&[parent_author])?);
        }
    }
    Ok(tags)
}

#[cfg(test)]
mod tests {
    use super::super::{build_forum_comment, build_message};
    use super::*;
    use nostr::{EventId, Keys};
    use uuid::Uuid;

    fn thread_ref(parent_author_pubkey: String) -> ThreadRef {
        ThreadRef {
            root_event_id: EventId::from_hex(
                "1111111111111111111111111111111111111111111111111111111111111111",
            )
            .unwrap(),
            parent_event_id: EventId::from_hex(
                "2222222222222222222222222222222222222222222222222222222222222222",
            )
            .unwrap(),
            parent_author_pubkey,
        }
    }

    fn p_tag_value(tag: &Tag) -> Option<String> {
        let parts = tag.as_slice();
        (parts.first().map(String::as_str) == Some("p"))
            .then(|| parts.get(1).cloned())
            .flatten()
    }

    #[test]
    fn stream_reply_mentions_parent_author_without_explicit_mentions() {
        let parent_author = Keys::generate().public_key().to_hex();
        let thread_ref = thread_ref(parent_author.clone());
        let event = build_message(
            Uuid::new_v4(),
            "reply",
            Some(&thread_ref),
            &[],
            &[],
            &[],
            &[],
            &[],
            "http://localhost:3000",
        )
        .unwrap()
        .sign_with_keys(&Keys::generate())
        .unwrap();

        assert_eq!(
            event
                .tags
                .iter()
                .filter_map(p_tag_value)
                .collect::<Vec<_>>(),
            vec![parent_author]
        );
    }

    #[test]
    fn parent_author_deduplicates_case_insensitively() {
        let parent_author = Keys::generate().public_key().to_hex();
        let duplicate_uppercase = parent_author.to_ascii_uppercase();
        let thread_ref = thread_ref(parent_author.clone());
        let tags = tags(Some(&thread_ref), &[&duplicate_uppercase]).unwrap();

        assert_eq!(
            tags.iter().filter_map(p_tag_value).collect::<Vec<_>>(),
            vec![parent_author]
        );
    }

    #[test]
    fn automatic_parent_does_not_reduce_explicit_mention_cap() {
        let parent_author = Keys::generate().public_key().to_hex();
        let thread_ref = thread_ref(parent_author.clone());
        let mentions: Vec<String> = (0..50)
            .map(|_| Keys::generate().public_key().to_hex())
            .collect();
        let mention_refs: Vec<&str> = mentions.iter().map(String::as_str).collect();
        let tags = tags(Some(&thread_ref), &mention_refs).unwrap();
        let recipients: Vec<String> = tags.iter().filter_map(p_tag_value).collect();

        assert_eq!(recipients.len(), 51);
        assert_eq!(recipients.last(), Some(&parent_author));
    }

    #[test]
    fn forum_reply_mentions_parent_author_without_explicit_mentions() {
        let parent_author = Keys::generate().public_key().to_hex();
        let thread_ref = thread_ref(parent_author.clone());
        let event = build_forum_comment(Uuid::new_v4(), "reply", &thread_ref, &[], &[], &[])
            .unwrap()
            .sign_with_keys(&Keys::generate())
            .unwrap();

        assert_eq!(
            event
                .tags
                .iter()
                .filter_map(p_tag_value)
                .collect::<Vec<_>>(),
            vec![parent_author]
        );
    }
}
