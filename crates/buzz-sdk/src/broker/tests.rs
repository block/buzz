//! Wire-contract tests for the broker envelope.

use super::*;

const CHANNEL: &str = "b2c38ca8-9ec3-411e-bab5-f9deab34d52e";
const PUBKEY: &str = "a02c4e0850e5e612b4ddf95dbe2f5c56467cf27c6552203bc833ff438fb31971";

fn create_args() -> CapabilityArgs {
    CapabilityArgs::AgentsCreate(AgentsCreateArgs {
        channel_id: CHANNEL.into(),
        display_name: "Research helper".into(),
        system_prompt: "Find sources.".into(),
        runtime: None,
        provider: None,
        model: None,
        respond_to: None,
    })
}

#[test]
fn request_serializes_capability_beside_args() {
    let request = BrokerRequest::new("req-1", create_args()).unwrap();
    let json = serde_json::to_value(&request).unwrap();

    assert_eq!(json["type"], BROKER_REQUEST_TYPE);
    assert_eq!(json["protocolVersion"], 1);
    assert_eq!(json["requestId"], "req-1");
    assert_eq!(json["capability"], "agents.create");
    assert_eq!(json["capabilityVersion"], 1);
    assert_eq!(json["args"]["displayName"], "Research helper");

    // Unset optionals stay off the wire, so the payload cannot imply a
    // runtime/provider/model the caller never chose.
    assert!(json["args"].get("runtime").is_none());
    assert!(json["args"].get("respondTo").is_none());

    let parsed: BrokerRequest = serde_json::from_value(json).unwrap();
    assert_eq!(parsed, request);
}

/// The envelope must not carry owner/requester/relay identity: those are
/// transport-derived. A body that could name its own owner would let any
/// signer act on another owner's agents.
#[test]
fn request_has_no_caller_supplied_identity_or_context() {
    let json = serde_json::to_value(BrokerRequest::new("req-1", create_args()).unwrap()).unwrap();
    let object = json.as_object().unwrap();

    for forbidden in [
        "ownerPubkey",
        "owner",
        "requesterPubkey",
        "requester",
        "relayUrl",
        "relayScope",
        "context",
        "authorizer",
        "scope",
        "expiry",
        "authorization",
        "requestDigest",
        "digest",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "broker request must not carry \"{forbidden}\""
        );
    }

    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "args",
            "capability",
            "capabilityVersion",
            "protocolVersion",
            "requestId",
            "type",
        ]
    );
}

#[test]
fn unknown_request_field_is_rejected() {
    let mut json =
        serde_json::to_value(BrokerRequest::new("req-1", create_args()).unwrap()).unwrap();
    json.as_object_mut()
        .unwrap()
        .insert("ownerPubkey".into(), serde_json::json!(PUBKEY));

    let error = serde_json::from_value::<BrokerRequest>(json).unwrap_err();
    assert!(
        error.to_string().contains("ownerPubkey"),
        "unexpected error: {error}"
    );
}

/// A secret smuggled into args must fail to deserialize rather than reach a
/// mutation.
#[test]
fn secret_bearing_args_fail_to_deserialize() {
    let json = serde_json::json!({
        "type": BROKER_REQUEST_TYPE,
        "protocolVersion": 1,
        "requestId": "req-1",
        "capability": "agents.create",
        "capabilityVersion": 1,
        "args": {
            "channelId": CHANNEL,
            "displayName": "Sneaky",
            "systemPrompt": "hi",
            "envVars": { "ANTHROPIC_API_KEY": "sk-live" },
        },
    });
    assert!(serde_json::from_value::<BrokerRequest>(json).is_err());
}

#[test]
fn wrong_protocol_version_is_refused() {
    let mut request = BrokerRequest::new("req-1", create_args()).unwrap();
    request.protocol_version = 2;
    let error = request.validate().unwrap_err().to_string();
    assert!(error.contains("protocolVersion"), "unexpected: {error}");
}

#[test]
fn wrong_capability_version_is_refused() {
    let mut request = BrokerRequest::new("req-1", create_args()).unwrap();
    request.capability_version = 7;
    let error = request.validate().unwrap_err().to_string();
    assert!(error.contains("capabilityVersion"), "unexpected: {error}");
}

#[test]
fn unknown_capability_name_is_refused() {
    let json = serde_json::json!({
        "type": BROKER_REQUEST_TYPE,
        "protocolVersion": 1,
        "requestId": "req-1",
        "capability": "agents.exfiltrate",
        "capabilityVersion": 1,
        "args": {},
    });
    assert!(serde_json::from_value::<BrokerRequest>(json).is_err());
    assert!(Capability::parse("agents.exfiltrate").is_err());
}

/// Signing, publishing, and credential access are not capabilities. Only
/// business operations are addressable.
#[test]
fn only_business_capabilities_are_addressable() {
    for forbidden in [
        "sign",
        "publish",
        "keys.export",
        "identity.nsec",
        "tool.exec",
        "agents.manage",
        "agents.start",
    ] {
        assert!(
            Capability::parse(forbidden).is_err(),
            "\"{forbidden}\" must not be a capability"
        );
    }
    for allowed in ["agents.create", "agents.update", "agents.delete"] {
        assert_eq!(Capability::parse(allowed).unwrap().as_str(), allowed);
    }
}

#[test]
fn request_id_must_be_present_bounded_and_printable() {
    assert!(BrokerRequest::new("", create_args()).is_err());
    assert!(BrokerRequest::new("a".repeat(MAX_REQUEST_ID_LEN), create_args()).is_ok());
    assert!(BrokerRequest::new("a".repeat(MAX_REQUEST_ID_LEN + 1), create_args()).is_err());
    assert!(BrokerRequest::new("has space", create_args()).is_err());
    assert!(BrokerRequest::new("has\nnewline", create_args()).is_err());
}

#[test]
fn update_requires_at_least_one_change() {
    let empty = AgentsUpdateArgs {
        target: AgentTarget::Pubkey(PUBKEY.into()),
        display_name: None,
        system_prompt: None,
        runtime: None,
        provider: None,
        model: None,
        respond_to: None,
    };
    let error = empty.validated().unwrap_err().to_string();
    assert!(error.contains("at least one field"), "unexpected: {error}");
}

#[test]
fn target_pubkey_must_be_hex_and_normalizes_case() {
    assert!(AgentTarget::Pubkey("nothex".into()).validated().is_err());
    assert!(AgentTarget::Pubkey(PUBKEY[..40].into())
        .validated()
        .is_err());
    let upper = AgentTarget::Pubkey(PUBKEY.to_ascii_uppercase());
    assert_eq!(
        upper.validated().unwrap(),
        AgentTarget::Pubkey(PUBKEY.into())
    );
}

#[test]
fn create_rejects_a_malformed_channel_uuid() {
    let args = AgentsCreateArgs {
        channel_id: "not-a-uuid".into(),
        display_name: "A".into(),
        system_prompt: "B".into(),
        runtime: None,
        provider: None,
        model: None,
        respond_to: None,
    };
    let error = args.validated().unwrap_err().to_string();
    assert!(error.contains("channel"), "unexpected: {error}");
}

#[test]
fn create_rejects_an_unsupported_respond_to_mode() {
    let args = AgentsCreateArgs {
        channel_id: CHANNEL.into(),
        display_name: "A".into(),
        system_prompt: "B".into(),
        runtime: None,
        provider: None,
        model: None,
        respond_to: Some("allowlist".into()),
    };
    assert!(args.validated().is_err());
}

/// Update and delete carry no channel: per the approved design, channel
/// appears only where the operation needs it (create attachment).
#[test]
fn update_and_delete_carry_no_channel() {
    let update = serde_json::to_value(AgentsUpdateArgs {
        target: AgentTarget::Pubkey(PUBKEY.into()),
        display_name: Some("New".into()),
        system_prompt: None,
        runtime: None,
        provider: None,
        model: None,
        respond_to: None,
    })
    .unwrap();
    assert!(update.get("channelId").is_none());

    let delete = serde_json::to_value(AgentsDeleteArgs {
        target: AgentTarget::Pubkey(PUBKEY.into()),
    })
    .unwrap();
    assert!(delete.get("channelId").is_none());
}

// ── Results ─────────────────────────────────────────────────────────────────

#[test]
fn succeeded_result_carries_outcome_and_no_error() {
    let response = BrokerResponse::new(
        "req-1",
        BrokerResult::succeeded(CapabilityOutcome::AgentsCreate(AgentsCreateOutcome {
            agent_pubkey: PUBKEY.into(),
            display_name: "Research helper".into(),
            channel_id: CHANNEL.into(),
        })),
    );
    response.validate().unwrap();

    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["type"], BROKER_RESULT_TYPE);
    assert_eq!(json["status"], "succeeded");
    assert_eq!(json["capability"], "agents.create");
    assert_eq!(json["outcome"]["agentPubkey"], PUBKEY);
    assert!(json.get("error").is_none());
    // `replayed` is response metadata and stays off the wire when false.
    assert!(json.get("replayed").is_none());

    let parsed: BrokerResponse = serde_json::from_value(json).unwrap();
    assert_eq!(parsed, response);
    assert!(parsed.result.is_succeeded());
    assert!(parsed.result.error().is_none());
}

#[test]
fn failed_and_indeterminate_are_distinct_and_carry_no_outcome() {
    let failed = BrokerResult::failed(BrokerError::new(
        BrokerErrorCode::CapabilityFailed,
        "runtime not installed",
    ));
    let failed_json = serde_json::to_value(BrokerResponse::new("r", failed.clone())).unwrap();
    assert_eq!(failed_json["status"], "failed");
    assert_eq!(failed_json["error"]["code"], "capability_failed");
    assert!(failed_json.get("outcome").is_none());

    let indeterminate = BrokerResult::indeterminate(BrokerError::new(
        BrokerErrorCode::OutcomeUnknown,
        "host restarted mid-execution",
    ));
    let json = serde_json::to_value(BrokerResponse::new("r", indeterminate.clone())).unwrap();
    assert_eq!(json["status"], "indeterminate");
    assert_eq!(json["error"]["code"], "outcome_unknown");
    assert!(json.get("outcome").is_none());

    assert_ne!(failed, indeterminate);
    assert!(!indeterminate.is_succeeded());
}

#[test]
fn replay_metadata_rides_the_response_not_the_result() {
    let result = BrokerResult::succeeded(CapabilityOutcome::AgentsDelete(AgentsDeleteOutcome {
        agent_pubkey: PUBKEY.into(),
        display_name: "Gone".into(),
    }));
    let fresh = BrokerResponse::new("req-9", result.clone());
    let replayed = BrokerResponse::new("req-9", result.clone()).replayed();

    // The domain outcome is identical; only the delivery metadata differs.
    assert_eq!(fresh.result, replayed.result);
    assert!(!fresh.replayed);
    assert!(replayed.replayed);
    assert_eq!(
        serde_json::to_value(&replayed).unwrap()["replayed"],
        serde_json::json!(true)
    );

    // `replayed` is not part of the stored result encoding.
    let stored = serde_json::to_value(&result).unwrap();
    assert!(stored.get("replayed").is_none());
}

#[test]
fn every_error_code_has_a_stable_wire_string() {
    for (code, expected) in [
        (BrokerErrorCode::InvalidRequest, "invalid_request"),
        (
            BrokerErrorCode::UnsupportedProtocolVersion,
            "unsupported_protocol_version",
        ),
        (BrokerErrorCode::UnknownCapability, "unknown_capability"),
        (
            BrokerErrorCode::UnsupportedCapabilityVersion,
            "unsupported_capability_version",
        ),
        (BrokerErrorCode::Unauthenticated, "unauthenticated"),
        (BrokerErrorCode::Unauthorized, "unauthorized"),
        (BrokerErrorCode::RequestIdConflict, "request_id_conflict"),
        (BrokerErrorCode::CapabilityFailed, "capability_failed"),
        (BrokerErrorCode::OutcomeUnknown, "outcome_unknown"),
        (BrokerErrorCode::Internal, "internal"),
    ] {
        assert_eq!(code.as_str(), expected);
        assert_eq!(
            serde_json::to_value(code).unwrap(),
            serde_json::json!(expected)
        );
    }
}

/// An outcome type must be structurally unable to carry secret material.
#[test]
fn outcomes_cannot_carry_secrets() {
    let json = serde_json::json!({
        "capability": "agents.create",
        "outcome": {
            "agentPubkey": PUBKEY,
            "displayName": "A",
            "channelId": CHANNEL,
            "privateKeyNsec": "nsec1deadbeef",
        },
    });
    assert!(
        serde_json::from_value::<CapabilityOutcome>(json).is_err(),
        "an outcome carrying a secret must not deserialize"
    );
}
