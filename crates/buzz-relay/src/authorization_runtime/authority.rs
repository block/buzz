//! Canonical relay-side admission and resource-witness composition.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use buzz_auth::{
    AuthorizationLeaseDependencySnapshot, FinalizedAuthContext, PreparedAuthorization,
    ProofTransport, RouteCapability, VerifiedNostrProof,
};
use buzz_core::CommunityId;
use buzz_db::authorization_admission::AdmissionEnrollmentEvidence;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::{
    InvalidationRegistration, InvalidationRegistry, ProtectedResourceKind, ResolvedProtectedRoute,
    RouteAuthority, RuntimeAuthorizationError,
};

/// Origin-sealed server resource used for pre-effect and streaming rechecks.
///
/// Production adapters own authoritative implementations. Raw handler paths, headers, and body
/// fields cannot implement authority by themselves; the adapter must first
/// verify transport provenance and resolve the server-owned object.
#[async_trait]
pub trait ProtectedResourceWitness: Send + Sync {
    /// Server-resolved authorization domain.
    fn authorization_domain(&self) -> CommunityId;
    /// Exact sealed route capability.
    fn capability(&self) -> RouteCapability;
    /// Closed resource namespace.
    fn resource_kind(&self) -> ProtectedResourceKind;
    /// Privacy-safe server-owned resource key.
    fn resource_key(&self) -> &[u8; 32];
    /// Independently verified proof transport.
    fn transport(&self) -> ProofTransport;
    /// Canonical request fingerprint.
    fn request_fingerprint(&self) -> &[u8; 32];
    /// Canonical target fingerprint.
    fn target_fingerprint(&self) -> &[u8; 32];
    /// Canonical transport-context fingerprint.
    fn transport_context_fingerprint(&self) -> &[u8; 32];
    /// Protected-object authority epoch captured during preparation.
    fn authority_epoch(&self) -> u64;
    /// Exclusive witness expiry.
    fn expires_at(&self) -> DateTime<Utc>;
    /// Optional opaque replay-claim key supplied only by the sealed transport.
    fn replay_claim_key(&self) -> Option<&[u8; 32]> {
        None
    }
    /// Exclusive replay-claim retention bound, when a claim is present.
    fn replay_claim_retain_until(&self) -> Option<DateTime<Utc>> {
        None
    }

    /// Re-read the authoritative resource after canonical admission and before
    /// application effect. Streaming callers invoke this periodically too.
    async fn recheck(
        &self,
        lease: &AuthorizationLeaseDependencySnapshot,
    ) -> Result<ResourceRecheck, RuntimeAuthorizationError>;
}

/// Fresh authoritative observation of one sealed resource witness.
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceRecheck {
    authorization_domain: CommunityId,
    resource_kind: ProtectedResourceKind,
    resource_key: [u8; 32],
    authority_epoch: u64,
    authoritative_now: DateTime<Utc>,
}

impl ResourceRecheck {
    /// Adapt one authoritative transport observation.
    pub fn from_authoritative_parts(
        authorization_domain: CommunityId,
        resource_kind: ProtectedResourceKind,
        resource_key: [u8; 32],
        authority_epoch: u64,
        authoritative_now: DateTime<Utc>,
    ) -> Result<Self, RuntimeAuthorizationError> {
        if authorization_domain.as_uuid().is_nil()
            || resource_key == [0; 32]
            || authority_epoch == 0
        {
            return Err(RuntimeAuthorizationError::ResourceWitnessMismatch);
        }
        Ok(Self {
            authorization_domain,
            resource_kind,
            resource_key,
            authority_epoch,
            authoritative_now,
        })
    }
}

impl fmt::Debug for ResourceRecheck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResourceRecheck([REDACTED])")
    }
}

/// Complete relay-side request for the corrected canonical admission adapter.
///
/// The adapter converts this value to the exact canonical request at the
/// application boundary. This module never performs enrollment, replay, audit,
/// or receipt writes itself.
pub struct RuntimeAdmissionRequest {
    operation_id: Uuid,
    attempt_id: Uuid,
    route: ResolvedProtectedRoute,
    preparation: RuntimeAdmissionPreparation,
    witness: Arc<dyn ProtectedResourceWitness>,
}

pub(super) enum RuntimeAdmissionPreparation {
    Existing(Box<PreparedAuthorization>),
    Enrollment(Box<RuntimeEnrollmentPreparation>),
}

pub(super) struct RuntimeEnrollmentPreparation {
    pub(super) correlation_id: Uuid,
    pub(super) evidence: AdmissionEnrollmentEvidence,
}

impl RuntimeAdmissionRequest {
    /// Bind a read-only authorization preparation to the independently sealed route and
    /// resource coordinates that the final transaction must consume.
    pub fn existing(
        operation_id: Uuid,
        attempt_id: Uuid,
        route: ResolvedProtectedRoute,
        prepared: PreparedAuthorization,
        witness: Arc<dyn ProtectedResourceWitness>,
    ) -> Result<Self, RuntimeAuthorizationError> {
        if operation_id.is_nil() || attempt_id.is_nil() {
            return Err(RuntimeAuthorizationError::ResourceWitnessMismatch);
        }
        validate_prepared_coordinates(route, &prepared, witness.as_ref())?;
        validate_replay_claim(witness.as_ref())?;
        Ok(Self {
            operation_id,
            attempt_id,
            route,
            preparation: RuntimeAdmissionPreparation::Existing(Box::new(prepared)),
            witness,
        })
    }

    /// Bind a transactionally committed enrollment to the same sealed route
    /// and protected resource coordinates.
    #[allow(clippy::too_many_arguments)]
    pub fn enrollment(
        operation_id: Uuid,
        attempt_id: Uuid,
        correlation_id: Uuid,
        route: ResolvedProtectedRoute,
        evidence: AdmissionEnrollmentEvidence,
        proof: VerifiedNostrProof,
        witness: Arc<dyn ProtectedResourceWitness>,
    ) -> Result<Self, RuntimeAuthorizationError> {
        if operation_id.is_nil() || attempt_id.is_nil() || correlation_id.is_nil() {
            return Err(RuntimeAuthorizationError::ResourceWitnessMismatch);
        }
        validate_enrollment_coordinates(route, &proof, witness.as_ref())?;
        validate_replay_claim(witness.as_ref())?;
        Ok(Self {
            operation_id,
            attempt_id,
            route,
            preparation: RuntimeAdmissionPreparation::Enrollment(Box::new(
                RuntimeEnrollmentPreparation {
                    correlation_id,
                    evidence,
                },
            )),
            witness,
        })
    }

    /// Server-generated idempotency identifier.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// Server-generated bounded-attempt identifier.
    pub const fn attempt_id(&self) -> Uuid {
        self.attempt_id
    }

    /// Exact typed route.
    pub const fn route(&self) -> ResolvedProtectedRoute {
        self.route
    }

    /// Borrow the sealed protected object.
    pub fn witness(&self) -> &dyn ProtectedResourceWitness {
        self.witness.as_ref()
    }

    /// Consume the request into the exact pieces needed by canonical admission.
    pub(super) fn into_parts(
        self,
    ) -> (
        Uuid,
        Uuid,
        ResolvedProtectedRoute,
        RuntimeAdmissionPreparation,
        Arc<dyn ProtectedResourceWitness>,
    ) {
        (
            self.operation_id,
            self.attempt_id,
            self.route,
            self.preparation,
            self.witness,
        )
    }
}

impl fmt::Debug for RuntimeAdmissionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeAdmissionRequest([REDACTED])")
    }
}

/// Object-safe adapter to the sole corrected atomic admission committer.
#[async_trait]
pub trait AdmissionCommitPort: Send + Sync {
    /// Atomically consume replay claim, enrollment, receipt, audit, and
    /// authority state, returning only finalized credential-free state.
    /// Mutation routes remain unavailable until the committer exposes its
    /// typed bounded application-effect input on this same transaction.
    async fn commit(
        &self,
        request: RuntimeAdmissionRequest,
    ) -> Result<FinalizedAuthContext, RuntimeAuthorizationError>;
}

/// Canonical route authority composed from immutable route and admission ports.
pub struct RuntimeAuthority {
    routes: Arc<RouteAuthority>,
    committer: Arc<dyn AdmissionCommitPort>,
    invalidation: Arc<InvalidationRegistry>,
}

impl RuntimeAuthority {
    /// Install one complete route table and one canonical admission port.
    pub fn new(
        routes: Arc<RouteAuthority>,
        committer: Arc<dyn AdmissionCommitPort>,
        invalidation: Arc<InvalidationRegistry>,
    ) -> Self {
        Self {
            routes,
            committer,
            invalidation,
        }
    }

    /// Immutable closed route inventory.
    pub const fn routes(&self) -> &Arc<RouteAuthority> {
        &self.routes
    }

    /// Exact live invalidation registry bound to every admitted lease.
    pub const fn invalidation(&self) -> &Arc<InvalidationRegistry> {
        &self.invalidation
    }

    /// Commit admission, then recheck the independently owned resource before
    /// returning permission to execute the application effect.
    pub async fn authorize(
        &self,
        request: RuntimeAdmissionRequest,
    ) -> Result<AuthorizedRoute, RuntimeAuthorizationError> {
        let route = request.route;
        if route.effect() == super::ProtectedEffect::Mutate {
            return Err(RuntimeAuthorizationError::DependencyUnavailable);
        }
        let witness = Arc::clone(&request.witness);
        let authorization = self.committer.commit(request).await?;
        validate_finalized_coordinates(route, &authorization, witness.as_ref())?;
        let invalidation = self.invalidation.register(authorization.lease())?;
        let lease_snapshot = authorization.lease().dependency_snapshot();
        let rechecked = witness.recheck(&lease_snapshot).await?;
        validate_resource_recheck(&authorization, witness.as_ref(), &rechecked)?;
        if invalidation.is_cancelled() {
            return Err(RuntimeAuthorizationError::InvalidationUnavailable);
        }
        Ok(AuthorizedRoute {
            route,
            authorization,
            witness,
            invalidation,
        })
    }
}

impl fmt::Debug for RuntimeAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeAuthority([REDACTED])")
    }
}

/// Finalized route whose resource was rechecked immediately before effect.
pub struct AuthorizedRoute {
    route: ResolvedProtectedRoute,
    authorization: FinalizedAuthContext,
    witness: Arc<dyn ProtectedResourceWitness>,
    invalidation: InvalidationRegistration,
}

impl AuthorizedRoute {
    /// Exact admitted route.
    pub const fn route(&self) -> ResolvedProtectedRoute {
        self.route
    }

    /// Credential-free finalized context.
    pub const fn authorization(&self) -> &FinalizedAuthContext {
        &self.authorization
    }

    /// Re-fence a long-running operation against current authorization and
    /// resource state. Drift or expiry terminates the operation.
    pub async fn recheck_stream(&self) -> Result<(), RuntimeAuthorizationError> {
        if self.invalidation.is_cancelled() {
            return Err(RuntimeAuthorizationError::InvalidationUnavailable);
        }
        let lease_snapshot = self.authorization.lease().dependency_snapshot();
        let rechecked = self.witness.recheck(&lease_snapshot).await?;
        validate_resource_recheck(&self.authorization, self.witness.as_ref(), &rechecked)
    }
}

impl fmt::Debug for AuthorizedRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorizedRoute([REDACTED])")
    }
}

fn validate_prepared_coordinates(
    route: ResolvedProtectedRoute,
    prepared: &PreparedAuthorization,
    witness: &dyn ProtectedResourceWitness,
) -> Result<(), RuntimeAuthorizationError> {
    let snapshot = prepared.recheck_request().lease_dependencies();
    let (_, domain) = snapshot.identity();
    let (capability, _, _) = snapshot.authority();
    let (request, target, transport, transport_context) = snapshot.request_binding();
    let (_, _, authority_epoch) = snapshot.dependency_versions();
    if domain != witness.authorization_domain()
        || capability != route.capability()
        || capability != witness.capability()
        || route.resource() != witness.resource_kind()
        || route.transport() != transport
        || transport != witness.transport()
        || request != witness.request_fingerprint()
        || target != witness.target_fingerprint()
        || transport_context != witness.transport_context_fingerprint()
        || authority_epoch != witness.authority_epoch()
        || witness.resource_key() == &[0; 32]
        || witness.expires_at() > prepared.expires_at()
    {
        return Err(RuntimeAuthorizationError::ResourceWitnessMismatch);
    }
    Ok(())
}

fn validate_enrollment_coordinates(
    route: ResolvedProtectedRoute,
    proof: &VerifiedNostrProof,
    witness: &dyn ProtectedResourceWitness,
) -> Result<(), RuntimeAuthorizationError> {
    if proof.authorization_domain() != witness.authorization_domain()
        || route.capability() != witness.capability()
        || route.resource() != witness.resource_kind()
        || route.transport() != witness.transport()
        || proof.transport() != witness.transport()
        || proof.request_fingerprint() != witness.request_fingerprint()
        || proof.target_fingerprint() != witness.target_fingerprint()
        || proof.transport_context_fingerprint() != witness.transport_context_fingerprint()
        || witness.resource_key() == &[0; 32]
        || witness.expires_at() > proof.expires_at()
    {
        return Err(RuntimeAuthorizationError::ResourceWitnessMismatch);
    }
    Ok(())
}

fn validate_replay_claim(
    witness: &dyn ProtectedResourceWitness,
) -> Result<(), RuntimeAuthorizationError> {
    match (
        witness.replay_claim_key(),
        witness.replay_claim_retain_until(),
    ) {
        (None, None) => Ok(()),
        (Some(key), Some(retain_until))
            if key != &[0; 32] && retain_until == witness.expires_at() =>
        {
            Ok(())
        }
        _ => Err(RuntimeAuthorizationError::ResourceWitnessMismatch),
    }
}

fn validate_finalized_coordinates(
    route: ResolvedProtectedRoute,
    authorization: &FinalizedAuthContext,
    witness: &dyn ProtectedResourceWitness,
) -> Result<(), RuntimeAuthorizationError> {
    let lease = authorization.lease();
    let (request, target, transport_context) = lease.request_binding();
    let (_, _, authority_epoch) = lease.dependency_versions();
    if authorization.authorization_domain() != witness.authorization_domain()
        || authorization.capability() != route.capability()
        || authorization.capability() != witness.capability()
        || authorization.transport() != route.transport()
        || authorization.transport() != witness.transport()
        || authorization.request_fingerprint() != witness.request_fingerprint()
        || request != witness.request_fingerprint()
        || target != witness.target_fingerprint()
        || transport_context != witness.transport_context_fingerprint()
        || authority_epoch != witness.authority_epoch()
    {
        return Err(RuntimeAuthorizationError::ResourceWitnessMismatch);
    }
    Ok(())
}

fn validate_resource_recheck(
    authorization: &FinalizedAuthContext,
    witness: &dyn ProtectedResourceWitness,
    rechecked: &ResourceRecheck,
) -> Result<(), RuntimeAuthorizationError> {
    if rechecked.authorization_domain != witness.authorization_domain()
        || rechecked.resource_kind != witness.resource_kind()
        || rechecked.resource_key.as_slice() != witness.resource_key()
        || rechecked.authority_epoch != witness.authority_epoch()
        || rechecked.authoritative_now >= witness.expires_at()
        || !authorization
            .lease()
            .is_valid_at(rechecked.authoritative_now)
    {
        return Err(RuntimeAuthorizationError::ResourceWitnessMismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Witness {
        expires_at: DateTime<Utc>,
        claim_key: Option<[u8; 32]>,
        retain_until: Option<DateTime<Utc>>,
    }

    #[async_trait]
    impl ProtectedResourceWitness for Witness {
        fn authorization_domain(&self) -> CommunityId {
            CommunityId::from_uuid(Uuid::from_u128(1))
        }

        fn capability(&self) -> RouteCapability {
            RouteCapability::MessagesRead
        }

        fn resource_kind(&self) -> ProtectedResourceKind {
            ProtectedResourceKind::Domain
        }

        fn resource_key(&self) -> &[u8; 32] {
            static KEY: [u8; 32] = [1; 32];
            &KEY
        }

        fn transport(&self) -> ProofTransport {
            ProofTransport::Nip98
        }

        fn request_fingerprint(&self) -> &[u8; 32] {
            static FINGERPRINT: [u8; 32] = [2; 32];
            &FINGERPRINT
        }

        fn target_fingerprint(&self) -> &[u8; 32] {
            static FINGERPRINT: [u8; 32] = [3; 32];
            &FINGERPRINT
        }

        fn transport_context_fingerprint(&self) -> &[u8; 32] {
            static FINGERPRINT: [u8; 32] = [4; 32];
            &FINGERPRINT
        }

        fn authority_epoch(&self) -> u64 {
            1
        }

        fn expires_at(&self) -> DateTime<Utc> {
            self.expires_at
        }

        fn replay_claim_key(&self) -> Option<&[u8; 32]> {
            self.claim_key.as_ref()
        }

        fn replay_claim_retain_until(&self) -> Option<DateTime<Utc>> {
            self.retain_until
        }

        async fn recheck(
            &self,
            _lease: &AuthorizationLeaseDependencySnapshot,
        ) -> Result<ResourceRecheck, RuntimeAuthorizationError> {
            std::future::pending().await
        }
    }

    #[test]
    fn replay_claim_is_absent_or_exactly_paired_and_expiry_bound() {
        let expires_at = Utc::now() + chrono::Duration::seconds(60);
        let witness = |claim_key, retain_until| Witness {
            expires_at,
            claim_key,
            retain_until,
        };
        assert!(validate_replay_claim(&witness(None, None)).is_ok());
        assert!(validate_replay_claim(&witness(Some([1; 32]), Some(expires_at))).is_ok());
        assert_eq!(
            validate_replay_claim(&witness(Some([0; 32]), Some(expires_at))),
            Err(RuntimeAuthorizationError::ResourceWitnessMismatch)
        );
        assert_eq!(
            validate_replay_claim(&witness(Some([1; 32]), None)),
            Err(RuntimeAuthorizationError::ResourceWitnessMismatch)
        );
        assert_eq!(
            validate_replay_claim(&witness(
                Some([1; 32]),
                Some(expires_at + chrono::Duration::seconds(1)),
            )),
            Err(RuntimeAuthorizationError::ResourceWitnessMismatch)
        );
    }

    #[test]
    fn resource_recheck_rejects_unallocated_server_coordinates() {
        let now = Utc::now();
        assert!(ResourceRecheck::from_authoritative_parts(
            CommunityId::from_uuid(Uuid::from_u128(1)),
            ProtectedResourceKind::Domain,
            [1; 32],
            1,
            now,
        )
        .is_ok());
        assert_eq!(
            ResourceRecheck::from_authoritative_parts(
                CommunityId::from_uuid(Uuid::nil()),
                ProtectedResourceKind::Domain,
                [1; 32],
                1,
                now,
            ),
            Err(RuntimeAuthorizationError::ResourceWitnessMismatch)
        );
        assert_eq!(
            ResourceRecheck::from_authoritative_parts(
                CommunityId::from_uuid(Uuid::from_u128(1)),
                ProtectedResourceKind::Domain,
                [0; 32],
                0,
                now,
            ),
            Err(RuntimeAuthorizationError::ResourceWitnessMismatch)
        );
    }
}
