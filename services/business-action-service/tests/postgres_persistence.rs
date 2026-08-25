use business_action_service::{acceptance_engine, PgActionStore};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[tokio::test]
async fn migrations_persist_state_constraints_and_append_only_audit() {
    let Ok(database_url) = std::env::var("BUSINESS_ACTION_TEST_DATABASE_URL") else {
        eprintln!("skipped: BUSINESS_ACTION_TEST_DATABASE_URL is not configured");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect test PostgreSQL");
    let store = PgActionStore::new(pool.clone());
    store.migrate().await.expect("apply migrations");
    let engine = acceptance_engine(Uuid::new_v4()).expect("acceptance engine");
    store.save(&engine).await.expect("persist action state");

    let loaded = store.load().await.expect("load state").expect("state row");
    assert_eq!(loaded.findings.len(), engine.state.findings.len());
    assert_eq!(loaded.proposals.len(), engine.state.proposals.len());
    let finding_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM business_anomaly_findings")
        .fetch_one(&pool)
        .await
        .expect("finding count");
    assert!(finding_rows > 0);

    let active_unique_constraint: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_indexes WHERE indexname='business_work_items_one_active_action'",
    )
    .fetch_one(&pool)
    .await
    .expect("active unique index");
    assert_eq!(active_unique_constraint, 1);

    let forbidden_tenant_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns
         WHERE table_schema='public' AND table_name LIKE 'business_%'
           AND column_name IN ('tenant_id','client_group_id')",
    )
    .fetch_one(&pool)
    .await
    .expect("tenant column check");
    assert_eq!(forbidden_tenant_columns, 0);

    let audit_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM business_action_audit_events ORDER BY occurred_at LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("audit row");
    let mutation =
        sqlx::query("UPDATE business_action_audit_events SET result='changed' WHERE id=$1")
            .bind(audit_id)
            .execute(&pool)
            .await;
    assert!(
        mutation.is_err(),
        "append-only audit update must be rejected"
    );
}
