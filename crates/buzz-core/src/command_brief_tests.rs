use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::json;

use super::command_brief::{
    build_command_brief_event, decrypt_command_brief_event, CommandBriefEventPayload,
    CommandBriefFailure, CommandBriefLifecycleState, COMMAND_BRIEF_PAYLOAD_VERSION,
};

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
        final_brief: successful.then(|| json!({"classification":"OFFICIAL","summary":"ready"})),
        failure: (!successful).then(|| CommandBriefFailure {
            code: "brief_generation_failed".to_string(),
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

    for forbidden in ["prompt", "reasoning", "credentials", "bearerToken"] {
        let mut unsafe_payload = payload(CommandBriefLifecycleState::Completed);
        unsafe_payload.final_brief = Some(json!({forbidden: "secret"}));
        assert!(build_command_brief_event(&owner, &unsafe_payload).is_err());
    }
}

#[test]
fn payload_bounds_and_terminal_shapes_are_enforced() {
    let owner = Keys::generate();
    let mut oversized = payload(CommandBriefLifecycleState::Completed);
    oversized.run_id = "x".repeat(257);
    assert!(build_command_brief_event(&owner, &oversized).is_err());

    let mut invalid_shape = payload(CommandBriefLifecycleState::Failed);
    invalid_shape.final_brief = Some(json!({"classification":"OFFICIAL"}));
    assert!(build_command_brief_event(&owner, &invalid_shape).is_err());

    let mut wrong_classification = payload(CommandBriefLifecycleState::Completed);
    wrong_classification.classification = "PUBLIC".to_string();
    assert!(build_command_brief_event(&owner, &wrong_classification).is_err());
}
