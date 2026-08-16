use buzz_core::agent_experience::{build_experience_event, ExperienceOutcome, MemoryScope};
use nostr::Keys;

use crate::experience_capture::{ExperienceCapture, RuntimeEvidence, TurnOutcome};
use crate::experience_outbox::{ExperienceOutbox, OutboxState};

fn evidence() -> RuntimeEvidence {
    RuntimeEvidence {
        record_id: "018f7f6a-2f11-7a90-a4b5-9dd45c198501".into(),
        memory_key: "turn.018f7f6a-2f11-7a90-a4b5-9dd45c198501".into(),
        occurred_at: "2026-08-16T10:00:00Z".into(),
        task_summary: "Processed one owner request.".into(),
        specialist_id: "navigation".into(),
        team_id: "command-team".into(),
        source_ids: vec!["buzz:event-1".into()],
        model_identity: "gemma4-26b-official".into(),
        prompt_template_id: "buzz-acp-v1".into(),
        memory_view_revision: "nip-ae-core".into(),
        rag_snapshot_id: "snapshot-f88174".into(),
    }
}

#[test]
fn experience_capture_maps_completed_failed_cancelled_and_non_substantive_turns() {
    let completed = ExperienceCapture::from_turn(TurnOutcome::Completed, evidence())
        .expect("completed capture")
        .expect("substantive record");
    assert_eq!(completed.outcome, ExperienceOutcome::Succeeded);

    let failed = ExperienceCapture::from_turn(
        TurnOutcome::Failed {
            code: "idle_timeout".into(),
        },
        evidence(),
    )
    .expect("failed capture")
    .expect("substantive record");
    assert_eq!(failed.outcome, ExperienceOutcome::Failed);
    assert!(failed.limitations[0].contains("idle_timeout"));

    let cancelled = ExperienceCapture::from_turn(TurnOutcome::Cancelled, evidence())
        .expect("cancelled capture")
        .expect("substantive record");
    assert_eq!(cancelled.outcome, ExperienceOutcome::Cancelled);

    assert!(
        ExperienceCapture::from_turn(TurnOutcome::NonSubstantive, evidence())
            .expect("classified non-substantive")
            .is_none()
    );
}

#[test]
fn experience_capture_records_owner_correction_and_supersession() {
    let corrected = ExperienceCapture::from_turn(
        TurnOutcome::OwnerCorrection {
            decision: "Use the corrected departure sequence.".into(),
            supersedes: vec!["prior-record".into()],
        },
        evidence(),
    )
    .expect("correction capture")
    .expect("substantive record");

    assert_eq!(corrected.outcome, ExperienceOutcome::Corrected);
    assert_eq!(
        corrected.decision.as_deref(),
        Some("Use the corrected departure sequence.")
    );
    assert_eq!(corrected.supersedes, vec!["prior-record"]);
    assert_eq!(corrected.scope, MemoryScope::SpecialistPrivate);
}

fn signed_fixture() -> (String, nostr::Event, serde_json::Value) {
    let record = ExperienceCapture::from_turn(TurnOutcome::Completed, evidence())
        .expect("capture")
        .expect("record");
    let agent = Keys::generate();
    let owner = Keys::generate();
    let event =
        build_experience_event(&agent, &owner.public_key(), &record, 1_777_777_777).expect("event");
    let projection = serde_json::json!({
        "source_event_id": event.id.to_hex(),
        "timestamp": record.occurred_at,
        "agent": record.specialist_id,
        "event_type": "command_experience",
        "content": record.task_summary,
        "metadata": {
            "memory_key": record.memory_key,
            "status": "active",
            "scope": "specialist-private"
        }
    });
    (record.record_id, event, projection)
}

#[test]
fn experience_outbox_survives_crash_after_enqueue_and_deduplicates_recovery() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("experience.sqlite3");
    let (record_id, event, projection) = signed_fixture();

    let outbox = ExperienceOutbox::open(&path).expect("open outbox");
    assert!(outbox
        .enqueue(&record_id, &event, &projection)
        .expect("first enqueue"));
    drop(outbox);

    let recovered = ExperienceOutbox::open(&path).expect("reopen outbox");
    assert_eq!(
        recovered.get(&record_id).expect("get").state,
        OutboxState::Pending
    );
    assert!(!recovered
        .enqueue(&record_id, &event, &projection)
        .expect("duplicate enqueue"));
    assert_eq!(recovered.health().expect("health").pending, 1);
}

#[test]
fn experience_outbox_survives_publish_and_projection_transitions() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("experience.sqlite3");
    let (record_id, event, projection) = signed_fixture();
    let outbox = ExperienceOutbox::open(&path).expect("open outbox");
    outbox
        .enqueue(&record_id, &event, &projection)
        .expect("enqueue");
    outbox.mark_published(&record_id).expect("published");
    drop(outbox);

    let recovered = ExperienceOutbox::open(&path).expect("reopen outbox");
    assert_eq!(
        recovered.get(&record_id).expect("get").state,
        OutboxState::Published
    );
    assert_eq!(recovered.ready_for_projection().expect("ready").len(), 1);
    recovered.mark_projected(&record_id).expect("projected");
    assert_eq!(
        recovered.get(&record_id).expect("get").state,
        OutboxState::Projected
    );
}
