//! Pipeline tests: authority, idempotency, and crash behavior.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use buzz_sdk_pkg::broker::{
    AgentTarget, AgentsCreateArgs, AgentsCreateOutcome, AgentsDeleteArgs, AgentsDeleteOutcome,
    CapabilityArgs,
};

use super::super::store::MemoryExecutionLog;
use super::*;

const CHANNEL: &str = "b2c38ca8-9ec3-411e-bab5-f9deab34d52e";
const OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const REQUESTER: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const AGENT: &str = "3333333333333333333333333333333333333333333333333333333333333333";

fn db() -> MemoryExecutionLog {
    MemoryExecutionLog::new()
}

fn create_request(request_id: &str) -> Vec<u8> {
    let request = BrokerRequest::new(
        request_id,
        CapabilityArgs::AgentsCreate(AgentsCreateArgs {
            channel_id: CHANNEL.into(),
            display_name: "Helper".into(),
            system_prompt: "Help.".into(),
            runtime: None,
            provider: None,
            model: None,
            respond_to: None,
        }),
    )
    .unwrap();
    serde_json::to_vec(&request).unwrap()
}

fn verified(payload: Vec<u8>) -> VerifiedRequest {
    VerifiedRequest {
        owner_pubkey: OWNER.into(),
        requester_pubkey: REQUESTER.into(),
        relay_scope: "wss://relay.example".into(),
        payload,
    }
}

struct AllowAll;
impl Authorizer for AllowAll {
    fn authorize<'a>(
        &'a self,
        _request: &'a VerifiedRequest,
        _capability: Capability,
        _parsed: &'a BrokerRequest,
    ) -> BoxFuture<'a, Result<(), BrokerError>> {
        Box::pin(async { Ok(()) })
    }
}

struct DenyAll;
impl Authorizer for DenyAll {
    fn authorize<'a>(
        &'a self,
        _request: &'a VerifiedRequest,
        _capability: Capability,
        _parsed: &'a BrokerRequest,
    ) -> BoxFuture<'a, Result<(), BrokerError>> {
        Box::pin(async { Err(BrokerError::unauthorized("not a member of that channel")) })
    }
}

/// Counts executions so a test can prove a handler ran at most once.
struct CountingHandler {
    calls: AtomicUsize,
    outcome: fn() -> HandlerOutcome,
}

impl CountingHandler {
    fn new(outcome: fn() -> HandlerOutcome) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            outcome,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl CapabilityHandler for CountingHandler {
    fn execute<'a>(
        &'a self,
        _request: &'a VerifiedRequest,
        _parsed: &'a BrokerRequest,
    ) -> BoxFuture<'a, HandlerOutcome> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            (self.outcome)()
        })
    }
}

struct OneHandler<'a> {
    capability: Capability,
    handler: &'a dyn CapabilityHandler,
}

impl CapabilityRegistry for OneHandler<'_> {
    fn handler(&self, capability: Capability) -> Option<&dyn CapabilityHandler> {
        (capability == self.capability).then_some(self.handler)
    }
}

struct NoHandlers;
impl CapabilityRegistry for NoHandlers {
    fn handler(&self, _capability: Capability) -> Option<&dyn CapabilityHandler> {
        None
    }
}

fn success() -> HandlerOutcome {
    HandlerOutcome::Succeeded(CapabilityOutcome::AgentsCreate(AgentsCreateOutcome {
        agent_pubkey: AGENT.into(),
        display_name: "Helper".into(),
        channel_id: CHANNEL.into(),
    }))
}

async fn run(
    request: &VerifiedRequest,
    authorizer: &dyn Authorizer,
    registry: &dyn CapabilityRegistry,
    log: &dyn ExecutionLog,
) -> BrokerResponse {
    execute(
        request,
        BrokerContext {
            authorizer,
            registry,
            log,
            now: 1000,
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_valid_authorized_request_executes_once_and_returns_its_outcome() {
    let log = db();
    let handler = CountingHandler::new(success);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };
    let request = verified(create_request("req-1"));

    let response = run(&request, &AllowAll, &registry, &log).await;
    assert_eq!(response.request_id, "req-1");
    assert!(response.result.is_succeeded());
    assert!(!response.replayed);
    assert_eq!(handler.calls(), 1);
}

/// The central idempotency property: replaying the identical request returns
/// the recorded outcome and does not run the handler a second time.
#[tokio::test]
async fn replaying_an_identical_request_returns_the_record_without_re_executing() {
    let log = db();
    let handler = CountingHandler::new(success);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };
    let request = verified(create_request("req-1"));

    let first = run(&request, &AllowAll, &registry, &log).await;
    let second = run(&request, &AllowAll, &registry, &log).await;

    assert_eq!(handler.calls(), 1, "handler must not run twice");
    assert!(second.replayed);
    assert_eq!(first.result, second.result, "replay must be identical");
}

/// Same request id, different content: the caller must be told, not handed
/// someone else's outcome.
#[tokio::test]
async fn a_different_payload_under_the_same_request_id_is_refused() {
    let log = db();
    let handler = CountingHandler::new(success);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };

    run(
        &verified(create_request("req-1")),
        &AllowAll,
        &registry,
        &log,
    )
    .await;

    let mut different = BrokerRequest::new(
        "req-1",
        CapabilityArgs::AgentsCreate(AgentsCreateArgs {
            channel_id: CHANNEL.into(),
            display_name: "Different agent".into(),
            system_prompt: "Help.".into(),
            runtime: None,
            provider: None,
            model: None,
            respond_to: None,
        }),
    )
    .unwrap();
    different.request_id = "req-1".into();
    let response = run(
        &verified(serde_json::to_vec(&different).unwrap()),
        &AllowAll,
        &registry,
        &log,
    )
    .await;

    assert_eq!(
        response.result.error().unwrap().code,
        BrokerErrorCode::RequestIdConflict
    );
    assert_eq!(handler.calls(), 1, "the conflict must not execute");
}

/// An unauthorized request must not leave a claim behind: otherwise a later
/// authorized retry would be answered from a refusal.
#[tokio::test]
async fn an_unauthorized_request_does_not_execute_or_leave_a_claim() {
    let log = db();
    let handler = CountingHandler::new(success);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };
    let request = verified(create_request("req-1"));

    let denied = run(&request, &DenyAll, &registry, &log).await;
    assert_eq!(
        denied.result.error().unwrap().code,
        BrokerErrorCode::Unauthorized
    );
    assert_eq!(handler.calls(), 0);

    // The same request, now authorized, still runs normally.
    let allowed = run(&request, &AllowAll, &registry, &log).await;
    assert!(allowed.result.is_succeeded());
    assert!(!allowed.replayed);
    assert_eq!(handler.calls(), 1);
}

/// A crash between the mutation and its record must surface as indeterminate,
/// never as a silent re-run.
#[tokio::test]
async fn an_interrupted_execution_reports_indeterminate_and_never_re_executes() {
    let log = db();
    let handler = CountingHandler::new(success);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };
    let request = verified(create_request("req-1"));

    // Simulate a process that claimed the key and died before completing.
    let key = ExecutionKey {
        relay_scope: "wss://relay.example".into(),
        owner_pubkey: OWNER.into(),
        requester_pubkey: REQUESTER.into(),
        capability: "agents.create".into(),
        request_id: "req-1".into(),
    };
    let digest = super::super::request_digest(&request.payload).unwrap();
    log.with_conn(|conn| store::claim(conn, &key, &digest, 1, 500))
        .unwrap();

    let response = run(&request, &AllowAll, &registry, &log).await;
    assert_eq!(
        response.result.error().unwrap().code,
        BrokerErrorCode::OutcomeUnknown
    );
    assert_eq!(
        handler.calls(),
        0,
        "an interrupted request must not be re-executed"
    );
}

#[tokio::test]
async fn a_failed_handler_is_recorded_and_replayed_as_failed() {
    fn failure() -> HandlerOutcome {
        HandlerOutcome::Failed(BrokerError::new(
            BrokerErrorCode::CapabilityFailed,
            "runtime not installed",
        ))
    }
    let log = db();
    let handler = CountingHandler::new(failure);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };
    let request = verified(create_request("req-1"));

    let first = run(&request, &AllowAll, &registry, &log).await;
    assert_eq!(
        first.result.error().unwrap().code,
        BrokerErrorCode::CapabilityFailed
    );

    // A failure is terminal: retrying replays it rather than re-running.
    let second = run(&request, &AllowAll, &registry, &log).await;
    assert!(second.replayed);
    assert_eq!(first.result, second.result);
    assert_eq!(handler.calls(), 1);
}

#[tokio::test]
async fn an_indeterminate_handler_outcome_is_recorded_as_indeterminate() {
    fn unknown() -> HandlerOutcome {
        HandlerOutcome::Indeterminate(BrokerError::new(
            BrokerErrorCode::OutcomeUnknown,
            "published but could not confirm",
        ))
    }
    let log = db();
    let handler = CountingHandler::new(unknown);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };
    let request = verified(create_request("req-1"));

    let response = run(&request, &AllowAll, &registry, &log).await;
    assert!(matches!(
        response.result,
        BrokerResult::Indeterminate { .. }
    ));
    assert_eq!(handler.calls(), 1);
}

#[tokio::test]
async fn a_malformed_payload_is_refused_without_a_claim() {
    let log = db();
    let response = run(
        &verified(b"not json".to_vec()),
        &AllowAll,
        &NoHandlers,
        &log,
    )
    .await;
    assert_eq!(
        response.result.error().unwrap().code,
        BrokerErrorCode::InvalidRequest
    );
}

#[tokio::test]
async fn an_oversized_payload_is_refused() {
    let log = db();
    let response = run(
        &verified(vec![b'x'; super::super::MAX_REQUEST_BYTES + 1]),
        &AllowAll,
        &NoHandlers,
        &log,
    )
    .await;
    assert_eq!(
        response.result.error().unwrap().code,
        BrokerErrorCode::InvalidRequest
    );
}

#[tokio::test]
async fn a_capability_this_host_does_not_offer_is_refused() {
    let log = db();
    let response = run(
        &verified(create_request("req-1")),
        &AllowAll,
        &NoHandlers,
        &log,
    )
    .await;
    assert_eq!(
        response.result.error().unwrap().code,
        BrokerErrorCode::UnknownCapability
    );
}

/// Two requesters using the same request id are separate executions: the key
/// includes the verified requester, so one cannot read the other's outcome.
#[tokio::test]
async fn the_same_request_id_from_two_requesters_are_separate_executions() {
    let log = db();
    let handler = CountingHandler::new(success);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };

    let payload = create_request("req-1");
    let first = verified(payload.clone());
    let second = VerifiedRequest {
        requester_pubkey: AGENT.into(),
        ..verified(payload)
    };

    assert!(!run(&first, &AllowAll, &registry, &log).await.replayed);
    assert!(
        !run(&second, &AllowAll, &registry, &log).await.replayed,
        "a different requester must not replay another's record"
    );
    assert_eq!(handler.calls(), 2);
}

/// The pipeline must take identity from the verified transport, never from the
/// payload. A body claiming another owner cannot even deserialize.
#[tokio::test]
async fn identity_in_the_payload_cannot_override_transport_identity() {
    let log = db();
    let handler = CountingHandler::new(success);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };

    let mut json: serde_json::Value = serde_json::from_slice(&create_request("req-1")).unwrap();
    json.as_object_mut()
        .unwrap()
        .insert("ownerPubkey".into(), serde_json::json!(AGENT));

    let response = run(
        &verified(serde_json::to_vec(&json).unwrap()),
        &AllowAll,
        &registry,
        &log,
    )
    .await;
    assert_eq!(
        response.result.error().unwrap().code,
        BrokerErrorCode::InvalidRequest
    );
    assert_eq!(handler.calls(), 0);
}

/// The authorizer sees the transport-derived identity, so policy is written
/// against who actually signed.
#[tokio::test]
async fn the_authorizer_receives_transport_derived_identity() {
    struct Recording(Mutex<Vec<(String, String, String)>>);
    impl Authorizer for Recording {
        fn authorize<'a>(
            &'a self,
            request: &'a VerifiedRequest,
            capability: Capability,
            _parsed: &'a BrokerRequest,
        ) -> BoxFuture<'a, Result<(), BrokerError>> {
            Box::pin(async move {
                self.0.lock().unwrap().push((
                    request.owner_pubkey.clone(),
                    request.requester_pubkey.clone(),
                    capability.as_str().to_string(),
                ));
                Ok(())
            })
        }
    }

    let log = db();
    let handler = CountingHandler::new(success);
    let registry = OneHandler {
        capability: Capability::AgentsCreate,
        handler: &handler,
    };
    let authorizer = Recording(Mutex::new(Vec::new()));

    run(
        &verified(create_request("req-1")),
        &authorizer,
        &registry,
        &log,
    )
    .await;

    assert_eq!(
        authorizer.0.lock().unwrap().as_slice(),
        &[(
            OWNER.to_string(),
            REQUESTER.to_string(),
            "agents.create".to_string()
        )]
    );
}

#[tokio::test]
async fn delete_requests_route_to_the_delete_capability() {
    fn deleted() -> HandlerOutcome {
        HandlerOutcome::Succeeded(CapabilityOutcome::AgentsDelete(AgentsDeleteOutcome {
            agent_pubkey: AGENT.into(),
            display_name: "Gone".into(),
        }))
    }
    let log = db();
    let handler = CountingHandler::new(deleted);
    let registry = OneHandler {
        capability: Capability::AgentsDelete,
        handler: &handler,
    };

    let request = BrokerRequest::new(
        "req-del",
        CapabilityArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Pubkey(AGENT.into()),
        }),
    )
    .unwrap();
    let response = run(
        &verified(serde_json::to_vec(&request).unwrap()),
        &AllowAll,
        &registry,
        &log,
    )
    .await;

    assert!(response.result.is_succeeded());
    assert_eq!(handler.calls(), 1);
}
