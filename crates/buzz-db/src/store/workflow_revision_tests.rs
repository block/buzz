//! Mixed-version writers must never leave an exact revision on legacy materialization.
use super::*;

async fn pool() -> PgPool {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("test database URL");
    PgPool::connect(&url).await.expect("connect")
}

async fn fixture(pool: &PgPool) -> WorkflowRecord {
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(format!("revision-{}.example", community.as_uuid()))
        .execute(pool)
        .await
        .unwrap();
    let owner = [0xa1; 32];
    crate::user::ensure_user(pool, community, &owner)
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let mut tx = pool.begin().await.unwrap();
    upsert_workflow(
        &mut tx,
        community,
        id,
        None,
        &owner,
        "revision-test",
        r#"{"trigger":{"on":"schedule"},"steps":[]}"#,
        &[0; 32],
        &[0x42; 32],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    get_workflow(pool, community, id).await.unwrap()
}

// The predecessor's actual ON CONFLICT statement: no reference to the new
// column. In particular, do not simulate it by explicitly writing NULL.
async fn legacy_upsert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    w: &WorkflowRecord,
    definition: &str,
) -> Result<()> {
    sqlx::query(r#"
        INSERT INTO workflows
            (community_id, id, name, owner_pubkey, channel_id, definition, definition_hash, status, enabled)
        VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7, 'active', TRUE)
        ON CONFLICT (community_id, id) DO UPDATE
        SET name = EXCLUDED.name,
            definition = EXCLUDED.definition,
            definition_hash = EXCLUDED.definition_hash,
            updated_at = NOW()
        WHERE workflows.owner_pubkey = EXCLUDED.owner_pubkey
          AND workflows.channel_id IS NOT DISTINCT FROM EXCLUDED.channel_id
        RETURNING id
    "#)
        .bind(w.community_id.as_uuid()).bind(w.id).bind(&w.name)
        .bind(&w.owner_pubkey).bind(w.channel_id).bind(definition)
        .bind(&w.definition_hash).fetch_one(&mut **tx).await?;
    Ok(())
}

async fn rebind(pool: &PgPool, w: &WorkflowRecord) {
    let mut tx = pool.begin().await.unwrap();
    upsert_workflow(
        &mut tx,
        w.community_id,
        w.id,
        w.channel_id,
        &w.owner_pubkey,
        &w.name,
        &w.definition.to_string(),
        &w.definition_hash,
        &[0x43; 32],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn legacy_materialization_clears_revision_even_when_values_match() {
    let pool = pool().await;
    let w = fixture(&pool).await;
    assert_eq!(w.definition_event_id, Some(vec![0x42; 32]));
    let original_run = create_workflow_run(
        &pool,
        w.community_id,
        w.id,
        w.definition_event_id.as_deref(),
        None,
        None,
    )
    .await
    .unwrap();

    for definition in [
        w.definition.to_string(),
        r#"{"steps":[{"id":"different"}]}"#.to_owned(),
    ] {
        rebind(&pool, &w).await;
        assert_eq!(
            get_workflow(&pool, w.community_id, w.id)
                .await
                .unwrap()
                .definition_event_id,
            Some(vec![0x43; 32])
        );
        let mut tx = pool.begin().await.unwrap();
        legacy_upsert(&mut tx, &w, &definition).await.unwrap();
        tx.commit().await.unwrap();
        let current = get_workflow(&pool, w.community_id, w.id).await.unwrap();
        assert!(current.definition_event_id.is_none());
        assert_eq!(
            current.definition,
            serde_json::from_str::<serde_json::Value>(&definition).unwrap()
        );
        let run = create_workflow_run(
            &pool,
            w.community_id,
            w.id,
            current.definition_event_id.as_deref(),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(get_workflow_run(&pool, w.community_id, run)
            .await
            .unwrap()
            .definition_event_id
            .is_none());
    }
    // Already-created runs keep the revision they actually selected.
    assert_eq!(
        get_workflow_run(&pool, w.community_id, original_run)
            .await
            .unwrap()
            .definition_event_id,
        Some(vec![0x42; 32])
    );

    rebind(&pool, &w).await;
    sqlx::query("UPDATE workflows SET enabled = FALSE, status = 'disabled' WHERE community_id = $1 AND id = $2")
        .bind(w.community_id.as_uuid()).bind(w.id).execute(&pool).await.unwrap();
    assert_eq!(
        get_workflow(&pool, w.community_id, w.id)
            .await
            .unwrap()
            .definition_event_id,
        Some(vec![0x43; 32])
    );
    // The sibling materialization helper must also invalidate, not just upsert.
    update_workflow(
        &pool,
        w.community_id,
        w.id,
        &w.name,
        &w.definition.to_string(),
        &w.definition_hash,
    )
    .await
    .unwrap();
    assert!(get_workflow(&pool, w.community_id, w.id)
        .await
        .unwrap()
        .definition_event_id
        .is_none());
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn revision_rebind_is_atomic_against_legacy_writer_and_rollback() {
    let pool = pool().await;
    let w = fixture(&pool).await;
    let mut new_tx = pool.begin().await.unwrap();
    upsert_workflow(
        &mut new_tx,
        w.community_id,
        w.id,
        w.channel_id,
        &w.owner_pubkey,
        &w.name,
        r#"{"steps":[{"id":"new"}]}"#,
        &[1; 32],
        &[0x43; 32],
    )
    .await
    .unwrap();
    // A separate reader sees the complete old row, never the intermediate NULL.
    let visible = get_workflow(&pool, w.community_id, w.id).await.unwrap();
    assert_eq!(visible.definition, w.definition);
    assert_eq!(visible.definition_event_id, w.definition_event_id);
    let mut old_tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL lock_timeout = '100ms'")
        .execute(&mut *old_tx)
        .await
        .unwrap();
    let error = legacy_upsert(&mut old_tx, &w, &w.definition.to_string())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("lock timeout"), "{error}");
    old_tx.rollback().await.unwrap();
    new_tx.rollback().await.unwrap();
    assert_eq!(
        get_workflow(&pool, w.community_id, w.id)
            .await
            .unwrap()
            .definition_event_id,
        w.definition_event_id
    );

    rebind(&pool, &w).await;
    let mut old_tx = pool.begin().await.unwrap();
    legacy_upsert(&mut old_tx, &w, &w.definition.to_string())
        .await
        .unwrap();
    // Rollback of an old writer restores the previous binding as well.
    old_tx.rollback().await.unwrap();
    assert_eq!(
        get_workflow(&pool, w.community_id, w.id)
            .await
            .unwrap()
            .definition_event_id,
        Some(vec![0x43; 32])
    );
    let mut old_tx = pool.begin().await.unwrap();
    legacy_upsert(&mut old_tx, &w, &w.definition.to_string())
        .await
        .unwrap();
    old_tx.commit().await.unwrap();
    assert!(get_workflow(&pool, w.community_id, w.id)
        .await
        .unwrap()
        .definition_event_id
        .is_none());
}
