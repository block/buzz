use std::collections::BTreeSet;

use nostr::Keys;
use uuid::Uuid;

use super::*;
use crate::domain::{DomainContext, DomainError};

fn principal(value: u8) -> Principal {
    let keys = Keys::parse(&format!("{value:064x}")).expect("test secret key");
    Principal::from_public_key(&keys.public_key()).expect("valid test principal")
}

fn readers(values: &[u8]) -> BTreeSet<Principal> {
    values.iter().copied().map(principal).collect()
}

fn community() -> CommunityId {
    CommunityId::from_uuid(Uuid::from_u128(1))
}

fn other_community() -> CommunityId {
    CommunityId::from_uuid(Uuid::from_u128(2))
}

fn label(values: &[u8]) -> ConfidentialityLabel {
    ConfidentialityLabel::restricted(community(), readers(values)).expect("non-empty readers")
}

fn non_egressing<const N: usize>(names: [&str; N]) -> CapabilitySet {
    CapabilitySet::from_operations(names.map(|name| (name, OperationEffect::NonEgressing)))
}

fn conversation_capabilities() -> CapabilitySet {
    CapabilitySet::from_operations([
        ("buzz.read.current", OperationEffect::NonEgressing),
        ("buzz.post", OperationEffect::Publication),
        ("buzz.reply", OperationEffect::Publication),
    ])
}

fn conversation(values: &[u8], channel_id: Uuid, epoch: &str) -> ExecutionDomain {
    ExecutionDomain::new(
        principal(9),
        Some(principal(1)),
        label(values),
        DomainContext::Conversation {
            community: community(),
            channel_id,
        },
        MembershipEpoch::new(epoch),
        conversation_capabilities(),
    )
    .expect("coherent conversation domain")
}

/// Reader identities enter the IFC lattice and domain key, so accepting an
/// arbitrary 32-byte string would let malformed identities become distinct
/// policy principals.
#[test]
fn principals_must_be_valid_x_only_secp256k1_points() {
    for invalid in ["00".repeat(32), "ff".repeat(32), format!("{:064x}", 5)] {
        assert_eq!(
            Principal::from_hex(&invalid),
            Err(PrincipalError::InvalidPublicKey)
        );
    }

    let lowercase = principal(1).to_hex();
    assert_eq!(
        Principal::from_hex(&lowercase.to_ascii_uppercase())
            .expect("uppercase hexadecimal principal")
            .to_hex(),
        lowercase
    );
}

/// The Buzz specialization must preserve the generic reader-set lattice: the
/// combined label may flow only to readers authorized for every input, and it
/// must never cross communities.
#[test]
fn buzz_labels_intersect_readers_and_never_cross_communities() {
    let combined = label(&[1, 2])
        .join(&label(&[1, 3]))
        .expect("same community");
    assert!(combined.can_flow_to(&label(&[1])));
    assert!(!combined.can_flow_to(&label(&[1, 2])));

    let other = ConfidentialityLabel::public(other_community());
    assert!(!ConfidentialityLabel::public(community()).can_flow_to(&other));
    assert_eq!(
        ConfidentialityLabel::public(community()).join(&other),
        Err(LabelError::CrossUniverse)
    );
}

/// Every capability ceiling must admit an operation. Conflicting effect
/// declarations keep the classification that requires more checking, so an
/// egressing operation cannot become an ordinary call.
#[test]
fn capability_intersection_keeps_the_most_restrictive_effect() {
    let bot = CapabilitySet::from_operations([
        ("buzz.post", OperationEffect::Publication),
        ("email.read", OperationEffect::NonEgressing),
    ]);
    let requester = CapabilitySet::from_operations([("buzz.post", OperationEffect::Publication)]);
    let domain = CapabilitySet::from_operations([
        ("buzz.post", OperationEffect::NonEgressing),
        ("drive.read", OperationEffect::NonEgressing),
    ]);

    assert_eq!(
        CapabilitySet::effective(&bot, &requester, &domain),
        CapabilitySet::from_operations([("buzz.post", OperationEffect::Publication)])
    );
}

/// Appendix B's domain mapping gives personal capabilities only to a genuine
/// two-party owner/agent DM. Shared conversations must use the narrower
/// conversation ceiling even when the owner initiated the work.
#[test]
fn derivation_grants_personal_capabilities_only_to_owner_private_work() {
    let owner = principal(1);
    let agent = principal(9);
    let policy = CapabilityPolicy::new(
        non_egressing(["buzz.read.current", "email.read"]),
        non_egressing(["buzz.read.current"]),
    );
    let owner_dm = derive_execution_domain(
        DomainFacts {
            community: community(),
            channel_id: Uuid::from_u128(1),
            kind: ConversationKind::DirectMessage,
            epoch: MembershipEpoch::new("membership:v1"),
            members: BTreeSet::from([agent, owner]),
            executing_agent: agent,
            requesters: BTreeSet::from([owner]),
            system_principal: None,
            owner: Some(owner),
        },
        &policy,
    )
    .expect("owner DM domain");
    assert!(matches!(
        &owner_dm.context,
        DomainContext::OwnerPrivate {
            owner: candidate,
            ..
        } if candidate == &owner
    ));
    assert_eq!(
        owner_dm.capabilities,
        non_egressing(["buzz.read.current", "email.read"])
    );
    assert_eq!(owner_dm.audience, label(&[1]));

    let public = derive_execution_domain(
        DomainFacts {
            community: community(),
            channel_id: Uuid::from_u128(2),
            kind: ConversationKind::Public,
            epoch: MembershipEpoch::new("community:v1"),
            members: BTreeSet::new(),
            executing_agent: agent,
            requesters: BTreeSet::from([owner]),
            system_principal: None,
            owner: Some(owner),
        },
        &policy,
    )
    .expect("public domain");
    assert_eq!(
        &public.context,
        &DomainContext::CommunityPublic(community())
    );
    assert!(public.audience.is_public());
    assert_eq!(public.capabilities, non_egressing(["buzz.read.current"]));
}

/// Restricted-domain derivation fails closed when authenticated requesters or
/// the executing agent are absent from the verified membership snapshot.
#[test]
fn restricted_derivation_requires_verified_membership() {
    let owner = principal(1);
    let agent = principal(9);
    let outsider = principal(8);
    let policy = CapabilityPolicy::new(
        non_egressing(["buzz.read.current"]),
        non_egressing(["buzz.read.current"]),
    );
    let facts = |members, requesters| DomainFacts {
        community: community(),
        channel_id: Uuid::from_u128(1),
        kind: ConversationKind::Restricted,
        epoch: MembershipEpoch::new("membership:v1"),
        members,
        executing_agent: agent,
        requesters,
        system_principal: None,
        owner: Some(owner),
    };

    assert_eq!(
        derive_execution_domain(facts(BTreeSet::new(), BTreeSet::new()), &policy),
        Err(DerivationError::EmptyRequesters)
    );
    assert_eq!(
        derive_execution_domain(
            facts(BTreeSet::from([owner]), BTreeSet::from([owner]),),
            &policy,
        ),
        Err(DerivationError::AgentNotMember)
    );
    assert_eq!(
        derive_execution_domain(
            facts(BTreeSet::from([owner, agent]), BTreeSet::from([outsider]),),
            &policy,
        ),
        Err(DerivationError::RequesterNotMember)
    );
}

/// The domain identifier is a security boundary for retained-state reuse. Every
/// field in `D = (agent, owner, audience, context, epoch, capabilities)` must
/// change the routing key independently.
#[test]
fn every_domain_component_changes_the_routing_key() {
    let base = conversation(&[1, 2], Uuid::from_u128(1), "v1");
    let build = |agent, owner, audience, channel_id, epoch, capabilities| {
        ExecutionDomain::new(
            agent,
            owner,
            audience,
            DomainContext::Conversation {
                community: community(),
                channel_id,
            },
            MembershipEpoch::new(epoch),
            capabilities,
        )
        .expect("coherent domain")
    };
    let ids = [
        base.key(),
        build(
            principal(8),
            Some(principal(1)),
            label(&[1, 2]),
            Uuid::from_u128(1),
            "v1",
            conversation_capabilities(),
        )
        .key(),
        build(
            principal(9),
            None,
            label(&[1, 2]),
            Uuid::from_u128(1),
            "v1",
            conversation_capabilities(),
        )
        .key(),
        build(
            principal(9),
            Some(principal(1)),
            label(&[1, 3]),
            Uuid::from_u128(1),
            "v1",
            conversation_capabilities(),
        )
        .key(),
        build(
            principal(9),
            Some(principal(1)),
            label(&[1, 2]),
            Uuid::from_u128(2),
            "v1",
            conversation_capabilities(),
        )
        .key(),
        build(
            principal(9),
            Some(principal(1)),
            label(&[1, 2]),
            Uuid::from_u128(1),
            "v2",
            conversation_capabilities(),
        )
        .key(),
        build(
            principal(9),
            Some(principal(1)),
            label(&[1, 2]),
            Uuid::from_u128(1),
            "v1",
            non_egressing(["buzz.read.current"]),
        )
        .key(),
    ];

    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());
}

/// This fixed value detects accidental changes to the canonical domain-key
/// encoding, which would otherwise strand or incorrectly reuse retained state.
#[test]
fn domain_key_has_a_canonical_golden_value() {
    let domain = conversation(&[1, 2], Uuid::from_u128(1), "membership:event-1");
    assert_eq!(
        domain.key().as_str(),
        "c83fb1c188109526fca87b73297eb00f9bf0b6545e36c6f0b8d3fb2771ce7962"
    );
}

/// Audience shape and retained context are independent inputs but must remain
/// coherent: public labels select public context, while restricted labels
/// select a conversation or exact owner-private context.
#[test]
fn domains_reject_incoherent_audience_context_pairs() {
    assert_eq!(
        ExecutionDomain::new(
            principal(9),
            Some(principal(1)),
            ConfidentialityLabel::public(community()),
            DomainContext::CommunityPublic(other_community()),
            MembershipEpoch::new("v1"),
            CapabilitySet::default(),
        ),
        Err(DomainError::ContextCommunityMismatch)
    );
    assert_eq!(
        ExecutionDomain::new(
            principal(9),
            Some(principal(1)),
            ConfidentialityLabel::public(community()),
            DomainContext::Conversation {
                community: community(),
                channel_id: Uuid::from_u128(2),
            },
            MembershipEpoch::new("v1"),
            CapabilitySet::default(),
        ),
        Err(DomainError::AudienceContextMismatch)
    );
}
