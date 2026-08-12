//! Operation-bound authorization restore refusal logic.

use std::collections::HashSet;
use std::fmt;

use async_trait::async_trait;
use buzz_core::CommunityId;
use uuid::Uuid;

use super::RuntimeAuthorizationError;

const MAX_RESTORE_DELTAS: usize = 64;

/// Closed durable component namespaces eligible for authorization rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RestoreComponent {
    /// Domain invalidation floor.
    InvalidationFloor,
    /// Local binding generation.
    Binding,
    /// Delegated relationship generation.
    DelegatedRelationship,
    /// Local authorization policy revision.
    Policy,
    /// Protected-object authority state.
    ProtectedObject,
    /// Required immutable audit-chain state.
    Audit,
}

/// Exact pre/post state for one component mutated by one operation.
#[derive(Clone, PartialEq, Eq)]
pub struct RestoreDelta {
    component: RestoreComponent,
    component_key: [u8; 32],
    before: [u8; 32],
    after: [u8; 32],
}

impl RestoreDelta {
    /// Construct one non-sentinel exact state transition.
    pub fn new(
        component: RestoreComponent,
        component_key: [u8; 32],
        before: [u8; 32],
        after: [u8; 32],
    ) -> Result<Self, RuntimeAuthorizationError> {
        if component_key == [0; 32] || before == [0; 32] || after == [0; 32] || before == after {
            return Err(RuntimeAuthorizationError::RestoreRejected);
        }
        Ok(Self {
            component,
            component_key,
            before,
            after,
        })
    }

    /// Closed component namespace.
    pub const fn component(&self) -> RestoreComponent {
        self.component
    }

    /// Privacy-safe component identity.
    pub const fn component_key(&self) -> &[u8; 32] {
        &self.component_key
    }

    /// Exact pre-operation digest.
    pub const fn before(&self) -> &[u8; 32] {
        &self.before
    }

    /// Exact committed post-operation digest.
    pub const fn after(&self) -> &[u8; 32] {
        &self.after
    }
}

impl fmt::Debug for RestoreDelta {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RestoreDelta([REDACTED])")
    }
}

/// Exact operation manifest retained by migration 0033.
#[derive(Clone)]
pub struct RestoreManifest {
    authorization_domain: CommunityId,
    operation_id: Uuid,
    request_fingerprint: [u8; 32],
    policy_revision: u64,
    invalidation_generation: u64,
    audit_predecessor: [u8; 32],
    audit_result: [u8; 32],
    deltas: Vec<RestoreDelta>,
}

impl RestoreManifest {
    /// Validate exact bounded attribution for one committed operation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        authorization_domain: CommunityId,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
        policy_revision: u64,
        invalidation_generation: u64,
        audit_predecessor: [u8; 32],
        audit_result: [u8; 32],
        deltas: Vec<RestoreDelta>,
    ) -> Result<Self, RuntimeAuthorizationError> {
        if authorization_domain.as_uuid().is_nil()
            || operation_id.is_nil()
            || request_fingerprint == [0; 32]
            || policy_revision == 0
            || audit_predecessor == [0; 32]
            || audit_result == [0; 32]
            || audit_predecessor == audit_result
            || deltas.is_empty()
            || deltas.len() > MAX_RESTORE_DELTAS
        {
            return Err(RuntimeAuthorizationError::RestoreRejected);
        }
        let mut coordinates = HashSet::new();
        if deltas
            .iter()
            .any(|delta| !coordinates.insert((delta.component, delta.component_key)))
        {
            return Err(RuntimeAuthorizationError::RestoreRejected);
        }
        Ok(Self {
            authorization_domain,
            operation_id,
            request_fingerprint,
            policy_revision,
            invalidation_generation,
            audit_predecessor,
            audit_result,
            deltas,
        })
    }

    /// Server-resolved authorization domain.
    pub const fn authorization_domain(&self) -> CommunityId {
        self.authorization_domain
    }

    /// Exact operation identity.
    pub const fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    /// Request semantics committed by the operation.
    pub const fn request_fingerprint(&self) -> &[u8; 32] {
        &self.request_fingerprint
    }

    /// Policy and invalidation generations at commit.
    pub const fn dependency_generations(&self) -> (u64, u64) {
        (self.policy_revision, self.invalidation_generation)
    }

    /// Required immutable audit predecessor and result.
    pub const fn audit_edge(&self) -> (&[u8; 32], &[u8; 32]) {
        (&self.audit_predecessor, &self.audit_result)
    }

    /// Exact bounded component transitions.
    pub fn deltas(&self) -> &[RestoreDelta] {
        &self.deltas
    }
}

impl fmt::Debug for RestoreManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RestoreManifest([REDACTED])")
    }
}

/// Current authority observation required before applying any rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreAuthorityObservation {
    /// Exact manifest lineage is current and unconsumed.
    pub lineage_current: bool,
    /// No key removed after the operation would be reintroduced.
    pub removed_keys_absent: bool,
    /// Every identity pair affected by the operation remains active.
    pub identity_pairs_active: bool,
    /// The operation lineage has not already been consumed by another restore.
    pub lineage_unconsumed: bool,
    /// No replay marker conflicts with this exact operation.
    pub replay_absent: bool,
    /// The immutable audit edge is present and continuous.
    pub audit_continuous: bool,
    /// Current application effects still exactly match the manifest.
    pub effects_consistent: bool,
    /// Current policy revision.
    pub policy_revision: u64,
    /// Current invalidation generation.
    pub invalidation_generation: u64,
}

/// Authoritative read-only port used to refuse ambiguous restore.
#[async_trait]
pub trait RestoreStateReader: Send + Sync {
    /// Read current digest for one exact component coordinate.
    async fn current_digest(
        &self,
        domain: CommunityId,
        component: RestoreComponent,
        component_key: &[u8; 32],
    ) -> Result<[u8; 32], RuntimeAuthorizationError>;

    /// Prove exact operation lineage, policy generation, and audit continuity.
    async fn authority_observation(
        &self,
        manifest: &RestoreManifest,
    ) -> Result<RestoreAuthorityObservation, RuntimeAuthorizationError>;
}

/// Read-only decision returned before the canonical transactional restore adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreDecision {
    /// Current state is exactly the recorded post-state and may be rolled back
    /// atomically to the included pre-state.
    Apply(Vec<RestoreDelta>),
    /// Every component already equals the recorded pre-state; repeat is a no-op.
    AlreadyRestored,
}

/// Fail-closed evaluator for one exact operation manifest.
#[derive(Debug, Default, Clone, Copy)]
pub struct RestoreCoordinator;

impl RestoreCoordinator {
    /// Compare current authority and every component with one exact manifest.
    /// Mixed, missing, advanced, or unavailable state is never guessed.
    pub async fn evaluate<R: RestoreStateReader>(
        &self,
        manifest: &RestoreManifest,
        reader: &R,
    ) -> Result<RestoreDecision, RuntimeAuthorizationError> {
        let authority = reader.authority_observation(manifest).await?;
        let (policy_revision, invalidation_generation) = manifest.dependency_generations();
        if !authority.lineage_current
            || !authority.removed_keys_absent
            || !authority.identity_pairs_active
            || !authority.lineage_unconsumed
            || !authority.replay_absent
            || !authority.audit_continuous
            || !authority.effects_consistent
            || authority.policy_revision != policy_revision
            || authority.invalidation_generation != invalidation_generation
        {
            return Err(RuntimeAuthorizationError::RestoreRejected);
        }

        let mut all_before = true;
        let mut all_after = true;
        for delta in manifest.deltas() {
            let current = reader
                .current_digest(
                    manifest.authorization_domain(),
                    delta.component(),
                    delta.component_key(),
                )
                .await?;
            all_before &= current.as_slice() == delta.before();
            all_after &= current.as_slice() == delta.after();
        }
        match (all_before, all_after) {
            (true, false) => Ok(RestoreDecision::AlreadyRestored),
            (false, true) => Ok(RestoreDecision::Apply(manifest.deltas.clone())),
            _ => Err(RuntimeAuthorizationError::RestoreRejected),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    type RestoreCoordinate = (RestoreComponent, [u8; 32]);
    type RestoreValues = HashMap<RestoreCoordinate, [u8; 32]>;

    struct Reader {
        authority: RestoreAuthorityObservation,
        values: Mutex<RestoreValues>,
    }

    #[async_trait]
    impl RestoreStateReader for Reader {
        async fn current_digest(
            &self,
            _domain: CommunityId,
            component: RestoreComponent,
            component_key: &[u8; 32],
        ) -> Result<[u8; 32], RuntimeAuthorizationError> {
            self.values
                .lock()
                .map_err(|_| RuntimeAuthorizationError::RestoreRejected)?
                .get(&(component, *component_key))
                .copied()
                .ok_or(RuntimeAuthorizationError::RestoreRejected)
        }

        async fn authority_observation(
            &self,
            _manifest: &RestoreManifest,
        ) -> Result<RestoreAuthorityObservation, RuntimeAuthorizationError> {
            Ok(self.authority)
        }
    }

    fn manifest() -> RestoreManifest {
        RestoreManifest::new(
            CommunityId::from_uuid(Uuid::from_u128(1)),
            Uuid::from_u128(2),
            [3; 32],
            4,
            5,
            [6; 32],
            [7; 32],
            vec![
                RestoreDelta::new(RestoreComponent::Binding, [8; 32], [9; 32], [10; 32]).unwrap(),
                RestoreDelta::new(
                    RestoreComponent::ProtectedObject,
                    [11; 32],
                    [12; 32],
                    [13; 32],
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn reader(manifest: &RestoreManifest, use_after: bool) -> Reader {
        let values = manifest
            .deltas()
            .iter()
            .map(|delta| {
                (
                    (delta.component(), *delta.component_key()),
                    if use_after {
                        *delta.after()
                    } else {
                        *delta.before()
                    },
                )
            })
            .collect();
        Reader {
            authority: RestoreAuthorityObservation {
                lineage_current: true,
                removed_keys_absent: true,
                identity_pairs_active: true,
                lineage_unconsumed: true,
                replay_absent: true,
                audit_continuous: true,
                effects_consistent: true,
                policy_revision: 4,
                invalidation_generation: 5,
            },
            values: Mutex::new(values),
        }
    }

    #[tokio::test]
    async fn exact_post_state_allows_and_exact_pre_state_is_idempotent() {
        let manifest = manifest();
        assert!(matches!(
            RestoreCoordinator
                .evaluate(&manifest, &reader(&manifest, true))
                .await,
            Ok(RestoreDecision::Apply(_))
        ));
        assert_eq!(
            RestoreCoordinator
                .evaluate(&manifest, &reader(&manifest, false))
                .await,
            Ok(RestoreDecision::AlreadyRestored)
        );
    }

    #[tokio::test]
    async fn mixed_state_and_lineage_or_audit_drift_are_rejected() {
        let manifest = manifest();
        let mixed = reader(&manifest, true);
        mixed
            .values
            .lock()
            .unwrap()
            .insert((RestoreComponent::Binding, [8; 32]), [9; 32]);
        assert_eq!(
            RestoreCoordinator.evaluate(&manifest, &mixed).await,
            Err(RuntimeAuthorizationError::RestoreRejected)
        );

        let mut stale = reader(&manifest, true);
        stale.authority.audit_continuous = false;
        assert_eq!(
            RestoreCoordinator.evaluate(&manifest, &stale).await,
            Err(RuntimeAuthorizationError::RestoreRejected)
        );

        let mut removed_key = reader(&manifest, true);
        removed_key.authority.removed_keys_absent = false;
        assert_eq!(
            RestoreCoordinator.evaluate(&manifest, &removed_key).await,
            Err(RuntimeAuthorizationError::RestoreRejected)
        );
    }

    #[test]
    fn duplicate_coordinates_and_empty_deltas_are_rejected() {
        let delta =
            RestoreDelta::new(RestoreComponent::Binding, [1; 32], [2; 32], [3; 32]).unwrap();
        assert!(RestoreManifest::new(
            CommunityId::from_uuid(Uuid::from_u128(1)),
            Uuid::from_u128(2),
            [3; 32],
            1,
            1,
            [4; 32],
            [5; 32],
            vec![delta.clone(), delta],
        )
        .is_err());
    }
}
