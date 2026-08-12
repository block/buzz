use super::*;

use uuid::Uuid;

const OPERATOR_AUDIT_SQL: &str =
    include_str!("../../../../../migrations/0034_nip_fi_operator_preauth.sql");

#[test]
fn operator_audit_upgrade_is_bounded_and_uses_constant_time_capacity() {
    for required in [
        "restrictive_reserve_events",
        "failure_generation",
        "recovery_generation",
        "authorization_event_capacity_before_insert_v2",
        "authorization_operator_denial_buckets",
        "selected_slot := (generation % 12)::SMALLINT",
        "authorization_operator_denial_bucket_record_v1",
    ] {
        assert!(OPERATOR_AUDIT_SQL.contains(required), "missing {required}");
    }
    assert!(OPERATOR_AUDIT_SQL.contains("FOR UPDATE"));
    assert!(OPERATOR_AUDIT_SQL.contains("NEW.event_kind IN (2, 3, 4, 5, 6, 7, 8, 9, 11, 14)"));
    assert!(!OPERATOR_AUDIT_SQL.contains("count(*) FROM authorization_events"));
    assert!(!OPERATOR_AUDIT_SQL.contains("sum(octet_length"));
    assert!(
        !OPERATOR_AUDIT_SQL.contains("CREATE TABLE authorization_operator_preauth_denial_events")
    );
}

#[tokio::test]
#[ignore = "requires a dedicated disposable Postgres database"]
async fn operator_0034_reserves_restrictive_audit_and_recovers_by_generation() {
    let pool = connect_test_pool().await;
    reset_public_schema(&pool).await;
    run_migrations(&pool)
        .await
        .expect("apply migrations through operator audit 0034");

    let community_id = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id,host) VALUES ($1,$2)")
        .bind(community_id)
        .bind(format!("operator-audit-{}.example", community_id.simple()))
        .execute(&pool)
        .await
        .expect("insert operator audit community");
    sqlx::query(
        "INSERT INTO authorization_event_capacity \
         (community_id,max_events_per_domain,max_bytes_per_domain,max_envelope_bytes, \
          restrictive_reserve_events,restrictive_reserve_bytes) \
         VALUES ($1,4,4096,512,1,512)",
    )
    .bind(community_id)
    .execute(&pool)
    .await
    .expect("install bounded operator audit capacity");

    for ordinal in 1..=3_u8 {
        insert_authenticated_event(&pool, community_id, ordinal)
            .await
            .expect("general event fits outside reserve");
    }
    let general_exhausted = insert_authenticated_event(&pool, community_id, 4)
        .await
        .expect_err("general event cannot consume restrictive reserve");
    assert_eq!(
        general_exhausted
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_event_capacity_exhausted")
    );
    insert_unresolved_denial(&pool, community_id)
        .await
        .expect("restrictive denial consumes reserved slot");

    let generation: i64 =
        sqlx::query_scalar("SELECT authorization_event_capacity_report_failure_v2($1,$2)")
            .bind(community_id)
            .bind(1_i16)
            .fetch_one(&pool)
            .await
            .expect("latch audit failure");
    assert_eq!(generation, 1);
    let rewrite_unhealthy = sqlx::query(
        "UPDATE authorization_event_capacity \
         SET failure_code=2,failure_observed_at=failure_observed_at + interval '1 microsecond', \
             updated_at=updated_at + interval '1 second' \
         WHERE community_id=$1",
    )
    .bind(community_id)
    .execute(&pool)
    .await
    .expect_err("same-state failure metadata is immutable");
    assert_eq!(
        rewrite_unhealthy
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_event_capacity_monotonic_v2")
    );
    let recovered: bool =
        sqlx::query_scalar("SELECT authorization_event_capacity_recover_v2($1,$2)")
            .bind(community_id)
            .bind(generation)
            .fetch_one(&pool)
            .await
            .expect("recover exact failure generation");
    assert!(recovered);
    let rewrite_healthy = sqlx::query(
        "UPDATE authorization_event_capacity \
         SET recovered_at=recovered_at + interval '1 microsecond', \
             updated_at=updated_at + interval '1 second' \
         WHERE community_id=$1",
    )
    .bind(community_id)
    .execute(&pool)
    .await
    .expect_err("same-state recovery metadata is immutable");
    assert_eq!(
        rewrite_healthy
            .as_database_error()
            .and_then(|error| error.constraint()),
        Some("authorization_event_capacity_monotonic_v2")
    );
    let stale: bool = sqlx::query_scalar("SELECT authorization_event_capacity_recover_v2($1,$2)")
        .bind(community_id)
        .bind(generation)
        .fetch_one(&pool)
        .await
        .expect("stale recovery is a closed false result");
    assert!(!stale);

    for expected_count in 1_i64..=3 {
        let retained: i64 =
            sqlx::query_scalar("SELECT authorization_operator_denial_bucket_record_v1($1,$2,$3)")
                .bind(community_id)
                .bind(4_i16)
                .bind(6_i16)
                .fetch_one(&pool)
                .await
                .expect("record fixed-cardinality denial bucket");
        assert_eq!(retained, expected_count);
    }
    let bucket_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_operator_denial_buckets WHERE community_id=$1",
    )
    .bind(community_id)
    .fetch_one(&pool)
    .await
    .expect("count bounded denial rows");
    assert_eq!(bucket_rows, 1);
}

async fn insert_authenticated_event(
    pool: &PgPool,
    community_id: Uuid,
    ordinal: u8,
) -> std::result::Result<(), sqlx::Error> {
    let operation_id = Uuid::new_v4();
    let request_fingerprint = vec![ordinal; 32];
    let actor_fingerprint = vec![ordinal.saturating_add(20); 32];
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO authorization_operation_receipts \
         (community_id,operation_id,request_fingerprint,operation_kind,actor_fingerprint, \
          outcome_code,result_digest) VALUES ($1,$2,$3,11,$4,1,$5)",
    )
    .bind(community_id)
    .bind(operation_id)
    .bind(&request_fingerprint)
    .bind(&actor_fingerprint)
    .bind(vec![ordinal.saturating_add(40); 32])
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO authorization_events \
         (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind, \
          actor_fingerprint,operation_id,request_fingerprint,correlation_id,attempt_id, \
          occurred_at,canonical_envelope,envelope_digest) \
         VALUES ($1,$2,10,1,1,1,$3,$4,$5,$6,$7,transaction_timestamp(),$8,$9)",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind(&actor_fingerprint)
    .bind(operation_id)
    .bind(&request_fingerprint)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![ordinal; 64])
    .bind(vec![ordinal.saturating_add(60); 32])
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await
}

async fn insert_unresolved_denial(
    pool: &PgPool,
    community_id: Uuid,
) -> std::result::Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO authorization_events \
         (community_id,event_id,event_kind,outcome_code,reason_code,actor_kind, \
          actor_fingerprint,subject_fingerprint,operation_id,request_fingerprint, \
          correlation_id,attempt_id,occurred_at,canonical_envelope,envelope_digest) \
         VALUES ($1,$2,9,2,9,4,NULL,NULL,$3,NULL,$4,$5,transaction_timestamp(),$6,$7)",
    )
    .bind(community_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(vec![9_u8; 64])
    .bind(vec![99_u8; 32])
    .execute(pool)
    .await?;
    Ok(())
}
