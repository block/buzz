use super::*;

use buzz_core::CommunityId;
use nostr::Keys;
use sqlx::{Executor, PgConnection};
use std::time::Duration;
use uuid::Uuid;

async fn insert_receipt(
    connection: &mut PgConnection,
    community_id: Uuid,
    operation_id: Uuid,
    request_fingerprint: &[u8],
    operation_kind: i16,
) {
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id, operation_id, request_fingerprint, operation_kind, actor_fingerprint, \
          outcome_code, result_digest) VALUES ($1,$2,$3,$4,$5,1,$6)",
    )
    .bind(community_id)
    .bind(operation_id)
    .bind(request_fingerprint)
    .bind(operation_kind)
    .bind(vec![241_u8; 32])
    .bind(vec![242_u8; 32])
    .execute(connection)
    .await
    .expect("insert authorization operation receipt");
}

async fn committed_receipt(
    pool: &PgPool,
    community_id: Uuid,
    operation_kind: i16,
    fingerprint_byte: u8,
) -> (Uuid, Vec<u8>) {
    let operation_id = Uuid::new_v4();
    let request_fingerprint = vec![fingerprint_byte; 32];
    let mut transaction = pool.begin().await.expect("begin receipt transaction");
    insert_receipt(
        &mut transaction,
        community_id,
        operation_id,
        &request_fingerprint,
        operation_kind,
    )
    .await;
    transaction.commit().await.expect("commit receipt");
    (operation_id, request_fingerprint)
}

async fn seed_binding(pool: &PgPool, community_id: Uuid) -> (Uuid, i64, Vec<u8>, nostr::PublicKey) {
    let operation_id = Uuid::new_v4();
    let history_id = Uuid::new_v4();
    let binding_id = Uuid::new_v4();
    let request_fingerprint = vec![31_u8; 32];
    let principal_fingerprint = vec![32_u8; 32];
    let event_author_pubkey = Keys::generate().public_key();
    let mut transaction = pool.begin().await.expect("begin binding seed");
    insert_receipt(
        &mut transaction,
        community_id,
        operation_id,
        &request_fingerprint,
        1,
    )
    .await;
    sqlx::query(
        "INSERT INTO authorization_events \
         (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind, \
          actor_fingerprint,operation_id,request_fingerprint,correlation_id,attempt_id, \
          occurred_at,canonical_envelope,envelope_digest) \
         VALUES ($1,$2,1,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind(vec![241_u8; 32])
    .bind(operation_id)
    .bind(&request_fingerprint)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![35_u8; 4])
    .bind(vec![36_u8; 32])
    .execute(&mut *transaction)
    .await
    .expect("insert authorization-state binding event");
    let binding_version: i64 = sqlx::query_scalar(
        "INSERT INTO identity_bindings \
         (community_id,binding_id,issuer,subject,principal_fingerprint,event_author_pubkey, \
          binding_state,lifecycle_revision,binding_provenance,policy_revision, \
          enrollment_evidence_digest,birth_history_id,creation_operation_id, \
          creation_request_fingerprint) \
         VALUES ($1,$2,'https://issuer.example','authorization-state',$3,$4,1,1,2,1,$5,$6,$7,$8) \
         RETURNING binding_version",
    )
    .bind(community_id)
    .bind(binding_id)
    .bind(&principal_fingerprint)
    .bind(event_author_pubkey.to_bytes().as_slice())
    .bind(vec![33_u8; 32])
    .bind(history_id)
    .bind(operation_id)
    .bind(&request_fingerprint)
    .fetch_one(&mut *transaction)
    .await
    .expect("insert authorization-state binding");
    sqlx::query(
        "INSERT INTO identity_lifecycle_history \
         (community_id,history_id,transition_kind,outcome_code,successor_binding_id, \
          successor_binding_version,successor_lifecycle_revision,successor_state, \
          operation_id,request_fingerprint,transition_digest) \
         VALUES ($1,$2,1,1,$3,$4,1,1,$5,$6,$7)",
    )
    .bind(community_id)
    .bind(history_id)
    .bind(binding_id)
    .bind(binding_version)
    .bind(operation_id)
    .bind(&request_fingerprint)
    .bind(vec![34_u8; 32])
    .execute(&mut *transaction)
    .await
    .expect("insert authorization-state binding history");
    (&mut *transaction)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("validate authorization-state binding");
    transaction.commit().await.expect("commit binding seed");
    (
        binding_id,
        binding_version,
        principal_fingerprint,
        event_author_pubkey,
    )
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn nip_fi_authorization_foundation_behavior() {
    let pool = connect_test_pool().await;
    reset_public_schema(&pool).await;
    run_migrations(&pool)
        .await
        .expect("apply direct-final authorization migrations");

    let durable_status_table: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('public.client_status_revisions')::text")
            .fetch_one(&pool)
            .await
            .expect("inspect durable status table absence");
    assert!(durable_status_table.is_none());

    let community_id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
        .bind(community_id)
        .bind(format!("authorization-{}.example", community_id.simple()))
        .execute(&pool)
        .await
        .expect("insert authorization test community");

    let retired_status_operation = sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
          outcome_code,result_digest) VALUES ($1,$2,$3,13,$4,1,$5)",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind(vec![20_u8; 32])
    .bind(vec![21_u8; 32])
    .bind(vec![22_u8; 32])
    .execute(&pool)
    .await
    .expect_err("retired durable status operation kind must be rejected");
    assert_eq!(
        retired_status_operation
            .as_database_error()
            .and_then(|error| error.code().map(|code| code.into_owned()))
            .as_deref(),
        Some("23514")
    );
    sqlx::query(
        "INSERT INTO identity_enrollment_policies \
         (community_id,policy_revision,enrollment_mode,policy_digest,effective_at) \
         VALUES ($1,1,2,$2,transaction_timestamp())",
    )
    .bind(community_id)
    .bind(vec![30_u8; 32])
    .execute(&pool)
    .await
    .expect("insert authorization test policy");
    sqlx::query(
        "INSERT INTO authorization_event_capacity \
         (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes) \
         VALUES ($1,4,16,4)",
    )
    .bind(community_id)
    .execute(&pool)
    .await
    .expect("install bounded authorization event capacity");
    let (binding_id, binding_version, principal_fingerprint, event_author_pubkey) =
        seed_binding(&pool, community_id).await;

    // The exported helper derives, sorts, and deduplicates the exact trigger
    // lock namespace before lifecycle DML.
    let coordinates = crate::identity_lifecycle::IdentityLifecycleLockCoordinates::new(
        CommunityId::from_uuid(community_id),
        Some(
            principal_fingerprint
                .clone()
                .try_into()
                .expect("32-byte principal"),
        ),
        Some(event_author_pubkey),
        Some(event_author_pubkey),
    )
    .expect("construct typed lifecycle lock coordinates");
    let mut lock_transaction = pool.begin().await.expect("begin typed lock transaction");
    crate::identity_lifecycle::lock_identity_lifecycle_transaction(
        &mut lock_transaction,
        &coordinates,
    )
    .await
    .expect("acquire canonical typed lifecycle locks");
    lock_transaction
        .rollback()
        .await
        .expect("release typed locks");

    // Invalidation floors accept exact replay and strict advancement, while
    // rejecting same-floor attribution rewrites and regression.
    let selector_fingerprint = vec![41_u8; 32];
    let (floor_operation_1, floor_request_1) = committed_receipt(&pool, community_id, 12, 42).await;
    sqlx::query(
        "INSERT INTO authorization_invalidation_floors \
         (community_id,selector_kind,selector_fingerprint,floor_generation, \
          relationship_revision_floor,operation_id,request_fingerprint) \
         VALUES ($1,7,$2,1,1,$3,$4)",
    )
    .bind(community_id)
    .bind(&selector_fingerprint)
    .bind(floor_operation_1)
    .bind(&floor_request_1)
    .execute(&pool)
    .await
    .expect("insert delegated-relationship invalidation floor");
    sqlx::query(
        "UPDATE authorization_invalidation_floors SET floor_generation=floor_generation \
         WHERE community_id=$1 AND selector_kind=7 AND selector_fingerprint=$2",
    )
    .bind(community_id)
    .bind(&selector_fingerprint)
    .execute(&pool)
    .await
    .expect("exact invalidation-floor replay");
    let (floor_operation_2, floor_request_2) = committed_receipt(&pool, community_id, 12, 43).await;
    sqlx::query(
        "UPDATE authorization_invalidation_floors \
         SET floor_generation=2,relationship_revision_floor=2,operation_id=$3, \
             request_fingerprint=$4,updated_at=updated_at+INTERVAL '1 microsecond' \
         WHERE community_id=$1 AND selector_kind=7 AND selector_fingerprint=$2",
    )
    .bind(community_id)
    .bind(&selector_fingerprint)
    .bind(floor_operation_2)
    .bind(&floor_request_2)
    .execute(&pool)
    .await
    .expect("strict invalidation-floor advancement");
    let (floor_operation_3, floor_request_3) = committed_receipt(&pool, community_id, 12, 44).await;
    let floor_relabel = sqlx::query(
        "UPDATE authorization_invalidation_floors \
         SET operation_id=$3,request_fingerprint=$4,updated_at=updated_at+INTERVAL '1 microsecond' \
         WHERE community_id=$1 AND selector_kind=7 AND selector_fingerprint=$2",
    )
    .bind(community_id)
    .bind(&selector_fingerprint)
    .bind(floor_operation_3)
    .bind(&floor_request_3)
    .execute(&pool)
    .await
    .expect_err("equal invalidation floor cannot relabel attribution");
    assert!(floor_relabel.as_database_error().is_some());
    sqlx::query(
        "UPDATE authorization_invalidation_floors SET floor_generation=1 \
         WHERE community_id=$1 AND selector_kind=7 AND selector_fingerprint=$2",
    )
    .bind(community_id)
    .bind(&selector_fingerprint)
    .execute(&pool)
    .await
    .expect_err("invalidation floor cannot regress");

    // Authority epochs require strict epoch, fence, and operation advancement.
    let object_key = vec![51_u8; 32];
    let fence_1 = vec![52_u8; 32];
    let fence_2 = vec![53_u8; 32];
    let (authority_operation_1, authority_request_1) =
        committed_receipt(&pool, community_id, 11, 54).await;
    sqlx::query(
        "INSERT INTO authorization_authority_epochs \
         (community_id,object_kind,object_key,authority_epoch,fence,operation_id,request_fingerprint) \
         VALUES ($1,2,$2,1,$3,$4,$5)",
    )
    .bind(community_id)
    .bind(&object_key)
    .bind(&fence_1)
    .bind(authority_operation_1)
    .bind(&authority_request_1)
    .execute(&pool)
    .await
    .expect("insert authority epoch");
    let (authority_operation_2, authority_request_2) =
        committed_receipt(&pool, community_id, 11, 55).await;
    sqlx::query(
        "UPDATE authorization_authority_epochs \
         SET fence=$3,operation_id=$4,request_fingerprint=$5,updated_at=updated_at+INTERVAL '1 microsecond' \
         WHERE community_id=$1 AND object_kind=2 AND object_key=$2",
    )
    .bind(community_id)
    .bind(&object_key)
    .bind(&fence_2)
    .bind(authority_operation_2)
    .bind(&authority_request_2)
    .execute(&pool)
    .await
    .expect_err("same authority epoch cannot be rewritten");
    sqlx::query(
        "UPDATE authorization_authority_epochs \
         SET authority_epoch=2,operation_id=$3,request_fingerprint=$4, \
             updated_at=updated_at+INTERVAL '1 microsecond' \
         WHERE community_id=$1 AND object_kind=2 AND object_key=$2",
    )
    .bind(community_id)
    .bind(&object_key)
    .bind(authority_operation_2)
    .bind(&authority_request_2)
    .execute(&pool)
    .await
    .expect_err("advanced authority epoch requires a changed fence");
    sqlx::query(
        "UPDATE authorization_authority_epochs \
         SET authority_epoch=2,fence=$3,operation_id=$4,request_fingerprint=$5, \
             updated_at=updated_at+INTERVAL '1 microsecond' \
         WHERE community_id=$1 AND object_kind=2 AND object_key=$2",
    )
    .bind(community_id)
    .bind(&object_key)
    .bind(&fence_2)
    .bind(authority_operation_2)
    .bind(&authority_request_2)
    .execute(&pool)
    .await
    .expect("strict authority epoch advancement");

    sqlx::query(
        "INSERT INTO protected_object_authority \
         (community_id,object_kind,object_key,capability,actor_pubkey,binding_id,binding_version, \
          policy_revision,invalidation_generation,authority_epoch,fence,issued_at,expires_at, \
          operation_id,request_fingerprint) \
         VALUES ($1,2,$2,1,$3,$4,$5,1,0,2,$6,transaction_timestamp(), \
                 transaction_timestamp()+INTERVAL '5 minutes',$7,$8)",
    )
    .bind(community_id)
    .bind(&object_key)
    .bind(event_author_pubkey.to_bytes().as_slice())
    .bind(binding_id)
    .bind(binding_version)
    .bind(&fence_2)
    .bind(authority_operation_2)
    .bind(&authority_request_2)
    .execute(&pool)
    .await
    .expect("insert exact protected authority");
    sqlx::query(
        "UPDATE protected_object_authority SET capability=2 \
         WHERE community_id=$1 AND object_kind=2 AND object_key=$2",
    )
    .bind(community_id)
    .bind(&object_key)
    .execute(&pool)
    .await
    .expect_err("protected authority cannot rewrite semantics at the same epoch");

    let fence_3 = vec![75_u8; 32];
    let (authority_operation_3, authority_request_3) =
        committed_receipt(&pool, community_id, 11, 76).await;
    let mut protected_replacement = pool
        .begin()
        .await
        .expect("begin protected authority replacement");
    sqlx::query(
        "UPDATE authorization_authority_epochs \
         SET authority_epoch=3,fence=$3,operation_id=$4,request_fingerprint=$5, \
             updated_at=updated_at+INTERVAL '1 microsecond' \
         WHERE community_id=$1 AND object_kind=2 AND object_key=$2",
    )
    .bind(community_id)
    .bind(&object_key)
    .bind(&fence_3)
    .bind(authority_operation_3)
    .bind(&authority_request_3)
    .execute(&mut *protected_replacement)
    .await
    .expect("advance authority tuple for protected replacement");
    sqlx::query(
        "UPDATE protected_object_authority \
         SET capability=2,authority_epoch=3,fence=$3,operation_id=$4,request_fingerprint=$5, \
             issued_at=issued_at+INTERVAL '1 second' \
         WHERE community_id=$1 AND object_kind=2 AND object_key=$2",
    )
    .bind(community_id)
    .bind(&object_key)
    .bind(&fence_3)
    .bind(authority_operation_3)
    .bind(&authority_request_3)
    .execute(&mut *protected_replacement)
    .await
    .expect("strict protected authority replacement");
    (&mut *protected_replacement)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("protected authority remains bound to exact epoch tuple");
    protected_replacement
        .commit()
        .await
        .expect("commit protected authority replacement");

    let delegated_key = vec![56_u8; 32];
    let delegated_fence = vec![57_u8; 32];
    let (delegated_operation, delegated_request) =
        committed_receipt(&pool, community_id, 11, 58).await;
    sqlx::query(
        "INSERT INTO authorization_authority_epochs \
         (community_id,object_kind,object_key,authority_epoch,fence,operation_id,request_fingerprint) \
         VALUES ($1,3,$2,1,$3,$4,$5)",
    )
    .bind(community_id)
    .bind(&delegated_key)
    .bind(&delegated_fence)
    .bind(delegated_operation)
    .bind(&delegated_request)
    .execute(&pool)
    .await
    .expect("insert delegated authority epoch");
    let nil_relationship = sqlx::query(
        "INSERT INTO protected_object_authority \
         (community_id,object_kind,object_key,capability,actor_pubkey,owner_pubkey,binding_id, \
          binding_version,delegated_relationship_id,delegated_relationship_revision, \
          delegation_conditions_fingerprint,policy_revision,invalidation_generation, \
          authority_epoch,fence,issued_at,expires_at,operation_id,request_fingerprint) \
         VALUES ($1,3,$2,1,$3,$3,$4,$5,'00000000-0000-0000-0000-000000000000',1,$6, \
                 1,0,1,$7,transaction_timestamp(),transaction_timestamp()+INTERVAL '5 minutes',$8,$9)",
    )
    .bind(community_id)
    .bind(&delegated_key)
    .bind(event_author_pubkey.to_bytes().as_slice())
    .bind(binding_id)
    .bind(binding_version)
    .bind(vec![59_u8; 32])
    .bind(&delegated_fence)
    .bind(delegated_operation)
    .bind(&delegated_request)
    .execute(&pool)
    .await
    .expect_err("delegated relationship id must reject the nil sentinel");
    assert_eq!(
        nil_relationship
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("protected_object_authority_delegated_relationship_non_nil")
    );

    // The lifecycle seed consumed the first bounded event. Two additional
    // success events reach the exact non-restrictive limits while preserving
    // one event and four bytes for security evidence. Exhaustion and unhealthy
    // state reject later events without advancing either counter, and health
    // cannot be reset or relabelled online.
    for event_byte in [71_u8, 72_u8] {
        let (operation_id, request_fingerprint) =
            committed_receipt(&pool, community_id, 11, event_byte).await;
        sqlx::query(
            "INSERT INTO authorization_events \
             (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind,actor_fingerprint, \
              operation_id,request_fingerprint,correlation_id,attempt_id,occurred_at, \
              canonical_envelope,envelope_digest) \
             VALUES ($1,$2,10,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
        )
        .bind(community_id)
        .bind(Uuid::new_v4())
        .bind(vec![event_byte; 32])
        .bind(operation_id)
        .bind(&request_fingerprint)
        .bind(Uuid::new_v4())
        .bind(Uuid::new_v4())
        .bind(vec![event_byte; 4])
        .bind(vec![event_byte; 32])
        .execute(&pool)
        .await
        .expect("insert bounded authorization event");
    }
    let exhausted_operation = Uuid::new_v4();
    let exhausted_request = vec![73_u8; 32];
    let exhausted_fence = vec![74_u8; 32];
    let mut exhausted_transaction = pool.begin().await.expect("begin exhausted audit mutation");
    insert_receipt(
        &mut exhausted_transaction,
        community_id,
        exhausted_operation,
        &exhausted_request,
        11,
    )
    .await;
    sqlx::query(
        "UPDATE authorization_authority_epochs \
         SET authority_epoch=4,fence=$3,operation_id=$4,request_fingerprint=$5, \
             updated_at=updated_at+INTERVAL '1 microsecond' \
         WHERE community_id=$1 AND object_kind=2 AND object_key=$2",
    )
    .bind(community_id)
    .bind(&object_key)
    .bind(&exhausted_fence)
    .bind(exhausted_operation)
    .bind(&exhausted_request)
    .execute(&mut *exhausted_transaction)
    .await
    .expect("stage protected authority epoch before exhausted audit append");
    sqlx::query(
        "UPDATE protected_object_authority \
         SET capability=3,authority_epoch=4,fence=$3,operation_id=$4,request_fingerprint=$5, \
             issued_at=issued_at+INTERVAL '1 second' \
         WHERE community_id=$1 AND object_kind=2 AND object_key=$2",
    )
    .bind(community_id)
    .bind(&object_key)
    .bind(&exhausted_fence)
    .bind(exhausted_operation)
    .bind(&exhausted_request)
    .execute(&mut *exhausted_transaction)
    .await
    .expect("stage protected mutation before exhausted audit append");
    let exhausted = sqlx::query(
        "INSERT INTO authorization_events \
         (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind,actor_fingerprint, \
          operation_id,request_fingerprint,correlation_id,attempt_id,occurred_at, \
          canonical_envelope,envelope_digest) \
         VALUES ($1,$2,10,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind(vec![73_u8; 32])
    .bind(exhausted_operation)
    .bind(&exhausted_request)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![73_u8; 1])
    .bind(vec![73_u8; 32])
    .execute(&mut *exhausted_transaction)
    .await
    .expect_err("authorization event capacity must fail closed");
    assert_eq!(
        exhausted
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_event_capacity_exhausted")
    );
    exhausted_transaction
        .rollback()
        .await
        .expect("rollback exhausted protected mutation and receipt");
    let rolled_back_authority: (i64, Vec<u8>, i16, i64) = sqlx::query_as(
        "SELECT epoch.authority_epoch,epoch.fence,protected.capability,protected.authority_epoch \
         FROM authorization_authority_epochs epoch \
         JOIN protected_object_authority protected \
           ON protected.community_id=epoch.community_id \
          AND protected.object_kind=epoch.object_kind \
          AND protected.object_key=epoch.object_key \
         WHERE epoch.community_id=$1 AND epoch.object_kind=2 AND epoch.object_key=$2",
    )
    .bind(community_id)
    .bind(&object_key)
    .fetch_one(&pool)
    .await
    .expect("read authority after exhausted audit rollback");
    assert_eq!(rolled_back_authority, (3, fence_3.clone(), 2, 3));
    let rolled_back_receipt: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_operation_receipts \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id)
    .bind(exhausted_operation)
    .fetch_one(&pool)
    .await
    .expect("count exhausted operation receipt residue");
    assert_eq!(rolled_back_receipt, 0);
    let counters: (i64, i64) = sqlx::query_as(
        "SELECT retained_event_count,retained_envelope_bytes \
         FROM authorization_event_capacity WHERE community_id=$1",
    )
    .bind(community_id)
    .fetch_one(&pool)
    .await
    .expect("read bounded authorization event counters");
    assert_eq!(counters, (3, 12));
    sqlx::query("UPDATE authorization_events SET reason_code=2 WHERE community_id=$1")
        .bind(community_id)
        .execute(&pool)
        .await
        .expect_err("authorization event envelopes are immutable");
    let failure_generation: i64 =
        sqlx::query_scalar("SELECT authorization_event_capacity_report_failure_v2($1,1::smallint)")
            .bind(community_id)
            .fetch_one(&pool)
            .await
            .expect("latch authorization event health");
    assert_eq!(failure_generation, 1);
    sqlx::query(
        "UPDATE authorization_event_capacity SET failure_code=2 \
         WHERE community_id=$1",
    )
    .bind(community_id)
    .execute(&pool)
    .await
    .expect_err("latched authorization event failure cannot be relabelled");
    sqlx::query(
        "UPDATE authorization_event_capacity SET health_state=1,failure_code=NULL, \
         failure_observed_at=NULL WHERE community_id=$1",
    )
    .bind(community_id)
    .execute(&pool)
    .await
    .expect_err("authorization event health cannot reset online");
    let (unhealthy_operation, unhealthy_request) =
        committed_receipt(&pool, community_id, 11, 74).await;
    let unhealthy = sqlx::query(
        "INSERT INTO authorization_events \
         (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind,actor_fingerprint, \
          operation_id,request_fingerprint,correlation_id,attempt_id,occurred_at, \
          canonical_envelope,envelope_digest) \
         VALUES ($1,$2,10,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind(vec![74_u8; 32])
    .bind(unhealthy_operation)
    .bind(&unhealthy_request)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![74_u8; 1])
    .bind(vec![74_u8; 32])
    .execute(&pool)
    .await
    .expect_err("unhealthy authorization event lane must fail closed");
    assert_eq!(
        unhealthy
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_event_capacity_health")
    );

    // Operation-bound restore attribution supports canonical empty and
    // non-empty manifests, enforces strict deltas and receipt fingerprints,
    // and remains immutable.
    let (empty_operation, empty_request) = committed_receipt(&pool, community_id, 12, 81).await;
    sqlx::query(
        "INSERT INTO authorization_operation_version_delta_manifests \
         (community_id,operation_id,request_fingerprint,component_count,before_digest, \
          after_digest,manifest_digest) VALUES ($1,$2,$3,0,$4,$5,$6)",
    )
    .bind(community_id)
    .bind(empty_operation)
    .bind(&empty_request)
    .bind(vec![82_u8; 32])
    .bind(vec![83_u8; 32])
    .bind(vec![84_u8; 32])
    .execute(&pool)
    .await
    .expect("insert empty operation-delta manifest");
    sqlx::query(
        "UPDATE authorization_operation_version_delta_manifests SET manifest_digest=$3 \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id)
    .bind(empty_operation)
    .bind(vec![85_u8; 32])
    .execute(&pool)
    .await
    .expect_err("operation-delta manifest is immutable");

    sqlx::query(
        "INSERT INTO authorization_operation_version_deltas \
         (community_id,operation_id,component_kind,component_key,before_version, \
          after_version,component_digest) VALUES ($1,$2,4,$3,1,1,$4)",
    )
    .bind(community_id)
    .bind(empty_operation)
    .bind(vec![85_u8; 32])
    .bind(vec![86_u8; 32])
    .execute(&pool)
    .await
    .expect_err("zero operation delta must be rejected");
    let late_component = sqlx::query(
        "INSERT INTO authorization_operation_version_deltas \
         (community_id,operation_id,component_kind,component_key,before_version, \
          after_version,component_digest) VALUES ($1,$2,4,$3,1,2,$4)",
    )
    .bind(community_id)
    .bind(empty_operation)
    .bind(vec![85_u8; 32])
    .bind(vec![86_u8; 32])
    .execute(&pool)
    .await
    .expect_err("a committed empty manifest cannot receive a late component");
    assert_eq!(
        late_component
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_operation_version_delta_cardinality")
    );

    let (missing_operation, missing_request) = committed_receipt(&pool, community_id, 12, 87).await;
    let missing_component = sqlx::query(
        "INSERT INTO authorization_operation_version_delta_manifests \
         (community_id,operation_id,request_fingerprint,component_count,before_digest, \
          after_digest,manifest_digest) VALUES ($1,$2,$3,1,$4,$5,$6)",
    )
    .bind(community_id)
    .bind(missing_operation)
    .bind(&missing_request)
    .bind(vec![88_u8; 32])
    .bind(vec![89_u8; 32])
    .bind(vec![90_u8; 32])
    .execute(&pool)
    .await
    .expect_err("a manifest cannot commit with a missing component");
    assert_eq!(
        missing_component
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_operation_version_delta_cardinality")
    );

    let (delta_operation, delta_request) = committed_receipt(&pool, community_id, 12, 86).await;
    let mut delta_transaction = pool.begin().await.expect("begin exact delta manifest");
    sqlx::query(
        "INSERT INTO authorization_operation_version_delta_manifests \
         (community_id,operation_id,request_fingerprint,component_count,before_digest, \
          after_digest,manifest_digest) VALUES ($1,$2,$3,1,$4,$5,$6)",
    )
    .bind(community_id)
    .bind(delta_operation)
    .bind(&delta_request)
    .bind(vec![87_u8; 32])
    .bind(vec![88_u8; 32])
    .bind(vec![89_u8; 32])
    .execute(&mut *delta_transaction)
    .await
    .expect("insert non-empty operation-delta manifest");
    sqlx::query(
        "INSERT INTO authorization_operation_version_deltas \
         (community_id,operation_id,component_kind,component_key,before_version, \
          after_version,component_digest) VALUES ($1,$2,4,$3,1,2,$4)",
    )
    .bind(community_id)
    .bind(delta_operation)
    .bind(vec![90_u8; 32])
    .bind(vec![91_u8; 32])
    .execute(&mut *delta_transaction)
    .await
    .expect("insert strict operation delta");
    (&mut *delta_transaction)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect("manifest and exact component set satisfy deferred cardinality");
    delta_transaction
        .commit()
        .await
        .expect("commit exact delta manifest");

    // Lock the already-valid count-one manifest while a concurrent transaction
    // stages an extra component. Its deferred recount must wait for this lock,
    // then reject the extra row without disturbing the committed component.
    let mut manifest_lock = pool.begin().await.expect("begin manifest lock");
    let component_count: i32 = sqlx::query_scalar(
        "SELECT component_count FROM authorization_operation_version_delta_manifests \
         WHERE community_id=$1 AND operation_id=$2 FOR NO KEY UPDATE",
    )
    .bind(community_id)
    .bind(delta_operation)
    .fetch_one(&mut *manifest_lock)
    .await
    .expect("lock committed exact manifest");
    assert_eq!(component_count, 1);
    let committed_components: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_operation_version_deltas \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id)
    .bind(delta_operation)
    .fetch_one(&mut *manifest_lock)
    .await
    .expect("count committed exact components");
    assert_eq!(committed_components, 1);

    let loser_component_key = vec![104_u8; 32];
    let loser_pool = pool.clone();
    let loser_key = loser_component_key.clone();
    let (loser_ready_tx, loser_ready_rx) = tokio::sync::oneshot::channel();
    let mut extra_component_loser = tokio::spawn(async move {
        let mut transaction = loser_pool
            .begin()
            .await
            .expect("begin concurrent extra component");
        sqlx::query(
            "INSERT INTO authorization_operation_version_deltas \
             (community_id,operation_id,component_kind,component_key,before_version, \
              after_version,component_digest) VALUES ($1,$2,6,$3,1,2,$4)",
        )
        .bind(community_id)
        .bind(delta_operation)
        .bind(&loser_key)
        .bind(vec![105_u8; 32])
        .execute(&mut *transaction)
        .await
        .expect("stage extra component against committed manifest");
        loser_ready_tx
            .send(())
            .expect("publish staged extra-component readiness");
        let error = (&mut *transaction)
            .execute("SET CONSTRAINTS ALL IMMEDIATE")
            .await
            .expect_err("extra component must violate exact manifest cardinality");
        let failed_constraint = error
            .as_database_error()
            .and_then(|database_error| database_error.constraint())
            .map(str::to_owned);
        transaction
            .rollback()
            .await
            .expect("rollback rejected extra component");
        failed_constraint
    });
    tokio::time::timeout(Duration::from_secs(2), loser_ready_rx)
        .await
        .expect("extra-component loser must start")
        .expect("extra-component loser must publish readiness");
    assert!(
        tokio::time::timeout(Duration::from_millis(250), &mut extra_component_loser)
            .await
            .is_err(),
        "extra-component loser must wait for the valid manifest transaction"
    );
    manifest_lock
        .commit()
        .await
        .expect("release exact manifest lock");
    let loser_constraint = tokio::time::timeout(Duration::from_secs(2), extra_component_loser)
        .await
        .expect("extra-component loser must resume")
        .expect("extra-component loser task must finish");
    assert_eq!(
        loser_constraint.as_deref(),
        Some("authorization_operation_version_delta_cardinality")
    );
    let delta_counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*),count(*) FILTER (WHERE component_key=$3) \
         FROM authorization_operation_version_deltas \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id)
    .bind(delta_operation)
    .bind(&loser_component_key)
    .fetch_one(&pool)
    .await
    .expect("verify exact manifest winner and zero loser residue");
    assert_eq!(delta_counts, (1, 0));
    sqlx::query(
        "DELETE FROM authorization_operation_version_deltas \
         WHERE community_id=$1 AND operation_id=$2",
    )
    .bind(community_id)
    .bind(delta_operation)
    .execute(&pool)
    .await
    .expect_err("operation delta is immutable");

    let (extra_operation, extra_request) = committed_receipt(&pool, community_id, 12, 96).await;
    let mut extra = pool.begin().await.expect("begin extra delta component");
    sqlx::query(
        "INSERT INTO authorization_operation_version_delta_manifests \
         (community_id,operation_id,request_fingerprint,component_count,before_digest, \
          after_digest,manifest_digest) VALUES ($1,$2,$3,1,$4,$5,$6)",
    )
    .bind(community_id)
    .bind(extra_operation)
    .bind(&extra_request)
    .bind(vec![97_u8; 32])
    .bind(vec![98_u8; 32])
    .bind(vec![99_u8; 32])
    .execute(&mut *extra)
    .await
    .expect("defer extra-component manifest cardinality");
    for (kind, key_byte) in [(4_i16, 100_u8), (6_i16, 101_u8)] {
        sqlx::query(
            "INSERT INTO authorization_operation_version_deltas \
             (community_id,operation_id,component_kind,component_key,before_version, \
              after_version,component_digest) VALUES ($1,$2,$3,$4,1,2,$5)",
        )
        .bind(community_id)
        .bind(extra_operation)
        .bind(kind)
        .bind(vec![key_byte; 32])
        .bind(vec![key_byte.wrapping_add(1); 32])
        .execute(&mut *extra)
        .await
        .expect("insert component before deferred exact-count check");
    }
    let extra_component = (&mut *extra)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect_err("a manifest cannot commit with extra components");
    assert_eq!(
        extra_component
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_operation_version_delta_cardinality")
    );
    extra.rollback().await.expect("rollback extra components");

    let (mismatch_operation, mismatch_request) =
        committed_receipt(&pool, community_id, 12, 92).await;
    let mut mismatch = pool.begin().await.expect("begin fingerprint mismatch");
    sqlx::query(
        "INSERT INTO authorization_operation_version_delta_manifests \
         (community_id,operation_id,request_fingerprint,component_count,before_digest, \
          after_digest,manifest_digest) VALUES ($1,$2,$3,0,$4,$5,$6)",
    )
    .bind(community_id)
    .bind(mismatch_operation)
    .bind(vec![mismatch_request[0].wrapping_add(1); 32])
    .bind(vec![93_u8; 32])
    .bind(vec![94_u8; 32])
    .bind(vec![95_u8; 32])
    .execute(&mut *mismatch)
    .await
    .expect("defer manifest receipt fingerprint validation");
    (&mut *mismatch)
        .execute("SET CONSTRAINTS ALL IMMEDIATE")
        .await
        .expect_err("manifest must bind the exact receipt fingerprint");
    mismatch
        .rollback()
        .await
        .expect("rollback fingerprint mismatch");
}
