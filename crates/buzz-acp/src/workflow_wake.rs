//! Fail-closed verification of relay-signed workflow mention wakes.

use buzz_core::kind::{KIND_STREAM_MESSAGE, KIND_WORKFLOW_DEF};
use buzz_core::workflow_wake::WorkflowMentionWake;
use nostr::{Event, PublicKey};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
struct WorkflowAuthority {
    steps: Vec<WorkflowAuthorityStep>,
}

#[derive(Debug, Deserialize)]
struct WorkflowAuthorityStep {
    id: String,
    action: String,
    #[serde(default)]
    channel: Option<String>,
}

/// Exact public authority bundle returned by the authenticated relay read.
#[derive(Debug, Deserialize)]
pub struct WorkflowWakeAuthority {
    /// Exact run ID.
    pub run_id: Uuid,
    /// Exact workflow channel.
    pub channel_id: Uuid,
    /// Workflow ID named by the signed definition.
    pub workflow_id: Uuid,
    /// Exact signed definition revision ID.
    pub definition_event_id: String,
    /// Workflow owner authenticated against relay workflow state.
    pub workflow_owner: String,
    /// Owner-signed workflow definition.
    pub definition: Event,
    /// Relay-signed visible message.
    pub message: Event,
}

/// Verify every authority edge and return the visible message plus its signed author principal.
pub fn verify(
    wake_event: &Event,
    authority: WorkflowWakeAuthority,
    relay_pubkey: PublicKey,
    agent_pubkey: PublicKey,
    subscription_channel: Uuid,
) -> Option<(Event, String)> {
    if wake_event.pubkey != relay_pubkey || wake_event.verify().is_err() {
        return None;
    }
    let wake = WorkflowMentionWake::parse(wake_event).ok()?;
    if wake.recipient() != agent_pubkey
        || authority.workflow_owner != authority.definition.pubkey.to_hex()
        || wake.run_id() != authority.run_id
        || wake.channel_id() != subscription_channel
        || wake.channel_id() != authority.channel_id
        || wake.definition_event_id().to_hex() != authority.definition_event_id
        || wake.message_event_id() != authority.message.id
    {
        return None;
    }

    let definition = authority.definition;
    if definition.verify().is_err()
        || definition.kind.as_u16() as u32 != KIND_WORKFLOW_DEF
        || definition.id != wake.definition_event_id()
        || !exact_tag(&definition, "d", &authority.workflow_id.to_string())
    {
        return None;
    }
    let channel = single_tag(&definition, "h")?;
    if channel != authority.channel_id.to_string() {
        return None;
    }
    let message = authority.message;
    if message.verify().is_err()
        || message.pubkey != relay_pubkey
        || message.kind.as_u16() as u32 != KIND_STREAM_MESSAGE
        || !exact_tag(&message, "h", channel)
        || !contains_tag(&message, "p", &agent_pubkey.to_hex())
        || !exact_tag(&message, "workflow-run", &authority.run_id.to_string())
        || !exact_tag(&message, "workflow-definition", &definition.id.to_hex())
    {
        return None;
    }
    let step_id = single_tag(&message, "workflow-step")?;
    let workflow: WorkflowAuthority = serde_yaml::from_str(&definition.content).ok()?;
    let step = workflow.steps.iter().find(|step| step.id == step_id)?;
    if step.action != "send_message" {
        return None;
    }
    if step
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|value| value != channel)
    {
        return None;
    }
    Some((message, definition.pubkey.to_hex()))
}

fn single_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    let mut matches = event.tags.iter().filter_map(|tag| {
        let values = tag.as_slice();
        (values.len() == 2 && values[0] == name).then(|| values[1].as_str())
    });
    let value = matches.next()?;
    matches.next().is_none().then_some(value)
}

fn exact_tag(event: &Event, name: &str, value: &str) -> bool {
    single_tag(event, name).is_some_and(|actual| actual.eq_ignore_ascii_case(value))
}

fn contains_tag(event: &Event, name: &str, value: &str) -> bool {
    event.tags.iter().any(|tag| {
        let values = tag.as_slice();
        values.len() == 2 && values[0] == name && values[1].eq_ignore_ascii_case(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::KIND_WORKFLOW_MENTION_WAKE;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    struct Fixture {
        relay: Keys,
        agent: Keys,
        owner: Keys,
        channel: Uuid,
        run: Uuid,
        workflow: Uuid,
        definition: Event,
        message: Event,
        wake: Event,
    }

    impl Fixture {
        fn new(definition_content: &str, target_channel: Option<Uuid>) -> Self {
            let relay = Keys::generate();
            let agent = Keys::generate();
            let owner = Keys::generate();
            let channel = Uuid::new_v4();
            let run = Uuid::new_v4();
            let workflow = Uuid::new_v4();
            let definition = EventBuilder::new(
                Kind::Custom(KIND_WORKFLOW_DEF as u16),
                definition_content
                    .replace("$CHANNEL", &target_channel.unwrap_or(channel).to_string()),
            )
            .tags([
                Tag::parse(["d", &workflow.to_string()]).expect("d tag"),
                Tag::parse(["h", &channel.to_string()]).expect("h tag"),
            ])
            .sign_with_keys(&owner)
            .expect("definition");
            let message = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "do work")
                .tags([
                    Tag::parse(["h", &channel.to_string()]).expect("h tag"),
                    Tag::parse(["p", &agent.public_key().to_hex()]).expect("p tag"),
                    Tag::parse(["workflow-run", &run.to_string()]).expect("run tag"),
                    Tag::parse(["workflow-definition", &definition.id.to_hex()])
                        .expect("definition tag"),
                    Tag::parse(["workflow-step", "notify"]).expect("step tag"),
                ])
                .sign_with_keys(&relay)
                .expect("message");
            let wake = WorkflowMentionWake::new(
                agent.public_key(),
                channel,
                run,
                definition.id,
                message.id,
            )
            .sign(&relay)
            .expect("wake");
            Self {
                relay,
                agent,
                owner,
                channel,
                run,
                workflow,
                definition,
                message,
                wake,
            }
        }

        fn valid() -> Self {
            Self::new(
                "name: wake\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify\n    action: send_message\n    text: do work\n    channel: $CHANNEL\n",
                None,
            )
        }

        fn authority(&self) -> WorkflowWakeAuthority {
            WorkflowWakeAuthority {
                run_id: self.run,
                channel_id: self.channel,
                workflow_id: self.workflow,
                definition_event_id: self.definition.id.to_hex(),
                workflow_owner: self.owner.public_key().to_hex(),
                definition: self.definition.clone(),
                message: self.message.clone(),
            }
        }

        fn verify(&self, authority: WorkflowWakeAuthority) -> Option<(Event, String)> {
            super::verify(
                &self.wake,
                authority,
                self.relay.public_key(),
                self.agent.public_key(),
                self.channel,
            )
        }
    }

    #[test]
    fn accepts_exact_authority_and_returns_signed_owner() {
        let fixture = Fixture::valid();
        let (message, author) = fixture.verify(fixture.authority()).expect("verified");
        assert_eq!(message.id, fixture.message.id);
        assert_eq!(author, fixture.owner.public_key().to_hex());
    }

    #[test]
    fn rejects_wrong_wake_signer_or_recipient() {
        let fixture = Fixture::valid();
        assert!(super::verify(
            &fixture.wake,
            fixture.authority(),
            Keys::generate().public_key(),
            fixture.agent.public_key(),
            fixture.channel,
        )
        .is_none());
        assert!(super::verify(
            &fixture.wake,
            fixture.authority(),
            fixture.relay.public_key(),
            Keys::generate().public_key(),
            fixture.channel,
        )
        .is_none());
    }

    #[test]
    fn rejects_mismatched_run_revision_message_channel_and_owner() {
        let fixture = Fixture::valid();
        let mut authority = fixture.authority();
        authority.run_id = Uuid::new_v4();
        assert!(fixture.verify(authority).is_none());

        let mut authority = fixture.authority();
        authority.definition_event_id = EventBuilder::text_note("other")
            .sign_with_keys(&Keys::generate())
            .expect("event")
            .id
            .to_hex();
        assert!(fixture.verify(authority).is_none());

        let mut authority = fixture.authority();
        authority.message = EventBuilder::text_note("other")
            .sign_with_keys(&fixture.relay)
            .expect("event");
        assert!(fixture.verify(authority).is_none());

        let mut authority = fixture.authority();
        authority.channel_id = Uuid::new_v4();
        assert!(fixture.verify(authority).is_none());

        let mut authority = fixture.authority();
        authority.workflow_owner = Keys::generate().public_key().to_hex();
        assert!(fixture.verify(authority).is_none());
    }

    #[test]
    fn rejects_malformed_or_non_send_message_instruction() {
        let malformed = Fixture::new("not: [valid", None);
        assert!(malformed.verify(malformed.authority()).is_none());

        let other_action = Fixture::new(
            "name: wake\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify\n    action: add_reaction\n    emoji: thumbsup\n",
            None,
        );
        assert!(other_action.verify(other_action.authority()).is_none());
    }

    #[test]
    fn rejects_wrong_step_or_target_channel() {
        let fixture = Fixture::valid();
        let wrong_step_message =
            EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "do work")
                .tags([
                    Tag::parse(["h", &fixture.channel.to_string()]).expect("h tag"),
                    Tag::parse(["p", &fixture.agent.public_key().to_hex()]).expect("p tag"),
                    Tag::parse(["workflow-run", &fixture.run.to_string()]).expect("run tag"),
                    Tag::parse(["workflow-definition", &fixture.definition.id.to_hex()])
                        .expect("definition tag"),
                    Tag::parse(["workflow-step", "missing"]).expect("step tag"),
                ])
                .sign_with_keys(&fixture.relay)
                .expect("message");
        let wrong_step_wake = WorkflowMentionWake::new(
            fixture.agent.public_key(),
            fixture.channel,
            fixture.run,
            fixture.definition.id,
            wrong_step_message.id,
        )
        .sign(&fixture.relay)
        .expect("wake");
        let mut authority = fixture.authority();
        authority.message = wrong_step_message;
        assert!(super::verify(
            &wrong_step_wake,
            authority,
            fixture.relay.public_key(),
            fixture.agent.public_key(),
            fixture.channel,
        )
        .is_none());

        let wrong_target = Fixture::new(
            "name: wake\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify\n    action: send_message\n    text: do work\n    channel: $CHANNEL\n",
            Some(Uuid::new_v4()),
        );
        assert!(wrong_target.verify(wrong_target.authority()).is_none());
    }

    #[test]
    fn wake_kind_remains_identifier_only() {
        let fixture = Fixture::valid();
        assert_eq!(
            fixture.wake.kind.as_u16() as u32,
            KIND_WORKFLOW_MENTION_WAKE
        );
        assert!(fixture.wake.content.is_empty());
    }
}
