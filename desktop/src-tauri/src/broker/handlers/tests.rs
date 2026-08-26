//! Handler tests, driven through the real pipeline against a fake service.

use std::sync::Mutex;

use buzz_sdk_pkg::broker::{AgentTarget, BrokerResponse, BrokerResult};

use super::super::agents_policy::{AgentRoster, AgentsAuthorizer};
use super::super::pipeline::{execute, BrokerContext};
use super::super::store::MemoryExecutionLog;
use super::*;

const CHANNEL: &str = "b2c38ca8-9ec3-411e-bab5-f9deab34d52e";
const OWNER: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const REQUESTER: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const AGENT: &str = "3333333333333333333333333333333333333333333333333333333333333333";

/// Records every call so a test can assert what the domain layer was asked to
/// do -- and that it was asked exactly once.
///
/// `Mutex` rather than `RefCell` because [`AgentService`] is `Send + Sync`: the
/// pipeline dispatches handlers from async code, so a fake has to be shareable
/// the same way a real service is.
#[derive(Default)]
struct FakeService {
    calls: Mutex<Vec<String>>,
    create: Option<Result<AgentsCreateOutcome, ServiceError>>,
    update: Option<Result<AgentsUpdateOutcome, ServiceError>>,
    delete: Option<Result<AgentsDeleteOutcome, ServiceError>>,
}

impl FakeService {
    fn creating() -> Self {
        Self {
            create: Some(Ok(AgentsCreateOutcome {
                agent_pubkey: AGENT.into(),
                display_name: "Helper".into(),
                channel_id: CHANNEL.into(),
            })),
            ..Self::default()
        }
    }

    fn record(&self, call: String) {
        self.calls.lock().expect("fake service poisoned").push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("fake service poisoned").clone()
    }
}

impl AgentService for FakeService {
    fn create<'a>(
        &'a self,
        owner_pubkey: &'a str,
        args: &'a AgentsCreateArgs,
    ) -> BoxFuture<'a, Result<AgentsCreateOutcome, ServiceError>> {
        Box::pin(async move {
            self.record(format!("create:{owner_pubkey}:{}", args.display_name));
            self.create
                .clone()
                .unwrap_or(Err(ServiceError::Failed("no create configured".into())))
        })
    }

    fn update<'a>(
        &'a self,
        owner_pubkey: &'a str,
        args: &'a AgentsUpdateArgs,
    ) -> BoxFuture<'a, Result<AgentsUpdateOutcome, ServiceError>> {
        Box::pin(async move {
            self.record(format!("update:{owner_pubkey}:{:?}", args.target));
            self.update
                .clone()
                .unwrap_or(Err(ServiceError::Failed("no update configured".into())))
        })
    }

    fn delete<'a>(
        &'a self,
        owner_pubkey: &'a str,
        args: &'a AgentsDeleteArgs,
    ) -> BoxFuture<'a, Result<AgentsDeleteOutcome, ServiceError>> {
        Box::pin(async move {
            self.record(format!("delete:{owner_pubkey}:{:?}", args.target));
            self.delete
                .clone()
                .unwrap_or(Err(ServiceError::Failed("no delete configured".into())))
        })
    }
}

struct OwnerRoster;
impl AgentRoster for OwnerRoster {
    fn agent_pubkeys<'a>(
        &'a self,
        _owner_pubkey: &'a str,
    ) -> BoxFuture<'a, Result<Vec<String>, String>> {
        Box::pin(async { Ok(vec![REQUESTER.into()]) })
    }
}

async fn run(payload: Vec<u8>, service: FakeService) -> (BrokerResponse, Vec<String>) {
    let handlers = AgentsHandlers::new(service);
    let registry = handlers.registry();
    let authorizer = AgentsAuthorizer::new(OwnerRoster);
    let log = MemoryExecutionLog::new();

    let request = VerifiedRequest {
        owner_pubkey: OWNER.into(),
        requester_pubkey: REQUESTER.into(),
        relay_scope: "wss://relay.example".into(),
        payload,
    };
    let response = execute(
        &request,
        BrokerContext {
            authorizer: &authorizer,
            registry: &registry,
            log: &log,
            now: 1000,
        },
    )
    .await
    .unwrap();
    let calls = handlers.service.calls();
    (response, calls)
}

fn payload(args: CapabilityArgs) -> Vec<u8> {
    serde_json::to_vec(&BrokerRequest::new("req-1", args).unwrap()).unwrap()
}

fn create_args() -> CapabilityArgs {
    CapabilityArgs::AgentsCreate(AgentsCreateArgs {
        channel_id: CHANNEL.into(),
        display_name: "Helper".into(),
        system_prompt: "Help.".into(),
        runtime: None,
        provider: None,
        model: None,
        respond_to: None,
    })
}

#[tokio::test]
async fn create_reaches_the_service_and_reports_the_new_agent() {
    let (response, calls) = run(payload(create_args()), FakeService::creating()).await;

    match response.result {
        BrokerResult::Succeeded {
            outcome: CapabilityOutcome::AgentsCreate(outcome),
        } => {
            assert_eq!(outcome.agent_pubkey, AGENT);
            assert_eq!(outcome.channel_id, CHANNEL);
        }
        other => panic!("expected create success, got {other:?}"),
    }
    assert_eq!(calls, vec![format!("create:{OWNER}:Helper")]);
}

/// The owner the service acts for comes from the verified transport, not the
/// payload -- so a handler can never be aimed at another owner's agents.
#[tokio::test]
async fn the_service_receives_the_transport_derived_owner() {
    let (_, calls) = run(payload(create_args()), FakeService::creating()).await;
    assert!(calls[0].contains(OWNER), "unexpected call: {}", calls[0]);
}

#[tokio::test]
async fn update_and_delete_route_to_their_own_service_methods() {
    let update = FakeService {
        update: Some(Ok(AgentsUpdateOutcome {
            agent_pubkey: AGENT.into(),
            display_name: "Renamed".into(),
            updated_fields: vec!["displayName".into()],
        })),
        ..FakeService::default()
    };
    let (response, calls) = run(
        payload(CapabilityArgs::AgentsUpdate(AgentsUpdateArgs {
            target: AgentTarget::Pubkey(AGENT.into()),
            display_name: Some("Renamed".into()),
            system_prompt: None,
            runtime: None,
            provider: None,
            model: None,
            respond_to: None,
        })),
        update,
    )
    .await;
    assert!(response.result.is_succeeded());
    assert!(calls[0].starts_with("update:"), "unexpected: {}", calls[0]);

    let delete = FakeService {
        delete: Some(Ok(AgentsDeleteOutcome {
            agent_pubkey: AGENT.into(),
            display_name: "Gone".into(),
        })),
        ..FakeService::default()
    };
    let (response, calls) = run(
        payload(CapabilityArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Pubkey(AGENT.into()),
        })),
        delete,
    )
    .await;
    assert!(response.result.is_succeeded());
    assert!(calls[0].starts_with("delete:"), "unexpected: {}", calls[0]);
}

/// A service that failed cleanly must produce `failed`, which promises no side
/// effects took hold.
#[tokio::test]
async fn a_clean_service_failure_becomes_failed() {
    let service = FakeService {
        create: Some(Err(ServiceError::Failed("runtime not installed".into()))),
        ..FakeService::default()
    };
    let (response, _) = run(payload(create_args()), service).await;

    match &response.result {
        BrokerResult::Failed { error } => {
            assert_eq!(error.code, BrokerErrorCode::CapabilityFailed);
            assert!(error.message.contains("runtime not installed"));
        }
        other => panic!("expected failed, got {other:?}"),
    }
}

/// A service that cannot tell whether it mutated must produce `indeterminate`,
/// never `failed` -- the difference is what stops a caller from safely retrying
/// something that may already have happened.
#[tokio::test]
async fn an_unknown_service_outcome_becomes_indeterminate_not_failed() {
    let service = FakeService {
        create: Some(Err(ServiceError::Unknown(
            "agent minted but profile publish unconfirmed".into(),
        ))),
        ..FakeService::default()
    };
    let (response, _) = run(payload(create_args()), service).await;

    match &response.result {
        BrokerResult::Indeterminate { error } => {
            assert_eq!(error.code, BrokerErrorCode::OutcomeUnknown);
        }
        other => panic!("expected indeterminate, got {other:?}"),
    }
}

/// A requester that is not one of the owner's agents must never reach the
/// service at all.
#[tokio::test]
async fn an_unauthorized_requester_never_reaches_the_service() {
    struct EmptyRoster;
    impl AgentRoster for EmptyRoster {
        fn agent_pubkeys<'a>(
            &'a self,
            _owner: &'a str,
        ) -> BoxFuture<'a, Result<Vec<String>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    let handlers = AgentsHandlers::new(FakeService::creating());
    let registry = handlers.registry();
    let authorizer = AgentsAuthorizer::new(EmptyRoster);
    let log = MemoryExecutionLog::new();

    let request = VerifiedRequest {
        owner_pubkey: OWNER.into(),
        requester_pubkey: REQUESTER.into(),
        relay_scope: "wss://relay.example".into(),
        payload: payload(create_args()),
    };
    let response = execute(
        &request,
        BrokerContext {
            authorizer: &authorizer,
            registry: &registry,
            log: &log,
            now: 1000,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        response.result.error().unwrap().code,
        BrokerErrorCode::Unauthorized
    );
    assert!(
        handlers.service.calls().is_empty(),
        "an unauthorized request must not touch the domain layer"
    );
}

/// End-to-end idempotency through the real handler stack: the domain mutation
/// must run once even though the request arrives twice.
#[tokio::test]
async fn a_replayed_request_does_not_mutate_twice() {
    let handlers = AgentsHandlers::new(FakeService::creating());
    let registry = handlers.registry();
    let authorizer = AgentsAuthorizer::new(OwnerRoster);
    let log = MemoryExecutionLog::new();

    let request = VerifiedRequest {
        owner_pubkey: OWNER.into(),
        requester_pubkey: REQUESTER.into(),
        relay_scope: "wss://relay.example".into(),
        payload: payload(create_args()),
    };
    let run_once = || async {
        execute(
            &request,
            BrokerContext {
                authorizer: &authorizer,
                registry: &registry,
                log: &log,
                now: 1000,
            },
        )
        .await
        .unwrap()
    };

    let first = run_once().await;
    let second = run_once().await;

    assert!(first.result.is_succeeded());
    assert_eq!(first.result, second.result);
    assert!(second.replayed);
    assert_eq!(
        handlers.service.calls().len(),
        1,
        "the domain mutation must run exactly once"
    );
}

/// A create outcome cannot carry the minted secret, by construction.
#[tokio::test]
async fn a_create_outcome_never_carries_the_minted_nsec() {
    let (response, _) = run(payload(create_args()), FakeService::creating()).await;
    let json = serde_json::to_string(&response).unwrap();
    assert!(!json.contains("nsec"), "response leaked a secret: {json}");
    for forbidden in ["privateKey", "private_key", "secret"] {
        assert!(
            !json.contains(forbidden),
            "response leaked {forbidden}: {json}"
        );
    }
}
