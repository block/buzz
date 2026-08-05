//! Focused scaffold for the disabled-by-default O5 operator composition path.

use std::{
    collections::HashMap,
    io::Write,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use buzz_relay::{
    api::operator::lifecycle_router,
    operator_runtime::{
        AuthenticatedOperatorDenial, AuthorizedOperatorOperation, DurableOperatorExecutor,
        GrantedOperatorCapability, GrantedOperatorReplacement, OpaqueOperatorReference,
        OperatorAction, OperatorAuthenticator, OperatorAuthorizationRequest, OperatorCapability,
        OperatorClock, OperatorCredential, OperatorOutcome, OperatorOutcomeStatus, OperatorRecord,
        OperatorRecordState, OperatorRuntime, OperatorRuntimeError,
    },
};
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const CREDENTIAL_CANARY: &str = "Synthetic operator credential must never escape";
const PRIVATE_CLAIM_CANARY: &str = "Synthetic private claim must never escape";

struct FixedClock;

impl OperatorClock for FixedClock {
    fn now_unix_seconds(&self) -> Result<u64, OperatorRuntimeError> {
        Ok(100)
    }
}

struct TestGrant {
    domain_id: Uuid,
    operation_id: Uuid,
    intent_fingerprint: [u8; 32],
    authority_evidence_id: Uuid,
    approval_evidence_ids: Vec<Uuid>,
    replacement: Option<GrantedOperatorReplacement>,
    allow: bool,
    actor_reference: OpaqueOperatorReference,
    expires_at: u64,
}

impl GrantedOperatorCapability for TestGrant {
    fn authority_evidence_id(&self) -> Uuid {
        self.authority_evidence_id
    }

    fn domain_id(&self) -> Uuid {
        self.domain_id
    }

    fn operation_id(&self) -> Uuid {
        self.operation_id
    }

    fn intent_fingerprint(&self) -> [u8; 32] {
        self.intent_fingerprint
    }

    fn actor_reference(&self) -> OpaqueOperatorReference {
        self.actor_reference
    }

    fn provenance_reference(&self) -> OpaqueOperatorReference {
        OpaqueOperatorReference::from_digest([2; 32])
    }

    fn approval_evidence_ids(&self) -> &[Uuid] {
        &self.approval_evidence_ids
    }

    fn replacement(&self) -> Option<GrantedOperatorReplacement> {
        self.replacement
    }

    fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at
    }

    fn permits(&self, _capability: OperatorCapability) -> bool {
        self.allow
    }
}

struct TestAuthenticator {
    allow: bool,
    calls: Mutex<Vec<OperatorAuthorizationRequest>>,
    domain_override: Option<Uuid>,
    actor_reference: OpaqueOperatorReference,
    expires_at: u64,
    approval_count: usize,
}

#[async_trait]
impl OperatorAuthenticator for TestAuthenticator {
    async fn authenticate(
        &self,
        credential: &OperatorCredential,
        request: OperatorAuthorizationRequest,
    ) -> Result<Box<dyn GrantedOperatorCapability>, OperatorRuntimeError> {
        assert_eq!(
            credential.expose_to_authenticator(),
            CREDENTIAL_CANARY.as_bytes()
        );
        assert_ne!(request.intent_fingerprint(), [0; 32]);
        self.calls.lock().expect("auth calls").push(request);
        let replacement = request
            .replacement_reference()
            .map(|reference| GrantedOperatorReplacement::new(reference, [5; 32], [6; 32]))
            .transpose()?;
        Ok(Box::new(TestGrant {
            domain_id: self.domain_override.unwrap_or_else(|| request.domain_id()),
            operation_id: request.operation_id(),
            intent_fingerprint: request.intent_fingerprint(),
            authority_evidence_id: Uuid::new_v4(),
            approval_evidence_ids: (0..self.approval_count).map(|_| Uuid::new_v4()).collect(),
            replacement,
            allow: self.allow,
            actor_reference: self.actor_reference,
            expires_at: self.expires_at,
        }))
    }
}

type ReceiptKey = (Uuid, Uuid);
type ReceiptValue = ([u8; 32], OperatorOutcome);

#[derive(Default)]
struct TestExecutor {
    receipts: Mutex<HashMap<ReceiptKey, ReceiptValue>>,
    committed_actions: Mutex<Vec<OperatorAction>>,
    denials: Mutex<Vec<OperatorRuntimeError>>,
    fail_denial_recording: bool,
}

#[async_trait]
impl DurableOperatorExecutor for TestExecutor {
    async fn execute_idempotent(
        &self,
        operation: AuthorizedOperatorOperation,
    ) -> Result<OperatorOutcome, OperatorRuntimeError> {
        let invocation = operation.invocation();
        let context = invocation.context();
        let action = invocation.intent().action();
        let fingerprint = invocation.fingerprint();
        let receipt_key = (context.domain_id(), context.operation_id());
        let mut receipts = self.receipts.lock().expect("receipts");
        if let Some((existing_fingerprint, outcome)) = receipts.get(&receipt_key) {
            return if *existing_fingerprint == fingerprint {
                Ok(outcome.clone())
            } else {
                Err(OperatorRuntimeError::IdempotencyConflict)
            };
        }

        assert_ne!(operation.actor_reference().digest(), [0; 32]);
        assert_ne!(operation.provenance_reference().digest(), [0; 32]);
        let (status, affected_count, records) = match action {
            OperatorAction::List => (
                OperatorOutcomeStatus::Listed,
                1,
                vec![OperatorRecord {
                    reference: OpaqueOperatorReference::from_digest([7; 32]),
                    state: OperatorRecordState::Active,
                    revision: context.expected_revision(),
                }],
            ),
            OperatorAction::Preview => (OperatorOutcomeStatus::Previewed, 1, Vec::new()),
            OperatorAction::Revoke => (OperatorOutcomeStatus::Revoked, 1, Vec::new()),
            OperatorAction::Rotate => (OperatorOutcomeStatus::Rotated, 1, Vec::new()),
        };
        let outcome = OperatorOutcome::new(
            context.operation_id(),
            context.correlation_id(),
            action,
            status,
            affected_count,
            context.expected_revision() + 1,
            records,
        )?;
        receipts.insert(receipt_key, (fingerprint, outcome.clone()));
        self.committed_actions
            .lock()
            .expect("committed actions")
            .push(action);
        Ok(outcome)
    }

    async fn record_denial(
        &self,
        denial: AuthenticatedOperatorDenial,
    ) -> Result<(), OperatorRuntimeError> {
        self.denials.lock().expect("denials").push(denial.reason());
        if self.fail_denial_recording {
            return Err(OperatorRuntimeError::StorageUnavailable);
        }
        Ok(())
    }
}

fn runtime() -> (
    Arc<OperatorRuntime>,
    Arc<TestAuthenticator>,
    Arc<TestExecutor>,
) {
    runtime_with_capability(true)
}

fn runtime_with_capability(
    allow: bool,
) -> (
    Arc<OperatorRuntime>,
    Arc<TestAuthenticator>,
    Arc<TestExecutor>,
) {
    runtime_with_grant(allow, None, [1; 32], 200, 1)
}

fn runtime_with_grant(
    allow: bool,
    domain_override: Option<Uuid>,
    actor_reference: [u8; 32],
    expires_at: u64,
    approval_count: usize,
) -> (
    Arc<OperatorRuntime>,
    Arc<TestAuthenticator>,
    Arc<TestExecutor>,
) {
    let authenticator = Arc::new(TestAuthenticator {
        allow,
        calls: Mutex::new(Vec::new()),
        domain_override,
        actor_reference: OpaqueOperatorReference::from_digest(actor_reference),
        expires_at,
        approval_count,
    });
    let executor = Arc::new(TestExecutor::default());
    let runtime = Arc::new(OperatorRuntime::new(
        authenticator.clone(),
        executor.clone(),
        Arc::new(FixedClock),
    ));
    (runtime, authenticator, executor)
}

fn reference(byte: u8) -> String {
    hex::encode([byte; 32])
}

fn assert_no_committed_actions(executor: &TestExecutor) {
    assert!(
        executor
            .committed_actions
            .lock()
            .expect("committed actions")
            .is_empty(),
        "denied operator request must not mutate"
    );
}

fn request_body(domain_id: Uuid, operation_id: Uuid, correlation_id: Uuid) -> Value {
    json!({
        "domain_id": domain_id,
        "operation_id": operation_id,
        "correlation_id": correlation_id,
        "reason": "planned_rotation",
        "expected_revision": 7,
        "approval_references": [reference(9)],
        "private_claim_canary": PRIVATE_CLAIM_CANARY,
    })
}

async fn post(runtime: Arc<OperatorRuntime>, path: &str, body: Value) -> (StatusCode, String) {
    let response = lifecycle_router(runtime)
        .oneshot(
            Request::post(path)
                .header(header::AUTHORIZATION, CREDENTIAL_CANARY)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("operator request"),
        )
        .await
        .expect("operator response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("response body");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("UTF-8 response"),
    )
}

#[tokio::test]
async fn real_composition_path_reaches_list_preview_revoke_and_rotate() {
    let (runtime, authenticator, executor) = runtime();
    let domain_id = Uuid::from_u128(0x501);
    let mut results = Vec::new();

    let mut list = request_body(domain_id, Uuid::from_u128(0x510), Uuid::from_u128(0x610));
    list["limit"] = json!(25);
    results.push(post(runtime.clone(), "/operator/v1/lifecycle/list", list.clone()).await);

    let mut preview = request_body(domain_id, Uuid::from_u128(0x511), Uuid::from_u128(0x611));
    preview["target"] = json!(reference(3));
    preview["replacement"] = json!(reference(4));
    results.push(post(runtime.clone(), "/operator/v1/lifecycle/preview", preview).await);

    let mut revoke = request_body(domain_id, Uuid::from_u128(0x512), Uuid::from_u128(0x612));
    revoke["target"] = json!(reference(3));
    results.push(post(runtime.clone(), "/operator/v1/lifecycle/revoke", revoke).await);

    let mut rotate = request_body(domain_id, Uuid::from_u128(0x513), Uuid::from_u128(0x613));
    rotate["target"] = json!(reference(3));
    rotate["replacement"] = json!(reference(4));
    results.push(post(runtime.clone(), "/operator/v1/lifecycle/rotate", rotate).await);

    // The same semantic operation returns the original result.
    results.push(post(runtime, "/operator/v1/lifecycle/list", list).await);

    for (status, body) in &results {
        assert_eq!(*status, StatusCode::OK, "unexpected body: {body}");
        assert!(!body.contains(CREDENTIAL_CANARY));
        assert!(!body.contains(PRIVATE_CLAIM_CANARY));
    }
    assert_eq!(authenticator.calls.lock().expect("auth calls").len(), 5);

    let actions = executor
        .committed_actions
        .lock()
        .expect("committed actions")
        .clone();
    assert_eq!(actions.len(), 4, "idempotent replay must not re-execute");
    for expected in [
        OperatorAction::List,
        OperatorAction::Preview,
        OperatorAction::Revoke,
        OperatorAction::Rotate,
    ] {
        assert!(actions.contains(&expected), "missing {expected:?}");
    }
}

#[tokio::test]
async fn conflicting_operation_replay_is_denied_without_second_execution() {
    let (runtime, _authenticator, executor) = runtime();
    let domain_id = Uuid::from_u128(0x520);
    let operation_id = Uuid::from_u128(0x521);
    let correlation_id = Uuid::from_u128(0x522);
    let mut first = request_body(domain_id, operation_id, correlation_id);
    first["target"] = json!(reference(3));
    assert_eq!(
        post(runtime.clone(), "/operator/v1/lifecycle/revoke", first,)
            .await
            .0,
        StatusCode::OK
    );

    let mut conflicting = request_body(domain_id, operation_id, correlation_id);
    conflicting["target"] = json!(reference(5));
    let (status, body) = post(runtime, "/operator/v1/lifecycle/revoke", conflicting).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body.contains("operator_idempotency_conflict"));
    assert_eq!(
        executor
            .committed_actions
            .lock()
            .expect("committed actions")
            .len(),
        1
    );
}

#[tokio::test]
async fn missing_credential_and_missing_capability_never_reach_executor() {
    let (runtime, _authenticator, executor) = runtime_with_capability(false);
    let domain_id = Uuid::from_u128(0x530);
    let mut body = request_body(domain_id, Uuid::from_u128(0x531), Uuid::from_u128(0x532));
    body["target"] = json!(reference(3));

    let missing = lifecycle_router(runtime.clone())
        .oneshot(
            Request::post("/operator/v1/lifecycle/revoke")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("missing-credential request"),
        )
        .await
        .expect("missing-credential response");
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

    let (status, response) = post(runtime, "/operator/v1/lifecycle/revoke", body).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(response.contains("operator_capability_missing"));
    assert!(executor
        .committed_actions
        .lock()
        .expect("committed actions")
        .is_empty());
    assert_eq!(
        executor.denials.lock().expect("denials").as_slice(),
        &[OperatorRuntimeError::MissingCapability]
    );
}

#[tokio::test]
async fn malformed_and_authenticated_adversarial_requests_never_mutate() {
    let domain_id = Uuid::from_u128(0x540);

    let (runtime, authenticator, executor) = runtime();
    let mut missing_reason =
        request_body(domain_id, Uuid::from_u128(0x541), Uuid::from_u128(0x542));
    missing_reason
        .as_object_mut()
        .expect("request object")
        .remove("reason");
    missing_reason["target"] = json!(reference(3));
    assert_eq!(
        post(runtime, "/operator/v1/lifecycle/revoke", missing_reason)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(authenticator.calls.lock().expect("auth calls").is_empty());
    assert!(executor
        .committed_actions
        .lock()
        .expect("committed actions")
        .is_empty());

    let (runtime, _, executor) =
        runtime_with_grant(true, Some(Uuid::from_u128(0x54f)), [1; 32], 200, 1);
    let mut cross_domain = request_body(domain_id, Uuid::from_u128(0x543), Uuid::from_u128(0x544));
    cross_domain["target"] = json!(reference(3));
    assert_eq!(
        post(runtime, "/operator/v1/lifecycle/revoke", cross_domain)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        executor.denials.lock().expect("denials").as_slice(),
        &[OperatorRuntimeError::CrossDomain]
    );
    assert_no_committed_actions(&executor);

    let (runtime, _, executor) = runtime_with_grant(true, None, [1; 32], 100, 1);
    let mut stale = request_body(domain_id, Uuid::from_u128(0x545), Uuid::from_u128(0x546));
    stale["target"] = json!(reference(3));
    assert_eq!(
        post(runtime, "/operator/v1/lifecycle/revoke", stale)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        executor.denials.lock().expect("denials").as_slice(),
        &[OperatorRuntimeError::StaleAuthority]
    );
    assert_no_committed_actions(&executor);

    let (runtime, _, executor) = runtime_with_grant(true, None, [9; 32], 200, 1);
    let mut self_approved = request_body(domain_id, Uuid::from_u128(0x547), Uuid::from_u128(0x548));
    self_approved["target"] = json!(reference(3));
    assert_eq!(
        post(runtime, "/operator/v1/lifecycle/revoke", self_approved)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        executor.denials.lock().expect("denials").as_slice(),
        &[OperatorRuntimeError::SelfApproval]
    );
    assert_no_committed_actions(&executor);

    let (runtime, _, executor) = runtime_with_grant(true, None, [1; 32], 200, 0);
    let mut missing_approval =
        request_body(domain_id, Uuid::from_u128(0x549), Uuid::from_u128(0x54a));
    missing_approval["target"] = json!(reference(3));
    missing_approval["approval_references"] = json!([]);
    let (status, response) = post(runtime, "/operator/v1/lifecycle/revoke", missing_approval).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(response.contains("operator_approval_missing"));
    assert_eq!(
        executor.denials.lock().expect("denials").as_slice(),
        &[OperatorRuntimeError::MissingApproval]
    );
    assert_no_committed_actions(&executor);
}

#[test]
fn stock_router_does_not_register_lifecycle_surface() {
    let stock_router = include_str!("../src/router.rs");
    assert!(!stock_router.contains("lifecycle_router"));
    assert!(!stock_router.contains("/operator/v1/lifecycle"));
}

#[derive(Clone)]
struct CapturingMakeWriter {
    buffer: Arc<Mutex<Vec<u8>>>,
}

struct CapturingWriter {
    buffer: Arc<Mutex<Vec<u8>>>,
}

impl Write for CapturingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.buffer.lock().expect("trace buffer").extend(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturingMakeWriter {
    type Writer = CapturingWriter;

    fn make_writer(&'writer self) -> Self::Writer {
        CapturingWriter {
            buffer: Arc::clone(&self.buffer),
        }
    }
}

#[test]
fn planted_canaries_never_cross_response_logs_or_metrics() {
    const RAW_ISSUER_CANARY: &str = "issuer-canary.invalid/private";
    const JWT_CANARY: &str = "eyJ.synthetic.jwt.canary";
    const JWKS_CANARY: &str = "{\"keys\":[{\"kid\":\"private-jwks-canary\"}]}";
    let authenticator = Arc::new(TestAuthenticator {
        allow: false,
        calls: Mutex::new(Vec::new()),
        domain_override: None,
        actor_reference: OpaqueOperatorReference::from_digest([1; 32]),
        expires_at: 200,
        approval_count: 1,
    });
    let executor = Arc::new(TestExecutor {
        fail_denial_recording: true,
        ..TestExecutor::default()
    });
    let runtime = Arc::new(OperatorRuntime::new(
        authenticator,
        executor,
        Arc::new(FixedClock),
    ));
    let domain = Uuid::from_u128(0x550);
    let mut body = request_body(domain, Uuid::from_u128(0x551), Uuid::from_u128(0x552));
    body["target"] = json!(reference(3));
    body["raw_issuer_canary"] = json!(RAW_ISSUER_CANARY);
    body["jwt_canary"] = json!(JWT_CANARY);
    body["jwks_canary"] = json!(JWKS_CANARY);

    let trace_buffer = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_writer(CapturingMakeWriter {
            buffer: Arc::clone(&trace_buffer),
        })
        .with_ansi(false)
        .finish();
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let runtime_handle = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread sentinel runtime");
    let (status, response) = metrics::with_local_recorder(&recorder, || {
        tracing::subscriber::with_default(subscriber, || {
            runtime_handle.block_on(post(runtime, "/operator/v1/lifecycle/revoke", body))
        })
    });
    assert_eq!(status, StatusCode::FORBIDDEN);

    let logs = String::from_utf8(trace_buffer.lock().expect("trace buffer").clone())
        .expect("UTF-8 trace output");
    let metrics = format!("{:?}", snapshotter.snapshot().into_vec());
    let surfaces = format!("{response}\n{logs}\n{metrics}");
    for canary in [
        CREDENTIAL_CANARY,
        PRIVATE_CLAIM_CANARY,
        RAW_ISSUER_CANARY,
        JWT_CANARY,
        JWKS_CANARY,
    ] {
        assert!(
            !surfaces.contains(canary),
            "planted canary crossed a response, log, or metric surface"
        );
    }
    assert!(logs.contains("request remains denied"));
}
