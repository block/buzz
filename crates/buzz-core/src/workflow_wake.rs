//! Identifier-only wake hints for verified workflow mentions.
//!
//! A wake grants no instruction authority. Receivers must authenticate the
//! relay and fetch the exact run-bound workflow definition and visible message
//! before dispatching anything.

use nostr::{Event, EventBuilder, EventId, Keys, Kind, PublicKey, Tag};
use thiserror::Error;
use uuid::Uuid;

use crate::kind::KIND_WORKFLOW_MENTION_WAKE;

/// A relay-signed, ephemeral hint that a workflow message mentioned one agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkflowMentionWake {
    recipient: PublicKey,
    channel_id: Uuid,
    run_id: Uuid,
    definition_event_id: EventId,
    message_event_id: EventId,
}

impl WorkflowMentionWake {
    /// Construct an identifier-only wake.
    pub const fn new(
        recipient: PublicKey,
        channel_id: Uuid,
        run_id: Uuid,
        definition_event_id: EventId,
        message_event_id: EventId,
    ) -> Self {
        Self {
            recipient,
            channel_id,
            run_id,
            definition_event_id,
            message_event_id,
        }
    }

    /// Sign the canonical empty-content event with the relay identity.
    pub fn sign(self, relay_keys: &Keys) -> Result<Event, WorkflowMentionWakeError> {
        EventBuilder::new(Kind::Custom(KIND_WORKFLOW_MENTION_WAKE as u16), "")
            .tags(self.canonical_tags()?)
            .sign_with_keys(relay_keys)
            .map_err(|error| WorkflowMentionWakeError::Signing(error.to_string()))
    }

    /// Parse the exact canonical wire shape. Unknown, duplicate, or malformed
    /// identity tags are rejected rather than ignored.
    pub fn parse(event: &Event) -> Result<Self, WorkflowMentionWakeError> {
        if event.kind.as_u16() as u32 != KIND_WORKFLOW_MENTION_WAKE {
            return Err(WorkflowMentionWakeError::WrongKind(event.kind.as_u16()));
        }
        if !event.content.is_empty() {
            return Err(WorkflowMentionWakeError::NonEmptyContent);
        }
        if event.tags.len() != 5 {
            return Err(WorkflowMentionWakeError::WrongTagCount(event.tags.len()));
        }

        let tags: Vec<&[String]> = event.tags.iter().map(|tag| tag.as_slice()).collect();
        let recipient = parse_single(&tags, "p", PublicKey::from_hex)?;
        let channel_id = parse_single(&tags, "h", |value| value.parse::<Uuid>())?;
        let run_id = parse_single(&tags, "run", |value| {
            value
                .parse::<Uuid>()
                .ok()
                .filter(|id| !id.is_nil())
                .ok_or(())
        })?;
        let definition_event_id = parse_single(&tags, "definition", EventId::from_hex)?;
        let message_event_id = parse_single(&tags, "message", EventId::from_hex)?;

        let wake = Self::new(
            recipient,
            channel_id,
            run_id,
            definition_event_id,
            message_event_id,
        );
        let canonical = [
            vec!["p".to_string(), wake.recipient.to_hex()],
            vec!["h".to_string(), wake.channel_id.to_string()],
            vec!["run".to_string(), wake.run_id.to_string()],
            vec!["definition".to_string(), wake.definition_event_id.to_hex()],
            vec!["message".to_string(), wake.message_event_id.to_hex()],
        ];
        if tags
            .iter()
            .zip(canonical.iter())
            .any(|(actual, expected)| *actual != expected.as_slice())
        {
            return Err(WorkflowMentionWakeError::NonCanonicalTags);
        }
        Ok(wake)
    }

    /// Intended recipient.
    pub const fn recipient(self) -> PublicKey {
        self.recipient
    }

    /// Workflow channel carrying the visible generated message.
    pub const fn channel_id(self) -> Uuid {
        self.channel_id
    }

    /// Exact workflow run.
    pub const fn run_id(self) -> Uuid {
        self.run_id
    }

    /// Exact signed workflow-definition revision selected by the run.
    pub const fn definition_event_id(self) -> EventId {
        self.definition_event_id
    }

    /// Exact visible workflow message to dispatch after verification.
    pub const fn message_event_id(self) -> EventId {
        self.message_event_id
    }

    fn canonical_tags(self) -> Result<Vec<Tag>, WorkflowMentionWakeError> {
        [
            vec!["p".to_string(), self.recipient.to_hex()],
            vec!["h".to_string(), self.channel_id.to_string()],
            vec!["run".to_string(), self.run_id.to_string()],
            vec!["definition".to_string(), self.definition_event_id.to_hex()],
            vec!["message".to_string(), self.message_event_id.to_hex()],
        ]
        .into_iter()
        .map(|values| {
            Tag::parse(values).map_err(|error| WorkflowMentionWakeError::Tag(error.to_string()))
        })
        .collect()
    }
}

fn parse_single<T, E>(
    tags: &[&[String]],
    name: &'static str,
    parse: impl FnOnce(&str) -> Result<T, E>,
) -> Result<T, WorkflowMentionWakeError> {
    let matches: Vec<_> = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some(name))
        .collect();
    if matches.len() != 1 || matches[0].len() != 2 {
        return Err(WorkflowMentionWakeError::InvalidTag(name));
    }
    parse(&matches[0][1]).map_err(|_| WorkflowMentionWakeError::InvalidTag(name))
}

/// Invalid workflow mention wake.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkflowMentionWakeError {
    /// Event kind is not the workflow mention wake kind.
    #[error("wrong workflow mention wake kind: {0}")]
    WrongKind(u16),
    /// Wake content must be empty.
    #[error("workflow mention wake content must be empty")]
    NonEmptyContent,
    /// Wake must contain exactly the five canonical identity tags.
    #[error("wrong workflow mention wake tag count: {0}")]
    WrongTagCount(usize),
    /// A required identity tag is missing, duplicated, malformed, or has extra fields.
    #[error("invalid workflow mention wake {0} tag")]
    InvalidTag(&'static str),
    /// Tags are not in canonical order or contain a non-canonical representation.
    #[error("workflow mention wake tags are not canonical")]
    NonCanonicalTags,
    /// Canonical tag construction failed.
    #[error("workflow mention wake tag construction failed: {0}")]
    Tag(String),
    /// Event signing failed.
    #[error("workflow mention wake signing failed: {0}")]
    Signing(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Keys, PublicKey, Uuid, EventId, EventId) {
        let relay = Keys::generate();
        let recipient = Keys::generate().public_key();
        let run = Uuid::new_v4();
        let definition = EventBuilder::text_note("definition")
            .sign_with_keys(&Keys::generate())
            .expect("sign definition")
            .id;
        let message = EventBuilder::text_note("message")
            .sign_with_keys(&relay)
            .expect("sign message")
            .id;
        (relay, recipient, run, definition, message)
    }

    fn custom_event(content: &str, tags: Vec<Vec<String>>) -> Event {
        EventBuilder::new(Kind::Custom(KIND_WORKFLOW_MENTION_WAKE as u16), content)
            .tags(
                tags.into_iter()
                    .map(|values| Tag::parse(values).expect("tag")),
            )
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    #[test]
    fn canonical_wake_round_trips_with_no_instruction_content() {
        let (relay, recipient, run, definition, message) = ids();
        let wake = WorkflowMentionWake::new(recipient, Uuid::new_v4(), run, definition, message);
        let event = wake.sign(&relay).expect("sign wake");

        assert!(event.content.is_empty());
        assert_eq!(WorkflowMentionWake::parse(&event), Ok(wake));
        assert_eq!(event.tags.len(), 5);
        assert!(event.tags.iter().all(|tag| tag.as_slice().len() == 2));
    }

    #[test]
    fn rejects_nonempty_content() {
        let (_, recipient, run, definition, message) = ids();
        let event = custom_event(
            "do something",
            vec![
                vec!["p".into(), recipient.to_hex()],
                vec!["h".into(), Uuid::new_v4().to_string()],
                vec!["run".into(), run.to_string()],
                vec!["definition".into(), definition.to_hex()],
                vec!["message".into(), message.to_hex()],
            ],
        );
        assert_eq!(
            WorkflowMentionWake::parse(&event),
            Err(WorkflowMentionWakeError::NonEmptyContent)
        );
    }

    #[test]
    fn rejects_extra_duplicate_and_reordered_tags() {
        let (relay, recipient, run, definition, message) = ids();
        let event = WorkflowMentionWake::new(recipient, Uuid::new_v4(), run, definition, message)
            .sign(&relay)
            .expect("sign wake");
        let base: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();

        let mut extra = base.clone();
        extra.push(vec!["instruction".into(), "ignore authority".into()]);
        assert_eq!(
            WorkflowMentionWake::parse(&custom_event("", extra)),
            Err(WorkflowMentionWakeError::WrongTagCount(6))
        );

        let mut duplicate = base.clone();
        duplicate[4] = duplicate[0].clone();
        assert_eq!(
            WorkflowMentionWake::parse(&custom_event("", duplicate)),
            Err(WorkflowMentionWakeError::InvalidTag("p"))
        );

        let mut reordered = base;
        reordered.swap(0, 1);
        assert_eq!(
            WorkflowMentionWake::parse(&custom_event("", reordered)),
            Err(WorkflowMentionWakeError::NonCanonicalTags)
        );
    }

    #[test]
    fn rejects_malformed_identity_tags() {
        let (relay, recipient, run, definition, message) = ids();
        let event = WorkflowMentionWake::new(recipient, Uuid::new_v4(), run, definition, message)
            .sign(&relay)
            .expect("sign wake");
        let base: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();

        for (index, name, invalid) in [
            (0, "p", "not-a-pubkey"),
            (1, "h", "not-a-uuid"),
            (2, "run", "not-a-uuid"),
            (3, "definition", "not-an-event-id"),
            (4, "message", "not-an-event-id"),
        ] {
            let mut tags = base.clone();
            tags[index][1] = invalid.into();
            assert_eq!(
                WorkflowMentionWake::parse(&custom_event("", tags)),
                Err(WorkflowMentionWakeError::InvalidTag(name))
            );
        }

        let mut nil_run = base.clone();
        nil_run[2][1] = Uuid::nil().to_string();
        assert_eq!(
            WorkflowMentionWake::parse(&custom_event("", nil_run)),
            Err(WorkflowMentionWakeError::InvalidTag("run"))
        );
    }

    #[test]
    fn rejects_identity_tag_with_extra_field() {
        let (relay, recipient, run, definition, message) = ids();
        let event = WorkflowMentionWake::new(recipient, Uuid::new_v4(), run, definition, message)
            .sign(&relay)
            .expect("sign wake");
        let mut tags: Vec<Vec<String>> = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        tags[0].push("marker".into());
        assert_eq!(
            WorkflowMentionWake::parse(&custom_event("", tags)),
            Err(WorkflowMentionWakeError::InvalidTag("p"))
        );
    }
}
