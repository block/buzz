use std::path::PathBuf;

use buzz_runtime::{
    CreateJobOutcome, EnqueueOutcome, InboxEvent, JobStartRequest, JobState, JobTransition, NewJob,
    OutboxEvent, PublicationState, ResumeMode, RunnerIdentity, SessionRecord, StartupRecoveryPhase,
    StoreError, StoreHandle,
};
use chrono::Utc;
use nostr::{EventBuilder, Keys, Kind};
use uuid::Uuid;

fn outbox(
    job_id: Uuid,
    channel_id: Uuid,
    event_id: &str,
    kind: u16,
    terminal: bool,
) -> OutboxEvent {
    OutboxEvent {
        event_id: event_id.into(),
        job_id: Some(job_id),
        channel_id,
        ordering_key: format!("job:{job_id}"),
        kind,
        seq: None,
        is_terminal: terminal,
        event_json: "{}".into(),
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn job_transitions_are_monotonic_and_terminal_is_unique() {
    let directory = tempfile::tempdir().unwrap();
    let store = StoreHandle::open(directory.path().join("state").join("runtime.sqlite3")).unwrap();
    let diagnostics = store.operational_diagnostics().await.unwrap();
    assert_eq!(
        diagnostics.schema_version,
        buzz_runtime::STORE_SCHEMA_VERSION
    );
    assert!(diagnostics.last_relay_progress_published_at.is_none());
    let job_id = Uuid::new_v4();
    let channel_id = Uuid::new_v4();
    let request = JobStartRequest {
        channel_id,
        source_event_id: Some("ab".repeat(32)),
        driver: "lh".into(),
        argv: vec!["lockdown".into(), "run".into()],
        cwd: "/workspace".into(),
        summary: "work".into(),
    };
    let created = store
        .create_local_job(
            NewJob {
                job_id,
                request_event_id: "request".into(),
                requester_pubkey: "cd".repeat(32),
                executable: PathBuf::from("/usr/local/bin/lh"),
                request,
                attempt: 1,
                created_at: Utc::now(),
            },
            outbox(job_id, channel_id, "request", 43_001, false),
        )
        .await
        .unwrap();
    assert!(matches!(created, CreateJobOutcome::Created(_)));
    let pending = store.pending_outbox(10, Utc::now()).await.unwrap();
    assert_eq!(pending.len(), 1);
    let retry_at = Utc::now() + chrono::Duration::seconds(60);
    assert!(store
        .mark_outbox_retry(pending[0].id, "offline".into(), retry_at)
        .await
        .unwrap());
    assert!(store
        .pending_outbox(10, Utc::now())
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store.pending_outbox(10, retry_at).await.unwrap().len(), 1);
    let remote_id = Uuid::new_v4();
    let remote_job = NewJob {
        job_id: remote_id,
        request_event_id: "remote-request".into(),
        requester_pubkey: "ef".repeat(32),
        executable: PathBuf::from("/usr/local/bin/lh"),
        request: JobStartRequest {
            channel_id,
            source_event_id: None,
            driver: "lh".into(),
            argv: vec!["run".into()],
            cwd: "/workspace".into(),
            summary: "remote".into(),
        },
        attempt: 1,
        created_at: Utc::now(),
    };
    assert!(matches!(
        store.create_remote_job(remote_job.clone()).await,
        Err(StoreError::ActiveJobExists)
    ));
    assert_eq!(store.pending_outbox(10, retry_at).await.unwrap().len(), 1);
    assert!(store
        .mark_outbox_published(pending[0].id, Utc::now())
        .await
        .unwrap());

    let running = store
        .transition_job(
            JobTransition {
                job_id,
                attempt: 1,
                next_state: JobState::Running,
                runner: Some(RunnerIdentity {
                    pid: 42,
                    start_marker: "marker".into(),
                    process_group: "42".into(),
                }),
                progress_seq: None,
                exit_code: None,
                result_json: None,
                error_code: None,
                terminal_event_id: None,
                publication_state: Some(PublicationState::Pending),
                publication_error: None,
                occurred_at: Utc::now(),
            },
            Some(outbox(job_id, channel_id, "accepted", 43_002, false)),
        )
        .await
        .unwrap();
    assert_eq!(running.state, JobState::Running);
    let accepted_outbox = store.pending_outbox(10, Utc::now()).await.unwrap();
    assert_eq!(accepted_outbox.len(), 1);
    assert!(store
        .mark_outbox_published(accepted_outbox[0].id, Utc::now())
        .await
        .unwrap());
    let mut progress_event = outbox(job_id, channel_id, "progress", 43_003, false);
    progress_event.seq = Some(1);
    let progress = store
        .transition_job(
            JobTransition {
                job_id,
                attempt: 1,
                next_state: JobState::Running,
                runner: None,
                progress_seq: Some(1),
                exit_code: None,
                result_json: None,
                error_code: None,
                terminal_event_id: None,
                publication_state: Some(PublicationState::Pending),
                publication_error: None,
                occurred_at: Utc::now(),
            },
            Some(progress_event),
        )
        .await
        .unwrap();
    assert_eq!(progress.progress_seq, 1);
    let progress_outbox = store.pending_outbox(10, Utc::now()).await.unwrap();
    assert_eq!(progress_outbox.len(), 1);
    assert!(store
        .mark_outbox_published(progress_outbox[0].id, Utc::now())
        .await
        .unwrap());
    assert!(store
        .operational_diagnostics()
        .await
        .unwrap()
        .last_relay_progress_published_at
        .is_some());

    let terminal = store
        .transition_job(
            JobTransition {
                job_id,
                attempt: 1,
                next_state: JobState::Succeeded,
                runner: None,
                progress_seq: None,
                exit_code: Some(0),
                result_json: Some("{}".into()),
                error_code: None,
                terminal_event_id: Some("result".into()),
                publication_state: Some(PublicationState::Pending),
                publication_error: None,
                occurred_at: Utc::now(),
            },
            Some(outbox(job_id, channel_id, "result", 43_004, true)),
        )
        .await
        .unwrap();
    assert_eq!(terminal.state, JobState::Succeeded);
    assert!(matches!(
        store.create_remote_job(remote_job).await.unwrap(),
        CreateJobOutcome::Created(record) if record.job_id == remote_id
    ));

    let second = store
        .transition_job(
            JobTransition {
                job_id,
                attempt: 1,
                next_state: JobState::Failed,
                runner: None,
                progress_seq: None,
                exit_code: Some(1),
                result_json: None,
                error_code: Some("late".into()),
                terminal_event_id: Some("second".into()),
                publication_state: Some(PublicationState::Pending),
                publication_error: None,
                occurred_at: Utc::now(),
            },
            Some(outbox(job_id, channel_id, "second", 43_006, true)),
        )
        .await;
    assert!(matches!(
        second,
        Err(StoreError::InvalidJobTransition { .. })
    ));
}

#[tokio::test]
async fn file_backed_restart_stays_recovering_until_every_component_completes() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("state").join("runtime.sqlite3");
    let channel_id = Uuid::new_v4();
    let job_id = Uuid::new_v4();
    let event = EventBuilder::new(Kind::Custom(9), "recover this assignment")
        .sign_with_keys(&Keys::generate())
        .unwrap();

    let initial = StoreHandle::open(&database).unwrap();
    assert_eq!(
        initial
            .enqueue_inbox(InboxEvent {
                channel_id,
                event: event.clone(),
                received_at: Utc::now(),
            })
            .await
            .unwrap(),
        EnqueueOutcome::Enqueued
    );
    initial
        .claim_inbox_batch(1, "interrupted-turn".into(), Utc::now())
        .await
        .unwrap()
        .expect("persist an in-turn inbox row");
    initial
        .upsert_channel_session(SessionRecord {
            channel_id,
            session_id: "persisted-session".into(),
            adapter_fingerprint: "buzz-agent:test".into(),
            cwd: "/workspace".into(),
            config_hash: "config".into(),
            resume_mode: ResumeMode::Resume,
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let assignment = initial
        .claim_assignment(
            channel_id,
            Some(event.id.to_hex()),
            "recover active assignment".into(),
            Some("persisted-session".into()),
            Utc::now(),
        )
        .await
        .unwrap();
    initial
        .create_local_job(
            NewJob {
                job_id,
                request_event_id: "restart-request".into(),
                requester_pubkey: "cd".repeat(32),
                executable: PathBuf::from("/usr/local/bin/lh"),
                request: JobStartRequest {
                    channel_id,
                    source_event_id: Some(event.id.to_hex()),
                    driver: "lh".into(),
                    argv: vec!["lockdown".into(), "run".into()],
                    cwd: "/workspace".into(),
                    summary: "recover runner".into(),
                },
                attempt: 1,
                created_at: Utc::now(),
            },
            outbox(job_id, channel_id, "restart-request", 43_001, false),
        )
        .await
        .unwrap();
    drop(initial);

    let restarted = StoreHandle::open(&database).unwrap();
    let pending = restarted
        .begin_startup_recovery("runtime_restart")
        .await
        .unwrap();
    assert_eq!(pending.in_turn_inbox, 1);
    assert_eq!(
        pending
            .active_assignment
            .as_ref()
            .map(|value| value.assignment_id.as_str()),
        Some(assignment.assignment_id.as_str())
    );
    assert_eq!(pending.active_jobs, vec![job_id]);
    assert_eq!(pending.channel_sessions, vec![channel_id]);
    let during = restarted.assignment_snapshot().await.unwrap();
    assert!(during.recovering);
    assert_eq!(during.recovery_reason.as_deref(), Some("runtime_restart"));
    assert!(matches!(
        restarted.set_recovery_state(false, None).await,
        Err(StoreError::InvalidData(message))
            if message == "startup recovery components remain pending"
    ));

    restarted
        .set_recovery_state(true, Some("inbox_reconciliation".into()))
        .await
        .unwrap();
    let inbox_recovery = restarted.recover_in_turn(Utc::now()).await.unwrap();
    assert_eq!(inbox_recovery.requeued, 1);
    assert_eq!(inbox_recovery.dead_lettered, 0);
    assert!(!restarted
        .complete_startup_recovery_phase(StartupRecoveryPhase::Inbox)
        .await
        .unwrap());
    assert!(restarted.assignment_snapshot().await.unwrap().recovering);

    restarted
        .set_recovery_state(true, Some("session_reconciliation".into()))
        .await
        .unwrap();
    assert_eq!(
        restarted.channel_sessions().await.unwrap()[0].channel_id,
        channel_id
    );
    assert!(!restarted
        .complete_startup_recovery_phase(StartupRecoveryPhase::Sessions)
        .await
        .unwrap());
    assert!(restarted.assignment_snapshot().await.unwrap().recovering);

    restarted
        .set_recovery_state(true, Some("assignment_reconciliation".into()))
        .await
        .unwrap();
    assert_eq!(
        restarted
            .active_assignment()
            .await
            .unwrap()
            .unwrap()
            .assignment_id,
        assignment.assignment_id
    );
    assert!(!restarted
        .complete_startup_recovery_phase(StartupRecoveryPhase::Assignments)
        .await
        .unwrap());
    assert!(restarted.assignment_snapshot().await.unwrap().recovering);

    restarted
        .set_recovery_state(true, Some("runner_reconciliation".into()))
        .await
        .unwrap();
    assert_eq!(
        restarted.list_jobs(Default::default()).await.unwrap()[0].job_id,
        job_id
    );
    let runner_phase = restarted.assignment_snapshot().await.unwrap();
    assert!(runner_phase.recovering);
    assert_eq!(
        runner_phase.recovery_reason.as_deref(),
        Some("runner_reconciliation")
    );
    assert!(restarted
        .complete_startup_recovery_phase(StartupRecoveryPhase::Runners)
        .await
        .unwrap());
    let complete = restarted.assignment_snapshot().await.unwrap();
    assert!(!complete.recovering);
    assert!(complete.recovery_reason.is_none());
}
