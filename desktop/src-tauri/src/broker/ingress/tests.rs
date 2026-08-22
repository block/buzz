//! Ingress tests: frame verification, the digest boundary, and result delivery.

use std::sync::Mutex;

use buzz_sdk_pkg::broker::{
    AgentsCreateArgs, AgentsCreateOutcome, BrokerErrorCode, BrokerRequest, BrokerResult,
    Capability, CapabilityArgs, CapabilityOutcome,
};
use nostr::{EventBuilder, Tag, Timestamp};

use super::super::pipeline::{Authorizer, CapabilityHandler, CapabilityRegistry, HandlerOutcome};
use super::super::store::MemoryExecutionLog;
use super::*;

const CHANNEL: &str = "b2c38ca8-9ec3-411e-bab5-f9deab34d52e";
const RELAY: &str = "wss://relay.example";
const NOW: i64 = 1_700_000_000;

fn request(request_id: &str) -> BrokerRequest {
    BrokerRequest::new(
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
    .expect("valid request")
}

/// Seal `payload` as requester-to-owner telemetry, the shape a real request has.
fn frame(requester: &Keys, owner: &Keys, payload: &serde_json::Value, created_at: i64) -> Event {
    let encrypted =
        encrypt_observer_payload(requester, &owner.public_key(), payload).expect("encrypt payload");
    EventBuilder::new(Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16), encrypted)
        .tags([
            Tag::parse(["p", &owner.public_key().to_hex()]).expect("p tag"),
            Tag::parse([OBSERVER_AGENT_TAG, &requester.public_key().to_hex()]).expect("agent tag"),
            Tag::parse([OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]).expect("frame tag"),
        ])
        .custom_created_at(Timestamp::from(created_at as u64))
        .sign_with_keys(requester)
        .expect("sign frame")
}

fn request_frame(requester: &Keys, owner: &Keys, request: &BrokerRequest) -> Event {
    frame(
        requester,
        owner,
        &serde_json::to_value(request).expect("request json"),
        NOW,
    )
}

struct AllowAll;
impl Authorizer for AllowAll {
    fn authorize<'a>(
        &'a self,
        _request: &'a VerifiedRequest,
        _capability: Capability,
        _parsed: &'a BrokerRequest,
    ) -> BoxFuture<'a, Result<(), buzz_sdk_pkg::broker::BrokerError>> {
        Box::pin(async { Ok(()) })
    }
}

/// Records every execution so a test can prove a retry did not re-run one.
struct RecordingHandler {
    calls: Mutex<Vec<String>>,
}

impl RecordingHandler {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        self.calls.lock().expect("calls").len()
    }
}

impl CapabilityHandler for RecordingHandler {
    fn execute<'a>(
        &'a self,
        _request: &'a VerifiedRequest,
        parsed: &'a BrokerRequest,
    ) -> BoxFuture<'a, HandlerOutcome> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("calls")
                .push(parsed.request_id.clone());
            HandlerOutcome::Succeeded(CapabilityOutcome::AgentsCreate(AgentsCreateOutcome {
                agent_pubkey: "44".repeat(32),
                display_name: "Helper".into(),
                channel_id: CHANNEL.into(),
            }))
        })
    }
}

struct OneHandler<'a>(&'a RecordingHandler);
impl CapabilityRegistry for OneHandler<'_> {
    fn handler(&self, _capability: Capability) -> Option<&dyn CapabilityHandler> {
        Some(self.0)
    }
}

/// Captures published frames instead of reaching a relay.
struct CapturingPublisher {
    published: Mutex<Vec<Event>>,
    fail: Option<String>,
}

impl CapturingPublisher {
    fn new() -> Self {
        Self {
            published: Mutex::new(Vec::new()),
            fail: None,
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            published: Mutex::new(Vec::new()),
            fail: Some(message.to_string()),
        }
    }

    fn frames(&self) -> Vec<Event> {
        self.published.lock().expect("published").clone()
    }
}

impl ResultPublisher for CapturingPublisher {
    fn publish(&self, event: Event) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.published.lock().expect("published").push(event);
            match &self.fail {
                Some(message) => Err(message.clone()),
                None => Ok(()),
            }
        })
    }
}

struct Harness {
    owner: Keys,
    requester: Keys,
    log: MemoryExecutionLog,
    handler: RecordingHandler,
}

impl Harness {
    fn new() -> Self {
        Self {
            owner: Keys::generate(),
            requester: Keys::generate(),
            log: MemoryExecutionLog::new(),
            handler: RecordingHandler::new(),
        }
    }

    async fn handle(&self, event: &Event, publisher: &dyn ResultPublisher) -> Handled {
        let registry = OneHandler(&self.handler);
        handle_frame(
            &self.owner,
            event,
            RELAY,
            NOW,
            BrokerContext {
                authorizer: &AllowAll,
                registry: &registry,
                log: &self.log,
                now: NOW,
            },
            publisher,
        )
        .await
        .expect("no host fault")
    }
}

fn response_of(handled: &Handled) -> &BrokerResponse {
    match handled {
        Handled::Executed { response, .. } => response,
        Handled::Ignored(rejection) => panic!("expected execution, got {rejection:?}"),
    }
}

fn rejection_of(handled: &Handled) -> &FrameRejection {
    match handled {
        Handled::Ignored(rejection) => rejection,
        Handled::Executed { response, .. } => panic!("expected rejection, got {response:?}"),
    }
}

// ── identity is transport-derived ───────────────────────────────────────────

#[test]
fn a_verified_frame_yields_transport_derived_identity() {
    let owner = Keys::generate();
    let requester = Keys::generate();
    let event = request_frame(&requester, &owner, &request("req-1"));

    let verified = verify_frame(&owner, &event, RELAY, NOW).expect("verified");
    assert_eq!(verified.owner_pubkey, owner.public_key().to_hex());
    assert_eq!(verified.requester_pubkey, requester.public_key().to_hex());
    assert_eq!(verified.relay_scope, RELAY);
}

/// The owner is this host's own key, never anything the frame carried — so a
/// request body that names another owner changes nothing about who is acted on.
#[test]
fn a_payload_cannot_name_its_own_owner_or_requester() {
    let owner = Keys::generate();
    let requester = Keys::generate();
    let mut payload = serde_json::to_value(request("req-1")).expect("json");
    payload["ownerPubkey"] = serde_json::json!("99".repeat(32));
    payload["requesterPubkey"] = serde_json::json!("88".repeat(32));
    let event = frame(&requester, &owner, &payload, NOW);

    let verified = verify_frame(&owner, &event, RELAY, NOW).expect("verified");
    assert_eq!(verified.owner_pubkey, owner.public_key().to_hex());
    assert_eq!(verified.requester_pubkey, requester.public_key().to_hex());
}

// ── the digest boundary ─────────────────────────────────────────────────────

/// The load-bearing retry property: two attempts at the same operation differ in
/// every frame-level field — event id, signature, `created_at` — and must still
/// replay rather than conflict. If the digest ever covered frame or envelope
/// metadata, this test fails with `request_id_conflict`.
#[tokio::test]
async fn a_retry_with_a_new_frame_replays_instead_of_conflicting() {
    let harness = Harness::new();
    let request = request("req-retry");
    let publisher = CapturingPublisher::new();

    let first = request_frame(&harness.requester, &harness.owner, &request);
    let second = frame(
        &harness.requester,
        &harness.owner,
        &serde_json::to_value(&request).expect("json"),
        NOW + 60,
    );
    assert_ne!(first.id, second.id, "retry must be a distinct frame");
    assert_ne!(first.created_at, second.created_at);

    let first = harness.handle(&first, &publisher).await;
    assert!(response_of(&first).result.is_succeeded());
    assert!(!response_of(&first).replayed);

    let second = harness.handle(&second, &publisher).await;
    let second = response_of(&second);
    assert!(
        second.result.is_succeeded(),
        "retry must replay the recorded success, got {:?}",
        second.result
    );
    assert!(second.replayed, "retry must be marked as a replay");
    assert_eq!(harness.handler.calls(), 1, "the handler must run once");
}

/// The other half of the contract: same `requestId`, genuinely different
/// request. This must be refused rather than answered with the first outcome.
#[tokio::test]
async fn the_same_request_id_with_different_content_still_conflicts() {
    let harness = Harness::new();
    let publisher = CapturingPublisher::new();

    let first = request_frame(&harness.requester, &harness.owner, &request("req-1"));
    harness.handle(&first, &publisher).await;

    let mut changed = request("req-1");
    if let CapabilityArgs::AgentsCreate(args) = &mut changed.capability {
        args.display_name = "Different".into();
    }
    let second = request_frame(&harness.requester, &harness.owner, &changed);
    let response = harness.handle(&second, &publisher).await;
    assert_eq!(
        response_of(&response)
            .result
            .error()
            .map(|error| error.code),
        Some(BrokerErrorCode::RequestIdConflict)
    );
    assert_eq!(harness.handler.calls(), 1);
}

// ── what is refused, and what is silently ignored ───────────────────────────

#[test]
fn ordinary_telemetry_is_not_broker_traffic() {
    let owner = Keys::generate();
    let requester = Keys::generate();
    let event = frame(
        &requester,
        &owner,
        &serde_json::json!({"kind": "turn_started", "seq": 4}),
        NOW,
    );
    assert_eq!(
        verify_frame(&owner, &event, RELAY, NOW).unwrap_err(),
        FrameRejection::NotBrokerTraffic
    );
}

#[test]
fn a_frame_for_another_owner_is_not_this_hosts_traffic() {
    let owner = Keys::generate();
    let other_owner = Keys::generate();
    let requester = Keys::generate();
    let event = request_frame(&requester, &other_owner, &request("req-1"));
    assert_eq!(
        verify_frame(&owner, &event, RELAY, NOW).unwrap_err(),
        FrameRejection::NotBrokerTraffic
    );
}

/// A frame signed by someone other than the agent it names would let a third
/// party borrow that agent's authorization.
#[test]
fn a_signer_that_is_not_the_named_agent_is_refused() {
    let owner = Keys::generate();
    let requester = Keys::generate();
    let impostor = Keys::generate();
    let encrypted = encrypt_observer_payload(
        &impostor,
        &owner.public_key(),
        &serde_json::to_value(request("req-1")).expect("json"),
    )
    .expect("encrypt");
    let event = EventBuilder::new(Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16), encrypted)
        .tags([
            Tag::parse(["p", &owner.public_key().to_hex()]).expect("p tag"),
            Tag::parse([OBSERVER_AGENT_TAG, &requester.public_key().to_hex()]).expect("agent tag"),
            Tag::parse([OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]).expect("frame tag"),
        ])
        .custom_created_at(Timestamp::from(NOW as u64))
        .sign_with_keys(&impostor)
        .expect("sign");

    let error = verify_frame(&owner, &event, RELAY, NOW).unwrap_err();
    assert!(
        matches!(&error, FrameRejection::Rejected(message) if message.contains("not the agent it names")),
        "unexpected: {error:?}"
    );
}

#[test]
fn a_stale_frame_is_refused() {
    let owner = Keys::generate();
    let requester = Keys::generate();
    let event = frame(
        &requester,
        &owner,
        &serde_json::to_value(request("req-1")).expect("json"),
        NOW - MAX_FRAME_SKEW_SECS - 1,
    );
    let error = verify_frame(&owner, &event, RELAY, NOW).unwrap_err();
    assert!(
        matches!(&error, FrameRejection::Rejected(message) if message.contains("outside the")),
        "unexpected: {error:?}"
    );
}

#[test]
fn a_future_dated_frame_is_refused() {
    let owner = Keys::generate();
    let requester = Keys::generate();
    let event = frame(
        &requester,
        &owner,
        &serde_json::to_value(request("req-1")).expect("json"),
        NOW + MAX_FRAME_SKEW_SECS + 1,
    );
    assert!(matches!(
        verify_frame(&owner, &event, RELAY, NOW),
        Err(FrameRejection::Rejected(_))
    ));
}

/// A tampered frame must be caught here, not merely trusted from the relay.
#[test]
fn a_frame_with_a_broken_signature_is_refused() {
    let owner = Keys::generate();
    let requester = Keys::generate();
    let other = Keys::generate();
    let event = request_frame(&requester, &owner, &request("req-1"));
    let mut forged = event.clone();
    forged.pubkey = other.public_key();

    let error = verify_frame(&owner, &forged, RELAY, NOW).unwrap_err();
    assert!(
        matches!(&error, FrameRejection::Rejected(message) if message.contains("invalid id")
            || message.contains("invalid signature")),
        "unexpected: {error:?}"
    );
}

#[test]
fn a_duplicated_routing_tag_is_refused() {
    let owner = Keys::generate();
    let requester = Keys::generate();
    let encrypted = encrypt_observer_payload(
        &requester,
        &owner.public_key(),
        &serde_json::to_value(request("req-1")).expect("json"),
    )
    .expect("encrypt");
    let event = EventBuilder::new(Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16), encrypted)
        .tags([
            Tag::parse(["p", &owner.public_key().to_hex()]).expect("p tag"),
            Tag::parse(["p", &Keys::generate().public_key().to_hex()]).expect("second p tag"),
            Tag::parse([OBSERVER_AGENT_TAG, &requester.public_key().to_hex()]).expect("agent tag"),
            Tag::parse([OBSERVER_FRAME_TAG, OBSERVER_FRAME_TELEMETRY]).expect("frame tag"),
        ])
        .custom_created_at(Timestamp::from(NOW as u64))
        .sign_with_keys(&requester)
        .expect("sign");

    assert!(matches!(
        verify_frame(&owner, &event, RELAY, NOW),
        Err(FrameRejection::Rejected(_))
    ));
}

#[test]
fn a_control_frame_is_not_an_inbound_request() {
    let owner = Keys::generate();
    let requester = Keys::generate();
    let encrypted = encrypt_observer_payload(
        &owner,
        &requester.public_key(),
        &serde_json::to_value(request("req-1")).expect("json"),
    )
    .expect("encrypt");
    let event = EventBuilder::new(Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16), encrypted)
        .tags([
            Tag::parse(["p", &requester.public_key().to_hex()]).expect("p tag"),
            Tag::parse([OBSERVER_AGENT_TAG, &requester.public_key().to_hex()]).expect("agent tag"),
            Tag::parse([OBSERVER_FRAME_TAG, OBSERVER_FRAME_CONTROL]).expect("frame tag"),
        ])
        .custom_created_at(Timestamp::from(NOW as u64))
        .sign_with_keys(&owner)
        .expect("sign");

    assert_eq!(
        verify_frame(&owner, &event, RELAY, NOW).unwrap_err(),
        FrameRejection::NotBrokerTraffic
    );
}

#[tokio::test]
async fn an_ignored_frame_never_reaches_a_handler_or_publishes() {
    let harness = Harness::new();
    let publisher = CapturingPublisher::new();
    let event = frame(
        &harness.requester,
        &harness.owner,
        &serde_json::json!({"kind": "turn_started"}),
        NOW,
    );

    let handled = harness.handle(&event, &publisher).await;
    assert_eq!(rejection_of(&handled), &FrameRejection::NotBrokerTraffic);
    assert_eq!(harness.handler.calls(), 0);
    assert!(publisher.frames().is_empty());
}

/// A malformed *broker* request must be answered — the requester needs to hear
/// `invalid_request` — while never reaching a capability.
#[tokio::test]
async fn a_malformed_broker_request_is_answered_but_not_executed() {
    let harness = Harness::new();
    let publisher = CapturingPublisher::new();
    let event = frame(
        &harness.requester,
        &harness.owner,
        &serde_json::json!({"type": "broker_request", "requestId": "req-1"}),
        NOW,
    );

    let handled = harness.handle(&event, &publisher).await;
    assert_eq!(
        response_of(&handled).result.error().map(|error| error.code),
        Some(BrokerErrorCode::InvalidRequest)
    );
    assert_eq!(harness.handler.calls(), 0);
    assert_eq!(
        publisher.frames().len(),
        1,
        "the refusal is still delivered"
    );
}

// ── result delivery ─────────────────────────────────────────────────────────

/// The requester must be able to read its own result: the frame is a control
/// frame encrypted to the requester and signed by the owner.
#[tokio::test]
async fn the_result_frame_is_readable_by_the_requester_and_signed_by_the_owner() {
    let harness = Harness::new();
    let publisher = CapturingPublisher::new();
    let event = request_frame(&harness.requester, &harness.owner, &request("req-1"));

    harness.handle(&event, &publisher).await;
    let frames = publisher.frames();
    assert_eq!(frames.len(), 1);
    let result_frame = &frames[0];

    assert_eq!(result_frame.pubkey, harness.owner.public_key());
    assert!(result_frame.verify_signature());
    assert_eq!(
        result_frame.kind,
        Kind::Custom(KIND_AGENT_OBSERVER_FRAME as u16)
    );
    assert_eq!(
        single_tag(result_frame, OBSERVER_FRAME_TAG).as_deref(),
        Some(OBSERVER_FRAME_CONTROL),
        "an owner-signed result must travel as a control frame"
    );
    assert_eq!(
        single_pubkey_tag(result_frame, "p"),
        Some(harness.requester.public_key())
    );

    let decrypted: BrokerResponse =
        buzz_core_pkg::observer::decrypt_observer_payload(&harness.requester, result_frame)
            .expect("requester can read its result");
    decrypted.validate().expect("a valid result envelope");
    assert_eq!(decrypted.request_id, "req-1");
    assert!(decrypted.result.is_succeeded());
}

/// The published result carries no secret material — the create outcome names
/// the new agent's public key only.
#[tokio::test]
async fn a_result_frame_carries_no_secret_material() {
    let harness = Harness::new();
    let publisher = CapturingPublisher::new();
    let event = request_frame(&harness.requester, &harness.owner, &request("req-1"));

    harness.handle(&event, &publisher).await;
    let decrypted: serde_json::Value = buzz_core_pkg::observer::decrypt_observer_payload(
        &harness.requester,
        &publisher.frames()[0],
    )
    .expect("decrypt");
    let json = serde_json::to_string(&decrypted).expect("json");
    for forbidden in ["nsec", "privateKey", "private_key", "secret"] {
        assert!(
            !json.contains(forbidden),
            "result leaked {forbidden}: {json}"
        );
    }
}

/// A delivery failure must not be reported as an execution failure, and must not
/// undo the recorded outcome: the retry replays the same success.
#[tokio::test]
async fn a_delivery_failure_leaves_the_recorded_outcome_intact() {
    let harness = Harness::new();
    let failing = CapturingPublisher::failing("relay unreachable");
    let request = request("req-1");
    let event = request_frame(&harness.requester, &harness.owner, &request);

    let handled = harness.handle(&event, &failing).await;
    match &handled {
        Handled::Executed { response, delivery } => {
            assert!(response.result.is_succeeded());
            assert_eq!(delivery.as_ref().unwrap_err(), "relay unreachable");
        }
        Handled::Ignored(rejection) => panic!("expected execution, got {rejection:?}"),
    }

    let ok = CapturingPublisher::new();
    let retry = frame(
        &harness.requester,
        &harness.owner,
        &serde_json::to_value(&request).expect("json"),
        NOW + 5,
    );
    let handled = harness.handle(&retry, &ok).await;
    let response = response_of(&handled);
    assert!(response.result.is_succeeded());
    assert!(response.replayed);
    assert_eq!(
        harness.handler.calls(),
        1,
        "a failed delivery must not cause a re-execution"
    );
}

#[test]
fn a_result_frame_for_a_malformed_requester_is_an_error_not_a_panic() {
    let owner = Keys::generate();
    let response = BrokerResponse::new(
        "req-1",
        BrokerResult::succeeded(CapabilityOutcome::AgentsCreate(AgentsCreateOutcome {
            agent_pubkey: "44".repeat(32),
            display_name: "Helper".into(),
            channel_id: CHANNEL.into(),
        })),
    );
    assert!(build_result_frame(&owner, "not-a-pubkey", &response).is_err());
}
