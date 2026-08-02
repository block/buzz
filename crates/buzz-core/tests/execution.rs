use buzz_core::execution::{
    AgentWorkloadContext, CredentialRef, ExecutionCapability, ExecutionCommand,
    ExecutionCommandEnvelope, ExecutionNodeAttestation, ExecutionNodeId, ExecutionNodeLifecycle,
    ExecutionNodeStatus, ExecutionReceipt, ExecutionValidationError, ProviderAuthResponse,
    ProviderAuthSession, ReceiptDetail, ReceiptOutcome, SafeErrorCode, WorkloadId,
    WorkloadLifecycle, WorkloadSpec, WorkloadStatus,
};
use chrono::{Duration, TimeZone, Utc};
use nostr::ToBech32;
use serde_json::json;

fn node_id() -> ExecutionNodeId {
    ExecutionNodeId::new("A".repeat(64)).expect("valid node id")
}

#[test]
fn execution_node_attestation_binds_owner_node_and_relay() {
    let owner = nostr::Keys::generate();
    let node = node_id();
    let attestation =
        ExecutionNodeAttestation::sign(&owner, &node, "relay.example").expect("attestation");

    attestation
        .verify(&node, "relay.example", Some(&owner.public_key().to_hex()))
        .expect("valid attestation");
    assert!(attestation.verify(&node, "other.example", None).is_err());
    assert!(attestation
        .verify(
            &node,
            "relay.example",
            Some(&nostr::Keys::generate().public_key().to_hex())
        )
        .is_err());
}

#[test]
fn managed_workload_identity_is_stable_for_repeated_deploys() {
    let agent = nostr::Keys::generate();
    let first = WorkloadId::stable_for_agent(&agent.public_key().to_hex()).expect("workload id");
    let second = WorkloadId::stable_for_agent(&agent.public_key().to_hex()).expect("workload id");

    assert_eq!(first, second);
}

#[test]
fn managed_workload_round_trips_agent_identity_and_context() {
    let owner = nostr::Keys::generate();
    let mut workload = workload();
    let agent_context = AgentWorkloadContext::new(
        owner.public_key().to_hex(),
        Some("You are a careful coding agent.".into()),
        Some("wss://relay.example/community".into()),
        Some("[\"oa\",\"owner\"]".into()),
        Some("allowlist".into()),
        vec![owner.public_key().to_hex()],
        Some("channel-123".into()),
    )
    .expect("valid agent context")
    .with_private_key(owner.secret_key().to_bech32().expect("nsec"))
    .expect("valid launch key");
    workload.agent = Some(agent_context.clone());

    let encoded = serde_json::to_value(&workload).expect("serialize workload");
    assert_eq!(encoded["agent"]["pubkey"], owner.public_key().to_hex());
    assert_eq!(
        encoded["agent"]["systemPrompt"],
        "You are a careful coding agent."
    );
    assert_eq!(encoded["agent"]["channelId"], "channel-123");
    assert!(encoded["agent"].get("privateKey").is_none());
    assert_eq!(
        encoded["agent"]["privateKeyNsec"],
        owner.secret_key().to_bech32().expect("nsec")
    );

    let decoded: WorkloadSpec = serde_json::from_value(encoded).expect("decode workload");
    assert_eq!(decoded, workload);
    assert_eq!(
        agent_context.clone().without_private_key().private_key_nsec,
        None
    );
    let debug = format!("{agent_context:?}");
    assert!(!debug.contains(&owner.secret_key().to_bech32().expect("nsec")));
}

fn workload_id() -> WorkloadId {
    WorkloadId::new("123e4567-e89b-12d3-a456-426614174000").expect("valid workload id")
}

fn workload() -> WorkloadSpec {
    WorkloadSpec::agent(
        workload_id(),
        "Research agent",
        "goose",
        Some("claude-sonnet".to_string()),
        Some("anthropic".to_string()),
        vec![CredentialRef::new("anthropic", "primary").expect("valid credential reference")],
    )
    .expect("valid workload")
}

#[test]
fn command_envelope_round_trips_and_carries_only_credential_references() {
    let issued_at = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
    let envelope = ExecutionCommandEnvelope::new(
        node_id(),
        issued_at,
        issued_at + Duration::minutes(5),
        ExecutionCommand::Deploy {
            workload: workload(),
        },
    )
    .expect("valid command");

    envelope
        .validate_at(issued_at + Duration::seconds(1))
        .unwrap();
    let encoded = serde_json::to_value(&envelope).unwrap();
    assert_eq!(encoded["command"]["operation"], "deploy");
    assert_eq!(
        encoded["command"]["workload"]["credentialRefs"][0]["provider"],
        "anthropic"
    );
    assert!(encoded["command"]["workload"].get("secret").is_none());
    assert!(encoded["command"]["workload"].get("privateKey").is_none());

    let decoded: ExecutionCommandEnvelope = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.node_id(), &node_id());
}

#[test]
fn envelope_rejects_expiry_and_malformed_payloads() {
    let issued_at = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
    let envelope = ExecutionCommandEnvelope::new(
        node_id(),
        issued_at,
        issued_at + Duration::minutes(5),
        ExecutionCommand::Start {
            workload_id: workload_id(),
        },
    )
    .unwrap();

    assert_eq!(
        envelope.validate_at(issued_at + Duration::minutes(5)),
        Err(ExecutionValidationError::Expired)
    );

    let malformed = json!({
        "protocolVersion": 1,
        "commandId": "123e4567-e89b-12d3-a456-426614174000",
        "requestId": "123e4567-e89b-12d3-a456-426614174001",
        "nodeId": "not-a-pubkey",
        "issuedAt": issued_at,
        "expiresAt": issued_at + Duration::minutes(5),
        "command": { "operation": "start", "workloadId": workload_id() }
    });
    assert!(serde_json::from_value::<ExecutionCommandEnvelope>(malformed).is_err());

    let valid_deploy = ExecutionCommandEnvelope::new(
        node_id(),
        issued_at,
        issued_at + Duration::minutes(5),
        ExecutionCommand::Deploy {
            workload: workload(),
        },
    )
    .unwrap();
    let mut unsafe_payload = serde_json::to_value(valid_deploy).unwrap();
    unsafe_payload["command"]["workload"]["displayName"] = json!("safe\0name");
    let parsed: ExecutionCommandEnvelope = serde_json::from_value(unsafe_payload).unwrap();
    assert_eq!(
        parsed.validate_at(issued_at + Duration::seconds(1)),
        Err(ExecutionValidationError::NulByte {
            field: "workload display name"
        })
    );

    let unsafe_json = serde_json::to_string(&parsed).unwrap();
    assert!(
        ExecutionCommandEnvelope::from_json_at(&unsafe_json, issued_at + Duration::seconds(1))
            .is_err()
    );

    let mut secret_payload = serde_json::to_value(parsed).unwrap();
    secret_payload["command"]["workload"]["secret"] = json!("must-not-cross-protocol");
    assert!(serde_json::from_value::<ExecutionCommandEnvelope>(secret_payload).is_err());

    let unsafe_workload = WorkloadSpec::agent(
        workload_id(),
        "safe\0name",
        "goose",
        None::<String>,
        None::<String>,
        Vec::new(),
    );
    assert!(matches!(
        unsafe_workload,
        Err(ExecutionValidationError::NulByte { .. })
    ));
}

#[test]
fn commands_cover_lifecycle_and_provider_authentication_without_secrets() {
    let auth = ProviderAuthSession::new(
        workload_id(),
        "anthropic",
        "login-session",
        Utc::now() + Duration::minutes(10),
    )
    .unwrap();
    let commands = [
        ExecutionCommand::Deploy {
            workload: workload(),
        },
        ExecutionCommand::Start {
            workload_id: workload_id(),
        },
        ExecutionCommand::Stop {
            workload_id: workload_id(),
        },
        ExecutionCommand::Restart {
            workload_id: workload_id(),
        },
        ExecutionCommand::Remove {
            workload_id: workload_id(),
        },
        ExecutionCommand::AuthenticateProvider { session: auth },
        ExecutionCommand::SubmitProviderAuthentication {
            response: ProviderAuthResponse::new(
                workload_id(),
                "login-session",
                "encrypted-provider-response",
            )
            .unwrap(),
        },
        ExecutionCommand::CancelProviderAuthentication {
            workload_id: workload_id(),
            session_id: "login-session".into(),
        },
    ];

    for command in commands {
        command.validate_at(Utc::now()).unwrap();
        let value = serde_json::to_value(command).unwrap();
        assert!(value.get("privateKey").is_none());
        assert!(value.get("secret").is_none());
        assert!(value.get("token").is_none());
    }

    assert!(matches!(
        ProviderAuthResponse::new(workload_id(), "login-session", ""),
        Err(ExecutionValidationError::EmptyAuthenticationResponse)
    ));
    assert!(matches!(
        ProviderAuthResponse::new(workload_id(), "login-session", "x".repeat(16 * 1024 + 1)),
        Err(ExecutionValidationError::TooLong {
            field: "provider auth response",
            ..
        })
    ));
}

#[test]
fn receipts_correlate_and_enforce_sequence_and_terminal_outcomes() {
    let issued_at = Utc.with_ymd_and_hms(2026, 8, 1, 12, 0, 0).unwrap();
    let command = ExecutionCommandEnvelope::new(
        node_id(),
        issued_at,
        issued_at + Duration::minutes(5),
        ExecutionCommand::Start {
            workload_id: workload_id(),
        },
    )
    .unwrap();
    let correlated =
        ExecutionReceipt::for_command(&command, workload_id(), 1, ReceiptOutcome::Accepted)
            .unwrap();
    assert_eq!(correlated.request_id, command.request_id);
    assert_eq!(correlated.command_id, command.command_id);

    let receipt = ExecutionReceipt::for_command(
        &command,
        command.command.workload_id().clone(),
        2,
        ReceiptOutcome::Failed {
            error: SafeErrorCode::RuntimeUnavailable,
        },
    )
    .unwrap();

    receipt.validate().unwrap();
    receipt.validate_after(1).unwrap();
    assert!(receipt.is_terminal());
    assert_eq!(receipt.sequence(), 2);

    let invalid = ExecutionReceipt::for_command(
        &command,
        command.command.workload_id().clone(),
        0,
        ReceiptOutcome::Progress,
    );
    assert_eq!(invalid, Err(ExecutionValidationError::InvalidSequence));
    let mismatch = ExecutionReceipt::for_command(
        &command,
        WorkloadId::new("123e4567-e89b-12d3-a456-426614174002").unwrap(),
        1,
        ReceiptOutcome::Accepted,
    );
    assert_eq!(mismatch, Err(ExecutionValidationError::WorkloadMismatch));
    let out_of_order = ExecutionReceipt::for_command(
        &command,
        command.command.workload_id().clone(),
        1,
        ReceiptOutcome::Progress,
    )
    .unwrap();
    assert_eq!(
        out_of_order.validate_after(1),
        Err(ExecutionValidationError::InvalidSequenceOrder {
            previous: 1,
            current: 1
        })
    );

    let detailed = ExecutionReceipt::for_command_with_detail(
        &command,
        workload_id(),
        3,
        ReceiptOutcome::Progress,
        Some(ReceiptDetail::ProviderAuthChallenge {
            provider: "anthropic".into(),
            session_id: "login-session".into(),
            instructions: "finish login".into(),
        }),
    )
    .unwrap();
    let mut legacy = serde_json::to_value(detailed).unwrap();
    assert_eq!(legacy["detail"]["detail"], "provider_auth_challenge");
    legacy.as_object_mut().unwrap().remove("detail");
    let decoded: ExecutionReceipt = serde_json::from_value(legacy).unwrap();
    assert_eq!(decoded.detail, None);
}

#[test]
fn status_projection_is_runtime_neutral_and_explicitly_capability_scoped() {
    let status = ExecutionNodeStatus::new(
        node_id(),
        "Example execution node",
        ExecutionNodeLifecycle::Ready,
        [
            ExecutionCapability::Deploy,
            ExecutionCapability::Start,
            ExecutionCapability::Stop,
            ExecutionCapability::Restart,
            ExecutionCapability::Remove,
            ExecutionCapability::ProviderAuthentication,
        ],
    )
    .unwrap();

    assert_eq!(status.capabilities().len(), 6);
    assert_eq!(status.workloads(), &[]);
    let status = status
        .with_workloads(vec![WorkloadStatus::new(
            workload_id(),
            WorkloadLifecycle::Running,
            1,
        )
        .unwrap()])
        .unwrap();
    status.validate().unwrap();
    assert_eq!(status.workloads().len(), 1);
    assert!(!WorkloadLifecycle::Running.is_terminal());
    assert!(serde_json::to_value(status)
        .unwrap()
        .get("docker")
        .is_none());
}
