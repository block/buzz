//! Upgrade-cost and mixed-writer contract for the additive revision migration.
use sqlx::{postgres::PgPoolOptions, Acquire, Executor};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn revision_binding_skips_history_validation_but_enforces_new_writes() {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("isolated test database URL");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect");
    let mut tx = pool.begin().await.expect("transaction");
    let schema = format!("revision_{}", uuid::Uuid::new_v4().simple());
    tx.execute(sqlx::AssertSqlSafe(format!(
        "CREATE SCHEMA {schema}; SET LOCAL search_path = {schema}"
    )))
    .await
    .expect("isolated transactional schema");
    // Populated legacy tables with the semantic columns named by the trigger.
    tx.execute(
        "CREATE TABLE workflows (id int, community_id int, owner_pubkey text,
         channel_id int, name text, definition text, definition_hash bytea, enabled bool);
         CREATE TABLE workflow_runs (id int);
         INSERT INTO workflows (id, name, enabled) VALUES (1, 'legacy', true);
         INSERT INTO workflow_runs VALUES (1);
         CREATE TABLE original_storage AS
         SELECT oid, relfilenode FROM pg_class
         WHERE oid IN ('workflows'::regclass, 'workflow_runs'::regclass);",
    )
    .await
    .expect("legacy tables");
    tx.execute(include_str!(
        "../../../migrations/0043_workflow_revision_binding.sql"
    ))
    .await
    .expect("revision migration");
    let (checks, validated): (i64, bool) = sqlx::query_as(
        "SELECT count(*), bool_or(convalidated) FROM pg_constraint
         WHERE conrelid IN ('workflows'::regclass, 'workflow_runs'::regclass)
         AND contype = 'c'",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("constraint state");
    assert_eq!(checks, 2);
    assert!(
        !validated,
        "upgrade must not scan historical rows to validate checks"
    );
    let unchanged: bool = sqlx::query_scalar(
        "SELECT bool_and(c.relfilenode = o.relfilenode)
         FROM original_storage o JOIN pg_class c USING (oid)",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("physical storage");
    assert!(unchanged, "additive metadata must not rewrite heaps");
    let legacy_null: bool = sqlx::query_scalar(
        "SELECT (SELECT definition_event_id IS NULL FROM workflows WHERE id=1)
         AND (SELECT definition_event_id IS NULL FROM workflow_runs WHERE id=1)",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("legacy provenance");
    assert!(legacy_null);

    // Catch check violations inside savepoints so both INSERT and UPDATE can be
    // tested without aborting the outer migration/fixture transaction.
    for statement in [
        "INSERT INTO workflows (id, definition_event_id) VALUES (2, decode('ab','hex'))",
        "INSERT INTO workflow_runs (id, definition_event_id) VALUES (2, decode('ab','hex'))",
        "UPDATE workflows SET definition_event_id = decode('ab','hex') WHERE id=1",
        "UPDATE workflow_runs SET definition_event_id = decode('ab','hex') WHERE id=1",
    ] {
        let mut savepoint = tx.begin().await.expect("savepoint");
        let error = savepoint
            .execute(statement)
            .await
            .expect_err("bad length denied");
        assert_eq!(
            error.as_database_error().and_then(|e| e.code()).as_deref(),
            Some("23514")
        );
        savepoint.rollback().await.expect("rollback rejected write");
    }
    tx.execute(
        "UPDATE workflows SET definition_event_id = decode(repeat('ab',32),'hex') WHERE id=1;
         UPDATE workflow_runs SET definition_event_id = decode(repeat('ab',32),'hex') WHERE id=1;
         UPDATE workflows SET enabled=false WHERE id=1;",
    )
    .await
    .expect("bind and operational update");
    let bound: bool =
        sqlx::query_scalar("SELECT definition_event_id IS NOT NULL FROM workflows WHERE id=1")
            .fetch_one(&mut *tx)
            .await
            .expect("operational update preserves binding");
    assert!(bound);
    tx.execute("UPDATE workflows SET name=name WHERE id=1")
        .await
        .expect("old writer equal-value semantic update");
    let cleared: bool =
        sqlx::query_scalar("SELECT definition_event_id IS NULL FROM workflows WHERE id=1")
            .fetch_one(&mut *tx)
            .await
            .expect("old writer invalidation");
    assert!(cleared);
    tx.rollback().await.expect("cleanup");
}
