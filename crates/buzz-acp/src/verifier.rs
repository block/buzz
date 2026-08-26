//! Pure verification for authenticated workflow deliveries.
//!
//! The relay returns the exact signed workflow definition, the visible message,
//! and a narrowly scoped relay-signed receipt. This module verifies those
//! public artifacts without receiving private trigger context, webhook fields,
//! prior-step outputs, execution traces, or secrets. It performs no I/O; the
//! capability-gated runtime supplies the authenticated API response directly.

use buzz_core::kind::{KIND_STREAM_MESSAGE, KIND_WORKFLOW_DEF};
use buzz_core::tenant::CommunityId;
use buzz_core::workflow_delivery::{
    message_v1_targets, WorkflowDeliveryBinding, WorkflowDeliveryCause, WorkflowDeliveryId,
    WorkflowDeliveryReceipt, WorkflowDeliveryWake,
};
use uuid::Uuid;

/// Immutable delivery identity returned by the authenticated relay API.
#[derive(Clone, Debug)]
pub struct DeliverySnapshot {
    pub id: Uuid,
    pub community_id: CommunityId,
    pub workflow_id: Uuid,
    pub run_id: Uuid,
    pub step_id: String,
    pub definition_event_id: String,
    pub message_event_id: String,
    pub target_pubkey: String,
    pub cause: WorkflowDeliveryCause,
}

/// A permanent disagreement. Retrying the same artifacts cannot succeed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MismatchKind {
    Target,
    Definition,
    Message,
    Step,
    Channel,
    Receipt,
}

/// A required public artifact was not supplied. The caller may retry its read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnavailableKind {
    Definition,
    Message,
    Receipt,
    RelayIdentity,
}

/// Typed fail-closed verification result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    Mismatch(MismatchKind),
    Unavailable(UnavailableKind),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mismatch(kind) => write!(f, "permanent mismatch: {kind:?}"),
            Self::Unavailable(kind) => write!(f, "authority unavailable: {kind:?}"),
        }
    }
}

/// Public artifacts returned together by the authenticated delivery API.
#[derive(Clone, Debug, Default)]
pub struct FetchedAuthority<'a> {
    pub definition: Option<&'a nostr::Event>,
    pub message: Option<&'a nostr::Event>,
    pub receipt: Option<&'a nostr::Event>,
}

fn exact_tags<'a>(event: &'a nostr::Event, name: &str) -> Vec<&'a nostr::Tag> {
    event
        .tags
        .iter()
        .filter(|tag| tag.as_slice().first().map(String::as_str) == Some(name))
        .collect()
}

fn single_uuid_tag(event: &nostr::Event, name: &str) -> Option<Uuid> {
    let tags = exact_tags(event, name);
    (tags.len() == 1 && tags[0].as_slice().len() == 2)
        .then(|| tags[0].as_slice()[1].parse().ok())
        .flatten()
}

fn workflow_uuid(definition: &nostr::Event) -> Option<Uuid> {
    single_uuid_tag(definition, "d")
}

fn event_channel(event: &nostr::Event) -> Option<Uuid> {
    single_uuid_tag(event, "h")
}

/// Parse an authenticated, identifier-only live wake.
///
/// A wake is only a hint. It grants no claim or dispatch authority.
pub fn trusted_live_workflow_wake_delivery_id(
    event: &nostr::Event,
    agent_pubkey: &str,
    relay_self: Option<&str>,
) -> Option<WorkflowDeliveryId> {
    let relay_self = relay_self?;
    if !event.pubkey.to_hex().eq_ignore_ascii_case(relay_self) || event.verify().is_err() {
        return None;
    }
    let wake = WorkflowDeliveryWake::parse(event).ok()?;
    wake.target_pubkey()
        .to_hex()
        .eq_ignore_ascii_case(agent_pubkey)
        .then(|| wake.delivery_id())
}

/// Ensure a claimed row is the delivery named by a trusted wake.
pub fn wake_references_delivery(
    event: &nostr::Event,
    agent_pubkey: &str,
    relay_self: Option<&str>,
    delivery: &DeliverySnapshot,
) -> bool {
    trusted_live_workflow_wake_delivery_id(event, agent_pubkey, relay_self)
        .is_some_and(|id| id.as_uuid() == delivery.id)
        && delivery.target_pubkey.eq_ignore_ascii_case(agent_pubkey)
}

/// A relay-authored visible message declaring workflow shape.
pub fn is_workflow_delivery_candidate(event: &nostr::Event, relay_self: Option<&str>) -> bool {
    relay_self.is_some_and(|relay| {
        event.kind.as_u16() as u32 == KIND_STREAM_MESSAGE
            && event.pubkey.to_hex().eq_ignore_ascii_case(relay)
            && !exact_tags(event, "workflow-definition").is_empty()
    })
}

/// Resolve the principal used by the ordinary owner-only dispatch gate.
pub fn workflow_delivery_principal(
    author: &str,
    durable_workflow_owner: Option<&str>,
    workflow_shape: bool,
) -> Option<String> {
    if durable_workflow_owner.is_none() && workflow_shape {
        return None;
    }
    Some(
        durable_workflow_owner
            .map(str::to_owned)
            .unwrap_or_else(|| author.to_owned()),
    )
}

/// Verify a claimed delivery using only its exact signed public artifacts.
///
/// The receipt is the relay's signed assertion that it produced this visible
/// message for the exact immutable binding and trigger cause. Receipt issuance
/// independently revalidates event, schedule, or webhook authority server-side;
/// the agent therefore verifies the receipt rather than receiving private
/// rendering inputs or durable cause rows.
pub fn verify_workflow_delivery(
    delivery: &DeliverySnapshot,
    authority: &FetchedAuthority<'_>,
    agent_pubkey: &str,
    relay_self: Option<&str>,
) -> Result<(nostr::Event, String), VerifyError> {
    use buzz_workflow::schema::ActionDef;

    let relay_self = relay_self.ok_or(VerifyError::Unavailable(UnavailableKind::RelayIdentity))?;
    let relay_pubkey = nostr::PublicKey::from_hex(relay_self)
        .map_err(|_| VerifyError::Mismatch(MismatchKind::Receipt))?;
    let definition = authority
        .definition
        .ok_or(VerifyError::Unavailable(UnavailableKind::Definition))?;
    let message = authority
        .message
        .ok_or(VerifyError::Unavailable(UnavailableKind::Message))?;
    let receipt = authority
        .receipt
        .ok_or(VerifyError::Unavailable(UnavailableKind::Receipt))?;

    if !delivery.target_pubkey.eq_ignore_ascii_case(agent_pubkey) {
        return Err(VerifyError::Mismatch(MismatchKind::Target));
    }
    let target = nostr::PublicKey::from_hex(&delivery.target_pubkey)
        .map_err(|_| VerifyError::Mismatch(MismatchKind::Target))?;
    let definition_id = nostr::EventId::from_hex(&delivery.definition_event_id)
        .map_err(|_| VerifyError::Mismatch(MismatchKind::Definition))?;
    let message_id = nostr::EventId::from_hex(&delivery.message_event_id)
        .map_err(|_| VerifyError::Mismatch(MismatchKind::Message))?;
    let binding = WorkflowDeliveryBinding::new(
        delivery.community_id,
        delivery.workflow_id,
        delivery.run_id,
        delivery.step_id.clone(),
        target,
        definition_id,
        message_id,
        delivery.cause.clone(),
    )
    .map_err(|_| VerifyError::Mismatch(MismatchKind::Receipt))?;

    if definition.id != definition_id
        || definition.verify().is_err()
        || definition.kind.as_u16() as u32 != KIND_WORKFLOW_DEF
        || workflow_uuid(definition) != Some(delivery.workflow_id)
    {
        return Err(VerifyError::Mismatch(MismatchKind::Definition));
    }
    let channel =
        event_channel(definition).ok_or(VerifyError::Mismatch(MismatchKind::Definition))?;

    let admitted =
        message_v1_targets(message).map_err(|_| VerifyError::Mismatch(MismatchKind::Message))?;
    if message.id != message_id
        || message.verify().is_err()
        || message.kind.as_u16() as u32 != KIND_STREAM_MESSAGE
        || message.pubkey != relay_pubkey
        || event_channel(message) != Some(channel)
        || !admitted.contains(&target)
        || !exact_tags(message, "workflow-definition")
            .iter()
            .any(|tag| {
                tag.as_slice()
                    .get(1)
                    .is_some_and(|value| value.eq_ignore_ascii_case(&delivery.definition_event_id))
            })
        || !exact_tags(message, "workflow-run").iter().any(|tag| {
            tag.as_slice()
                .get(1)
                .is_some_and(|value| value == &delivery.run_id.to_string())
        })
        || !exact_tags(message, "workflow-step").iter().any(|tag| {
            tag.as_slice()
                .get(1)
                .is_some_and(|value| value == &delivery.step_id)
        })
    {
        return Err(VerifyError::Mismatch(MismatchKind::Message));
    }

    let (workflow, _) = buzz_workflow::WorkflowEngine::parse_yaml(&definition.content)
        .map_err(|_| VerifyError::Mismatch(MismatchKind::Step))?;
    let step = workflow
        .steps
        .iter()
        .find(|step| step.id == delivery.step_id)
        .ok_or(VerifyError::Mismatch(MismatchKind::Step))?;
    let ActionDef::SendMessage {
        channel: step_channel,
        ..
    } = &step.action
    else {
        return Err(VerifyError::Mismatch(MismatchKind::Step));
    };
    if step_channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some_and(|value| value.parse::<Uuid>().ok() != Some(channel))
    {
        return Err(VerifyError::Mismatch(MismatchKind::Channel));
    }

    WorkflowDeliveryReceipt::verify(
        receipt,
        relay_pubkey,
        WorkflowDeliveryId::from_uuid(delivery.id),
        &binding,
        message,
    )
    .map_err(|_| VerifyError::Mismatch(MismatchKind::Receipt))?;

    Ok((message.clone(), definition.pubkey.to_hex()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::workflow_delivery::WORKFLOW_DELIVERY_TARGET_MARKER;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    type DeliveryMutation = Box<dyn Fn(&mut DeliverySnapshot)>;

    struct Fixture {
        relay: Keys,
        owner: Keys,
        agent: Keys,
        delivery: DeliverySnapshot,
        definition: nostr::Event,
        message: nostr::Event,
        receipt: nostr::Event,
    }

    fn tag(values: &[&str]) -> Tag {
        Tag::parse(
            values
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn fixture() -> Fixture {
        let relay = Keys::generate();
        let owner = Keys::generate();
        let agent = Keys::generate();
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let workflow_id = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let delivery_id = Uuid::new_v4();
        let channel = Uuid::new_v4();
        let cause_event = EventBuilder::text_note("cause")
            .sign_with_keys(&owner)
            .unwrap();
        let definition = EventBuilder::new(
            Kind::Custom(KIND_WORKFLOW_DEF as u16),
            format!(
                "name: test\ntrigger:\n  on: webhook\nsteps:\n  - id: send\n    action: send_message\n    channel: {channel}\n    text: 'private rendering stays server-side'\n"
            ),
        )
        .tags([
            tag(&["d", &workflow_id.to_string()]),
            tag(&["h", &channel.to_string()]),
        ])
        .sign_with_keys(&owner)
        .unwrap();
        let message = EventBuilder::new(Kind::Custom(KIND_STREAM_MESSAGE as u16), "visible result")
            .tags([
                tag(&["h", &channel.to_string()]),
                tag(&[
                    "p",
                    &agent.public_key().to_hex(),
                    "",
                    WORKFLOW_DELIVERY_TARGET_MARKER,
                ]),
                tag(&["workflow-definition", &definition.id.to_hex()]),
                tag(&["workflow-run", &run_id.to_string()]),
                tag(&["workflow-step", "send"]),
            ])
            .sign_with_keys(&relay)
            .unwrap();
        let cause = WorkflowDeliveryCause::Event(cause_event.id);
        let binding = WorkflowDeliveryBinding::new(
            community,
            workflow_id,
            run_id,
            "send",
            agent.public_key(),
            definition.id,
            message.id,
            cause.clone(),
        )
        .unwrap();
        let receipt = WorkflowDeliveryReceipt::new(
            WorkflowDeliveryId::from_uuid(delivery_id),
            binding,
            &message,
        )
        .unwrap()
        .sign(&relay, &message)
        .unwrap();
        let delivery = DeliverySnapshot {
            id: delivery_id,
            community_id: community,
            workflow_id,
            run_id,
            step_id: "send".to_owned(),
            definition_event_id: definition.id.to_hex(),
            message_event_id: message.id.to_hex(),
            target_pubkey: agent.public_key().to_hex(),
            cause,
        };
        Fixture {
            relay,
            owner,
            agent,
            delivery,
            definition,
            message,
            receipt,
        }
    }

    fn verify(f: &Fixture) -> Result<(nostr::Event, String), VerifyError> {
        verify_workflow_delivery(
            &f.delivery,
            &FetchedAuthority {
                definition: Some(&f.definition),
                message: Some(&f.message),
                receipt: Some(&f.receipt),
            },
            &f.agent.public_key().to_hex(),
            Some(&f.relay.public_key().to_hex()),
        )
    }

    #[test]
    fn receipt_verifies_without_private_execution_inputs() {
        let f = fixture();
        let (message, owner) = verify(&f).unwrap();
        assert_eq!(message, f.message);
        assert_eq!(owner, f.owner.public_key().to_hex());
        let wire = serde_json::to_string(&f.receipt).unwrap();
        assert!(!wire.contains("private rendering stays server-side"));
        assert!(!wire.contains("trigger_context"));
        assert!(!wire.contains("execution_trace"));
    }

    #[test]
    fn missing_public_artifacts_are_transient() {
        let f = fixture();
        for (authority, expected) in [
            (
                FetchedAuthority {
                    definition: None,
                    message: Some(&f.message),
                    receipt: Some(&f.receipt),
                },
                UnavailableKind::Definition,
            ),
            (
                FetchedAuthority {
                    definition: Some(&f.definition),
                    message: None,
                    receipt: Some(&f.receipt),
                },
                UnavailableKind::Message,
            ),
            (
                FetchedAuthority {
                    definition: Some(&f.definition),
                    message: Some(&f.message),
                    receipt: None,
                },
                UnavailableKind::Receipt,
            ),
        ] {
            assert_eq!(
                verify_workflow_delivery(
                    &f.delivery,
                    &authority,
                    &f.agent.public_key().to_hex(),
                    Some(&f.relay.public_key().to_hex()),
                ),
                Err(VerifyError::Unavailable(expected))
            );
        }
    }

    #[test]
    fn every_receipt_binding_field_fails_closed_when_mutated() {
        let f = fixture();
        let mut mutations: Vec<DeliveryMutation> = vec![
            Box::new(|d| d.id = Uuid::new_v4()),
            Box::new(|d| d.community_id = CommunityId::from_uuid(Uuid::new_v4())),
            Box::new(|d| d.workflow_id = Uuid::new_v4()),
            Box::new(|d| d.run_id = Uuid::new_v4()),
            Box::new(|d| d.step_id = "other".to_owned()),
            Box::new(|d| d.definition_event_id = "11".repeat(32)),
            Box::new(|d| d.message_event_id = "22".repeat(32)),
            Box::new(|d| d.target_pubkey = Keys::generate().public_key().to_hex()),
            Box::new(|d| {
                d.cause = WorkflowDeliveryCause::Webhook {
                    invocation_id: Uuid::new_v4(),
                }
            }),
        ];
        for mutate in mutations.drain(..) {
            let mut delivery = f.delivery.clone();
            mutate(&mut delivery);
            assert!(verify_workflow_delivery(
                &delivery,
                &FetchedAuthority {
                    definition: Some(&f.definition),
                    message: Some(&f.message),
                    receipt: Some(&f.receipt),
                },
                &f.agent.public_key().to_hex(),
                Some(&f.relay.public_key().to_hex()),
            )
            .is_err());
        }
    }

    #[test]
    fn wrong_relay_or_tampered_receipt_fails_closed() {
        let f = fixture();
        assert_eq!(
            verify_workflow_delivery(
                &f.delivery,
                &FetchedAuthority {
                    definition: Some(&f.definition),
                    message: Some(&f.message),
                    receipt: Some(&f.receipt),
                },
                &f.agent.public_key().to_hex(),
                Some(&Keys::generate().public_key().to_hex()),
            ),
            Err(VerifyError::Mismatch(MismatchKind::Message))
        );
        let mut receipt = f.receipt.clone();
        receipt.content = "tampered".to_owned();
        assert_eq!(
            verify_workflow_delivery(
                &f.delivery,
                &FetchedAuthority {
                    definition: Some(&f.definition),
                    message: Some(&f.message),
                    receipt: Some(&receipt),
                },
                &f.agent.public_key().to_hex(),
                Some(&f.relay.public_key().to_hex()),
            ),
            Err(VerifyError::Mismatch(MismatchKind::Receipt))
        );
    }

    #[test]
    fn wake_is_only_an_identifier_hint() {
        let f = fixture();
        let wake = WorkflowDeliveryWake::new(
            f.agent.public_key(),
            WorkflowDeliveryId::from_uuid(f.delivery.id),
        )
        .event_builder()
        .unwrap()
        .sign_with_keys(&f.relay)
        .unwrap();
        assert!(wake_references_delivery(
            &wake,
            &f.agent.public_key().to_hex(),
            Some(&f.relay.public_key().to_hex()),
            &f.delivery,
        ));
        assert!(!wake_references_delivery(
            &wake,
            &Keys::generate().public_key().to_hex(),
            Some(&f.relay.public_key().to_hex()),
            &f.delivery,
        ));
    }

    #[test]
    fn workflow_shape_without_durable_owner_fails_closed() {
        assert_eq!(workflow_delivery_principal("author", None, true), None);
        assert_eq!(
            workflow_delivery_principal("author", Some("owner"), true),
            Some("owner".to_owned())
        );
    }
}
