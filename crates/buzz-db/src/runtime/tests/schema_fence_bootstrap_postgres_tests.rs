use sqlx::PgPool;

/// A fresh bootstrap keeps the declared backstops but never discovers new
/// community tables and silently attaches a trigger to them.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_bootstrap_keeps_only_explicit_legacy_fences() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE new_tenant_probe (community_id UUID NOT NULL)")
        .execute(&pool)
        .await
        .unwrap();
    // Only checked-in schema files contribute SQL to this test bootstrap.
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        crate::test_support::desired_state_schema_sql(),
    ))
    .execute(&pool)
    .await
    .unwrap();

    for (table, expected) in [
        ("new_tenant_probe", false),
        ("events", true),
        ("users", true),
    ] {
        let fenced: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_trigger \
             WHERE tgrelid = $1::regclass \
               AND tgfoid = 'enforce_community_write_fence()'::regprocedure)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(fenced, expected, "unexpected fence on {table}");
    }
}
