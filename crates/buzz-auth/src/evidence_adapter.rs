//! Narrow trusted-workspace adapter for sealed authorization evidence.
//!
//! Wire data cannot deserialize into any type in this module. The adapter is
//! used only after the existing cryptographic verifier, assertion verifier,
//! community policy, or typed binding store has returned success. It repeats
//! exact domain, transport, actor, time, identifier, version, and provenance
//! checks before crossing the crate's sealed evidence boundary.

use buzz_core::{tenant::TenantContext, CommunityId};
use nostr::{Event, PublicKey};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    context::{
        AssertionExpiry, AssertionNotBefore, AssertionTransport, AuthContextError, AuthMethod,
        AuthTransport, AuthorizedCommunityAccess, BindingExpiry, BindingSource, BindingVersion,
        DelegationExpiry, FederatedPrincipal, VerifiedFederatedAssertion, VerifiedKeyAttestation,
        VerifiedNostrProof, VerifiedOperationBinding, VerifiedOperationBindingKind,
        VerifiedProviderEvidence, VerifiedTransportDelegation, VersionedBindingRef,
    },
    nip42::verify_nip42_event,
    nip98::verify_nip98_event,
    AuthorizationProfileId, AuthorizationReason, CapabilitySet, Scope,
};

/// Binding lifecycle result returned by an authoritative store adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveBindingResolution {
    /// Exact active binding already existed.
    Existing,
    /// Binding was atomically enrolled during this decision.
    Enrolled,
}

/// Existing-verifier output for transport-wide Nostr delegation.
///
/// Construction is deliberately explicit and non-serializable. The relay may
/// create it only by translating a successful NIP-OA verifier result whose
/// constraints were proved transport-wide; raw `auth` tag data is not such a
/// result.
pub struct VerifiedDelegationOutput {
    owner_pubkey: PublicKey,
    delegate_pubkey: PublicKey,
    relationship_id: Uuid,
    relationship_revision: u64,
    expires_at: Option<u64>,
    transport_wide: bool,
}

impl VerifiedDelegationOutput {
    /// Translate an existing verifier's exact output.
    pub const fn from_workspace_verifier(
        owner_pubkey: PublicKey,
        delegate_pubkey: PublicKey,
        relationship_id: Uuid,
        relationship_revision: u64,
        expires_at: Option<u64>,
        transport_wide: bool,
    ) -> Self {
        Self {
            owner_pubkey,
            delegate_pubkey,
            relationship_id,
            relationship_revision,
            expires_at,
            transport_wide,
        }
    }
}

/// Stateless factory for sealed evidence.
///
/// This value carries no configuration and cannot select a domain, provider,
/// or capability. Every method requires the server-resolved values again and
/// rejects mismatches in the translated verifier/store output.
#[derive(Debug, Default, Clone, Copy)]
pub struct VerifiedEvidenceAdapter;

fn operation_binding(
    kind: VerifiedOperationBindingKind,
    parts: &[&[u8]],
) -> VerifiedOperationBinding {
    let mut hasher = Sha256::new();
    hasher.update(b"buzz-auth:verified-operation-binding:v1");
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    VerifiedOperationBinding::from_evidence_adapter(kind, hasher.finalize().into())
}

impl VerifiedEvidenceAdapter {
    /// Create the trusted-workspace adapter.
    pub const fn new() -> Self {
        Self
    }

    /// Attach exact transport-wide delegation output to an already sealed
    /// cryptographic proof.
    ///
    /// This consumes the original proof, rejects replacement of an existing
    /// delegation, and rechecks that the verified delegate is the proof actor.
    pub fn attach_transport_delegation(
        &self,
        proof: VerifiedNostrProof,
        delegation: VerifiedDelegationOutput,
    ) -> Result<VerifiedNostrProof, EvidenceAdapterError> {
        if proof.verified_delegation().is_some() {
            return Err(EvidenceAdapterError::DelegationAlreadyPresent);
        }
        let verified_delegation = self.delegation(proof.actor_pubkey(), delegation)?;
        VerifiedNostrProof::from_evidence_adapter(
            proof.authorization_domain(),
            proof.authorized_transport(),
            proof.actor_pubkey(),
            proof.proof_method(),
            proof.operation_binding(),
            Some(verified_delegation),
        )
        .map_err(Into::into)
    }

    /// Verify NIP-42 and bind its result to relay or audio transport.
    pub fn verify_nip42(
        &self,
        authorization_domain: CommunityId,
        transport: AuthTransport,
        event: &Event,
        expected_challenge: &str,
        relay_url: &str,
        delegation: Option<VerifiedDelegationOutput>,
    ) -> Result<VerifiedNostrProof, EvidenceAdapterError> {
        if !matches!(
            transport,
            AuthTransport::RelayWebSocket | AuthTransport::Audio
        ) {
            return Err(EvidenceAdapterError::TransportMethodMismatch);
        }
        verify_nip42_event(event, expected_challenge, relay_url)?;
        let delegation = delegation
            .map(|delegation| self.delegation(event.pubkey, delegation))
            .transpose()?;
        let binding = operation_binding(
            VerifiedOperationBindingKind::NostrSession,
            &[expected_challenge.as_bytes(), relay_url.as_bytes()],
        );
        VerifiedNostrProof::from_evidence_adapter(
            authorization_domain,
            transport,
            event.pubkey,
            AuthMethod::Nip42,
            binding,
            delegation,
        )
        .map_err(Into::into)
    }

    /// Verify NIP-98 and bind it to one exact HTTP transport operation.
    // The individual arguments are intentional trust-boundary inputs: folding
    // them into an unverified request bag would make it easier to omit an exact
    // domain, transport, method, body, or delegation cross-check.
    #[allow(clippy::too_many_arguments)]
    pub fn verify_nip98(
        &self,
        authorization_domain: CommunityId,
        transport: AuthTransport,
        event_json: &str,
        expected_url: &str,
        expected_method: &str,
        body: Option<&[u8]>,
        delegation: Option<VerifiedDelegationOutput>,
    ) -> Result<VerifiedNostrProof, EvidenceAdapterError> {
        if !matches!(
            transport,
            AuthTransport::HttpBridge | AuthTransport::Git | AuthTransport::MediaUpload
        ) {
            return Err(EvidenceAdapterError::TransportMethodMismatch);
        }
        let actor = verify_nip98_event(event_json, expected_url, expected_method, body)?;
        let delegation = delegation
            .map(|delegation| self.delegation(actor, delegation))
            .transpose()?;
        let body_presence = [u8::from(body.is_some())];
        let binding = operation_binding(
            VerifiedOperationBindingKind::HttpRequest,
            &[
                expected_method.as_bytes(),
                expected_url.as_bytes(),
                &body_presence,
                body.unwrap_or_default(),
            ],
        );
        VerifiedNostrProof::from_evidence_adapter(
            authorization_domain,
            transport,
            actor,
            AuthMethod::Nip98,
            binding,
            delegation,
        )
        .map_err(Into::into)
    }

    /// Fully verify and bind one exact Blossom upload operation.
    pub fn verify_blossom_upload(
        &self,
        authorization_domain: CommunityId,
        event: &Event,
        sha256: &str,
        server_domain: Option<&str>,
        max_age_secs: u64,
    ) -> Result<VerifiedNostrProof, EvidenceAdapterError> {
        crate::blossom::verify_blossom_upload_auth(event, sha256, server_domain, max_age_secs)?;
        let event_id = event.id.to_bytes();
        let max_age = max_age_secs.to_be_bytes();
        let binding = operation_binding(
            VerifiedOperationBindingKind::BlossomUpload,
            &[
                &event_id,
                sha256.as_bytes(),
                server_domain.unwrap_or_default().as_bytes(),
                &max_age,
            ],
        );
        self.blossom_proof(
            authorization_domain,
            AuthTransport::MediaUpload,
            event,
            binding,
        )
    }

    /// Fully verify and bind one exact Blossom GET or HEAD operation.
    pub fn verify_blossom_download(
        &self,
        authorization_domain: CommunityId,
        event: &Event,
        sha256: &str,
        server_domain: Option<&str>,
        max_age_secs: u64,
    ) -> Result<VerifiedNostrProof, EvidenceAdapterError> {
        crate::blossom::verify_blossom_get_auth(event, sha256, server_domain, max_age_secs)?;
        let event_id = event.id.to_bytes();
        let max_age = max_age_secs.to_be_bytes();
        let binding = operation_binding(
            VerifiedOperationBindingKind::BlossomDownload,
            &[
                &event_id,
                sha256.as_bytes(),
                server_domain.unwrap_or_default().as_bytes(),
                &max_age,
            ],
        );
        self.blossom_proof(
            authorization_domain,
            AuthTransport::MediaDownload,
            event,
            binding,
        )
    }

    fn blossom_proof(
        &self,
        authorization_domain: CommunityId,
        transport: AuthTransport,
        event: &Event,
        binding: VerifiedOperationBinding,
    ) -> Result<VerifiedNostrProof, EvidenceAdapterError> {
        VerifiedNostrProof::from_evidence_adapter(
            authorization_domain,
            transport,
            event.pubkey,
            AuthMethod::Blossom,
            binding,
            None,
        )
        .map_err(Into::into)
    }

    fn delegation(
        &self,
        actor: PublicKey,
        output: VerifiedDelegationOutput,
    ) -> Result<VerifiedTransportDelegation, EvidenceAdapterError> {
        if !output.transport_wide {
            return Err(EvidenceAdapterError::NarrowDelegation);
        }
        if output.delegate_pubkey != actor {
            return Err(EvidenceAdapterError::DelegatedActorMismatch);
        }
        let expires_at = output.expires_at.map(DelegationExpiry::new).transpose()?;
        VerifiedTransportDelegation::new_unrestricted(
            output.owner_pubkey,
            output.delegate_pubkey,
            output.relationship_id,
            output.relationship_revision,
            expires_at,
        )
        .map_err(Into::into)
    }

    /// Translate validated assertion claims into sealed, exact-bound evidence.
    ///
    /// `now_unix_seconds` and every claim argument must be copied from the
    /// successful configured assertion-verifier result, never decoded again
    /// from request data at this boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn federated_assertion_from_validated_claims(
        &self,
        authorization_domain: CommunityId,
        transport: AuthTransport,
        issuer: &str,
        subject: &str,
        attested_pubkey: Option<PublicKey>,
        assertion_transport: AssertionTransport,
        not_before: Option<u64>,
        expires_at: u64,
        now_unix_seconds: u64,
    ) -> Result<VerifiedFederatedAssertion, EvidenceAdapterError> {
        let principal = FederatedPrincipal::new(issuer, subject)?;
        let expires_at = AssertionExpiry::new(expires_at)?;
        if expires_at.is_expired_at(now_unix_seconds) {
            return Err(EvidenceAdapterError::AssertionExpired);
        }
        let not_before = not_before.map(AssertionNotBefore::new);
        if not_before.is_some_and(|bound| bound.is_not_yet_valid_at(now_unix_seconds)) {
            return Err(EvidenceAdapterError::AssertionNotYetValid);
        }
        let key_attestation = attested_pubkey.map(VerifiedKeyAttestation::from_evidence_adapter);
        Ok(VerifiedFederatedAssertion::from_evidence_adapter(
            authorization_domain,
            transport,
            principal,
            key_attestation,
            assertion_transport,
            not_before,
            expires_at,
        ))
    }

    /// Seal provider-neutral evidence from an already verified assertion.
    ///
    /// The adapter rejects malformed or stale freshness bounds before the
    /// evidence can reach a protected route. The profile and capabilities are
    /// typed server-side values and cannot be selected by raw request input.
    pub fn provider_evidence_from_verified_assertion(
        &self,
        assertion: VerifiedFederatedAssertion,
        profile_id: AuthorizationProfileId,
        capabilities: CapabilitySet,
        issued_at: u64,
        fresh_until: u64,
        now_unix_seconds: u64,
    ) -> Result<VerifiedProviderEvidence, EvidenceAdapterError> {
        if issued_at == 0 || issued_at > now_unix_seconds {
            return Err(EvidenceAdapterError::ProviderEvidenceNotYetValid);
        }
        if fresh_until <= issued_at
            || fresh_until <= now_unix_seconds
            || fresh_until > assertion.expires_at().unix_seconds()
        {
            return Err(EvidenceAdapterError::ProviderEvidenceExpired);
        }
        if assertion
            .not_before()
            .is_some_and(|bound| bound.is_not_yet_valid_at(now_unix_seconds))
        {
            return Err(EvidenceAdapterError::AssertionNotYetValid);
        }
        if assertion.expires_at().is_expired_at(now_unix_seconds) {
            return Err(EvidenceAdapterError::AssertionExpired);
        }
        Ok(VerifiedProviderEvidence::from_evidence_adapter(
            assertion,
            profile_id,
            capabilities,
            issued_at,
            fresh_until,
        ))
    }

    /// Translate a typed active binding-store result.
    ///
    /// The adapter derives the authorization reason from lifecycle resolution
    /// and provenance. Callers cannot label an enrolled row as pre-existing or
    /// turn a provisioned row into a first-use enrollment.
    #[allow(clippy::too_many_arguments)]
    pub fn active_binding_from_store(
        &self,
        authorization_domain: CommunityId,
        binding_domain: CommunityId,
        binding_id: Uuid,
        issuer: &str,
        subject: &str,
        bound_pubkey: PublicKey,
        binding_version: u64,
        expires_at: Option<u64>,
        source: BindingSource,
        resolution: ActiveBindingResolution,
        assertion: Option<&VerifiedFederatedAssertion>,
    ) -> Result<VersionedBindingRef, EvidenceAdapterError> {
        if binding_domain != authorization_domain {
            return Err(EvidenceAdapterError::BindingDomainMismatch);
        }
        let binding_version = BindingVersion::new(binding_version)?;
        let expires_at = expires_at.map(BindingExpiry::new).transpose()?;
        let principal = FederatedPrincipal::new(issuer, subject)?;
        if let Some(assertion) = assertion {
            if assertion.authorization_domain() != authorization_domain
                || assertion.principal() != &principal
            {
                return Err(EvidenceAdapterError::AssertionBindingMismatch);
            }
            if assertion
                .key_attestation()
                .is_some_and(|key| key.pubkey() != bound_pubkey)
            {
                return Err(EvidenceAdapterError::AssertionBindingMismatch);
            }
        }
        let reason = match (resolution, source) {
            (ActiveBindingResolution::Existing, _) => AuthorizationReason::ExistingBinding,
            (ActiveBindingResolution::Enrolled, BindingSource::AttestedKey) => {
                if assertion
                    .and_then(VerifiedFederatedAssertion::key_attestation)
                    .is_none_or(|key| key.pubkey() != bound_pubkey)
                {
                    return Err(EvidenceAdapterError::AttestationRequired);
                }
                AuthorizationReason::EnrolledAttestedKey
            }
            (ActiveBindingResolution::Enrolled, BindingSource::Tofu) => {
                AuthorizationReason::EnrolledTofu
            }
            (ActiveBindingResolution::Enrolled, BindingSource::Provisioned) => {
                return Err(EvidenceAdapterError::InvalidBindingResolution)
            }
        };
        VersionedBindingRef::from_evidence_adapter(
            authorization_domain,
            binding_id,
            principal,
            bound_pubkey,
            binding_version,
            expires_at,
            source,
            reason,
        )
        .map_err(Into::into)
    }

    /// Translate successful local community policy into sealed admission.
    pub fn community_access_from_policy(
        &self,
        tenant: &TenantContext,
        resolved_domain: CommunityId,
        scopes: Vec<Scope>,
        channel_ids: Option<Vec<Uuid>>,
    ) -> Result<AuthorizedCommunityAccess, EvidenceAdapterError> {
        if tenant.community() != resolved_domain {
            return Err(EvidenceAdapterError::AdmissionDomainMismatch);
        }
        Ok(AuthorizedCommunityAccess::from_evidence_adapter(
            resolved_domain,
            scopes,
            channel_ids,
        ))
    }
}

/// Rejection while translating trusted-workspace verifier/store output.
#[derive(Debug, Error)]
pub enum EvidenceAdapterError {
    /// Existing cryptographic proof failed.
    #[error(transparent)]
    Authentication(#[from] crate::AuthError),
    /// Existing full Blossom operation verification failed.
    #[error(transparent)]
    Blossom(#[from] crate::blossom::BlossomAuthError),
    /// Sealed context evidence was inconsistent.
    #[error(transparent)]
    Context(#[from] AuthContextError),
    /// Proof method cannot establish the requested transport.
    #[error("verified proof method does not match requested transport")]
    TransportMethodMismatch,
    /// Delegation output named a different actor.
    #[error("verified delegation output does not match authenticated actor")]
    DelegatedActorMismatch,
    /// Operation-scoped delegation cannot be promoted to transport-wide.
    #[error("verified delegation output is narrower than the transport")]
    NarrowDelegation,
    /// A sealed proof cannot have its verified delegation replaced.
    #[error("verified transport delegation is already present")]
    DelegationAlreadyPresent,
    /// Validated assertion was already expired.
    #[error("validated assertion is expired")]
    AssertionExpired,
    /// Validated assertion is not yet current.
    #[error("validated assertion is not yet valid")]
    AssertionNotYetValid,
    /// Provider evidence claims an issuance time after the server clock.
    #[error("verified provider evidence is not yet valid")]
    ProviderEvidenceNotYetValid,
    /// Provider evidence has an invalid or expired hard freshness bound.
    #[error("verified provider evidence is expired")]
    ProviderEvidenceExpired,
    /// Binding store result came from another exact domain.
    #[error("active binding output belongs to another authorization domain")]
    BindingDomainMismatch,
    /// Binding does not match the validated assertion.
    #[error("active binding output does not match validated assertion")]
    AssertionBindingMismatch,
    /// Attested-key enrollment did not carry the exact attested key.
    #[error("attested-key enrollment lacks matching attestation")]
    AttestationRequired,
    /// Store provenance cannot produce the supplied lifecycle result.
    #[error("active binding resolution and provenance are inconsistent")]
    InvalidBindingResolution,
    /// Local policy result belongs to another exact domain.
    #[error("community admission belongs to another authorization domain")]
    AdmissionDomainMismatch,
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, RelayUrl, Tag, Timestamp};

    use super::*;

    fn domain(value: u128) -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(value))
    }

    fn neutral_capabilities() -> CapabilitySet {
        CapabilitySet::new(vec![crate::AuthorizationCapability::CommunityRead])
            .expect("synthetic capability set")
    }

    fn neutral_profile(value: &str) -> AuthorizationProfileId {
        AuthorizationProfileId::from_server_configuration(value)
            .expect("synthetic provider profile")
    }

    #[test]
    fn neutral_verified_evidence_accepts_and_rejects_exact_parts() {
        let adapter = VerifiedEvidenceAdapter::new();
        let assertion = adapter
            .federated_assertion_from_validated_claims(
                domain(1),
                AuthTransport::HttpBridge,
                "https://issuer.example",
                "subject",
                None,
                AssertionTransport::TrustedProxy,
                Some(90),
                200,
                100,
            )
            .expect("synthetic verified assertion");
        let evidence = adapter
            .provider_evidence_from_verified_assertion(
                assertion,
                neutral_profile("profile-a"),
                neutral_capabilities(),
                100,
                180,
                100,
            )
            .expect("matching verified provider evidence");

        assert_eq!(
            evidence.validate_for(
                domain(1),
                AuthTransport::HttpBridge,
                &neutral_profile("profile-a"),
                &neutral_capabilities(),
                100,
            ),
            Ok(())
        );
        assert_eq!(
            evidence.validate_for(
                domain(2),
                AuthTransport::HttpBridge,
                &neutral_profile("profile-a"),
                &neutral_capabilities(),
                100,
            ),
            Err(crate::ProviderEvidenceValidationError::AuthorizationDomainMismatch)
        );
        assert_eq!(
            evidence.validate_for(
                domain(1),
                AuthTransport::Git,
                &neutral_profile("profile-a"),
                &neutral_capabilities(),
                100,
            ),
            Err(crate::ProviderEvidenceValidationError::TransportMismatch)
        );
        assert_eq!(
            evidence.validate_for(
                domain(1),
                AuthTransport::HttpBridge,
                &neutral_profile("profile-b"),
                &neutral_capabilities(),
                100,
            ),
            Err(crate::ProviderEvidenceValidationError::ProfileMismatch)
        );
        assert_eq!(
            evidence.validate_for(
                domain(1),
                AuthTransport::HttpBridge,
                &neutral_profile("profile-a"),
                &CapabilitySet::new(vec![crate::AuthorizationCapability::GitRead])
                    .expect("synthetic capability set"),
                100,
            ),
            Err(crate::ProviderEvidenceValidationError::CapabilityMismatch)
        );
        assert_eq!(
            evidence.validate_for(
                domain(1),
                AuthTransport::HttpBridge,
                &neutral_profile("profile-a"),
                &neutral_capabilities(),
                180,
            ),
            Err(crate::ProviderEvidenceValidationError::Expired)
        );
        assert_eq!(
            format!("{evidence:?}"),
            "VerifiedProviderEvidence { authorization_domain: \"[redacted]\", authorized_transport: \"[redacted]\", principal: \"[redacted]\", profile_id: \"[redacted]\", capabilities: \"[redacted]\", issued_at: \"[redacted]\", fresh_until: \"[redacted]\" }"
        );
    }

    #[test]
    fn nip42_factory_reverifies_signature_challenge_transport_and_actor() {
        let adapter = VerifiedEvidenceAdapter::new();
        let actor = Keys::generate();
        let owner = Keys::generate();
        let challenge = "challenge";
        let relay = "wss://relay.example";
        let event = EventBuilder::auth(challenge, RelayUrl::parse(relay).expect("relay url"))
            .sign_with_keys(&actor)
            .expect("auth event");

        let proof = adapter
            .verify_nip42(
                domain(1),
                AuthTransport::RelayWebSocket,
                &event,
                challenge,
                relay,
                Some(VerifiedDelegationOutput::from_workspace_verifier(
                    owner.public_key(),
                    actor.public_key(),
                    Uuid::from_u128(0x701),
                    1,
                    None,
                    true,
                )),
            )
            .expect("verified NIP-42 proof");
        assert_eq!(
            proof.operation_binding().kind(),
            VerifiedOperationBindingKind::NostrSession
        );
        let substituted = adapter
            .verify_nip42(
                domain(1),
                AuthTransport::RelayWebSocket,
                &event,
                challenge,
                "wss://other.example",
                None,
            )
            .expect_err("relay URL substitution must fail full verification");
        assert!(matches!(
            substituted,
            EvidenceAdapterError::Authentication(_)
        ));
        assert!(matches!(
            adapter.verify_nip42(
                domain(1),
                AuthTransport::Git,
                &event,
                challenge,
                relay,
                None,
            ),
            Err(EvidenceAdapterError::TransportMethodMismatch)
        ));
        assert!(matches!(
            adapter.verify_nip42(
                domain(1),
                AuthTransport::RelayWebSocket,
                &event,
                challenge,
                relay,
                Some(VerifiedDelegationOutput::from_workspace_verifier(
                    owner.public_key(),
                    Keys::generate().public_key(),
                    Uuid::from_u128(0x702),
                    1,
                    None,
                    true,
                )),
            ),
            Err(EvidenceAdapterError::DelegatedActorMismatch)
        ));
    }

    #[test]
    fn assertion_and_binding_factories_reject_cross_domain_and_forged_provenance() {
        let adapter = VerifiedEvidenceAdapter::new();
        let actor = Keys::generate().public_key();
        let assertion = adapter
            .federated_assertion_from_validated_claims(
                domain(1),
                AuthTransport::HttpBridge,
                "https://issuer.example",
                "subject",
                Some(actor),
                AssertionTransport::TrustedProxy,
                None,
                200,
                100,
            )
            .expect("assertion");
        assert!(matches!(
            adapter.active_binding_from_store(
                domain(1),
                domain(2),
                Uuid::new_v4(),
                "https://issuer.example",
                "subject",
                actor,
                1,
                None,
                BindingSource::AttestedKey,
                ActiveBindingResolution::Existing,
                Some(&assertion),
            ),
            Err(EvidenceAdapterError::BindingDomainMismatch)
        ));
        assert!(matches!(
            adapter.active_binding_from_store(
                domain(1),
                domain(1),
                Uuid::new_v4(),
                "https://issuer.example",
                "subject",
                actor,
                1,
                None,
                BindingSource::Provisioned,
                ActiveBindingResolution::Enrolled,
                Some(&assertion),
            ),
            Err(EvidenceAdapterError::InvalidBindingResolution)
        ));
    }

    #[test]
    fn blossom_factories_reverify_exact_hash_verb_and_server() {
        let adapter = VerifiedEvidenceAdapter::new();
        let expiration = (Timestamp::now().as_secs() + 300).to_string();
        let upload_hash = "a".repeat(64);
        let substituted_hash = "b".repeat(64);
        let upload = EventBuilder::new(Kind::from(24_242), "upload")
            .tags([
                Tag::parse(["t", "upload"]).expect("verb"),
                Tag::parse(["x", &upload_hash]).expect("hash"),
                Tag::parse(["server", "relay.example"]).expect("server"),
                Tag::parse(["expiration", &expiration]).expect("expiration"),
            ])
            .sign_with_keys(&Keys::generate())
            .expect("event");

        assert!(adapter
            .verify_blossom_upload(domain(1), &upload, &upload_hash, Some("relay.example"), 600,)
            .is_ok());
        assert!(adapter
            .verify_blossom_upload(
                domain(1),
                &upload,
                &substituted_hash,
                Some("relay.example"),
                600,
            )
            .is_err());
        assert!(adapter
            .verify_blossom_download(domain(1), &upload, &upload_hash, Some("relay.example"), 600,)
            .is_err());
        assert!(adapter
            .verify_blossom_upload(domain(1), &upload, &upload_hash, Some("other.example"), 600,)
            .is_err());
    }
}
