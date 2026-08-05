use async_trait::async_trait;
use buzz_audit::authorization::{
    ActorReference, AttemptId, AuthorizationEventV1, CapacityClass, ControlCode, CorrelationId,
    DecisionReason, EventId, EventKind, EventPayloadV1, EventResult, EvidenceHealthSignal,
    OperationClass, SourceClass, StreamId, TransportClass, VersionVectorV1,
};
use buzz_core::CommunityId;
use buzz_db::authorization_evidence::AcceptedEvidence;
use buzz_relay::authorization_runtime::evidence::{
    accept_authorization_decision, AcceptedAuthorizationDecision, DecisionDisposition,
    DecisionEvidenceError, DecisionEvidenceSink,
};
use chrono::Utc;
use uuid::Uuid;

struct AlwaysUnavailable;

#[async_trait]
impl DecisionEvidenceSink for AlwaysUnavailable {
    async fn accept(
        &self,
        _event: &AuthorizationEventV1,
        _capacity: CapacityClass,
    ) -> Result<AcceptedEvidence, DecisionEvidenceError> {
        Err(DecisionEvidenceError::AcceptanceUnavailable)
    }
}

struct AlwaysAccept;

#[async_trait]
impl DecisionEvidenceSink for AlwaysAccept {
    async fn accept(
        &self,
        event: &AuthorizationEventV1,
        _capacity: CapacityClass,
    ) -> Result<AcceptedEvidence, DecisionEvidenceError> {
        Ok(AcceptedEvidence {
            event_id: event.event_id(),
            stream_id: StreamId::generate(),
            stream_position: 1,
            content_digest: [7; 32],
            chain_digest: [9; 32],
        })
    }
}

fn event(result: EventResult, kind: EventKind) -> AuthorizationEventV1 {
    AuthorizationEventV1::new(
        EventId::generate(),
        CommunityId::from_uuid(Uuid::new_v4()),
        Utc::now(),
        None,
        CorrelationId::generate(),
        AttemptId::generate(),
        None,
        ActorReference::Unresolved,
        TransportClass::Internal,
        OperationClass::Read,
        SourceClass::Policy,
        kind,
        result,
        DecisionReason::PolicyDenied,
        VersionVectorV1::default(),
        EventPayloadV1::None,
    )
}

#[tokio::test]
async fn new_allow_never_releases_when_durable_acceptance_fails() {
    let health = EvidenceHealthSignal::default();
    let result = accept_authorization_decision(
        &AlwaysUnavailable,
        &health,
        &event(EventResult::Allowed, EventKind::AdmissionAllowed),
        DecisionDisposition::Allow,
        "protected-value",
    )
    .await;

    assert_eq!(
        result.expect_err("new allow must fail closed").code(),
        "authorization_evidence_acceptance_unavailable"
    );
    assert_eq!(health.count(ControlCode::AcceptanceUnavailable), 0);
}

#[tokio::test]
async fn deny_remains_denied_and_emits_one_independent_health_signal() {
    let health = EvidenceHealthSignal::default();
    let result = accept_authorization_decision(
        &AlwaysUnavailable,
        &health,
        &event(EventResult::Denied, EventKind::AdmissionDenied),
        DecisionDisposition::Deny,
        "denied-value",
    )
    .await
    .expect("evidence degradation must not turn a denial into an error or allow");

    match result {
        AcceptedAuthorizationDecision::Deny { value, evidence } => {
            assert_eq!(value, "denied-value");
            assert!(evidence.is_none());
        }
        AcceptedAuthorizationDecision::Allow { .. } => panic!("denial became an allow"),
    }
    assert_eq!(health.count(ControlCode::AcceptanceUnavailable), 1);
}

#[tokio::test]
async fn successful_acceptance_preserves_disposition_and_receipt_identity() {
    let health = EvidenceHealthSignal::default();
    let evidence_event = event(EventResult::Allowed, EventKind::AdmissionAllowed);
    let result = accept_authorization_decision(
        &AlwaysAccept,
        &health,
        &evidence_event,
        DecisionDisposition::Allow,
        "protected-value",
    )
    .await
    .expect("synthetic durable acceptance succeeds");

    match result {
        AcceptedAuthorizationDecision::Allow { value, evidence } => {
            assert_eq!(value, "protected-value");
            assert_eq!(evidence.event_id, evidence_event.event_id());
        }
        AcceptedAuthorizationDecision::Deny { .. } => panic!("allow became a denial"),
    }
    assert_eq!(health.count(ControlCode::AcceptanceUnavailable), 0);
}
