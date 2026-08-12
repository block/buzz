//! Adapter to the canonical atomic admission contract.

use async_trait::async_trait;
use buzz_db::authorization_admission::{
    AdmissionCommitError, AdmissionCommitOutcome, AdmissionCommitRequest, AdmissionObject,
    AdmissionObjectKind, AdmissionReplayClaim, AdmissionReplayClaimKind,
    CanonicalAdmissionCommitter,
};

use super::authority::RuntimeAdmissionPreparation;
use super::{
    AdmissionCommitPort, ProtectedResourceKind, RuntimeAdmissionRequest, RuntimeAuthorizationError,
};

/// Exact relay adapter to the sole canonical admission committer.
pub struct CanonicalAdmissionAdapter<C> {
    committer: C,
}

impl<C> CanonicalAdmissionAdapter<C> {
    /// Bind one concrete canonical committer.
    pub const fn new(committer: C) -> Self {
        Self { committer }
    }
}

impl<C> std::fmt::Debug for CanonicalAdmissionAdapter<C> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CanonicalAdmissionAdapter([REDACTED])")
    }
}

#[async_trait]
impl<C> AdmissionCommitPort for CanonicalAdmissionAdapter<C>
where
    C: CanonicalAdmissionCommitter,
{
    async fn commit(
        &self,
        request: RuntimeAdmissionRequest,
    ) -> Result<buzz_auth::FinalizedAuthContext, RuntimeAuthorizationError> {
        let (_operation_id, attempt_id, route, preparation, witness) = request.into_parts();
        let object_kind = admission_object_kind(witness.resource_kind())?;
        let object = AdmissionObject::new(object_kind, *witness.resource_key())
            .ok_or(RuntimeAuthorizationError::ResourceWitnessMismatch)?;
        let mut request = match preparation {
            RuntimeAdmissionPreparation::Existing(prepared) => {
                AdmissionCommitRequest::existing(attempt_id, object, *prepared)
            }
            RuntimeAdmissionPreparation::Enrollment(enrollment) => {
                AdmissionCommitRequest::enrollment(
                    attempt_id,
                    enrollment.correlation_id,
                    object,
                    route.capability(),
                    enrollment.evidence,
                )
            }
        }
        .map_err(map_admission_error)?;

        match (
            witness.replay_claim_key(),
            witness.replay_claim_retain_until(),
        ) {
            (Some(key), Some(retain_until)) => {
                let claim = AdmissionReplayClaim::new(
                    AdmissionReplayClaimKind::TrustedProxyNonce,
                    *key,
                    retain_until,
                )
                .map_err(map_admission_error)?;
                request = request.with_replay_claim(claim);
            }
            (None, None) => {}
            _ => return Err(RuntimeAuthorizationError::ResourceWitnessMismatch),
        }

        match self
            .committer
            .commit(request)
            .await
            .map_err(map_admission_error)?
        {
            AdmissionCommitOutcome::Committed { authorization, .. } => Ok(*authorization),
            AdmissionCommitOutcome::ExactReplay { .. } => {
                Err(RuntimeAuthorizationError::ResourceWitnessMismatch)
            }
        }
    }
}

fn admission_object_kind(
    resource: ProtectedResourceKind,
) -> Result<AdmissionObjectKind, RuntimeAuthorizationError> {
    match resource {
        ProtectedResourceKind::Domain => Ok(AdmissionObjectKind::Domain),
        ProtectedResourceKind::Channel => Ok(AdmissionObjectKind::Channel),
        ProtectedResourceKind::Repository => Ok(AdmissionObjectKind::Repository),
        ProtectedResourceKind::Media => Ok(AdmissionObjectKind::Media),
        ProtectedResourceKind::ModerationTarget => Ok(AdmissionObjectKind::ModerationTarget),
        ProtectedResourceKind::AudioSession => Ok(AdmissionObjectKind::AudioSession),
        ProtectedResourceKind::Event => Ok(AdmissionObjectKind::Event),
        ProtectedResourceKind::Invitation => Ok(AdmissionObjectKind::Invitation),
        ProtectedResourceKind::BindingStatus => Ok(AdmissionObjectKind::BindingStatus),
        ProtectedResourceKind::DelegatedAgent => {
            Err(RuntimeAuthorizationError::ResourceWitnessMismatch)
        }
    }
}

const fn map_admission_error(error: AdmissionCommitError) -> RuntimeAuthorizationError {
    match error {
        AdmissionCommitError::AuditUnavailable
        | AdmissionCommitError::RecordedAuditUnavailable
        | AdmissionCommitError::DependencyUnavailable => {
            RuntimeAuthorizationError::DependencyUnavailable
        }
        AdmissionCommitError::InvalidRequest
        | AdmissionCommitError::RecordedInvalidRequest
        | AdmissionCommitError::AuthorizationDenied
        | AdmissionCommitError::RecordedAuthorizationDenied
        | AdmissionCommitError::IntentConflict
        | AdmissionCommitError::RecordedIntentConflict
        | AdmissionCommitError::ReplayRejected
        | AdmissionCommitError::RecordedReplayRejected => {
            RuntimeAuthorizationError::ResourceWitnessMismatch
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_object_mapping_rejects_unsealed_resource_namespaces() {
        for supported in [
            ProtectedResourceKind::Domain,
            ProtectedResourceKind::Channel,
            ProtectedResourceKind::Repository,
            ProtectedResourceKind::Media,
            ProtectedResourceKind::ModerationTarget,
            ProtectedResourceKind::AudioSession,
            ProtectedResourceKind::Event,
            ProtectedResourceKind::Invitation,
            ProtectedResourceKind::BindingStatus,
        ] {
            assert!(admission_object_kind(supported).is_ok());
        }
        assert_eq!(
            admission_object_kind(ProtectedResourceKind::DelegatedAgent),
            Err(RuntimeAuthorizationError::ResourceWitnessMismatch)
        );
    }

    #[test]
    fn admission_errors_map_dependency_failures_separately_from_denials() {
        for dependency in [
            AdmissionCommitError::AuditUnavailable,
            AdmissionCommitError::RecordedAuditUnavailable,
            AdmissionCommitError::DependencyUnavailable,
        ] {
            assert_eq!(
                map_admission_error(dependency),
                RuntimeAuthorizationError::DependencyUnavailable
            );
        }
        for denial in [
            AdmissionCommitError::InvalidRequest,
            AdmissionCommitError::RecordedInvalidRequest,
            AdmissionCommitError::AuthorizationDenied,
            AdmissionCommitError::RecordedAuthorizationDenied,
            AdmissionCommitError::IntentConflict,
            AdmissionCommitError::RecordedIntentConflict,
            AdmissionCommitError::ReplayRejected,
            AdmissionCommitError::RecordedReplayRejected,
        ] {
            assert_eq!(
                map_admission_error(denial),
                RuntimeAuthorizationError::ResourceWitnessMismatch
            );
        }
    }
}
