use nostr::Keys;
use serde_json::json;

use crate::agent_experience::{
    build_experience_event, experience_projection_payload, from_engram_body, ExperienceOutcome,
    ExperienceRecordV1, MemoryScope, SkillVersionV1, ToolEvidenceV1, ValidationResultV1,
};
use crate::engram::{validate_and_decrypt, Body};

fn record() -> ExperienceRecordV1 {
    ExperienceRecordV1 {
        record_id: "018f7f6a-2f11-7a90-a4b5-9dd45c198501".into(),
        memory_key: "navigation.briefing-order".into(),
        scope: MemoryScope::CommandTeamShared,
        specialist_id: Some("navigation".into()),
        team_id: Some("command-team".into()),
        occurred_at: "2026-08-16T10:00:00Z".into(),
        task_summary: "Prepared the navigation brief.".into(),
        decision: Some("Brief pilotage before the command update.".into()),
        assumptions: vec!["Departure remains at 1000.".into()],
        dissent: vec![],
        limitations: vec!["Pilot availability is not confirmed.".into()],
        outcome: ExperienceOutcome::Succeeded,
        tool_evidence: vec![ToolEvidenceV1 {
            tool: "rag.search".into(),
            result_code: "ok".into(),
            summary: "Returned cited navigation doctrine.".into(),
        }],
        source_ids: vec!["rag:adf-doctrine:point-123".into()],
        model_identity: "gemma4-26b-official".into(),
        prompt_template_id: "command-adviser-v1".into(),
        memory_view_revision: "memory-view-7".into(),
        rag_snapshot_id: "snapshot-f88174".into(),
        skill_versions: vec![SkillVersionV1 {
            skill_id: "navigation-advice".into(),
            version: "1.0.0".into(),
        }],
        validation_results: vec![ValidationResultV1 {
            check_id: "schema".into(),
            passed: true,
            detail: None,
        }],
        supersedes: vec![],
        confidence: 0.8,
    }
}

#[test]
fn agent_experience_round_trips_through_encrypted_engram() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let expected = record();

    let event = build_experience_event(&agent, &owner.public_key(), &expected, 1_777_777_777)
        .expect("build experience event");
    let body = validate_and_decrypt(
        &event,
        &agent.public_key(),
        &owner.public_key(),
        agent.secret_key(),
        &owner.public_key(),
    )
    .expect("decrypt experience event");
    let decoded = from_engram_body(&body).expect("decode experience body");

    assert_eq!(decoded, expected);
    assert_eq!(
        body.slug(),
        format!("mem/experience/{}", expected.record_id)
    );
}

#[test]
fn agent_experience_accepts_terminal_outcomes_and_scopes() {
    for outcome in [
        ExperienceOutcome::Succeeded,
        ExperienceOutcome::Failed,
        ExperienceOutcome::Corrected,
        ExperienceOutcome::Cancelled,
        ExperienceOutcome::Abandoned,
    ] {
        let mut candidate = record();
        candidate.outcome = outcome;
        assert!(candidate.validate().is_ok());
    }

    let mut private = record();
    private.scope = MemoryScope::SpecialistPrivate;
    private.team_id = None;
    assert!(private.validate().is_ok());

    let mut invalid_private = private;
    invalid_private.specialist_id = None;
    assert!(invalid_private.validate().is_err());
}

#[test]
fn agent_experience_requires_supersession_for_superseded_outcome() {
    let mut candidate = record();
    candidate.outcome = ExperienceOutcome::Superseded;
    assert!(candidate.validate().is_err());

    candidate.supersedes = vec!["prior-event-id".into()];
    assert!(candidate.validate().is_ok());
}

#[test]
fn agent_experience_rejects_invalid_source_and_oversized_summary() {
    let mut invalid_source = record();
    invalid_source.source_ids = vec!["source with whitespace".into()];
    assert!(invalid_source.validate().is_err());

    let mut oversized = record();
    oversized.task_summary = "x".repeat(4_097);
    assert!(oversized.validate().is_err());
}

#[test]
fn agent_experience_rejects_secret_shaped_structured_fields() {
    let mut value = serde_json::to_value(record()).expect("serialize fixture");
    value["apiKey"] = json!("must-not-be-stored");

    assert!(serde_json::from_value::<ExperienceRecordV1>(value).is_err());
}

#[test]
fn agent_experience_redacts_obvious_free_text_secrets_before_storage() {
    let mut candidate = record();
    candidate.task_summary = "Login failed password=Farout23 during setup".into();

    let body = candidate.to_engram_body().expect("create redacted body");
    let Body::Memory {
        value: Some(value), ..
    } = body
    else {
        panic!("expected memory body");
    };

    assert!(!value.contains("Farout23"));
    assert!(value.contains("password=[REDACTED]"));
}

#[test]
fn experience_projection_is_bound_to_the_signed_event_and_owner() {
    let agent = Keys::generate();
    let owner = Keys::generate();
    let expected = record();
    let event = build_experience_event(&agent, &owner.public_key(), &expected, 1_777_777_777)
        .expect("event");

    let projection =
        experience_projection_payload(&event, &owner.public_key(), &expected).expect("projection");

    assert_eq!(projection["source_event_id"], event.id.to_hex());
    assert_eq!(projection["metadata"]["source_event_id"], event.id.to_hex());
    assert_eq!(
        projection["metadata"]["owner_id"],
        owner.public_key().to_hex()
    );
    assert_eq!(
        projection["metadata"]["source_created_at"],
        1_777_777_777_u64
    );
    assert_eq!(projection["metadata"]["status"], "active");
}
