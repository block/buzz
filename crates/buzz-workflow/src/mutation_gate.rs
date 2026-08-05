//! Provider-neutral admission gate for workflow mutations and external effects.

use buzz_core::tenant::CommunityId;

use crate::WorkflowError;

/// Server-owned gate evaluated before every workflow mutation or external effect.
///
/// The workflow engine deliberately knows nothing about identity providers,
/// leases, or deployment configuration. A relay can install a gate that denies
/// an authorization domain until it has a transaction-owning executor. When no
/// gate is installed, the standalone engine preserves its legacy behavior.
pub trait MutationGate: Send + Sync {
    /// Require current authority for one server-resolved authorization domain.
    fn require_mutation(&self, community_id: CommunityId) -> Result<(), WorkflowError>;

    /// Require authority for an outbound network effect.
    fn require_outbound_webhook(&self, community_id: CommunityId) -> Result<(), WorkflowError> {
        self.require_mutation(community_id)
    }
}

pub(crate) fn require_configured_mutation(
    gate: Option<&dyn MutationGate>,
    community_id: CommunityId,
) -> Result<(), WorkflowError> {
    match gate {
        Some(gate) => gate.require_mutation(community_id),
        None => Ok(()),
    }
}

pub(crate) fn require_configured_outbound_webhook(
    gate: Option<&dyn MutationGate>,
    community_id: CommunityId,
) -> Result<(), WorkflowError> {
    match gate {
        Some(gate) => gate.require_outbound_webhook(community_id),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct DenyGate(AtomicUsize);

    impl MutationGate for DenyGate {
        fn require_mutation(&self, _community_id: CommunityId) -> Result<(), WorkflowError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(WorkflowError::Unauthorized(
                "synthetic mutation denial".into(),
            ))
        }
    }

    #[test]
    fn absent_gate_preserves_legacy_and_configured_denial_is_authoritative() {
        let community_id = CommunityId::from_uuid(uuid::Uuid::from_u128(1));
        assert!(require_configured_mutation(None, community_id).is_ok());

        let gate = DenyGate(AtomicUsize::new(0));
        assert!(matches!(
            require_configured_mutation(Some(&gate), community_id),
            Err(WorkflowError::Unauthorized(_))
        ));
        assert_eq!(gate.0.load(Ordering::SeqCst), 1);
        assert!(matches!(
            require_configured_outbound_webhook(Some(&gate), community_id),
            Err(WorkflowError::Unauthorized(_))
        ));
        assert_eq!(gate.0.load(Ordering::SeqCst), 2);
    }
}
