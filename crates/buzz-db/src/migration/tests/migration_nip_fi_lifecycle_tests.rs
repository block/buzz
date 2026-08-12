use super::*;

use buzz_core::CommunityId;
use uuid::Uuid;

use crate::lifecycle_invalidation::{
    advance_lifecycle_invalidation_tx, LifecycleInvalidationSelector,
};

const LIFECYCLE_UPGRADE_SQL: &str =
    include_str!("../../../../../migrations/0031_nip_fi_identity_lifecycle_upgrade.sql");

#[test]
fn lifecycle_upgrade_is_additive_and_closes_receipt_event_cardinality() {
    for forbidden in [
        "CREATE TABLE identity_bindings",
        "CREATE TABLE identity_lifecycle_history",
        "CREATE TABLE identity_lifecycle_selectors",
        "CREATE TABLE authorization_events",
        "ALTER TABLE identity_bindings",
    ] {
        assert!(
            !LIFECYCLE_UPGRADE_SQL.contains(forbidden),
            "0031 must not recreate or weaken v29/v30: {forbidden}"
        );
    }
    assert!(LIFECYCLE_UPGRADE_SQL.contains("nip_fi_lock_authorization_operation_v1"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("authorization_operation_receipt_event_guard_v1"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("CREATE TABLE authorization_admission_results"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("semantic_fingerprint BYTEA NOT NULL"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("object_kind SMALLINT NOT NULL"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("object_key BYTEA NOT NULL"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("application_type BYTEA"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("application_version SMALLINT"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("application_code SMALLINT"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("octet_length(application_payload) <= 4096"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("DEFERRABLE INITIALLY DEFERRED"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("matching_event_count <> 1"));
    assert!(LIFECYCLE_UPGRADE_SQL.contains("WHEN 7 THEN 4  -- recover"));
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn lifecycle_0031_enforces_attested_provisioned_tofu_and_atomic_audit() {
    let pool = connect_test_pool().await;
    reset_public_schema(&pool).await;
    run_migrations(&pool)
        .await
        .expect("apply migrations through lifecycle 0031");

    let community_id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
        .bind(community_id)
        .bind(format!("lifecycle-{}.example", community_id.simple()))
        .execute(&pool)
        .await
        .expect("insert lifecycle community");
    for (revision, mode) in [(1_i64, 1_i16), (2, 2), (3, 3)] {
        sqlx::query(
            "INSERT INTO identity_enrollment_policies \
             (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
             VALUES ($1,$2,$3,$4,transaction_timestamp())",
        )
        .bind(community_id)
        .bind(revision)
        .bind(mode)
        .bind(vec![mode as u8 + 100; 32])
        .execute(&pool)
        .await
        .expect("insert closed enrollment policy");
    }
    sqlx::query(
        "INSERT INTO authorization_event_capacity \
         (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes) \
         VALUES ($1,32,32768,2048)",
    )
    .bind(community_id)
    .execute(&pool)
    .await
    .expect("install lifecycle audit capacity");

    for (ordinal, mode, operation_kind, transition_kind) in
        [(1_u8, 1_i16, 1_i16, 1_i16), (2, 2, 2, 2), (3, 3, 1, 1)]
    {
        insert_complete_birth(
            &pool,
            community_id,
            ordinal,
            mode,
            operation_kind,
            transition_kind,
            true,
        )
        .await
        .expect("commit complete lifecycle birth");
    }
    let committed_bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM identity_bindings WHERE community_id=$1 AND binding_state=1",
    )
    .bind(community_id)
    .fetch_one(&pool)
    .await
    .expect("count committed bindings");
    assert_eq!(committed_bindings, 3);

    let conflicting_principal = sqlx::query(
        "INSERT INTO identity_bindings \
         (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
          binding_state,lifecycle_revision,binding_provenance,policy_revision, \
          enrollment_evidence_digest,birth_history_id,creation_operation_id, \
          creation_request_fingerprint) \
         SELECT community_id,$2,issuer,subject,principal_fingerprint,$3,1,1, \
                binding_provenance,policy_revision,enrollment_evidence_digest,$4,$5,$6 \
         FROM identity_bindings WHERE community_id=$1 ORDER BY binding_version LIMIT 1",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind([201_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind([202_u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect_err("one principal cannot acquire a different active key");
    assert_eq!(
        conflicting_principal
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("identity_bindings_active_principal")
    );
    let conflicting_key = sqlx::query(
        "INSERT INTO identity_bindings \
         (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
          binding_state,lifecycle_revision,binding_provenance,policy_revision, \
          enrollment_evidence_digest,birth_history_id,creation_operation_id, \
          creation_request_fingerprint) \
         SELECT community_id,$2,'https://other-issuer.example','other-subject',$3, \
                event_author_pubkey,1,1,binding_provenance,policy_revision, \
                enrollment_evidence_digest,$4,$5,$6 \
         FROM identity_bindings WHERE community_id=$1 ORDER BY binding_version LIMIT 1",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind([203_u8; 32].as_slice())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind([204_u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect_err("one key cannot acquire a different active principal");
    assert_eq!(
        conflicting_key
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("identity_bindings_active_event_author")
    );

    let retained_before: i64 = sqlx::query_scalar(
        "SELECT retained_event_count FROM authorization_event_capacity WHERE community_id=$1",
    )
    .bind(community_id)
    .fetch_one(&pool)
    .await
    .expect("read audit counter before rollback");
    let missing_event = insert_complete_birth(&pool, community_id, 9, 1, 1, 1, false)
        .await
        .expect_err("receipt/history/binding without audit event must roll back");
    assert_eq!(
        missing_event
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_operation_receipt_event_cardinality")
    );
    let retained_after: i64 = sqlx::query_scalar(
        "SELECT retained_event_count FROM authorization_event_capacity WHERE community_id=$1",
    )
    .bind(community_id)
    .fetch_one(&pool)
    .await
    .expect("read audit counter after rollback");
    assert_eq!(retained_after, retained_before);

    let contended_operation = Uuid::new_v4();
    let mut first = pool.begin().await.expect("begin first operation fence");
    sqlx::query("SELECT nip_fi_lock_authorization_operation_v1($1,$2)")
        .bind(community_id)
        .bind(contended_operation)
        .execute(&mut *first)
        .await
        .expect("acquire first operation fence");
    let waiting_pool = pool.clone();
    let waiter = tokio::spawn(async move {
        let mut transaction = waiting_pool.begin().await.expect("begin waiter");
        sqlx::query("SELECT nip_fi_lock_authorization_operation_v1($1,$2)")
            .bind(community_id)
            .bind(contended_operation)
            .execute(&mut *transaction)
            .await
            .expect("acquire serialized operation fence");
        transaction.commit().await.expect("commit waiter");
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !waiter.is_finished(),
        "same-domain operation contenders must not pass the fence concurrently"
    );
    first.commit().await.expect("release first operation fence");
    tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
        .await
        .expect("serialized waiter completes after release")
        .expect("waiter task succeeds");

    sqlx::query(
        "INSERT INTO authorization_invalidation_domains (community_id,current_generation) \
         VALUES ($1,0)",
    )
    .bind(community_id)
    .execute(&pool)
    .await
    .expect("activate lifecycle invalidation domain");
    let invalidation_operation = Uuid::new_v4();
    let invalidation_request = [91_u8; 32];
    let invalidation_actor = [92_u8; 32];
    let mut invalidation_tx = pool.begin().await.expect("begin invalidation transaction");
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
          outcome_code,result_digest) VALUES ($1,$2,$3,12,$4,1,$5)",
    )
    .bind(community_id)
    .bind(invalidation_operation)
    .bind(invalidation_request.as_slice())
    .bind(invalidation_actor.as_slice())
    .bind([93_u8; 32].as_slice())
    .execute(&mut *invalidation_tx)
    .await
    .expect("stage invalidation receipt");
    let notice = advance_lifecycle_invalidation_tx(
        &mut invalidation_tx,
        CommunityId::from_uuid(community_id),
        invalidation_operation,
        invalidation_request,
        vec![LifecycleInvalidationSelector::Principal([94; 32])],
    )
    .await
    .expect("advance invalidation in lifecycle transaction");
    assert_eq!(notice.generation(), 1);
    invalidation_tx
        .commit()
        .await
        .expect("commit lifecycle invalidation");
    let (generation, floors): (i64, i64) = sqlx::query_as(
        "SELECT d.current_generation,count(f.selector_fingerprint) \
         FROM authorization_invalidation_domains d \
         LEFT JOIN authorization_invalidation_floors f ON f.community_id=d.community_id \
         WHERE d.community_id=$1 GROUP BY d.current_generation",
    )
    .bind(community_id)
    .fetch_one(&pool)
    .await
    .expect("read committed invalidation");
    assert_eq!((generation, floors), (1, 1));

    let mut rollback_tx = pool.begin().await.expect("begin rollback invalidation");
    let rollback_operation = Uuid::new_v4();
    let rollback_request = [95_u8; 32];
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
          outcome_code,result_digest) VALUES ($1,$2,$3,12,$4,1,$5)",
    )
    .bind(community_id)
    .bind(rollback_operation)
    .bind(rollback_request.as_slice())
    .bind(invalidation_actor.as_slice())
    .bind([96_u8; 32].as_slice())
    .execute(&mut *rollback_tx)
    .await
    .expect("stage rollback invalidation receipt");
    advance_lifecycle_invalidation_tx(
        &mut rollback_tx,
        CommunityId::from_uuid(community_id),
        rollback_operation,
        rollback_request,
        vec![LifecycleInvalidationSelector::EventAuthor([97; 32])],
    )
    .await
    .expect("stage rollback invalidation");
    rollback_tx
        .rollback()
        .await
        .expect("roll back lifecycle invalidation");
    let generation_after_rollback: i64 = sqlx::query_scalar(
        "SELECT current_generation FROM authorization_invalidation_domains WHERE community_id=$1",
    )
    .bind(community_id)
    .fetch_one(&pool)
    .await
    .expect("read generation after rollback");
    assert_eq!(generation_after_rollback, 1);
}

#[allow(clippy::too_many_arguments)]
async fn insert_complete_birth(
    pool: &PgPool,
    community_id: Uuid,
    ordinal: u8,
    provenance: i16,
    operation_kind: i16,
    transition_kind: i16,
    include_event: bool,
) -> std::result::Result<(), sqlx::Error> {
    let operation_id = Uuid::new_v4();
    let history_id = Uuid::new_v4();
    let binding_id = Uuid::new_v4();
    let request_fingerprint = vec![ordinal; 32];
    let actor_fingerprint = vec![ordinal.saturating_add(20); 32];
    let mut transaction = pool.begin().await?;
    sqlx::query("SELECT nip_fi_lock_authorization_operation_v1($1,$2)")
        .bind(community_id)
        .bind(operation_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
          outcome_code,result_digest) VALUES ($1,$2,$3,$4,$5,1,$6)",
    )
    .bind(community_id)
    .bind(operation_id)
    .bind(&request_fingerprint)
    .bind(operation_kind)
    .bind(&actor_fingerprint)
    .bind(vec![ordinal.saturating_add(30); 32])
    .execute(&mut *transaction)
    .await?;
    let binding_version: i64 = sqlx::query_scalar(
        "INSERT INTO identity_bindings \
         (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
          binding_state,lifecycle_revision,binding_provenance,policy_revision, \
          enrollment_evidence_digest,birth_history_id,creation_operation_id, \
          creation_request_fingerprint) \
         VALUES ($1,$2,$3,$4,$5,$6,1,1,$7,$8,$9,$10,$11,$12) RETURNING binding_version",
    )
    .bind(community_id)
    .bind(binding_id)
    .bind(format!("https://issuer-{ordinal}.example"))
    .bind(format!("subject-{ordinal}"))
    .bind(vec![ordinal.saturating_add(40); 32])
    .bind(vec![ordinal.saturating_add(50); 32])
    .bind(provenance)
    .bind(i64::from(provenance))
    .bind(vec![ordinal.saturating_add(60); 32])
    .bind(history_id)
    .bind(operation_id)
    .bind(&request_fingerprint)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO identity_lifecycle_history \
         (community_id,history_id,transition_kind,outcome_code,successor_binding_id, \
          successor_binding_version,successor_lifecycle_revision,successor_state, \
          operation_id,request_fingerprint,transition_digest) \
         VALUES ($1,$2,$3,1,$4,$5,1,1,$6,$7,$8)",
    )
    .bind(community_id)
    .bind(history_id)
    .bind(transition_kind)
    .bind(binding_id)
    .bind(binding_version)
    .bind(operation_id)
    .bind(&request_fingerprint)
    .bind(vec![ordinal.saturating_add(70); 32])
    .execute(&mut *transaction)
    .await?;
    if include_event {
        let envelope = vec![ordinal; 32];
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind, \
              actor_fingerprint,operation_id,request_fingerprint,correlation_id,attempt_id, \
              occurred_at,canonical_envelope,envelope_digest) \
             VALUES ($1,$2,1,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
        )
        .bind(community_id)
        .bind(Uuid::new_v4())
        .bind(&actor_fingerprint)
        .bind(operation_id)
        .bind(&request_fingerprint)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(&envelope)
        .bind(vec![ordinal.saturating_add(80); 32])
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await
}
