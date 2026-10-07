use nostr::{EventBuilder, Kind, Tag, Timestamp};
use uuid::Uuid;

use super::{check_content, check_pubkey, tag, thread_tags, ThreadRef};

/// A stable NIP-AR artifact UUID paired with one exact revision event ID.
pub struct ArtifactRevisionRef<'a> {
    pub artifact_id: &'a str,
    pub revision_event_id: &'a str,
}

fn revision_ref_tag(name: &str, reference: &ArtifactRevisionRef<'_>) -> Result<Tag, String> {
    let canonical_uuid = Uuid::parse_str(reference.artifact_id)
        .ok()
        .filter(|id| !id.is_nil() && id.to_string() == reference.artifact_id);
    if canonical_uuid.is_none() {
        return Err(format!(
            "{name} tag needs a canonical lowercase artifact UUID"
        ));
    }
    let id = reference.revision_event_id;
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!(
            "{name} tag needs a 64-character lowercase hex revision event ID"
        ));
    }
    tag(vec![name, reference.artifact_id, id])
}

/// Kind 9 — the in-thread reply that wakes the executive agent after a human
/// published a `synaxis.artifact-feedback` NIP-AR revision.
///
/// NIP-AR state changes never notify, so this ordinary message is the wake
/// path. It carries the exact `h` channel, NIP-10 root/reply lineage, one `p`
/// mention of the executive agent, and two custom tags naming the immutable
/// feedback revision (`feedback`) and the reviewed revision (`artifact`).
pub fn build_artifact_feedback_message(
    channel_id: Uuid,
    content: &str,
    thread_ref: &ThreadRef,
    executive_agent_pubkey: &str,
    feedback: &ArtifactRevisionRef<'_>,
    reviewed: &ArtifactRevisionRef<'_>,
    created_at: u64,
) -> Result<EventBuilder, String> {
    if created_at == 0 {
        return Err("feedback notification needs a frozen timestamp".into());
    }
    check_content(content)?;
    if content.trim().is_empty() {
        return Err("feedback notification content is required".into());
    }
    check_pubkey(executive_agent_pubkey)?;
    let mut tags = vec![tag(vec!["h", &channel_id.to_string()])?];
    tags.extend(thread_tags(thread_ref)?);
    tags.push(tag(vec![
        "p",
        &executive_agent_pubkey.to_ascii_lowercase(),
    ])?);
    tags.push(revision_ref_tag("feedback", feedback)?);
    tags.push(revision_ref_tag("artifact", reviewed)?);
    // The timestamp is part of the event ID. The caller freezes it with the rest
    // of the draft, so a prompt retry rebuilds the byte-identical event. After a
    // stale-timestamp refusal the caller rebuilds it once with a fresh one: the
    // same bindings under a new ID, which the relay's per-feedback wake ledger
    // collapses into the one recorded wake.
    Ok(EventBuilder::new(Kind::Custom(9), content.trim())
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventId, Keys};

    const CHANNEL: &str = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
    const FEEDBACK: &str = "24737c81-e5e8-4412-bb47-f446813cfeba";
    const REVIEW: &str = "04737c81-e5e8-4412-bb47-f446813cfeba";
    const AGENT: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const FEEDBACK_REV: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const REVIEW_REV: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const ORIGIN: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    const NOTIFICATION: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

    fn refs() -> (ArtifactRevisionRef<'static>, ArtifactRevisionRef<'static>) {
        (
            ArtifactRevisionRef {
                artifact_id: FEEDBACK,
                revision_event_id: FEEDBACK_REV,
            },
            ArtifactRevisionRef {
                artifact_id: REVIEW,
                revision_event_id: REVIEW_REV,
            },
        )
    }

    fn thread() -> ThreadRef {
        ThreadRef {
            root_event_id: EventId::from_hex(ORIGIN).unwrap(),
            parent_event_id: EventId::from_hex(NOTIFICATION).unwrap(),
        }
    }

    fn build(content: &str, agent: &str) -> Result<nostr::Event, String> {
        let (feedback, reviewed) = refs();
        build_artifact_feedback_message(
            Uuid::parse_str(CHANNEL).unwrap(),
            content,
            &thread(),
            agent,
            &feedback,
            &reviewed,
            1_700_000_000,
        )
        .map(|builder| builder.sign_with_keys(&Keys::generate()).unwrap())
    }

    fn tags(event: &nostr::Event) -> Vec<Vec<String>> {
        event.tags.iter().map(|t| t.as_slice().to_vec()).collect()
    }

    #[test]
    fn wake_message_carries_exact_lineage_mention_and_revision_refs() {
        let event = build("Feedback on Primary checkout action", AGENT).unwrap();
        assert_eq!(event.kind.as_u16(), 9);
        assert_eq!(
            tags(&event),
            vec![
                vec!["h", CHANNEL],
                vec!["e", ORIGIN, "", "root"],
                vec!["e", NOTIFICATION, "", "reply"],
                vec!["p", AGENT],
                vec!["feedback", FEEDBACK, FEEDBACK_REV],
                vec!["artifact", REVIEW, REVIEW_REV],
            ]
        );
    }

    #[test]
    fn the_wake_event_id_is_deterministic_for_a_frozen_draft() {
        let keys = Keys::generate();
        let (feedback, reviewed) = refs();
        let build = |created_at: u64, content: &str| {
            build_artifact_feedback_message(
                Uuid::parse_str(CHANNEL).unwrap(),
                content,
                &thread(),
                AGENT,
                &feedback,
                &reviewed,
                created_at,
            )
            .unwrap()
            .sign_with_keys(&keys)
            .unwrap()
        };
        let first = build(1_700_000_000, "Feedback on a block");
        let retry = build(1_700_000_000, "Feedback on a block");
        assert_eq!(first.id, retry.id, "a retry rebuilds the same event ID");
        assert_eq!(first.created_at.as_secs(), 1_700_000_000);
        // Anything that is not frozen would mint a second, distinct wake.
        assert_ne!(first.id, build(1_700_000_001, "Feedback on a block").id);
        assert_ne!(first.id, build(1_700_000_000, "Feedback on another block").id);
    }

    #[test]
    fn a_zero_timestamp_is_refused() {
        let (feedback, reviewed) = refs();
        assert!(build_artifact_feedback_message(
            Uuid::parse_str(CHANNEL).unwrap(),
            "hello",
            &thread(),
            AGENT,
            &feedback,
            &reviewed,
            0,
        )
        .is_err());
    }

    #[test]
    fn rejects_blank_content_bad_agent_and_non_canonical_references() {
        assert!(build("   ", AGENT).is_err());
        assert!(build("hello", "not-a-pubkey").is_err());

        let (feedback, reviewed) = refs();
        let upper = FEEDBACK.to_uppercase();
        let bad_uuid = ArtifactRevisionRef {
            artifact_id: &upper,
            ..feedback
        };
        let result = build_artifact_feedback_message(
            Uuid::parse_str(CHANNEL).unwrap(),
            "hello",
            &thread(),
            AGENT,
            &bad_uuid,
            &reviewed,
            1_700_000_000,
        );
        assert!(result.is_err());

        let bad_event = ArtifactRevisionRef {
            revision_event_id: "ABC",
            ..reviewed
        };
        let (feedback, _) = refs();
        let result = build_artifact_feedback_message(
            Uuid::parse_str(CHANNEL).unwrap(),
            "hello",
            &thread(),
            AGENT,
            &feedback,
            &bad_event,
            1_700_000_000,
        );
        assert!(result.is_err());
    }
}
