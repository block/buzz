//! Durable PostgreSQL implementation behind the storage-agnostic status seam.

use async_trait::async_trait;
use uuid::Uuid;

use super::{
    ClientStatusCurrentRequirement, ClientStatusIssuanceReceipt, ClientStatusRevisionScope,
    DurableClientStatusRevision, DurableClientStatusRevisionSource,
};

/// PostgreSQL-backed revision source coupled to the independent restore witness.
///
/// Construction does not enable presentation; the unconstructible presentation
/// permit remains the separate runtime gate.
pub struct PostgresClientStatusRevisionSource {
    db: buzz_db::Db,
    restore: std::sync::Arc<super::super::restore::RestoreProtectionRuntime>,
}

impl PostgresClientStatusRevisionSource {
    /// Bind the writer database and the exact initialized restore runtime.
    pub fn new(
        db: buzz_db::Db,
        restore: std::sync::Arc<super::super::restore::RestoreProtectionRuntime>,
    ) -> Self {
        Self { db, restore }
    }

    async fn reconcile_allocation(
        &self,
        scope: ClientStatusRevisionScope,
        operation_id: Uuid,
        request_fingerprint: [u8; 32],
        result: Result<
            buzz_db::client_status::AllocatedStatusRevision,
            buzz_db::client_status::ClientStatusAllocationError,
        >,
        witness: super::super::restore::RestoreMutationGuard,
    ) -> Option<DurableClientStatusRevision> {
        match result {
            Ok(revision) => {
                witness.commit().await.ok()?;
                DurableClientStatusRevision::from_durable_state(revision.revision, revision.floor)
                    .ok()
            }
            Err(buzz_db::client_status::ClientStatusAllocationError::CommitUnknown(_)) => {
                witness.commit().await.ok()?;
                let revision = self
                    .db
                    .committed_status_revision(
                        scope.authorization_domain(),
                        operation_id,
                        request_fingerprint,
                    )
                    .await
                    .ok()??;
                DurableClientStatusRevision::from_durable_state(revision.revision, revision.floor)
                    .ok()
            }
            Err(_) => {
                let _ = witness.abort().await;
                None
            }
        }
    }
}

#[async_trait]
impl DurableClientStatusRevisionSource for PostgresClientStatusRevisionSource {
    async fn current_revision_for(
        &self,
        requirement: &ClientStatusCurrentRequirement<'_>,
        issuance_fingerprint: [u8; 32],
    ) -> Option<DurableClientStatusRevision> {
        let scope = requirement.scope();
        let operation_id = super::super::executor::ProtectedOperationId::derive(
            scope.authorization_domain(),
            "client.status.current.v1",
            &issuance_fingerprint,
        )
        .ok()?
        .as_uuid();
        let witness = self
            .restore
            .begin(
                scope.authorization_domain(),
                operation_id,
                issuance_fingerprint,
            )
            .await
            .ok()?;
        let event_author_pubkey = scope.event_author_pubkey().to_bytes();
        let result = self
            .db
            .allocate_current_status_revision(buzz_db::client_status::CurrentStatusAllocation {
                community_id: scope.authorization_domain(),
                event_author_pubkey: &event_author_pubkey,
                binding_id: requirement.binding_id(),
                binding_version: requirement.binding_version().get(),
                policy_version: requirement.policy_version().as_str(),
                evaluation_generation: requirement.evaluation_generation(),
                fresh_until: requirement.fresh_until(),
                operation_id,
                request_fingerprint: issuance_fingerprint,
            })
            .await;
        self.reconcile_allocation(scope, operation_id, issuance_fingerprint, result, witness)
            .await
    }

    async fn withdrawal_revision_for(
        &self,
        receipt: &ClientStatusIssuanceReceipt,
        withdrawal_fingerprint: [u8; 32],
    ) -> Option<DurableClientStatusRevision> {
        let operation_id = super::super::executor::ProtectedOperationId::derive(
            receipt.scope.authorization_domain(),
            "client.status.withdraw.v1",
            &withdrawal_fingerprint,
        )
        .ok()?
        .as_uuid();
        let witness = self
            .restore
            .begin(
                receipt.scope.authorization_domain(),
                operation_id,
                withdrawal_fingerprint,
            )
            .await
            .ok()?;
        let event_author_pubkey = receipt.scope.event_author_pubkey().to_bytes();
        let result = self
            .db
            .allocate_withdrawn_status_revision(
                buzz_db::client_status::WithdrawalStatusAllocation {
                    community_id: receipt.scope.authorization_domain(),
                    event_author_pubkey: &event_author_pubkey,
                    supersedes_revision: receipt.revision,
                    issuance_fingerprint: receipt.issuance_fingerprint,
                    operation_id,
                    request_fingerprint: withdrawal_fingerprint,
                },
            )
            .await;
        self.reconcile_allocation(
            receipt.scope,
            operation_id,
            withdrawal_fingerprint,
            result,
            witness,
        )
        .await
    }
}
