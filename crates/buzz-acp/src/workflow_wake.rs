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

/// Production wake ingress: resolve current identity before definitive signer
/// rejection, and reopen transport dedup only for paced, pending discovery.
pub(crate) async fn authenticate_for_listener(
    gate: &mut crate::inbound_author_gate::InboundAuthorGate,
    relay: &crate::relay::HarnessRelay,
    rest: &crate::relay::RestClient,
    event: &crate::relay::BuzzEvent,
) -> Option<(WorkflowMentionWake, PublicKey)> {
    use crate::inbound_author_gate::WakeIdentity;
    match gate
        .wake_identity_for_generation(rest, event.connection_generation)
        .await
    {
        WakeIdentity::Ready(key) => authenticate(&event.event, key).map(|wake| (wake, key)),
        WakeIdentity::Unavailable => None,
        WakeIdentity::Retry => {
            if let Err(error) = relay
                .replay_event(
                    event.channel_id,
                    event.event.id.to_hex(),
                    event.event.created_at.as_secs(),
                )
                .await
            {
                tracing::warn!(%error, "failed to arrange workflow identity replay");
            }
            None
        }
    }
}

/// Return whether a revision-labelled workflow message must dispatch only
/// through its separately verified wake, regardless of relay key rotation.
pub fn requires_verified_wake(event: &Event) -> bool {
    event.kind.as_u16() as u32 == KIND_STREAM_MESSAGE
        && single_tag(event, "workflow-run").is_some()
        && single_tag(event, "workflow-definition").is_some()
        && single_tag(event, "workflow-step").is_some()
}

/// Authenticate and parse a relay-signed workflow wake before any authority lookup.
pub fn authenticate(wake_event: &Event, relay_pubkey: PublicKey) -> Option<WorkflowMentionWake> {
    if wake_event.pubkey != relay_pubkey || wake_event.verify().is_err() {
        return None;
    }
    WorkflowMentionWake::parse(wake_event).ok()
}

/// Verify every authority edge and return the visible message plus its signed author principal.
pub fn verify(
    wake_event: &Event,
    authority: WorkflowWakeAuthority,
    relay_pubkey: PublicKey,
    agent_pubkey: PublicKey,
    subscription_channel: Uuid,
) -> Option<(Event, String)> {
    let wake = authenticate(wake_event, relay_pubkey)?;
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
    // Use the same authored-mention boundary as ordinary listener admission.
    // A legacy `p` tag can come entirely from trigger-controlled rendered text;
    // even a signed wake must not turn it into the definition owner's authority.
    let attributed_owner = crate::verified_workflow_owner(
        &message,
        Some(&relay_pubkey.to_hex()),
        &agent_pubkey.to_hex(),
    )?;
    if attributed_owner != definition.pubkey.to_hex() {
        return None;
    }
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
    if let Some(target) = step
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // The endpoint's relay-signed message is the authority for a resolved
        // template target. A definition stores templates, while its execution
        // resolves them from per-run state unavailable to ACP; comparing raw
        // text would reject valid targets. Literal targets remain an independent
        // constraint and are compared as UUIDs so noncanonical spelling works.
        if !target.contains("{{") {
            let target = Uuid::parse_str(target).ok()?;
            let message_channel = Uuid::parse_str(channel).ok()?;
            if target != message_channel {
                return None;
            }
        }
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
                    Tag::parse(["buzz:workflow", "true"]).expect("workflow marker"),
                    Tag::parse(["buzz:workflow-owner", &owner.public_key().to_hex()])
                        .expect("owner tag"),
                    Tag::parse(["buzz:workflow-mention", &agent.public_key().to_hex()])
                        .expect("authored mention"),
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
    fn workflow_message_is_ineligible_for_direct_dispatch() {
        let fixture = Fixture::valid();
        assert!(requires_verified_wake(&fixture.message));

        let ordinary = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "ordinary")
            .tags([
                Tag::parse(["h", &fixture.channel.to_string()]).expect("h tag"),
                Tag::parse(["p", &fixture.agent.public_key().to_hex()]).expect("p tag"),
                Tag::parse(["buzz:workflow", "true"]).expect("workflow marker"),
                Tag::parse(["buzz:workflow-owner", &fixture.owner.public_key().to_hex()])
                    .expect("owner tag"),
                Tag::parse([
                    "buzz:workflow-mention",
                    &fixture.agent.public_key().to_hex(),
                ])
                .expect("authored mention"),
            ])
            .sign_with_keys(&fixture.relay)
            .expect("ordinary message");
        assert!(!requires_verified_wake(&ordinary));
    }

    #[test]
    fn rejects_forged_wake_before_authority_lookup() {
        let fixture = Fixture::valid();
        let forged = WorkflowMentionWake::new(
            fixture.agent.public_key(),
            fixture.channel,
            fixture.run,
            fixture.definition.id,
            fixture.message.id,
        )
        .sign(&Keys::generate())
        .expect("forged wake");

        assert!(authenticate(&forged, fixture.relay.public_key()).is_none());
        assert!(authenticate(&fixture.wake, fixture.relay.public_key()).is_some());
    }

    #[test]
    fn accepts_exact_authority_and_returns_signed_owner() {
        let fixture = Fixture::valid();
        let (message, author) = fixture.verify(fixture.authority()).expect("verified");
        assert_eq!(message.id, fixture.message.id);
        assert_eq!(author, fixture.owner.public_key().to_hex());
    }

    #[test]
    fn signed_wake_cannot_promote_rendered_only_mentions_to_owner_authority() {
        let fixture = Fixture::valid();
        // Re-sign both objects so every existing signature, recipient and exact
        // provenance edge is valid. Only the authored-mention boundary differs.
        for replacement in [None, Some(Keys::generate().public_key().to_hex())] {
            let mut tags: Vec<Tag> = fixture
                .message
                .tags
                .iter()
                .filter(|tag| tag.as_slice()[0] != "buzz:workflow-mention")
                .cloned()
                .collect();
            if let Some(other) = replacement {
                tags.push(Tag::parse(["buzz:workflow-mention", &other]).expect("other mention"));
            }
            let message = EventBuilder::new(fixture.message.kind, "@Agent injected by trigger")
                .tags(tags)
                .sign_with_keys(&fixture.relay)
                .expect("signed message");
            let wake = WorkflowMentionWake::new(
                fixture.agent.public_key(),
                fixture.channel,
                fixture.run,
                fixture.definition.id,
                message.id,
            )
            .sign(&fixture.relay)
            .expect("signed wake");
            let mut authority = fixture.authority();
            authority.message = message;
            assert!(super::verify(
                &wake,
                authority,
                fixture.relay.public_key(),
                fixture.agent.public_key(),
                fixture.channel
            )
            .is_none());
        }
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
                    Tag::parse(["buzz:workflow", "true"]).expect("workflow marker"),
                    Tag::parse(["buzz:workflow-owner", &fixture.owner.public_key().to_hex()])
                        .expect("owner tag"),
                    Tag::parse([
                        "buzz:workflow-mention",
                        &fixture.agent.public_key().to_hex(),
                    ])
                    .expect("authored mention"),
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
    fn accepts_template_target_using_relay_signed_resolved_message_channel() {
        let fixture = Fixture::new(
            "name: wake\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify\n    action: send_message\n    text: do work\n    channel: '{{trigger.channel_id}}'\n",
            None,
        );
        assert!(fixture.verify(fixture.authority()).is_some());
    }

    #[test]
    fn accepts_noncanonical_literal_uuid_target() {
        let fixture = Fixture::new(
            "name: wake\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify\n    action: send_message\n    text: do work\n    channel: $CHANNEL\n",
            None,
        );
        let noncanonical = fixture.channel.simple().to_string();
        let definition = EventBuilder::new(
            Kind::Custom(KIND_WORKFLOW_DEF as u16),
            format!("name: wake\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify\n    action: send_message\n    text: do work\n    channel: {noncanonical}\n"),
        )
        .tags([
            Tag::parse(["d", &fixture.workflow.to_string()]).expect("d tag"),
            Tag::parse(["h", &fixture.channel.to_string()]).expect("h tag"),
        ])
        .sign_with_keys(&fixture.owner)
        .expect("definition");
        let message = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "do work")
            .tags([
                Tag::parse(["h", &fixture.channel.to_string()]).expect("h tag"),
                Tag::parse(["p", &fixture.agent.public_key().to_hex()]).expect("p tag"),
                Tag::parse(["buzz:workflow", "true"]).expect("workflow marker"),
                Tag::parse(["buzz:workflow-owner", &fixture.owner.public_key().to_hex()])
                    .expect("owner tag"),
                Tag::parse([
                    "buzz:workflow-mention",
                    &fixture.agent.public_key().to_hex(),
                ])
                .expect("authored mention"),
                Tag::parse(["workflow-run", &fixture.run.to_string()]).expect("run tag"),
                Tag::parse(["workflow-definition", &definition.id.to_hex()])
                    .expect("definition tag"),
                Tag::parse(["workflow-step", "notify"]).expect("step tag"),
            ])
            .sign_with_keys(&fixture.relay)
            .expect("message");
        let wake = WorkflowMentionWake::new(
            fixture.agent.public_key(),
            fixture.channel,
            fixture.run,
            definition.id,
            message.id,
        )
        .sign(&fixture.relay)
        .expect("wake");
        let authority = WorkflowWakeAuthority {
            definition_event_id: definition.id.to_hex(),
            definition,
            message,
            ..fixture.authority()
        };
        assert!(super::verify(
            &wake,
            authority,
            fixture.relay.public_key(),
            fixture.agent.public_key(),
            fixture.channel,
        )
        .is_some());
    }

    #[test]
    fn malformed_literal_target_remains_rejected() {
        let fixture = Fixture::new(
            "name: wake\ntrigger:\n  on: message_posted\nsteps:\n  - id: notify\n    action: send_message\n    text: do work\n    channel: not-a-channel\n",
            None,
        );
        assert!(fixture.verify(fixture.authority()).is_none());
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
