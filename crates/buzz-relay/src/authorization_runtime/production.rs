//! Disabled-by-default production construction for protected authorization.
//!
//! Configuration names exact authorization domains. There is no default
//! domain, provider fallback, request-selected profile, or implicit Enforce.

use std::{collections::HashMap, env, sync::Arc, time::Duration};

use async_trait::async_trait;
use buzz_audit::authorization::{
    ActorReference, AttemptId, AuthorizationEventV1, CorrelationId, DecisionReason, EventId,
    EventKind, EventPayloadV1, EventResult, EvidenceHealthSignal, OperationClass, PseudonymKey,
    Pseudonymizer, ReferenceKind, SourceClass, TransportClass, VersionVectorV1,
};
use buzz_auth::{
    resolve_current_federated_policy, AccessLeasePolicy, ActiveBindingResolution,
    ApplicationLeaseLimit, AuthContextInput, AuthTransport, AuthorizationCapability,
    AuthorizationClockSkew, AuthorizationDenialReason, AuthorizationOutcome,
    AuthorizationProfileId, AuthorizationProvider, BindingLeaseBound, BindingSource, CapabilitySet,
    EnrollmentMode, FederatedAuthorization, LeaseVersion, ProviderTimeout, ResolvedFederatedPolicy,
    Scope, SharedAuthorizationClock, SystemAuthorizationClock, VerificationStatusPolicy,
    VerifiedEvidenceAdapter, VerifiedProviderEvidence,
};
use buzz_core::{CommunityId, TenantContext};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::{
    finalization::{
        AuthorizationMode, DomainAuthorizationPolicy, DomainProviderSelector,
        RelayAuthorizationFinalizer, RuntimeAuthorizationDisposition,
    },
    invalidation::{
        AuthorizationDependencies, AuthorizationInvalidationConfig,
        AuthorizationInvalidationRuntime, AuthorizationObserver,
    },
    transport::{
        DomainTransportPolicy, LeaseCurrentState, LeaseCurrentStateError,
        LeaseCurrentStateObserver, ProtectedAuthorizationResolver, ProtectedOperationRequest,
        ProtectedResolution, ProtectedResolutionError, ProtectedTransportRuntime,
        VerifiedProviderEvidenceResolver,
    },
};

const DOMAINS_ENV: &str = "BUZZ_PROTECTED_AUTHORIZATION_DOMAINS";
const PROFILE_ENV: &str = "BUZZ_PROTECTED_AUTHORIZATION_PROFILE";
const LEASE_SECONDS_ENV: &str = "BUZZ_PROTECTED_AUTHORIZATION_LEASE_SECONDS";
const RESTORE_BOOTSTRAPS_ENV: &str = "BUZZ_PROTECTED_AUTHORIZATION_RESTORE_BOOTSTRAPS";
const AUDIT_PSEUDONYM_KEY_ENV: &str = "BUZZ_AUTHORIZATION_AUDIT_PSEUDONYM_KEY_HEX";
const AUDIT_PSEUDONYM_KEY_EPOCH_ENV: &str = "BUZZ_AUTHORIZATION_AUDIT_PSEUDONYM_KEY_EPOCH";
const MAX_AUDIO_RECONCILIATION_SWEEPS: usize = 8;

/// Runtime plus its durable invalidation worker.
pub struct InstalledProtectedRuntime {
    /// Exact-domain transport runtime.
    pub transport: Arc<ProtectedTransportRuntime>,
    /// Durable invalidation runtime initialized before transport installation.
    pub invalidation: AuthorizationInvalidationRuntime,
    /// Independent high-water witness verified before transport installation.
    pub restore: Arc<super::restore::RestoreProtectionRuntime>,
    /// Whether any exact domain is authoritative Enforce.
    pub enforce_enabled: bool,
    /// Exact authoritative domains whose crash remnants may be reconciled.
    pub enforcing_domains: Vec<CommunityId>,
    /// Enforce domains whose optional public projection requires reconciliation.
    pub projection_domains: Vec<CommunityId>,
}

/// Result of the single production installation boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedRuntimeInstallation {
    /// No exact authorization domains were configured; legacy behavior remains.
    Disabled,
    /// Exact-domain transport, restore, and invalidation runtimes were installed.
    Installed,
}

/// Exact-domain provider registry supplied by the deployment adapter.
///
/// The OSS runtime never invents a current-admission decision. Enforce
/// construction fails when a configured domain has no injected O2 provider.
#[derive(Default)]
pub struct ProductionProviderRegistry {
    providers: HashMap<CommunityId, Arc<dyn AuthorizationProvider>>,
}

impl ProductionProviderRegistry {
    /// Build an exact registry, rejecting duplicate domain mappings.
    pub fn new(
        providers: impl IntoIterator<Item = (CommunityId, Arc<dyn AuthorizationProvider>)>,
    ) -> Result<Self, ProductionRuntimeError> {
        let mut exact = HashMap::new();
        for (domain, provider) in providers {
            if exact.insert(domain, provider).is_some() {
                return Err(ProductionRuntimeError::InvalidConfiguration);
            }
        }
        Ok(Self { providers: exact })
    }

    fn provider_for(
        &self,
        domain: CommunityId,
    ) -> Result<Arc<dyn AuthorizationProvider>, ProductionRuntimeError> {
        self.providers
            .get(&domain)
            .cloned()
            .ok_or(ProductionRuntimeError::ProviderMissing)
    }
}

struct ProductionLeaseObserver {
    invalidation: AuthorizationObserver,
    current: LeaseCurrentState,
}

impl LeaseCurrentStateObserver for ProductionLeaseObserver {
    fn observe_current(&self) -> Result<LeaseCurrentState, LeaseCurrentStateError> {
        self.invalidation
            .recheck()
            .map_err(|_| LeaseCurrentStateError::Stale)?;
        Ok(self.current.clone())
    }

    fn observe_commit_fence(
        &self,
    ) -> Result<super::executor::AuthorizationCommitFence, LeaseCurrentStateError> {
        self.invalidation
            .commit_fence()
            .map_err(|_| LeaseCurrentStateError::Unavailable)
    }
}

struct ProductionResolver {
    db: buzz_db::Db,
    tenants: HashMap<CommunityId, TenantContext>,
    finalizer: RelayAuthorizationFinalizer,
    invalidation: AuthorizationInvalidationRuntime,
    clock: SharedAuthorizationClock,
    profiles: HashMap<CommunityId, AuthorizationProfileId>,
    evidence_pseudonymizer: Option<Pseudonymizer>,
    evidence_health: EvidenceHealthSignal,
}

impl ProductionResolver {
    fn decision_event(
        &self,
        request: &ProtectedOperationRequest,
        outcome: &AuthorizationOutcome,
    ) -> Result<AuthorizationEventV1, ProtectedResolutionError> {
        let domain = request.authorization_domain();
        let pseudonymizer = self
            .evidence_pseudonymizer
            .as_ref()
            .ok_or(ProtectedResolutionError::new("evidence_configuration"))?;
        let actor = pseudonymizer
            .derive(
                domain,
                ReferenceKind::Actor,
                &request.actor_pubkey().to_bytes(),
            )
            .map_err(|_| ProtectedResolutionError::new("evidence_pseudonym"))?;
        let actor = if let Some(delegation) = request.verified_proof().verified_delegation() {
            let owner = pseudonymizer
                .derive(
                    domain,
                    ReferenceKind::Actor,
                    &delegation.owner_pubkey().to_bytes(),
                )
                .map_err(|_| ProtectedResolutionError::new("evidence_pseudonym"))?;
            ActorReference::delegated(actor, owner, delegation.relationship_revision().get())
                .map_err(|_| ProtectedResolutionError::new("evidence_pseudonym"))?
        } else {
            ActorReference::direct(actor)
                .map_err(|_| ProtectedResolutionError::new("evidence_pseudonym"))?
        };
        let delegated = request.owner_pubkey().is_some();
        let (kind, result, reason, versions) = match outcome {
            AuthorizationOutcome::Allow(snapshot) => {
                let mut policy = Sha256::new();
                policy.update(b"buzz-authorization-policy-version-v1");
                policy.update(snapshot.policy_version().as_str().as_bytes());
                (
                    if delegated {
                        EventKind::DelegatedAllowed
                    } else {
                        EventKind::AdmissionAllowed
                    },
                    EventResult::Allowed,
                    DecisionReason::PolicyAllowed,
                    VersionVectorV1 {
                        binding: snapshot.binding_version().map(|value| value.get()),
                        lease: None,
                        lifecycle: None,
                        invalidation: None,
                        policy_digest: Some(policy.finalize().into()),
                    },
                )
            }
            AuthorizationOutcome::Deny(denial) => (
                if delegated {
                    EventKind::DelegatedDenied
                } else {
                    EventKind::AdmissionDenied
                },
                EventResult::Denied,
                decision_reason_for_denial(denial.reason()),
                VersionVectorV1::default(),
            ),
            AuthorizationOutcome::Unavailable(_) => (
                if delegated {
                    EventKind::DelegatedDenied
                } else {
                    EventKind::AdmissionDenied
                },
                EventResult::Unavailable,
                DecisionReason::EvidenceUnavailable,
                VersionVectorV1::default(),
            ),
            _ => (
                if delegated {
                    EventKind::DelegatedDenied
                } else {
                    EventKind::AdmissionDenied
                },
                EventResult::Unavailable,
                DecisionReason::SchemaUnsupported,
                VersionVectorV1::default(),
            ),
        };
        let occurred_at = self
            .clock
            .now()
            .ok()
            .and_then(|time| DateTime::<Utc>::from_timestamp(time.unix_seconds() as i64, 0))
            .ok_or(ProtectedResolutionError::new("authorization_clock"))?;
        let key_reference = pseudonymizer
            .derive(
                domain,
                ReferenceKind::Key,
                &request.actor_pubkey().to_bytes(),
            )
            .map_err(|_| ProtectedResolutionError::new("evidence_pseudonym"))?;
        let principal = match outcome {
            AuthorizationOutcome::Allow(snapshot) => Some(snapshot.principal()),
            _ => request
                .provider_evidence()
                .map(|evidence| evidence.verified_assertion().principal())
                .or_else(|| {
                    request
                        .verified_assertion()
                        .map(|assertion| assertion.principal())
                }),
        };
        let principal_reference = principal
            .map(|principal| {
                let issuer = principal.issuer().as_bytes();
                let subject = principal.subject().as_bytes();
                let mut input = Vec::with_capacity(16 + issuer.len() + subject.len());
                input.extend_from_slice(&(issuer.len() as u64).to_be_bytes());
                input.extend_from_slice(issuer);
                input.extend_from_slice(&(subject.len() as u64).to_be_bytes());
                input.extend_from_slice(subject);
                pseudonymizer.derive(domain, ReferenceKind::Principal, &input)
            })
            .transpose()
            .map_err(|_| ProtectedResolutionError::new("evidence_pseudonym"))?;
        AuthorizationEventV1::new(
            EventId::generate(),
            domain,
            occurred_at,
            None,
            CorrelationId::from_uuid(request.correlation_id())
                .map_err(|_| ProtectedResolutionError::new("evidence_correlation"))?,
            AttemptId::generate(),
            None,
            actor,
            transport_class(request.transport()),
            operation_class(request.capability()),
            SourceClass::Policy,
            kind,
            result,
            reason,
            versions,
            EventPayloadV1::None,
        )
        .with_subject_references(principal_reference, Some(key_reference))
        .map_err(|_| ProtectedResolutionError::new("evidence_pseudonym"))
    }

    async fn accept_outcome(
        &self,
        request: &ProtectedOperationRequest,
        outcome: AuthorizationOutcome,
    ) -> Result<AuthorizationOutcome, ProtectedResolutionError> {
        use super::evidence::{
            accept_authorization_decision, AcceptedAuthorizationDecision, DecisionDisposition,
        };

        let event = self.decision_event(request, &outcome)?;
        let disposition = if matches!(outcome, AuthorizationOutcome::Allow(_)) {
            DecisionDisposition::Allow
        } else {
            DecisionDisposition::Deny
        };
        match accept_authorization_decision(
            &self.db,
            &self.evidence_health,
            &event,
            disposition,
            outcome,
        )
        .await
        {
            Ok(AcceptedAuthorizationDecision::Allow { value, .. }) => Ok(value),
            Ok(AcceptedAuthorizationDecision::Deny { value, evidence }) => {
                if evidence.is_none() {
                    metrics::counter!(
                        "buzz_authorization_evidence_control_total",
                        "code" => "acceptance_unavailable"
                    )
                    .increment(1);
                    tracing::error!(
                        code = "acceptance_unavailable",
                        "authorization denial evidence was unavailable; denial preserved"
                    );
                }
                Ok(value)
            }
            Err(error) => {
                metrics::counter!(
                    "buzz_authorization_evidence_control_total",
                    "code" => "allow_acceptance_unavailable"
                )
                .increment(1);
                tracing::error!(
                    code = error.code(),
                    "authorization allow rejected because durable evidence was unavailable"
                );
                Err(ProtectedResolutionError::new(error.code()))
            }
        }
    }

    fn tenant(&self, domain: CommunityId) -> Result<&TenantContext, ProtectedResolutionError> {
        self.tenants
            .get(&domain)
            .ok_or(ProtectedResolutionError::new("configured_domain_missing"))
    }

    async fn current_policy(
        &self,
        domain: CommunityId,
        correlation_id: uuid::Uuid,
    ) -> Result<ResolvedFederatedPolicy, ProtectedResolutionError> {
        let now = self
            .clock
            .now()
            .map_err(|_| ProtectedResolutionError::new("authorization_clock"))?
            .unix_seconds();
        resolve_current_federated_policy(
            &self.db.federated_authority_adapter(),
            domain,
            correlation_id,
            now,
        )
        .await
        .map_err(|_| ProtectedResolutionError::new("federated_policy_unavailable"))
    }

    async fn active_binding(
        &self,
        domain: CommunityId,
        pubkey: nostr::PublicKey,
        assertion: Option<&buzz_auth::VerifiedFederatedAssertion>,
    ) -> Result<buzz_auth::VersionedBindingRef, ProtectedResolutionError> {
        let binding = self
            .db
            .get_active_identity_binding_by_pubkey(domain, pubkey.as_bytes())
            .await
            .map_err(|_| ProtectedResolutionError::new("binding_unavailable"))?
            .ok_or(ProtectedResolutionError::new("active_binding_required"))?;
        let source = match binding.binding_provenance {
            buzz_db::identity_binding::BindingProvenance::AttestedKey => BindingSource::AttestedKey,
            buzz_db::identity_binding::BindingProvenance::Provisioned => BindingSource::Provisioned,
            buzz_db::identity_binding::BindingProvenance::Tofu => BindingSource::Tofu,
        };
        let expires_at = binding
            .expires_at
            .as_ref()
            .map(|value| u64::try_from(value.timestamp()))
            .transpose()
            .map_err(|_| ProtectedResolutionError::new("binding_expiry_invalid"))?;
        VerifiedEvidenceAdapter::new()
            .active_binding_from_store(
                domain,
                domain,
                binding.binding_id,
                &binding.issuer,
                &binding.uid,
                pubkey,
                binding.binding_version,
                expires_at,
                source,
                ActiveBindingResolution::Existing,
                assertion,
            )
            .map_err(|_| ProtectedResolutionError::new("binding_evidence_mismatch"))
    }

    async fn require_membership(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<(), ProtectedResolutionError> {
        let member = request
            .owner_pubkey()
            .unwrap_or_else(|| request.actor_pubkey());
        match self
            .db
            .is_relay_member(request.authorization_domain(), &member.to_hex())
            .await
        {
            Ok(true) => Ok(()),
            Ok(false) => Err(ProtectedResolutionError::new("membership_required")),
            Err(_) => Err(ProtectedResolutionError::new("membership_unavailable")),
        }
    }

    fn reseal_assertion(
        &self,
        assertion: &buzz_auth::VerifiedFederatedAssertion,
    ) -> Result<buzz_auth::VerifiedFederatedAssertion, ProtectedResolutionError> {
        let now = self
            .clock
            .now()
            .map_err(|_| ProtectedResolutionError::new("authorization_clock"))?
            .unix_seconds();
        VerifiedEvidenceAdapter::new()
            .federated_assertion_from_validated_claims(
                assertion.authorization_domain(),
                assertion.authorized_transport(),
                assertion.principal().issuer(),
                assertion.principal().subject(),
                assertion.key_attestation().map(|key| key.pubkey()),
                assertion.transport(),
                assertion.not_before().map(|bound| bound.unix_seconds()),
                assertion.expires_at().unix_seconds(),
                now,
            )
            .map_err(|_| ProtectedResolutionError::new("assertion_stale"))
    }

    fn normalized_provider_evidence(
        &self,
        request: &ProtectedOperationRequest,
        capabilities: &CapabilitySet,
    ) -> Result<Arc<VerifiedProviderEvidence>, ProtectedResolutionError> {
        let profile = self
            .profiles
            .get(&request.authorization_domain())
            .ok_or(ProtectedResolutionError::new("provider_profile_missing"))?;
        let now = self
            .clock
            .now()
            .map_err(|_| ProtectedResolutionError::new("authorization_clock"))?
            .unix_seconds();

        if let Some(evidence) = request.provider_evidence() {
            evidence
                .validate_for(
                    request.authorization_domain(),
                    request.transport(),
                    profile,
                    capabilities,
                    now,
                )
                .map_err(|_| ProtectedResolutionError::new("provider_evidence_invalid"))?;
            return Ok(Arc::clone(evidence));
        }

        let assertion = request
            .verified_assertion()
            .ok_or(ProtectedResolutionError::new("provider_evidence_missing"))?;
        let assertion = self.reseal_assertion(assertion)?;
        let fresh_until = assertion.expires_at().unix_seconds();
        VerifiedEvidenceAdapter::new()
            .provider_evidence_from_verified_assertion(
                assertion,
                profile.clone(),
                capabilities.clone(),
                now,
                fresh_until,
                now,
            )
            .map(Arc::new)
            .map_err(|_| ProtectedResolutionError::new("provider_evidence_invalid"))
    }
}

#[async_trait]
impl ProtectedAuthorizationResolver for ProductionResolver {
    async fn observe(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<(), ProtectedResolutionError> {
        let tenant = self.tenant(request.authorization_domain())?;
        let fence = self
            .invalidation
            .capture_before_evaluation(request.authorization_domain())
            .await
            .map_err(|_| ProtectedResolutionError::new("invalidation_unavailable"))?;
        self.require_membership(request).await?;
        let capabilities = CapabilitySet::single(request.capability());
        let provider_evidence = self.normalized_provider_evidence(request, &capabilities)?;
        let assertion = provider_evidence.verified_assertion();
        let federated_policy = self
            .current_policy(request.authorization_domain(), request.correlation_id())
            .await?;
        let outcome = if let Some(owner) = request.owner_pubkey() {
            let binding = self
                .active_binding(request.authorization_domain(), owner, Some(assertion))
                .await?;
            self.finalizer
                .evaluate_delegated(
                    tenant,
                    request.verified_proof(),
                    &binding,
                    federated_policy,
                    capabilities.clone(),
                    request.correlation_id(),
                )
                .await
        } else {
            let _binding = self
                .active_binding(
                    request.authorization_domain(),
                    request.actor_pubkey(),
                    Some(assertion),
                )
                .await?;
            self.finalizer
                .evaluate_direct(
                    tenant,
                    request.verified_proof(),
                    assertion,
                    federated_policy,
                    capabilities.clone(),
                    request.correlation_id(),
                )
                .await
        }
        .map_err(|_| ProtectedResolutionError::new("provider_request_invalid"))?;
        let outcome = self.accept_outcome(request, outcome).await?;
        if !matches!(outcome, AuthorizationOutcome::Allow(_)) {
            return Err(ProtectedResolutionError::new("provider_denied"));
        }
        self.invalidation
            .recheck_evaluation(fence)
            .map_err(|_| ProtectedResolutionError::new("invalidation_race"))
    }

    async fn present(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<super::transport::ProtectedStatusResolution, ProtectedResolutionError> {
        if request.owner_pubkey().is_some() {
            return Err(ProtectedResolutionError::new(
                "client_status_requires_direct_binding",
            ));
        }
        let tenant = self.tenant(request.authorization_domain())?;
        let fence = self
            .invalidation
            .capture_before_evaluation(request.authorization_domain())
            .await
            .map_err(|_| ProtectedResolutionError::new("invalidation_unavailable"))?;
        self.require_membership(request).await?;
        let capabilities = CapabilitySet::single(request.capability());
        let provider_evidence = self.normalized_provider_evidence(request, &capabilities)?;
        let assertion = provider_evidence.verified_assertion();
        let binding = self
            .active_binding(
                request.authorization_domain(),
                request.actor_pubkey(),
                Some(assertion),
            )
            .await?;
        let evaluation_policy = self
            .current_policy(request.authorization_domain(), request.correlation_id())
            .await?;
        let outcome = self
            .finalizer
            .evaluate_direct(
                tenant,
                request.verified_proof(),
                assertion,
                evaluation_policy,
                capabilities,
                request.correlation_id(),
            )
            .await
            .map_err(|_| ProtectedResolutionError::new("provider_request_invalid"))?;
        let snapshot = match self.accept_outcome(request, outcome).await? {
            AuthorizationOutcome::Allow(snapshot) => snapshot,
            _ => return Err(ProtectedResolutionError::new("provider_denied")),
        };
        let binding_bound = BindingLeaseBound::new(&binding, snapshot.effective_until())
            .map_err(|_| ProtectedResolutionError::new("binding_bound_invalid"))?;
        let authorization = FederatedAuthorization::Direct {
            binding,
            assertion: self.reseal_assertion(assertion)?,
        };
        let access = VerifiedEvidenceAdapter::new()
            .community_access_from_policy(
                tenant,
                request.authorization_domain(),
                Scope::all_known(),
                None,
            )
            .map_err(|_| ProtectedResolutionError::new("community_access_invalid"))?;
        let input = AuthContextInput::new(
            tenant.clone(),
            request.correlation_id(),
            Arc::clone(request.verified_proof()),
            access,
        );
        let finalization_policy = self
            .current_policy(request.authorization_domain(), request.correlation_id())
            .await?;
        let disposition = self
            .finalizer
            .finalize_client_status(
                input,
                finalization_policy,
                authorization,
                snapshot,
                binding_bound,
            )
            .map_err(|_| ProtectedResolutionError::new("status_finalization"))?;
        let dependencies = AuthorizationDependencies::from_verification_only(
            request.session_target(),
            &disposition,
        )
        .map_err(|_| ProtectedResolutionError::new("invalidation_dependencies"))?;
        let observer = self
            .invalidation
            .observe_authority(fence, dependencies, request.cancellation())
            .map_err(|_| ProtectedResolutionError::new("invalidation_race"))?;
        let current = LeaseCurrentState::from_trusted_runtime(
            disposition.binding_version(),
            disposition.profile_id().clone(),
            disposition.policy_version().clone(),
        );
        Ok(super::transport::ProtectedStatusResolution::new(
            disposition,
            Arc::new(ProductionLeaseObserver {
                invalidation: observer,
                current,
            }),
            fence.generation(),
        ))
    }

    async fn resolve(
        &self,
        request: &ProtectedOperationRequest,
    ) -> Result<ProtectedResolution, ProtectedResolutionError> {
        let tenant = self.tenant(request.authorization_domain())?;
        let fence = self
            .invalidation
            .capture_before_evaluation(request.authorization_domain())
            .await
            .map_err(|_| ProtectedResolutionError::new("invalidation_unavailable"))?;
        let capabilities = CapabilitySet::single(request.capability());
        let provider_evidence = self.normalized_provider_evidence(request, &capabilities)?;
        let assertion = provider_evidence.verified_assertion();

        if request.enrollment_requested() {
            let evaluation_policy = self
                .current_policy(request.authorization_domain(), request.correlation_id())
                .await?;
            let outcome = self
                .finalizer
                .evaluate_direct(
                    tenant,
                    request.verified_proof(),
                    assertion,
                    evaluation_policy,
                    capabilities.clone(),
                    request.correlation_id(),
                )
                .await
                .map_err(|_| ProtectedResolutionError::new("provider_request_invalid"))?;
            let outcome = self.accept_outcome(request, outcome).await?;
            let AuthorizationOutcome::Allow(snapshot) = outcome else {
                return Err(ProtectedResolutionError::new("provider_denied"));
            };
            let finalization_policy = self
                .current_policy(request.authorization_domain(), request.correlation_id())
                .await?;
            let disposition = self
                .finalizer
                .finalize_enrollment(
                    tenant,
                    request.verified_proof(),
                    assertion,
                    finalization_policy,
                    snapshot,
                    request.correlation_id(),
                )
                .map_err(|_| ProtectedResolutionError::new("enrollment_finalization"))?;
            let dependencies =
                AuthorizationDependencies::from_enrollment(request.session_target(), &disposition)
                    .map_err(|_| ProtectedResolutionError::new("invalidation_dependencies"))?;
            let observer = self
                .invalidation
                .observe_authority(fence, dependencies, request.cancellation())
                .map_err(|_| ProtectedResolutionError::new("invalidation_race"))?;
            let current = LeaseCurrentState::from_trusted_runtime(
                buzz_auth::BindingVersion::INITIAL,
                disposition.profile_id().clone(),
                disposition.policy_version().clone(),
            );
            return Ok(ProtectedResolution::enrollment(
                disposition,
                Arc::new(ProductionLeaseObserver {
                    invalidation: observer,
                    current,
                }),
            ));
        }

        self.require_membership(request).await?;
        let adapter = VerifiedEvidenceAdapter::new();
        let evaluation_policy = self
            .current_policy(request.authorization_domain(), request.correlation_id())
            .await?;
        let (authorization, snapshot) = if let Some(owner) = request.owner_pubkey() {
            let binding = self
                .active_binding(request.authorization_domain(), owner, Some(assertion))
                .await?;
            let outcome = self
                .finalizer
                .evaluate_delegated(
                    tenant,
                    request.verified_proof(),
                    &binding,
                    evaluation_policy,
                    capabilities.clone(),
                    request.correlation_id(),
                )
                .await
                .map_err(|_| ProtectedResolutionError::new("provider_request_invalid"))?;
            let outcome = self.accept_outcome(request, outcome).await?;
            let AuthorizationOutcome::Allow(snapshot) = outcome else {
                return Err(ProtectedResolutionError::new("provider_denied"));
            };
            let admission = snapshot
                .verified_owner_admission(&binding)
                .map_err(|_| ProtectedResolutionError::new("owner_admission_mismatch"))?;
            (
                FederatedAuthorization::Delegated {
                    owner: binding,
                    admission,
                },
                snapshot,
            )
        } else {
            let binding = self
                .active_binding(
                    request.authorization_domain(),
                    request.actor_pubkey(),
                    Some(assertion),
                )
                .await?;
            let outcome = self
                .finalizer
                .evaluate_direct(
                    tenant,
                    request.verified_proof(),
                    assertion,
                    evaluation_policy,
                    capabilities.clone(),
                    request.correlation_id(),
                )
                .await
                .map_err(|_| ProtectedResolutionError::new("provider_request_invalid"))?;
            let outcome = self.accept_outcome(request, outcome).await?;
            let AuthorizationOutcome::Allow(snapshot) = outcome else {
                return Err(ProtectedResolutionError::new("provider_denied"));
            };
            let owned_assertion = self.reseal_assertion(assertion)?;
            (
                FederatedAuthorization::Direct {
                    binding,
                    assertion: owned_assertion,
                },
                snapshot,
            )
        };
        let binding = match &authorization {
            FederatedAuthorization::Direct { binding, .. } => binding,
            FederatedAuthorization::Delegated { owner, .. } => owner,
            FederatedAuthorization::NotRequired => {
                unreachable!("protected resolver requires identity")
            }
        };
        let binding_bound = BindingLeaseBound::new(binding, snapshot.effective_until())
            .map_err(|_| ProtectedResolutionError::new("binding_bound_invalid"))?;
        let access = adapter
            .community_access_from_policy(
                tenant,
                request.authorization_domain(),
                Scope::all_known(),
                None,
            )
            .map_err(|_| ProtectedResolutionError::new("community_access_invalid"))?;
        let input = AuthContextInput::new(
            tenant.clone(),
            request.correlation_id(),
            Arc::clone(request.verified_proof()),
            access,
        );
        let finalization_policy = self
            .current_policy(request.authorization_domain(), request.correlation_id())
            .await?;
        let disposition = self
            .finalizer
            .finalize_allowed(
                input,
                finalization_policy,
                authorization,
                snapshot,
                binding_bound,
                LeaseVersion::INITIAL,
            )
            .map_err(|_| ProtectedResolutionError::new("authorization_finalization"))?;
        let RuntimeAuthorizationDisposition::Access(context) = disposition else {
            return Err(ProtectedResolutionError::new(
                "non_authoritative_disposition",
            ));
        };
        let dependencies =
            AuthorizationDependencies::from_context(request.session_target(), &context)
                .map_err(|_| ProtectedResolutionError::new("invalidation_dependencies"))?;
        let observer = self
            .invalidation
            .observe_authority(fence, dependencies, request.cancellation())
            .map_err(|_| ProtectedResolutionError::new("invalidation_race"))?;
        let lease = context
            .authorization_lease()
            .ok_or(ProtectedResolutionError::new("access_lease_missing"))?;
        let current = LeaseCurrentState::from_trusted_runtime(
            lease.binding_version(),
            lease.profile_id().clone(),
            lease.policy_version().clone(),
        );
        Ok(ProtectedResolution::access(
            context,
            Arc::new(ProductionLeaseObserver {
                invalidation: observer,
                current,
            }),
        ))
    }
}

/// Build the optional exact-domain runtime from provider-neutral environment
/// configuration. An absent or blank domain list installs nothing.
pub async fn build_from_environment(
    state: &crate::state::AppState,
) -> Result<Option<InstalledProtectedRuntime>, ProductionRuntimeError> {
    build_from_environment_with_providers(state, ProductionProviderRegistry::default()).await
}

/// Build and install the disabled-by-default stock runtime.
///
/// The stock OSS binary has no admission provider and therefore fails closed
/// if a non-Off domain is configured. A deployment composition root with an
/// exact O2 provider calls [`install_from_environment_with_providers`] instead.
pub async fn install_from_environment(
    state: &Arc<crate::state::AppState>,
) -> Result<ProtectedRuntimeInstallation, ProductionRuntimeError> {
    install_from_environment_with_providers(state, ProductionProviderRegistry::default()).await
}

/// Build and install one exact provider-neutral production runtime.
///
/// This is the sole production composition seam for externally supplied O2
/// providers. Restore and invalidation state is initialized before either is
/// made reachable from `AppState`; missing providers and partial installation
/// fail startup rather than falling back to legacy authorization.
pub async fn install_from_environment_with_providers(
    state: &Arc<crate::state::AppState>,
    providers: ProductionProviderRegistry,
) -> Result<ProtectedRuntimeInstallation, ProductionRuntimeError> {
    install_from_environment_with_providers_and_evidence(state, providers, None).await
}

/// Build and install one runtime with an optional provider-neutral evidence seam.
///
/// When installed, the resolver must return zero, exactly one, or ambiguous
/// sealed evidence for every protected request. JWT verification remains an
/// optional provider profile and conflicts with supplied neutral evidence.
pub async fn install_from_environment_with_providers_and_evidence(
    state: &Arc<crate::state::AppState>,
    providers: ProductionProviderRegistry,
    evidence_resolver: Option<Arc<dyn VerifiedProviderEvidenceResolver>>,
) -> Result<ProtectedRuntimeInstallation, ProductionRuntimeError> {
    if state.protected_transport().is_some() || state.restore_protection().is_some() {
        return Err(ProductionRuntimeError::AlreadyInstalled);
    }
    let Some(installed) =
        build_from_environment_with_providers_and_evidence(state, providers, evidence_resolver)
            .await?
    else {
        return Ok(ProtectedRuntimeInstallation::Disabled);
    };
    let InstalledProtectedRuntime {
        transport,
        invalidation,
        restore,
        enforce_enabled,
        enforcing_domains,
        projection_domains,
    } = installed;
    let audio_restore = Arc::clone(&restore);
    state
        .install_restore_protection(restore)
        .map_err(|_| ProductionRuntimeError::AlreadyInstalled)?;
    state
        .install_protected_transport(transport)
        .map_err(|_| ProductionRuntimeError::AlreadyInstalled)?;

    let invalidation_worker = invalidation.clone();
    let invalidation_failure = invalidation.clone();
    let authorization_hint_subscriber = Arc::clone(&state.pubsub);
    let mut workers = tokio::task::JoinSet::new();
    workers.spawn(async move { invalidation_worker.run().await });
    workers.spawn(async move {
        authorization_hint_subscriber
            .run_authorization_invalidation_subscriber()
            .await;
    });
    if enforce_enabled {
        workers.spawn(run_audio_reconciliation(
            state.db.clone(),
            audio_restore,
            enforcing_domains,
        ));
    }
    workers.spawn(
        crate::corporate_identity::run_public_projection_retirement_reconciliation(
            Arc::clone(state),
            projection_domains,
        ),
    );
    // Any required worker exit is a runtime-health failure. A detached bare
    // worker could otherwise panic while Enforce kept serving from stale
    // authority. The supervisor first invalidates every domain, then aborts
    // the remaining workers; all protected observations fail closed.
    tokio::spawn(async move {
        let completion = workers.join_next().await;
        invalidation_failure.fail_closed();
        workers.abort_all();
        while workers.join_next().await.is_some() {}
        match completion {
            Some(Ok(())) => {
                tracing::error!(
                    "required protected authorization worker exited; runtime failed closed"
                )
            }
            Some(Err(error)) => tracing::error!(
                %error,
                "required protected authorization worker failed; runtime failed closed"
            ),
            None => tracing::error!(
                "protected authorization worker set was empty; runtime failed closed"
            ),
        }
    });
    Ok(ProtectedRuntimeInstallation::Installed)
}

/// Build the disabled-by-default runtime with exact provider implementations
/// supplied by the deployment adapter. The stock OSS binary supplies an empty
/// registry, so any configured non-Off domain fails closed instead of using a
/// permissive fallback.
pub async fn build_from_environment_with_providers(
    state: &crate::state::AppState,
    providers: ProductionProviderRegistry,
) -> Result<Option<InstalledProtectedRuntime>, ProductionRuntimeError> {
    build_from_environment_with_providers_and_evidence(state, providers, None).await
}

/// Build the disabled-by-default runtime with optional verified evidence input.
pub async fn build_from_environment_with_providers_and_evidence(
    state: &crate::state::AppState,
    providers: ProductionProviderRegistry,
    evidence_resolver: Option<Arc<dyn VerifiedProviderEvidenceResolver>>,
) -> Result<Option<InstalledProtectedRuntime>, ProductionRuntimeError> {
    let raw = env::var(DOMAINS_ENV).unwrap_or_default();
    let configured = parse_domains(&raw)?;
    let activated = state.db.activated_authorization_domains().await?;
    validate_activated_domain_configuration(&configured, activated.iter().copied())?;
    if configured.is_empty() {
        return Ok(None);
    }
    let evidence_pseudonymizer = if configured.values().any(|mode| mode.evaluates_provider()) {
        Some(parse_evidence_pseudonymizer()?)
    } else {
        None
    };
    validate_provider_coverage(&configured, &providers)?;
    if evidence_resolver.is_none()
        && configured.values().any(|mode| mode.evaluates_provider())
        && state.identity_assertion_provenance().is_none()
    {
        return Err(ProductionRuntimeError::AssertionProvenanceMissing);
    }
    let protected_domains = protected_domains(&configured);
    let enforcing_domains = enforcing_domains(&configured);
    let projection_domains = projection_reconciliation_domains(&configured);
    if evidence_resolver.is_none()
        && !enforcing_domains.is_empty()
        && state.corporate_identity.is_none()
    {
        return Err(ProductionRuntimeError::VerifierMissing);
    }
    let clock: SharedAuthorizationClock = Arc::new(SystemAuthorizationClock);
    let restore_bootstraps = parse_restore_bootstraps(
        &env::var(RESTORE_BOOTSTRAPS_ENV).unwrap_or_default(),
        &protected_domains,
    )?;
    let profile = env::var(PROFILE_ENV).unwrap_or_else(|_| "current-membership-v1".to_owned());
    let installed_profile = AuthorizationProfileId::from_server_configuration(profile.clone())
        .map_err(|_| ProductionRuntimeError::InvalidConfiguration)?;
    let lease_seconds = parse_positive_seconds(LEASE_SECONDS_ENV, 300)?;
    let lease_limit = ApplicationLeaseLimit::from_seconds(lease_seconds)?;
    let status_limit = ApplicationLeaseLimit::from_seconds(lease_seconds.min(60))?;
    let skew = AuthorizationClockSkew::from_seconds(0)?;
    let mut policies = Vec::with_capacity(configured.len());
    let mut transports = Vec::with_capacity(configured.len());
    let mut profiles = HashMap::with_capacity(configured.len());
    for (domain, mode) in &configured {
        transports.push(DomainTransportPolicy::from_server_configuration(
            *domain, *mode,
        ));
        if !mode.evaluates_provider() {
            continue;
        }
        let provider = providers.provider_for(*domain)?;
        profiles.insert(*domain, installed_profile.clone());
        policies.push(DomainAuthorizationPolicy::from_server_configuration(
            *domain,
            profile.clone(),
            provider,
            EnrollmentMode::AttestedKey,
            *mode,
            ProviderTimeout::new(Duration::from_secs(2))?,
            AccessLeasePolicy::new(lease_limit, skew),
            VerificationStatusPolicy::new(status_limit, skew),
        )?);
    }
    let hosts = state.db.usage_community_hosts().await?;
    let host_map = hosts
        .into_iter()
        .map(|entry| {
            (
                CommunityId::from_uuid(entry.id),
                TenantContext::resolved(CommunityId::from_uuid(entry.id), entry.host),
            )
        })
        .collect::<HashMap<_, _>>();
    for domain in configured.keys() {
        if !host_map.contains_key(domain) {
            return Err(ProductionRuntimeError::ConfiguredDomainMissing);
        }
    }
    let restore = super::restore::RestoreProtectionRuntime::initialize(
        state.db.clone(),
        state.git_store.clone(),
        restore_bootstraps,
    )
    .await?;
    activate_protected_domains(&state.db, &restore, protected_domains.iter().copied()).await?;
    reconcile_audio_admissions_once(&state.db, &restore, enforcing_domains.iter().copied()).await?;
    let invalidation = AuthorizationInvalidationRuntime::new_with_restore(
        state.db.clone(),
        Arc::clone(&state.pubsub),
        AuthorizationInvalidationConfig::default(),
        Arc::clone(&restore),
    );
    invalidation
        .initialize_domains(protected_domains.iter().copied())
        .await?;
    crate::corporate_identity::reconcile_public_projection_retirements_startup(
        state,
        &projection_domains,
    )
    .await
    .map_err(|_| ProductionRuntimeError::PublicProjection)?;
    let finalizer = RelayAuthorizationFinalizer::new(
        DomainProviderSelector::new(policies)?,
        Arc::clone(&clock),
    );
    let resolver: Arc<dyn ProtectedAuthorizationResolver> = Arc::new(ProductionResolver {
        db: state.db.clone(),
        tenants: host_map,
        finalizer,
        invalidation: invalidation.clone(),
        clock: Arc::clone(&clock),
        profiles,
        evidence_pseudonymizer,
        evidence_health: EvidenceHealthSignal::default(),
    });
    let mut transport = ProtectedTransportRuntime::new(transports, resolver, clock)?;
    if let Some(evidence_resolver) = evidence_resolver {
        transport = transport.with_provider_evidence_resolver(evidence_resolver);
    }
    let transport = Arc::new(transport);
    Ok(Some(InstalledProtectedRuntime {
        transport,
        invalidation,
        restore,
        enforce_enabled: !enforcing_domains.is_empty(),
        enforcing_domains,
        projection_domains,
    }))
}

fn parse_evidence_pseudonymizer() -> Result<Pseudonymizer, ProductionRuntimeError> {
    let raw_key = env::var(AUDIT_PSEUDONYM_KEY_ENV)
        .map_err(|_| ProductionRuntimeError::EvidenceConfigurationMissing)?;
    let decoded =
        hex::decode(raw_key).map_err(|_| ProductionRuntimeError::EvidenceConfigurationInvalid)?;
    let key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| ProductionRuntimeError::EvidenceConfigurationInvalid)?;
    let epoch = env::var(AUDIT_PSEUDONYM_KEY_EPOCH_ENV)
        .map_err(|_| ProductionRuntimeError::EvidenceConfigurationMissing)?
        .parse::<u32>()
        .map_err(|_| ProductionRuntimeError::EvidenceConfigurationInvalid)?;
    if epoch == 0 {
        return Err(ProductionRuntimeError::EvidenceConfigurationInvalid);
    }
    let key =
        PseudonymKey::new(key).map_err(|_| ProductionRuntimeError::EvidenceConfigurationInvalid)?;
    Ok(Pseudonymizer::new(key, epoch))
}

fn transport_class(transport: AuthTransport) -> TransportClass {
    match transport {
        AuthTransport::RelayWebSocket => TransportClass::WebSocket,
        AuthTransport::HttpBridge => TransportClass::Http,
        AuthTransport::Git => TransportClass::Repository,
        AuthTransport::MediaUpload | AuthTransport::MediaDownload => TransportClass::Media,
        AuthTransport::Audio => TransportClass::Audio,
    }
}

fn operation_class(capability: AuthorizationCapability) -> OperationClass {
    match capability {
        AuthorizationCapability::CommunityRead
        | AuthorizationCapability::MediaRead
        | AuthorizationCapability::GitRead => OperationClass::Read,
        AuthorizationCapability::CommunityWrite => OperationClass::Publish,
        AuthorizationCapability::MediaWrite | AuthorizationCapability::GitWrite => {
            OperationClass::Write
        }
        AuthorizationCapability::AudioJoin => OperationClass::Join,
        AuthorizationCapability::Moderate
        | AuthorizationCapability::InviteMint
        | AuthorizationCapability::InviteClaim => OperationClass::Lifecycle,
        _ => OperationClass::NotApplicable,
    }
}

fn decision_reason_for_denial(reason: AuthorizationDenialReason) -> DecisionReason {
    match reason {
        AuthorizationDenialReason::ProviderDenied
        | AuthorizationDenialReason::AuthorizationProfileMismatch
        | AuthorizationDenialReason::FederatedPolicyNotCurrent => DecisionReason::PolicyDenied,
        AuthorizationDenialReason::AuthorizationDomainMismatch => DecisionReason::DomainMismatch,
        AuthorizationDenialReason::PrincipalMismatch => DecisionReason::TargetMismatch,
        AuthorizationDenialReason::MissingCapability => DecisionReason::OperationMismatch,
        AuthorizationDenialReason::StaleDecision
        | AuthorizationDenialReason::IdentityEvidenceExpired => DecisionReason::EvidenceExpired,
        AuthorizationDenialReason::FutureDecision
        | AuthorizationDenialReason::IdentityEvidenceNotYetValid => DecisionReason::EvidenceInvalid,
        _ => DecisionReason::SchemaUnsupported,
    }
}

async fn activate_protected_domains(
    db: &buzz_db::Db,
    restore: &Arc<super::restore::RestoreProtectionRuntime>,
    domains: impl IntoIterator<Item = CommunityId>,
) -> Result<(), ProductionRuntimeError> {
    for domain in domains {
        let mut digest = Sha256::new();
        digest.update(b"buzz-protected-domain-activation-v1");
        digest.update(domain.as_uuid().as_bytes());
        let fingerprint: [u8; 32] = digest.finalize().into();
        let operation_id = super::executor::ProtectedOperationId::derive(
            domain,
            "runtime.domain.activate.v1",
            &fingerprint,
        )?
        .as_uuid();
        let witness = restore.begin(domain, operation_id, fingerprint).await?;
        match db
            .activate_authorization_domain(domain, operation_id, fingerprint)
            .await
        {
            Ok(()) => witness.commit().await?,
            Err(error) => {
                let committed = db
                    .authorization_operation_receipt_fingerprint(domain, operation_id)
                    .await?;
                if committed == Some(fingerprint) {
                    witness.commit().await?;
                } else {
                    witness.abort().await?;
                    return Err(error.into());
                }
            }
        }
    }
    Ok(())
}

fn validate_provider_coverage(
    configured: &HashMap<CommunityId, AuthorizationMode>,
    providers: &ProductionProviderRegistry,
) -> Result<(), ProductionRuntimeError> {
    for (domain, mode) in configured {
        if mode.evaluates_provider() {
            providers.provider_for(*domain)?;
        }
    }
    Ok(())
}

fn validate_activated_domain_configuration(
    configured: &HashMap<CommunityId, AuthorizationMode>,
    activated: impl IntoIterator<Item = CommunityId>,
) -> Result<(), ProductionRuntimeError> {
    if activated.into_iter().any(|domain| {
        !configured
            .get(&domain)
            .is_some_and(|mode| mode.protects_surfaces())
    }) {
        return Err(ProductionRuntimeError::ActivatedDomainDowngrade);
    }
    Ok(())
}

/// Reconcile expired durable audio attempts. Discovery is read-only; an
/// unexpired claimant remains exclusively owned by its healthy replica and
/// every orphan transition is independently witnessed.
pub async fn run_audio_reconciliation(
    db: buzz_db::Db,
    restore: Arc<super::restore::RestoreProtectionRuntime>,
    enforcing_domains: Vec<CommunityId>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(error) =
            reconcile_audio_admissions_once(&db, &restore, enforcing_domains.iter().copied()).await
        {
            tracing::warn!(%error, "durable audio admission reconciliation failed");
        }
    }
}

async fn reconcile_audio_admissions_once(
    db: &buzz_db::Db,
    restore: &Arc<super::restore::RestoreProtectionRuntime>,
    domains: impl IntoIterator<Item = CommunityId>,
) -> Result<u64, ProductionRuntimeError> {
    let mut reconciled = 0_u64;
    let mut first_error: Option<ProductionRuntimeError> = None;
    for domain in domains {
        let mut domain_failed = false;
        for sweep in 0..MAX_AUDIO_RECONCILIATION_SWEEPS {
            let mut cursor = None;
            loop {
                let discovered = buzz_db::audio_admission::reconcilable_audio_admissions_after(
                    db, domain, cursor,
                )
                .await;
                let candidates = match discovered {
                    Ok(candidates) => candidates,
                    Err(error) => {
                        tracing::warn!(%error, "durable audio admission discovery failed for one domain");
                        first_error.get_or_insert_with(|| error.into());
                        domain_failed = true;
                        break;
                    }
                };
                let Some(last) = candidates.last().map(|candidate| candidate.admission_id) else {
                    break;
                };
                cursor = Some(last);
                for candidate in candidates {
                    match reconcile_audio_admission(db, restore, domain, candidate).await {
                        Ok(true) => reconciled = reconciled.saturating_add(1),
                        Ok(false) => {}
                        Err(error) => {
                            tracing::warn!(
                                %error,
                                "durable audio admission candidate failed without starving later cleanup"
                            );
                            first_error.get_or_insert(error);
                            domain_failed = true;
                        }
                    }
                }
            }
            if domain_failed {
                break;
            }
            let remaining =
                buzz_db::audio_admission::reconcilable_audio_admissions(db, domain).await;
            match remaining {
                Ok(remaining) if remaining.is_empty() => break,
                Ok(_) if sweep + 1 < MAX_AUDIO_RECONCILIATION_SWEEPS => continue,
                Ok(_) => {
                    first_error
                        .get_or_insert(ProductionRuntimeError::AudioReconciliationIncomplete);
                    break;
                }
                Err(error) => {
                    first_error.get_or_insert_with(|| error.into());
                    break;
                }
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(reconciled),
    }
}

async fn reconcile_audio_admission(
    db: &buzz_db::Db,
    restore: &Arc<super::restore::RestoreProtectionRuntime>,
    domain: CommunityId,
    candidate: buzz_db::audio_admission::AudioAdmissionReconciliationCandidate,
) -> Result<bool, ProductionRuntimeError> {
    use super::executor::ProtectedOperationId;

    // Reconciliation never proves a graceful disconnect. Even a formerly
    // visible orphan is conservatively compensated as aborted after its
    // durable ownership deadline and grace period.
    let finished = false;
    let terminal = b"aborted".as_slice();
    let mut stable = Sha256::new();
    stable.update(b"buzz-audio-admission-reconciliation-v1");
    stable.update(candidate.admission_id.as_bytes());
    stable.update(candidate.claimant_id.as_bytes());
    stable.update(candidate.source_state.as_str().as_bytes());
    stable.update(terminal);
    stable.update(candidate.state_version.to_be_bytes());
    let stable: [u8; 32] = stable.finalize().into();
    let operation_id =
        ProtectedOperationId::derive(domain, "audio.admission.reconcile.v1", &stable)
            .map_err(|_| ProductionRuntimeError::InvalidConfiguration)?;
    let mut request = Sha256::new();
    request.update(b"buzz-audio-admission-reconciliation-request-v1");
    request.update(stable);
    let request: [u8; 32] = request.finalize().into();
    let witness = restore
        .begin(domain, operation_id.as_uuid(), request)
        .await?;
    match buzz_db::audio_admission::reconcile_claimed_audio_admission_with_receipt(
        db,
        domain,
        candidate,
        finished,
        Some("orphaned_attachment"),
        operation_id.as_uuid(),
        request,
    )
    .await
    {
        Ok(true) => witness.commit().await?,
        Ok(false) => {
            witness.abort().await?;
            return Ok(false);
        }
        Err(error) => {
            witness.abort().await?;
            return Err(error.into());
        }
    }
    Ok(true)
}

fn parse_positive_seconds(name: &'static str, default: u64) -> Result<u64, ProductionRuntimeError> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or(ProductionRuntimeError::InvalidConfiguration),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(env::VarError::NotUnicode(_)) => Err(ProductionRuntimeError::InvalidConfiguration),
    }
}

fn parse_domains(
    raw: &str,
) -> Result<HashMap<CommunityId, AuthorizationMode>, ProductionRuntimeError> {
    let mut domains = HashMap::new();
    for item in raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (id, mode) = item
            .split_once(':')
            .ok_or(ProductionRuntimeError::InvalidConfiguration)?;
        let domain = CommunityId::from_uuid(
            uuid::Uuid::parse_str(id).map_err(|_| ProductionRuntimeError::InvalidConfiguration)?,
        );
        let mode = match mode.trim().to_ascii_lowercase().as_str() {
            "off" => AuthorizationMode::Off,
            "shadow" => AuthorizationMode::Shadow,
            "verify_only" => AuthorizationMode::VerifyOnly,
            "enforce" => AuthorizationMode::Enforce,
            "deny_protected" => AuthorizationMode::DenyProtected,
            _ => return Err(ProductionRuntimeError::InvalidConfiguration),
        };
        if domains.insert(domain, mode).is_some() {
            return Err(ProductionRuntimeError::InvalidConfiguration);
        }
    }
    Ok(domains)
}

fn protected_domains(configured: &HashMap<CommunityId, AuthorizationMode>) -> Vec<CommunityId> {
    configured
        .iter()
        .filter_map(|(domain, mode)| mode.protects_surfaces().then_some(*domain))
        .collect()
}

fn enforcing_domains(configured: &HashMap<CommunityId, AuthorizationMode>) -> Vec<CommunityId> {
    configured
        .iter()
        .filter_map(|(domain, mode)| (*mode == AuthorizationMode::Enforce).then_some(*domain))
        .collect()
}

fn projection_reconciliation_domains(
    configured: &HashMap<CommunityId, AuthorizationMode>,
) -> Vec<CommunityId> {
    // Public projection retirement belongs to authoritative Enforce runtime.
    // Observational modes must not queue or publish projection changes.
    configured
        .iter()
        .filter_map(|(domain, mode)| (*mode == AuthorizationMode::Enforce).then_some(*domain))
        .collect()
}

fn parse_restore_bootstraps(
    raw: &str,
    protected_domains: &[CommunityId],
) -> Result<Vec<(CommunityId, uuid::Uuid)>, ProductionRuntimeError> {
    let mut anchors = HashMap::new();
    for item in raw
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        let (domain, bootstrap) = item
            .split_once('=')
            .ok_or(ProductionRuntimeError::InvalidConfiguration)?;
        let domain = CommunityId::from_uuid(
            uuid::Uuid::parse_str(domain.trim())
                .map_err(|_| ProductionRuntimeError::InvalidConfiguration)?,
        );
        let bootstrap = uuid::Uuid::parse_str(bootstrap.trim())
            .map_err(|_| ProductionRuntimeError::InvalidConfiguration)?;
        if bootstrap.is_nil() || anchors.insert(domain, bootstrap).is_some() {
            return Err(ProductionRuntimeError::InvalidConfiguration);
        }
    }
    let mut result = Vec::with_capacity(protected_domains.len());
    for domain in protected_domains {
        let bootstrap = anchors
            .remove(domain)
            .ok_or(ProductionRuntimeError::RestoreBootstrapMissing)?;
        result.push((*domain, bootstrap));
    }
    if !anchors.is_empty() {
        return Err(ProductionRuntimeError::InvalidConfiguration);
    }
    Ok(result)
}

/// Fail-closed production construction error.
#[derive(Debug, Error)]
pub enum ProductionRuntimeError {
    /// Configuration was malformed or ambiguous.
    #[error("protected authorization configuration is invalid")]
    InvalidConfiguration,
    /// An evaluating domain has no audit-only pseudonymization key or epoch.
    #[error("authorization evidence configuration is missing")]
    EvidenceConfigurationMissing,
    /// The audit-only pseudonymization key or epoch is malformed.
    #[error("authorization evidence configuration is invalid")]
    EvidenceConfigurationInvalid,
    /// An exact configured domain has no durable host mapping.
    #[error("protected authorization domain is not present")]
    ConfiguredDomainMissing,
    /// Typed provider or finalization configuration was rejected.
    #[error(transparent)]
    Provider(#[from] buzz_auth::ProviderContractError),
    /// A configured authoritative or observational domain has no exact O2 provider.
    #[error("protected authorization provider is not configured for this domain")]
    ProviderMissing,
    /// Protected production state was already partially or fully installed.
    #[error("protected authorization runtime is already installed")]
    AlreadyInstalled,
    /// Enforce was requested without a complete domain-usable assertion verifier.
    #[error("protected authorization assertion verifier is not configured")]
    VerifierMissing,
    /// A protected domain was configured without deployment-verified ingress
    /// provenance for its direct identity assertion.
    #[error("protected authorization assertion provenance is not configured")]
    AssertionProvenanceMissing,
    /// A previously activated domain was omitted or configured non-authoritatively.
    #[error("protected authorization domain cannot be downgraded after activation")]
    ActivatedDomainDowngrade,
    /// An Enforce domain has no exact externally provisioned restore anchor.
    #[error("protected authorization restore bootstrap is not configured")]
    RestoreBootstrapMissing,
    /// Lease bounds were rejected.
    #[error(transparent)]
    Lease(#[from] buzz_auth::LeasePolicyError),
    /// Durable domain construction failed.
    #[error(transparent)]
    Database(#[from] buzz_db::DbError),
    /// Invalidation initialization failed.
    #[error(transparent)]
    Invalidation(#[from] super::invalidation::AuthorizationInvalidationRuntimeError),
    /// Exact-domain policy construction failed.
    #[error(transparent)]
    DomainPolicy(#[from] super::finalization::DomainPolicyError),
    /// Protected transport construction failed.
    #[error(transparent)]
    Transport(#[from] super::transport::ProtectedTransportError),
    /// Independent restore witness initialization failed.
    #[error(transparent)]
    Restore(#[from] super::restore::RestoreProtectionError),
    /// Stable operation construction failed before production activation.
    #[error(transparent)]
    Execution(#[from] super::executor::AuthorizationExecutionError),
    /// Public projection startup reconciliation failed.
    #[error("public identity projection reconciliation failed")]
    PublicProjection,
    /// Startup could not prove that all durable audio remnants were reconciled.
    #[error("protected audio reconciliation did not reach a complete fixed point")]
    AudioReconciliationIncomplete,
}

#[cfg(test)]
mod tests {
    use buzz_auth::{
        AuthorizationProviderFuture, AuthorizationRequest, ProviderDecision, ProviderUnavailable,
        ProviderUnavailableReason,
    };

    use super::*;

    #[test]
    fn every_production_provider_evaluation_reaches_durable_acceptance() {
        fn offsets(section: &str, needles: &[&str]) -> Vec<usize> {
            let mut offsets = needles
                .iter()
                .flat_map(|needle| section.match_indices(needle).map(|(offset, _)| offset))
                .collect::<Vec<_>>();
            offsets.sort_unstable();
            offsets
        }

        let source = include_str!("production.rs");
        let production = source
            .split_once("\n#[cfg(test)]\nmod tests")
            .map(|(production, _)| production)
            .expect("production source has a bounded test module");
        let present_start = production
            .find("    async fn present(")
            .expect("production resolver has present");
        let resolve_start = production
            .find("    async fn resolve(")
            .expect("production resolver has resolve");
        let observe = &production[..present_start];
        let present = &production[present_start..resolve_start];
        let resolve = &production[resolve_start..];
        let evaluation_needles = [".evaluate_direct(", ".evaluate_delegated("];
        let acceptance_needles = [".accept_outcome(request, outcome).await?"];

        let evaluations = offsets(production, &evaluation_needles);
        let acceptances = offsets(production, &acceptance_needles);
        assert_eq!(evaluations.len(), 6, "inventory all six decision branches");
        assert_eq!(acceptances.len(), 5, "inventory five durable joins");

        let observe_evaluations = offsets(observe, &evaluation_needles);
        let observe_acceptances = offsets(observe, &acceptance_needles);
        assert_eq!(observe_evaluations.len(), 2);
        assert_eq!(observe_acceptances.len(), 1);
        assert!(observe.contains("let outcome = if let Some(owner)"));
        assert!(
            observe_evaluations
                .iter()
                .all(|evaluation| *evaluation < observe_acceptances[0]),
            "both mutually exclusive observe branches must converge on the durable join"
        );

        for (section, expected_evaluations) in [(present, 1_usize), (resolve, 3_usize)] {
            let section_evaluations = offsets(section, &evaluation_needles);
            let section_acceptances = offsets(section, &acceptance_needles);
            assert_eq!(section_evaluations.len(), expected_evaluations);
            assert_eq!(section_acceptances.len(), expected_evaluations);
            assert!(
                section_evaluations
                    .iter()
                    .zip(section_acceptances.iter())
                    .all(|(evaluation, acceptance)| evaluation < acceptance),
                "no production decision branch may bypass its durable acceptance join"
            );
        }

        assert_eq!(
            observe_evaluations.len()
                + offsets(present, &evaluation_needles).len()
                + offsets(resolve, &evaluation_needles).len(),
            evaluations.len(),
            "every intended production decision branch remains in the structural inventory"
        );
    }

    struct SyntheticUnavailableProvider;

    impl AuthorizationProvider for SyntheticUnavailableProvider {
        fn profile_id(&self) -> buzz_auth::AuthorizationProfileId {
            buzz_auth::AuthorizationProfileId::from_server_configuration(
                "profile.synthetic-unavailable.example",
            )
            .expect("synthetic profile is valid")
        }

        fn authorize<'a>(
            &'a self,
            _request: &'a AuthorizationRequest,
        ) -> AuthorizationProviderFuture<'a> {
            Box::pin(async {
                ProviderDecision::Unavailable(ProviderUnavailable::new(
                    ProviderUnavailableReason::DependencyUnavailable,
                    None,
                ))
            })
        }
    }

    #[test]
    fn absent_configuration_is_disabled() {
        assert!(parse_domains("").expect("empty is disabled").is_empty());
    }

    #[test]
    fn exact_modes_parse_without_a_default() {
        let first = uuid::Uuid::new_v4();
        let second = uuid::Uuid::new_v4();
        let third = uuid::Uuid::new_v4();
        let parsed = parse_domains(&format!(
            "{first}:enforce,{second}:verify_only,{third}:deny_protected"
        ))
        .expect("valid exact domains");
        assert_eq!(
            parsed.get(&CommunityId::from_uuid(first)),
            Some(&AuthorizationMode::Enforce)
        );
        assert_eq!(
            parsed.get(&CommunityId::from_uuid(second)),
            Some(&AuthorizationMode::VerifyOnly)
        );
        assert_eq!(
            parsed.get(&CommunityId::from_uuid(third)),
            Some(&AuthorizationMode::DenyProtected)
        );
    }

    #[test]
    fn duplicate_or_unknown_domains_fail_closed() {
        let id = uuid::Uuid::new_v4();
        assert!(parse_domains(&format!("{id}:enforce,{id}:shadow")).is_err());
        assert!(parse_domains(&format!("{id}:automatic")).is_err());
    }

    #[test]
    fn observational_modes_have_no_durable_runtime_domains() {
        let off = uuid::Uuid::new_v4();
        let shadow = uuid::Uuid::new_v4();
        let verify = uuid::Uuid::new_v4();
        let enforce = uuid::Uuid::new_v4();
        let deny = uuid::Uuid::new_v4();
        let parsed = parse_domains(&format!(
            "{off}:off,{shadow}:shadow,{verify}:verify_only,{enforce}:enforce,{deny}:deny_protected"
        ))
        .expect("valid exact modes");
        let protected = protected_domains(&parsed)
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(protected.len(), 2);
        assert!(protected.contains(&CommunityId::from_uuid(enforce)));
        assert!(protected.contains(&CommunityId::from_uuid(deny)));
        assert_eq!(
            enforcing_domains(&parsed),
            vec![CommunityId::from_uuid(enforce)]
        );
        let projection_domains = projection_reconciliation_domains(&parsed)
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(projection_domains.len(), 1);
        assert!(!projection_domains.contains(&CommunityId::from_uuid(off)));
        assert!(!projection_domains.contains(&CommunityId::from_uuid(shadow)));
        assert!(!projection_domains.contains(&CommunityId::from_uuid(verify)));
        assert!(projection_domains.contains(&CommunityId::from_uuid(enforce)));
        assert!(!projection_domains.contains(&CommunityId::from_uuid(deny)));
    }

    #[test]
    fn activated_domain_cannot_be_omitted_or_downgraded() {
        let activated = CommunityId::from_uuid(uuid::Uuid::new_v4());
        assert!(matches!(
            validate_activated_domain_configuration(&HashMap::new(), [activated]),
            Err(ProductionRuntimeError::ActivatedDomainDowngrade)
        ));
        for mode in [
            AuthorizationMode::Off,
            AuthorizationMode::Shadow,
            AuthorizationMode::VerifyOnly,
        ] {
            let configured = HashMap::from([(activated, mode)]);
            assert!(matches!(
                validate_activated_domain_configuration(&configured, [activated]),
                Err(ProductionRuntimeError::ActivatedDomainDowngrade)
            ));
        }
        let configured = HashMap::from([(activated, AuthorizationMode::Enforce)]);
        validate_activated_domain_configuration(&configured, [activated])
            .expect("exact Enforce configuration preserves one-way activation");
        let configured = HashMap::from([(activated, AuthorizationMode::DenyProtected)]);
        validate_activated_domain_configuration(&configured, [activated])
            .expect("deny-protected preserves the protected inventory after activation");
    }

    #[test]
    fn exact_provider_registry_has_no_fallback() {
        let configured = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let absent = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let provider: Arc<dyn AuthorizationProvider> = Arc::new(SyntheticUnavailableProvider);
        let registry = ProductionProviderRegistry::new([(configured, provider)])
            .expect("exact provider registry");
        assert!(registry.provider_for(configured).is_ok());
        assert!(matches!(
            registry.provider_for(absent),
            Err(ProductionRuntimeError::ProviderMissing)
        ));
    }

    #[test]
    fn exact_provider_coverage_makes_enforce_constructible_without_fallback() {
        let enforce = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let off = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let deny = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let configured = parse_domains(&format!(
            "{enforce}:enforce,{off}:off,{deny}:deny_protected"
        ))
        .expect("exact production configuration");
        assert!(matches!(
            validate_provider_coverage(&configured, &ProductionProviderRegistry::default()),
            Err(ProductionRuntimeError::ProviderMissing)
        ));
        let provider: Arc<dyn AuthorizationProvider> = Arc::new(SyntheticUnavailableProvider);
        let providers = ProductionProviderRegistry::new([(enforce, provider)])
            .expect("exact provider registry");
        validate_provider_coverage(&configured, &providers)
            .expect("an exact Enforce provider reaches production construction");
    }

    #[test]
    fn stock_binary_uses_the_single_production_installation_boundary() {
        let main = include_str!("../main.rs");
        assert!(main.contains("production::install_from_environment(&state)"));
        assert!(!main.contains("production::build_from_environment(&state)"));
        assert!(!main.contains("migration::prepare_postgres_authority(&state"));
        let install = main
            .find("production::install_from_environment(&state)")
            .expect("single production installation");
        let cutover_verify = main
            .find("migration::require_reconciled_authority(&state")
            .expect("read-only cutover verification");
        assert!(install < cutover_verify);
    }

    #[test]
    fn enforce_domain_activation_precedes_snapshot_and_transport_reachability() {
        let source = include_str!("production.rs");
        let activation = source
            .find("activate_protected_domains(&state.db")
            .expect("durable activation is part of construction");
        let snapshot = source
            .find(".initialize_domains(protected_domains.iter().copied())")
            .expect("invalidation snapshot is initialized");
        let transport = source
            .find("ProtectedTransportRuntime::new(transports, resolver, clock)")
            .expect("transport is constructed");
        assert!(activation < snapshot);
        assert!(snapshot < transport);
    }

    #[test]
    fn production_supervises_cross_replica_authorization_hints() {
        let source = include_str!("production.rs");
        let worker_set = source
            .find("let mut workers = tokio::task::JoinSet::new()")
            .expect("protected worker supervisor");
        let invalidation_runtime = source
            .find("invalidation_worker.run().await")
            .expect("durable invalidation runtime worker");
        let redis_subscriber = source
            .find("run_authorization_invalidation_subscriber()")
            .expect("cross-replica authorization hint subscriber");
        let supervisor = source
            .find("let completion = workers.join_next().await")
            .expect("fail-closed worker supervisor");

        assert!(worker_set < invalidation_runtime);
        assert!(worker_set < redis_subscriber);
        assert!(invalidation_runtime < supervisor);
        assert!(redis_subscriber < supervisor);
    }

    #[test]
    fn production_reconciles_projection_before_reachability_and_supervises_retry_worker() {
        let source = include_str!("production.rs");
        let projection_domains = source
            .find("let projection_domains = projection_reconciliation_domains(&configured)")
            .expect("exact projection domain selection");
        let startup = source
            .find("reconcile_public_projection_retirements_startup(")
            .expect("startup reconciliation");
        let transport = source
            .find("ProtectedTransportRuntime::new(transports, resolver, clock)")
            .expect("transport construction");
        let worker_set = source
            .find("let mut workers = tokio::task::JoinSet::new()")
            .expect("protected worker supervisor");
        let retry_worker = source
            .find("run_public_projection_retirement_reconciliation(")
            .expect("continuous projection retry worker");
        let supervisor = source
            .find("let completion = workers.join_next().await")
            .expect("fail-closed worker supervisor");

        assert!(projection_domains < startup);
        assert!(startup < transport);
        assert!(worker_set < retry_worker);
        assert!(retry_worker < supervisor);
    }

    #[test]
    fn restore_bootstraps_are_exact_and_non_nil() {
        let domain = CommunityId::from_uuid(uuid::Uuid::new_v4());
        let bootstrap = uuid::Uuid::new_v4();
        assert_eq!(
            parse_restore_bootstraps(&format!("{domain}={bootstrap}"), &[domain])
                .expect("exact bootstrap"),
            vec![(domain, bootstrap)]
        );
        assert!(matches!(
            parse_restore_bootstraps("", &[domain]),
            Err(ProductionRuntimeError::RestoreBootstrapMissing)
        ));
        assert!(
            parse_restore_bootstraps(&format!("{domain}={}", uuid::Uuid::nil()), &[domain])
                .is_err()
        );
    }
}
