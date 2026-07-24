use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::json;

use super::command_brief::{
    build_command_brief_event, decrypt_command_brief_event, CommandBriefEventPayload,
    CommandBriefFailure, CommandBriefFailureCode, CommandBriefLifecycleState, CommandBriefWire,
    COMMAND_BRIEF_PAYLOAD_VERSION,
};

const ADVISORY_LIMITATION: &str = "This Daily Command Brief is advisory only. Navigation content identifies considerations and source limitations; it does not generate executable navigation orders or make navigational decisions.";

fn finding() -> serde_json::Value {
    json!({
        "classification": "OFFICIAL",
        "text": "A supported specialist finding.",
        "sourceIds": ["ledger-1"]
    })
}

fn contribution(adviser: &str, section: &str) -> serde_json::Value {
    json!({
        "classification": "OFFICIAL",
        "adviser": adviser,
        "section": section,
        "findings": [finding()],
        "confidence": 0.85,
        "limitations": ["The source is bounded to the frozen snapshot."],
        "dissent": [],
        "proposedActions": []
    })
}

fn command_brief(run_id: &str) -> serde_json::Value {
    json!({
        "version": 1,
        "classification": "OFFICIAL",
        "generatedAt": "2026-07-25T06:00:00Z",
        "runId": run_id,
        "scheduleId": "daily",
        "snapshotId": "snapshot-1",
        "sections": {
            "today": [finding()],
            "operations": [],
            "navigation": [],
            "daily_routine": [],
            "reports": [],
            "planning_30_60_90": [],
            "decisions": [],
            "conflicts_and_gaps": [],
            "sources": []
        },
        "degradedSections": [],
        "missingInformation": [],
        "dissent": [],
        "sourceLedger": [{
            "classification": "OFFICIAL",
            "ledgerId": "ledger-1",
            "sourceId": "source-1",
            "sourceKind": "rag",
            "collection": "engineering-orders",
            "documentId": "document-1",
            "chunkId": "chunk-1",
            "timestamp": "2026-07-25T06:00:00Z",
            "snapshotId": "snapshot-1",
            "quotedLocation": {"quote": "A supported quote.", "location": "section 1"},
            "retrievedAt": "2026-07-25T06:00:00Z",
            "observedAt": "2026-07-25T06:00:00Z"
        }],
        "sourceFreshness": {
            "classification": "OFFICIAL",
            "asOf": "2026-07-25T06:00:00Z",
            "staleSourceIds": []
        },
        "contributions": [
            contribution("operations", "operations"),
            contribution("navigation", "navigation"),
            contribution("daily_routine", "daily_routine"),
            contribution("reporting", "reports"),
            contribution("plans", "planning_30_60_90")
        ],
        "advisoryLimitation": ADVISORY_LIMITATION
    })
}

fn payload(state: CommandBriefLifecycleState) -> CommandBriefEventPayload {
    let successful = matches!(
        state,
        CommandBriefLifecycleState::Completed | CommandBriefLifecycleState::Degraded
    );
    CommandBriefEventPayload {
        version: COMMAND_BRIEF_PAYLOAD_VERSION,
        classification: "OFFICIAL".to_string(),
        run_id: "run-1".to_string(),
        schedule_id: "daily".to_string(),
        lifecycle_state: state,
        occurred_at: "2026-07-25T06:00:00Z".to_string(),
        frozen_snapshot_id: "snapshot-1".to_string(),
        final_brief: successful
            .then(|| CommandBriefWire::try_from(command_brief("run-1")).expect("valid brief")),
        failure: (!successful).then_some(CommandBriefFailure {
            code: CommandBriefFailureCode::BriefGenerationFailed,
        }),
        previous_lifecycle_event_id: None,
    }
}

#[test]
fn owner_to_self_round_trip_has_exact_public_envelope() {
    let owner = Keys::generate();
    let expected = payload(CommandBriefLifecycleState::Completed);
    let event = build_command_brief_event(&owner, &expected).expect("build");

    assert_eq!(event.kind, Kind::Custom(44_210));
    let tags: Vec<Vec<String>> = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    assert_eq!(
        tags,
        vec![
            vec!["p".into(), owner.public_key().to_hex()],
            vec!["d".into(), "run-1".into()],
            vec!["status".into(), "completed".into()],
        ]
    );
    assert!(
        decrypt_command_brief_event(&owner, &event).expect("decrypt") == expected,
        "decrypted payload must equal the input"
    );
}

#[test]
fn wrong_identity_and_public_envelope_tampering_fail_closed() {
    let owner = Keys::generate();
    let wrong = Keys::generate();
    let expected = payload(CommandBriefLifecycleState::Degraded);
    let event = build_command_brief_event(&owner, &expected).expect("build");
    assert!(decrypt_command_brief_event(&wrong, &event).is_err());

    let tampered = EventBuilder::new(event.kind, event.content.clone())
        .tags([
            Tag::parse(["p", &owner.public_key().to_hex()]).expect("p"),
            Tag::parse(["d", "other-run"]).expect("d"),
            Tag::parse(["status", "degraded"]).expect("status"),
        ])
        .sign_with_keys(&owner)
        .expect("sign");
    assert!(decrypt_command_brief_event(&owner, &tampered).is_err());
}

#[test]
fn predecessor_is_exact_and_payload_never_accepts_sensitive_fields() {
    let owner = Keys::generate();
    let previous = "a".repeat(64);
    let mut expected = payload(CommandBriefLifecycleState::Completed);
    expected.previous_lifecycle_event_id = Some(previous.clone());
    let event = build_command_brief_event(&owner, &expected).expect("build");
    assert!(event
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["previous", previous.as_str()]));
}

#[test]
fn strict_final_brief_rejects_extras_bad_citations_and_sensitive_fields() {
    for forbidden in [
        "prompt",
        "reasoning",
        "credentials",
        "bearerToken",
        "provider",
        "token",
    ] {
        let mut unsafe_brief = command_brief("run-1");
        unsafe_brief[forbidden] = json!("secret");
        assert!(CommandBriefWire::try_from(unsafe_brief).is_err());
    }

    let mut bad_citation = command_brief("run-1");
    bad_citation["sections"]["today"][0]["sourceIds"] = json!(["missing-ledger"]);
    assert!(CommandBriefWire::try_from(bad_citation).is_err());

    let mut nested_extra = command_brief("run-1");
    nested_extra["sourceLedger"][0]["extra"] = json!(true);
    assert!(CommandBriefWire::try_from(nested_extra).is_err());

    assert!(CommandBriefWire::try_from(json!({
        "classification": "OFFICIAL",
        "summary": "arbitrary JSON is not the authoritative contract"
    }))
    .is_err());
}

#[test]
fn payload_bounds_and_terminal_shapes_are_enforced() {
    let owner = Keys::generate();
    let mut oversized = payload(CommandBriefLifecycleState::Completed);
    oversized.run_id = "x".repeat(257);
    assert!(build_command_brief_event(&owner, &oversized).is_err());

    let mut invalid_shape = payload(CommandBriefLifecycleState::Failed);
    invalid_shape.final_brief =
        Some(CommandBriefWire::try_from(command_brief("run-1")).expect("valid brief"));
    assert!(build_command_brief_event(&owner, &invalid_shape).is_err());

    let mut wrong_classification = payload(CommandBriefLifecycleState::Completed);
    wrong_classification.classification = "PUBLIC".to_string();
    assert!(build_command_brief_event(&owner, &wrong_classification).is_err());

    let mut large_but_structurally_valid = command_brief("run-1");
    large_but_structurally_valid["sourceLedger"] = serde_json::Value::Array(
        (0..256)
            .map(|index| {
                json!({
                    "classification": "OFFICIAL",
                    "ledgerId": format!("ledger-{index}"),
                    "sourceId": format!("source-{index}"),
                    "sourceKind": "rag",
                    "collection": "engineering-orders",
                    "documentId": format!("document-{index}"),
                    "chunkId": format!("chunk-{index}"),
                    "timestamp": "2026-07-25T06:00:00Z",
                    "snapshotId": "snapshot-1",
                    "quotedLocation": {
                        "quote": "x".repeat(4096),
                        "location": format!("section {index}")
                    },
                    "retrievedAt": "2026-07-25T06:00:00Z",
                    "observedAt": "2026-07-25T06:00:00Z"
                })
            })
            .collect(),
    );
    let mut oversized_ciphertext = payload(CommandBriefLifecycleState::Completed);
    oversized_ciphertext.final_brief = Some(
        CommandBriefWire::try_from(large_but_structurally_valid)
            .expect("wire shape remains structurally valid"),
    );
    assert!(
        build_command_brief_event(&owner, &oversized_ciphertext).is_err(),
        "the shared bound applies to final NIP-44 ciphertext, not only field shapes"
    );
}

#[test]
fn failure_codes_are_closed_and_reject_provider_or_token_text() {
    let mut value = serde_json::to_value(payload(CommandBriefLifecycleState::Failed))
        .expect("serialize payload");
    for code in [
        "unknown",
        "openai_500",
        "provider_timeout",
        "token_expired",
        "sk-secret",
    ] {
        value["failure"]["code"] = json!(code);
        assert!(
            serde_json::from_value::<CommandBriefEventPayload>(value.clone()).is_err(),
            "{code} must not enter the closed failure vocabulary"
        );
    }
}
