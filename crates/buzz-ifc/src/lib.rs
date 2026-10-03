//! Deterministic information-flow policy for Buzz agent execution.
//!
//! This crate contains no relay, ACP, process, or storage code. A trusted Buzz
//! adapter verifies events and membership, supplies [`DomainFacts`], and uses
//! these rules to derive an [`ExecutionDomain`] and decide whether a worker may
//! be reused, read a resource, call an operation, or publish a result. Keeping
//! the policy pure lets local ACP and remote harnesses apply the same rules
//! without sharing an agent implementation.
//!
//! Comments prefixed with `Paper:` identify the matching section of
//! "Practical information-flow for Buzz agents."

mod declassification;
mod domain;
mod hash;
mod label;
mod policy;

pub use declassification::{
    DeclassificationGrant, GrantError, GrantSignatureVerifier, PendingGrant, VerifiedGrant,
};
pub use domain::{
    derive_execution_domain, CapabilityPolicy, CapabilitySet, CompartmentProfile, ConversationKind,
    DerivationError, DomainContext, DomainError, DomainFacts, DomainKey, ExecutionDomain,
    MembershipEpoch, ResourceLabel,
};
pub use label::{ConfidentialityLabel, LabelError, Principal, PrincipalError, RealmId};
pub use policy::{ProcessState, RuleDecision, RuleEvaluator};

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nostr::Keys;
    use uuid::Uuid;

    use super::*;
    use crate::label::ReaderSet;

    fn principal(value: u8) -> Principal {
        let secret_key = format!("{value:064x}");
        let keys = Keys::parse(&secret_key).expect("test secret key");
        Principal::from_public_key(&keys.public_key()).expect("test principal")
    }

    fn readers(values: &[u8]) -> BTreeSet<Principal> {
        values.iter().copied().map(principal).collect()
    }

    fn realm() -> RealmId {
        RealmId::from_relay_url("wss://buzz.example")
    }

    fn label(values: &[u8]) -> ConfidentialityLabel {
        ConfidentialityLabel::restricted(realm(), readers(values)).expect("non-empty readers")
    }

    #[test]
    fn principals_must_be_valid_x_only_secp256k1_points() {
        for invalid in ["00".repeat(32), "ff".repeat(32), format!("{:064x}", 5)] {
            assert_eq!(
                Principal::from_hex(&invalid),
                Err(PrincipalError::InvalidPublicKey)
            );
        }
    }

    fn conversation(values: &[u8], channel_id: Uuid, epoch: &str) -> ExecutionDomain {
        ExecutionDomain::new(
            principal(9),
            label(values),
            DomainContext::Conversation {
                realm: realm(),
                channel_id,
            },
            MembershipEpoch::new(epoch),
            CapabilitySet::from_names(["buzz.read.current"]),
        )
        .expect("same realm")
    }

    #[test]
    fn reader_sets_obey_lattice_laws() {
        let values = [
            ReaderSet::Everyone,
            ReaderSet::Only(readers(&[1])),
            ReaderSet::Only(readers(&[2])),
            ReaderSet::Only(readers(&[1, 2])),
        ];

        for a in &values {
            assert_eq!(a.join(a), *a);
            assert_eq!(a.meet(a), *a);
            for b in &values {
                assert_eq!(a.join(b), b.join(a));
                assert_eq!(a.meet(b), b.meet(a));
                assert_eq!(a.join(&a.meet(b)), *a);
                assert_eq!(a.meet(&a.join(b)), *a);
                for c in &values {
                    assert_eq!(a.join(&b.join(c)), a.join(b).join(c));
                    assert_eq!(a.meet(&b.meet(c)), a.meet(b).meet(c));
                }
            }
        }
    }

    #[test]
    fn combining_inputs_intersects_authorized_readers() {
        let left = label(&[1, 2]);
        let right = label(&[1, 3]);
        let combined = left.join(&right).expect("same realm");

        assert!(combined.can_flow_to(&label(&[1])));
        assert!(!combined.can_flow_to(&label(&[1, 2])));
    }

    #[test]
    fn labels_never_flow_across_realms() {
        let first = ConfidentialityLabel::public(RealmId::from_relay_url("wss://one"));
        let second = ConfidentialityLabel::public(RealmId::from_relay_url("wss://two"));

        assert!(!first.can_flow_to(&second));
        assert_eq!(first.join(&second), Err(LabelError::CrossRealm));
    }

    #[test]
    fn read_requires_both_audience_and_context() {
        let channel = Uuid::from_u128(1);
        let domain = conversation(&[1, 2], channel, "v1");
        let wrong_audience = ResourceLabel::conversation(realm(), channel, readers(&[1]))
            .expect("non-empty readers");
        let wrong_context =
            ResourceLabel::conversation(realm(), Uuid::from_u128(2), readers(&[1, 2]))
                .expect("non-empty readers");

        assert!(!RuleEvaluator::read(&domain, &wrong_audience).allowed());
        assert!(!RuleEvaluator::read(&domain, &wrong_context).allowed());
        assert!(RuleEvaluator::read(&domain, &domain.resource_label()).allowed());
    }

    #[test]
    fn owner_private_context_aggregates_only_owner_readable_conversations() {
        let owner = principal(1);
        let domain = ExecutionDomain::owner_private(
            principal(9),
            realm(),
            owner,
            MembershipEpoch::new("owner-v1"),
            CapabilitySet::default(),
        );
        let readable = ResourceLabel::conversation(realm(), Uuid::from_u128(1), readers(&[1, 2]))
            .expect("non-empty readers");
        let unreadable = ResourceLabel::conversation(realm(), Uuid::from_u128(2), readers(&[2, 3]))
            .expect("non-empty readers");

        assert!(RuleEvaluator::read(&domain, &readable).allowed());
        assert!(!RuleEvaluator::read(&domain, &unreadable).allowed());
    }

    #[test]
    fn effective_capabilities_are_the_three_way_intersection() {
        let bot = CapabilitySet::from_names(["buzz.read.current", "email.read", "drive.read"]);
        let requester = CapabilitySet::from_names(["buzz.read.current", "email.read"]);
        let domain = CapabilitySet::from_names(["buzz.read.current", "drive.read"]);

        assert_eq!(
            CapabilitySet::effective(&bot, &requester, &domain),
            CapabilitySet::from_names(["buzz.read.current"])
        );
    }

    #[test]
    fn domain_derivation_grants_personal_tools_only_in_owner_private_work() {
        let owner = principal(1);
        let agent = principal(9);
        let policy = CapabilityPolicy::new(
            CapabilitySet::from_names(["buzz.read.current", "email.read"]),
            CapabilitySet::from_names(["buzz.read.current"]),
        );
        let owner_dm = derive_execution_domain(
            DomainFacts {
                realm: realm(),
                channel_id: Uuid::from_u128(1),
                kind: ConversationKind::DirectMessage,
                epoch: MembershipEpoch::new("membership:v1"),
                members: BTreeSet::from([agent.clone(), owner.clone()]),
                executing_agent: agent.clone(),
                requesters: BTreeSet::from([owner.clone()]),
                system_principal: None,
                owner: Some(owner.clone()),
            },
            &policy,
        )
        .expect("owner DM domain");
        assert!(owner_dm.context().is_owner_private_for(&owner));
        assert!(owner_dm.capabilities().contains("email.read"));

        let public = derive_execution_domain(
            DomainFacts {
                realm: realm(),
                channel_id: Uuid::from_u128(2),
                kind: ConversationKind::Public,
                epoch: MembershipEpoch::new("community:v1"),
                members: BTreeSet::new(),
                executing_agent: agent,
                requesters: BTreeSet::from([owner.clone()]),
                system_principal: None,
                owner: Some(owner),
            },
            &policy,
        )
        .expect("public domain");
        assert!(!public.capabilities().contains("email.read"));
    }

    #[test]
    fn restricted_domain_derivation_checks_agent_and_requester_membership() {
        let owner = principal(1);
        let agent = principal(9);
        let outsider = principal(8);
        let policy = CapabilityPolicy::new(
            CapabilitySet::from_names(["buzz.read.current"]),
            CapabilitySet::from_names(["buzz.read.current"]),
        );
        let facts = |members, requesters| DomainFacts {
            realm: realm(),
            channel_id: Uuid::from_u128(1),
            kind: ConversationKind::Restricted,
            epoch: MembershipEpoch::new("membership:v1"),
            members,
            executing_agent: agent.clone(),
            requesters,
            system_principal: None,
            owner: Some(owner.clone()),
        };

        assert_eq!(
            derive_execution_domain(
                facts(
                    BTreeSet::from([owner.clone()]),
                    BTreeSet::from([owner.clone()]),
                ),
                &policy,
            ),
            Err(DerivationError::AgentNotMember)
        );
        assert_eq!(
            derive_execution_domain(
                facts(
                    BTreeSet::from([owner.clone(), agent.clone()]),
                    BTreeSet::from([outsider]),
                ),
                &policy,
            ),
            Err(DerivationError::RequesterNotMember)
        );
    }

    #[test]
    fn process_reuse_requires_the_complete_domain() {
        let first = conversation(&[1, 2], Uuid::from_u128(1), "v1");
        let same = first.clone();
        let changed_epoch = conversation(&[1, 2], Uuid::from_u128(1), "v2");
        let mut state = ProcessState::default();

        assert!(state.enter(&first).allowed());
        assert!(state.enter(&same).allowed());
        assert!(!state.enter(&changed_epoch).allowed());
        assert!(!state.enter(&first).allowed());
        state.observe(&first.resource_label());
        assert!(!state
            .publish(&first, first.audience(), first.context(), &[1; 32], None)
            .allowed());
    }

    #[test]
    fn domain_id_has_a_canonical_golden_value() {
        let domain = conversation(&[1, 2], Uuid::from_u128(1), "membership:event-1");
        assert_eq!(
            domain.id(),
            "2ada28a5b33888f827a5845fd6e84b37e954a31dd4d0652d2468015d68e9080a"
        );
    }

    #[test]
    fn domain_ids_bind_the_managed_agent_identity() {
        let first = conversation(&[1, 2], Uuid::from_u128(1), "membership:event-1");
        let second = ExecutionDomain::new(
            principal(8),
            first.audience().clone(),
            first.context().clone(),
            MembershipEpoch::new("membership:event-1"),
            first.capabilities().clone(),
        )
        .expect("same realm");

        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn domain_shape_rejects_a_public_audience_for_a_private_context() {
        let result = ExecutionDomain::new(
            principal(9),
            ConfidentialityLabel::public(realm()),
            DomainContext::Conversation {
                realm: realm(),
                channel_id: Uuid::from_u128(1),
            },
            MembershipEpoch::new("v1"),
            CapabilitySet::default(),
        );

        assert_eq!(result, Err(DomainError::AudienceContextMismatch));
    }

    #[test]
    fn public_domain_ids_do_not_depend_on_the_triggering_channel() {
        let first = ExecutionDomain::public(
            principal(9),
            realm(),
            MembershipEpoch::new("community"),
            CapabilitySet::from_names(["buzz.read.current"]),
        );
        let second = ExecutionDomain::public(
            principal(9),
            realm(),
            MembershipEpoch::new("community"),
            CapabilitySet::from_names(["buzz.read.current"]),
        );

        assert_eq!(first.id(), second.id());
    }

    #[test]
    fn compartment_profile_is_asymmetric() {
        let public = ExecutionDomain::public(
            principal(9),
            realm(),
            MembershipEpoch::new("community"),
            CapabilitySet::default(),
        );
        let restricted = conversation(&[1, 2], Uuid::from_u128(1), "membership:v1");
        let owner_private = ExecutionDomain::owner_private(
            principal(9),
            realm(),
            principal(1),
            MembershipEpoch::new("owner:v1"),
            CapabilitySet::default(),
        );

        assert_eq!(
            public.compartment_profile(),
            CompartmentProfile::SharedPublic
        );
        assert_eq!(
            restricted.compartment_profile(),
            CompartmentProfile::DomainConfined
        );
        assert_eq!(
            owner_private.compartment_profile(),
            CompartmentProfile::DomainConfined
        );
    }

    #[test]
    fn denied_input_still_taints_an_audit_only_process() {
        let domain = conversation(&[1, 2], Uuid::from_u128(1), "v1");
        let private = ResourceLabel::owner_private(realm(), principal(1));
        assert!(!RuleEvaluator::read(&domain, &private).allowed());

        let mut state = ProcessState::default();
        state.enter(&domain);
        state.observe(&domain.resource_label());
        state.observe(&private);

        assert!(!state
            .publish(&domain, domain.audience(), domain.context(), &[7; 32], None,)
            .allowed());
    }

    #[test]
    fn ordinary_publication_stays_in_the_source_context() {
        let domain = conversation(&[1, 2], Uuid::from_u128(1), "v1");
        let other_context = DomainContext::Conversation {
            realm: realm(),
            channel_id: Uuid::from_u128(2),
        };
        let mut state = ProcessState::default();
        state.enter(&domain);
        state.observe(&domain.resource_label());

        assert!(!state
            .publish(&domain, domain.audience(), &other_context, &[7; 32], None)
            .allowed());
    }

    #[test]
    fn equal_reader_sets_do_not_hide_cross_context_input() {
        let domain = conversation(&[1, 2], Uuid::from_u128(1), "v1");
        let other = ResourceLabel::conversation(realm(), Uuid::from_u128(2), readers(&[1, 2]))
            .expect("non-empty readers");
        let mut state = ProcessState::default();
        state.enter(&domain);
        state.observe(&domain.resource_label());
        state.observe(&other);

        assert!(!state
            .publish(&domain, domain.audience(), domain.context(), &[7; 32], None)
            .allowed());
    }

    struct AlwaysValid;

    impl GrantSignatureVerifier for AlwaysValid {
        fn verifies(&self, _grant: &DeclassificationGrant<PendingGrant>) -> bool {
            true
        }
    }

    #[test]
    fn declassification_is_owner_verified_and_exact() {
        let owner = principal(1);
        let domain = conversation(&[1], Uuid::from_u128(1), "v1");
        let destination = ConfidentialityLabel::public(realm());
        let destination_context = DomainContext::RealmPublic(realm());
        let content = [9; 32];
        let mut grant = DeclassificationGrant::pending(
            owner.clone(),
            domain.id(),
            destination.clone(),
            destination_context.clone(),
            content,
        )
        .verify(&owner, &AlwaysValid)
        .expect("owner-authenticated grant");
        let mut state = ProcessState::default();
        state.enter(&domain);
        state.observe(&domain.resource_label());

        assert!(!state
            .publish(
                &domain,
                &destination,
                &destination_context,
                &[8; 32],
                Some(&mut grant),
            )
            .allowed());
        assert!(state
            .publish(
                &domain,
                &destination,
                &destination_context,
                &content,
                Some(&mut grant),
            )
            .allowed());
        assert!(!state
            .publish(
                &domain,
                &destination,
                &destination_context,
                &content,
                Some(&mut grant),
            )
            .allowed());
        state.mark_unknown();
        assert!(!state
            .publish(
                &domain,
                &destination,
                &destination_context,
                &content,
                Some(&mut grant),
            )
            .allowed());
    }

    #[test]
    fn unknown_input_prevents_publication() {
        let domain = conversation(&[1, 2], Uuid::from_u128(1), "v1");
        let mut state = ProcessState::default();
        state.enter(&domain);
        state.observe(&domain.resource_label());
        state.mark_unknown();

        assert!(!state
            .publish(&domain, domain.audience(), domain.context(), &[7; 32], None,)
            .allowed());
    }
}
