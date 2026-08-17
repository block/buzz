use std::sync::{Arc, Mutex};

use buzz_core::agent_experience::build_experience_event;
use nostr::Keys;

use crate::{
    experience_capture::{ExperienceCapture, RuntimeEvidence, TurnOutcome},
    experience_outbox::{ExperienceOutbox, OutboxState},
    experience_projection::project_pending_with,
};

fn published_fixture(outbox: &ExperienceOutbox, suffix: &str) -> String {
    let record_id = format!("018f7f6a-2f11-7a90-a4b5-9dd45c1985{suffix}");
    let record = ExperienceCapture::from_turn(
        TurnOutcome::Completed,
        RuntimeEvidence {
            record_id: record_id.clone(),
            memory_key: format!("turn.{record_id}"),
            occurred_at: "2026-08-16T10:00:00Z".into(),
            task_summary: "Processed one owner request.".into(),
            specialist_id: "navigation".into(),
            team_id: "command-team".into(),
            source_ids: vec!["buzz:event-1".into()],
            model_identity: "gemma4-26b-official".into(),
            prompt_template_id: "buzz-acp-v1".into(),
            memory_view_revision: "nip-ae-core".into(),
            rag_snapshot_id: "snapshot-f88174".into(),
            skill_versions: vec![],
            validation_results: vec![],
        },
    )
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
    outbox
        .enqueue(&record_id, &event, &projection)
        .expect("enqueue");
    outbox.mark_published(&record_id).expect("published");
    record_id
}

#[tokio::test]
async fn projection_outage_leaves_published_work_for_retry_then_projects_once() {
    let directory = tempfile::tempdir().expect("tempdir");
    let outbox =
        ExperienceOutbox::open(&directory.path().join("experience.sqlite3")).expect("outbox");
    let record_id = published_fixture(&outbox, "01");

    let failed = project_pending_with(&outbox, |_| async { Err::<(), ()>(()) }).await;
    assert_eq!(failed.delayed, 1);
    assert_eq!(
        outbox.get(&record_id).expect("record").state,
        OutboxState::Published
    );

    let calls = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&calls);
    let recovered = project_pending_with(&outbox, move |arguments| {
        let captured = Arc::clone(&captured);
        async move {
            captured.lock().expect("calls").push(arguments);
            Ok::<(), ()>(())
        }
    })
    .await;
    assert_eq!(recovered.projected, 1);
    assert_eq!(calls.lock().expect("calls").len(), 1);
    assert_eq!(
        outbox.get(&record_id).expect("record").state,
        OutboxState::Projected
    );

    let duplicate_retry = project_pending_with(&outbox, |_| async { Ok::<(), ()>(()) }).await;
    assert_eq!(duplicate_retry.projected, 0);
}

#[tokio::test]
async fn poisoned_projection_is_isolated_from_valid_records() {
    let directory = tempfile::tempdir().expect("tempdir");
    let outbox =
        ExperienceOutbox::open(&directory.path().join("experience.sqlite3")).expect("outbox");
    let valid_id = published_fixture(&outbox, "02");

    let record = outbox.get(&valid_id).expect("valid record");
    let poisoned_id = "018f7f6a-2f11-7a90-a4b5-9dd45c198503";
    let poisoned = serde_json::json!({
        "source_event_id": "not-the-signed-event",
        "timestamp": "2026-08-16T10:00:00Z",
        "event_type": "command_experience",
        "content": "poisoned",
        "metadata": {}
    });
    outbox
        .enqueue(poisoned_id, &record.signed_event, &poisoned)
        .expect("poisoned enqueue");
    outbox.mark_published(poisoned_id).expect("published");

    let report = project_pending_with(&outbox, |_| async { Ok::<(), ()>(()) }).await;
    assert_eq!(report.projected, 1);
    assert_eq!(report.poisoned, 1);
    assert_eq!(
        outbox.get(poisoned_id).expect("poisoned").state,
        OutboxState::Published
    );
}
