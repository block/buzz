use super::*;
use crate::archive::store::{open_archive_db, SCHEMA};
use rusqlite::Connection;

fn in_memory() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn
}

fn override_input(summary: &str) -> OwnerJournalOverrideInput {
    OwnerJournalOverrideInput {
        journal_id: "agent:channel:turn-1".into(),
        correlation_id: "tool-call-1".into(),
        summary: summary.into(),
        note: Some("Owner corrected the narrative.".into()),
    }
}

fn verification_input() -> JournalVerificationInput {
    JournalVerificationInput {
        journal_id: "agent:channel:turn-1".into(),
        correlation_id: "tool-call-1".into(),
        receipt_ref: "receipt://archive/tool-call-1".into(),
        source_event_ids: vec!["a".repeat(64), "b".repeat(64)],
    }
}

#[test]
fn owner_override_is_signed_persisted_and_idempotent() {
    let conn = in_memory();
    let owner = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let raw =
        build_owner_override_event(&owner, &override_input("Observed owner result."), 1).unwrap();

    let first = upsert_signed_artifact(&conn, &owner_pk, &raw, 10).unwrap();
    let replay = upsert_signed_artifact(&conn, &owner_pk, &raw, 11).unwrap();
    assert_eq!(first, replay);
    assert_eq!(
        first.artifact_type,
        JournalAuthorityArtifactType::OwnerOverride
    );
    assert_eq!(first.summary.as_deref(), Some("Observed owner result."));
    assert_eq!(
        get_journal_authority_artifacts(&conn, &owner_pk, &first.journal_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn stale_valid_revision_cannot_replay_over_current_state() {
    let conn = in_memory();
    let owner = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let first = build_owner_override_event(&owner, &override_input("First"), 1).unwrap();
    let second = build_owner_override_event(&owner, &override_input("Second"), 2).unwrap();
    let stale = build_owner_override_event(&owner, &override_input("Stale rewrite"), 1).unwrap();
    upsert_signed_artifact(&conn, &owner_pk, &first, 10).unwrap();
    upsert_signed_artifact(&conn, &owner_pk, &second, 11).unwrap();
    let error = upsert_signed_artifact(&conn, &owner_pk, &stale, 12).unwrap_err();
    assert!(error.contains("stale journal authority replay"));

    let rows = get_journal_authority_artifacts(&conn, &owner_pk, "agent:channel:turn-1").unwrap();
    assert_eq!(rows[0].revision, 2);
    assert_eq!(rows[0].summary.as_deref(), Some("Second"));
}

#[test]
fn first_insert_must_start_at_revision_one() {
    let conn = in_memory();
    let owner = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let raw = build_owner_override_event(&owner, &override_input("Skipped"), 2).unwrap();
    let error = upsert_signed_artifact(&conn, &owner_pk, &raw, 10).unwrap_err();
    assert!(error.contains("first journal authority revision must be 1"));
}

#[test]
fn wrong_signer_fails_closed_and_identity_rows_are_isolated() {
    let conn = in_memory();
    let owner = Keys::generate();
    let other = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let other_pk = other.public_key().to_hex();
    let raw = build_owner_override_event(&owner, &override_input("Owner only"), 1).unwrap();
    assert!(upsert_signed_artifact(&conn, &other_pk, &raw, 10)
        .unwrap_err()
        .contains("signer is not the active owner"));

    upsert_signed_artifact(&conn, &owner_pk, &raw, 10).unwrap();
    assert!(
        get_journal_authority_artifacts(&conn, &other_pk, "agent:channel:turn-1")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn tampered_signature_and_tampered_database_columns_fail_closed() {
    let conn = in_memory();
    let owner = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let raw = build_owner_override_event(&owner, &override_input("Untampered"), 1).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value["content"] = serde_json::Value::String("{}".into());
    let tampered = serde_json::to_string(&value).unwrap();
    assert!(upsert_signed_artifact(&conn, &owner_pk, &tampered, 10)
        .unwrap_err()
        .contains("signature verification failed"));

    let artifact = upsert_signed_artifact(&conn, &owner_pk, &raw, 10).unwrap();
    conn.execute(
        "UPDATE journal_authority_artifacts SET revision = 99
         WHERE identity_pubkey = ?1 AND journal_id = ?2",
        params![owner_pk, artifact.journal_id],
    )
    .unwrap();
    assert!(
        get_journal_authority_artifacts(&conn, &owner_pk, "agent:channel:turn-1")
            .unwrap_err()
            .contains("columns do not match signed event")
    );
}

#[test]
fn verification_binds_receipt_correlation_and_source_events() {
    let conn = in_memory();
    let owner = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let raw = build_verification_event(&owner, &verification_input(), 1).unwrap();
    let artifact = upsert_signed_artifact(&conn, &owner_pk, &raw, 10).unwrap();
    assert_eq!(
        artifact.artifact_type,
        JournalAuthorityArtifactType::Verification
    );
    assert_eq!(artifact.correlation_id, "tool-call-1");
    assert_eq!(
        artifact.receipt_ref.as_deref(),
        Some("receipt://archive/tool-call-1")
    );
    assert_eq!(artifact.source_event_ids, ["a".repeat(64), "b".repeat(64)]);
}

#[test]
fn verification_missing_receipt_or_source_event_fails_closed() {
    let owner = Keys::generate();
    let mut no_receipt = verification_input();
    no_receipt.receipt_ref = " ".into();
    assert!(build_verification_event(&owner, &no_receipt, 1)
        .unwrap_err()
        .contains("receiptRef"));

    let mut no_source = verification_input();
    no_source.source_event_ids.clear();
    assert!(build_verification_event(&owner, &no_source, 1)
        .unwrap_err()
        .contains("must bind between"));
}

#[test]
fn verification_rejects_duplicate_and_malformed_source_ids() {
    let owner = Keys::generate();
    let mut duplicate = verification_input();
    duplicate.source_event_ids = vec!["a".repeat(64), "a".repeat(64)];
    assert!(build_verification_event(&owner, &duplicate, 1)
        .unwrap_err()
        .contains("must be unique"));

    let mut malformed = verification_input();
    malformed.source_event_ids = vec!["not-an-event".into()];
    assert!(build_verification_event(&owner, &malformed, 1)
        .unwrap_err()
        .contains("64-character hexadecimal"));
}

#[test]
fn verification_sources_must_exist_and_remain_valid_in_owner_archive() {
    let conn = in_memory();
    let owner = Keys::generate();
    let agent = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let event = EventBuilder::new(Kind::Custom(24200), "signed observer")
        .sign_with_keys(&agent)
        .unwrap();
    let event_id = event.id.to_hex();
    let input = JournalVerificationInput {
        source_event_ids: vec![event_id.clone()],
        ..verification_input()
    };
    let raw = build_verification_event(&owner, &input, 1).unwrap();
    let artifact = upsert_signed_artifact(&conn, &owner_pk, &raw, 10).unwrap();
    assert!(
        validate_archived_verification_sources(&conn, &owner_pk, &artifact)
            .unwrap_err()
            .contains("is not archived")
    );

    conn.execute(
        "INSERT INTO archived_events
         (identity_pubkey, relay_url, id, kind, pubkey, created_at, raw_json, archived_at)
         VALUES (?1, 'wss://r', ?2, 24200, ?3, 1, ?4, 1)",
        params![
            owner_pk,
            event_id,
            agent.public_key().to_hex(),
            event.as_json()
        ],
    )
    .unwrap();
    validate_archived_verification_sources(&conn, &owner_pk, &artifact).unwrap();

    conn.execute(
        "UPDATE archived_events SET raw_json = '{}' WHERE identity_pubkey = ?1 AND id = ?2",
        params![owner_pk, event_id],
    )
    .unwrap();
    assert!(
        validate_archived_verification_sources(&conn, &owner_pk, &artifact)
            .unwrap_err()
            .contains("failed validation")
    );
}

#[test]
fn durable_artifact_survives_close_and_reopen() {
    let db_file = tempfile::NamedTempFile::new().unwrap();
    let owner = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let raw = build_verification_event(&owner, &verification_input(), 1).unwrap();
    {
        let conn = open_archive_db(db_file.path()).unwrap();
        upsert_signed_artifact(&conn, &owner_pk, &raw, 10).unwrap();
    }
    let reopened = open_archive_db(db_file.path()).unwrap();
    let rows =
        get_journal_authority_artifacts(&reopened, &owner_pk, "agent:channel:turn-1").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].receipt_ref.as_deref(),
        Some("receipt://archive/tool-call-1")
    );
}

#[test]
fn bounded_today_query_is_owner_scoped_and_returns_public_fields_only() {
    let conn = in_memory();
    let owner = Keys::generate();
    let other = Keys::generate();
    let owner_pk = owner.public_key().to_hex();
    let other_pk = other.public_key().to_hex();
    let raw = build_owner_override_event(&owner, &override_input("Today"), 1).unwrap();
    let artifact = upsert_signed_artifact(&conn, &owner_pk, &raw, 10).unwrap();

    let rows = query_journal_authority_artifacts(
        &conn,
        &owner_pk,
        artifact.created_at - 1,
        artifact.created_at + 1,
        10,
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(query_journal_authority_artifacts(
        &conn,
        &other_pk,
        artifact.created_at - 1,
        artifact.created_at + 1,
        10,
    )
    .unwrap()
    .is_empty());
    assert!(query_journal_authority_artifacts(&conn, &owner_pk, 1, 2, 501).is_err());
}
