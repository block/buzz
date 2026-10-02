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
