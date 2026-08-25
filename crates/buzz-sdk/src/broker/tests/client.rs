//! Retry byte-identity and the client trait's dispatch guarantees.

use super::*;

// ── Retry is identical bytes ────────────────────────────────────────────────

/// The retry contract is byte identity, so the client takes frozen bytes rather
/// than a typed value it would have to reserialize. Preparing once and reading
/// `body()` twice is the only way to send the same request twice.
#[test]
fn preparing_a_request_freezes_the_bytes_every_attempt_sends() {
    let request = BrokerRequest::new(
        "req-idem",
        ActionArgs::MessagePost(MessagePostArgs {
            channel_id: CHANNEL.into(),
            content: "exactly once".into(),
            mentions: vec![pubkey()],
        }),
    )
    .unwrap();
    let prepared = request.clone().prepare().expect("valid request prepares");

    assert_eq!(
        prepared.body(),
        prepared.body(),
        "body is frozen, not re-rendered"
    );
    // Correlation metadata is all a transport gets. There is deliberately no
    // accessor for the typed request: one would let an implementation serialize
    // the value a second time, which is the possibility freezing removes.
    assert_eq!(prepared.request_id(), "req-idem");
    assert_eq!(prepared.action(), Action::MessagePost);

    // The frozen bytes are the envelope, and they parse back to the same value.
    let parsed: BrokerRequest =
        serde_json::from_slice(prepared.body()).expect("frozen body is the envelope");
    assert_eq!(parsed, request);

    // Preparing validates, so an invalid request never reaches a transport.
    let invalid = BrokerRequest {
        r#type: BROKER_REQUEST_TYPE.into(),
        protocol_version: 99,
        request_id: "req-bad".into(),
        action_version: 1,
        action: ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Pubkey(pubkey()),
        }),
    };
    assert!(invalid.prepare().is_err());
}

/// The hand-written [`BrokerErrorCode::as_str`] and serde's derived name are two
/// encodings of one wire string, so each is pinned against the other and the
/// whole set is pinned against this literal — a rename in either fails here.
/// This is also what pins [`all_error_codes`] against the enum: a new variant
/// missing from that fixture changes the joined string and fails here.
#[test]
fn error_codes_have_stable_wire_strings() {
    let codes = all_error_codes();
    for code in codes {
        assert_eq!(
            serde_json::to_value(code).unwrap(),
            serde_json::json!(code.as_str()),
            "as_str and the serde name must not drift"
        );
    }
    assert_eq!(
        codes.map(BrokerErrorCode::as_str).join(","),
        "invalid_request,unsupported_protocol_version,unknown_action,\
unsupported_action_version,unsupported,unauthenticated,unauthorized,\
request_id_conflict,action_failed,outcome_unknown,internal"
    );
}

// ── Client trait ────────────────────────────────────────────────────────────

/// A test double, and the only implementation in this crate. It exists to prove
/// the trait is object-safe and usable behind `dyn`, which is what lets an
/// in-process host and an HTTP client be interchangeable.
///
/// Note what it does *not* do: it never calls `validate_for`. It cannot — it has
/// no way to build a [`ValidatedResponse`] except through the blanket
/// [`BrokerClientExt::execute`], which is the whole point of splitting the
/// trait. A deliberately hostile implementation is still forced through the
/// same check.
struct DoubleBroker {
    response: Result<BrokerResponse, BrokerTransportError>,
}

impl BrokerClient for DoubleBroker {
    fn send<'a>(&'a self, request: &'a PreparedRequest, _: Dispatch) -> BrokerFuture<'a> {
        // A real implementation sends `request.body()` verbatim. The double
        // stands in for a host that answers under the id it was asked with, and
        // returns the envelope unjudged.
        let response = self.response.clone().map(|mut response| {
            response.request_id = request.request_id().to_string();
            response
        });
        Box::pin(async move { response })
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    // A hand-rolled park-free executor: the double's future is always ready, so
    // one poll suffices and pulling in a runtime would be the heavier choice.
    use std::task::{Context, Poll, Wake, Waker};
    struct NoopWake;
    impl Wake for NoopWake {
        fn wake(self: std::sync::Arc<Self>) {}
    }
    let waker = Waker::from(std::sync::Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("test double must not park"),
    }
}

#[test]
fn the_client_trait_is_object_safe_and_returns_a_validated_host_verdict() {
    let request = prepared(ActionArgs::ChannelRead(ChannelReadArgs {
        channel_id: CHANNEL.into(),
        mentions_only: true,
        ..ChannelReadArgs::default()
    }));

    let succeeded: Box<dyn BrokerClient> = Box::new(DoubleBroker {
        response: Ok(BrokerResponse::new(
            "placeholder",
            BrokerResult::succeeded(ActionOutcome::ChannelRead(MessagePage {
                messages: vec![],
                next_cursor: None,
            })),
        )),
    });
    // `execute` is available on `dyn BrokerClient` and is the only way to a
    // `ValidatedResponse` — the caller does no correlation of its own.
    let response = block_on(succeeded.execute(&request)).expect("double answers");
    assert_eq!(response.request_id(), "req-1");
    assert!(response.result().outcome().is_some());
    assert!(!response.replayed());

    // A refusal — including a rejected credential — is still an answer: `Ok`
    // with the verdict in the envelope.
    for code in [
        BrokerErrorCode::Unauthorized,
        BrokerErrorCode::Unauthenticated,
    ] {
        let refused: Box<dyn BrokerClient> = Box::new(DoubleBroker {
            response: Ok(BrokerResponse::new(
                "placeholder",
                BrokerResult::failed(BrokerError::new(code, "no")),
            )),
        });
        let response =
            block_on(refused.execute(&request)).expect("a refusal is not a transport error");
        assert_eq!(response.result().error().map(|e| e.code), Some(code));
    }

    // No usable answer at all is a transport error, and says nothing about side
    // effects. An intermediary's status is operator detail, not a verdict.
    for error in [
        BrokerTransportError::Unreachable("connection reset".into()),
        BrokerTransportError::NoEnvelope {
            status: 401,
            detail: "proxy denied".into(),
        },
        BrokerTransportError::MalformedResponse("not json".into()),
    ] {
        let broken: Box<dyn BrokerClient> = Box::new(DoubleBroker {
            response: Err(error.clone()),
        });
        assert_eq!(block_on(broken.execute(&request)).unwrap_err(), error);
    }
}

/// The double returns whatever it is given, unvalidated — a hostile client
/// cannot do otherwise. `execute` is still the only door, so the mismatch
/// surfaces as a transport failure and never reaches a caller as `Ok`.
#[test]
fn a_client_cannot_hand_back_a_response_that_answers_a_different_request() {
    let request = prepared(ActionArgs::ChannelRead(ChannelReadArgs::channel(CHANNEL)));

    // Wrong action for this request.
    let confused: Box<dyn BrokerClient> = Box::new(DoubleBroker {
        response: Ok(BrokerResponse::new(
            "placeholder",
            BrokerResult::succeeded(ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
                agent_pubkey: pubkey(),
                display_name: "Gone".into(),
            })),
        )),
    });
    // The envelope is well-formed in isolation — that is exactly why `send`
    // cannot be the caller's door. `execute` is the only reachable one (a
    // `Dispatch` token cannot be built outside the client module), and it
    // rejects the mismatch rather than passing it on.
    assert!(matches!(
        block_on(confused.execute(&request)).unwrap_err(),
        BrokerTransportError::MalformedResponse(_)
    ));

    // Malformed identifiers inside an otherwise well-shaped outcome, too.
    let bad_cursor: Box<dyn BrokerClient> = Box::new(DoubleBroker {
        response: Ok(BrokerResponse::new(
            "placeholder",
            BrokerResult::succeeded(ActionOutcome::ChannelRead(MessagePage {
                messages: vec![],
                next_cursor: Some("not a cursor".into()),
            })),
        )),
    });
    assert!(matches!(
        block_on(bad_cursor.execute(&request)).unwrap_err(),
        BrokerTransportError::MalformedResponse(_)
    ));

    // A status contradicting its own code, which is how the review reached this:
    // `unauthenticated` is a known pre-dispatch refusal, so claiming not to know
    // the fate is not a verdict `execute` may pass on as `Ok`.
    let contradictory: Box<dyn BrokerClient> = Box::new(DoubleBroker {
        response: Ok(BrokerResponse::new(
            "placeholder",
            BrokerResult::indeterminate(BrokerError::new(
                BrokerErrorCode::Unauthenticated,
                "credential rejected",
            )),
        )),
    });
    assert!(matches!(
        block_on(contradictory.execute(&request)).unwrap_err(),
        BrokerTransportError::MalformedResponse(_)
    ));
}

/// A second double, parsing bytes the way a real HTTP client does, because the
/// strict-envelope and strict-event guards live in `Deserialize` and the typed
/// double above can never exercise them: it hands back a value that was never on
/// a wire.
///
/// This is the shape the bug actually had — bytes arriving from a host — and what
/// the caller sees now is [`BrokerTransportError::MalformedResponse`], not an
/// `Ok` whose extra members were quietly dropped.
struct WireBroker {
    body: Vec<u8>,
}

impl BrokerClient for WireBroker {
    fn send<'a>(&'a self, _: &'a PreparedRequest, _: Dispatch) -> BrokerFuture<'a> {
        // Exactly a transport's job: parse an envelope, and report the absence
        // of one as a transport failure.
        let parsed = serde_json::from_slice::<BrokerResponse>(&self.body)
            .map_err(|e| BrokerTransportError::MalformedResponse(e.to_string()));
        Box::pin(async move { parsed })
    }
}

#[test]
fn bytes_carrying_more_than_the_contract_declares_never_reach_a_caller_as_ok() {
    let signer = Keys::generate();
    let request = prepared(ActionArgs::ChannelRead(ChannelReadArgs::channel(CHANNEL)));
    let event = serde_json::to_value(signed_message(&signer)).expect("event serializes");
    let envelope = || {
        serde_json::json!({
            "type": BROKER_RESULT_TYPE,
            "protocolVersion": 1,
            "requestId": request.request_id(),
            "status": "succeeded",
            "action": "channel.read",
            "outcome": { "messages": [event.clone()] },
        })
    };

    // The honest bytes are accepted, so the rejections below are about the
    // smuggled members and not about this fixture being unparseable.
    let client = WireBroker {
        body: serde_json::to_vec(&envelope()).unwrap(),
    };
    let response = block_on(client.execute(&request)).expect("honest bytes are an answer");
    assert!(response.result().outcome().is_some());

    // A key at each depth: on the envelope, inside the outcome, and inside the
    // signed event — the last being the one `nostr` would have discarded.
    let mut on_envelope = envelope();
    on_envelope["secretKey"] = serde_json::json!("nsec1deadbeef");
    let mut in_outcome = envelope();
    in_outcome["outcome"]["secretKey"] = serde_json::json!("nsec1deadbeef");
    let mut in_event = envelope();
    in_event["outcome"]["messages"][0]["secretKey"] = serde_json::json!("nsec1deadbeef");
    // And the contradiction the envelope could previously hold on the wire.
    let mut error_beside_success = envelope();
    error_beside_success["error"] = serde_json::json!({ "code": "internal", "message": "?" });

    for (what, json) in [
        ("on the envelope", on_envelope),
        ("inside the outcome", in_outcome),
        ("inside the event", in_event),
        ("an error beside a success", error_beside_success),
    ] {
        let client = WireBroker {
            body: serde_json::to_vec(&json).unwrap(),
        };
        assert!(
            matches!(
                block_on(client.execute(&request)),
                Err(BrokerTransportError::MalformedResponse(_))
            ),
            "{what}: must not reach the caller as Ok"
        );
    }
}
