use std::sync::{Arc, Mutex};

use nostr::{Event, Keys};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use super::audit::{AuditPublishFuture, BriefAuditPublisher, EncryptedBriefAudit};
use super::store::open_command_brief_store;
use super::types::{CommandBrief, PublicationState};

#[derive(Default)]
struct FakePublisher {
    events: Mutex<Vec<Event>>,
    fail: Mutex<bool>,
}

impl BriefAuditPublisher for FakePublisher {
    fn publish<'a>(&'a self, event: Event) -> AuditPublishFuture<'a> {
        Box::pin(async move {
            self.events.lock().map_err(|_| ())?.push(event);
            if *self.fail.lock().map_err(|_| ())? {
                Err(())
            } else {
                Ok(())
            }
        })
    }
}

fn brief() -> CommandBrief {
    CommandBrief::try_from(super::types_tests::brief_value()).expect("brief")
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
    *publisher.fail.lock().expect("fail") = true;
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
    *publisher.fail.lock().expect("fail") = false;
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
