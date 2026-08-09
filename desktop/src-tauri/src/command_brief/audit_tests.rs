use std::sync::{Arc, Mutex};

use nostr::{Event, Keys};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use super::audit::{
    AuditCommitFuture, AuditPublishError, AuditPublishFuture, BriefAuditCommitGate,
    BriefAuditPublisher, EncryptedBriefAudit, TerminalAuditInput,
};
use super::store::{mark_publish_failed, open_command_brief_store};
use super::types::{CommandBrief, PublicationState};
use buzz_core_pkg::command_brief::{CommandBriefFailureCode, CommandBriefLifecycleState};

#[derive(Default)]
struct FakePublisher {
    events: Mutex<Vec<Event>>,
    failure: Mutex<Option<AuditPublishError>>,
}

impl BriefAuditPublisher for FakePublisher {
    fn publish<'a>(&'a self, event: Event) -> AuditPublishFuture<'a> {
        Box::pin(async move {
            self.events
                .lock()
                .map_err(|_| AuditPublishError::Transient)?
                .push(event);
            if let Some(error) = *self
                .failure
                .lock()
                .map_err(|_| AuditPublishError::Transient)?
            {
                Err(error)
            } else {
                Ok(())
            }
        })
    }
}

fn brief() -> CommandBrief {
    CommandBrief::try_from(super::types_tests::brief_value()).expect("brief")
}

struct BarrierCommitGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

impl BarrierCommitGate {
    fn new() -> Self {
        Self {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }
}

impl BriefAuditCommitGate for BarrierCommitGate {
    fn wait<'a>(&'a self) -> AuditCommitFuture<'a> {
        Box::pin(async move {
            self.entered.notify_one();
            self.release.notified().await;
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_spool_commit_is_cancellation_linearization_and_writes_one_terminal() {
    let dir = tempdir().expect("tempdir");
    let owner = Keys::generate();
    let publisher = Arc::new(FakePublisher::default());
    let gate = Arc::new(BarrierCommitGate::new());
    let audit = Arc::new(EncryptedBriefAudit::new_with_commit_gate(
        dir.path().join("brief.db"),
        owner.clone(),
        publisher.clone(),
        gate.clone(),
    ));
    let token = CancellationToken::new();
    let task_audit = Arc::clone(&audit);
    let task_token = token.clone();
    let task = tokio::spawn(async move {
        task_audit
            .persist_terminal_input(TerminalAuditInput::completed(brief()), task_token)
            .await
    });
    gate.entered.notified().await;
    assert!(audit.request_cancel("run-1", &token));
    gate.release.notify_one();
    let persisted = task
        .await
        .expect("join")
        .expect("cancelled terminal persists");
    assert_eq!(
        persisted.lifecycle_state(),
        CommandBriefLifecycleState::Cancelled
    );
    assert_eq!(
        persisted.failure_code(),
        Some(CommandBriefFailureCode::CancellationRequested)
    );
    assert!(persisted.published_brief().is_none());
    assert_eq!(publisher.events.lock().expect("events").len(), 1);

    let conn = open_command_brief_store(&dir.path().join("brief.db")).expect("store");
    let (count, status): (i64, String) = conn
        .query_row(
            "SELECT COUNT(*),MIN(status) FROM command_brief_spool",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("terminal row");
    assert_eq!(count, 1);
    assert_eq!(status, "cancelled");

    let event = publisher.events.lock().expect("events")[0].clone();
    let payload =
        buzz_core_pkg::command_brief::decrypt_command_brief_event(&owner, &event).expect("decrypt");
    assert_eq!(
        payload.lifecycle_state,
        CommandBriefLifecycleState::Cancelled
    );
    assert_eq!(
        payload.failure.as_ref().map(|failure| failure.code),
        persisted.failure_code()
    );
    assert!(payload.final_brief.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_after_real_spool_success_commit_is_rejected() {
    let dir = tempdir().expect("tempdir");
    let owner = Keys::generate();
    let publisher = Arc::new(FakePublisher::default());
    let audit = EncryptedBriefAudit::new(dir.path().join("brief.db"), owner, publisher);
    let token = CancellationToken::new();
    let persisted = audit
        .persist_terminal_input(TerminalAuditInput::completed(brief()), token.clone())
        .await
        .expect("success terminal");
    assert_eq!(
        persisted.lifecycle_state(),
        CommandBriefLifecycleState::Completed
    );
    assert!(persisted.published_brief().is_some());
    assert!(!audit.request_cancel("run-1", &token));
    assert!(!token.is_cancelled());
}

#[tokio::test]
async fn every_terminal_class_is_signed_spooled_and_decrypts_to_closed_wire_state() {
    let dir = tempdir().expect("tempdir");
    let owner = Keys::generate();
    let publisher = Arc::new(FakePublisher::default());
    let audit = EncryptedBriefAudit::new(
        dir.path().join("brief.db"),
        owner.clone(),
        publisher.clone(),
    );

    let completed = brief();
    audit
        .persist_terminal_input(
            TerminalAuditInput::completed(completed),
            CancellationToken::new(),
        )
        .await
        .expect("completed");

    let mut degraded_value = super::types_tests::brief_value();
    degraded_value["runId"] = serde_json::json!("run-degraded");
    degraded_value["degradedSections"] = serde_json::json!(["navigation"]);
    let degraded = CommandBrief::try_from(degraded_value).expect("degraded brief");
    audit
        .persist_terminal_input(
            TerminalAuditInput::completed(degraded),
            CancellationToken::new(),
        )
        .await
        .expect("degraded");

    for (run_id, code) in [
        ("run-source-failed", CommandBriefFailureCode::RagUnavailable),
        (
            "run-assembly-failed",
            CommandBriefFailureCode::BriefAssemblyRejected,
        ),
    ] {
        audit
            .persist_terminal_input(
                TerminalAuditInput::closed(
                    run_id.to_string(),
                    "daily".to_string(),
                    "2026-07-25T06:00:00Z".to_string(),
                    "snapshot-1".to_string(),
                    CommandBriefLifecycleState::Failed,
                    code,
                )
                .expect("closed input"),
                CancellationToken::new(),
            )
            .await
            .expect("failed terminal");
    }

    audit
        .persist_terminal_input(
            TerminalAuditInput::closed(
                "run-explicit-cancel".to_string(),
                "daily".to_string(),
                "2026-07-25T06:00:00Z".to_string(),
                "snapshot-1".to_string(),
                CommandBriefLifecycleState::Cancelled,
                CommandBriefFailureCode::CancellationRequested,
            )
            .expect("cancel input"),
            CancellationToken::new(),
        )
        .await
        .expect("cancelled terminal");

    let events = publisher.events.lock().expect("events");
    assert_eq!(events.len(), 5);
    let payloads = events
        .iter()
        .map(|event| {
            buzz_core_pkg::command_brief::decrypt_command_brief_event(&owner, event)
                .expect("strict terminal decrypt")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        payloads
            .iter()
            .map(|payload| payload.lifecycle_state)
            .collect::<Vec<_>>(),
        vec![
            CommandBriefLifecycleState::Completed,
            CommandBriefLifecycleState::Degraded,
            CommandBriefLifecycleState::Failed,
            CommandBriefLifecycleState::Failed,
            CommandBriefLifecycleState::Cancelled,
        ]
    );
    assert_eq!(
        payloads[2].failure.as_ref().map(|failure| failure.code),
        Some(CommandBriefFailureCode::RagUnavailable)
    );
    assert_eq!(
        payloads[3].failure.as_ref().map(|failure| failure.code),
        Some(CommandBriefFailureCode::BriefAssemblyRejected)
    );
    assert_eq!(
        payloads[4].failure.as_ref().map(|failure| failure.code),
        Some(CommandBriefFailureCode::CancellationRequested)
    );

    let conn = open_command_brief_store(&dir.path().join("brief.db")).expect("store");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM command_brief_spool", [], |row| {
            row.get(0)
        })
        .expect("count");
    assert_eq!(count, 5);
}

#[tokio::test]
async fn signs_spools_then_publishes_without_event_id_self_reference() {
    let dir = tempdir().expect("tempdir");
    let owner = Keys::generate();
    let publisher = Arc::new(FakePublisher::default());
    let audit = EncryptedBriefAudit::new(
        dir.path().join("brief.db"),
        owner.clone(),
        publisher.clone(),
    );
    let published = audit
        .persist_terminal(&brief(), CancellationToken::new())
        .await
        .expect("persist");
    assert_eq!(published.publication_state(), PublicationState::Published);
    let events = publisher.events.lock().expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id.to_hex(), published.lifecycle_audit_event_id());
    let decrypted = audit
        .decrypt_for_current_owner(&events[0])
        .expect("owner decrypt");
    assert_eq!(decrypted.run_id(), "run-1");
    let plaintext = serde_json::to_string(&decrypted).expect("json");
    assert!(!plaintext.contains(published.lifecycle_audit_event_id()));
}

#[tokio::test]
async fn offline_publish_remains_queued_and_republishes_same_event_id() {
    let dir = tempdir().expect("tempdir");
    let owner = Keys::generate();
    let publisher = Arc::new(FakePublisher::default());
    *publisher.failure.lock().expect("failure") = Some(AuditPublishError::Transient);
    let audit = EncryptedBriefAudit::new(
        dir.path().join("brief.db"),
        owner.clone(),
        publisher.clone(),
    );
    let published = audit
        .persist_terminal(&brief(), CancellationToken::new())
        .await
        .expect("local completion");
    assert_eq!(published.publication_state(), PublicationState::Queued);
    let first_id = published.lifecycle_audit_event_id().to_string();
    *publisher.failure.lock().expect("failure") = None;
    audit.republish_due(i64::MAX).await.expect("republish");
    let ids: Vec<String> = publisher
        .events
        .lock()
        .expect("events")
        .iter()
        .map(|event| event.id.to_hex())
        .collect();
    assert_eq!(ids, vec![first_id.clone(), first_id]);
    let conn = open_command_brief_store(&dir.path().join("brief.db")).expect("store");
    let state: String = conn
        .query_row("SELECT publish_state FROM command_brief_spool", [], |row| {
            row.get(0)
        })
        .expect("state");
    assert_eq!(state, "published");
}

#[tokio::test]
async fn restart_readiness_recovers_exact_event_after_eight_prior_failures() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("brief.db");
    let owner = Keys::generate();
    let offline = Arc::new(FakePublisher::default());
    *offline.failure.lock().expect("failure") = Some(AuditPublishError::Transient);
    let first = EncryptedBriefAudit::new(path.clone(), owner.clone(), offline.clone());
    let published = first
        .persist_terminal(&brief(), CancellationToken::new())
        .await
        .expect("local completion");
    let event_id = published.lifecycle_audit_event_id().to_string();
    {
        let conn = open_command_brief_store(&path).expect("store");
        for now in 1..8 {
            mark_publish_failed(&conn, &owner.public_key().to_hex(), &event_id, now)
                .expect("exhaust retry");
        }
    }

    let ready = Arc::new(FakePublisher::default());
    let restarted = EncryptedBriefAudit::new(path.clone(), owner, ready.clone());
    assert_eq!(
        restarted
            .recover_on_relay_ready(10_000)
            .await
            .expect("recover"),
        1
    );
    let ids = ready
        .events
        .lock()
        .expect("events")
        .iter()
        .map(|event| event.id.to_hex())
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![event_id]);
    let state: String = open_command_brief_store(&path)
        .expect("store")
        .query_row("SELECT publish_state FROM command_brief_spool", [], |row| {
            row.get(0)
        })
        .expect("state");
    assert_eq!(state, "published");
}

#[tokio::test]
async fn readiness_quarantines_invalid_spool_bytes_without_retry_hot_loop() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("brief.db");
    let owner = Keys::generate();
    let publisher = Arc::new(FakePublisher::default());
    *publisher.failure.lock().expect("failure") = Some(AuditPublishError::Transient);
    let audit = EncryptedBriefAudit::new(path.clone(), owner.clone(), publisher.clone());
    let published = audit
        .persist_terminal(&brief(), CancellationToken::new())
        .await
        .expect("queued terminal");
    let event_id = published.lifecycle_audit_event_id().to_string();
    {
        let conn = open_command_brief_store(&path).expect("store");
        conn.execute(
            "UPDATE command_brief_spool SET raw_event='{}' WHERE event_id=?1",
            [&event_id],
        )
        .expect("tamper row");
    }
    *publisher.failure.lock().expect("failure") = None;

    assert_eq!(
        audit.recover_on_relay_ready(10_000).await.expect("recover"),
        0
    );
    let (retry_count, next_retry_at, error): (i64, i64, String) = open_command_brief_store(&path)
        .expect("store")
        .query_row(
            "SELECT retry_count,next_retry_at,last_error_code
                 FROM command_brief_spool WHERE event_id=?1",
            [&event_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("quarantine");
    assert_eq!(retry_count, 8);
    assert_eq!(next_retry_at, i64::MAX);
    assert_eq!(error, "invalid_event");
    assert_eq!(
        audit
            .recover_on_relay_ready(20_000)
            .await
            .expect("second recover"),
        0
    );
    assert_eq!(
        publisher.events.lock().expect("events").len(),
        1,
        "only the initial best-effort publish saw the event"
    );
}

#[tokio::test]
async fn permanent_relay_rejection_is_quarantined_and_never_rearmed() {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("brief.db");
    let owner = Keys::generate();
    let publisher = Arc::new(FakePublisher::default());
    *publisher.failure.lock().expect("failure") = Some(AuditPublishError::Permanent);
    let audit = EncryptedBriefAudit::new(path.clone(), owner, publisher.clone());

    let published = audit
        .persist_terminal(&brief(), CancellationToken::new())
        .await
        .expect("durable local terminal");
    assert_eq!(published.publication_state(), PublicationState::Queued);
    assert_eq!(
        audit
            .recover_on_relay_ready(10_000)
            .await
            .expect("first readiness"),
        0
    );
    assert_eq!(
        audit
            .recover_on_relay_ready(20_000)
            .await
            .expect("second readiness"),
        0
    );
    assert_eq!(publisher.events.lock().expect("events").len(), 1);
    let (retry_count, next_retry_at, error): (i64, i64, String) = open_command_brief_store(&path)
        .expect("store")
        .query_row(
            "SELECT retry_count,next_retry_at,last_error_code FROM command_brief_spool",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("quarantine");
    assert_eq!((retry_count, next_retry_at), (8, i64::MAX));
    assert_eq!(error, "invalid_event");
}

#[tokio::test]
async fn wrong_unlocked_identity_cannot_decrypt_or_publish_another_owner_row() {
    let dir = tempdir().expect("tempdir");
    let owner = Keys::generate();
    let publisher = Arc::new(FakePublisher::default());
    let audit = EncryptedBriefAudit::new(dir.path().join("brief.db"), owner, publisher.clone());
    let _ = audit
        .persist_terminal(&brief(), CancellationToken::new())
        .await
        .expect("persist");
    let event = publisher.events.lock().expect("events")[0].clone();
    let wrong = EncryptedBriefAudit::new(dir.path().join("brief.db"), Keys::generate(), publisher);
    assert!(wrong.decrypt_for_current_owner(&event).is_err());
    assert_eq!(wrong.republish_due(i64::MAX).await.expect("republish"), 0);
}
