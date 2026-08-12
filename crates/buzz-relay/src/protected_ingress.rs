//! Shared provider-neutral protected-ingress composition.

use axum::http::HeaderMap;
use buzz_auth::{
    AuthorizationFinalizer, FinalizedAuthContext, ProofTransport, RouteCapability,
    VerifiedFederatedAssertion, VerifiedNostrProof,
};
use buzz_core::CommunityId;
use buzz_db::authorization_admission::{
    AdmissionCommitError, AdmissionCommitRequest, AdmissionObject, CanonicalProtectedIntent,
};
use uuid::Uuid;

use crate::authorization_runtime::{ProtectedEffect, ProtectedIngress};
use crate::state::{AppState, CanonicalAssertionError};

/// Stable fail-closed outcomes exposed to protected route adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProtectedIngressError {
    /// Credential, binding, policy, or lifecycle evidence denied the request.
    Denied,
    /// The canonical assertion crossed its half-open expiry bound.
    Expired,
    /// A required authority dependency could not be observed.
    Unavailable,
}

impl ProtectedIngressError {
    /// Stable credential-free code for protocol adapters.
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Denied => "nip_fi_auth_denied",
            Self::Expired => "nip_fi_auth_expired",
            Self::Unavailable => "nip_fi_auth_unavailable",
        }
    }
}

/// Exact server-derived coordinates shared by assertion, proof, and storage.
#[derive(Clone, Copy)]
pub(crate) struct ProtectedRequestCoordinates {
    ingress: ProtectedIngress,
    domain: CommunityId,
    capability: RouteCapability,
    object: AdmissionObject,
    transport: ProofTransport,
    request_fingerprint: [u8; 32],
    transport_context_fingerprint: [u8; 32],
}

impl ProtectedRequestCoordinates {
    /// Seal non-sentinel coordinates before any verifier or database lookup.
    pub(crate) fn new(
        ingress: ProtectedIngress,
        domain: CommunityId,
        capability: RouteCapability,
        object: AdmissionObject,
        transport: ProofTransport,
        request_fingerprint: [u8; 32],
        transport_context_fingerprint: [u8; 32],
    ) -> Result<Self, ProtectedIngressError> {
        if domain.as_uuid().is_nil()
            || request_fingerprint == [0; 32]
            || transport_context_fingerprint == [0; 32]
        {
            return Err(ProtectedIngressError::Denied);
        }
        Ok(Self {
            ingress,
            domain,
            capability,
            object,
            transport,
            request_fingerprint,
            transport_context_fingerprint,
        })
    }
}

impl std::fmt::Debug for ProtectedRequestCoordinates {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProtectedRequestCoordinates([REDACTED])")
    }
}

/// Extract one bounded exact assertion without accepting ambiguous headers.
pub(crate) fn exact_assertion(
    headers: &HeaderMap,
    header_name: &str,
) -> Result<String, ProtectedIngressError> {
    let mut values = headers.get_all(header_name).iter();
    let value = values
        .next()
        .filter(|_| values.next().is_none())
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .and_then(|value| value.strip_prefix("Bearer ").or(Some(value)))
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 64 * 1024
                && value.split('.').count() == 3
                && !value.contains(',')
        })
        .ok_or(ProtectedIngressError::Denied)?;
    Ok(value.to_owned())
}

/// Verify the assertion half of one exact canonical protected request.
pub(crate) async fn verify_assertion(
    state: &AppState,
    assertion_token: &str,
    coordinates: ProtectedRequestCoordinates,
) -> Result<VerifiedFederatedAssertion, ProtectedIngressError> {
    resolve_route(state, coordinates, None)?;
    let authority = state
        .canonical_protected_authority()
        .ok_or(ProtectedIngressError::Unavailable)?;
    authority
        .assertion_verifier()
        .verify(
            assertion_token,
            coordinates.domain,
            coordinates.transport,
            *coordinates.object.key(),
            coordinates.request_fingerprint,
            coordinates.transport_context_fingerprint,
        )
        .await
        .map_err(map_assertion_error)
}

const fn map_assertion_error(error: CanonicalAssertionError) -> ProtectedIngressError {
    match error {
        CanonicalAssertionError::Denied => ProtectedIngressError::Denied,
        CanonicalAssertionError::Expired => ProtectedIngressError::Expired,
        CanonicalAssertionError::Unavailable => ProtectedIngressError::Unavailable,
    }
}

/// Finalize one side-effect-free protected read through the shared authority.
pub(crate) async fn authorize_read(
    state: &AppState,
    coordinates: ProtectedRequestCoordinates,
    assertion: VerifiedFederatedAssertion,
    proof: VerifiedNostrProof,
) -> Result<FinalizedAuthContext, ProtectedIngressError> {
    resolve_route(state, coordinates, Some(ProtectedEffect::Read))?;
    let authority = state
        .canonical_protected_authority()
        .ok_or(ProtectedIngressError::Unavailable)?;
    let prepared = state
        .db
        .prepare_canonical_protected_authorization(
            assertion,
            proof,
            coordinates.capability,
            coordinates.object,
            CanonicalProtectedIntent::Read,
        )
        .await
        .map_err(map_admission_error)?;
    let rechecker = state
        .db
        .canonical_protected_read_rechecker(coordinates.object, authority.final_rechecker());
    let witness = AuthorizationFinalizer::recheck(&prepared, &rechecker)
        .await
        .map_err(|_| ProtectedIngressError::Denied)?;
    AuthorizationFinalizer::finalize(prepared, witness).map_err(|_| ProtectedIngressError::Denied)
}

/// Prepare one exact protected mutation without publishing any route state.
pub(crate) async fn prepare_mutation(
    state: &AppState,
    coordinates: ProtectedRequestCoordinates,
    assertion: VerifiedFederatedAssertion,
    proof: VerifiedNostrProof,
) -> Result<AdmissionCommitRequest, ProtectedIngressError> {
    resolve_route(state, coordinates, Some(ProtectedEffect::Mutate))?;
    let prepared = state
        .db
        .prepare_canonical_protected_authorization(
            assertion,
            proof,
            coordinates.capability,
            coordinates.object,
            CanonicalProtectedIntent::Mutation,
        )
        .await
        .map_err(map_admission_error)?;
    AdmissionCommitRequest::existing(Uuid::new_v4(), coordinates.object, prepared)
        .map_err(map_admission_error)
}

fn resolve_route(
    state: &AppState,
    coordinates: ProtectedRequestCoordinates,
    expected_effect: Option<ProtectedEffect>,
) -> Result<(), ProtectedIngressError> {
    let route = state
        .authorization_runtime
        .routes()
        .map_err(|_| ProtectedIngressError::Unavailable)?
        .resolve(coordinates.ingress, coordinates.transport)
        .map_err(|_| ProtectedIngressError::Denied)?;
    if route.capability() != coordinates.capability
        || expected_effect.is_some_and(|effect| route.effect() != effect)
    {
        return Err(ProtectedIngressError::Denied);
    }
    Ok(())
}

/// Validate one capability-specific Enforce session grant before any quota,
/// read, or mutation owned by the WebSocket dispatcher.
pub(crate) fn session_authorizes(
    state: &AppState,
    ingress: ProtectedIngress,
    authorization: &FinalizedAuthContext,
    domain: CommunityId,
    actor: nostr::PublicKey,
) -> bool {
    let Ok(route) = state
        .authorization_runtime
        .routes()
        .and_then(|routes| routes.resolve(ingress, ProofTransport::Nip42))
    else {
        return false;
    };
    authorization.authorization_domain() == domain
        && authorization.actor_pubkey() == actor
        && authorization.capability() == route.capability()
        && authorization.transport() == route.transport()
        && chrono::Utc::now() < authorization.lease().expires_at()
}

/// Build the sole committer paired with shared protected mutation preparation.
pub(crate) fn mutation_committer(
    state: &AppState,
) -> Result<impl buzz_db::authorization_admission::CanonicalAdmissionCommitter, ProtectedIngressError>
{
    let authority = state
        .canonical_protected_authority()
        .ok_or(ProtectedIngressError::Unavailable)?;
    Ok(state
        .db
        .canonical_protected_committer(authority.final_rechecker()))
}

fn map_admission_error(error: AdmissionCommitError) -> ProtectedIngressError {
    match error {
        AdmissionCommitError::DependencyUnavailable
        | AdmissionCommitError::AuditUnavailable
        | AdmissionCommitError::RecordedAuditUnavailable => ProtectedIngressError::Unavailable,
        _ => ProtectedIngressError::Denied,
    }
}

/// Domain-separated SHA-256 helper for server-owned route coordinates.
pub(crate) fn fingerprint(domain: &[u8], fields: &[&[u8]]) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    digest.update((domain.len() as u64).to_be_bytes());
    digest.update(domain);
    for field in fields {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    digest.finalize().into()
}

/// Resolve the one domain-wide protected-object coordinate shared by every
/// transport adapter. Route names and wire encodings never create parallel
/// authority lineages for the same authorization domain.
pub(crate) fn domain_object(domain: CommunityId) -> Result<AdmissionObject, ProtectedIngressError> {
    let key = fingerprint(
        b"buzz:nip-fi:domain-target:v1",
        &[domain.as_uuid().as_bytes()],
    );
    AdmissionObject::new(
        buzz_db::authorization_admission::AdmissionObjectKind::Domain,
        key,
    )
    .ok_or(ProtectedIngressError::Denied)
}

#[cfg(test)]
mod tests {
    use super::{map_assertion_error, ProtectedIngressError};
    use crate::state::CanonicalAssertionError;
    use buzz_auth::RouteCapability;
    use buzz_db::authorization_admission::{
        protected_capability_matches, AdmissionObjectKind, CanonicalProtectedIntent,
    };

    #[test]
    fn expired_assertion_preserves_stable_transport_code() {
        let mapped = map_assertion_error(CanonicalAssertionError::Expired);
        assert_eq!(mapped, ProtectedIngressError::Expired);
        assert_eq!(mapped.code(), "nip_fi_auth_expired");
    }

    #[test]
    fn canonical_protected_object_capability_matrix_is_exact() {
        const ALL_CAPABILITIES: [RouteCapability; 29] = [
            RouteCapability::MessagesRead,
            RouteCapability::MessagesWrite,
            RouteCapability::ChannelsRead,
            RouteCapability::ChannelsWrite,
            RouteCapability::AdminChannels,
            RouteCapability::UsersRead,
            RouteCapability::UsersWrite,
            RouteCapability::AdminUsers,
            RouteCapability::JobsRead,
            RouteCapability::JobsWrite,
            RouteCapability::SubscriptionsRead,
            RouteCapability::SubscriptionsWrite,
            RouteCapability::FilesRead,
            RouteCapability::FilesWrite,
            RouteCapability::ReposRead,
            RouteCapability::ReposWrite,
            RouteCapability::GitRead,
            RouteCapability::GitWrite,
            RouteCapability::GitStream,
            RouteCapability::MediaRead,
            RouteCapability::MediaWrite,
            RouteCapability::Moderation,
            RouteCapability::AudioJoin,
            RouteCapability::AudioMedia,
            RouteCapability::Discovery,
            RouteCapability::BindingStatus,
            RouteCapability::Enrollment,
            RouteCapability::InviteMint,
            RouteCapability::InviteClaim,
        ];
        let matrix: &[(
            AdmissionObjectKind,
            CanonicalProtectedIntent,
            &[RouteCapability],
        )] = &[
            (
                AdmissionObjectKind::Domain,
                CanonicalProtectedIntent::Read,
                &[
                    RouteCapability::MessagesRead,
                    RouteCapability::MessagesWrite,
                    RouteCapability::Discovery,
                ],
            ),
            (
                AdmissionObjectKind::Domain,
                CanonicalProtectedIntent::Mutation,
                &[],
            ),
            (
                AdmissionObjectKind::Channel,
                CanonicalProtectedIntent::Read,
                &[
                    RouteCapability::ChannelsRead,
                    RouteCapability::ChannelsWrite,
                    RouteCapability::MessagesRead,
                    RouteCapability::MessagesWrite,
                ],
            ),
            (
                AdmissionObjectKind::Channel,
                CanonicalProtectedIntent::Mutation,
                &[],
            ),
            (
                AdmissionObjectKind::Repository,
                CanonicalProtectedIntent::Read,
                &[
                    RouteCapability::ReposRead,
                    RouteCapability::GitRead,
                    RouteCapability::GitStream,
                ],
            ),
            (
                AdmissionObjectKind::Repository,
                CanonicalProtectedIntent::Mutation,
                &[RouteCapability::ReposWrite, RouteCapability::GitWrite],
            ),
            (
                AdmissionObjectKind::Media,
                CanonicalProtectedIntent::Read,
                &[RouteCapability::MediaRead],
            ),
            (
                AdmissionObjectKind::Media,
                CanonicalProtectedIntent::Mutation,
                &[RouteCapability::MediaWrite],
            ),
            (
                AdmissionObjectKind::ModerationTarget,
                CanonicalProtectedIntent::Read,
                &[RouteCapability::Moderation],
            ),
            (
                AdmissionObjectKind::ModerationTarget,
                CanonicalProtectedIntent::Mutation,
                &[RouteCapability::Moderation],
            ),
            (
                AdmissionObjectKind::AudioSession,
                CanonicalProtectedIntent::Read,
                &[RouteCapability::AudioJoin, RouteCapability::AudioMedia],
            ),
            (
                AdmissionObjectKind::AudioSession,
                CanonicalProtectedIntent::Mutation,
                &[],
            ),
            (
                AdmissionObjectKind::Event,
                CanonicalProtectedIntent::Read,
                &[],
            ),
            (
                AdmissionObjectKind::Event,
                CanonicalProtectedIntent::Mutation,
                &[RouteCapability::MessagesWrite],
            ),
            (
                AdmissionObjectKind::BindingStatus,
                CanonicalProtectedIntent::Read,
                &[],
            ),
            (
                AdmissionObjectKind::BindingStatus,
                CanonicalProtectedIntent::Mutation,
                &[RouteCapability::BindingStatus],
            ),
            (
                AdmissionObjectKind::Invitation,
                CanonicalProtectedIntent::Read,
                &[],
            ),
            (
                AdmissionObjectKind::Invitation,
                CanonicalProtectedIntent::Mutation,
                &[RouteCapability::InviteMint],
            ),
        ];

        for (kind, intent, allowed) in matrix {
            for capability in ALL_CAPABILITIES {
                assert_eq!(
                    protected_capability_matches(*kind, capability, *intent),
                    allowed.contains(&capability),
                    "unexpected matrix result for {kind:?}/{intent:?}/{capability:?}"
                );
            }
        }
    }
}
