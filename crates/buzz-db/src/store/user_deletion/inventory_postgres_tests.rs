use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn user_approval_validates_inventory_shape_before_saving_evidence() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let users = db.user_deletion_store();
    let request = users
        .submit("demo:invalid-scope", "operator", None)
        .await
        .unwrap();
    let mut inventory = request.inventory_manifest.unwrap();
    inventory["communities"][0]["relationship"] = "owner".into();
    let digest = crate::entity_deletion::inventory_digest(&inventory).unwrap();
    sqlx::query("UPDATE entity_deletion_requests SET inventory_manifest = $2, inventory_digest = $3 WHERE id = $1")
        .bind(request.id).bind(inventory).bind(&digest).execute(&pool).await.unwrap();
    assert!(users
        .approve(request.id, "approver", &digest)
        .await
        .unwrap_err()
        .to_string()
        .contains("owned-community execution is not supported yet"));
    let after = db
        .entity_deletion_store()
        .inspect(request.id)
        .await
        .unwrap();
    assert_eq!(after.request.status, "inventoried");
    assert!(after.approvals.is_empty());
    assert!(after.stage_history.is_empty());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn inventory_failure_rolls_back_request_and_legacy_manifests_remain_readable() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let users = Db::from_pool(pool.clone()).user_deletion_store();
    let mut request = users.submit("demo:legacy", "operator", None).await.unwrap();
    let mut manifest = request.inventory_manifest.clone().unwrap();
    manifest["version"] = 1.into();
    manifest["target"] = request.target.clone().into();
    for row in manifest["communities"].as_array_mut().unwrap() {
        row["action"] = "delete_subject_data".into();
    }
    request.inventory_manifest = Some(manifest.clone());
    let legacy = UserDeletionInventory::read(&request).unwrap();
    assert_eq!(legacy.communities[0].1, manifest["communities"][0]);
    request.inventory_manifest.as_mut().unwrap()["target"] = "demo:wrong".into();
    assert!(UserDeletionInventory::read(&request).is_err());

    // A real discovery error must not leave a submitted request behind.
    sqlx::query("ALTER TABLE channel_members RENAME TO unavailable_channel_members")
        .execute(&pool)
        .await
        .unwrap();
    let key = "ab".repeat(32);
    assert!(users.submit(&key, "operator", None).await.is_err());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM entity_deletion_requests WHERE target = $1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_user_target_constraint_matches_desired_state() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    Db::from_pool(pool.clone()).migrate().await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    sqlx::raw_sql("CREATE SCHEMA target_parity; SET LOCAL search_path TO target_parity, public;")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../schema/tables/public/entity_deletion_requests.sql"
    ))
    .execute(&mut *tx)
    .await
    .unwrap();
    let definitions: Vec<(String, bool)> = sqlx::query_as(
        "SELECT pg_get_constraintdef(oid), convalidated FROM pg_constraint
         WHERE conname = 'entity_deletion_requests_target_check' ORDER BY conrelid",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    assert_eq!(definitions.len(), 2);
    assert_eq!(definitions[0], definitions[1]);
    assert!(definitions[0].1);
    let restricted_types: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint WHERE conname = 'entity_deletion_requests_entity_type_check'",
    ).fetch_one(&mut *tx).await.unwrap();
    assert_eq!(restricted_types, 2);

    tx.rollback().await.unwrap();
    let error = sqlx::query(
        "INSERT INTO entity_deletion_requests (entity_type, target, requested_by)
         VALUES ('repository', 'repository-123', 'operator')",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().constraint(),
        Some("entity_deletion_requests_entity_type_check")
    );
    let users = Db::from_pool(pool.clone()).user_deletion_store();
    users
        .submit("demo:old-client", "operator", None)
        .await
        .unwrap();
    users
        .submit(&"ab".repeat(32), "operator", None)
        .await
        .unwrap();
    // Storage remains identity-format agnostic. Account lookup belongs in intake.
    use nostr::ToBech32;
    let npub = nostr::PublicKey::parse(&"ab".repeat(32))
        .unwrap()
        .to_bech32()
        .unwrap();
    for target in ["builderlab:account-123", npub.as_str()] {
        sqlx::query(
            "INSERT INTO entity_deletion_requests (target, requested_by) VALUES ($1, 'operator')",
        )
        .bind(target)
        .execute(&pool)
        .await
        .unwrap();
    }
    for target in ["", " ", " account-123", "account-123 ", &"x".repeat(257)] {
        assert!(sqlx::query(
            "INSERT INTO entity_deletion_requests (target, requested_by) VALUES ($1, 'operator')"
        )
        .bind(target)
        .execute(&pool)
        .await
        .is_err());
    }
    assert!(users
        .submit("builderlab:account-123", "operator", None)
        .await
        .is_err());
}
