use std::path::PathBuf;
use std::sync::Arc;

use buzz_core::replication::{
    ReplicationIngestOutcome, ReplicationSinkPort, ReplicationSourceId, ReplicationSourcePort,
};
use buzz_local_relay::{
    EventStore, LocalRelay, LocalReplicationSource, ReplicationSourceAllowlist, StorageMode,
};
use nostr::{Event, EventBuilder, Filter, Keys, Kind};
use serde_json::Value;
use uuid::Uuid;

fn test_log_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "buzz-replication-{label}-{}.ndjson",
        Uuid::new_v4()
    ))
}

fn fixture_event() -> Event {
    serde_json::from_str(include_str!(
        "../../../specs/fixtures/local-relay/signed-message.json"
    ))
    .expect("signed fixture parses")
}

fn replication_vector() -> Value {
    serde_json::from_str(include_str!(
        "../../../specs/fixtures/portable-relay/replication-v0.1.json"
    ))
    .expect("replication vector parses")
}

fn signed_event(kind: u16, content: &str) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .sign_with_keys(&Keys::generate())
        .expect("test event signs")
}

#[tokio::test]
async fn durable_cursor_resumes_after_restart_and_destination_ingest_is_idempotent() {
    let vector = replication_vector();
    assert_eq!(vector["profile"], "portable-relay-replication-v0.1");
    let source_path = test_log_path("source");
    let destination_path = test_log_path("destination");
    let first_event = fixture_event();
    let second_event = signed_event(1, "replicated after checkpoint");
    let ephemeral = signed_event(20_001, "never replicate");
    let source_id = ReplicationSourceId::new(
        vector["source_id"]
            .as_str()
            .expect("vector source ID is a string"),
    );
    assert_eq!(
        first_event.id.to_hex(),
        vector["event_id"]
            .as_str()
            .expect("vector event ID is a string")
    );
    assert_eq!(vector["expectations"]["first_ingest"], "stored");
    assert_eq!(vector["expectations"]["replayed_ingest"], "duplicate");
    assert_eq!(vector["expectations"]["terminal_checkpoint_safe"], true);
    assert_eq!(vector["expectations"]["export_ephemeral_events"], false);

    let source_store = Arc::new(
        EventStore::open(StorageMode::Durable(source_path.clone()))
            .await
            .expect("source store opens"),
    );
    source_store
        .accept(first_event.clone())
        .await
        .expect("first source event stores");
    source_store
        .accept(ephemeral)
        .await
        .expect("ephemeral source event is accepted");
    source_store
        .accept(second_event.clone())
        .await
        .expect("second source event stores");

    let source = LocalReplicationSource::new(source_id.clone(), Arc::clone(&source_store));
    let first_batch = source
        .read_batch(None, 1)
        .await
        .expect("first replication batch reads");
    assert_eq!(first_batch.records.len(), 1);
    assert_eq!(first_batch.records[0].event, first_event);
    assert!(!first_batch.caught_up);
    let first_record = first_batch.records[0].clone();
    let checkpoint = first_batch.next_cursor;
    drop(source);
    drop(source_store);

    let reopened_store = Arc::new(
        EventStore::open(StorageMode::Durable(source_path.clone()))
            .await
            .expect("source store reopens"),
    );
    let reopened_source =
        LocalReplicationSource::new(source_id.clone(), Arc::clone(&reopened_store));
    let resumed_batch = reopened_source
        .read_batch(Some(checkpoint), 10)
        .await
        .expect("replication resumes after restart");
    assert_eq!(resumed_batch.records.len(), 1);
    assert_eq!(resumed_batch.records[0].event, second_event);
    assert!(resumed_batch.caught_up);
    let second_record = resumed_batch.records[0].clone();

    let policy = ReplicationSourceAllowlist::new([source_id]);
    let destination = LocalRelay::open_with_replication_policy(
        StorageMode::Durable(destination_path.clone()),
        Arc::new(policy),
    )
    .await
    .expect("destination relay opens");

    let first_receipt = destination
        .ingest_replication(first_record.clone())
        .await
        .expect("first destination ingest completes");
    assert_eq!(first_receipt.outcome, ReplicationIngestOutcome::Stored);
    assert!(first_receipt.checkpoint_safe());
    let second_receipt = destination
        .ingest_replication(second_record)
        .await
        .expect("second destination ingest completes");
    assert_eq!(second_receipt.outcome, ReplicationIngestOutcome::Stored);
    assert!(second_receipt.checkpoint_safe());

    let duplicate_receipt = destination
        .ingest_replication(first_record)
        .await
        .expect("duplicate destination ingest completes");
    assert_eq!(
        duplicate_receipt.outcome,
        ReplicationIngestOutcome::Duplicate
    );
    assert!(duplicate_receipt.checkpoint_safe());

    let first_query = destination
        .store()
        .query(&[Filter::new().id(first_event.id)])
        .await
        .expect("destination query succeeds");
    assert_eq!(first_query, vec![first_event]);
    let second_query = destination
        .store()
        .query(&[Filter::new().id(second_event.id)])
        .await
        .expect("destination query succeeds");
    assert_eq!(second_query, vec![second_event]);

    drop(destination);
    drop(reopened_source);
    drop(reopened_store);
    std::fs::remove_file(source_path).expect("source journal removes");
    std::fs::remove_file(destination_path).expect("destination journal removes");
}

#[tokio::test]
async fn destination_denies_unconfigured_sources_without_mutation() {
    let vector = replication_vector();
    assert_eq!(vector["expectations"]["unconfigured_source"], "rejected");
    assert_eq!(vector["expectations"]["rejected_checkpoint_safe"], false);
    let event = fixture_event();
    let source_store = Arc::new(
        EventStore::open(StorageMode::Ephemeral)
            .await
            .expect("source opens"),
    );
    source_store
        .accept(event.clone())
        .await
        .expect("source event stores");
    let source =
        LocalReplicationSource::new(ReplicationSourceId::new("untrusted/source"), source_store);
    let record = source
        .read_batch(None, 1)
        .await
        .expect("source batch reads")
        .records
        .into_iter()
        .next()
        .expect("source record exists");

    let destination = LocalRelay::open(StorageMode::Ephemeral)
        .await
        .expect("destination opens");
    let receipt = destination
        .ingest_replication(record)
        .await
        .expect("policy rejection is a receipt");
    assert!(matches!(
        receipt.outcome,
        ReplicationIngestOutcome::Rejected { .. }
    ));
    assert!(!receipt.checkpoint_safe());
    assert!(destination
        .store()
        .query(&[Filter::new().id(event.id)])
        .await
        .expect("destination query succeeds")
        .is_empty());
}
