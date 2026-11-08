use super::*;
use sqlx::postgres::PgPoolOptions;

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn bootstrap_refuses_legacy_data_before_migrations_and_allows_initialized_data() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").expect("test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect test database");
    let schema = format!("bootstrap_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE SCHEMA {schema}; SET search_path TO {schema}"
    )))
    .execute(&pool)
    .await
    .expect("isolated schema");

    // Legacy schema plus real authority data. The production entry point must
    // refuse before SQLx can create its history or run destructive old steps.
    for migration in GATEWAY_MIGRATOR.iter().take(3) {
        sqlx::raw_sql(migration.sql.as_ref())
            .execute(&pool)
            .await
            .expect("legacy schema");
    }
    sqlx::query("INSERT INTO push_gateway_challenges (id, challenge_hash, expires_at) VALUES ($1, $2, now() + interval '1 hour')")
        .bind(Uuid::new_v4())
        .bind(vec![1_u8; 32])
        .execute(&pool)
        .await
        .expect("legacy challenge");
    let error = PostgresAuthorityStore::apply_migrations_and_grants(&pool, "")
        .await
        .expect_err("populated legacy database must be refused");
    assert!(error.to_string().contains("non-empty pre-launch database"));
    let (count, history_missing): (i64, bool) = sqlx::query_as(
        "SELECT count(*), to_regclass('_sqlx_migrations') IS NULL FROM push_gateway_challenges",
    )
    .fetch_one(&pool)
    .await
    .expect("unchanged legacy state");
    assert_eq!(count, 1);
    assert!(history_missing, "refusal must precede all SQLx migrations");

    // Only the test discards its isolated fixture, never the gateway.
    sqlx::raw_sql(AssertSqlSafe(format!(
        "DROP SCHEMA {schema} CASCADE; CREATE SCHEMA {schema}; SET search_path TO {schema}"
    )))
    .execute(&pool)
    .await
    .expect("fresh fixture");
    for populated in [false, true] {
        if populated {
            sqlx::query("INSERT INTO push_gateway_challenges (id, challenge_hash, expires_at) VALUES ($1, $2, now() + interval '1 hour')")
                .bind(Uuid::new_v4())
                .bind(vec![2_u8; 32])
                .execute(&pool)
                .await
                .expect("initialized gateway data");
        }
        // An invalid role deliberately stops after migrations, avoiding grants
        // against the shared test database's public schema.
        let error = PostgresAuthorityStore::apply_migrations_and_grants(&pool, "")
            .await
            .expect_err("invalid role after successful initialization");
        assert!(error.to_string().contains("runtime database role"));
    }
    sqlx::raw_sql(AssertSqlSafe(format!(
        "SET search_path TO public; DROP SCHEMA {schema} CASCADE"
    )))
    .execute(&pool)
    .await
    .expect("remove isolated test fixture");
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn single_application_cutover_refuses_populated_v5_authority() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").expect("test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .unwrap();
    let schema = format!("cutover_{}", Uuid::new_v4().simple());
    sqlx::raw_sql(AssertSqlSafe(format!(
        "CREATE SCHEMA {schema}; SET search_path TO {schema}"
    )))
    .execute(&pool)
    .await
    .unwrap();
    migrate_gateway_through(&pool, 5).await.unwrap();
    sqlx::query("INSERT INTO push_gateway_installations(id,app_attest_key_id,app_attest_public_key,assertion_counter,app_profile,token_ciphertext,token_fingerprint,endpoint_epoch,expires_at) VALUES($1,$2,$3,0,'buzz-ios-dogfood',$4,$5,1,now()+interval '1 day')")
        .bind(Uuid::new_v4()).bind(vec![1_u8]).bind(vec![2_u8;33]).bind(vec![3_u8]).bind(vec![4_u8;32])
        .execute(&pool).await.unwrap();
    let error = PostgresAuthorityStore::apply_migrations_and_grants(&pool, "")
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("empty gateway authority store"),
        "{error}"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM push_gateway_installations WHERE app_profile='buzz-ios-dogfood'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    // Only the fixture deletes authority. Production refuses without mutation.
    sqlx::query("DELETE FROM push_gateway_installations")
        .execute(&pool)
        .await
        .unwrap();
    let error = PostgresAuthorityStore::apply_migrations_and_grants(&pool, "")
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("runtime database role"),
        "{error}"
    );
    assert!(
        sqlx::query("SELECT app_profile FROM push_gateway_installations")
            .execute(&pool)
            .await
            .is_err()
    );
    sqlx::raw_sql(AssertSqlSafe(format!(
        "SET search_path TO public; DROP SCHEMA {schema} CASCADE"
    )))
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
}
