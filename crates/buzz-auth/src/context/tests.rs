use super::*;
use buzz_core::CommunityId;
use nostr::Keys;

fn tenant(value: u128) -> TenantContext {
    TenantContext::resolved(authorization_domain(value), "relay.example")
}

fn authorization_domain(value: u128) -> CommunityId {
    CommunityId::from_uuid(Uuid::from_u128(value))
}

fn principal() -> FederatedPrincipal {
    FederatedPrincipal::new("https://idp.example", "subject-123")
        .expect("synthetic principal is valid")
}

fn assertion(
    principal: FederatedPrincipal,
    transport: AssertionTransport,
    expiry: u64,
) -> VerifiedFederatedAssertion {
    let authorized_transport = match transport {
        AssertionTransport::TrustedProxy => AuthTransport::RelayWebSocket,
        AssertionTransport::ClientAttached => AuthTransport::HttpBridge,
    };
    assertion_in(1, authorized_transport, principal, transport, expiry)
}

fn assertion_in(
    domain: u128,
    authorized_transport: AuthTransport,
    principal: FederatedPrincipal,
    transport: AssertionTransport,
    expiry: u64,
) -> VerifiedFederatedAssertion {
    assertion_with_bounds_in(
        domain,
        authorized_transport,
        principal,
        transport,
        None,
        expiry,
    )
}

fn assertion_with_bounds_in(
    domain: u128,
    authorized_transport: AuthTransport,
    principal: FederatedPrincipal,
    transport: AssertionTransport,
    not_before: Option<u64>,
    expiry: u64,
) -> VerifiedFederatedAssertion {
    VerifiedFederatedAssertion::new(
        authorization_domain(domain),
        authorized_transport,
        principal,
        None,
        transport,
        not_before.map(AssertionNotBefore::new),
        AssertionExpiry::new(expiry).expect("synthetic assertion expiry is valid"),
    )
}

fn assertion_with_attested_key(
    principal: FederatedPrincipal,
    transport: AssertionTransport,
    expiry: u64,
    attested_pubkey: PublicKey,
) -> VerifiedFederatedAssertion {
    let authorized_transport = match transport {
        AssertionTransport::TrustedProxy => AuthTransport::RelayWebSocket,
        AssertionTransport::ClientAttached => AuthTransport::HttpBridge,
    };
    VerifiedFederatedAssertion::new(
        authorization_domain(1),
        authorized_transport,
        principal,
        Some(VerifiedKeyAttestation::new(attested_pubkey)),
        transport,
        None,
        AssertionExpiry::new(expiry).expect("synthetic assertion expiry is valid"),
    )
}

fn policy_not_required() -> ResolvedFederatedPolicy {
    ResolvedFederatedPolicy::not_required(authorization_domain(1))
}

fn policy_required(enrollment_mode: EnrollmentMode) -> ResolvedFederatedPolicy {
    ResolvedFederatedPolicy::required(authorization_domain(1), enrollment_mode)
}

fn policy_with_lineage(
    enrollment_mode: EnrollmentMode,
    correlation_id: Uuid,
    effective_from: u64,
    effective_until: u64,
) -> ResolvedFederatedPolicy {
    ResolvedFederatedPolicy::from_authoritative_resolution(
        FederatedPolicyStamp::from_authoritative_state(
            authorization_domain(1),
            Uuid::from_u128(40),
            7,
            correlation_id,
            FederatedIdentityRequirement::Required(enrollment_mode),
            effective_from,
            effective_until,
        )
        .expect("synthetic federated policy lineage is valid"),
    )
}

fn binding(pubkey: PublicKey) -> VersionedBindingRef {
    binding_in(1, pubkey)
}

fn authoritative_binding_evidence(
    pubkey: PublicKey,
    source: BindingSource,
) -> AuthoritativeBindingEvidence {
    AuthoritativeBindingEvidence::new(
        authorization_domain(1),
        Uuid::from_u128(10),
        principal(),
        pubkey,
        BindingVersion::INITIAL,
        None,
        source,
    )
    .expect("synthetic authoritative binding evidence is valid")
}

struct TestAuthorityAdapter;

impl FederatedAuthorityAdapter for TestAuthorityAdapter {
    type Error = &'static str;

    fn resolve_current_policy<'a>(
        &'a self,
        request: CurrentPolicyRequest,
        sink: CurrentPolicyResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        Result<ResolvedFederatedPolicy, AuthorityAdapterError<Self::Error>>,
    > {
        Box::pin(async move {
            assert_eq!(request.authorization_domain(), authorization_domain(1));
            assert_eq!(request.correlation_id(), Uuid::from_u128(2));
            assert_eq!(request.observed_at(), 100);
            assert!(!format!("{request:?}").contains("100"));
            sink.resolved(
                request.authorization_domain(),
                Uuid::from_u128(40),
                7,
                FederatedIdentityRequirement::Required(EnrollmentMode::AttestedKey),
                90,
                200,
            )
            .map_err(AuthorityAdapterError::from)
        })
    }

    fn resolve_direct_binding<'a>(
        &'a self,
        request: BindingResolutionRequest,
        sink: DirectBindingResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        Result<AuthoritativeBindingResolution, AuthorityAdapterError<Self::Error>>,
    > {
        Box::pin(async move {
            assert_eq!(request.policy_id(), Uuid::from_u128(40));
            assert_eq!(request.policy_epoch(), 7);
            assert_eq!(
                request.policy_requirement(),
                FederatedIdentityRequirement::Required(EnrollmentMode::AttestedKey)
            );
            assert!(request.key_attested());
            assert_eq!(request.effective_from(), 90);
            assert_eq!(request.effective_until(), 180);
            assert_eq!(request.observed_at(), 100);
            assert!(!format!("{request:?}").contains("subject-123"));
            sink.atomically_enrolled(
                request.authorization_domain(),
                Uuid::from_u128(10),
                request.principal().clone(),
                request.bound_pubkey(),
                BindingVersion::INITIAL,
                Some(BindingExpiry::new(180).expect("synthetic binding expiry is valid")),
                BindingSource::AttestedKey,
            )
            .map_err(AuthorityAdapterError::from)
        })
    }

    fn resolve_existing_binding<'a>(
        &'a self,
        request: BindingResolutionRequest,
        sink: ExistingBindingResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        Result<AuthoritativeBindingResolution, AuthorityAdapterError<Self::Error>>,
    > {
        Box::pin(async move {
            sink.existing_active(
                request.authorization_domain(),
                Uuid::from_u128(10),
                request.principal().clone(),
                request.bound_pubkey(),
                BindingVersion::INITIAL,
                Some(BindingExpiry::new(180).expect("synthetic binding expiry is valid")),
                BindingSource::Provisioned,
            )
            .map_err(AuthorityAdapterError::from)
        })
    }
}

fn binding_in(domain: u128, pubkey: PublicKey) -> VersionedBindingRef {
    binding_with_source_in(domain, pubkey, BindingSource::AttestedKey)
}

fn binding_with_source_in(
    domain: u128,
    pubkey: PublicKey,
    source: BindingSource,
) -> VersionedBindingRef {
    VersionedBindingRef::new_existing_active_for_test(
        authorization_domain(domain),
        Uuid::from_u128(10),
        principal(),
        pubkey,
        BindingVersion::INITIAL,
        None,
        source,
    )
    .expect("synthetic binding identifier is valid")
}

fn enrolled_binding(
    pubkey: PublicKey,
    source: BindingSource,
    reason: AuthorizationReason,
) -> VersionedBindingRef {
    VersionedBindingRef::new_enrolled_active_for_test(
        authorization_domain(1),
        Uuid::from_u128(10),
        principal(),
        pubkey,
        BindingVersion::INITIAL,
        None,
        source,
        reason,
    )
    .expect("synthetic enrolled binding is valid")
}

fn expiring_binding(pubkey: PublicKey, expires_at: u64) -> VersionedBindingRef {
    VersionedBindingRef::new_existing_active_for_test(
        authorization_domain(1),
        Uuid::from_u128(10),
        principal(),
        pubkey,
        BindingVersion::INITIAL,
        Some(BindingExpiry::new(expires_at).expect("synthetic binding expiry is valid")),
        BindingSource::AttestedKey,
    )
    .expect("synthetic binding identifier is valid")
}

fn input(
    actor_pubkey: PublicKey,
    transport: AuthTransport,
    verified_owner_pubkey: Option<PublicKey>,
) -> AuthContextInput {
    input_with_delegation_expiry(actor_pubkey, transport, verified_owner_pubkey, 200)
}

fn input_with_delegation_expiry(
    actor_pubkey: PublicKey,
    transport: AuthTransport,
    verified_owner_pubkey: Option<PublicKey>,
    delegation_expiry: u64,
) -> AuthContextInput {
    let verified_delegation = verified_owner_pubkey.map(|owner_pubkey| {
        VerifiedTransportDelegation::new_unrestricted(
            owner_pubkey,
            actor_pubkey,
            Some(
                DelegationExpiry::new(delegation_expiry)
                    .expect("synthetic delegation expiry is valid"),
            ),
        )
        .expect("synthetic owner and delegate are distinct")
    });
    let proof_method = match transport {
        AuthTransport::RelayWebSocket | AuthTransport::Audio => AuthMethod::Nip42,
        _ => AuthMethod::Nip98,
    };
    AuthContextInput::new(
        tenant(1),
        Uuid::from_u128(2),
        VerifiedNostrProof::new(
            authorization_domain(1),
            transport,
            actor_pubkey,
            proof_method,
            verified_delegation,
        )
        .expect("synthetic Nostr proof is internally consistent"),
        AuthorizedCommunityAccess::new(authorization_domain(1), Scope::all_known(), None),
    )
}

fn proof_in(domain: u128, transport: AuthTransport, actor_pubkey: PublicKey) -> VerifiedNostrProof {
    let proof_method = match transport {
        AuthTransport::RelayWebSocket | AuthTransport::Audio => AuthMethod::Nip42,
        _ => AuthMethod::Nip98,
    };
    VerifiedNostrProof::new(
        authorization_domain(domain),
        transport,
        actor_pubkey,
        proof_method,
        None,
    )
    .expect("synthetic Nostr proof is valid")
}

fn community_access_in(domain: u128) -> AuthorizedCommunityAccess {
    AuthorizedCommunityAccess::new(authorization_domain(domain), Scope::all_known(), None)
}

fn delegated_authorization(
    domain: u128,
    owner_pubkey: PublicKey,
    admission_principal: FederatedPrincipal,
    admission_expiry: u64,
) -> FederatedAuthorization {
    FederatedAuthorization::Delegated {
        owner: binding_in(domain, owner_pubkey),
        admission: VerifiedOwnerAdmission::new(
            authorization_domain(domain),
            admission_principal,
            AdmissionExpiry::new(admission_expiry).expect("synthetic admission expiry is valid"),
        ),
    }
}

#[test]
fn context_preserves_server_resolved_authority() {
    let keys = Keys::generate();
    let correlation_id = Uuid::from_u128(2);
    let context = AuthContext::finalize_v1(
        AuthContextInput::new(
            tenant(1),
            correlation_id,
            VerifiedNostrProof::new(
                authorization_domain(1),
                AuthTransport::RelayWebSocket,
                keys.public_key(),
                AuthMethod::Nip42,
                None,
            )
            .expect("synthetic Nostr proof is valid"),
            AuthorizedCommunityAccess::new(
                authorization_domain(1),
                vec![Scope::MessagesRead],
                None,
            ),
        ),
        policy_not_required(),
        FederatedAuthorization::NotRequired,
        100,
    )
    .expect("Nostr-only policy is final authorization");

    assert_eq!(context.version(), AuthContextVersion::V1);
    assert_eq!(context.tenant().community().as_uuid(), &Uuid::from_u128(1));
    assert_eq!(context.correlation_id(), correlation_id);
    assert_eq!(context.transport(), AuthTransport::RelayWebSocket);
    assert_eq!(context.pubkey(), keys.public_key());
    assert_eq!(context.auth_method(), AuthMethod::Nip42);
    assert_eq!(
        context.federated_policy().requirement(),
        FederatedIdentityRequirement::NotRequired
    );
    assert!(context.has_scope(&Scope::MessagesRead));
    assert_eq!(
        context.federated_authorization(),
        &FederatedAuthorization::NotRequired
    );
}

#[test]
fn direct_authorization_requires_the_authenticated_key() {
    let actor = Keys::generate();
    let other = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: binding(other.public_key()),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect_err("a direct binding for another key must be rejected");
    assert_eq!(error, AuthContextError::DirectBindingKeyMismatch);
}

#[test]
fn delegated_authorization_requires_the_verified_owner() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let context = AuthContext::finalize_v1(
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        delegated_authorization(1, owner.public_key(), principal(), 200),
        100,
    )
    .expect("verified owner and delegate match");

    assert!(matches!(
        context.federated_authorization(),
        FederatedAuthorization::Delegated { .. }
    ));
}

#[test]
fn delegated_authorization_requires_verified_delegation() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        delegated_authorization(1, owner.public_key(), principal(), 200),
        100,
    )
    .expect_err("delegated authorization requires verifier-issued proof");

    assert_eq!(error, AuthContextError::DelegationRequired);
    assert_eq!(error.code(), "federated_delegation_required");
}

#[test]
fn principal_debug_output_redacts_claim_values() {
    let principal = FederatedPrincipal::new("https://idp.example", "subject-123")
        .expect("synthetic principal is valid");
    let output = format!("{principal:?}");

    assert_eq!(
        output,
        "FederatedPrincipal { issuer: \"[redacted]\", subject: \"[redacted]\" }"
    );
}

#[test]
fn principal_preserves_exact_validated_values() {
    let principal = FederatedPrincipal::new(" HTTPS://IDP.EXAMPLE/ ", " Subject-123 ")
        .expect("non-empty validated values are accepted exactly");

    assert_eq!(principal.issuer(), " HTTPS://IDP.EXAMPLE/ ");
    assert_eq!(principal.subject(), " Subject-123 ");
    assert_eq!(
        FederatedPrincipal::new("", "subject-123"),
        Err(AuthContextError::EmptyIssuer)
    );
    assert_eq!(
        FederatedPrincipal::new("https://idp.example", ""),
        Err(AuthContextError::EmptySubject)
    );
}

#[test]
fn context_debug_output_omits_tenant_host() {
    let actor = Keys::generate();
    let channel_id = Uuid::from_u128(20);
    let context = AuthContext::finalize_v1(
        AuthContextInput::new(
            tenant(1),
            Uuid::from_u128(2),
            VerifiedNostrProof::new(
                authorization_domain(1),
                AuthTransport::RelayWebSocket,
                actor.public_key(),
                AuthMethod::Nip42,
                None,
            )
            .expect("synthetic Nostr proof is valid"),
            AuthorizedCommunityAccess::new(
                authorization_domain(1),
                vec![Scope::MessagesRead],
                Some(vec![channel_id]),
            ),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect("matching direct authorization is valid");

    assert_eq!(
        format!("{context:?}"),
        concat!(
            "V1(AuthContextV1 { authorization_domain: \"[redacted]\", ",
            "correlation_id: \"[redacted]\", transport: RelayWebSocket, ",
            "nostr: NostrAuthority { actor_pubkey: \"[redacted]\", ",
            "proof_method: Nip42, verified_delegation: \"[redacted]\" }, ",
            "federated_policy: ResolvedFederatedPolicy { ",
            "stamp: FederatedPolicyStamp { authorization_domain: \"[redacted]\", ",
            "policy_id: \"[redacted]\", epoch: \"[redacted]\", ",
            "correlation_id: \"[redacted]\", requirement: \"[redacted]\", ",
            "effective_from: \"[redacted]\", effective_until: \"[redacted]\" } }, ",
            "federated: FederatedAuthorization(\"[redacted]\"), ",
            "scopes: \"[redacted]\", channel_ids: \"[redacted]\" })"
        )
    );
}

#[test]
fn direct_authorization_rejects_a_verified_owner() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect_err("direct authorization cannot derive authority from an owner");

    assert_eq!(error, AuthContextError::DirectAuthorizationHasOwner);
}

#[test]
fn delegated_authorization_requires_current_owner_admission() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        delegated_authorization(1, owner.public_key(), principal(), 100),
        100,
    )
    .expect_err("delegated authorization must not survive owner admission expiry");

    assert_eq!(error, AuthContextError::OwnerAdmissionExpired);
}

#[test]
fn delegated_authorization_rejects_cross_domain_owner_admission() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Delegated {
            owner: binding(owner.public_key()),
            admission: VerifiedOwnerAdmission::new(
                authorization_domain(2),
                principal(),
                AdmissionExpiry::new(200).expect("synthetic admission expiry is valid"),
            ),
        },
        100,
    )
    .expect_err("delegated owner admission cannot cross authorization domains");

    assert_eq!(error, AuthContextError::OwnerAdmissionDomainMismatch);
}

#[test]
fn delegated_authorization_requires_the_owner_admission_principal() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let admission_principal = FederatedPrincipal::new("https://idp.example", "other-subject")
        .expect("synthetic principal is valid");
    let error = AuthContext::finalize_v1(
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        delegated_authorization(1, owner.public_key(), admission_principal, 200),
        100,
    )
    .expect_err("current admission must identify the bound owner");

    assert_eq!(error, AuthContextError::OwnerAdmissionPrincipalMismatch);
}

#[test]
fn delegated_authorization_rejects_an_expired_proof() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let error = AuthContext::finalize_v1(
        input_with_delegation_expiry(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
            100,
        ),
        policy_required(EnrollmentMode::AttestedKey),
        delegated_authorization(1, owner.public_key(), principal(), 200),
        100,
    )
    .expect_err("delegated authorization must not survive delegation expiry");

    assert_eq!(error, AuthContextError::DelegationExpired);
}

#[test]
fn verified_nostr_proof_requires_the_authenticated_delegate() {
    let actor = Keys::generate();
    let other_delegate = Keys::generate();
    let owner = Keys::generate();
    let delegation = VerifiedTransportDelegation::new_unrestricted(
        owner.public_key(),
        other_delegate.public_key(),
        None,
    )
    .expect("synthetic owner and delegate are distinct");
    let error = VerifiedNostrProof::new(
        authorization_domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        Some(delegation),
    )
    .expect_err("verified proof must name the authenticated actor");

    assert_eq!(error, AuthContextError::DelegateKeyMismatch);
}

#[test]
fn delegated_authorization_requires_the_bound_owner() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let other_owner = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(other_owner.public_key()),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        delegated_authorization(1, owner.public_key(), principal(), 200),
        100,
    )
    .expect_err("delegated authorization must match the verified owner");

    assert_eq!(error, AuthContextError::DelegatedOwnerMismatch);
}

#[test]
fn delegated_binding_cannot_cross_authorization_domains() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        delegated_authorization(2, owner.public_key(), principal(), 200),
        100,
    )
    .expect_err("a delegated binding from another domain must be rejected");

    assert_eq!(error, AuthContextError::BindingDomainMismatch);
}

#[test]
fn zero_binding_version_is_rejected() {
    assert_eq!(
        BindingVersion::new(0),
        Err(AuthContextError::InvalidBindingVersion)
    );
}

#[test]
fn zero_binding_expiry_is_rejected() {
    assert_eq!(
        BindingExpiry::new(0),
        Err(AuthContextError::InvalidBindingExpiry)
    );
}

#[test]
fn nil_binding_identifier_is_rejected() {
    let actor = Keys::generate();
    let error = VersionedBindingRef::new_existing_active_for_test(
        CommunityId::from_uuid(Uuid::from_u128(1)),
        Uuid::nil(),
        principal(),
        actor.public_key(),
        BindingVersion::INITIAL,
        None,
        BindingSource::AttestedKey,
    )
    .expect_err("nil is not a stable binding identifier");

    assert_eq!(error, AuthContextError::InvalidBindingId);
    assert_eq!(error.code(), "federated_binding_invalid_id");
}

#[test]
fn evidence_value_debug_output_redacts_numeric_values() {
    let assertion_expiry = AssertionExpiry::new(200).expect("synthetic expiry is valid");
    let assertion_not_before = AssertionNotBefore::new(100);
    let delegation_expiry = DelegationExpiry::new(300).expect("synthetic expiry is valid");
    let admission_expiry = AdmissionExpiry::new(350).expect("synthetic expiry is valid");
    let binding_expiry = BindingExpiry::new(375).expect("synthetic expiry is valid");
    let binding_version = BindingVersion::new(400).expect("synthetic version is valid");

    assert_eq!(
        format!("{assertion_expiry:?}"),
        "AssertionExpiry(\"[redacted]\")"
    );
    assert_eq!(
        format!("{assertion_not_before:?}"),
        "AssertionNotBefore(\"[redacted]\")"
    );
    assert_eq!(
        format!("{delegation_expiry:?}"),
        "DelegationExpiry(\"[redacted]\")"
    );
    assert_eq!(
        format!("{admission_expiry:?}"),
        "AdmissionExpiry(\"[redacted]\")"
    );
    assert_eq!(
        format!("{binding_expiry:?}"),
        "BindingExpiry(\"[redacted]\")"
    );
    assert_eq!(
        format!("{binding_version:?}"),
        "BindingVersion(\"[redacted]\")"
    );
}

#[test]
fn authority_adapter_error_debug_output_redacts_storage_detail() {
    let error = AuthorityAdapterError::adapter("private-storage-detail");

    let rendered = format!("{error:?}");
    assert_eq!(
        rendered,
        "AuthorityAdapterError { variant: \"Adapter\", detail: \"[redacted]\" }"
    );
    assert!(!rendered.contains("private-storage-detail"));
}

#[test]
fn zero_owner_admission_expiry_is_rejected() {
    assert_eq!(
        AdmissionExpiry::new(0),
        Err(AuthContextError::InvalidAdmissionExpiry)
    );
}

#[test]
fn owner_admission_debug_output_is_fully_redacted() {
    let admission = VerifiedOwnerAdmission::new(
        authorization_domain(1),
        principal(),
        AdmissionExpiry::new(200).expect("synthetic admission expiry is valid"),
    );

    assert_eq!(
        format!("{admission:?}"),
        concat!(
            "VerifiedOwnerAdmission { authorization_domain: \"[redacted]\", ",
            "principal: FederatedPrincipal { issuer: \"[redacted]\", ",
            "subject: \"[redacted]\" }, fresh_until: \"[redacted]\" }"
        )
    );
}

#[test]
fn verified_assertion_debug_output_is_fully_redacted() {
    let actor = Keys::generate();
    let assertion = VerifiedFederatedAssertion::new(
        authorization_domain(1),
        AuthTransport::RelayWebSocket,
        principal(),
        Some(VerifiedKeyAttestation::new(actor.public_key())),
        AssertionTransport::TrustedProxy,
        Some(AssertionNotBefore::new(100)),
        AssertionExpiry::new(200).expect("synthetic assertion expiry is valid"),
    );

    assert_eq!(
        format!("{assertion:?}"),
        concat!(
            "VerifiedFederatedAssertion { authorization_domain: \"[redacted]\", ",
            "authorized_transport: RelayWebSocket, principal: FederatedPrincipal { ",
            "issuer: \"[redacted]\", subject: \"[redacted]\" }, ",
            "key_attestation: \"[redacted]\", transport: \"[redacted]\", ",
            "not_before: \"[redacted]\", expires_at: \"[redacted]\" }"
        )
    );
}

#[test]
fn direct_authorization_rejects_expired_assertions() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::HttpBridge, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion(principal(), AssertionTransport::ClientAttached, 100),
        },
        100,
    )
    .expect_err("authorization must not survive assertion expiry");

    assert_eq!(error, AuthContextError::AssertionExpired);
    assert_eq!(error.code(), "federated_assertion_expired");
}

#[test]
fn direct_authorization_rejects_binding_at_exact_expiry() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::HttpBridge, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: expiring_binding(actor.public_key(), 100),
            assertion: assertion(principal(), AssertionTransport::ClientAttached, 200),
        },
        100,
    )
    .expect_err("authorization must not survive binding expiry");

    assert_eq!(error, AuthContextError::BindingExpired);
    assert_eq!(error.code(), "federated_binding_expired");
}

#[test]
fn delegated_authorization_rejects_owner_binding_at_exact_expiry() {
    let owner = Keys::generate();
    let delegate = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(
            delegate.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Delegated {
            owner: expiring_binding(owner.public_key(), 100),
            admission: VerifiedOwnerAdmission::new(
                authorization_domain(1),
                principal(),
                AdmissionExpiry::new(200).expect("synthetic admission expiry is valid"),
            ),
        },
        100,
    )
    .expect_err("delegated authorization must not survive owner-binding expiry");

    assert_eq!(error, AuthContextError::BindingExpired);
}

#[test]
fn direct_authorization_rejects_a_future_assertion() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::HttpBridge, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion_with_bounds_in(
                1,
                AuthTransport::HttpBridge,
                principal(),
                AssertionTransport::ClientAttached,
                Some(101),
                200,
            ),
        },
        100,
    )
    .expect_err("authorization must enforce the assertion's not-before bound");

    assert_eq!(error, AuthContextError::AssertionNotYetValid);
    assert_eq!(error.code(), "federated_assertion_not_yet_valid");
}

#[test]
fn direct_authorization_requires_the_assertion_principal() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion(
                FederatedPrincipal::new("https://idp.example", "other-subject")
                    .expect("synthetic principal is valid"),
                AssertionTransport::TrustedProxy,
                200,
            ),
        },
        100,
    )
    .expect_err("the current assertion must identify the bound principal");

    assert_eq!(error, AuthContextError::AssertionPrincipalMismatch);
    assert_eq!(error.code(), "federated_assertion_principal_mismatch");
}

#[test]
fn enrolled_reason_must_match_policy_and_binding_source() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::Provisioned),
        FederatedAuthorization::Direct {
            binding: enrolled_binding(
                actor.public_key(),
                BindingSource::AttestedKey,
                AuthorizationReason::EnrolledAttestedKey,
            ),
            assertion: assertion_with_attested_key(
                principal(),
                AssertionTransport::TrustedProxy,
                200,
                actor.public_key(),
            ),
        },
        100,
    )
    .expect_err("provisioned mode cannot enroll during authorization");

    assert_eq!(error, AuthContextError::InvalidAuthorizationReason);
}

#[test]
fn existing_active_bindings_are_independent_of_enrollment_mode() {
    let actor = Keys::generate();
    let enrollment_modes = [
        EnrollmentMode::AttestedKey,
        EnrollmentMode::Provisioned,
        EnrollmentMode::Tofu,
    ];
    let binding_sources = [
        BindingSource::AttestedKey,
        BindingSource::Provisioned,
        BindingSource::Tofu,
    ];

    for enrollment_mode in enrollment_modes {
        for binding_source in binding_sources {
            let context = AuthContext::finalize_v1(
                input(actor.public_key(), AuthTransport::RelayWebSocket, None),
                policy_required(enrollment_mode),
                FederatedAuthorization::Direct {
                    binding: binding_with_source_in(1, actor.public_key(), binding_source),
                    assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
                },
                100,
            )
            .expect("enrollment mode governs new bindings, not existing active bindings");

            assert_eq!(
                context.authorization_reason(),
                AuthorizationReason::ExistingBinding
            );
        }
    }
}

#[test]
fn binding_lifecycle_result_owns_the_authorization_reason() {
    let actor = Keys::generate();
    let existing = binding(actor.public_key());
    assert_eq!(
        existing.authorization_reason(),
        AuthorizationReason::ExistingBinding
    );

    let enrolled = enrolled_binding(
        actor.public_key(),
        BindingSource::AttestedKey,
        AuthorizationReason::EnrolledAttestedKey,
    );
    assert_eq!(
        enrolled.authorization_reason(),
        AuthorizationReason::EnrolledAttestedKey
    );

    let error = VersionedBindingRef::new_enrolled_active_for_test(
        authorization_domain(1),
        Uuid::from_u128(10),
        principal(),
        actor.public_key(),
        BindingVersion::INITIAL,
        None,
        BindingSource::AttestedKey,
        AuthorizationReason::ExistingBinding,
    )
    .expect_err("fresh enrollment cannot be relabeled as an existing binding");
    assert_eq!(error, AuthContextError::InvalidAuthorizationReason);
}

#[test]
fn attested_enrollment_requires_matching_verified_key_evidence() {
    let actor = Keys::generate();
    let other = Keys::generate();

    let missing = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: enrolled_binding(
                actor.public_key(),
                BindingSource::AttestedKey,
                AuthorizationReason::EnrolledAttestedKey,
            ),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect_err("attested enrollment cannot silently accept a missing key claim");
    assert_eq!(missing, AuthContextError::KeyAttestationRequired);

    let mismatch = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: enrolled_binding(
                actor.public_key(),
                BindingSource::AttestedKey,
                AuthorizationReason::EnrolledAttestedKey,
            ),
            assertion: assertion_with_attested_key(
                principal(),
                AssertionTransport::TrustedProxy,
                200,
                other.public_key(),
            ),
        },
        100,
    )
    .expect_err("attested enrollment cannot accept another Nostr key");
    assert_eq!(mismatch, AuthContextError::KeyAttestationMismatch);

    let context = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: enrolled_binding(
                actor.public_key(),
                BindingSource::AttestedKey,
                AuthorizationReason::EnrolledAttestedKey,
            ),
            assertion: assertion_with_attested_key(
                principal(),
                AssertionTransport::TrustedProxy,
                200,
                actor.public_key(),
            ),
        },
        100,
    )
    .expect("matching verified key evidence permits attested enrollment");
    assert_eq!(
        context.authorization_reason(),
        AuthorizationReason::EnrolledAttestedKey
    );
}

#[test]
fn present_key_attestation_never_ignores_an_actor_mismatch() {
    let actor = Keys::generate();
    let other = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::Tofu),
        FederatedAuthorization::Direct {
            binding: binding_with_source_in(1, actor.public_key(), BindingSource::Tofu),
            assertion: assertion_with_attested_key(
                principal(),
                AssertionTransport::TrustedProxy,
                200,
                other.public_key(),
            ),
        },
        100,
    )
    .expect_err("a present but mismatched key claim must never be ignored");

    assert_eq!(error, AuthContextError::KeyAttestationMismatch);
}

#[test]
fn tofu_enrollment_uses_tofu_reason_with_attested_provenance() {
    let actor = Keys::generate();
    let authorization = FederatedAuthorization::Direct {
        binding: enrolled_binding(
            actor.public_key(),
            BindingSource::AttestedKey,
            AuthorizationReason::EnrolledTofu,
        ),
        assertion: assertion_with_attested_key(
            principal(),
            AssertionTransport::TrustedProxy,
            200,
            actor.public_key(),
        ),
    };

    let context = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::Tofu),
        authorization,
        100,
    )
    .expect("TOFU policy may retain stronger attested-key provenance");

    assert_eq!(
        context.authorization_reason(),
        AuthorizationReason::EnrolledTofu
    );
}

#[test]
fn authorization_reason_codes_are_unique_and_redaction_safe() {
    let reasons = [
        AuthorizationReason::NostrOnly,
        AuthorizationReason::ExistingBinding,
        AuthorizationReason::EnrolledAttestedKey,
        AuthorizationReason::EnrolledTofu,
        AuthorizationReason::DelegatedOwnerBinding,
    ];
    let mut codes = reasons
        .iter()
        .copied()
        .map(AuthorizationReason::code)
        .collect::<Vec<_>>();
    codes.sort_unstable();
    codes.dedup();

    assert_eq!(codes.len(), reasons.len());
    for code in codes {
        assert!(code.starts_with("authorization_allow_"));
        assert!(!code.contains("tofu"));
        assert!(!code.contains("attest"));
        assert!(!code.contains("provision"));
        assert!(!code.contains("provider"));
    }
}

#[test]
fn authorization_error_codes_are_unique_and_provider_neutral() {
    let errors = [
        AuthContextError::EmptyIssuer,
        AuthContextError::EmptySubject,
        AuthContextError::InvalidBindingVersion,
        AuthContextError::InvalidBindingId,
        AuthContextError::InvalidBindingExpiry,
        AuthContextError::InvalidFederatedPolicyId,
        AuthContextError::InvalidFederatedPolicyEpoch,
        AuthContextError::InvalidFederatedPolicyCorrelation,
        AuthContextError::InvalidFederatedPolicyInterval,
        AuthContextError::InvalidAssertionExpiry,
        AuthContextError::InvalidDelegationExpiry,
        AuthContextError::InvalidAdmissionExpiry,
        AuthContextError::AssertionExpired,
        AuthContextError::BindingExpired,
        AuthContextError::FederatedPolicyCorrelationMismatch,
        AuthContextError::FederatedPolicyNotYetEffective,
        AuthContextError::FederatedPolicyExpired,
        AuthContextError::AssertionNotYetValid,
        AuthContextError::KeyAttestationRequired,
        AuthContextError::KeyAttestationMismatch,
        AuthContextError::OwnerAdmissionExpired,
        AuthContextError::FederatedIdentityRequired,
        AuthContextError::UnexpectedFederatedAuthorization,
        AuthContextError::DelegationExpired,
        AuthContextError::SelfDelegation,
        AuthContextError::InvalidAuthorizationReason,
        AuthContextError::BindingDomainMismatch,
        AuthContextError::NostrProofDomainMismatch,
        AuthContextError::PolicyDomainMismatch,
        AuthContextError::CommunityAccessDomainMismatch,
        AuthContextError::AssertionDomainMismatch,
        AuthContextError::AssertionTransportMismatch,
        AuthContextError::OwnerAdmissionDomainMismatch,
        AuthContextError::OwnerAdmissionPrincipalMismatch,
        AuthContextError::AssertionPrincipalMismatch,
        AuthContextError::TransportProofMismatch,
        AuthContextError::DirectAuthorizationHasOwner,
        AuthContextError::DirectBindingKeyMismatch,
        AuthContextError::DelegateKeyMismatch,
        AuthContextError::DelegationRequired,
        AuthContextError::DelegatedOwnerMismatch,
        AuthContextError::DelegatedBindingNotExistingActive,
    ];
    let mut codes = errors
        .iter()
        .copied()
        .map(AuthContextError::code)
        .collect::<Vec<_>>();
    codes.sort_unstable();
    codes.dedup();

    assert_eq!(codes.len(), errors.len());
    for code in codes {
        assert!(!code.contains("registry"));
        assert!(!code.contains("proxy"));
        assert!(!code.contains("role"));
        assert!(!code.contains("group"));
    }
}

#[test]
fn security_posture_debug_output_is_fully_redacted() {
    assert_eq!(
        format!("{:?}", EnrollmentMode::AttestedKey),
        "EnrollmentMode(\"[redacted]\")"
    );
    assert_eq!(
        format!(
            "{:?}",
            FederatedIdentityRequirement::Required(EnrollmentMode::Tofu)
        ),
        "FederatedIdentityRequirement(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", BindingSource::Provisioned),
        "BindingSource(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", AssertionTransport::TrustedProxy),
        "AssertionTransport(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", AuthorizationReason::EnrolledTofu),
        "AuthorizationReason(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", DelegationCapability::TransportWide),
        "DelegationCapability(\"[redacted]\")"
    );
    let key = Keys::generate();
    assert_eq!(
        format!("{:?}", VerifiedKeyAttestation::new(key.public_key())),
        "VerifiedKeyAttestation { pubkey: \"[redacted]\" }"
    );
    assert_eq!(
        format!("{:?}", binding(key.public_key())),
        concat!(
            "VersionedBindingRef { authorization_domain: \"[redacted]\", ",
            "binding_id: \"[redacted]\", principal: FederatedPrincipal { ",
            "issuer: \"[redacted]\", subject: \"[redacted]\" }, ",
            "bound_pubkey: \"[redacted]\", binding_version: \"[redacted]\", ",
            "expires_at: \"[redacted]\", source: \"[redacted]\", ",
            "resolution_reason: \"[redacted]\" }"
        )
    );
    assert_eq!(
        format!("{:?}", FederatedAuthorization::NotRequired),
        "FederatedAuthorization(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", policy_required(EnrollmentMode::AttestedKey)),
        concat!(
            "ResolvedFederatedPolicy { stamp: FederatedPolicyStamp { ",
            "authorization_domain: \"[redacted]\", policy_id: \"[redacted]\", ",
            "epoch: \"[redacted]\", correlation_id: \"[redacted]\", ",
            "requirement: \"[redacted]\", effective_from: \"[redacted]\", ",
            "effective_until: \"[redacted]\" } }"
        )
    );
}

#[test]
fn federated_policy_must_match_the_authorization_correlation() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_with_lineage(EnrollmentMode::AttestedKey, Uuid::from_u128(99), 1, 200),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect_err("policy evidence from another decision must not finalize");

    assert_eq!(error, AuthContextError::FederatedPolicyCorrelationMismatch);
}

#[test]
fn federated_policy_effective_interval_is_half_open() {
    let actor = Keys::generate();
    let not_yet_effective = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_with_lineage(EnrollmentMode::AttestedKey, Uuid::from_u128(2), 101, 200),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 300),
        },
        100,
    )
    .expect_err("policy must not authorize before its effective interval");
    assert_eq!(
        not_yet_effective,
        AuthContextError::FederatedPolicyNotYetEffective
    );

    let expired = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_with_lineage(EnrollmentMode::AttestedKey, Uuid::from_u128(2), 50, 100),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 300),
        },
        100,
    )
    .expect_err("policy must deny at its exact exclusive bound");
    assert_eq!(expired, AuthContextError::FederatedPolicyExpired);
}

#[test]
fn federated_policy_stamp_rejects_invalid_lineage() {
    let requirement = FederatedIdentityRequirement::Required(EnrollmentMode::Provisioned);
    assert_eq!(
        FederatedPolicyStamp::from_authoritative_state(
            authorization_domain(1),
            Uuid::nil(),
            1,
            Uuid::from_u128(2),
            requirement,
            1,
            200,
        ),
        Err(AuthContextError::InvalidFederatedPolicyId)
    );
    assert_eq!(
        FederatedPolicyStamp::from_authoritative_state(
            authorization_domain(1),
            Uuid::from_u128(40),
            0,
            Uuid::from_u128(2),
            requirement,
            1,
            200,
        ),
        Err(AuthContextError::InvalidFederatedPolicyEpoch)
    );
    assert_eq!(
        FederatedPolicyStamp::from_authoritative_state(
            authorization_domain(1),
            Uuid::from_u128(40),
            1,
            Uuid::nil(),
            requirement,
            1,
            200,
        ),
        Err(AuthContextError::InvalidFederatedPolicyCorrelation)
    );
    assert_eq!(
        FederatedPolicyStamp::from_authoritative_state(
            authorization_domain(1),
            Uuid::from_u128(40),
            1,
            Uuid::from_u128(2),
            requirement,
            200,
            200,
        ),
        Err(AuthContextError::InvalidFederatedPolicyInterval)
    );
}

#[test]
fn authoritative_finalizer_derives_binding_reason() {
    let existing_actor = Keys::generate();
    let existing = AuthContext::finalize_authoritative_v1(
        CapabilityFinalizationSeal::new(),
        input(
            existing_actor.public_key(),
            AuthTransport::RelayWebSocket,
            None,
        ),
        policy_required(EnrollmentMode::Tofu),
        AuthoritativeFederatedResolution::Direct {
            binding: AuthoritativeBindingResolution::existing_active(
                authoritative_binding_evidence(
                    existing_actor.public_key(),
                    BindingSource::Provisioned,
                ),
            ),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect("existing authoritative binding is eligible");
    assert_eq!(
        existing.authorization_reason(),
        AuthorizationReason::ExistingBinding
    );

    let attested_actor = Keys::generate();
    let attested = AuthContext::finalize_authoritative_v1(
        CapabilityFinalizationSeal::new(),
        input(
            attested_actor.public_key(),
            AuthTransport::RelayWebSocket,
            None,
        ),
        policy_required(EnrollmentMode::AttestedKey),
        AuthoritativeFederatedResolution::Direct {
            binding: AuthoritativeBindingResolution::atomically_enrolled(
                authoritative_binding_evidence(
                    attested_actor.public_key(),
                    BindingSource::AttestedKey,
                ),
            ),
            assertion: assertion_with_attested_key(
                principal(),
                AssertionTransport::TrustedProxy,
                200,
                attested_actor.public_key(),
            ),
        },
        100,
    )
    .expect("attested enrollment result is eligible");
    assert_eq!(
        attested.authorization_reason(),
        AuthorizationReason::EnrolledAttestedKey
    );

    let tofu_actor = Keys::generate();
    let tofu = AuthContext::finalize_authoritative_v1(
        CapabilityFinalizationSeal::new(),
        input(tofu_actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::Tofu),
        AuthoritativeFederatedResolution::Direct {
            binding: AuthoritativeBindingResolution::atomically_enrolled(
                authoritative_binding_evidence(tofu_actor.public_key(), BindingSource::AttestedKey),
            ),
            assertion: assertion_with_attested_key(
                principal(),
                AssertionTransport::TrustedProxy,
                200,
                tofu_actor.public_key(),
            ),
        },
        100,
    )
    .expect("stronger attested provenance remains valid under TOFU enrollment");
    assert_eq!(
        tofu.authorization_reason(),
        AuthorizationReason::EnrolledTofu
    );
}

#[tokio::test]
async fn cross_crate_authority_adapter_seals_policy_and_binding_outcome() {
    let actor = Keys::generate();
    let adapter = TestAuthorityAdapter;
    let policy = resolve_current_federated_policy(
        &adapter,
        authorization_domain(1),
        Uuid::from_u128(2),
        100,
    )
    .await
    .expect("current authoritative policy is valid");
    let binding = authority::resolve_direct_binding(
        &adapter,
        &policy,
        principal(),
        actor.public_key(),
        true,
        90,
        180,
        100,
    )
    .await
    .expect("atomic binding resolution is valid");
    let context = AuthContext::finalize_authoritative_v1(
        CapabilityFinalizationSeal::new(),
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy,
        AuthoritativeFederatedResolution::Direct {
            binding,
            assertion: assertion_with_attested_key(
                principal(),
                AssertionTransport::TrustedProxy,
                180,
                actor.public_key(),
            ),
        },
        100,
    )
    .expect("sealed adapter output finalizes");

    assert_eq!(
        context.authorization_reason(),
        AuthorizationReason::EnrolledAttestedKey
    );
}

#[tokio::test]
async fn binding_sink_rejects_missing_attestation_for_attested_enrollment() {
    struct MissingAttestationAdapter;

    impl FederatedAuthorityAdapter for MissingAttestationAdapter {
        type Error = &'static str;

        fn resolve_current_policy<'a>(
            &'a self,
            request: CurrentPolicyRequest,
            sink: CurrentPolicyResolutionSink,
        ) -> AuthorityAdapterFuture<
            'a,
            Result<ResolvedFederatedPolicy, AuthorityAdapterError<Self::Error>>,
        > {
            Box::pin(async move {
                sink.resolved(
                    request.authorization_domain(),
                    Uuid::from_u128(40),
                    7,
                    FederatedIdentityRequirement::Required(EnrollmentMode::AttestedKey),
                    90,
                    200,
                )
                .map_err(AuthorityAdapterError::from)
            })
        }

        fn resolve_direct_binding<'a>(
            &'a self,
            request: BindingResolutionRequest,
            sink: DirectBindingResolutionSink,
        ) -> AuthorityAdapterFuture<
            'a,
            Result<AuthoritativeBindingResolution, AuthorityAdapterError<Self::Error>>,
        > {
            Box::pin(async move {
                sink.atomically_enrolled(
                    request.authorization_domain(),
                    Uuid::from_u128(10),
                    request.principal().clone(),
                    request.bound_pubkey(),
                    BindingVersion::INITIAL,
                    None,
                    BindingSource::AttestedKey,
                )
                .map_err(AuthorityAdapterError::from)
            })
        }

        fn resolve_existing_binding<'a>(
            &'a self,
            _request: BindingResolutionRequest,
            _sink: ExistingBindingResolutionSink,
        ) -> AuthorityAdapterFuture<
            'a,
            Result<AuthoritativeBindingResolution, AuthorityAdapterError<Self::Error>>,
        > {
            Box::pin(async { Err(AuthorityAdapterError::adapter("not called")) })
        }
    }

    let actor = Keys::generate();
    let adapter = MissingAttestationAdapter;
    let policy = resolve_current_federated_policy(
        &adapter,
        authorization_domain(1),
        Uuid::from_u128(2),
        100,
    )
    .await
    .expect("current authoritative policy is valid");
    let error = authority::resolve_direct_binding(
        &adapter,
        &policy,
        principal(),
        actor.public_key(),
        false,
        90,
        180,
        100,
    )
    .await
    .expect_err("attested-key enrollment requires sealed matching attestation");

    assert_eq!(
        error,
        AuthorityAdapterError::Contract(AuthContextError::KeyAttestationRequired)
    );
}

#[test]
fn authoritative_finalizer_rejects_incompatible_enrollment_result() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_authoritative_v1(
        CapabilityFinalizationSeal::new(),
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::Provisioned),
        AuthoritativeFederatedResolution::Direct {
            binding: AuthoritativeBindingResolution::atomically_enrolled(
                authoritative_binding_evidence(actor.public_key(), BindingSource::Provisioned),
            ),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect_err("ordinary finalization cannot relabel provisioned state as enrollment");

    assert_eq!(error, AuthContextError::InvalidAuthorizationReason);
}

#[test]
fn authoritative_finalizer_carries_binding_expiry() {
    let actor = Keys::generate();
    let evidence = AuthoritativeBindingEvidence::new(
        authorization_domain(1),
        Uuid::from_u128(10),
        principal(),
        actor.public_key(),
        BindingVersion::INITIAL,
        Some(BindingExpiry::new(100).expect("synthetic binding expiry is valid")),
        BindingSource::Provisioned,
    )
    .expect("synthetic authoritative binding evidence is valid");
    let error = AuthContext::finalize_authoritative_v1(
        CapabilityFinalizationSeal::new(),
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::Provisioned),
        AuthoritativeFederatedResolution::Direct {
            binding: AuthoritativeBindingResolution::existing_active(evidence),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect_err("production finalization must preserve the authoritative binding bound");

    assert_eq!(error, AuthContextError::BindingExpired);
}

#[test]
fn authoritative_finalizer_requires_existing_active_delegated_owner() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let admission = || {
        VerifiedOwnerAdmission::new(
            authorization_domain(1),
            principal(),
            AdmissionExpiry::new(200).expect("synthetic admission expiry is valid"),
        )
    };
    let enrolled_error = AuthContext::finalize_authoritative_v1(
        CapabilityFinalizationSeal::new(),
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::Provisioned),
        AuthoritativeFederatedResolution::Delegated {
            owner: AuthoritativeBindingResolution::atomically_enrolled(
                authoritative_binding_evidence(owner.public_key(), BindingSource::Provisioned),
            ),
            admission: admission(),
        },
        100,
    )
    .expect_err("a newly enrolled record cannot be relabeled as an existing delegated owner");
    assert_eq!(
        enrolled_error,
        AuthContextError::DelegatedBindingNotExistingActive
    );

    let existing = AuthContext::finalize_authoritative_v1(
        CapabilityFinalizationSeal::new(),
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::Provisioned),
        AuthoritativeFederatedResolution::Delegated {
            owner: AuthoritativeBindingResolution::existing_active(authoritative_binding_evidence(
                owner.public_key(),
                BindingSource::Provisioned,
            )),
            admission: admission(),
        },
        100,
    )
    .expect("an existing active owner binding is eligible for delegated finalization");
    assert_eq!(
        existing.authorization_reason(),
        AuthorizationReason::DelegatedOwnerBinding
    );
}

#[test]
fn tofu_enrollment_cannot_use_attested_key_policy_reason() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::Tofu),
        FederatedAuthorization::Direct {
            binding: enrolled_binding(
                actor.public_key(),
                BindingSource::AttestedKey,
                AuthorizationReason::EnrolledAttestedKey,
            ),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect_err("TOFU policy must emit the TOFU enrollment reason");

    assert_eq!(error, AuthContextError::InvalidAuthorizationReason);
}

#[test]
fn transport_and_proof_method_must_agree() {
    let actor = Keys::generate();
    let error = VerifiedNostrProof::new(
        authorization_domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip98,
        None,
    )
    .expect_err("HTTP proof must not authorize a relay WebSocket");
    assert_eq!(error, AuthContextError::TransportProofMismatch);
}

#[test]
fn blossom_upload_proof_cannot_authorize_media_downloads() {
    let actor = Keys::generate();
    let error = VerifiedNostrProof::new(
        authorization_domain(1),
        AuthTransport::MediaDownload,
        actor.public_key(),
        AuthMethod::Blossom,
        None,
    )
    .expect_err("upload-only proof must not be widened to media download authority");
    assert_eq!(error, AuthContextError::TransportProofMismatch);

    VerifiedNostrProof::new(
        authorization_domain(1),
        AuthTransport::MediaUpload,
        actor.public_key(),
        AuthMethod::Blossom,
        None,
    )
    .expect("Blossom proof may authorize the verified upload operation");
}

#[test]
fn binding_cannot_cross_authorization_domains() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: binding_in(2, actor.public_key()),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect_err("a binding from another domain must be rejected");

    assert_eq!(error, AuthContextError::BindingDomainMismatch);
    assert_eq!(error.code(), "federated_binding_domain_mismatch");
}

#[test]
fn nostr_only_authorization_may_preserve_a_verified_owner() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let context = AuthContext::finalize_v1(
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_not_required(),
        FederatedAuthorization::NotRequired,
        100,
    )
    .expect("Nostr delegation remains independent of federated policy");

    assert_eq!(context.agent_owner_pubkey(), Some(owner.public_key()));
    assert_eq!(
        context.authorization_reason(),
        AuthorizationReason::NostrOnly
    );
}

#[test]
fn transport_delegation_rejects_self_reference() {
    let actor = Keys::generate();
    let error =
        VerifiedTransportDelegation::new_unrestricted(actor.public_key(), actor.public_key(), None)
            .expect_err("an actor cannot be its own verified owner");

    assert_eq!(error, AuthContextError::SelfDelegation);
}

#[test]
fn transport_delegation_is_explicitly_transport_wide() {
    let owner = Keys::generate();
    let delegate = Keys::generate();
    let delegation = VerifiedTransportDelegation::new_unrestricted(
        owner.public_key(),
        delegate.public_key(),
        None,
    )
    .expect("synthetic owner and delegate are distinct");

    assert_eq!(delegation.capability(), DelegationCapability::TransportWide);
}

#[test]
fn required_policy_rejects_nostr_only_authorization() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::NotRequired,
        100,
    )
    .expect_err("required federated identity cannot be bypassed by the caller");

    assert_eq!(error, AuthContextError::FederatedIdentityRequired);
    assert_eq!(error.code(), "federated_identity_required");
}

#[test]
fn not_required_policy_rejects_federated_authorization() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_not_required(),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion(principal(), AssertionTransport::TrustedProxy, 200),
        },
        100,
    )
    .expect_err("federated evidence cannot override the resolved domain policy");

    assert_eq!(error, AuthContextError::UnexpectedFederatedAuthorization);
    assert_eq!(error.code(), "federated_authorization_unexpected");
}

#[test]
fn nostr_proof_cannot_cross_authorization_domains() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        AuthContextInput::new(
            tenant(1),
            Uuid::from_u128(2),
            proof_in(2, AuthTransport::RelayWebSocket, actor.public_key()),
            community_access_in(1),
        ),
        policy_not_required(),
        FederatedAuthorization::NotRequired,
        100,
    )
    .expect_err("a Nostr proof from another domain must be rejected");

    assert_eq!(error, AuthContextError::NostrProofDomainMismatch);
    assert_eq!(error.code(), "nostr_proof_domain_mismatch");
}

#[test]
fn federated_policy_cannot_cross_authorization_domains() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        ResolvedFederatedPolicy::not_required(authorization_domain(2)),
        FederatedAuthorization::NotRequired,
        100,
    )
    .expect_err("policy from another domain must be rejected");

    assert_eq!(error, AuthContextError::PolicyDomainMismatch);
    assert_eq!(error.code(), "federated_policy_domain_mismatch");
}

#[test]
fn community_admission_cannot_cross_authorization_domains() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        AuthContextInput::new(
            tenant(1),
            Uuid::from_u128(2),
            proof_in(1, AuthTransport::RelayWebSocket, actor.public_key()),
            community_access_in(2),
        ),
        policy_not_required(),
        FederatedAuthorization::NotRequired,
        100,
    )
    .expect_err("community admission from another domain must be rejected");

    assert_eq!(error, AuthContextError::CommunityAccessDomainMismatch);
    assert_eq!(error.code(), "community_access_domain_mismatch");
}

#[test]
fn assertion_cannot_cross_authorization_domains() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion_in(
                2,
                AuthTransport::RelayWebSocket,
                principal(),
                AssertionTransport::TrustedProxy,
                200,
            ),
        },
        100,
    )
    .expect_err("an assertion from another domain must be rejected");

    assert_eq!(error, AuthContextError::AssertionDomainMismatch);
    assert_eq!(error.code(), "federated_assertion_domain_mismatch");
}

#[test]
fn assertion_must_match_the_authorized_transport() {
    let actor = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(actor.public_key(), AuthTransport::RelayWebSocket, None),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Direct {
            binding: binding(actor.public_key()),
            assertion: assertion_in(
                1,
                AuthTransport::HttpBridge,
                principal(),
                AssertionTransport::TrustedProxy,
                200,
            ),
        },
        100,
    )
    .expect_err("an assertion verified for another transport must be rejected");

    assert_eq!(error, AuthContextError::AssertionTransportMismatch);
    assert_eq!(error.code(), "federated_assertion_transport_mismatch");
}

#[test]
fn delegated_owner_admission_must_match_the_authorization_domain() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let error = AuthContext::finalize_v1(
        input(
            actor.public_key(),
            AuthTransport::RelayWebSocket,
            Some(owner.public_key()),
        ),
        policy_required(EnrollmentMode::AttestedKey),
        FederatedAuthorization::Delegated {
            owner: binding(owner.public_key()),
            admission: VerifiedOwnerAdmission::new(
                authorization_domain(2),
                principal(),
                AdmissionExpiry::new(200).expect("synthetic admission expiry is valid"),
            ),
        },
        100,
    )
    .expect_err("owner admission cannot cross authorization domains");

    assert_eq!(error, AuthContextError::OwnerAdmissionDomainMismatch);
}
