use super::*;

async fn store() -> EntityDeletionStore {
    EntityDeletionStore {
        pool: PgPool::connect(&crate::test_support::database_url())
            .await
            .expect("connect isolated database"),
    }
}

async fn assert_plain_schema(pool: &PgPool) {
    let fences: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger
         WHERE tgrelid = 'entity_deletion_requests'::regclass",
    )
    .fetch_one(pool)
    .await
    .expect("fences");
    assert_eq!(fences, 0);
    let foreign_keys: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint
         WHERE conrelid = 'entity_deletion_requests'::regclass AND contype = 'f'",
    )
    .fetch_one(pool)
    .await
    .expect("foreign keys");
    assert_eq!(foreign_keys, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_submissions_converge_and_conflicts_preserve_request() {
    let store = store().await;
    let (first, second) = tokio::join!(
        store.submit("demo:alice", "operator-a", Some("manual test")),
        store.submit("demo:alice", "operator-a", Some("manual test")),
    );
    let first = first.expect("first submission");
    let second = second.expect("concurrent submission");
    assert_eq!(first.id, second.id);
    assert_eq!(first.status, "submitted");
    assert_eq!(first.entity_type, "user");
    assert_eq!(first.created_at, second.created_at);
    assert_eq!(first.updated_at, second.updated_at);
    assert!(store
        .submit("demo:alice", "operator-b", Some("manual test"))
        .await
        .is_err());
    assert!(store
        .submit("demo:alice", "operator-a", None)
        .await
        .is_err());
    let saved = store.get(first.id).await.expect("saved request");
    assert_eq!(
        serde_json::to_value(saved).unwrap(),
        serde_json::to_value(first).unwrap()
    );
    assert_eq!(store.list(100).await.expect("list").len(), 1);
    store
        .submit("demo:bob", "operator-a", None)
        .await
        .expect("second target");
    let recent = store.list(1).await.expect("bounded newest first");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].target, "demo:bob");
    assert!(matches!(
        store.get(Uuid::new_v4()).await,
        Err(DbError::NotFound(_))
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invalid_submissions_do_not_create_rows() {
    let store = store().await;
    for target in [
        "alice",
        "demo:",
        "demo:Alice",
        "demo:-alice",
        "demo:alice@example.com",
    ] {
        assert!(
            store.submit(target, "operator-a", None).await.is_err(),
            "{target}"
        );
    }
    assert!(store
        .submit(&format!("demo:{}", "a".repeat(65)), "operator-a", None)
        .await
        .is_err());
    for actor in ["", " operator-a", "operator-a "] {
        assert!(store.submit("demo:alice", actor, None).await.is_err());
    }
    assert!(store
        .submit("demo:alice", &"a".repeat(257), None)
        .await
        .is_err());
    assert!(store
        .submit("demo:alice", "operator-a", Some(&"a".repeat(4097)))
        .await
        .is_err());
    assert!(store.list(100).await.expect("empty list").is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn desired_schema_has_plain_intake_without_database_guards() {
    let store = store().await;
    assert_plain_schema(&store.pool).await;
    crate::Db::from_pool(store.pool.clone())
        .validate_deletion_catalog()
        .await
        .expect("community catalog");
    assert!(sqlx::query(
        "INSERT INTO entity_deletion_requests (target, requested_by)
         VALUES ('real-user', 'operator-a')",
    )
    .execute(&store.pool)
    .await
    .is_err());
    assert!(sqlx::query(
        "INSERT INTO entity_deletion_requests (target, requested_by, status)
         VALUES ('demo:alice', 'operator-a', 'running')",
    )
    .execute(&store.pool)
    .await
    .is_err());
    store
        .submit("demo:alice", "operator-a", None)
        .await
        .expect("valid submission");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_entity_intake_preserves_existing_data() {
    let store = store().await;
    crate::migration::run_migrations_through(&store.pool, 67)
        .await
        .expect("previous schema");
    let community = Uuid::new_v4();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, 'intake-upgrade.invalid')")
        .bind(community)
        .execute(&store.pool)
        .await
        .expect("existing community");
    crate::migration::run_migrations(&store.pool)
        .await
        .expect("upgrade");
    assert_plain_schema(&store.pool).await;
    let registered: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM _operator_global_tables
         WHERE table_name = 'entity_deletion_requests')",
    )
    .fetch_one(&store.pool)
    .await
    .expect("registry");
    assert!(registered);
    let request = store
        .submit("demo:alice", "operator-a", None)
        .await
        .expect("submit");
    crate::migration::run_migrations(&store.pool)
        .await
        .expect("rerun");
    assert_eq!(
        store
            .get(request.id)
            .await
            .expect("preserved request")
            .target,
        "demo:alice"
    );
    let host: String = sqlx::query_scalar("SELECT host FROM communities WHERE id = $1")
        .bind(community)
        .fetch_one(&store.pool)
        .await
        .expect("preserved community");
    assert_eq!(host, "intake-upgrade.invalid");
    crate::Db::from_pool(store.pool.clone())
        .validate_deletion_catalog()
        .await
        .expect("community catalog");
}
