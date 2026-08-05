//! PostgreSQL executor for the explicitly composed operator lifecycle runtime.
//!
//! This module does not register routes or construct an authenticator. A caller
//! must explicitly provide independent authentication and the two dedicated
//! pseudonymization keys before it can build [`crate::operator_runtime::OperatorRuntime`].

use async_trait::async_trait;
use buzz_audit::authorization::{
    DecisionReason, Pseudonymizer, PseudonymousReference, ReferenceKind,
};
use buzz_core::CommunityId;
use buzz_db::operator_lifecycle::{
    OperatorAuthorityEvidence, OperatorBindingState, OperatorLifecycleAction,
    OperatorLifecycleCommand, OperatorLifecycleDenialAttempt, OperatorLifecycleFailure,
    OperatorLifecycleResult, OperatorLifecycleStatus, OperatorReferenceKey,
    VerifiedOperatorReplacement,
};
use buzz_db::Db;
use chrono::{DateTime, Utc};

use crate::operator_runtime::{
    AuthenticatedOperatorDenial, AuthorizedOperatorOperation, DurableOperatorExecutor,
    OpaqueOperatorReference, OperatorAction, OperatorIntent, OperatorOutcome,
    OperatorOutcomeStatus, OperatorRecord, OperatorRecordState, OperatorRuntimeError,
};

/// Durable PostgreSQL implementation of the disabled lifecycle executor seam.
pub struct PostgresOperatorExecutor {
    db: Db,
    reference_key: OperatorReferenceKey,
    pseudonymizer: Pseudonymizer,
}

impl PostgresOperatorExecutor {
    /// Bind the database to dedicated operator-reference and audit pseudonym keys.
    pub const fn new(
        db: Db,
        reference_key: OperatorReferenceKey,
        pseudonymizer: Pseudonymizer,
    ) -> Self {
        Self {
            db,
            reference_key,
            pseudonymizer,
        }
    }

    fn pseudonymize(
        &self,
        domain: CommunityId,
        kind: ReferenceKind,
        reference: OpaqueOperatorReference,
    ) -> Result<PseudonymousReference, OperatorRuntimeError> {
        self.pseudonymizer
            .derive(domain, kind, &reference.digest())
            .map_err(|_| OperatorRuntimeError::InvalidAuthority)
    }

    fn authority(
        &self,
        operation: &AuthorizedOperatorOperation,
        domain: CommunityId,
    ) -> Result<OperatorAuthorityEvidence, OperatorRuntimeError> {
        let expires_at_seconds = i64::try_from(operation.expires_at_unix_seconds())
            .map_err(|_| OperatorRuntimeError::InvalidAuthority)?;
        let expires_at = DateTime::<Utc>::from_timestamp(expires_at_seconds, 0)
            .ok_or(OperatorRuntimeError::InvalidAuthority)?;
        let actor = self.pseudonymize(domain, ReferenceKind::Actor, operation.actor_reference())?;
        let approvers = operation
            .invocation()
            .context()
            .approval_references()
            .iter()
            .copied()
            .map(|reference| self.pseudonymize(domain, ReferenceKind::Approver, reference))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OperatorAuthorityEvidence {
            evidence_id: operation.authority_evidence_id(),
            actor,
            actor_independence_reference: operation.actor_reference().digest(),
            provenance_reference: operation.provenance_reference().digest(),
            approvers,
            approver_independence_references: operation
                .invocation()
                .context()
                .approval_references()
                .iter()
                .map(|reference| reference.digest())
                .collect(),
            approval_ids: operation.approval_evidence_ids().to_vec(),
            expires_at,
        })
    }

    fn command(
        &self,
        operation: &AuthorizedOperatorOperation,
    ) -> Result<OperatorLifecycleCommand, OperatorRuntimeError> {
        let invocation = operation.invocation();
        let context = invocation.context();
        let domain = CommunityId::from_uuid(context.domain_id());
        let (action, target, replacement_reference, list_limit, list_after) =
            match invocation.intent() {
                OperatorIntent::List { limit, after } => (
                    OperatorLifecycleAction::List,
                    None,
                    None,
                    *limit,
                    after.map(OpaqueOperatorReference::digest),
                ),
                OperatorIntent::Preview {
                    target,
                    replacement,
                } => (
                    OperatorLifecycleAction::Preview,
                    Some(target.digest()),
                    Some(replacement.digest()),
                    1,
                    None,
                ),
                OperatorIntent::Revoke { target } => (
                    OperatorLifecycleAction::Revoke,
                    Some(target.digest()),
                    None,
                    1,
                    None,
                ),
                OperatorIntent::Rotate {
                    target,
                    replacement,
                } => (
                    OperatorLifecycleAction::Rotate,
                    Some(target.digest()),
                    Some(replacement.digest()),
                    1,
                    None,
                ),
            };
        let target_pseudonym = target
            .map(OpaqueOperatorReference::from_digest)
            .map(|reference| self.pseudonymize(domain, ReferenceKind::Binding, reference))
            .transpose()?;
        let replacement = operation
            .replacement()
            .map(|value| {
                VerifiedOperatorReplacement::new(
                    value.reference().digest(),
                    value.public_key(),
                    value.policy_digest(),
                )
                .map_err(|_| OperatorRuntimeError::InvalidAuthority)
            })
            .transpose()?;
        Ok(OperatorLifecycleCommand {
            domain,
            operation_id: context.operation_id(),
            correlation_id: context.correlation_id(),
            semantic_fingerprint: invocation.fingerprint(),
            expected_revision: context.expected_revision(),
            action,
            reason_code: context.reason().discriminant(),
            target_reference: target,
            target_pseudonym,
            replacement_reference,
            replacement,
            list_limit,
            list_after,
            authority: self.authority(operation, domain)?,
        })
    }

    fn denial_attempt(
        &self,
        denial: &AuthenticatedOperatorDenial,
    ) -> Result<OperatorLifecycleDenialAttempt, OperatorRuntimeError> {
        let invocation = denial.invocation();
        let context = invocation.context();
        let domain = CommunityId::from_uuid(context.domain_id());
        let actor = self.pseudonymize(domain, ReferenceKind::Actor, denial.actor_reference())?;
        let approvers = context
            .approval_references()
            .iter()
            .copied()
            .map(|reference| self.pseudonymize(domain, ReferenceKind::Approver, reference))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OperatorLifecycleDenialAttempt {
            domain,
            operation_id: context.operation_id(),
            correlation_id: context.correlation_id(),
            semantic_fingerprint: invocation.fingerprint(),
            expected_revision: context.expected_revision(),
            action: lifecycle_action(invocation.intent().action()),
            reason_code: context.reason().discriminant(),
            actor,
            provenance_reference: denial.provenance_reference().digest(),
            approvers,
            denial_reason: denial_reason(denial.reason()),
        })
    }
}

#[async_trait]
impl DurableOperatorExecutor for PostgresOperatorExecutor {
    async fn execute_idempotent(
        &self,
        operation: AuthorizedOperatorOperation,
    ) -> Result<OperatorOutcome, OperatorRuntimeError> {
        let command = self.command(&operation)?;
        self.db
            .execute_operator_lifecycle(&self.reference_key, &command)
            .await
            .map_err(map_lifecycle_failure)
            .and_then(map_lifecycle_result)
    }

    async fn record_denial(
        &self,
        denial: AuthenticatedOperatorDenial,
    ) -> Result<(), OperatorRuntimeError> {
        let attempt = self.denial_attempt(&denial)?;
        self.db
            .record_operator_lifecycle_denial(&attempt)
            .await
            .map_err(map_lifecycle_failure)
    }
}

const fn lifecycle_action(action: OperatorAction) -> OperatorLifecycleAction {
    match action {
        OperatorAction::List => OperatorLifecycleAction::List,
        OperatorAction::Preview => OperatorLifecycleAction::Preview,
        OperatorAction::Revoke => OperatorLifecycleAction::Revoke,
        OperatorAction::Rotate => OperatorLifecycleAction::Rotate,
    }
}

const fn denial_reason(error: OperatorRuntimeError) -> DecisionReason {
    match error {
        OperatorRuntimeError::CrossDomain => DecisionReason::CrossDomain,
        OperatorRuntimeError::StaleAuthority => DecisionReason::StaleApproval,
        OperatorRuntimeError::MissingCapability => DecisionReason::UnauthorizedActor,
        OperatorRuntimeError::MissingApproval => DecisionReason::MissingApproval,
        OperatorRuntimeError::SelfApproval => DecisionReason::SelfApproval,
        OperatorRuntimeError::ReplayedAuthority => DecisionReason::EvidenceReplayed,
        OperatorRuntimeError::IdempotencyConflict => DecisionReason::IntentConflict,
        OperatorRuntimeError::StorageUnavailable => DecisionReason::StorageUnavailable,
        OperatorRuntimeError::MissingCredential
        | OperatorRuntimeError::InvalidCredential
        | OperatorRuntimeError::InvalidRequest
        | OperatorRuntimeError::Unauthenticated
        | OperatorRuntimeError::InvalidAuthority
        | OperatorRuntimeError::ExecutorContract => DecisionReason::EvidenceInvalid,
    }
}

fn map_lifecycle_failure(failure: OperatorLifecycleFailure) -> OperatorRuntimeError {
    match failure {
        OperatorLifecycleFailure::Storage(_) => OperatorRuntimeError::StorageUnavailable,
        OperatorLifecycleFailure::Denied(reason) => match reason {
            DecisionReason::IntentConflict => OperatorRuntimeError::IdempotencyConflict,
            DecisionReason::EvidenceReplayed | DecisionReason::ReplayedApproval => {
                OperatorRuntimeError::ReplayedAuthority
            }
            DecisionReason::StaleApproval => OperatorRuntimeError::StaleAuthority,
            DecisionReason::MissingApproval => OperatorRuntimeError::MissingApproval,
            DecisionReason::SelfApproval | DecisionReason::ApprovalNotIndependent => {
                OperatorRuntimeError::SelfApproval
            }
            DecisionReason::CrossDomain | DecisionReason::DomainMismatch => {
                OperatorRuntimeError::CrossDomain
            }
            DecisionReason::UnauthorizedActor => OperatorRuntimeError::MissingCapability,
            _ => OperatorRuntimeError::InvalidAuthority,
        },
    }
}

fn map_lifecycle_result(
    result: OperatorLifecycleResult,
) -> Result<OperatorOutcome, OperatorRuntimeError> {
    let action = match result.action {
        OperatorLifecycleAction::List => OperatorAction::List,
        OperatorLifecycleAction::Preview => OperatorAction::Preview,
        OperatorLifecycleAction::Revoke => OperatorAction::Revoke,
        OperatorLifecycleAction::Rotate => OperatorAction::Rotate,
    };
    let status = match result.status {
        OperatorLifecycleStatus::Listed => OperatorOutcomeStatus::Listed,
        OperatorLifecycleStatus::Previewed => OperatorOutcomeStatus::Previewed,
        OperatorLifecycleStatus::Revoked => OperatorOutcomeStatus::Revoked,
        OperatorLifecycleStatus::Rotated => OperatorOutcomeStatus::Rotated,
        OperatorLifecycleStatus::Denied => return Err(OperatorRuntimeError::ExecutorContract),
    };
    let records = result
        .records
        .into_iter()
        .map(|record| OperatorRecord {
            reference: OpaqueOperatorReference::from_digest(record.reference),
            state: match record.state {
                OperatorBindingState::Active => OperatorRecordState::Active,
                OperatorBindingState::Revoked => OperatorRecordState::Revoked,
                OperatorBindingState::Rotated => OperatorRecordState::Rotated,
                OperatorBindingState::Archived => OperatorRecordState::Archived,
            },
            revision: record.revision,
        })
        .collect();
    OperatorOutcome::new(
        result.operation_id,
        result.correlation_id,
        action,
        status,
        result.affected_count,
        result.lifecycle_revision,
        records,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_denials_have_closed_database_reasons() {
        assert_eq!(
            denial_reason(OperatorRuntimeError::MissingCapability),
            DecisionReason::UnauthorizedActor
        );
        assert_eq!(
            denial_reason(OperatorRuntimeError::SelfApproval),
            DecisionReason::SelfApproval
        );
    }
}
