use std::{
    future::pending,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use nostr::Keys;

use super::*;
use crate::context::{
    AssertionExpiry, AssertionNotBefore, AssertionTransport, AuthTransport,
    AuthoritativeBindingEvidence, AuthoritativeBindingResolution, BindingExpiry, BindingSource,
    BindingVersion, DelegationExpiry, EnrollmentMode, FederatedIdentityRequirement,
    FederatedPolicyStamp, ResolvedFederatedPolicy, VerifiedKeyAttestation,
    VerifiedTransportDelegation,
};
use crate::{
    AuthorityAdapterFuture, AuthorizedCommunityAccess, BindingResolutionRequest,
    CurrentPolicyRequest, CurrentPolicyResolutionSink, DirectBindingResolutionSink,
    ExistingBindingResolutionSink, Scope,
};

const NOW: u64 = 100;

fn domain(value: u128) -> CommunityId {
    CommunityId::from_uuid(Uuid::from_u128(value))
}

fn principal() -> FederatedPrincipal {
    FederatedPrincipal::new("https://idp.example", "subject-123")
        .expect("synthetic principal is valid")
}

fn profile() -> AuthorizationProfileId {
    AuthorizationProfileId::from_server_configuration("profile-1")
        .expect("synthetic profile is valid")
}

fn policy_version(value: &str) -> PolicyVersion {
    PolicyVersion::new(value).expect("synthetic policy version is valid")
}

fn federated_policy_with(
    domain_value: u128,
    correlation_id: Uuid,
    epoch: u64,
    enrollment_mode: EnrollmentMode,
    effective_from: u64,
    effective_until: u64,
) -> ResolvedFederatedPolicy {
    ResolvedFederatedPolicy::from_authoritative_resolution(
        FederatedPolicyStamp::from_authoritative_state(
            domain(domain_value),
            Uuid::from_u128(40),
            epoch,
            correlation_id,
            FederatedIdentityRequirement::Required(enrollment_mode),
            effective_from,
            effective_until,
        )
        .expect("synthetic federated policy lineage is valid"),
    )
}

fn federated_policy() -> ResolvedFederatedPolicy {
    federated_policy_with(
        1,
        Uuid::from_u128(20),
        1,
        EnrollmentMode::Provisioned,
        1,
        200,
    )
}

fn provider_timeout() -> ProviderTimeout {
    ProviderTimeout::new(Duration::from_secs(1)).expect("synthetic timeout is finite")
}

#[derive(Clone)]
struct TestClock {
    now: Arc<AtomicU64>,
    available: Arc<AtomicBool>,
    reads: Arc<AtomicUsize>,
}

impl TestClock {
    fn at(now: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(now)),
            available: Arc::new(AtomicBool::new(true)),
            reads: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn set(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }

    fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl AuthorizationClock for TestClock {
    fn now_unix_seconds(&self) -> Option<u64> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.available
            .load(Ordering::SeqCst)
            .then(|| self.now.load(Ordering::SeqCst))
    }
}

#[derive(Clone)]
struct TestAuthorityAdapter {
    policy_epoch: u64,
    enrollment_mode: EnrollmentMode,
    enroll_direct: bool,
    policy_reads: Arc<AtomicUsize>,
    direct_calls: Arc<AtomicUsize>,
    existing_calls: Arc<AtomicUsize>,
    committed_enrollments: Arc<AtomicUsize>,
}

impl TestAuthorityAdapter {
    fn new(policy_epoch: u64, enrollment_mode: EnrollmentMode, enroll_direct: bool) -> Self {
        Self {
            policy_epoch,
            enrollment_mode,
            enroll_direct,
            policy_reads: Arc::new(AtomicUsize::new(0)),
            direct_calls: Arc::new(AtomicUsize::new(0)),
            existing_calls: Arc::new(AtomicUsize::new(0)),
            committed_enrollments: Arc::new(AtomicUsize::new(0)),
        }
    }
}

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
            self.policy_reads.fetch_add(1, Ordering::SeqCst);
            sink.resolved(
                request.authorization_domain(),
                Uuid::from_u128(40),
                self.policy_epoch,
                FederatedIdentityRequirement::Required(self.enrollment_mode),
                1,
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
            self.direct_calls.fetch_add(1, Ordering::SeqCst);
            let result = if self.enroll_direct {
                let source = match self.enrollment_mode {
                    EnrollmentMode::AttestedKey => BindingSource::AttestedKey,
                    EnrollmentMode::Tofu => BindingSource::Tofu,
                    EnrollmentMode::Provisioned => BindingSource::Provisioned,
                };
                sink.atomically_enrolled(
                    request.authorization_domain(),
                    Uuid::from_u128(10),
                    request.principal().clone(),
                    request.bound_pubkey(),
                    BindingVersion::INITIAL,
                    None,
                    source,
                )
            } else {
                sink.existing_active(
                    request.authorization_domain(),
                    Uuid::from_u128(10),
                    request.principal().clone(),
                    request.bound_pubkey(),
                    BindingVersion::INITIAL,
                    None,
                    BindingSource::Provisioned,
                )
            };
            let resolution = result.map_err(AuthorityAdapterError::from)?;
            if self.enroll_direct {
                self.committed_enrollments.fetch_add(1, Ordering::SeqCst);
            }
            Ok(resolution)
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
            self.existing_calls.fetch_add(1, Ordering::SeqCst);
            sink.existing_active(
                request.authorization_domain(),
                Uuid::from_u128(10),
                request.principal().clone(),
                request.bound_pubkey(),
                BindingVersion::INITIAL,
                None,
                BindingSource::Provisioned,
            )
            .map_err(AuthorityAdapterError::from)
        })
    }
}

fn direct_evidence(
    actor: &Keys,
    enrollment_mode: EnrollmentMode,
    key_attested: bool,
) -> (
    VerifiedNostrProof,
    VerifiedFederatedAssertion,
    AuthorizationRequest,
) {
    let proof = VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        None,
    )
    .expect("synthetic proof is valid");
    let assertion = VerifiedFederatedAssertion::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        principal(),
        key_attested.then(|| VerifiedKeyAttestation::new(actor.public_key())),
        AssertionTransport::TrustedProxy,
        Some(AssertionNotBefore::new(90)),
        AssertionExpiry::new(180).expect("synthetic assertion expiry is valid"),
    );
    let request = AuthorizationRequest::direct(
        &proof,
        &assertion,
        federated_policy_with(1, Uuid::from_u128(20), 1, enrollment_mode, 1, 200),
        capabilities(&[AuthorizationCapability::CommunityRead]),
        Uuid::from_u128(20),
        NOW,
    )
    .expect("synthetic direct request is valid");
    (proof, assertion, request)
}

fn finalization_input(proof: VerifiedNostrProof) -> AuthContextInput {
    AuthContextInput::new(
        buzz_core::TenantContext::resolved(domain(1), "relay.example"),
        Uuid::from_u128(20),
        proof,
        AuthorizedCommunityAccess::new(domain(1), Scope::all_known(), None),
    )
}

async fn resolve_at(
    provider: &dyn AuthorizationProvider,
    request: &AuthorizationRequest,
    now: u64,
    timeout: ProviderTimeout,
) -> AuthorizationOutcome {
    resolve_authorization(
        provider,
        request,
        &TestClock::at(now),
        timeout,
        Uuid::from_u128(99),
    )
    .await
}

fn capabilities(values: &[AuthorizationCapability]) -> CapabilitySet {
    CapabilitySet::new(values.to_vec()).expect("synthetic capabilities are non-empty")
}

fn all_capabilities() -> [AuthorizationCapability; 10] {
    [
        AuthorizationCapability::CommunityRead,
        AuthorizationCapability::CommunityWrite,
        AuthorizationCapability::Moderate,
        AuthorizationCapability::InviteMint,
        AuthorizationCapability::InviteClaim,
        AuthorizationCapability::MediaRead,
        AuthorizationCapability::MediaWrite,
        AuthorizationCapability::GitRead,
        AuthorizationCapability::GitWrite,
        AuthorizationCapability::AudioJoin,
    ]
}

fn capability_coverage_is_exhaustive(capability: AuthorizationCapability) {
    match capability {
        AuthorizationCapability::CommunityRead
        | AuthorizationCapability::CommunityWrite
        | AuthorizationCapability::Moderate
        | AuthorizationCapability::InviteMint
        | AuthorizationCapability::InviteClaim
        | AuthorizationCapability::MediaRead
        | AuthorizationCapability::MediaWrite
        | AuthorizationCapability::GitRead
        | AuthorizationCapability::GitWrite
        | AuthorizationCapability::AudioJoin => {}
    }
}

fn proof_method_for_transport(transport: AuthTransport) -> AuthMethod {
    match transport {
        AuthTransport::RelayWebSocket => AuthMethod::Nip42,
        AuthTransport::HttpBridge | AuthTransport::Git | AuthTransport::MediaDownload => {
            AuthMethod::Nip98
        }
        AuthTransport::MediaUpload => AuthMethod::Blossom,
        AuthTransport::Audio => AuthMethod::Nip42,
    }
}

fn all_contract_errors() -> [ProviderContractError; 34] {
    [
        ProviderContractError::EmptyCapabilitySet,
        ProviderContractError::EmptyProfileId,
        ProviderContractError::ProfileIdTooLong,
        ProviderContractError::EmptyPolicyVersion,
        ProviderContractError::PolicyVersionTooLong,
        ProviderContractError::InvalidIssuedAt,
        ProviderContractError::InvalidFreshnessBound,
        ProviderContractError::FreshnessWindowTooLong,
        ProviderContractError::InvalidRetryAfter,
        ProviderContractError::InvalidProviderTimeout,
        ProviderContractError::InvalidCorrelationId,
        ProviderContractError::DirectRequestHasOwner,
        ProviderContractError::AuthorizationDomainMismatch,
        ProviderContractError::TransportMismatch,
        ProviderContractError::AssertionNotYetValid,
        ProviderContractError::AssertionExpired,
        ProviderContractError::KeyAttestationMismatch,
        ProviderContractError::DelegatedBindingNotExistingActive,
        ProviderContractError::DelegationRequired,
        ProviderContractError::DelegatedOwnerMismatch,
        ProviderContractError::DelegationExpired,
        ProviderContractError::BindingExpired,
        ProviderContractError::FederatedPolicyDomainMismatch,
        ProviderContractError::FederatedPolicyCorrelationMismatch,
        ProviderContractError::FederatedPolicyNotYetEffective,
        ProviderContractError::FederatedPolicyExpired,
        ProviderContractError::CapabilityNotYetEffective,
        ProviderContractError::CapabilityExpired,
        ProviderContractError::CapabilityContextMismatch,
        ProviderContractError::CapabilityAuthorityMismatch,
        ProviderContractError::CapabilityPrincipalMismatch,
        ProviderContractError::CapabilityBindingChanged,
        ProviderContractError::FederatedPolicyChanged,
        ProviderContractError::AuthorizationRuntimeMismatch,
    ]
}

fn direct_request_for_transport(
    actor: &Keys,
    transport: AuthTransport,
    proof_method: AuthMethod,
    not_before: Option<u64>,
    expiry: u64,
    requested: CapabilitySet,
) -> Result<AuthorizationRequest, ProviderContractError> {
    let proof =
        VerifiedNostrProof::new(domain(1), transport, actor.public_key(), proof_method, None)
            .expect("synthetic proof is valid");
    let assertion = VerifiedFederatedAssertion::new(
        domain(1),
        transport,
        principal(),
        Some(VerifiedKeyAttestation::new(actor.public_key())),
        AssertionTransport::TrustedProxy,
        not_before.map(AssertionNotBefore::new),
        AssertionExpiry::new(expiry).expect("synthetic assertion expiry is valid"),
    );
    AuthorizationRequest::direct(
        &proof,
        &assertion,
        federated_policy(),
        requested,
        Uuid::from_u128(20),
        NOW,
    )
}

fn direct_request_with_expiry(
    actor: &Keys,
    expiry: u64,
    requested: CapabilitySet,
) -> AuthorizationRequest {
    direct_request_for_transport(
        actor,
        AuthTransport::RelayWebSocket,
        AuthMethod::Nip42,
        None,
        expiry,
        requested,
    )
    .expect("synthetic direct request is valid")
}

fn direct_request(actor: &Keys) -> AuthorizationRequest {
    direct_request_with_expiry(
        actor,
        200,
        capabilities(&[AuthorizationCapability::CommunityRead]),
    )
}

fn existing_binding(owner: &Keys) -> AuthoritativeBindingResolution {
    existing_binding_in(1, owner)
}

fn existing_binding_in(domain_value: u128, owner: &Keys) -> AuthoritativeBindingResolution {
    existing_binding_with_expiry_in(domain_value, owner, None)
}

fn existing_binding_with_expiry_in(
    domain_value: u128,
    owner: &Keys,
    expires_at: Option<u64>,
) -> AuthoritativeBindingResolution {
    let evidence = AuthoritativeBindingEvidence::new(
        domain(domain_value),
        Uuid::from_u128(10),
        principal(),
        owner.public_key(),
        BindingVersion::INITIAL,
        expires_at
            .map(|expiry| BindingExpiry::new(expiry).expect("synthetic binding expiry is valid")),
        BindingSource::Provisioned,
    )
    .expect("synthetic binding is valid");
    AuthoritativeBindingResolution::existing_active(evidence)
}

fn delegated_proof(actor: &Keys, owner: &Keys, expiry: u64) -> VerifiedNostrProof {
    let delegation = VerifiedTransportDelegation::new_unrestricted(
        owner.public_key(),
        actor.public_key(),
        Some(DelegationExpiry::new(expiry).expect("synthetic delegation expiry is valid")),
    )
    .expect("synthetic delegation is valid");
    VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        Some(delegation),
    )
    .expect("synthetic delegated proof is valid")
}

fn delegated_request(actor: &Keys, owner: &Keys, expiry: u64) -> AuthorizationRequest {
    let proof = delegated_proof(actor, owner, expiry);
    AuthorizationRequest::delegated(
        &proof,
        &existing_binding(owner),
        federated_policy(),
        capabilities(&[AuthorizationCapability::CommunityRead]),
        Uuid::from_u128(20),
        NOW,
    )
    .expect("synthetic delegated request is valid")
}

fn allow_for(
    request: &AuthorizationRequest,
    granted: CapabilitySet,
    version: &str,
    issued_at: u64,
    fresh_until: u64,
) -> ProviderDecision {
    ProviderDecision::Allow(
        ProviderAllow::new(
            request.authorization_domain(),
            request.principal().clone(),
            profile(),
            granted,
            policy_version(version),
            issued_at,
            fresh_until,
        )
        .expect("synthetic provider allow is structurally valid"),
    )
}

struct FakeProvider {
    decision: Mutex<Option<ProviderDecision>>,
}

impl FakeProvider {
    fn returning(decision: ProviderDecision) -> Self {
        Self {
            decision: Mutex::new(Some(decision)),
        }
    }
}

impl AuthorizationProvider for FakeProvider {
    fn profile_id(&self) -> AuthorizationProfileId {
        profile()
    }

    fn authorize<'a>(
        &'a self,
        _request: &'a AuthorizationRequest,
    ) -> AuthorizationProviderFuture<'a> {
        Box::pin(async move {
            self.decision
                .lock()
                .expect("synthetic provider mutex is not poisoned")
                .take()
                .expect("synthetic provider is called exactly once")
        })
    }
}

struct EchoAllowProvider;

impl AuthorizationProvider for EchoAllowProvider {
    fn profile_id(&self) -> AuthorizationProfileId {
        profile()
    }

    fn authorize<'a>(
        &'a self,
        request: &'a AuthorizationRequest,
    ) -> AuthorizationProviderFuture<'a> {
        Box::pin(async move {
            allow_for(
                request,
                request.requested_capabilities().clone(),
                "version-a",
                90,
                180,
            )
        })
    }
}

struct AdvancingProvider {
    decision: Mutex<Option<ProviderDecision>>,
    clock: TestClock,
    decision_time: u64,
    clock_available: bool,
}

impl AdvancingProvider {
    fn returning_at(decision: ProviderDecision, clock: TestClock, decision_time: u64) -> Self {
        Self {
            decision: Mutex::new(Some(decision)),
            clock,
            decision_time,
            clock_available: true,
        }
    }

    fn returning_with_clock_failure(decision: ProviderDecision, clock: TestClock) -> Self {
        Self {
            decision: Mutex::new(Some(decision)),
            clock,
            decision_time: 0,
            clock_available: false,
        }
    }
}

impl AuthorizationProvider for AdvancingProvider {
    fn profile_id(&self) -> AuthorizationProfileId {
        profile()
    }

    fn authorize<'a>(
        &'a self,
        _request: &'a AuthorizationRequest,
    ) -> AuthorizationProviderFuture<'a> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            assert_eq!(
                self.clock.reads(),
                0,
                "decision time must not be sampled before provider I/O completes"
            );
            self.clock.set(self.decision_time);
            self.clock.set_available(self.clock_available);
            self.decision
                .lock()
                .expect("synthetic provider mutex is not poisoned")
                .take()
                .expect("synthetic provider is called exactly once")
        })
    }
}

struct PendingProvider {
    calls: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

struct CancellationMarker(Arc<AtomicBool>);

impl Drop for CancellationMarker {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl AuthorizationProvider for PendingProvider {
    fn profile_id(&self) -> AuthorizationProfileId {
        profile()
    }

    fn authorize<'a>(
        &'a self,
        _request: &'a AuthorizationRequest,
    ) -> AuthorizationProviderFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let marker = CancellationMarker(Arc::clone(&self.dropped));
        Box::pin(async move {
            let _marker = marker;
            pending().await
        })
    }
}

#[tokio::test]
async fn current_allow_returns_request_scoped_snapshot() {
    let actor = Keys::generate();
    let request = direct_request(&actor);
    let provider = FakeProvider::returning(allow_for(
        &request,
        capabilities(&[
            AuthorizationCapability::CommunityRead,
            AuthorizationCapability::CommunityWrite,
        ]),
        "version-a",
        90,
        180,
    ));

    let AuthorizationOutcome::Allow(snapshot) =
        resolve_at(&provider, &request, NOW, provider_timeout()).await
    else {
        panic!("current provider policy must allow");
    };

    assert_eq!(snapshot.authorization_domain(), domain(1));
    assert_eq!(snapshot.transport(), AuthTransport::RelayWebSocket);
    assert_eq!(snapshot.actor_pubkey(), actor.public_key());
    assert_eq!(snapshot.owner_pubkey(), None);
    assert_eq!(snapshot.binding_id(), None);
    assert_eq!(snapshot.binding_version(), None);
    assert_eq!(snapshot.proof_method(), AuthMethod::Nip42);
    assert_eq!(snapshot.principal(), request.principal());
    assert_eq!(snapshot.profile_id(), &profile());
    assert_eq!(
        snapshot.capabilities().as_slice(),
        &[AuthorizationCapability::CommunityRead]
    );
    assert_eq!(snapshot.policy_version().as_str(), "version-a");
    assert_eq!(snapshot.issued_at(), 90);
    assert_eq!(snapshot.fresh_until(), 180);
    assert_eq!(snapshot.effective_until(), 180);
    assert_eq!(snapshot.decision_source(), DecisionSource::DirectAssertion);
    assert_eq!(snapshot.correlation_id(), request.correlation_id());
    assert_eq!(snapshot.reason(), ProviderAllowReason::CurrentPolicy);
}

#[tokio::test]
async fn runtime_finalizer_allows_existing_binding_without_key_claim() {
    let actor = Keys::generate();
    let (proof, assertion, request) = direct_evidence(&actor, EnrollmentMode::Provisioned, false);
    let provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        90,
        180,
    ));
    let authority = TestAuthorityAdapter::new(1, EnrollmentMode::Provisioned, false);
    let runtime = AuthorizationRuntime::from_server_configuration(
        authority.clone(),
        TestClock::at(NOW),
        provider,
    );
    let AuthorizationOutcome::Allow(snapshot) = runtime
        .resolve_authorization(&request, provider_timeout())
        .await
    else {
        panic!("current provider decision must allow");
    };

    let context = runtime
        .finalize_direct_v1(*snapshot, finalization_input(proof), assertion)
        .await
        .expect("an existing active binding does not require a later key claim");

    assert_eq!(
        context.authorization_reason(),
        crate::AuthorizationReason::ExistingBinding
    );
    assert_eq!(authority.policy_reads.load(Ordering::SeqCst), 1);
    assert_eq!(authority.direct_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn mismatched_embedded_proof_domain_fails_before_authority_io() {
    let actor = Keys::generate();
    let (_, assertion, request) = direct_evidence(&actor, EnrollmentMode::Tofu, false);
    let provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        90,
        180,
    ));
    let authority = TestAuthorityAdapter::new(1, EnrollmentMode::Tofu, true);
    let runtime = AuthorizationRuntime::from_server_configuration(
        authority.clone(),
        TestClock::at(NOW),
        provider,
    );
    let AuthorizationOutcome::Allow(snapshot) = runtime
        .resolve_authorization(&request, provider_timeout())
        .await
    else {
        panic!("current provider decision must allow");
    };
    let mismatched_proof = VerifiedNostrProof::new(
        domain(2),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        None,
    )
    .expect("synthetic mismatched proof is structurally valid");

    let error = runtime
        .finalize_direct_v1(*snapshot, finalization_input(mismatched_proof), assertion)
        .await
        .expect_err("embedded proof domain mismatch must precede authority I/O");

    assert_eq!(
        error,
        ProviderAuthorizationError::Context(AuthContextError::NostrProofDomainMismatch)
    );
    assert_eq!(authority.policy_reads.load(Ordering::SeqCst), 0);
    assert_eq!(authority.direct_calls.load(Ordering::SeqCst), 0);
    assert_eq!(authority.committed_enrollments.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn mismatched_community_access_domain_fails_before_authority_io() {
    let actor = Keys::generate();
    let (proof, assertion, request) = direct_evidence(&actor, EnrollmentMode::Tofu, false);
    let provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        90,
        180,
    ));
    let authority = TestAuthorityAdapter::new(1, EnrollmentMode::Tofu, true);
    let runtime = AuthorizationRuntime::from_server_configuration(
        authority.clone(),
        TestClock::at(NOW),
        provider,
    );
    let AuthorizationOutcome::Allow(snapshot) = runtime
        .resolve_authorization(&request, provider_timeout())
        .await
    else {
        panic!("current provider decision must allow");
    };
    let input = AuthContextInput::new(
        buzz_core::TenantContext::resolved(domain(1), "relay.example"),
        Uuid::from_u128(20),
        proof,
        AuthorizedCommunityAccess::new(domain(2), Scope::all_known(), None),
    );

    let error = runtime
        .finalize_direct_v1(*snapshot, input, assertion)
        .await
        .expect_err("embedded admission domain mismatch must precede authority I/O");

    assert_eq!(
        error,
        ProviderAuthorizationError::Context(AuthContextError::CommunityAccessDomainMismatch)
    );
    assert_eq!(authority.policy_reads.load(Ordering::SeqCst), 0);
    assert_eq!(authority.direct_calls.load(Ordering::SeqCst), 0);
    assert_eq!(authority.committed_enrollments.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn attested_enrollment_without_sealed_key_claim_fails_before_commit() {
    let actor = Keys::generate();
    let (proof, assertion, request) = direct_evidence(&actor, EnrollmentMode::AttestedKey, false);
    let provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        90,
        180,
    ));
    let authority = TestAuthorityAdapter::new(1, EnrollmentMode::AttestedKey, true);
    let runtime = AuthorizationRuntime::from_server_configuration(
        authority.clone(),
        TestClock::at(NOW),
        provider,
    );
    let AuthorizationOutcome::Allow(snapshot) = runtime
        .resolve_authorization(&request, provider_timeout())
        .await
    else {
        panic!("provider evaluation may allow before binding resolution");
    };

    let error = runtime
        .finalize_direct_v1(*snapshot, finalization_input(proof), assertion)
        .await
        .expect_err("attested-key enrollment requires the sealed matching key claim");

    assert_eq!(
        error,
        ProviderAuthorizationError::Authority(AuthorityAdapterError::Contract(
            AuthContextError::KeyAttestationRequired
        ))
    );
    assert_eq!(authority.direct_calls.load(Ordering::SeqCst), 1);
    assert_eq!(authority.committed_enrollments.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fresh_policy_epoch_drift_blocks_binding_mutation() {
    let actor = Keys::generate();
    let (proof, assertion, request) = direct_evidence(&actor, EnrollmentMode::Tofu, false);
    let provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        90,
        180,
    ));
    let authority = TestAuthorityAdapter::new(2, EnrollmentMode::Tofu, true);
    let runtime = AuthorizationRuntime::from_server_configuration(
        authority.clone(),
        TestClock::at(NOW),
        provider,
    );
    let AuthorizationOutcome::Allow(snapshot) = runtime
        .resolve_authorization(&request, provider_timeout())
        .await
    else {
        panic!("request-time policy is current during provider evaluation");
    };

    let error = runtime
        .finalize_direct_v1(*snapshot, finalization_input(proof), assertion)
        .await
        .expect_err("fresh authoritative policy drift must fail before binding I/O");

    assert_eq!(
        error,
        ProviderAuthorizationError::Contract(ProviderContractError::FederatedPolicyChanged)
    );
    assert_eq!(authority.policy_reads.load(Ordering::SeqCst), 1);
    assert_eq!(authority.direct_calls.load(Ordering::SeqCst), 0);
    assert_eq!(authority.committed_enrollments.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn runtime_binding_rejects_forged_adapter_and_clock_substitution() {
    let actor = Keys::generate();
    let (proof, assertion, request) = direct_evidence(&actor, EnrollmentMode::Tofu, false);
    let genuine_provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        90,
        180,
    ));
    let genuine_runtime = AuthorizationRuntime::from_server_configuration(
        TestAuthorityAdapter::new(1, EnrollmentMode::Tofu, true),
        TestClock::at(NOW),
        genuine_provider,
    );
    let AuthorizationOutcome::Allow(snapshot) = genuine_runtime
        .resolve_authorization(&request, provider_timeout())
        .await
    else {
        panic!("genuine runtime must issue the capability snapshot");
    };

    let forged_authority = TestAuthorityAdapter::new(1, EnrollmentMode::Tofu, true);
    let forged_runtime = AuthorizationRuntime::from_server_configuration(
        forged_authority.clone(),
        TestClock::at(NOW),
        FakeProvider::returning(ProviderDecision::Deny(AuthorizationDenial::new(
            AuthorizationDenialReason::ProviderDenied,
        ))),
    );
    let error = forged_runtime
        .finalize_direct_v1(*snapshot, finalization_input(proof), assertion)
        .await
        .expect_err("a legitimate snapshot cannot be spliced to a caller-selected runtime");

    assert_eq!(
        error,
        ProviderAuthorizationError::Contract(ProviderContractError::AuthorizationRuntimeMismatch)
    );
    assert_eq!(forged_authority.policy_reads.load(Ordering::SeqCst), 0);
    assert_eq!(forged_authority.direct_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        forged_authority
            .committed_enrollments
            .load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn runtime_resolves_and_refinalizes_existing_delegated_owner() {
    let delegate = Keys::generate();
    let owner = Keys::generate();
    let proof = delegated_proof(&delegate, &owner, 180);
    let authority = TestAuthorityAdapter::new(1, EnrollmentMode::Provisioned, false);
    let runtime = AuthorizationRuntime::from_server_configuration(
        authority.clone(),
        TestClock::at(NOW),
        EchoAllowProvider,
    );
    let request = runtime
        .resolve_delegated_authorization_request(
            &proof,
            principal(),
            federated_policy(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            Uuid::from_u128(20),
        )
        .await
        .expect("the configured adapter resolves an existing delegated owner");
    let AuthorizationOutcome::Allow(snapshot) = runtime
        .resolve_authorization(&request, provider_timeout())
        .await
    else {
        panic!("the current owner admission must allow");
    };

    let context = runtime
        .finalize_delegated_v1(*snapshot, finalization_input(proof))
        .await
        .expect("the owner is reread and finalized without enrollment");

    assert_eq!(
        context.authorization_reason(),
        crate::AuthorizationReason::DelegatedOwnerBinding
    );
    assert_eq!(authority.policy_reads.load(Ordering::SeqCst), 1);
    assert_eq!(authority.existing_calls.load(Ordering::SeqCst), 2);
    assert_eq!(authority.committed_enrollments.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn allowed_snapshot_preserves_every_requested_transport_scope() {
    let transports = [
        AuthTransport::RelayWebSocket,
        AuthTransport::HttpBridge,
        AuthTransport::Git,
        AuthTransport::MediaUpload,
        AuthTransport::MediaDownload,
        AuthTransport::Audio,
    ];

    for transport in transports {
        let proof_method = proof_method_for_transport(transport);
        let actor = Keys::generate();
        let request = direct_request_for_transport(
            &actor,
            transport,
            proof_method,
            None,
            200,
            capabilities(&[AuthorizationCapability::CommunityRead]),
        )
        .expect("synthetic direct request is valid");
        let provider = FakeProvider::returning(allow_for(
            &request,
            request.requested_capabilities().clone(),
            "version-a",
            90,
            180,
        ));

        let AuthorizationOutcome::Allow(snapshot) =
            resolve_at(&provider, &request, NOW, provider_timeout()).await
        else {
            panic!("current provider policy must allow every transport profile");
        };
        assert_eq!(snapshot.transport(), transport);
        assert_eq!(snapshot.proof_method(), proof_method);
        assert_eq!(snapshot.actor_pubkey(), actor.public_key());
    }
}

#[tokio::test]
async fn explicit_denial_is_preserved() {
    let actor = Keys::generate();
    let request = direct_request(&actor);
    let provider = FakeProvider::returning(ProviderDecision::Deny(AuthorizationDenial::new(
        AuthorizationDenialReason::ProviderDenied,
    )));

    let AuthorizationOutcome::Deny(denial) =
        resolve_at(&provider, &request, NOW, provider_timeout()).await
    else {
        panic!("provider denial must fail closed");
    };
    assert_eq!(denial.reason(), AuthorizationDenialReason::ProviderDenied);
}

#[tokio::test]
async fn provider_unavailability_never_falls_back_to_allow() {
    let actor = Keys::generate();
    let request = direct_request(&actor);
    let retry_after = RetryAfter::new(30).expect("synthetic retry hint is bounded");
    let provider =
        FakeProvider::returning(ProviderDecision::Unavailable(ProviderUnavailable::new(
            ProviderUnavailableReason::TemporarilyUnavailable,
            Some(retry_after),
        )));

    let AuthorizationOutcome::Unavailable(unavailable) =
        resolve_at(&provider, &request, NOW, provider_timeout()).await
    else {
        panic!("provider unavailability must remain fail closed");
    };
    assert_eq!(
        unavailable.reason(),
        ProviderUnavailableReason::TemporarilyUnavailable
    );
    assert_eq!(unavailable.retry_after(), Some(retry_after));
}

#[tokio::test]
async fn provider_call_deadline_returns_timeout_unavailability() {
    let actor = Keys::generate();
    let request = direct_request(&actor);
    let calls = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let provider = PendingProvider {
        calls: Arc::clone(&calls),
        dropped: Arc::clone(&dropped),
    };
    let timeout =
        ProviderTimeout::new(Duration::from_millis(1)).expect("synthetic timeout is finite");

    let AuthorizationOutcome::Unavailable(unavailable) =
        resolve_at(&provider, &request, NOW, timeout).await
    else {
        panic!("provider timeout must remain fail closed");
    };
    assert_eq!(unavailable.reason(), ProviderUnavailableReason::Timeout);
    assert_eq!(unavailable.retry_after(), None);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn provider_freshness_is_evaluated_after_async_io() {
    let actor = Keys::generate();
    let request = direct_request(&actor);
    let clock = TestClock::at(NOW);
    let provider = AdvancingProvider::returning_at(
        allow_for(
            &request,
            request.requested_capabilities().clone(),
            "version-a",
            NOW,
            105,
        ),
        clock.clone(),
        105,
    );

    let AuthorizationOutcome::Deny(denial) = resolve_authorization(
        &provider,
        &request,
        &clock,
        provider_timeout(),
        Uuid::from_u128(99),
    )
    .await
    else {
        panic!("a provider decision stale after I/O must deny");
    };
    assert_eq!(denial.reason(), AuthorizationDenialReason::StaleDecision);
    assert_eq!(clock.reads(), 1);
}

#[tokio::test]
async fn federated_policy_expiry_is_evaluated_after_async_io() {
    let actor = Keys::generate();
    let proof = VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        None,
    )
    .expect("synthetic proof is valid");
    let assertion = VerifiedFederatedAssertion::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        principal(),
        Some(VerifiedKeyAttestation::new(actor.public_key())),
        AssertionTransport::TrustedProxy,
        None,
        AssertionExpiry::new(180).expect("synthetic assertion expiry is valid"),
    );
    let policy = federated_policy_with(
        1,
        Uuid::from_u128(20),
        7,
        EnrollmentMode::Provisioned,
        1,
        105,
    );
    let request = AuthorizationRequest::direct(
        &proof,
        &assertion,
        policy,
        capabilities(&[AuthorizationCapability::CommunityRead]),
        Uuid::from_u128(20),
        NOW,
    )
    .expect("federated policy is current when provider I/O begins");
    let clock = TestClock::at(NOW);
    let provider = AdvancingProvider::returning_at(
        allow_for(
            &request,
            request.requested_capabilities().clone(),
            "capability-policy-v1",
            NOW,
            180,
        ),
        clock.clone(),
        105,
    );

    let AuthorizationOutcome::Deny(denial) = resolve_authorization(
        &provider,
        &request,
        &clock,
        provider_timeout(),
        Uuid::from_u128(99),
    )
    .await
    else {
        panic!("federated enrollment policy expired after I/O must deny");
    };
    assert_eq!(
        denial.reason(),
        AuthorizationDenialReason::FederatedPolicyNotCurrent
    );
    assert_eq!(clock.reads(), 1);
}

#[tokio::test]
async fn snapshot_requires_exact_enrollment_policy_lineage() {
    let actor = Keys::generate();
    let proof = VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        None,
    )
    .expect("synthetic proof is valid");
    let assertion = VerifiedFederatedAssertion::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        principal(),
        Some(VerifiedKeyAttestation::new(actor.public_key())),
        AssertionTransport::TrustedProxy,
        None,
        AssertionExpiry::new(180).expect("synthetic assertion expiry is valid"),
    );
    let current_policy = federated_policy_with(
        1,
        Uuid::from_u128(20),
        7,
        EnrollmentMode::Provisioned,
        1,
        160,
    );
    let request = AuthorizationRequest::direct(
        &proof,
        &assertion,
        current_policy,
        capabilities(&[AuthorizationCapability::CommunityRead]),
        Uuid::from_u128(20),
        NOW,
    )
    .expect("current policy can enter provider evaluation");
    let provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "6",
        90,
        180,
    ));
    let AuthorizationOutcome::Allow(snapshot) =
        resolve_at(&provider, &request, NOW, provider_timeout()).await
    else {
        panic!("current provider and enrollment policy must allow");
    };

    let stale_tofu_policy =
        federated_policy_with(1, Uuid::from_u128(20), 6, EnrollmentMode::Tofu, 1, 160);
    let current_policy_for_comparison = federated_policy_with(
        1,
        Uuid::from_u128(20),
        7,
        EnrollmentMode::Provisioned,
        1,
        160,
    );
    assert!(snapshot.is_bound_to_federated_policy(&current_policy_for_comparison));
    assert!(!snapshot.is_bound_to_federated_policy(&stale_tofu_policy));
    assert_eq!(snapshot.policy_version().as_str(), "6");
    assert_ne!(
        snapshot.policy_version().as_str(),
        snapshot.federated_policy().epoch().to_string()
    );
}

#[tokio::test]
async fn enrollment_policy_bounds_snapshot_effective_interval() {
    let actor = Keys::generate();
    let proof = VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        None,
    )
    .expect("synthetic proof is valid");
    let assertion = VerifiedFederatedAssertion::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        principal(),
        Some(VerifiedKeyAttestation::new(actor.public_key())),
        AssertionTransport::TrustedProxy,
        None,
        AssertionExpiry::new(180).expect("synthetic assertion expiry is valid"),
    );
    let policy = federated_policy_with(
        1,
        Uuid::from_u128(20),
        7,
        EnrollmentMode::Provisioned,
        1,
        150,
    );
    let request = AuthorizationRequest::direct(
        &proof,
        &assertion,
        policy,
        capabilities(&[AuthorizationCapability::CommunityRead]),
        Uuid::from_u128(20),
        NOW,
    )
    .expect("current policy can enter provider evaluation");
    let provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "capability-policy-v1",
        90,
        170,
    ));
    let AuthorizationOutcome::Allow(snapshot) =
        resolve_at(&provider, &request, NOW, provider_timeout()).await
    else {
        panic!("current bounded policy must allow");
    };

    assert_eq!(request.evidence_valid_until(), 150);
    assert_eq!(snapshot.effective_until(), 150);
}

#[tokio::test]
async fn identity_evidence_is_evaluated_after_async_io() {
    let actor = Keys::generate();
    let request = direct_request_with_expiry(
        &actor,
        105,
        capabilities(&[AuthorizationCapability::CommunityRead]),
    );
    let clock = TestClock::at(NOW);
    let provider = AdvancingProvider::returning_at(
        allow_for(
            &request,
            request.requested_capabilities().clone(),
            "version-a",
            NOW,
            180,
        ),
        clock.clone(),
        105,
    );

    let AuthorizationOutcome::Deny(denial) = resolve_authorization(
        &provider,
        &request,
        &clock,
        provider_timeout(),
        Uuid::from_u128(99),
    )
    .await
    else {
        panic!("identity evidence expired after I/O must deny");
    };
    assert_eq!(
        denial.reason(),
        AuthorizationDenialReason::IdentityEvidenceExpired
    );
}

#[tokio::test]
async fn owner_binding_expiry_is_evaluated_after_async_io() {
    let delegate = Keys::generate();
    let owner = Keys::generate();
    let proof = delegated_proof(&delegate, &owner, 140);
    let binding = existing_binding_with_expiry_in(1, &owner, Some(105));
    let request = AuthorizationRequest::delegated(
        &proof,
        &binding,
        federated_policy(),
        capabilities(&[AuthorizationCapability::CommunityRead]),
        Uuid::from_u128(20),
        NOW,
    )
    .expect("owner binding is current at request construction");
    assert_eq!(request.evidence_valid_until(), 105);

    let clock = TestClock::at(NOW);
    let provider = AdvancingProvider::returning_at(
        allow_for(
            &request,
            request.requested_capabilities().clone(),
            "version-a",
            NOW,
            180,
        ),
        clock.clone(),
        105,
    );
    let AuthorizationOutcome::Deny(denial) = resolve_authorization(
        &provider,
        &request,
        &clock,
        provider_timeout(),
        Uuid::from_u128(99),
    )
    .await
    else {
        panic!("owner binding expired after provider I/O must deny");
    };
    assert_eq!(
        denial.reason(),
        AuthorizationDenialReason::IdentityEvidenceExpired
    );
}

#[test]
fn delegated_request_rejects_owner_binding_at_exact_expiry() {
    let delegate = Keys::generate();
    let owner = Keys::generate();
    let proof = delegated_proof(&delegate, &owner, 140);
    let binding = existing_binding_with_expiry_in(1, &owner, Some(NOW));

    let error = AuthorizationRequest::delegated(
        &proof,
        &binding,
        federated_policy(),
        capabilities(&[AuthorizationCapability::CommunityRead]),
        Uuid::from_u128(20),
        NOW,
    )
    .expect_err("expired owner binding must not enter provider evaluation");
    assert_eq!(error, ProviderContractError::BindingExpired);
}

#[tokio::test]
async fn decision_issued_during_async_io_is_not_false_future() {
    let actor = Keys::generate();
    let request = direct_request(&actor);
    let clock = TestClock::at(NOW);
    let provider = AdvancingProvider::returning_at(
        allow_for(
            &request,
            request.requested_capabilities().clone(),
            "version-a",
            104,
            180,
        ),
        clock.clone(),
        105,
    );

    assert!(matches!(
        resolve_authorization(
            &provider,
            &request,
            &clock,
            provider_timeout(),
            Uuid::from_u128(99)
        )
        .await,
        AuthorizationOutcome::Allow(_)
    ));
}

#[tokio::test]
async fn clock_failure_after_provider_io_is_unavailable() {
    let actor = Keys::generate();
    let request = direct_request(&actor);
    let clock = TestClock::at(NOW);
    let provider = AdvancingProvider::returning_with_clock_failure(
        allow_for(
            &request,
            request.requested_capabilities().clone(),
            "version-a",
            NOW,
            180,
        ),
        clock.clone(),
    );

    let AuthorizationOutcome::Unavailable(unavailable) = resolve_authorization(
        &provider,
        &request,
        &clock,
        provider_timeout(),
        Uuid::from_u128(99),
    )
    .await
    else {
        panic!("unavailable decision time must fail closed");
    };
    assert_eq!(
        unavailable.reason(),
        ProviderUnavailableReason::DependencyUnavailable
    );
}

#[tokio::test]
async fn stale_and_future_provider_decisions_deny() {
    let actor = Keys::generate();
    let request = direct_request(&actor);
    let stale = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        80,
        90,
    ));
    let AuthorizationOutcome::Deny(stale_denial) =
        resolve_at(&stale, &request, NOW, provider_timeout()).await
    else {
        panic!("stale decision must deny");
    };
    assert_eq!(
        stale_denial.reason(),
        AuthorizationDenialReason::StaleDecision
    );

    let future = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        110,
        180,
    ));
    let AuthorizationOutcome::Deny(future_denial) =
        resolve_at(&future, &request, NOW, provider_timeout()).await
    else {
        panic!("future decision must deny");
    };
    assert_eq!(
        future_denial.reason(),
        AuthorizationDenialReason::FutureDecision
    );
}

#[tokio::test]
async fn provider_time_boundaries_and_current_assertion_are_exact() {
    let actor = Keys::generate();
    let request = direct_request_for_transport(
        &actor,
        AuthTransport::RelayWebSocket,
        AuthMethod::Nip42,
        Some(NOW),
        200,
        capabilities(&[AuthorizationCapability::CommunityRead]),
    )
    .expect("assertion with not-before equal to server time is current");

    let issued_now = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        NOW,
        180,
    ));
    assert!(matches!(
        resolve_at(&issued_now, &request, NOW, provider_timeout()).await,
        AuthorizationOutcome::Allow(_)
    ));

    let stale_at_now = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        90,
        NOW,
    ));
    let AuthorizationOutcome::Deny(denial) =
        resolve_at(&stale_at_now, &request, NOW, provider_timeout()).await
    else {
        panic!("freshness ending at server time must deny");
    };
    assert_eq!(denial.reason(), AuthorizationDenialReason::StaleDecision);
}

#[tokio::test]
async fn domain_principal_and_capability_mismatches_deny() {
    let actor = Keys::generate();
    let request = direct_request(&actor);

    let wrong_domain = FakeProvider::returning(ProviderDecision::Allow(
        ProviderAllow::new(
            domain(2),
            request.principal().clone(),
            profile(),
            request.requested_capabilities().clone(),
            policy_version("version-a"),
            90,
            180,
        )
        .expect("synthetic provider allow is structurally valid"),
    ));
    let AuthorizationOutcome::Deny(denial) =
        resolve_at(&wrong_domain, &request, NOW, provider_timeout()).await
    else {
        panic!("cross-domain decision must deny");
    };
    assert_eq!(
        denial.reason(),
        AuthorizationDenialReason::AuthorizationDomainMismatch
    );

    let wrong_principal = FakeProvider::returning(ProviderDecision::Allow(
        ProviderAllow::new(
            domain(1),
            FederatedPrincipal::new("https://idp.example", "other-subject")
                .expect("synthetic principal is valid"),
            profile(),
            request.requested_capabilities().clone(),
            policy_version("version-a"),
            90,
            180,
        )
        .expect("synthetic provider allow is structurally valid"),
    ));
    let AuthorizationOutcome::Deny(denial) =
        resolve_at(&wrong_principal, &request, NOW, provider_timeout()).await
    else {
        panic!("principal mismatch must deny");
    };
    assert_eq!(
        denial.reason(),
        AuthorizationDenialReason::PrincipalMismatch
    );

    let wrong_profile = FakeProvider::returning(ProviderDecision::Allow(
        ProviderAllow::new(
            domain(1),
            request.principal().clone(),
            AuthorizationProfileId::from_server_configuration("other-profile")
                .expect("synthetic profile is valid"),
            request.requested_capabilities().clone(),
            policy_version("version-a"),
            90,
            180,
        )
        .expect("synthetic provider allow is structurally valid"),
    ));
    let AuthorizationOutcome::Deny(denial) =
        resolve_at(&wrong_profile, &request, NOW, provider_timeout()).await
    else {
        panic!("profile mismatch must deny");
    };
    assert_eq!(
        denial.reason(),
        AuthorizationDenialReason::AuthorizationProfileMismatch
    );

    let missing_capability = FakeProvider::returning(allow_for(
        &request,
        capabilities(&[AuthorizationCapability::CommunityWrite]),
        "version-a",
        90,
        180,
    ));
    let AuthorizationOutcome::Deny(denial) =
        resolve_at(&missing_capability, &request, NOW, provider_timeout()).await
    else {
        panic!("missing capability must deny");
    };
    assert_eq!(
        denial.reason(),
        AuthorizationDenialReason::MissingCapability
    );
}

#[tokio::test]
async fn invite_mint_does_not_authorize_invite_claim() {
    let actor = Keys::generate();
    let request = direct_request_with_expiry(
        &actor,
        200,
        capabilities(&[AuthorizationCapability::InviteClaim]),
    );
    let provider = FakeProvider::returning(allow_for(
        &request,
        capabilities(&[AuthorizationCapability::InviteMint]),
        "version-a",
        90,
        180,
    ));

    let AuthorizationOutcome::Deny(denial) =
        resolve_at(&provider, &request, NOW, provider_timeout()).await
    else {
        panic!("invitation minting must not authorize a claim");
    };
    assert_eq!(
        denial.reason(),
        AuthorizationDenialReason::MissingCapability
    );
}

#[test]
fn capability_sets_are_normalized_and_deduplicated() {
    let normalized = CapabilitySet::new(vec![
        AuthorizationCapability::GitWrite,
        AuthorizationCapability::CommunityRead,
        AuthorizationCapability::GitWrite,
        AuthorizationCapability::CommunityRead,
    ])
    .expect("synthetic capabilities are non-empty");

    assert_eq!(
        normalized.as_slice(),
        &[
            AuthorizationCapability::CommunityRead,
            AuthorizationCapability::GitWrite,
        ]
    );
}

#[tokio::test]
async fn no_distinct_capability_authorizes_another_capability() {
    let actor = Keys::generate();
    for requested in all_capabilities() {
        for granted in all_capabilities() {
            if requested == granted {
                continue;
            }

            let request = direct_request_with_expiry(&actor, 200, capabilities(&[requested]));
            let provider = FakeProvider::returning(allow_for(
                &request,
                capabilities(&[granted]),
                "version-a",
                90,
                180,
            ));
            let AuthorizationOutcome::Deny(denial) =
                resolve_at(&provider, &request, NOW, provider_timeout()).await
            else {
                panic!("a distinct capability must not widen provider authority");
            };
            assert_eq!(
                denial.reason(),
                AuthorizationDenialReason::MissingCapability
            );
        }
    }
}

#[tokio::test]
async fn assertion_expiry_bounds_provider_freshness() {
    let actor = Keys::generate();
    let request = direct_request_with_expiry(
        &actor,
        120,
        capabilities(&[AuthorizationCapability::CommunityRead]),
    );
    let provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        90,
        180,
    ));

    let AuthorizationOutcome::Allow(snapshot) =
        resolve_at(&provider, &request, NOW, provider_timeout()).await
    else {
        panic!("current bounded policy must allow");
    };
    assert_eq!(snapshot.fresh_until(), 180);
    assert_eq!(snapshot.effective_until(), 120);
}

#[tokio::test]
async fn identity_evidence_expiring_during_provider_resolution_denies() {
    let actor = Keys::generate();
    let direct = direct_request_with_expiry(
        &actor,
        120,
        capabilities(&[AuthorizationCapability::CommunityRead]),
    );
    let direct_provider = FakeProvider::returning(allow_for(
        &direct,
        direct.requested_capabilities().clone(),
        "version-a",
        110,
        180,
    ));
    let AuthorizationOutcome::Deny(direct_denial) =
        resolve_at(&direct_provider, &direct, 120, provider_timeout()).await
    else {
        panic!("assertion expiring during provider resolution must deny");
    };
    assert_eq!(
        direct_denial.reason(),
        AuthorizationDenialReason::IdentityEvidenceExpired
    );

    let delegate = Keys::generate();
    let owner = Keys::generate();
    let delegated = delegated_request(&delegate, &owner, 140);
    let delegated_provider = FakeProvider::returning(allow_for(
        &delegated,
        delegated.requested_capabilities().clone(),
        "version-a",
        130,
        180,
    ));
    let AuthorizationOutcome::Deny(delegated_denial) =
        resolve_at(&delegated_provider, &delegated, 140, provider_timeout()).await
    else {
        panic!("delegation expiring during provider resolution must deny");
    };
    assert_eq!(
        delegated_denial.reason(),
        AuthorizationDenialReason::IdentityEvidenceExpired
    );
}

#[tokio::test]
async fn delegated_owner_admission_does_not_require_owner_assertion() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let request = delegated_request(&actor, &owner, 140);
    assert!(matches!(
        request.authority(),
        AuthorizationAuthority::Delegated { owner_pubkey, .. }
            if *owner_pubkey == owner.public_key()
    ));
    assert_eq!(
        request.decision_source(),
        DecisionSource::DelegatedOwnerBinding
    );

    let provider = FakeProvider::returning(allow_for(
        &request,
        request.requested_capabilities().clone(),
        "version-a",
        90,
        180,
    ));
    let AuthorizationOutcome::Allow(snapshot) =
        resolve_at(&provider, &request, NOW, provider_timeout()).await
    else {
        panic!("current owner admission must allow delegated authority");
    };
    assert_eq!(snapshot.effective_until(), 140);
    assert_eq!(snapshot.actor_pubkey(), actor.public_key());
    assert_eq!(snapshot.owner_pubkey(), Some(owner.public_key()));
    assert_eq!(snapshot.binding_id(), Some(Uuid::from_u128(10)));
    assert_eq!(snapshot.binding_version(), Some(BindingVersion::INITIAL));
    assert_eq!(snapshot.transport(), AuthTransport::RelayWebSocket);
}

#[tokio::test]
async fn policy_versions_detect_equality_and_change_without_ordering() {
    let actor = Keys::generate();
    let request_a = direct_request(&actor);
    let provider_a = FakeProvider::returning(allow_for(
        &request_a,
        request_a.requested_capabilities().clone(),
        "opaque-a",
        90,
        180,
    ));
    let AuthorizationOutcome::Allow(snapshot_a) =
        resolve_at(&provider_a, &request_a, NOW, provider_timeout()).await
    else {
        panic!("current provider policy must allow");
    };

    let request_b = direct_request(&actor);
    let provider_b = FakeProvider::returning(allow_for(
        &request_b,
        request_b.requested_capabilities().clone(),
        "opaque-b",
        90,
        180,
    ));
    let AuthorizationOutcome::Allow(snapshot_b) =
        resolve_at(&provider_b, &request_b, NOW, provider_timeout()).await
    else {
        panic!("current provider policy must allow");
    };

    assert_ne!(snapshot_a.policy_version(), snapshot_b.policy_version());
    assert_eq!(snapshot_a.policy_version(), &policy_version("opaque-a"));
}

#[test]
fn provider_contract_rejects_malformed_values() {
    assert_eq!(
        CapabilitySet::new(Vec::new()),
        Err(ProviderContractError::EmptyCapabilitySet)
    );
    assert_eq!(
        AuthorizationProfileId::from_server_configuration(""),
        Err(ProviderContractError::EmptyProfileId)
    );
    assert_eq!(
        AuthorizationProfileId::from_server_configuration("x".repeat(MAX_OPAQUE_ID_BYTES + 1)),
        Err(ProviderContractError::ProfileIdTooLong)
    );
    assert!(
        AuthorizationProfileId::from_server_configuration("x".repeat(MAX_OPAQUE_ID_BYTES)).is_ok()
    );
    assert_eq!(
        PolicyVersion::new(""),
        Err(ProviderContractError::EmptyPolicyVersion)
    );
    assert_eq!(
        PolicyVersion::new("x".repeat(MAX_OPAQUE_ID_BYTES + 1)),
        Err(ProviderContractError::PolicyVersionTooLong)
    );
    assert!(PolicyVersion::new("x".repeat(MAX_OPAQUE_ID_BYTES)).is_ok());
    assert_eq!(
        RetryAfter::new(0),
        Err(ProviderContractError::InvalidRetryAfter)
    );
    assert_eq!(
        RetryAfter::new(MAX_RETRY_AFTER_SECONDS + 1),
        Err(ProviderContractError::InvalidRetryAfter)
    );
    assert_eq!(
        RetryAfter::new(MAX_RETRY_AFTER_SECONDS)
            .expect("maximum retry hint is valid")
            .seconds(),
        MAX_RETRY_AFTER_SECONDS
    );
    assert_eq!(
        ProviderTimeout::new(Duration::ZERO),
        Err(ProviderContractError::InvalidProviderTimeout)
    );
    assert_eq!(
        ProviderTimeout::new(MAX_PROVIDER_TIMEOUT + Duration::from_nanos(1)),
        Err(ProviderContractError::InvalidProviderTimeout)
    );
    assert_eq!(
        ProviderTimeout::new(MAX_PROVIDER_TIMEOUT)
            .expect("maximum provider timeout is valid")
            .duration(),
        MAX_PROVIDER_TIMEOUT
    );
    assert_eq!(
        ProviderAllow::new(
            domain(1),
            principal(),
            profile(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            policy_version("version-a"),
            0,
            180,
        ),
        Err(ProviderContractError::InvalidIssuedAt)
    );
    assert_eq!(
        ProviderAllow::new(
            domain(1),
            principal(),
            profile(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            policy_version("version-a"),
            100,
            100,
        ),
        Err(ProviderContractError::InvalidFreshnessBound)
    );
    assert_eq!(
        ProviderAllow::new(
            domain(1),
            principal(),
            profile(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            policy_version("version-a"),
            100,
            99,
        ),
        Err(ProviderContractError::InvalidFreshnessBound)
    );
    assert_eq!(
        ProviderAllow::new(
            domain(1),
            principal(),
            profile(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            policy_version("version-a"),
            100,
            100 + MAX_PROVIDER_FRESHNESS_SECONDS + 1,
        ),
        Err(ProviderContractError::FreshnessWindowTooLong)
    );
    assert!(ProviderAllow::new(
        domain(1),
        principal(),
        profile(),
        capabilities(&[AuthorizationCapability::CommunityRead]),
        policy_version("version-a"),
        100,
        100 + MAX_PROVIDER_FRESHNESS_SECONDS,
    )
    .is_ok());
}

#[test]
fn request_construction_rechecks_verified_bounds_and_relationships() {
    let actor = Keys::generate();
    let proof = VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        None,
    )
    .expect("synthetic proof is valid");
    let expired = VerifiedFederatedAssertion::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        principal(),
        Some(VerifiedKeyAttestation::new(actor.public_key())),
        AssertionTransport::TrustedProxy,
        None,
        AssertionExpiry::new(NOW).expect("synthetic expiry is valid"),
    );
    assert_eq!(
        AuthorizationRequest::direct(
            &proof,
            &expired,
            federated_policy(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            Uuid::nil(),
            NOW,
        ),
        Err(ProviderContractError::InvalidCorrelationId)
    );
    assert_eq!(
        AuthorizationRequest::direct(
            &proof,
            &expired,
            federated_policy(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            Uuid::from_u128(20),
            NOW,
        ),
        Err(ProviderContractError::AssertionExpired)
    );

    let future = VerifiedFederatedAssertion::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        principal(),
        Some(VerifiedKeyAttestation::new(actor.public_key())),
        AssertionTransport::TrustedProxy,
        Some(AssertionNotBefore::new(NOW + 1)),
        AssertionExpiry::new(NOW + 20).expect("synthetic expiry is valid"),
    );
    assert_eq!(
        AuthorizationRequest::direct(
            &proof,
            &future,
            federated_policy(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            Uuid::from_u128(20),
            NOW,
        ),
        Err(ProviderContractError::AssertionNotYetValid)
    );
}

#[test]
fn request_construction_rejects_non_current_or_mismatched_federated_policy() {
    let actor = Keys::generate();
    let proof = VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        None,
    )
    .expect("synthetic proof is valid");
    let assertion = VerifiedFederatedAssertion::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        principal(),
        Some(VerifiedKeyAttestation::new(actor.public_key())),
        AssertionTransport::TrustedProxy,
        None,
        AssertionExpiry::new(180).expect("synthetic assertion expiry is valid"),
    );
    let request_with = |policy: ResolvedFederatedPolicy| {
        AuthorizationRequest::direct(
            &proof,
            &assertion,
            policy,
            capabilities(&[AuthorizationCapability::CommunityRead]),
            Uuid::from_u128(20),
            NOW,
        )
    };

    let wrong_domain = federated_policy_with(
        2,
        Uuid::from_u128(20),
        1,
        EnrollmentMode::Provisioned,
        1,
        180,
    );
    assert_eq!(
        request_with(wrong_domain),
        Err(ProviderContractError::FederatedPolicyDomainMismatch)
    );
    let wrong_correlation = federated_policy_with(
        1,
        Uuid::from_u128(21),
        1,
        EnrollmentMode::Provisioned,
        1,
        180,
    );
    assert_eq!(
        request_with(wrong_correlation),
        Err(ProviderContractError::FederatedPolicyCorrelationMismatch)
    );
    let future = federated_policy_with(
        1,
        Uuid::from_u128(20),
        1,
        EnrollmentMode::Provisioned,
        NOW + 1,
        180,
    );
    assert_eq!(
        request_with(future),
        Err(ProviderContractError::FederatedPolicyNotYetEffective)
    );
    let expired = federated_policy_with(
        1,
        Uuid::from_u128(20),
        1,
        EnrollmentMode::Provisioned,
        1,
        NOW,
    );
    assert_eq!(
        request_with(expired),
        Err(ProviderContractError::FederatedPolicyExpired)
    );
}

#[test]
fn request_construction_rejects_mismatched_verified_evidence() {
    let actor = Keys::generate();
    let owner = Keys::generate();
    let other = Keys::generate();
    let proof = VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        None,
    )
    .expect("synthetic proof is valid");

    let assertion_in_domain =
        |domain_value, transport, attested_pubkey: Option<nostr::PublicKey>| {
            VerifiedFederatedAssertion::new(
                domain(domain_value),
                transport,
                principal(),
                attested_pubkey.map(VerifiedKeyAttestation::new),
                AssertionTransport::TrustedProxy,
                None,
                AssertionExpiry::new(NOW + 20).expect("synthetic expiry is valid"),
            )
        };
    let request = |proof: &VerifiedNostrProof, assertion: &VerifiedFederatedAssertion| {
        AuthorizationRequest::direct(
            proof,
            assertion,
            federated_policy(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            Uuid::from_u128(20),
            NOW,
        )
    };

    assert_eq!(
        request(
            &proof,
            &assertion_in_domain(2, AuthTransport::RelayWebSocket, None),
        ),
        Err(ProviderContractError::AuthorizationDomainMismatch)
    );
    assert_eq!(
        request(
            &proof,
            &assertion_in_domain(1, AuthTransport::HttpBridge, None),
        ),
        Err(ProviderContractError::TransportMismatch)
    );
    assert_eq!(
        request(
            &proof,
            &assertion_in_domain(1, AuthTransport::RelayWebSocket, Some(other.public_key()),),
        ),
        Err(ProviderContractError::KeyAttestationMismatch)
    );
    assert!(request(
        &proof,
        &assertion_in_domain(1, AuthTransport::RelayWebSocket, None),
    )
    .is_ok());

    let delegation = VerifiedTransportDelegation::new_unrestricted(
        owner.public_key(),
        actor.public_key(),
        Some(DelegationExpiry::new(NOW + 20).expect("synthetic expiry is valid")),
    )
    .expect("synthetic delegation is valid");
    let delegated_proof = VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        Some(delegation),
    )
    .expect("synthetic proof is valid");
    assert_eq!(
        request(
            &delegated_proof,
            &assertion_in_domain(1, AuthTransport::RelayWebSocket, None),
        ),
        Err(ProviderContractError::DirectRequestHasOwner)
    );

    let delegated_request_from =
        |proof: &VerifiedNostrProof, binding: &AuthoritativeBindingResolution| {
            AuthorizationRequest::delegated(
                proof,
                binding,
                federated_policy(),
                capabilities(&[AuthorizationCapability::CommunityRead]),
                Uuid::from_u128(20),
                NOW,
            )
        };
    assert_eq!(
        AuthorizationRequest::delegated(
            &delegated_proof,
            &existing_binding(&owner),
            federated_policy(),
            capabilities(&[AuthorizationCapability::CommunityRead]),
            Uuid::nil(),
            NOW,
        ),
        Err(ProviderContractError::InvalidCorrelationId)
    );
    assert_eq!(
        delegated_request_from(&proof, &existing_binding(&owner)),
        Err(ProviderContractError::DelegationRequired)
    );
    assert_eq!(
        delegated_request_from(&delegated_proof, &existing_binding(&other)),
        Err(ProviderContractError::DelegatedOwnerMismatch)
    );
    assert_eq!(
        delegated_request_from(&delegated_proof, &existing_binding_in(2, &owner)),
        Err(ProviderContractError::AuthorizationDomainMismatch)
    );

    let expired_delegation = VerifiedTransportDelegation::new_unrestricted(
        owner.public_key(),
        actor.public_key(),
        Some(DelegationExpiry::new(NOW).expect("synthetic expiry is valid")),
    )
    .expect("synthetic delegation is valid");
    let expired_proof = VerifiedNostrProof::new(
        domain(1),
        AuthTransport::RelayWebSocket,
        actor.public_key(),
        AuthMethod::Nip42,
        Some(expired_delegation),
    )
    .expect("synthetic proof is valid");
    assert_eq!(
        delegated_request_from(&expired_proof, &existing_binding(&owner)),
        Err(ProviderContractError::DelegationExpired)
    );
}

#[tokio::test]
async fn request_decision_snapshot_and_errors_are_redaction_safe() {
    let actor = Keys::generate();
    let request = direct_request(&actor);
    let request_debug = concat!(
        "AuthorizationRequest { authorization_domain: \"[redacted]\", ",
        "transport: \"[redacted]\", actor_pubkey: \"[redacted]\", ",
        "proof_method: \"[redacted]\", ",
        "authority: \"[redacted]\", principal: \"[redacted]\", ",
        "key_attested: \"[redacted]\", assertion_transport: \"[redacted]\", ",
        "assertion_not_before: \"[redacted]\", ",
        "assertion_expires_at: \"[redacted]\", ",
        "federated_policy: \"[redacted]\", ",
        "requested_capabilities: \"[redacted]\", ",
        "correlation_id: \"[redacted]\", decision_source: \"[redacted]\", ",
        "evidence_valid_from: \"[redacted]\", ",
        "evidence_valid_until: \"[redacted]\" }"
    );
    // Keep this exact-shape assertion deliberately: adding a field must fail until
    // the disclosure contract explicitly confirms that the new field is redacted.
    assert_eq!(format!("{request:?}"), request_debug);

    let delegate = Keys::generate();
    let owner = Keys::generate();
    let delegated_request = delegated_request(&delegate, &owner, 180);
    assert_eq!(format!("{delegated_request:?}"), request_debug);
    assert_eq!(
        format!("{:?}", request.authority()),
        "AuthorizationAuthority(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", delegated_request.authority()),
        "AuthorizationAuthority(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", request.decision_source()),
        "DecisionSource(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", delegated_request.decision_source()),
        "DecisionSource(\"[redacted]\")"
    );

    let allow = ProviderAllow::new(
        request.authorization_domain(),
        request.principal().clone(),
        profile(),
        request.requested_capabilities().clone(),
        policy_version("private-policy-version"),
        90,
        180,
    )
    .expect("synthetic provider allow is structurally valid");
    assert_eq!(
        format!("{allow:?}"),
        concat!(
            "ProviderAllow { authorization_domain: \"[redacted]\", ",
            "principal: \"[redacted]\", profile_id: \"[redacted]\", ",
            "capabilities: \"[redacted]\", policy_version: \"[redacted]\", ",
            "issued_at: \"[redacted]\", fresh_until: \"[redacted]\" }"
        )
    );
    let decision = ProviderDecision::Allow(allow);
    assert_eq!(format!("{decision:?}"), "ProviderDecision(\"[redacted]\")");
    let provider = FakeProvider::returning(decision);
    let outcome = resolve_at(&provider, &request, NOW, provider_timeout()).await;
    assert_eq!(
        format!("{outcome:?}"),
        "AuthorizationOutcome(\"[redacted]\")"
    );
    let AuthorizationOutcome::Allow(snapshot) = outcome else {
        panic!("current provider policy must allow");
    };
    assert_eq!(
        format!("{snapshot:?}"),
        concat!(
            "CapabilitySnapshot { runtime_binding: \"[redacted]\", ",
            "authorization_domain: \"[redacted]\", ",
            "transport: \"[redacted]\", actor_pubkey: \"[redacted]\", ",
            "owner_pubkey: \"[redacted]\", binding_id: \"[redacted]\", ",
            "binding_version: \"[redacted]\", proof_method: \"[redacted]\", ",
            "principal: \"[redacted]\", ",
            "key_attested: \"[redacted]\", assertion_transport: \"[redacted]\", ",
            "assertion_not_before: \"[redacted]\", ",
            "assertion_expires_at: \"[redacted]\", ",
            "federated_policy: \"[redacted]\", ",
            "profile_id: \"[redacted]\", capabilities: \"[redacted]\", ",
            "policy_version: \"[redacted]\", issued_at: \"[redacted]\", ",
            "fresh_until: \"[redacted]\", effective_from: \"[redacted]\", ",
            "effective_until: \"[redacted]\", ",
            "decision_source: \"[redacted]\", correlation_id: \"[redacted]\", ",
            "reason: \"[redacted]\" }"
        )
    );

    let denial = AuthorizationDenial::new(AuthorizationDenialReason::ProviderDenied);
    assert_eq!(
        format!("{denial:?}"),
        "AuthorizationDenial { reason: \"[redacted]\" }"
    );
    let unavailable = ProviderUnavailable::new(
        ProviderUnavailableReason::DependencyUnavailable,
        Some(RetryAfter::new(30).expect("synthetic retry hint is bounded")),
    );
    assert_eq!(
        format!("{unavailable:?}"),
        concat!(
            "ProviderUnavailable { reason: \"[redacted]\", ",
            "retry_after: \"[redacted]\" }"
        )
    );
    assert_eq!(
        format!(
            "{:?}",
            ProviderDecision::Deny(AuthorizationDenial::new(
                AuthorizationDenialReason::ProviderDenied,
            ))
        ),
        "ProviderDecision(\"[redacted]\")"
    );
    assert_eq!(
        format!(
            "{:?}",
            ProviderDecision::Unavailable(ProviderUnavailable::new(
                ProviderUnavailableReason::DependencyUnavailable,
                None,
            ))
        ),
        "ProviderDecision(\"[redacted]\")"
    );
    assert_eq!(
        format!(
            "{:?}",
            AuthorizationOutcome::Deny(AuthorizationDenial::new(
                AuthorizationDenialReason::ProviderDenied,
            ))
        ),
        "AuthorizationOutcome(\"[redacted]\")"
    );
    assert_eq!(
        format!(
            "{:?}",
            AuthorizationOutcome::Unavailable(ProviderUnavailable::new(
                ProviderUnavailableReason::DependencyUnavailable,
                None,
            ))
        ),
        "AuthorizationOutcome(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", provider_timeout()),
        "ProviderTimeout(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", &profile()),
        "AuthorizationProfileId(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", snapshot.policy_version()),
        "PolicyVersion(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", snapshot.capabilities()),
        "CapabilitySet(\"[redacted]\")"
    );

    for capability in all_capabilities() {
        capability_coverage_is_exhaustive(capability);
        assert_eq!(
            format!("{capability:?}"),
            "AuthorizationCapability(\"[redacted]\")"
        );
    }
    for reason in [
        AuthorizationDenialReason::ProviderDenied,
        AuthorizationDenialReason::AuthorizationDomainMismatch,
        AuthorizationDenialReason::PrincipalMismatch,
        AuthorizationDenialReason::AuthorizationProfileMismatch,
        AuthorizationDenialReason::MissingCapability,
        AuthorizationDenialReason::StaleDecision,
        AuthorizationDenialReason::FutureDecision,
        AuthorizationDenialReason::IdentityEvidenceExpired,
        AuthorizationDenialReason::IdentityEvidenceNotYetValid,
        AuthorizationDenialReason::FederatedPolicyNotCurrent,
    ] {
        assert_eq!(
            format!("{reason:?}"),
            "AuthorizationDenialReason(\"[redacted]\")"
        );
    }
    for reason in [
        ProviderUnavailableReason::TemporarilyUnavailable,
        ProviderUnavailableReason::Timeout,
        ProviderUnavailableReason::DependencyUnavailable,
    ] {
        assert_eq!(
            format!("{reason:?}"),
            "ProviderUnavailableReason(\"[redacted]\")"
        );
    }
    assert_eq!(
        format!("{:?}", ProviderAllowReason::CurrentPolicy),
        "ProviderAllowReason(\"[redacted]\")"
    );
    assert_eq!(
        format!("{:?}", RetryAfter::new(30).expect("retry hint is valid")),
        "RetryAfter(\"[redacted]\")"
    );

    for error in all_contract_errors() {
        for rendered in [error.to_string(), format!("{error:?}")] {
            for private_value in [
                "idp.example",
                "subject-123",
                "profile-1",
                "private-policy-version",
            ] {
                assert!(!rendered.contains(private_value));
            }
        }
    }
}

#[test]
fn provider_trait_is_object_safe_and_codes_are_unique() {
    let provider: Arc<dyn AuthorizationProvider> =
        Arc::new(FakeProvider::returning(ProviderDecision::Deny(
            AuthorizationDenial::new(AuthorizationDenialReason::ProviderDenied),
        )));
    assert!(Arc::strong_count(&provider) == 1);

    let mut codes = vec![
        ProviderAllowReason::CurrentPolicy.code(),
        AuthorizationDenialReason::ProviderDenied.code(),
        AuthorizationDenialReason::AuthorizationDomainMismatch.code(),
        AuthorizationDenialReason::PrincipalMismatch.code(),
        AuthorizationDenialReason::AuthorizationProfileMismatch.code(),
        AuthorizationDenialReason::MissingCapability.code(),
        AuthorizationDenialReason::StaleDecision.code(),
        AuthorizationDenialReason::FutureDecision.code(),
        AuthorizationDenialReason::IdentityEvidenceExpired.code(),
        AuthorizationDenialReason::IdentityEvidenceNotYetValid.code(),
        AuthorizationDenialReason::FederatedPolicyNotCurrent.code(),
        ProviderUnavailableReason::TemporarilyUnavailable.code(),
        ProviderUnavailableReason::Timeout.code(),
        ProviderUnavailableReason::DependencyUnavailable.code(),
    ];
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), 14);

    let contract_errors = all_contract_errors();
    let mut contract_codes = contract_errors
        .iter()
        .copied()
        .map(ProviderContractError::code)
        .collect::<Vec<_>>();
    contract_codes.sort_unstable();
    contract_codes.dedup();
    assert_eq!(contract_codes.len(), contract_errors.len());
}
