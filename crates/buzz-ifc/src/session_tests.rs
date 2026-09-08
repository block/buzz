use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use nostr::Keys;
use uuid::Uuid;

use super::*;

const READ: &str = "buzz.read.current";
const REPLY: &str = "buzz.reply";

fn principal(value: u8) -> Principal {
    let keys = Keys::parse(&format!("{value:064x}")).expect("test secret key");
    Principal::from_public_key(&keys.public_key()).expect("valid test principal")
}

fn community(value: u128) -> CommunityId {
    CommunityId::from_uuid(Uuid::from_u128(value))
}

fn policy() -> CapabilityPolicy {
    let operations = || {
        CapabilitySet::from_operations([
            (READ, OperationEffect::NonEgressing),
            (REPLY, OperationEffect::Publication),
        ])
    };
    CapabilityPolicy::new(operations(), operations())
}

fn public_domain(community: CommunityId, channel: u128, epoch: &str) -> ExecutionDomain {
    derive_execution_domain(
        DomainFacts {
            community,
            channel_id: Uuid::from_u128(channel),
            kind: ConversationKind::Public,
            epoch: MembershipEpoch::new(epoch),
            members: BTreeSet::new(),
            executing_agent: principal(9),
            requesters: BTreeSet::from([principal(1)]),
            system_principal: None,
            owner: Some(principal(1)),
        },
        &policy(),
    )
    .expect("valid public domain")
}

fn restricted_domain(
    community: CommunityId,
    channel: u128,
    epoch: &str,
    readers: &[u8],
) -> ExecutionDomain {
    let agent = principal(9);
    let members = readers
        .iter()
        .copied()
        .map(principal)
        .chain([agent])
        .collect();
    derive_execution_domain(
        DomainFacts {
            community,
            channel_id: Uuid::from_u128(channel),
            kind: ConversationKind::Restricted,
            epoch: MembershipEpoch::new(epoch),
            members,
            executing_agent: agent,
            requesters: BTreeSet::from([principal(readers[0])]),
            system_principal: None,
            owner: Some(principal(1)),
        },
        &policy(),
    )
    .expect("valid restricted domain")
}

fn owner_private_domain(community: CommunityId, channel: u128, epoch: &str) -> ExecutionDomain {
    let owner = principal(1);
    let agent = principal(9);
    derive_execution_domain(
        DomainFacts {
            community,
            channel_id: Uuid::from_u128(channel),
            kind: ConversationKind::DirectMessage,
            epoch: MembershipEpoch::new(epoch),
            members: BTreeSet::from([agent, owner]),
            executing_agent: agent,
            requesters: BTreeSet::from([owner]),
            system_principal: None,
            owner: Some(owner),
        },
        &policy(),
    )
    .expect("valid owner-private domain")
}

fn deliver_to_agent(
    session: &IfcSession,
    label: &ResourceLabel,
    value: &str,
    inbox: &mut Vec<String>,
) -> Result<(), IfcError> {
    session.read(label)?;
    inbox.push(value.to_owned());
    Ok(())
}

fn execute_publication(
    authorization: AuthorizedPublication,
    sink_log: &mut Vec<(String, ConfidentialityLabel, Vec<u8>)>,
) {
    sink_log.push(authorization.into_parts());
}

/// The broker checks a resource before delivery. The sink receives the checked
/// operation, audience, and bytes together, including the concrete destination.
#[test]
fn broker_turn_uses_one_small_checked_surface() {
    let domain = restricted_domain(community(1), 10, "membership:v1", &[1, 2]);
    let resource = ResourceLabel::from_domain(&domain);
    let destination = domain.audience().clone();
    let session = IfcSession::enter(domain);
    let mut inbox = Vec::new();
    let mut sink_log = Vec::new();
    let request = br#"{"channel":10,"text":"answer"}"#.to_vec();

    session
        .call(READ)
        .expect("read operation cannot publish information");
    deliver_to_agent(&session, &resource, "question", &mut inbox)
        .expect("broker may deliver the current conversation");
    let authorization = session
        .publish(REPLY, &destination, request.clone())
        .expect("reply may flow to the current audience");
    execute_publication(authorization, &mut sink_log);

    assert_eq!(inbox, ["question"]);
    assert_eq!(sink_log, [(REPLY.to_owned(), destination, request)]);
}

/// Shared request state can change while a publication waits for its sink.
/// Those changes must not alter an existing authorization, even if the session
/// has since received unknown input and can no longer authorize new requests.
#[test]
fn publication_keeps_the_checked_bytes_operation_and_destination() {
    let domain = public_domain(community(1), 10, "community:v1");
    let mut destination = domain.audience().clone();
    let checked_destination = destination.clone();
    let mut session = IfcSession::enter(domain);
    let mut operation = REPLY.to_owned();
    let original = br#"{"channel":10,"text":"public answer"}"#.to_vec();
    let shared_request = Rc::new(RefCell::new(original.clone()));
    let writer = Rc::clone(&shared_request);
    let authorization = session
        .publish(&operation, &destination, shared_request.borrow().clone())
        .expect("authorize an owned snapshot of the request");

    session.mark_unknown_input();
    *writer.borrow_mut() = br#"{"channel":20,"text":"unknown secret"}"#.to_vec();
    operation.clear();
    destination = ConfidentialityLabel::public(community(2));

    assert_eq!(
        session
            .publish(REPLY, &checked_destination, shared_request.borrow().clone())
            .err(),
        Some(IfcError::InformationFlow(
            ifc_core::EgressError::UnresolvedInput
        ))
    );
    assert_ne!(destination, checked_destination);
    assert_ne!(operation, authorization.operation());
    assert_eq!(authorization.operation(), REPLY);
    assert_eq!(authorization.payload(), original);
    let mut sink_log = Vec::new();
    execute_publication(authorization, &mut sink_log);
    assert_eq!(
        sink_log,
        [(REPLY.to_owned(), checked_destination, original)]
    );
}

/// A rejected read must never reach the agent and must not taint the session.
/// This binds the test to the broker seam: `read` runs before the value is
/// appended to the simulated agent inbox.
#[test]
fn broker_does_not_deliver_a_resource_with_a_narrower_audience() {
    let group = restricted_domain(community(1), 10, "membership:v1", &[1, 2]);
    let alice_only = restricted_domain(community(1), 20, "membership:v1", &[1]);
    let resource = ResourceLabel::from_domain(&alice_only);
    let destination = group.audience().clone();
    let session = IfcSession::enter(group);
    let mut inbox = Vec::new();

    assert_eq!(
        deliver_to_agent(&session, &resource, "alice secret", &mut inbox),
        Err(IfcError::ReadAudienceDenied)
    );
    assert!(inbox.is_empty());
    assert!(session
        .publish(REPLY, &destination, b"safe".to_vec())
        .is_ok());
}

/// Publication operations must not enter through `call`, which performs no
/// destination-flow check. Misclassifying this path would bypass IFC entirely.
#[test]
fn egressing_operation_cannot_use_the_call_path() {
    let session = IfcSession::enter(public_domain(community(1), 10, "community:v1"));

    assert_eq!(
        session.call(REPLY),
        Err(IfcError::PublicationRequiresPublish)
    );
}

/// The inverse mismatch is also rejected so every operation has one obvious
/// broker API and policy cannot silently change how a call is executed.
#[test]
fn non_egressing_operation_cannot_use_the_publish_path() {
    let domain = public_domain(community(1), 10, "community:v1");
    let destination = domain.audience().clone();
    let session = IfcSession::enter(domain);

    assert!(matches!(
        session.publish(READ, &destination, b"payload".to_vec()),
        Err(IfcError::NonEgressingRequiresCall)
    ));
}

/// Capability admission fails closed on both paths. An operation name supplied
/// by an agent cannot become authority merely because the broker recognizes
/// how to execute it.
#[test]
fn operation_absent_from_the_domain_is_denied() {
    let domain = public_domain(community(1), 10, "community:v1");
    let destination = domain.audience().clone();
    let session = IfcSession::enter(domain);

    assert_eq!(session.call("email.send"), Err(IfcError::CapabilityDenied));
    assert!(matches!(
        session.publish("email.send", &destination, b"payload".to_vec()),
        Err(IfcError::CapabilityDenied)
    ));
}

/// Accumulated private state must not be widened to a public audience. This is
/// the central no-write-down confidentiality invariant at the checked sink.
#[test]
fn private_session_cannot_publish_to_a_public_audience() {
    let private = restricted_domain(community(1), 10, "membership:v1", &[1, 2]);
    let public = public_domain(community(1), 20, "community:v1");
    let destination = public.audience();
    let session = IfcSession::enter(private);

    assert_eq!(
        session
            .publish(REPLY, destination, b"secret".to_vec())
            .err(),
        Some(IfcError::InformationFlow(
            ifc_core::EgressError::DestinationWidensReaders
        ))
    );
}

/// Public information may be sent to fewer readers. Reversing this ordering is
/// an easy reader-set lattice bug that would reject safe confidentiality
/// narrowing while potentially allowing the unsafe direction above.
#[test]
fn public_session_may_publish_to_a_private_audience() {
    let public = public_domain(community(1), 10, "community:v1");
    // A destination needs an audience, not an agent execution domain.
    let destination = ConfidentialityLabel::restricted_to(community(1), principal(1));
    let session = IfcSession::enter(public);

    assert!(session
        .publish(REPLY, &destination, b"public data".to_vec())
        .is_ok());
}

/// An unknown input permanently poisons ordinary egress. A later labeled read
/// must not reset the flag and accidentally launder unknown data.
#[test]
fn unknown_input_permanently_blocks_publication() {
    let domain = public_domain(community(1), 10, "community:v1");
    let resource = ResourceLabel::from_domain(&domain);
    let destination = domain.audience().clone();
    let mut session = IfcSession::enter(domain);

    session.mark_unknown_input();
    session
        .read(&resource)
        .expect("a later labeled read is still admissible");
    assert_eq!(
        session
            .publish(REPLY, &destination, b"output".to_vec())
            .err(),
        Some(IfcError::InformationFlow(
            ifc_core::EgressError::UnresolvedInput
        ))
    );
}

/// Retained restricted state from an older membership snapshot cannot enter a
/// newly routed session for the same conversation.
#[test]
fn same_conversation_rejects_a_stale_membership_epoch() {
    let old = restricted_domain(community(1), 10, "membership:v1", &[1, 2]);
    let current = restricted_domain(community(1), 10, "membership:v2", &[1, 2]);
    let resource = ResourceLabel::from_domain(&old);
    let session = IfcSession::enter(current);

    assert_eq!(session.read(&resource), Err(IfcError::StaleResourceEpoch));
}

/// Public community data intentionally has no conversation membership epoch,
/// so a restricted session may read it without comparing unrelated epochs.
/// This catches the earlier design bug where public data inherited an epoch
/// and was rejected by every private domain with a different epoch.
/// Reading that data must not make the session's private state public.
#[test]
fn private_session_may_read_public_data_from_its_community() {
    let public = public_domain(community(1), 20, "community:v7");
    let private = restricted_domain(community(1), 10, "membership:v2", &[1, 2]);
    let resource = ResourceLabel::from_domain(&public);
    let private_audience = private.audience().clone();
    let session = IfcSession::enter(private);

    assert_eq!(session.read(&resource), Ok(()));
    assert_eq!(
        session
            .publish(REPLY, public.audience(), b"private state".to_vec())
            .err(),
        Some(IfcError::InformationFlow(
            ifc_core::EgressError::DestinationWidensReaders
        ))
    );
    assert!(session
        .publish(REPLY, &private_audience, b"private state".to_vec())
        .is_ok());
}

/// Owner-private work may explicitly import conversation data when the owner
/// is an authorized reader. Its output is narrowed to the owner, while another
/// owner's private state remains protected by the exact-context rule.
#[test]
fn owner_private_session_may_read_conversation_data_safe_for_its_owner() {
    let conversation = restricted_domain(community(1), 20, "membership:v7", &[1, 2]);
    let owner_private = owner_private_domain(community(1), 10, "membership:v2");
    let resource = ResourceLabel::from_domain(&conversation);
    let session = IfcSession::enter(owner_private);

    assert_eq!(session.read(&resource), Ok(()));
}

/// Equal reader sets do not make two conversations the same retained-state
/// context. Otherwise one private channel could inject history into another.
#[test]
fn equal_audiences_do_not_merge_restricted_conversation_contexts() {
    let source = restricted_domain(community(1), 10, "membership:v1", &[1, 2]);
    let destination = restricted_domain(community(1), 20, "membership:v1", &[1, 2]);
    let resource = ResourceLabel::from_domain(&source);
    let session = IfcSession::enter(destination);

    assert_eq!(session.read(&resource), Err(IfcError::ReadContextDenied));
}

/// Universes are isolated even when two communities happen to contain the
/// same principals. A target in another community is never a valid IFC sink.
#[test]
fn publication_cannot_cross_communities() {
    let source = public_domain(community(1), 10, "community:v1");
    let destination = public_domain(community(2), 10, "community:v1");
    let session = IfcSession::enter(source);

    assert_eq!(
        session
            .publish(REPLY, destination.audience(), b"output".to_vec())
            .err(),
        Some(IfcError::InformationFlow(
            ifc_core::EgressError::DestinationUniverseMismatch
        ))
    );
}

/// This models the broker's retained-session pool. Public channels share one
/// community domain, while restricted channel identity and membership epoch
/// each select different state.
#[test]
fn broker_routes_retained_sessions_by_complete_domain_key() {
    let public_a = public_domain(community(1), 10, "community:v1");
    let public_b = public_domain(community(1), 20, "community:v1");
    let restricted_a = restricted_domain(community(1), 10, "membership:v1", &[1, 2]);
    let restricted_b = restricted_domain(community(1), 20, "membership:v1", &[1, 2]);
    let restricted_new_epoch = restricted_domain(community(1), 10, "membership:v2", &[1, 2]);
    let public_key = public_a.key();
    let domains = [
        public_a,
        public_b,
        restricted_a,
        restricted_b,
        restricted_new_epoch,
    ];
    let mut pool = HashMap::new();

    for domain in domains {
        pool.entry(domain.key())
            .or_insert_with(|| IfcSession::enter(domain));
    }

    assert_eq!(pool.len(), 4);
    assert_eq!(
        pool.get(&public_key).map(IfcSession::domain_key),
        Some(public_key)
    );
}
